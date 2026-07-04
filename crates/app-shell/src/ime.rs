//! IME integration for composition input (M1 Task 7 — design risk R3).
//!
//! # Why IMM32 and not TSF
//!
//! `windows-reactor`'s XAML surface exposes **no** TSF / `CoreTextEditContext`
//! text store (the M0 re-baseline confirmed the keyboard/char/focus/IME slots are
//! vtable stubs). Full `ITfThreadMgr` TSF embedding needs a real TSF-enabled
//! surface we do not have, so M1 rides the **IMM32 composition-message path** on
//! the host `HWND` — `WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION` (with
//! `GCS_COMPSTR` for the in-flight preview and `GCS_RESULTSTR` for the commit) /
//! `WM_IME_ENDCOMPOSITION`, plus `WM_IME_SETCONTEXT` to suppress the system
//! composition window so we draw the candidate inline ourselves. IMM32 rides on
//! top of TSF for every shipping Win11 IME, so this is the standard fallback for
//! exactly this "no cooperating text store" situation.
//!
//! # Structure (two layers, deliberately split)
//!
//!   * [`ImeSession`] — a pure state machine (`Idle → Composing → Commit|Cancel`)
//!     that consumes parsed [`CompositionEvent`]s and emits [`ImeAction`]s. It
//!     touches **no** Win32 API and is exhaustively unit-tested off-Windows.
//!   * [`win32`] (`cfg(windows)`) — translates raw `WM_IME_*` / `WM_KILLFOCUS`
//!     messages into [`CompositionEvent`]s via `ImmGetCompositionStringW`, and
//!     owns the candidate-window positioning + the double-commit suppression
//!     window. It never holds composition state itself; the state lives in
//!     [`ImeSession`].
//!
//! The commit path writes committed UTF-8 **directly** to the PTY input sender,
//! bypassing the key encoder — see [`ImeAction::SendToPty`] and its doc comment.

/// A parsed composition event — the boundary between the Win32 message layer and
/// the [`ImeSession`] state machine. The Win32 layer produces these from
/// `WM_IME_*`; tests produce them directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionEvent {
    /// `WM_IME_STARTCOMPOSITION` — a composition began.
    Start,
    /// `WM_IME_COMPOSITION` carrying `GCS_COMPSTR` — the in-flight preview text
    /// changed. `caret` is the caret position **in `char`s** within `comp_text`
    /// (from `GCS_CURSORPOS`), clamped to `comp_text.chars().count()`.
    Update { comp_text: String, caret: usize },
    /// `WM_IME_COMPOSITION` carrying `GCS_RESULTSTR` — the composition committed
    /// this exact string. Emitted before any same-message `GCS_COMPSTR` restart
    /// (see the Win32 layer's "result-then-comp in one message" handling).
    Commit { result_text: String },
    /// `WM_IME_ENDCOMPOSITION` — composition finished (commit already delivered
    /// via its own `Commit` event, so this only clears preview state).
    End,
    /// `WM_KILLFOCUS` while composing — cancel with no committed bytes.
    FocusLost,
}

/// An action the host loop must perform in response to a [`CompositionEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeAction {
    /// Draw `text` as an inline composition preview at the cursor cell, with the
    /// caret at `caret` (char index). The host threads this into the render tick
    /// as a [`crate::ime::CompositionOverlay`] handed to the renderer.
    RenderInline { text: String, caret: usize },
    /// Clear any inline composition preview (composition ended / committed /
    /// cancelled). The renderer draws no composition overlay next frame.
    ClearInline,
    /// Write this committed text to the PTY **as UTF-8, exactly once**.
    ///
    /// # Why this bypasses the key encoder
    ///
    /// Typed keys go `KeyEvent{key, text} → Encoder → bytes`. A committed IME
    /// string is **not a key**: there is no `Key` that produced it, it can be
    /// many graphemes (a kanji run, an emoji), and no encoder mode (Kitty,
    /// legacy, modified) may transform it — the bytes the app must receive are
    /// exactly the committed UTF-8. Routing it per-grapheme through the encoder
    /// would (a) invent a fake `Key::Char` per code point and (b) risk an
    /// encoding mode rewriting it. So the host writes these bytes straight to
    /// the same PTY input channel the encoder's output feeds (`INPUT_TX`).
    SendToPty(String),
}

/// State of the composition state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// No composition in flight.
    Idle,
    /// Composing: `text` is the current preview, `caret` its char-index caret.
    Composing { text: String, caret: usize },
}

/// The IME composition state machine. Platform-independent and fully testable
/// without any Win32 call.
///
/// Feed it [`CompositionEvent`]s in arrival order; it returns the [`ImeAction`]s
/// to apply. It guarantees:
///   * a commit emits exactly one [`ImeAction::SendToPty`] with the final string;
///   * focus loss mid-composition emits [`ImeAction::ClearInline`] and **no**
///     `SendToPty` (no partial bytes reach the PTY);
///   * a commit-then-new-composition sequence (result + comp in one message,
///     delivered as `Commit` then `Update`) sends the committed text once and
///     then re-enters preview for the new composition.
#[derive(Debug, Default)]
pub struct ImeSession {
    state: StateSlot,
}

/// Newtype so `Default` gives `Idle` (an enum can't derive a chosen default
/// without an attribute on the variant; keep the enum private + un-annotated).
#[derive(Debug)]
struct StateSlot(State);

impl Default for StateSlot {
    fn default() -> Self {
        StateSlot(State::Idle)
    }
}

impl ImeSession {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a composition preview is in flight (used by the Win32 layer to
    /// flag focus-loss-mid-composition and by diagnostics).
    #[must_use]
    pub fn is_composing(&self) -> bool {
        matches!(self.state.0, State::Composing { .. })
    }

    /// Feed one event; returns the actions to apply, in order.
    pub fn on_event(&mut self, ev: CompositionEvent) -> Vec<ImeAction> {
        match ev {
            CompositionEvent::Start => {
                // Enter composing with an empty preview. A lone Start draws
                // nothing yet (no text), but establishes the state so a later
                // FocusLost is recognised as mid-composition.
                self.state.0 = State::Composing {
                    text: String::new(),
                    caret: 0,
                };
                Vec::new()
            }
            CompositionEvent::Update { comp_text, caret } => {
                let caret = caret.min(comp_text.chars().count());
                self.state.0 = State::Composing {
                    text: comp_text.clone(),
                    caret,
                };
                if comp_text.is_empty() {
                    // Empty preview (e.g. the user deleted the whole reading):
                    // clear the inline overlay but stay composing.
                    vec![ImeAction::ClearInline]
                } else {
                    vec![ImeAction::RenderInline {
                        text: comp_text,
                        caret,
                    }]
                }
            }
            CompositionEvent::Commit { result_text } => {
                // Commit ends the preview: clear inline, then send exactly once.
                // Stay/return to Idle — a same-message GCS_COMPSTR restart arrives
                // as a following Update, which re-enters Composing.
                self.state.0 = State::Idle;
                if result_text.is_empty() {
                    // Defensive: a zero-length commit sends nothing.
                    vec![ImeAction::ClearInline]
                } else {
                    vec![ImeAction::ClearInline, ImeAction::SendToPty(result_text)]
                }
            }
            CompositionEvent::End => {
                // End after a commit: preview already cleared, nothing to send.
                // End after a cancel: likewise. Idempotent clear.
                let was_composing = self.is_composing();
                self.state.0 = State::Idle;
                if was_composing {
                    vec![ImeAction::ClearInline]
                } else {
                    Vec::new()
                }
            }
            CompositionEvent::FocusLost => {
                // Cancel: drop the preview with NO SendToPty. If we were not
                // composing, this is a no-op (no stray ClearInline needed, but
                // emitting one is harmless and keeps the overlay definitively
                // cleared on any focus loss).
                let was_composing = self.is_composing();
                self.state.0 = State::Idle;
                if was_composing {
                    vec![ImeAction::ClearInline]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

/// The inline composition overlay the renderer draws: the preview `text`, the
/// caret char index within it, and the origin cell `(col, row)` (the cursor cell
/// at compose time). Mirrors `term_render::CompositionOverlay`; the host converts
/// between them so `ime.rs` does not depend on renderer internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionOverlay {
    pub text: String,
    pub caret_idx: usize,
    pub origin_col: u16,
    pub origin_row: u16,
}

// ───────────────────────── Win32 message translation ─────────────────────────

#[cfg(windows)]
pub mod win32 {
    //! Translates raw `WM_IME_*` / focus messages into [`super::CompositionEvent`]s
    //! and owns candidate-window positioning + double-commit suppression.
    //!
    //! This layer holds **no** composition state — it only (a) reads the IMM
    //! composition strings out of the message, (b) positions the candidate list,
    //! and (c) arms/disarms the commit-swallow window that eats the redundant
    //! `WM_CHAR` / `WM_IME_CHAR` messages IMM emits after a `GCS_RESULTSTR`.

    use super::CompositionEvent;

    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::Input::Ime::{
        ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow,
        ImmSetCompositionWindow, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, GCS_COMPSTR,
        GCS_CURSORPOS, GCS_RESULTSTR, HIMC, IME_COMPOSITION_STRING, ISC_SHOWUICOMPOSITIONWINDOW,
    };

    /// IMM message ids the host matches on but which the `windows` binding does
    /// not surface as named constants in `WindowsAndMessaging` at this rev, so we
    /// define them here. (`WM_IME_STARTCOMPOSITION` / `_COMPOSITION` /
    /// `_ENDCOMPOSITION` and `WM_CHAR` / `WM_KILLFOCUS` *are* in that binding and
    /// the host uses those directly; only these two need a local definition.)
    pub const WM_IME_SETCONTEXT: u32 = 0x0281;
    pub const WM_IME_CHAR: u32 = 0x0286;

    /// A cursor pixel rectangle (top-left origin, device pixels) supplied by the
    /// host from `CellRenderer` metrics — used to place the candidate window just
    /// below the cursor cell so it does not cover the composition preview.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct CursorRect {
        pub x: i32,
        pub y: i32,
        pub w: i32,
        pub h: i32,
    }

    /// UTF-16 → `String`, correct for surrogate pairs (emoji, astral CJK). Never
    /// assumes UTF-16 unit count equals `char` count.
    fn utf16_to_string(units: &[u16]) -> String {
        String::from_utf16_lossy(units)
    }

    /// Read one composition string (`GCS_COMPSTR` or `GCS_RESULTSTR`) as UTF-8.
    ///
    /// # Safety
    /// `himc` must be a live input context obtained from `ImmGetContext` for the
    /// same `HWND` and not yet released.
    unsafe fn read_comp_string(himc: HIMC, which: IME_COMPOSITION_STRING) -> String {
        // First call with a null buffer returns the byte length of the string.
        let byte_len = unsafe { ImmGetCompositionStringW(himc, which, None, 0) };
        if byte_len <= 0 {
            return String::new();
        }
        let u16_len = (byte_len as usize) / std::mem::size_of::<u16>();
        let mut buf = vec![0u16; u16_len];
        let wrote = unsafe {
            ImmGetCompositionStringW(
                himc,
                which,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                byte_len as u32,
            )
        };
        if wrote <= 0 {
            return String::new();
        }
        let wrote_units = (wrote as usize) / std::mem::size_of::<u16>();
        buf.truncate(wrote_units.min(buf.len()));
        utf16_to_string(&buf)
    }

    /// Read the composition caret (`GCS_CURSORPOS`) as a **char** index within the
    /// current comp string. IMM reports it in UTF-16 units; convert to a char
    /// index so a surrogate pair before the caret does not offset it by two.
    ///
    /// # Safety
    /// See [`read_comp_string`].
    unsafe fn read_caret_chars(himc: HIMC, comp_text: &str) -> usize {
        let units = unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0) };
        // GCS_CURSORPOS returns the position directly (not a length), and can be
        // 0. A negative return means "unsupported": caret at end.
        if units < 0 {
            return comp_text.chars().count();
        }
        let units = units as usize;
        // Map a UTF-16 unit offset to a char index.
        let mut u16_seen = 0usize;
        for (char_idx, ch) in comp_text.chars().enumerate() {
            if u16_seen >= units {
                return char_idx;
            }
            u16_seen += ch.len_utf16();
        }
        comp_text.chars().count()
    }

    /// Parse a `WM_IME_COMPOSITION` into zero, one, or two [`CompositionEvent`]s.
    ///
    /// The classic subtlety: a single message can carry **both** `GCS_RESULTSTR`
    /// and `GCS_COMPSTR` (commit the finished text AND immediately start a new
    /// composition — common when an IME auto-commits on the next keystroke). We
    /// emit the `Commit` first, then the `Update`, so the state machine sends the
    /// old text before previewing the new. Returns them in apply order.
    ///
    /// # Safety
    /// `hwnd` must be the composition-owning window on the current thread.
    #[must_use]
    pub unsafe fn parse_composition(hwnd: HWND, lparam_flags: u32) -> Vec<CompositionEvent> {
        let mut out = Vec::new();
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.0.is_null() {
            return out;
        }
        if lparam_flags & GCS_RESULTSTR.0 != 0 {
            let result = unsafe { read_comp_string(himc, GCS_RESULTSTR) };
            if !result.is_empty() {
                out.push(CompositionEvent::Commit {
                    result_text: result,
                });
            }
        }
        if lparam_flags & GCS_COMPSTR.0 != 0 {
            let comp = unsafe { read_comp_string(himc, GCS_COMPSTR) };
            let caret = unsafe { read_caret_chars(himc, &comp) };
            out.push(CompositionEvent::Update {
                comp_text: comp,
                caret,
            });
        }
        unsafe {
            let _ = ImmReleaseContext(hwnd, himc);
        }
        out
    }

    /// Position the candidate + composition window at the cursor cell so the IME's
    /// candidate list appears just under our inline preview (we draw the preview;
    /// the candidate list still belongs to the IME UI).
    ///
    /// # Safety
    /// `hwnd` must be a valid window on the calling thread.
    pub unsafe fn position_candidate(hwnd: HWND, rect: CursorRect) {
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.0.is_null() {
            return;
        }
        // CFS_POINT anchors the composition window at the cursor cell's top-left.
        let comp_form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x: rect.x, y: rect.y },
            rcArea: RECT::default(),
        };
        // CFS_EXCLUDE keeps the candidate list clear of the cursor cell rectangle
        // so it never covers the character being composed.
        let cand_form = CANDIDATEFORM {
            dwIndex: 0,
            dwStyle: CFS_EXCLUDE,
            ptCurrentPos: POINT { x: rect.x, y: rect.y },
            rcArea: RECT {
                left: rect.x,
                top: rect.y,
                right: rect.x + rect.w,
                bottom: rect.y + rect.h,
            },
        };
        unsafe {
            let _ = ImmSetCompositionWindow(himc, &comp_form);
            let _ = ImmSetCandidateWindow(himc, &cand_form);
            let _ = ImmReleaseContext(hwnd, himc);
        }
    }

    /// Given the raw `WM_IME_SETCONTEXT` `lparam`, return the lparam to pass to
    /// `DefWindowProc` with the `ISC_SHOWUICOMPOSITIONWINDOW` bit stripped, so the
    /// system does not draw its own composition window over our inline preview.
    #[must_use]
    pub fn suppress_system_composition_window(lparam: isize) -> isize {
        (lparam as usize & !(ISC_SHOWUICOMPOSITIONWINDOW as usize)) as isize
    }

    // ── Double-commit suppression window ──────────────────────────────────────
    //
    // After a `GCS_RESULTSTR` commit, if `WM_IME_COMPOSITION` is passed to
    // `DefWindowProc`, IMM re-delivers the committed text as `WM_IME_CHAR` /
    // `WM_CHAR` messages — the classic IME double-commit. Our defence is twofold:
    //   1. We do NOT call `DefWindowProc` for a `WM_IME_COMPOSITION` we handled
    //      (we already extracted GCS_RESULTSTR ourselves), which stops IMM from
    //      synthesising the char messages in the first place for most IMEs.
    //   2. As belt-and-braces for IMEs/hosts that still post them, we arm a short
    //      suppression window on commit: while armed, `WM_CHAR` / `WM_IME_CHAR`
    //      matching the committed text (by remaining code-point count) are
    //      swallowed instead of re-sent to the PTY. The window disarms once the
    //      committed code points have been consumed (or on the next non-char
    //      message).
    //
    // The arming/counting logic is pure and unit-tested via [`CommitSwallow`].

    /// Tracks how many trailing char messages to swallow after a commit.
    #[derive(Debug, Default)]
    pub struct CommitSwallow {
        /// Remaining code points to swallow (0 = disarmed).
        remaining: usize,
    }

    impl CommitSwallow {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// Arm the window for a just-committed string: swallow one char message
        /// per code point of the commit.
        pub fn arm(&mut self, committed: &str) {
            self.remaining = committed.chars().count();
        }

        #[must_use]
        pub fn is_armed(&self) -> bool {
            self.remaining > 0
        }

        /// Offer a `WM_CHAR` / `WM_IME_CHAR` to the swallow window. Returns `true`
        /// if it was swallowed (caller must NOT forward it to the PTY). A
        /// surrogate-pair `WM_CHAR` arrives as two messages (high then low): each
        /// counts against one code point only when it completes a scalar, but for
        /// robustness we count every char message as consuming one slot — IMM
        /// posts one `WM_IME_CHAR` per committed *character* (already combined),
        /// and legacy `WM_CHAR` surrogate halves are rare on the commit path.
        pub fn offer(&mut self) -> bool {
            if self.remaining > 0 {
                self.remaining -= 1;
                true
            } else {
                false
            }
        }

        /// Disarm unconditionally (called on any non-char message that arrives
        /// while armed — the swallow window only spans the immediate post-commit
        /// char burst).
        pub fn disarm(&mut self) {
            self.remaining = 0;
        }
    }
}

// ───────────────────────── unit tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn update(text: &str, caret: usize) -> CompositionEvent {
        CompositionEvent::Update {
            comp_text: text.to_string(),
            caret,
        }
    }

    fn commit(text: &str) -> CompositionEvent {
        CompositionEvent::Commit {
            result_text: text.to_string(),
        }
    }

    /// start → update×3 → commit → end: exactly one SendToPty with the final
    /// string, and the preview is cleared.
    #[test]
    fn full_compose_commit_walk() {
        let mut s = ImeSession::new();
        assert!(s.on_event(CompositionEvent::Start).is_empty());
        assert_eq!(
            s.on_event(update("に", 1)),
            vec![ImeAction::RenderInline {
                text: "に".into(),
                caret: 1
            }]
        );
        assert_eq!(
            s.on_event(update("にほ", 2)),
            vec![ImeAction::RenderInline {
                text: "にほ".into(),
                caret: 2
            }]
        );
        assert_eq!(
            s.on_event(update("日本", 2)),
            vec![ImeAction::RenderInline {
                text: "日本".into(),
                caret: 2
            }]
        );
        // Commit emits ClearInline then exactly one SendToPty.
        assert_eq!(
            s.on_event(commit("日本")),
            vec![
                ImeAction::ClearInline,
                ImeAction::SendToPty("日本".into())
            ]
        );
        assert!(!s.is_composing());
        // Trailing End is a harmless no-op (not composing anymore).
        assert!(s.on_event(CompositionEvent::End).is_empty());
    }

    /// A commit produces the SendToPty exactly once across the whole sequence.
    #[test]
    fn commit_sends_exactly_once() {
        let mut s = ImeSession::new();
        s.on_event(CompositionEvent::Start);
        s.on_event(update("あ", 1));
        let mut sends = 0;
        for a in s.on_event(commit("亜")) {
            if let ImeAction::SendToPty(t) = a {
                assert_eq!(t, "亜");
                sends += 1;
            }
        }
        // End afterwards must not send again.
        for a in s.on_event(CompositionEvent::End) {
            assert!(!matches!(a, ImeAction::SendToPty(_)));
        }
        assert_eq!(sends, 1);
    }

    /// Focus loss mid-composition: ClearInline and NO SendToPty (no partial bytes
    /// to the PTY).
    #[test]
    fn focus_loss_cancels_no_send() {
        let mut s = ImeSession::new();
        s.on_event(CompositionEvent::Start);
        s.on_event(update("にほ", 2));
        let actions = s.on_event(CompositionEvent::FocusLost);
        assert_eq!(actions, vec![ImeAction::ClearInline]);
        assert!(!actions.iter().any(|a| matches!(a, ImeAction::SendToPty(_))));
        assert!(!s.is_composing());
    }

    /// Focus loss with no composition in flight is a clean no-op.
    #[test]
    fn focus_loss_idle_is_noop() {
        let mut s = ImeSession::new();
        assert!(s.on_event(CompositionEvent::FocusLost).is_empty());
    }

    /// Commit-then-new-composition delivered as Commit then Update in one burst:
    /// the old text is sent once, then the new preview re-enters Composing.
    #[test]
    fn commit_then_new_composition() {
        let mut s = ImeSession::new();
        s.on_event(CompositionEvent::Start);
        s.on_event(update("か", 1));
        // The IME auto-commits "可" and immediately starts composing "き".
        let commit_actions = s.on_event(commit("可"));
        assert_eq!(
            commit_actions,
            vec![ImeAction::ClearInline, ImeAction::SendToPty("可".into())]
        );
        assert!(!s.is_composing());
        let update_actions = s.on_event(update("き", 1));
        assert_eq!(
            update_actions,
            vec![ImeAction::RenderInline {
                text: "き".into(),
                caret: 1
            }]
        );
        assert!(s.is_composing());
    }

    /// Surrogate-pair / astral text round-trips through the state machine intact:
    /// the emoji and a CJK char both commit as their exact UTF-8.
    #[test]
    fn surrogate_pair_roundtrips() {
        for text in ["🎉", "你", "🎉你🎉"] {
            let mut s = ImeSession::new();
            s.on_event(CompositionEvent::Start);
            // caret in CHARS, not UTF-16 units.
            let n = text.chars().count();
            s.on_event(update(text, n));
            let actions = s.on_event(commit(text));
            assert!(
                actions.contains(&ImeAction::SendToPty(text.to_string())),
                "expected exact UTF-8 commit for {text:?}"
            );
        }
    }

    /// Emoji picker (Win+.) arrives on the same IMM commit path: a single commit
    /// event, one SendToPty, no preview assumptions.
    #[test]
    fn emoji_picker_single_commit() {
        let mut s = ImeSession::new();
        // Win+. typically commits without a visible multi-step preview.
        let actions = s.on_event(commit("😀"));
        assert_eq!(
            actions,
            vec![ImeAction::ClearInline, ImeAction::SendToPty("😀".into())]
        );
    }

    /// Empty comp update clears the inline overlay but stays composing.
    #[test]
    fn empty_update_clears_but_keeps_composing() {
        let mut s = ImeSession::new();
        s.on_event(CompositionEvent::Start);
        s.on_event(update("あ", 1));
        assert_eq!(s.on_event(update("", 0)), vec![ImeAction::ClearInline]);
        assert!(s.is_composing());
    }

    /// Caret past the end is clamped to the char length.
    #[test]
    fn caret_clamped_to_len() {
        let mut s = ImeSession::new();
        s.on_event(CompositionEvent::Start);
        let actions = s.on_event(update("ab", 99));
        assert_eq!(
            actions,
            vec![ImeAction::RenderInline {
                text: "ab".into(),
                caret: 2
            }]
        );
    }

    // ── Commit-swallow window (double-commit suppression) ──
    #[cfg(windows)]
    #[test]
    fn commit_swallow_window_arms_and_disarms() {
        use super::win32::CommitSwallow;
        let mut sw = CommitSwallow::new();
        assert!(!sw.is_armed());
        // Commit of a 3-code-point string arms 3 slots.
        sw.arm("abc");
        assert!(sw.is_armed());
        // Three char messages are swallowed…
        assert!(sw.offer());
        assert!(sw.offer());
        assert!(sw.offer());
        // …then the window is disarmed and further chars pass through.
        assert!(!sw.is_armed());
        assert!(!sw.offer());
    }

    #[cfg(windows)]
    #[test]
    fn commit_swallow_counts_code_points_not_utf16() {
        use super::win32::CommitSwallow;
        let mut sw = CommitSwallow::new();
        // One emoji = one code point (2 UTF-16 units) → swallow exactly one
        // WM_IME_CHAR (IMM posts one per committed character).
        sw.arm("🎉");
        assert!(sw.offer());
        assert!(!sw.is_armed());
    }

    #[cfg(windows)]
    #[test]
    fn commit_swallow_explicit_disarm() {
        use super::win32::CommitSwallow;
        let mut sw = CommitSwallow::new();
        sw.arm("abc");
        assert!(sw.is_armed());
        sw.disarm();
        assert!(!sw.is_armed());
        assert!(!sw.offer());
    }
}
