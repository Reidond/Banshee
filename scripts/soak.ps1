#requires -Version 5.1
<#
.SYNOPSIS
    Banshee M1 Task 13 — soak harness. Launches app-shell with a busy pwsh
    session and samples its memory/handle footprint over time, verdicting on a
    linear-regression memory slope at the end.

.DESCRIPTION
    SPEC S11 asks for a soak run with a `top`-equivalent busy pane. This script:

      1. Launches `app-shell.exe` (built via `cargo build -p app-shell
         --release` unless -ExePath is given) with NO special flags — the real
         interactive window, real ConPTY session, default profile.
      2. Waits for the window + PTY session to come up, then injects a busy
         loop into the pane via the Task-13 debug input mechanism: since
         app-shell has no scripted-stdin mode, the busy command is injected the
         same way a human would type it — this script does NOT send synthetic
         keystrokes (that's the E2E test's job); instead the OPERATOR types (or
         this script types via SendKeys, see -AutoType) the busy command once
         the window is focused. See "top choice" below for what to type.
      3. Every `-SampleSeconds` (default 60), samples the app-shell process's
         Private Bytes (working-set proxy for RSS) and handle count via
         `Get-Process -Id`, appending a row to a CSV.
      4. Runs for `-DurationMinutes` (default 1440 = 24h) or until Ctrl+C.
      5. On completion (normal, Ctrl+C, or duration elapsed), computes an
         ordinary-least-squares linear regression of Private Bytes (KB) over
         elapsed minutes, and prints:
           SOAK PASS: slope <= <threshold> KB/h (observed <slope> KB/h)
         or
           SOAK FAIL: slope > <threshold> KB/h (observed <slope> KB/h)

.PARAMETER top choice
    The spec literally asks for a `top`-equivalent busy pane. This machine's
    WSL Ubuntu distro ships real `top` (verified: `wsl.exe -e which top` ->
    /usr/bin/top), so the documented/recommended busy command is:

        wsl.exe top -b -d 1

    (`-b` batch mode so it does not need a real tty redraw, `-d 1` one-second
    refresh — matches the CSV sample cadence's spirit even though `top`'s
    redraw and this script's sampling are independent loops). If WSL is
    unavailable on the run machine, fall back to the pure-pwsh equivalent:

        while ($true) { ps | select -First 20; sleep 1 }

    Either way, the busy pane exists to give the render/vt pipeline continuous
    scrollback churn during the soak — the memory metric under test is
    app-shell's own process, not the child shell's.

.PARAMETER DurationMinutes
    Total soak duration in minutes. Default 1440 (24 hours). The 24h run is an
    OPERATOR item (not run by an agent) — use a short value (e.g. 2) for a
    quick validation run.

.PARAMETER SampleSeconds
    Seconds between samples. Default 60.

.PARAMETER ExePath
    Path to app-shell.exe. Defaults to target\release\app-shell.exe, falling
    back to target\debug\app-shell.exe if the release build is absent.

.PARAMETER OutCsv
    CSV output path. Defaults to soak-results\soak-<timestamp>.csv.

.PARAMETER ThresholdKbPerHour
    Slope threshold for PASS/FAIL, in KB/hour. Default 500 (see "threshold
    rationale" below).

.PARAMETER AutoType
    If set, the script uses SendKeys to type the busy command into the
    app-shell window once it's focused, instead of waiting for the operator to
    type it. Best-effort only (SendKeys needs the window focused and frontmost;
    unreliable from a non-interactive/agent shell — the DoD-level validation
    run in this repo's history was done by launching the window and typing the
    command by hand, see docs/soak.md).

.PARAMETER threshold rationale
    500 KB/h is a conservative gate: a genuinely leak-free terminal's steady
    -state RSS should be flat (bounded scrollback ring buffer, bounded glyph
    atlas, no unbounded per-frame allocation) — real drift should be noise
    (page-fault jitter, allocator fragmentation), not a trend. 500 KB/h over a
    24h soak is 12 MB/day: small enough to catch a slow leak, large enough to
    not false-positive on ASLR/heap-fragmentation noise at 60 s sampling
    granularity. Tune down once a real 24h baseline exists (this is a Task 13
    starting gate, not a tuned production SLO).

.EXAMPLE
    # 2-minute validation run (this is what an agent/CI would run; NOT the
    # 24h soak, which is an operator-run item):
    .\scripts\soak.ps1 -DurationMinutes 2 -SampleSeconds 10

.EXAMPLE
    # Full 24h soak (operator item):
    .\scripts\soak.ps1
#>
[CmdletBinding()]
param(
    [int]$DurationMinutes = 1440,
    [int]$SampleSeconds = 60,
    [string]$ExePath,
    [string]$OutCsv,
    [double]$ThresholdKbPerHour = 500,
    [switch]$AutoType
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot

if (-not $ExePath) {
    $release = Join-Path $RepoRoot 'target\release\app-shell.exe'
    $debug = Join-Path $RepoRoot 'target\debug\app-shell.exe'
    if (Test-Path $release) {
        $ExePath = $release
    } elseif (Test-Path $debug) {
        $ExePath = $debug
        Write-Warning "Using debug build ($debug) — memory numbers from a debug build are not representative of release steady-state. Build with 'cargo build -p app-shell --release' for a real soak."
    } else {
        throw "app-shell.exe not found at '$release' or '$debug'. Build it first: cargo build -p app-shell --release"
    }
}

if (-not $OutCsv) {
    $resultsDir = Join-Path $RepoRoot 'soak-results'
    New-Item -ItemType Directory -Force -Path $resultsDir | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $OutCsv = Join-Path $resultsDir "soak-$stamp.csv"
}

Write-Host "Banshee soak harness"
Write-Host "  exe:              $ExePath"
Write-Host "  duration:         $DurationMinutes min"
Write-Host "  sample interval:  $SampleSeconds s"
Write-Host "  threshold:        $ThresholdKbPerHour KB/h"
Write-Host "  csv:              $OutCsv"
Write-Host ""

# ── Launch ──
$proc = Start-Process -FilePath $ExePath -PassThru
Start-Sleep -Seconds 3

if (-not (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)) {
    throw "app-shell exited immediately after launch (pid $($proc.Id)) — check it starts standalone."
}

Write-Host "app-shell launched (pid=$($proc.Id))."

if ($AutoType) {
    Write-Host "Attempting SendKeys busy-pane injection (best-effort; see -AutoType doc)..."
    try {
        Add-Type -AssemblyName Microsoft.VisualBasic
        Add-Type -AssemblyName System.Windows.Forms
        [Microsoft.VisualBasic.Interaction]::AppActivate($proc.Id) | Out-Null
        Start-Sleep -Milliseconds 500
        [System.Windows.Forms.SendKeys]::SendWait('wsl.exe top -b -d 1{ENTER}')
        Write-Host "Sent 'wsl.exe top -b -d 1' + Enter via SendKeys."
    } catch {
        Write-Warning "SendKeys injection failed ($_); type the busy command into the app-shell window by hand: wsl.exe top -b -d 1"
    }
} else {
    Write-Host ""
    Write-Host "ACTION NEEDED: click the app-shell window and type the busy command, then press Enter:"
    Write-Host "    wsl.exe top -b -d 1"
    Write-Host "(falls back to a pwsh loop if WSL is unavailable on this machine: while (`$true) { ps | select -First 20; sleep 1 })"
    Write-Host ""
}

# ── Sample loop ──
'TimestampUtc,ElapsedMinutes,PrivateBytesKB,HandleCount' | Set-Content -Path $OutCsv -Encoding utf8

$samples = New-Object System.Collections.Generic.List[object]
$start = Get-Date
$deadline = $start.AddMinutes($DurationMinutes)
$stopRequested = $false

# Ctrl+C: PowerShell raises a terminating pipeline stop; wrap the loop in
# try/finally so we always compute + print the verdict even on early Ctrl+C.
try {
    while ((Get-Date) -lt $deadline) {
        $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
        if (-not $p) {
            Write-Warning "app-shell (pid $($proc.Id)) exited early at $(Get-Date -Format 'o') — ending soak early."
            break
        }

        $elapsedMin = ((Get-Date) - $start).TotalMinutes
        $privateKb = [math]::Round($p.PrivateMemorySize64 / 1KB, 1)
        $handles = $p.HandleCount

        $row = [PSCustomObject]@{
            TimestampUtc     = (Get-Date).ToUniversalTime().ToString('o')
            ElapsedMinutes   = [math]::Round($elapsedMin, 3)
            PrivateBytesKB   = $privateKb
            HandleCount      = $handles
        }
        $samples.Add($row)
        "$($row.TimestampUtc),$($row.ElapsedMinutes),$($row.PrivateBytesKB),$($row.HandleCount)" |
            Add-Content -Path $OutCsv -Encoding utf8

        Write-Host ("[{0}] t={1,7:N2}min  private={2,10:N0} KB  handles={3,5}" -f `
            (Get-Date -Format 'HH:mm:ss'), $elapsedMin, $privateKb, $handles)

        Start-Sleep -Seconds $SampleSeconds
    }
} finally {
    if (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue) {
        Write-Host ""
        Write-Host "Soak loop ending — closing app-shell (pid $($proc.Id))."
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    }
}

# ── Verdict: OLS linear regression of PrivateBytesKB over ElapsedMinutes ──
if ($samples.Count -lt 2) {
    Write-Warning "Fewer than 2 samples collected ($($samples.Count)) — cannot compute a slope. No verdict."
    exit 1
}

$n = $samples.Count
$sumX = 0.0; $sumY = 0.0; $sumXY = 0.0; $sumXX = 0.0
foreach ($s in $samples) {
    $x = $s.ElapsedMinutes
    $y = $s.PrivateBytesKB
    $sumX += $x
    $sumY += $y
    $sumXY += ($x * $y)
    $sumXX += ($x * $x)
}
$denom = ($n * $sumXX) - ($sumX * $sumX)
if ([math]::Abs($denom) -lt 1e-9) {
    Write-Warning "Degenerate regression (all samples at the same elapsed time) — no verdict."
    exit 1
}
$slopeKbPerMin = (($n * $sumXY) - ($sumX * $sumY)) / $denom
$slopeKbPerHour = $slopeKbPerMin * 60.0

Write-Host ""
Write-Host "Samples: $n over $([math]::Round($samples[$n-1].ElapsedMinutes, 2)) minutes"
Write-Host ("Memory slope: {0:N2} KB/h ({1:N4} KB/min)" -f $slopeKbPerHour, $slopeKbPerMin)

if ($slopeKbPerHour -le $ThresholdKbPerHour) {
    Write-Host ("SOAK PASS: slope <= {0} KB/h (observed {1:N2} KB/h)" -f $ThresholdKbPerHour, $slopeKbPerHour)
    exit 0
} else {
    Write-Host ("SOAK FAIL: slope > {0} KB/h (observed {1:N2} KB/h)" -f $ThresholdKbPerHour, $slopeKbPerHour)
    exit 1
}
