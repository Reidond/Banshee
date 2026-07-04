# One-command entry point for the automated live-input matrix.
# Launches real Banshee windows (focus-free: input is posted, not injected),
# runs the scenarios serially, prints the verdict. Safe to run while working,
# though windows will briefly appear on the desktop.
param([switch]$Release)

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$flags = @("-p", "app-shell", "--test", "live_input_matrix")
if ($Release) { $flags = @("--release") + $flags }

cargo test @flags -- --include-ignored --test-threads=1 --nocapture
if ($LASTEXITCODE -eq 0) {
    Write-Host "`nLIVE-MATRIX PASS" -ForegroundColor Green
} else {
    Write-Host "`nLIVE-MATRIX FAIL (exit $LASTEXITCODE)" -ForegroundColor Red
}
exit $LASTEXITCODE
