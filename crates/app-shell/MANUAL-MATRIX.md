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
