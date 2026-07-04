# Banshee configuration reference (v0)

Banshee reads its configuration from a single TOML file:

```
%APPDATA%\banshee\config.toml
```

If the file is missing, Banshee starts with the defaults documented below —
a missing config file is not an error. The file is watched while Banshee
runs; saving a change applies it to the running window within about a
second (see "Hot reload" below). If a saved file fails to parse or fails
validation, the **previous valid configuration stays in effect** and a
diagnostic is surfaced — a bad edit can never crash or blank a running
session.

## Naming posture (Ghostty inspiration, not compatibility)

Where a Banshee config concept matches one from [Ghostty](https://ghostty.org),
this file adopts Ghostty's kebab-case vocabulary (`font-family`, `theme`,
`window-padding-x`, ...). **This is inspiration, not a compatibility
promise**: Banshee does not read Ghostty config files, does not guarantee
identical semantics for same-named keys, and may diverge as Banshee's own
needs require. Do not assume a Ghostty config will work unmodified in
Banshee, or vice versa.

## Hot reload

- Banshee watches the **directory** containing `config.toml` (not just the
  file itself), so editor save patterns that write a temp file and rename it
  over the target (atomic save) are picked up correctly, in addition to
  direct in-place writes.
- Changes are debounced (~100 ms) so a single save that produces several
  filesystem events collapses into one reload.
- **Malformed TOML**: the file is rejected outright; the previous valid
  config remains active; a diagnostic reports the parse error's line and
  column and a message.
- **Unknown keys**: a key Banshee doesn't recognize produces a *warning*
  diagnostic naming the key, but does not reject the file — all recognized
  keys in the same file still apply.
- **Partial validity is not a thing**: a saved file either parses and
  validates as a whole (and is applied, possibly with warnings), or it is
  rejected as a whole and the last-good config is kept. There is no
  "apply the parts that were fine" behavior.

## Keys

### Font

| Key | Type | Default | Description |
|---|---|---|---|
| `font-family` | string | `"Cascadia Mono"` | Font family name resolved via DirectWrite family enumeration. |
| `font-size` | float | `12.0` | Point size. Must be in the range `4.0..=128.0`; out-of-range values are rejected (whole-file error). |

```toml
font-family = "JetBrains Mono"
font-size = 13.0
```

### Colors

| Key | Type | Default | Description |
|---|---|---|---|
| `background` | string (`#rrggbb`) | `"#0c0c0c"` | Default pane background color. |
| `foreground` | string (`#rrggbb`) | `"#cccccc"` | Default pane text color. |
| `palette` | array of 16 `#rrggbb` strings | xterm-like 16-color default | ANSI colors 0–15, in order (black, red, green, yellow, blue, magenta, cyan, white, then bright variants). Must have exactly 16 entries if specified at all. |

```toml
background = "#101010"
foreground = "#eeeeee"
palette = [
  "#0c0c0c", "#c50f1f", "#13a10e", "#c19c00",
  "#0037da", "#881798", "#3a96dd", "#cccccc",
  "#767676", "#e74856", "#16c60c", "#f9f1a5",
  "#3b78ff", "#b4009e", "#61d6d6", "#f2f2f2",
]
```

### Scrollback

| Key | Type | Default | Description |
|---|---|---|---|
| `scrollback-limit` | integer (bytes) | `12000000` | **This is a byte budget, not a line count.** libghostty-vt evicts scrollback by page, not by line, so retention is governed by memory, not line count. The default (~12 MB) retains roughly 10.9k 80-column lines — chosen empirically during M1 Task 3 to comfortably clear the "≥10k lines retrievable" requirement with headroom under the 80 MB idle-memory budget. |

```toml
scrollback-limit = 20000000
```

### Keybinds

`[keybinds]` maps a chord string to an action name. Chords are written as
modifier names joined with `+` and a final key, e.g. `"ctrl+shift+c"`.
Modifier order in the string doesn't matter (`"ctrl+shift+c"` and
`"shift+ctrl+c"` are the same chord). Recognized modifiers: `ctrl`
(`control` also accepted), `alt`, `shift`.

This crate only **parses and validates** the chord → action mapping into a
typed map; executing the action is the shell's responsibility.

| Action | Description |
|---|---|
| `copy` | Copy the current selection to the clipboard. |
| `paste` | Paste clipboard contents. |
| `scroll-to-bottom` | Reset the viewport to the live bottom of the buffer. |

Defaults (used for any chord not overridden):

```toml
[keybinds]
"ctrl+shift+c" = "copy"
"ctrl+shift+v" = "paste"
```

Example overriding and adding a binding:

```toml
[keybinds]
"ctrl+shift+c" = "copy"
"ctrl+shift+v" = "paste"
"ctrl+shift+b" = "scroll-to-bottom"
```

### Clipboard / OSC 52

| Key | Type | Default | Description |
|---|---|---|---|
| `clipboard-read` | string (`"deny"` \| `"allow"`) | `"deny"` | Whether an OSC 52 clipboard **read** request from the running application is honored. Denied by default for security (SPEC §8) — a malicious or careless program in the terminal cannot silently read your clipboard unless you opt in. |
| `clipboard-write-max-bytes` | integer (bytes) | `1000000` | Cap on the size of an OSC 52 clipboard **write** the application may perform; oversized writes are truncated at this size, not rejected. |

```toml
clipboard-read = "allow"
clipboard-write-max-bytes = 2000000
```

### Profiles

`[[profile]]` is an array of tables, each describing a launchable session
type (a shell configuration Banshee can open a pane with). This crate
defines and validates the schema shape; consuming it into an actual runnable
profile list (with built-in defaults layered underneath) is the `layout`
crate's job (M1 Task 9, see `layout::profile`).

Built-in profiles (`pwsh`, `Windows PowerShell`, `cmd`) always exist even
with no config file. A user `[[profile]]` entry whose `name` matches a
built-in (or an auto-discovered profile from a later source, e.g. WSL
distro detection in M1 Task 10) **replaces that profile wholesale** —
override is whole-profile, not field-by-field, because this crate's
`Profile` model has already resolved every field to a concrete value by
the time `layout` sees it, so there is no reliable way to tell "user left
this field unset" apart from "user explicitly set it to the default
value." See `layout::profile` rustdoc for the full merge algorithm.

| Key | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Display name; must be non-empty. |
| `command` | string | yes | Executable to launch; must be non-empty. |
| `args` | array of strings | no (default `[]`) | Command-line arguments. |
| `cwd` | string | no | Initial working directory. |
| `env` | table of string → string | no (default `{}`) | Extra environment variables merged into the sanitized session environment. |
| `type` | string (`"windows"` \| `"wsl"`) | yes | Session kind — controls launch mechanics (direct ConPTY spawn vs. `wsl.exe`). |
| `icon` | string | no | Optional icon identifier/path (interpretation is the shell's job). |
| `color` | string (`#rrggbb`) | no | Optional accent color override for this profile. |
| `font-size` | float | no | Optional per-profile font size override; same `4.0..=128.0` bounds as the top-level `font-size`. |
| `theme` | string | no | Optional per-profile theme name override (theme resolution is out of scope for config v0). |
| `default` | boolean | no (default `false`) | Marks this profile as the default (selected first). If multiple profiles set this, the first in declaration order wins; see `layout::ProfileSet::default_profile` (M1 Task 9). |

```toml
[[profile]]
name = "WSL Ubuntu"
command = "wsl.exe"
args = ["-d", "Ubuntu"]
cwd = "/home/user"
type = "wsl"

[profile.env]
FOO = "bar"

[[profile]]
name = "PowerShell (large font)"
command = "pwsh.exe"
type = "windows"
font-size = 16.0
```

## Diagnostics

Every load attempt (startup or reload) produces a list of diagnostics, each
with:

- **severity** — `Error` (file rejected, last-good retained) or `Warning`
  (file applied, but something deserves attention, e.g. an unknown key)
- **message** — human-readable description
- **span** — `(line, column)` into the source file, when the underlying TOML
  parser could locate one (always present for syntax errors; not applicable
  to semantic warnings like "unknown key")
- **key** — the config key the diagnostic concerns, when applicable

These are available programmatically via `ConfigService::diagnostics()` for
the app shell to surface to the user (e.g. a status-bar indicator or
notification) — this crate does not render UI itself.
