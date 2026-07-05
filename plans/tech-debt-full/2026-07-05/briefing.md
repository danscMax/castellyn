# Tech Debt Briefing: full (whole project)

## Git metadata
- Branch: `main`, commit: `1c02e3d`
- Date: 2026-07-05

## Project Type
Tauri v2 desktop app (Windows-first). Frontend: SvelteKit static/SPA + Svelte 5 runes + Tailwind v4 + xterm.js. Backend: Rust (single big file `src-tauri/src/lib.rs`, ~14.2k lines — all `#[tauri::command]`s). No DB, no sidecar. Spawns user PowerShell maintenance scripts and streams output. Secrets in Windows Credential Manager via `keyring`. PTY sessions via `portable-pty 0.8` (pinned). Win32 Job Objects for child-tree cleanup.

## System Map
- `src-tauri/src/lib.rs` (14187 ln) — ALL backend commands: config (`%APPDATA%\castellyn\config.json` + legacy fallback), `spawn_streamed` (single streaming path, one run at a time, `cancel_run` kills tree), native readers (`read_mcp`, `read_providers`, `port_listening`, plugin/skill scans), PTY sessions, keyring (`kr_get`/`kr_delete` with legacy re-home), autostart HKCU Run, tray, updater, provider HTTP probes (`ureq` blocking in `spawn_blocking`), profiles CRUD, onboarding reconciler (`read_onboarding`).
- `src-tauri/src/agent_status.rs` (622) — agent status detection from PTY output.
- `src-tauri/src/limits.rs` (308) — usage limits parsing.
- `src-tauri/src/i18n.rs` (371) — tray label localization (ru/en/zh).
- `src/routes/+page.svelte` (~101KB) — orchestrator: tab state, all invoke calls, confirm dialog, run-log, toasts.
- `src/lib/components/*.svelte` (50) — one per tab + dialogs + shell (Sidebar, Console/xterm, WindowTitleBar, OnboardingWizard, SessionsTab, ProfilesTab, PluginsTab, McpTab, ProvidersTab, EnvsTab, BackupTab, AnalyticsTab, UpdatesTab, ForksTab, ScheduleTab...).
- `src/lib/*.ts` — ipc.ts (typed invoke wrappers), outcome.ts, attention.ts, envelope.ts, i18n/ (ru/en/zh parity enforced), theme.ts, runHistory/toast/running/agentStatus/navOrder svelte.ts stores.
- `tools/ScriptKit.ps1`, `tools/Sync-ScriptKit.ps1` — vendored PS helper (Write-StatusJson envelope contract), canonical copy.
- `manifest/maintenance-manifest.json` — component list, read from disk at runtime with embedded fallback.

## Trust boundaries / sensitive surfaces
- Spawns PowerShell scripts from `SCRIPTS_ROOT` (user-configurable path) — command construction, arg injection.
- Keyring secrets (provider API keys, freellmapi token) — must never leak to logs/JSON/events.
- Provider HTTP probes (`ureq`) — URLs from user config; timeouts, TLS.
- PTY sessions run `claude`/`pwsh`/`ssh` — env handling, secret env vars.
- Config/JSON files written by external PS scripts (UTF-8 BOM handling), read by Rust.
- Registry writes (autostart), file system writes (config, sidecars, Syncthing-synced prefs).
- Updater (tauri-plugin-updater).

## Audit Axes
Base 5: Security, Reliability, Performance, Code Quality, Supply Chain. Security = multi-run (2 independent passes).

## Files for Analysis (absolute paths)
Backend: `E:\Scripts\Castellyn\src-tauri\src\lib.rs`, `agent_status.rs`, `limits.rs`, `i18n.rs`, `main.rs`, `E:\Scripts\Castellyn\src-tauri\Cargo.toml`, `tauri.conf.json`, `capabilities\*`.
Frontend: `E:\Scripts\Castellyn\src\routes\+page.svelte`, `+layout.svelte`, `+layout.ts`, `E:\Scripts\Castellyn\src\lib\**\*.ts`, `E:\Scripts\Castellyn\src\lib\components\*.svelte`, `package.json`, `vite.config.js`, `svelte.config.js`.
Scripts: `E:\Scripts\Castellyn\tools\*.ps1`, `E:\Scripts\Castellyn\build_all.ps1`, `verify.ps1`, `scripts\check-i18n-parity.ts`.
Manifest: `E:\Scripts\Castellyn\manifest\maintenance-manifest.json`.

Chunking guidance: `lib.rs` is huge — read it in sections (search-first with Grep for your axis's patterns: `unwrap`, `expect`, `Command::new`, `spawn`, `kr_`, `emit`, `lock()`, `clone()`, `read_to_string`, etc.), then deep-read hot regions. Do NOT try to read all 14k lines linearly. For frontend, prioritize `+page.svelte`, Console.svelte, SessionsTab, ipc.ts, then sweep the rest.

## Scope Boundary (OUT of scope)
- Style-only remarks with no correctness impact; UI/UX polish without safety/data consequences.
- Theoretical attacks requiring physical/local-admin access (this is a local single-user desktop tool; the threat model is: malicious script output, malformed JSON, hostile provider endpoints, secret leakage — NOT a remote attacker).
- Micro-optimizations without measurable effect.
- i18n translation quality (parity is CI-enforced already).
- The old `agenthub` fallback names — deliberate migration compat, do NOT flag as dead code (see CLAUDE.md).
- `agenthub-local` dummy token, `agenthub:<name>` label, `agenthub-pty-probe` — deliberate sentinels, not secrets/bugs.

## Known Issues (do not re-report)
- Full goal-audit ran 2026-07-04 on this codebase: 0 Critical/High, 6 Medium + 17 Low ALL FIXED and merged to main (commits 95bb872→3225e9e). Current HEAD 1c02e3d includes those fixes plus the onboarding wizard. Findings must be NEW relative to current HEAD — verify against actual code, not assumptions.
- cargo audit 2026-06-21 flagged 5 unmaintained/unsound transitive crates (worth re-checking freshness — supply-chain agent should re-run/re-verify).
- `portable-pty` pinned to 0.8 deliberately (0.9 hangs headless PTYs — see Cargo.toml comment). Do not flag the pin itself; DO check for CVEs in 0.8.
- One-run-at-a-time limitation of `spawn_streamed` is a design decision, not a bug.
- Known: claude-mem 13.10.1 worker instability (external, out of scope).

## Output requirements
Each finding MUST include: file, exact lines, severity (Critical/High/Medium/Low), description, evidence (verbatim code quote you actually read), fix suggestion. Max 25 findings per agent, Critical+High prioritized. Findings without a verbatim code quote will be discarded.
