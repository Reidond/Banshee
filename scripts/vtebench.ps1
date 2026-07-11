[CmdletBinding()]
param(
    [string] $Terminal = (Join-Path $PSScriptRoot '..\target\release\app-shell.exe')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$runsPerScenario = 3
$timeoutSeconds = 120
$scenarios = @('scrolling', 'dense-cells', 'unicode')

function Resolve-Executable([string] $Path) {
    $resolved = Resolve-Path -LiteralPath $Path -ErrorAction SilentlyContinue
    if ($null -ne $resolved) {
        return $resolved.Path
    }

    $command = Get-Command $Path -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    throw "Executable not found: $Path"
}

function Resolve-Vtebench {
    $candidates = @(
        (Join-Path $PSScriptRoot 'vtebench.exe'),
        (Join-Path $HOME '.cargo\bin\vtebench.exe')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }

    $command = Get-Command vtebench.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    throw 'vtebench.exe not found. Install it with: cargo install --git https://github.com/alacritty/vtebench'
}

function ConvertTo-TomlString([string] $Value) {
    return '"' + $Value.Replace('\', '\\').Replace('"', '\"') + '"'
}

function ConvertTo-WindowsCommandArg([string] $Value) {
    if ($Value -notmatch '[\s"]') {
        return $Value
    }
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Read-SharedText([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return ''
    }
    $stream = [IO.File]::Open(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::ReadWrite
    )
    try {
        $reader = [IO.StreamReader]::new($stream)
        try {
            return $reader.ReadToEnd()
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Stop-ProcessTree([int] $RootPid) {
    $processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue)
    $ids = [Collections.Generic.List[int]]::new()
    $ids.Add($RootPid)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($child in $processes | Where-Object ParentProcessId -eq $ids[$i]) {
            $childId = [int] $child.ProcessId
            if (-not $ids.Contains($childId)) {
                $ids.Add($childId)
            }
        }
    }
    for ($i = $ids.Count - 1; $i -ge 0; $i--) {
        Stop-Process -Id $ids[$i] -Force -ErrorAction SilentlyContinue
    }
}

function New-NativeBenchmarkCorpus([string] $Root) {
    $source = Join-Path $Root 'benchmark-generator.rs'
    $generator = Join-Path $Root 'benchmark-generator.exe'
    @'
use std::env;
use std::io::{self, Write};

fn main() {
    let scenario = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|p| p.to_path_buf()))
        .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    match scenario.as_str() {
        "scrolling" => {
            for line in 1..=20_000 {
                writeln!(out, "vtebench scrolling line {line:05}").unwrap();
            }
        }
        "dense-cells" => {
            write!(out, "\x1b[?1049h").unwrap();
            for frame in 0..36 {
                write!(out, "\x1b[H").unwrap();
                for row in 0..30 {
                    for col in 0..100 {
                        let fg = 16 + ((frame + row + col) % 216);
                        let bg = 16 + ((frame * 3 + row + col) % 216);
                        let ch = (b'A' + (frame % 26) as u8) as char;
                        write!(out, "\x1b[38;5;{fg};48;5;{bg};1m{ch}").unwrap();
                    }
                }
            }
            write!(out, "\x1b[0m").unwrap();
        }
        "unicode" => {
            let sample = "ASCII Ελληνικά Кирилиця Україна العربية हिन्दी 日本語 한글 "
                .to_owned() + "e\u{301} a\u{308} ⚡☂♜ →⇧∑∫∞ 😀🚀🧪\r\n";
            for _ in 0..8_000 {
                write!(out, "{sample}").unwrap();
            }
        }
        _ => panic!("unknown benchmark scenario: {}", scenario),
    }
}
'@ | Set-Content -LiteralPath $source -Encoding utf8

    & rustc -O $source -o $generator
    if ($LASTEXITCODE -ne 0) {
        throw "rustc failed to build the temporary benchmark generator (exit $LASTEXITCODE)"
    }

    foreach ($scenario in $scenarios) {
        $directory = Join-Path $Root $scenario
        New-Item -ItemType Directory -Path $directory | Out-Null
        # vtebench intentionally discovers files named exactly `benchmark`.
        # CreateProcess accepts a PE image without an .exe suffix when the full
        # path is supplied, so one native generator can back all three folders.
        [IO.File]::Copy($generator, (Join-Path $directory 'benchmark'), $true)
    }
}

function New-Runner(
    [string] $Path,
    [string] $Vtebench,
    [string] $BenchmarkDirectory,
    [string] $DonePath
) {
    @"
`$ErrorActionPreference = 'Stop'
& '$($Vtebench.Replace("'", "''"))' --silent --benchmarks '$($BenchmarkDirectory.Replace("'", "''"))' --warmup 1 --min-bytes 1048576 --max-samples 1 --max-secs 10
if (`$LASTEXITCODE -ne 0) { exit `$LASTEXITCODE }
[IO.File]::WriteAllText('$($DonePath.Replace("'", "''"))', 'ok')
exit 0
"@ | Set-Content -LiteralPath $Path -Encoding utf8
}

function New-BansheeConfig([string] $Path, [string] $Runner) {
    $arguments = @('-NoLogo', '-NoProfile', '-File', $Runner) |
        ForEach-Object { ConvertTo-TomlString $_ }
    @"
[[profile]]
name = "vtebench"
command = "pwsh.exe"
args = [$($arguments -join ', ')]
type = "windows"
default = true
"@ | Set-Content -LiteralPath $Path -Encoding utf8
}

function New-WinghosttyConfig([string] $Root, [string] $Runner) {
    $parts = @('pwsh.exe', '-NoLogo', '-NoProfile', '-File', $Runner) |
        ForEach-Object { ConvertTo-WindowsCommandArg $_ }
    $content = @"
command = $($parts -join ' ')
window-save-state = never
shell-integration = none
"@
    # Current winghostty builds use the fork namespace for the Windows fallback,
    # while the shared Ghostty XDG helper uses `ghostty`. Populate both names in
    # the isolated temporary root; only the path selected by the binary is read.
    foreach ($namespace in @('ghostty', 'winghostty')) {
        $directory = Join-Path $Root $namespace
        New-Item -ItemType Directory -Path $directory | Out-Null
        $content | Set-Content -LiteralPath (Join-Path $directory 'config.ghostty') -Encoding utf8
    }
}

$terminalPath = Resolve-Executable $Terminal
$vtebenchPath = Resolve-Vtebench
$terminalKind = if ([IO.Path]::GetFileName($terminalPath) -ieq 'app-shell.exe') {
    'banshee'
}
elseif ([IO.Path]::GetFileName($terminalPath) -like 'winghostty*') {
    'winghostty'
}
else {
    throw '-Terminal currently supports app-shell.exe or winghostty.exe.'
}

$workspace = Join-Path ([IO.Path]::GetTempPath()) ("banshee-vtebench-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workspace | Out-Null
$oldConfig = $env:BANSHEE_CONFIG_PATH
$oldDump = $env:BANSHEE_DEBUG_DUMP_GRID
$oldXdg = $env:XDG_CONFIG_HOME
$results = @()

try {
    New-NativeBenchmarkCorpus $workspace
    foreach ($scenario in $scenarios) {
        foreach ($run in 1..$runsPerScenario) {
            $runRoot = Join-Path $workspace "$scenario-$run"
            New-Item -ItemType Directory -Path $runRoot | Out-Null
            $done = Join-Path $runRoot 'complete.marker'
            $runner = Join-Path $runRoot 'run.ps1'
            $dump = Join-Path $runRoot 'grid.txt'
            New-Runner $runner $vtebenchPath (Join-Path $workspace $scenario) $done

            if ($terminalKind -eq 'banshee') {
                $config = Join-Path $runRoot 'config.toml'
                New-BansheeConfig $config $runner
                $env:BANSHEE_CONFIG_PATH = $config
                $env:BANSHEE_DEBUG_DUMP_GRID = $dump
                Remove-Item Env:XDG_CONFIG_HOME -ErrorAction SilentlyContinue
            }
            else {
                $xdg = Join-Path $runRoot 'xdg'
                New-WinghosttyConfig $xdg $runner
                $env:XDG_CONFIG_HOME = $xdg
                Remove-Item Env:BANSHEE_CONFIG_PATH, Env:BANSHEE_DEBUG_DUMP_GRID -ErrorAction SilentlyContinue
            }

            $watch = [Diagnostics.Stopwatch]::StartNew()
            $process = Start-Process -FilePath $terminalPath -PassThru
            $completedMs = $null
            $exitConfirmed = $false
            try {
                while ($watch.Elapsed.TotalSeconds -lt $timeoutSeconds) {
                    if ($null -eq $completedMs -and (Test-Path -LiteralPath $done)) {
                        $completedMs = $watch.Elapsed.TotalMilliseconds
                    }

                    if ($terminalKind -eq 'banshee') {
                        if ((Read-SharedText $dump) -match '(?i)\[banshee\].*(session ended|exited|terminated|killed)') {
                            $exitConfirmed = $true
                            break
                        }
                    }
                    else {
                        $process.Refresh()
                        if ($process.HasExited -and $null -ne $completedMs) {
                            $exitConfirmed = $true
                            break
                        }
                    }
                    Start-Sleep -Milliseconds 5
                }

                if ($null -eq $completedMs) {
                    throw "$scenario run $run timed out after $timeoutSeconds s before vtebench completed"
                }
                if (-not $exitConfirmed) {
                    throw "$scenario run $run timed out after $timeoutSeconds s before terminal session exit was confirmed"
                }

                $result = [pscustomobject]@{
                    scenario = $scenario
                    run = $run
                    wall_ms = [math]::Round($completedMs, 2)
                    completion = if ($terminalKind -eq 'banshee') { 'marker + rendered death banner' } else { 'marker + terminal process exit' }
                }
                $results += $result
                Write-Host ("{0} run {1}: {2:N2} ms" -f $scenario, $run, $result.wall_ms)
            }
            finally {
                Stop-ProcessTree $process.Id
                Start-Sleep -Milliseconds 250
            }
        }
    }
}
finally {
    if ($null -eq $oldConfig) { Remove-Item Env:BANSHEE_CONFIG_PATH -ErrorAction SilentlyContinue } else { $env:BANSHEE_CONFIG_PATH = $oldConfig }
    if ($null -eq $oldDump) { Remove-Item Env:BANSHEE_DEBUG_DUMP_GRID -ErrorAction SilentlyContinue } else { $env:BANSHEE_DEBUG_DUMP_GRID = $oldDump }
    if ($null -eq $oldXdg) { Remove-Item Env:XDG_CONFIG_HOME -ErrorAction SilentlyContinue } else { $env:XDG_CONFIG_HOME = $oldXdg }
    Remove-Item -LiteralPath $workspace -Recurse -Force -ErrorAction SilentlyContinue
}

$summary = foreach ($scenario in $scenarios) {
    $samples = @($results | Where-Object scenario -eq $scenario | Sort-Object wall_ms)
    [pscustomobject]@{
        scenario = $scenario
        run_1_ms = ($results | Where-Object { $_.scenario -eq $scenario -and $_.run -eq 1 }).wall_ms
        run_2_ms = ($results | Where-Object { $_.scenario -eq $scenario -and $_.run -eq 2 }).wall_ms
        run_3_ms = ($results | Where-Object { $_.scenario -eq $scenario -and $_.run -eq 3 }).wall_ms
        median_ms = $samples[1].wall_ms
    }
}

$summary | Format-Table -AutoSize
$summary | ConvertTo-Json -Compress
