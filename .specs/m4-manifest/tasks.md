# Tasks: m4-manifest — OUTLINE (re-derive full DAG at M3-exit re-baseline)

> Coarse work packages only; promote to the full task template when this spec is
> promoted to full depth. Q1 (name/icon) must be resolved BEFORE this milestone
> starts — it blocks signing identity, package ids, and store metadata.

## Indicative groups

- **Group A — Identity & packaging**:
  - A1 (S): finalize name/icon (Q1) → package identity, winget id
  - A2 (L): MSIX packaging x64+ARM64 (framework-dependency wiring, App Installer updates)
  - A3 (M): portable self-contained ZIP + Q7 size/startup measurement memo + daily update check
  - A4 (M): winget manifests + release pipeline (GitHub Releases, both artifacts)
- **Group B — Signing & supply chain** (with A):
  - B1 (M): Azure Trusted Signing (or OV cert) integration; updater signature verification incl. tamper-rejection test
  - B2 (S): cargo-deny/cargo-audit CI gates; SBOM generation per release; vendored-vt checksum re-audit
- **Group C — Accessibility & i18n** (parallel):
  - C1 (M): UIA completeness pass over all chrome + keyboard-complete audit
  - C2 (M): announce mode (screen-reader detection → LiveRegion notifications for new output)
  - C3 (S): high-contrast detection (NFR-6 remainder)
  - C4 (M): resource-string extraction, EN+UK translations, RTL-safe chrome layout audit
- **Group D — Hardening** (parallel):
  - D1 (M): full SPEC §10 perf table + 24 h soak + regression fixes
  - D2 (M): ARM64 functional validation (first functional pass; M0–M3 only linked) + Q3 revisit if hybrid-GPU issues
  - D3 (S): crash minidumps (local only, reveal-in-Explorer)
  - D4 (S): enterprise policy file (`%ProgramData%\banshee\policy.toml`: force ai=off, pin providers, disable OSC 52)
- **Group E — Docs & CLI**:
  - E1 (M): docs set (config reference, shell integration, agent setup, themes, install/SmartScreen expectations)
  - E2 (S): `banshee` CLI (Q8): new-tab/profile/cwd scripting parity with wt.exe
- **Group F — Ship**:
  - F1 (M): author + execute the 1.0 ship checklist (every P0 FR/NFR verified against SPEC §4; sign-off recorded)

## Known constraints for re-baseline

- A1 gates A2/A4/B1 — schedule it at milestone entry, not in parallel.
- D2 needs ARM64 hardware access — arrange before the milestone starts.
- C2 (announce mode) should be user-tested with a real screen reader, not just UIA inspection.

## Deviations Log

| Task | Deviation | Rationale |
|------|-----------|-----------|
| — | — | — |
