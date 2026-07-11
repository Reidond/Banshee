# vtebench throughput harness

`scripts/vtebench.ps1` runs the `scrolling`, `dense-cells`, and `unicode`
streams inside a terminal, three times each, and reports host-measured wall-time
medians. Each invocation has a hard 120-second timeout and never depends on
window focus.

The upstream vtebench corpus uses POSIX shell generators. The Windows harness
builds equivalent native temporary generators, then asks vtebench to emit at
least 1 MiB with one internal warmup and one measured sample. The reported wall
time starts immediately before terminal launch and stops when the in-terminal
vtebench process writes its completion marker. For Banshee, the run is accepted
only after `BANSHEE_DEBUG_DUMP_GRID` also shows the rendered session-death
banner. Thus the marker provides precise timing while the banner independently
proves that the configured session ran and exited.

## Banshee

Build release first, then run from the repository root:

```powershell
$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build --release -p app-shell
pwsh -NoLogo -NoProfile -File scripts/vtebench.ps1
```

The harness looks for `scripts/vtebench.exe` first and then
`$HOME/.cargo/bin/vtebench.exe`. If neither exists, reinstall it with:

```powershell
cargo install --git https://github.com/alacritty/vtebench
```

## winghostty comparison

Install or unpack winghostty, close any existing winghostty instance (its
single-instance routing would make process ownership ambiguous), and pass the
binary explicitly:

```powershell
pwsh -NoLogo -NoProfile -File scripts/vtebench.ps1 `
  -Terminal C:\path\to\winghostty.exe
```

For winghostty the harness uses a temporary `XDG_CONFIG_HOME` (documented by
[winghostty's Windows runtime](https://github.com/amanthanvi/winghostty/blob/main/docs/windows.md#paths))
to select the same bare PowerShell runner; it does not touch
`%LOCALAPPDATA%\winghostty\config.ghostty`. Completion requires both the marker
and terminal-process exit. Run Banshee and winghostty on the same machine with
the same display state and no competing load, then compute for each scenario:

```text
Banshee median / winghostty median
```

The M1 NFR passes only if every recorded ratio is at most `1.5`. vtebench
measures PTY-read throughput; it does not measure latency, frame pacing, or
photon time, so keep those conclusions separate.
