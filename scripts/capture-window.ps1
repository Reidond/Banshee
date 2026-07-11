# Capture a Banshee window to PNG — the visual-verification primitive for
# agent-driven UI checks (see scripts/visual-smoke.ps1 for the harness).
#
# Uses PrintWindow with PW_RENDERFULLCONTENT (required for DirectComposition/
# WinUI3-composed content; plain PrintWindow/BitBlt returns black). Falls back
# to a screen copy of the window rect if PrintWindow yields an empty image
# (window must then be visible and unoccluded).
param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$OutFile
)

Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Win32 -Name Capture -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
public struct RECT { public int Left, Top, Right, Bottom; }
'@

[Win32.Capture]::SetProcessDPIAware() | Out-Null

$proc = Get-Process -Id $ProcessId -ErrorAction Stop
$hwnd = $proc.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { throw "process $ProcessId has no main window" }

$rect = New-Object Win32.Capture+RECT
[Win32.Capture]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left
$h = $rect.Bottom - $rect.Top
if ($w -le 0 -or $h -le 0) { throw "window has empty rect ($w x $h)" }

$bmp = New-Object System.Drawing.Bitmap($w, $h)
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $gfx.GetHdc()
# PW_RENDERFULLCONTENT = 0x2 — composes DComp/XAML content into the DC.
$ok = [Win32.Capture]::PrintWindow($hwnd, $hdc, 2)
$gfx.ReleaseHdc($hdc)
$gfx.Dispose()

# Detect the black-frame failure mode: sample a grid of pixels; if every
# sample is pure black AND PrintWindow claimed success, still fall back.
$allBlack = $true
foreach ($sx in 0..4) {
    foreach ($sy in 0..4) {
        $p = $bmp.GetPixel([int]($w * ($sx + 0.5) / 5), [int]($h * ($sy + 0.5) / 5))
        if ($p.R -ne 0 -or $p.G -ne 0 -or $p.B -ne 0) { $allBlack = $false; break }
    }
    if (-not $allBlack) { break }
}

if (-not $ok -or $allBlack) {
    $bmp.Dispose()
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $gfx = [System.Drawing.Graphics]::FromImage($bmp)
    $gfx.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $gfx.Dispose()
    Write-Host "capture: PrintWindow empty -> screen-copy fallback"
} else {
    Write-Host "capture: PrintWindow(PW_RENDERFULLCONTENT) ok"
}

$dir = Split-Path -Parent $OutFile
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force $dir | Out-Null }
$bmp.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Host "saved: $OutFile ($w x $h)"
