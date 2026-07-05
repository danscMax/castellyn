# src-tauri — Rust / Tauri v2 backend

## Purpose

The native backend: a single Tauri v2 binary that spawns the PowerShell maintenance scripts,
streams their output to the UI, and exposes native readers where cheaper than a script.

## Ownership

- `src/lib.rs` — essentially the whole backend: all `#[tauri::command]`s. Key pieces:
  - `spawn_streamed(...)` — the single DRY entry to run a script and stream `run-log` /
    `run-done` events. One run at a time; `cancel_run` kills the process tree.
  - Native readers: `read_mcp`, `read_providers`, `port_listening`, plugin/skill scans
  - Config: `HubConfig` in `%APPDATA%\castellyn\config.json` (`config_path()`), with legacy-path
    read fallback (`legacy_config_path`)
  - Autostart: HKCU Run value `Castellyn` (`AUTOSTART_NAME`), migrated once from `AgentHub`
- `src/i18n.rs` — localized tray labels (`tr("tray.*", lang)`); `set_language` relabels live
- `src/agent_status.rs`, `src/limits.rs` — agent status + usage-limit helpers
- `src/main.rs`, `build.rs` — entry point + build script
- `tauri.conf.json`, `capabilities/`, `Cargo.toml`, `icons/`, `assets/`

## Local Contracts

- **Every** `Command` sets `CREATE_NO_WINDOW` (0x08000000) — otherwise a black console flashes
- Do not add a second streaming path; reuse `spawn_streamed`
- Keep the `agenthub`→`castellyn` migration fallbacks (config path chain, keyring re-home in
  `kr_get`/`kr_delete`, `migrate_autostart`) — removing them loses existing user data
- Deliberately-kept old-name sentinels (`agenthub-local` token, `agenthub:<name>` label,
  `agenthub-pty-probe`) must stay in sync with the PS scripts / tests
- Strip a leading BOM (`\u{feff}`) before `serde_json` on script output; own writers emit no BOM
- Emit/consume the root Status envelope shape unchanged

## Work Guidance

- Prefer a native reader over a script only when clearly cheaper; otherwise route through
  `spawn_streamed`
- Icon master: `icons/icon.png` (1024) → `python tools/make-icon.py` → `npm run tauri -- icon <path>`

## Verification

- `cargo test`, `cargo clippy` (cargo not on PATH — use full path, trust `$LASTEXITCODE`)
- Full app: `npm run tauri dev`; release: `.\build_all.ps1`

## Child DOX Index

None.
