# Banshee vtebench baseline — 2026-07-11

The filename is the M1 operator-item path requested on 2026-07-05; the actual
measurement was run on 2026-07-11.

## System and build

- Banshee commit: `8aead0e0d953e7759f905631f0ebe66540da7dcc`
  (`m1-first-wail`, plus the uncommitted M1 cold-start probe/harness changes)
- Build: `cargo build --release -p app-shell`, MSVC, app-shell release binary
- Machine: MSI MS-7C35, AMD Ryzen 9 5900X (12C/24T), 63.9 GiB RAM
- GPU/display: AMD Radeon RX 9070 XT, driver 32.0.31021.5001, ~160 Hz
- OS: Windows 11 Pro 10.0.26200 (build 26200)
- Toolchain: rustc 1.96.1, cargo 1.96.1
- vtebench: alacritty/vtebench checkout `ead8003`

## Method

Run from the repository root with:

```powershell
$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"
pwsh.exe -NoLogo -NoProfile -File scripts/vtebench.ps1
```

The harness generated native Windows streams corresponding to upstream's
`scrolling`, `dense_cells`, and `unicode` cases. For each scenario, vtebench
ran inside a fresh Banshee release process using a temporary default profile
(`pwsh.exe -NoLogo -NoProfile -File <runner>`), at least 1 MiB per sample, one
vtebench warmup, and one measured sample. Three independent warm runs were
recorded. A host stopwatch started immediately before Banshee launch and stopped
when the in-terminal vtebench process wrote its completion marker. Every sample
also had to render Banshee's session-death banner before it was accepted; each
run had a 120 s hard timeout. No window focus or input injection was used.

## Results

| Scenario | Run 1 | Run 2 | Run 3 | Median |
|---|---:|---:|---:|---:|
| scrolling | 1,983.14 ms | 1,240.32 ms | 1,212.37 ms | **1,240.32 ms** |
| dense-cells | 1,029.86 ms | 1,000.74 ms | 1,004.53 ms | **1,004.53 ms** |
| unicode | 947.70 ms | 977.28 ms | 950.82 ms | **950.82 ms** |

All 9/9 runs completed and passed marker + rendered-death-banner confirmation;
no timeout fired. These are Banshee baseline numbers, not an NFR verdict.
winghostty is not installed on this machine, so the required
`Banshee median / winghostty median <= 1.5` comparison remains an operator item.
