# tools — Scripts and helpers

## Purpose

PowerShell maintenance helpers vendored with the app, plus Python build/screenshot utilities.
The heavy maintenance scripts themselves live under `SCRIPTS_ROOT` (default `E:\Scripts`); this
folder holds the shared kit and Castellyn-local tooling.

## Ownership

- `ScriptKit.ps1` — vendored, auto-synced helper library. Canonical copy lives here; exposes
  `Write-StatusJson` (the status-envelope writer every component script uses)
- `Sync-ScriptKit.ps1` — syncs `ScriptKit.ps1` out to the component scripts
- `make-icon.py` — regenerates icon formats from `src-tauri/icons/icon.png`
- `shoot.py`, `shoot-all.py` — screenshot capture for docs / design review
- `claude-profiles/`, `fork-updater/`, `stack/`, `analytics/` — Castellyn-local script sets for
  those feature areas

## Local Contracts

- Component scripts write the root Status envelope via `Write-StatusJson`; keep that shape stable
- Scripts run non-interactively (`-Yes -Unattended`, never `Read-Host`); destructive paths need a
  `-WhatIf`/preview and a UI confirm before the real run
- PowerShell may emit UTF-8 with BOM; the Rust side strips it — do not rely on absence of BOM
- Cyrillic paths: use `-LiteralPath` + single quotes; never drive these via Bash (mangles paths)
- `ScriptKit.ps1` is canonical here — edit here, then `Sync-ScriptKit.ps1`; do not hand-edit copies

## Work Guidance

- Adding a component means: a script writing `<id>.last.json`, a `manifest` entry, backend wiring,
  and i18n — coordinate across those, not just here

## Verification

- Run the script `-WhatIf`/`-Unattended` and confirm a valid `<id>.last.json` envelope is written

## Child DOX Index

None.
