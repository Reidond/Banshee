# Tier-A shell — manual input matrix (D2 memo evidence)

Operator instructions for the four Gherkin scenarios in
`.specs/m0-seance/requirements.md` → *Feature: Tier-A shell keyboard focus and
text input*. This is the manual half of Task 7's acceptance ("manual matrix run
recorded in the D2 memo"). Every probe line is emitted to **stderr** with a
stable prefix and two timestamps (`t=<ms since start>`, `wall=<ms since epoch>`)
so runs can be grepped straight into the memo.

## Why this is a manual matrix (read first)

`windows-reactor` (rev `a4f7b2cb7c63c6bb7fc77a2affe57145be1d8c4f`) exposes **no
declarative keyboard / character / focus / IME callbacks** on elements — only
pointer events and `keyboard_accelerator` (a VirtualKey + modifier chord). The
raw `KeyDown` / `KeyUp` / `CharacterReceived` / `GotFocus` / `LostFocus` slots in
its `IUIElement` binding are unimplemented stubs, and there is **no TSF /
CoreTextEditContext surface at all**. So this spike observes input the way a real
terminal must anyway: it subclasses the host top-level `HWND` (located by window
title via `FindWindowW`) and logs the Win32 `WM_*` messages. The manual matrix
below drives real keyboard / IME / focus state and reads the resulting log.

> PTY wiring (keystroke → encoder → ConPTY → echo → grid) lands in **T10**. Until
> then the round-trip check is **"the expected input events are logged"**, not
> "characters appear in the grid". Each scenario below states its T0 (now) check
> and its T10 (later) check.

## Build & launch

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build -p app-shell
# Interactive (window stays open; run the matrix here):
./target/debug/app-shell.exe   2> probe.log
# Headless verification (5 s, exits 0 with a SELFTEST summary):
./target/debug/app-shell.exe --self-test
```

Grep tip (PowerShell): `Get-Content probe.log -Wait | Select-String 'PROBE'`.
Grep tip (bash): `tail -f probe.log | grep PROBE`.

The window titled **"Banshee M0 spike"** shows an animated colored 16×9 cell grid
(Mica backdrop). Click it once to give the grid surface focus before each
scenario.

---

## Scenario 1 — Plain typing round-trips through the PTY

**Setup:** window focused, grid surface active.
**Do:** type `echo hello`.

**Expected log (T0 — now):** one `CHAR` line per printable character, plus
`KEY down`/`KEY up` pairs. Example:

```
[PROBE KEY]  t=… vk=0x45 (A-Z) sys=false …      # 'e'
[PROBE CHAR] t=… U+0065 'e'
...
[PROBE CHAR] t=… U+0020 ' '                      # space
[PROBE CHAR] t=… U+0068 'h'                      # hello
```

**Grep:** `grep -E '\[PROBE (KEY|CHAR)\]' probe.log`
**Pass (T0):** ten `CHAR` events (`echo hello` = 4 + space + 5) in order.
**Pass (T10):** each keypress encoded & written to the PTY within one frame
budget, and `hello` echoes back into the rendered grid.

---

## Scenario 2 — AltGr is not misread as Ctrl+Alt

**Setup:** switch the active keyboard layout to **Ukrainian** or **German**
(Win+Space to cycle, or add the layout in Settings → Time & language → Language).
Focus the window.
**Do:** press an AltGr character combination — on German, `AltGr+Q` → `@`; on
Ukrainian, `AltGr`-level glyphs per that layout.

**Expected log:** AltGr is delivered by Windows as a synthetic
`Ctrl(0x11)`+`RMenu(0xA5)` pair followed by the **composed character**. The
critical evidence is that a `CHAR` for the layout's glyph is produced, and that
the app does not treat it as a Ctrl+Alt control chord:

```
[PROBE KEY]  t=… vk=0x11 (CONTROL) …
[PROBE KEY]  t=… vk=0xA5 (RMENU/AltGr) …
[PROBE CHAR] t=… U+0040 '@'        # the layout character, NOT a control byte
```

**Grep:** `grep -E '\[PROBE (KEY|CHAR)\]' probe.log`
**Pass (T0):** a `CHAR` line carrying the layout's glyph appears; there is a
`vk=0xA5 (RMENU/AltGr)` KEY line (right-Alt), confirming AltGr is distinguishable
from left-Alt.
**Pass (T10):** the encoder (T9) emits the layout character and sends **no**
`Ctrl+Alt`-modified escape sequence to the PTY. (This spike proves the raw
signal is present and distinguishable; the encoder decision is verified in T9's
golden rig + T10 integration.)

---

## Scenario 3 — IME composition commits exactly once

**Setup:** enable the **Japanese IME** (add Japanese language, switch input to
Hiragana). Focus the window.
**Do:** type a reading (e.g. `nihongo`), watch the candidate window, press
**Enter/Space** to commit the composition.

**Expected log:**

```
[PROBE IME_START]  t=… composition started
[PROBE IME_UPDATE] t=… composition updated (GCS flags=0x0008)   # per keystroke
...
[PROBE IME_COMMIT] t=… result string committed (GCS flags=0x0800)
[PROBE IME_END]    t=… composition ended
```

**Grep:** `grep -E '\[PROBE IME_' probe.log`
**Pass (T0):** exactly **one** `IME_COMMIT` line per committed string (the
`GCS_RESULTSTR` message fires once); `IME_UPDATE` lines may repeat during
composition but the commit is singular. No `CHAR` control bytes from the
composition UI appear between START and COMMIT.
**Pass (T10):** the committed text reaches the PTY as UTF-8 exactly once; no
composition-UI control bytes leak into the stream.

> **Known limitation to record in the memo:** because Reactor has no TSF surface,
> this spike observes IME at the **IMM32 `WM_IME_*`** level, not TSF. Production
> inline composition (underlined candidate at the cursor, SPEC §6.3) will require
> either a TSF text store the shell owns or an IMM32 path — neither is provided
> by Reactor. This is a first-class D2 finding, not a matrix failure.

---

## Scenario 4 — Focus loss mid-composition cancels cleanly

**Setup:** JA IME active, focus the window, **start a composition** (type a
reading but do NOT commit — leave the candidate window open).
**Do:** click another window (or Alt+Tab away) so the spike window loses focus.

**Expected log:**

```
[PROBE IME_START] t=… composition started
[PROBE FOCUS]     t=… lost (WM_KILLFOCUS) mid_composition=true
[PROBE IME_CANCEL] t=… focus lost while composing — expect composition cancelled
[PROBE IME_END]   t=… composition ended
```

**Grep:** `grep -E '\[PROBE (FOCUS|IME_)' probe.log`
**Pass (T0):** the `FOCUS lost … mid_composition=true` line is followed by an
`IME_END`; re-focusing and typing produces fresh `CHAR`/`IME_START` events with
no residual composition state (no orphan `IME_UPDATE` before a new `IME_START`).
**Pass (T10):** the composition is cancelled with no residual bytes sent to the
PTY; subsequent typing behaves as if no composition had started.

---

## Focus round-trip sanity (any layout)

Click the window, then Alt+Tab away and back:

```
[PROBE FOCUS] t=… gained (WM_SETFOCUS)
[PROBE FOCUS] t=… lost (WM_KILLFOCUS) mid_composition=false
[PROBE FOCUS] t=… gained (WM_SETFOCUS)
```

**Grep:** `grep '\[PROBE FOCUS\]' probe.log`

## Reactor-native accelerator check

Press **Ctrl+K** with the window focused:

```
[PROBE KEY] t=… reactor keyboard_accelerator Ctrl+K invoked
```

This confirms the ONE declarative keyboard surface Reactor does expose fires,
for comparison against the WM_-level raw keys above.

## Recording into the memo

After a matrix run, attach the filtered log to `.specs/m0-seance/d2-memo.md`:

```bash
grep -E '\[PROBE (KEY|CHAR|IME_|FOCUS)\]' probe.log > d2-input-evidence.txt
```

Each scenario's pass/fail is decided by the greps above; paste the grep output
under the "Keyboard focus + TSF" criterion of the D2 decision table.

---

# M1 IME composition matrix (Task 7 — design risk R3)

The M0 matrix above only *observed* `WM_IME_*` at the log level. **M1 Task 7
integrates** it: `crates/app-shell/src/ime.rs` drives an `ImeSession` state
machine off parsed composition events, renders the in-flight composition inline
at the cursor with a distinct underline (`term-render` composition overlay pass),
and commits UTF-8 straight to the PTY exactly once.

## Why this is still a manual matrix (read first)

TSF automation is unreliable and there is **no TSF-enabled surface** in the
reactor XAML tree (the M0 re-baseline confirmed the keyboard/char/focus/IME slots
are vtable stubs). So M1 rides the **IMM32 composition-message path** on the host
`HWND`: `WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION` (with `GCS_COMPSTR` for
the preview and `GCS_RESULTSTR` for the commit) / `WM_IME_ENDCOMPOSITION`, plus
`WM_IME_SETCONTEXT` handling to request suppression of the system composition
window. IMM32 rides on top of TSF for every shipping Win11 IME, so this is the
standard fallback for exactly this "no cooperating text store" situation. The
composition **state machine** (`ImeSession`) is unit-tested headlessly
(`cargo test -p app-shell`); this matrix covers the parts only a human at a real
IME can verify: correct inline rendering, single commit, layout switching, emoji
picker, and clean focus-loss cancel.

### Architecture the operator is verifying

- **Message path:** the M0 thread-scoped hooks (`WH_GETMESSAGE` + `WH_CALLWNDPROC`
  on the UI thread) now *handle* IME messages, not just log them. `WM_IME_*` →
  `ime::win32::parse_composition` (reads `ImmGetCompositionStringW`) →
  `CompositionEvent` → `ImeSession` → `ImeAction`.
- **Inline render:** `RenderInline` publishes a `CompositionOverlay` (preview text
  + caret + cursor origin cell) that the render tick threads into
  `CellRenderer::render_snapshot(.., composition, ..)`; the renderer draws a bg
  mask + the composition glyphs + a distinct thicker underline over the cursor
  cell.
- **Commit path:** `SendToPty(text)` writes the committed UTF-8 **directly** to
  `INPUT_TX` (the same channel typed keys feed), bypassing the key encoder — a
  commit is not a "key" and no encoding mode may transform it.
- **Double-commit guard (two parts):** (1) the hook *handles*
  `WM_IME_COMPOSITION` and does not pass it to `DefWindowProc`, so most IMEs do
  not synthesise the redundant char echo; (2) belt-and-braces, a **commit-swallow
  window** is armed on commit (`CommitSwallow`, one slot per committed code
  point), and while armed the `getmessage_hook` rewrites the redundant
  `WM_CHAR` / `WM_IME_CHAR` to `WM_NULL` so committed text never reaches the PTY
  twice. A real keydown disarms the window.
- **Surrogate pairs:** UTF-16 composition strings are converted with
  `from_utf16_lossy` and the caret is mapped from UTF-16 units to a `char` index;
  the code never assumes UTF-16 unit count == char count (emoji, astral CJK).

### Recorded-run columns

Fill in **Observed** / **Date & build** at run time. Build id: paste the first
`[BANSHEE-M1]` startup line and the `git rev-parse --short HEAD`.

### Automation status (tests/live_input_matrix.rs — run `scripts/live-matrix.ps1`)

The Banshee-side delivery contract for these scenarios is covered by the
automated live-input matrix (focus-free: it posts the exact WM_CHAR /
WM_MOUSEWHEEL messages Windows delivers). Only the OS-side conversion UI
remains human-verified:

| Scenario | Status | Automated by |
|---|---|---|
| M1-IME-1 JA composition | **MANUAL** (needs real IME conversion UI) — commit/swallow path unit-tested in `ime.rs` | — |
| M1-IME-2 ZH pinyin | **MANUAL** (same reason) | — |
| M1-IME-3 UA/RU mid-line switch | **AUTOMATED** (byte-level: Cyrillic WM_CHARs mid-line round-trip) — visual spot-check optional | `cyrillic_mid_line_roundtrip` |
| M1-IME-4 Win+. emoji | **AUTOMATED** (delivery path: surrogate-pair WM_CHARs → one UTF-8 sequence); opening the picker UI itself is manual-optional | `emoji_surrogate_pair_single_sequence` |
| M1-IME-5 focus-loss cancel | **PARTIAL** — state machine unit-tested (`ime.rs`); real-IME cancel needs a human once | unit tests + manual |
| M1-IME-6 PSReadLine interplay | **AUTOMATED** (real profile, realistic typing cadence, garbling assert) | `psreadline_profile_no_garbling` |
| Wheel scrollback (requirements) | **AUTOMATED** (enter/pin/return) | `wheel_scrollback_pin_and_return` |

> These tests already caught two shipped bugs on first run: WM_CHAR surrogate
> halves were dropped (emoji never reached the PTY) and snapshots read the
> ACTIVE area so wheel scrollback was invisible on screen. Keep them in the
> release gate.

## Scenario M1-IME-1 — JA romaji → kanji commits once, rendered inline

**Setup:** enable the Japanese IME (Hiragana input). Focus the window; ensure a
pwsh prompt is visible.
**Steps:**
1. Type `nihongo`.
2. Press Space to convert to candidates; pick 日本語.
3. Press Enter to commit.

**Expected:**
- During typing: the composition (にほんご → 日本語) renders **inline at the
  cursor with an underline** (not in a floating box at screen origin). Logs show
  `IME_START` then repeated `IME_UPDATE (GCS flags=0x0008)`.
- On commit: exactly **one** `IME_COMMIT (GCS flags=0x0800)`; `日本語` appears at
  the prompt exactly once; **no** duplicate characters.
- Grep: `grep -E '\[PROBE IME_' probe.log` shows one `IME_COMMIT` per commit;
  `grep 'SWALLOWED' probe.log` may show swallowed post-commit chars (guard fired)
  or nothing (IME did not re-echo) — both are pass.

**Observed:** _______   **Date & build:** _______

## Scenario M1-IME-2 — ZH pinyin composition

**Setup:** enable Chinese (Simplified) Pinyin IME. Focus the window.
**Steps:**
1. Type `nihao`.
2. Select 你好 from candidates; commit.

**Expected:** inline underlined preview during composition; single `IME_COMMIT`;
你好 (two wide cells each) at the prompt exactly once. Wide-cell span of the
inline preview underline covers 4 columns.

**Observed:** _______   **Date & build:** _______

## Scenario M1-IME-3 — UA/RU layout switch mid-line drops/duplicates nothing

**Setup:** add a Cyrillic layout (Ukrainian or Russian) alongside the Latin one.
Focus the window.
**Steps:**
1. Type `hello ` (Latin) at the prompt — do NOT press Enter.
2. Switch layout (Win+Space or Alt+Shift) to Cyrillic.
3. Continue typing `привіт`.

**Expected:** the line reads `hello привіт` with **no dropped and no duplicated**
characters at the switch boundary. (Direct-keyboard Cyrillic arrives via
`WM_CHAR`, not composition; the check is that the switch does not corrupt the
byte stream — the encoder path and the IME path do not interfere.)

**Observed:** _______   **Date & build:** _______

## Scenario M1-IME-4 — Emoji picker (Win+.) arrives as one UTF-8 sequence

**Setup:** focus the window at a pwsh prompt.
**Steps:**
1. Press **Win+.** to open the emoji picker.
2. Pick an emoji that is a surrogate pair (e.g. 🎉 or 😀) and, ideally, one that
   is a ZWJ sequence (e.g. 👨‍💻) to stress multi-scalar commits.
3. Confirm.

**Expected:** the emoji reaches the prompt as a **single UTF-8 sequence** (one
insert, correct glyph — not two tofu boxes, not a doubled emoji). Win+. commits
via the same IMM path, so `IME_COMMIT` fires once; the swallow guard prevents any
`WM_CHAR` surrogate-half re-echo. Verify the byte count downstream matches the
emoji's UTF-8 length (no truncated surrogate).

**Observed:** _______   **Date & build:** _______

## Scenario M1-IME-5 — Focus loss mid-composition cancels cleanly, no partial bytes

**Setup:** JA (or ZH) IME active; focus the window.
**Steps:**
1. Start a composition (type a reading) but do **not** commit — leave candidates
   open.
2. Click another window / Alt+Tab away.

**Expected:**
- Logs: `FOCUS lost (WM_KILLFOCUS) mid_composition=true` followed by `IME_CANCEL`
  and (from the IME) `IME_END`.
- The inline composition preview **disappears** (ClearInline).
- **No partial composition bytes reach the PTY** — the prompt shows nothing from
  the abandoned reading; the state machine emits no `SendToPty` on `FocusLost`
  (verified by the `focus_loss_cancels_no_send` unit test; confirm at the prompt
  that no stray characters were inserted).
- Re-focus and type: fresh `IME_START` with no residual/orphan `IME_UPDATE`.

**Observed:** _______   **Date & build:** _______

## Scenario M1-IME-6 — PSReadLine interaction (no clean-echo assumption)

**Setup:** run the operator's **real** pwsh profile (with PSReadLine loaded — NOT
`-NoProfile`). Focus the window.
**Steps:**
1. With a JA/ZH IME, compose and commit text into the middle of an existing
   command line (e.g. after typing `git commit -m "`, compose Japanese, then
   continue).
2. Use Left/Right arrows and Home/End around the committed IME text.

**Expected:** committed IME text integrates with PSReadLine's redraw — real
profiles **interleave SGR / cursor-move / line-redraw sequences with echo**, so
the committed bytes must survive PSReadLine reprinting the line. No duplicated
IME text after a redraw; no composition-underline artifact left behind after
commit (the overlay clears on `IME_END`). This is the scenario the spike's
`-NoProfile` echo self-test cannot cover — the input path must not assume the
shell echoes committed text back on a clean, un-decorated line.

**Observed:** _______   **Date & build:** _______

## Recording into the memo

```bash
grep -E '\[PROBE (IME_|FOCUS|CHAR)\]|SWALLOWED' probe.log > m1-ime-evidence.txt
```

Paste under the IME criterion of the M1 exit checklist. The state-machine half is
covered green by `cargo test -p app-shell` (12 tests incl. commit-once,
focus-cancel-no-send, surrogate round-trip, commit-swallow window); this matrix
records the human-driven half.
