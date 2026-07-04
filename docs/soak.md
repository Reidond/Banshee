# Soak harness (Task 13)

`scripts/soak.ps1` launches `app-shell.exe`, samples its Private Bytes and
handle count every `-SampleSeconds` (default 60) into a CSV, runs for
`-DurationMinutes` (default 1440 = 24h) or until Ctrl+C, and prints a
linear-regression memory-slope verdict at the end.

## Busy pane ("top" requirement)

SPEC S11 asks for a `top`-equivalent busy pane so the render/vt pipeline gets
continuous scrollback churn during the soak. This machine's WSL Ubuntu distro
ships real `top` (verified via `wsl.exe -e which top` → `/usr/bin/top`), so the
default/recommended busy command, typed into the app-shell window once it's
up, is:

```
wsl.exe top -b -d 1
```

(`-b` batch mode, `-d 1` one-second refresh.) If WSL isn't available on the run
machine, use the pure-pwsh equivalent instead:

```powershell
while ($true) { ps | select -First 20; sleep 1 }
```

The script's `-AutoType` switch best-effort injects the WSL `top` command via
`SendKeys` (needs the window focused/frontmost — unreliable from a
non-interactive/agent shell); otherwise the operator types it by hand once the
window appears.

## Threshold rationale

Default gate: **500 KB/h**. A leak-free terminal's steady-state RSS should be
flat (bounded scrollback ring, bounded glyph atlas, no unbounded per-frame
allocation) — real drift should read as noise (page faults, allocator
fragmentation), not a trend. 500 KB/h over a 24h soak is 12 MB/day: small
enough to catch a slow leak, large enough not to false-positive on
ASLR/fragmentation noise at 60 s sampling. Tune down once a real 24h baseline
exists — this is a Task 13 starting gate, not a tuned production SLO.

## Validation run vs. the real soak

The 24-hour run is an **operator item**, not something an agent session should
launch. A short validation run (e.g. `-DurationMinutes 2 -SampleSeconds 10`)
proves the harness (CSV shape, sampling, regression, verdict line) but its
slope number is **not a meaningful leak signal** — a 1-2 minute window is
dominated by process-startup transients (glyph atlas warmup, font-cache fill,
first-paint allocations), which shows up as a large positive slope that says
nothing about steady-state behavior. Expect (and ignore) a `SOAK FAIL` from a
sub-5-minute validation run; only trust the verdict from a run long enough
that the startup transient is a negligible fraction of the total window (a few
hours at minimum, 24h for the real gate).

## Running

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build -p app-shell --release
.\scripts\soak.ps1                                   # full 24h (operator)
.\scripts\soak.ps1 -DurationMinutes 2 -SampleSeconds 10 -AutoType   # quick harness check
```

Output CSV: `soak-results\soak-<timestamp>.csv` (git-ignored — add
`soak-results/` to `.gitignore` if it isn't already, this doc does not modify
`.gitignore` itself).
