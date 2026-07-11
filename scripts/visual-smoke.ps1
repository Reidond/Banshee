# Agent-driven visual smoke: launches the real Banshee window, drives it with
# posted messages (focus-free), and captures PNG screenshots per scene via
# scripts/capture-window.ps1 (PrintWindow PW_RENDERFULLCONTENT).
#
# Output: a timestamped gallery dir the operator (or a vision-capable agent)
# can eyeball. This is the M1 answer to "computer use for UI verification":
# input = PostMessageW, eyes = window capture, assertions = human/agent review
# (structural asserts live in tests/live_input_matrix.rs; this adds pixels).
param(
    [string]$OutDir = (Join-Path (Split-Path -Parent $PSScriptRoot) ("soak-results\visual\" + (Get-Date -Format "yyyyMMdd-HHmmss"))),
    [switch]$UserProfile  # also capture a scene with the operator's real profile
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\release\app-shell.exe"
if (-not (Test-Path $exe)) { throw "release binary missing - run: cargo build --release -p app-shell" }
New-Item -ItemType Directory -Force $OutDir | Out-Null

$sig = '[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);'
$u = Add-Type -MemberDefinition $sig -Name P -Namespace VSmoke -PassThru

function Start-Banshee([bool]$bare) {
    if ($bare) {
        $cfgdir = Join-Path $env:TEMP "banshee-visual-cfg"
        New-Item -ItemType Directory -Force $cfgdir | Out-Null
        "[[profile]]`nname = `"pwsh-bare`"`ncommand = `"pwsh.exe`"`nargs = [`"-NoLogo`", `"-NoProfile`"]`ntype = `"windows`"`ndefault = true" |
            Set-Content (Join-Path $cfgdir "config.toml")
        $env:BANSHEE_CONFIG_PATH = Join-Path $cfgdir "config.toml"
    } else {
        Remove-Item Env:\BANSHEE_CONFIG_PATH -ErrorAction SilentlyContinue
    }
    $p = Start-Process -FilePath $exe -PassThru
    Start-Sleep -Seconds 4
    return $p
}

function Send-Line([IntPtr]$hwnd, [string]$text) {
    foreach ($ch in $text.ToCharArray()) {
        $u::PostMessageW($hwnd, 0x0102, [IntPtr][int]$ch, [IntPtr]1) | Out-Null
        Start-Sleep -Milliseconds 8
    }
    $u::PostMessageW($hwnd, 0x0102, [IntPtr]13, [IntPtr]1) | Out-Null
}

function Send-Wheel([IntPtr]$hwnd, [int]$notches) {
    $delta = [uint16]([int16]($notches * 120))
    $wparam = [IntPtr]([int64]$delta -shl 16)
    $u::PostMessageW($hwnd, 0x020A, $wparam, [IntPtr]0) | Out-Null
}

function Capture([int]$procId, [string]$scene) {
    & (Join-Path $PSScriptRoot "capture-window.ps1") -ProcessId $procId -OutFile (Join-Path $OutDir "$scene.png")
}

# ── Scene 1-3: bare profile — styles, scripts, scrollback ──
$p = Start-Banshee $true
$hwnd = (Get-Process -Id $p.Id).MainWindowHandle
try {
    Capture $p.Id "01-startup-prompt"

    Send-Line $hwnd 'echo "`e[31mred `e[42mgreen-bg `e[1mbold `e[4munderline `e[9mstrike `e[7minverse`e[0m `e[38;2;255;128;0mtruecolor-orange`e[0m"'
    Start-Sleep -Seconds 2
    Capture $p.Id "02-sgr-styles"

    Send-Line $hwnd 'echo "latin привіт-українська 中文汉字 emoji: 🎉🚀 mixed-in-line done"'
    Start-Sleep -Seconds 2
    Capture $p.Id "03-scripts-emoji"

    Send-Line $hwnd '1..200 | ForEach-Object { "scrollback line $_" }'
    Start-Sleep -Seconds 3
    Capture $p.Id "04-after-flood-tail"

    1..12 | ForEach-Object { Send-Wheel $hwnd 1 }
    Start-Sleep -Seconds 2
    Capture $p.Id "05-scrolled-into-history"
} finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
}

# ── Scene 6 (optional): the operator's real profile (starship etc.) ──
if ($UserProfile) {
    $p = Start-Banshee $false
    try {
        Start-Sleep -Seconds 3  # heavyweight prompt settle
        Capture $p.Id "06-user-profile-prompt"
    } finally {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "`nVISUAL SMOKE gallery: $OutDir"
Get-ChildItem $OutDir | Select-Object Name, Length
