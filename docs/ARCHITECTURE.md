# Architecture

Castellyn is a **Tauri v2** desktop app: a Svelte 5 frontend talking to a Rust backend over
Tauri IPC. The backend mostly orchestrates the user's PowerShell maintenance scripts under
`SCRIPTS_ROOT` (default `E:\Scripts`) and exposes their state to the UI. No database, no
sidecar process, single binary.

```
┌─────────────────────────────────────────────────────────────┐
│ Tauri window (decorations:false, custom titlebar)            │
│                                                              │
│  Frontend (Svelte 5 / SvelteKit static SPA)   │  Backend (Rust)         │
│  routes/+page.svelte  ── invoke() ───────────▶│  src/lib.rs             │
│   tab state, confirm dialog, toasts           │   #[tauri::command] fns │
│  lib/components/*  (one per tab)               │   spawn_streamed()      │
│  lib/ipc.ts (typed wrappers)                   │   native readers        │
│        ▲   run-log / run-done events           │        │                │
│        └────────────────────────────────────  │  spawns PowerShell ─────┼─▶ E:\Scripts\*.ps1
│  renders <id>.last.json status envelopes       │  reads  *.last.json      │
└─────────────────────────────────────────────────────────────┘
```

## Backend (`src-tauri/src/lib.rs`)

One file holds all commands. Key pieces:

- **`spawn_streamed(component, program, args, state)`** — the single way to run an external
  process. Streams stdout/stderr line-by-line as `run-log` events and a final `run-done`
  (exit code) event. Enforces **one run at a time** (a `RunState` mutex holds the child PID);
  `cancel_run` does `taskkill /T /F`. Every command that runs a script funnels through it
  (`run_component`, `run_forks`, `run_backup`, `run_profiles`, `run_mcp`, `run_sync`,
  `run_engine`, `run_provider`, `run_router`, `run_schedule`, `run_plugin`, …).
- **Native readers** (no script, pure Rust) where it's cheaper/safer: `read_mcp`,
  `read_providers` (reads each profile's `settings.json` env, never returns tokens — only
  `hasToken`), `read_engines` (+ TCP `port_listening`), `list_skills`, `list_plugin_contents`.
- **`CREATE_NO_WINDOW`** is set on every `Command` (pwsh/reg/taskkill/explorer) so no console
  window flashes. Required.
- **Config** — `HubConfig { scriptsRoot, startHidden, fetchTimeoutSec, ghTimeoutSec }` at
  `%APPDATA%\agenthub\config.json`. `read_config_file()` reads the current path and falls back
  to `legacy_config_path()` (pre-rename location) so settings survive the rename. Writes always
  go to the new path (`write_config` → `config_path()`).
- **`scripts_root()`** — `$SCRIPTS_ROOT` env → `config.scriptsRoot` → default `E:\Scripts`.
- **Tray / window** — `build_tray` (Show / Check-all / Quit), close-to-tray, autostart via
  HKCU\…\Run value `AgentHub`. Tray menu strings are hardcoded Russian (not i18n'd).

Registered commands (frontend calls these via `lib/ipc.ts`): `list_components`, `read_status`,
`run_component`, `run_forks`, `list_backups`, `run_backup`, `read_profiles`, `run_profiles`,
`read_profiles_config`, `run_profile_mgmt`, `open_profile_dir`, `launch_profile`,
`read_launch_config`, `set_launch_config`, `measure_context`, `read_sync`, `run_sync`,
`read_engines`, `update_engine`, `run_engine`, `run_router`, `run_connect_router`,
`read_engine_models`, `read_providers`, `run_provider`, `read_mcp`, `run_mcp`, `list_plugins`,
`list_skills`, `list_plugin_updates`, `list_plugin_contents`, `run_plugin`, `read_schedules`,
`run_schedule`, `read_config`, `write_config`, `app_paths`, `open_path`, `open_terminal`,
`get_autostart`, `set_autostart`, `cancel_run`.

## Frontend (`src/`)

- **`routes/+page.svelte`** is the orchestrator. It owns tab state (`active`), holds the data
  for every tab, makes all `read_*`/`run_*` calls, lazy-loads heavy tabs on first open
  (`$effect`), and centralizes the **confirm dialog** (`askConfirm`/`doConfirm`) and the
  **run lifecycle** (`run-log` appends to the console log; `run-done` reloads the relevant
  tab's data and surfaces a toast via `lib/outcome.ts`).
- **Components** (`lib/components/`): one per tab (`UpdatesTab`, `ForksTab`, `BackupTab`,
  `ProfilesTab`, `McpTab`, `SyncTab`, `ProvidersTab`, `PluginsTab`, `ScheduleTab`,
  `SettingsTab`) + dialogs (`ConfirmDialog`, `RestoreDialog`, `ProfileEditDialog`,
  `LaunchConfigDialog`, `ProviderEditDialog`, `RouterConnectDialog`) + shell (`Sidebar`,
  `Console`, `WindowTitleBar`, `ToastHost`, `Toggle`, `DropdownMenu`, `ComponentCard`).
- **Support modules**: `lib/ipc.ts` (typed invoke + types), `lib/i18n/` (localization),
  `lib/outcome.ts` (run → toast), `lib/attention.ts` (sidebar badges), `lib/glossary.ts`
  (per-component help), `lib/theme.ts` (dark/light), `lib/toast.svelte.ts` (toast store).
- **Persistence** is `localStorage` (`cmh-theme`, `cmh-language`, `cmh-console-*`). Init runs
  in `routes/+layout.svelte` (`initTheme()`, `initLocale()`).

## The component / status model

`manifest/maintenance-manifest.json` lists the maintenance components and where each writes its
status (`lastJsonRel`). It is read **from disk at runtime** (`manifest_text()` in lib.rs) so the
canonical manifest under the repo is authoritative; an embedded copy is the fallback if the
file is missing (e.g. the exe relocated without the repo).

Every script writes a **status envelope** to its `<id>.last.json`:

```json
{ "schemaVersion": 1, "component": "forks", "status": "ok|changes|error|held",
  "timestamp": "…", "mode": "check|apply", "durationSec": 12,
  "counts": { "changed": 0, "failed": 0, "total": 9 }, "summary": { } }
```

Scripts emit it through `Write-StatusJson` in `tools/ScriptKit.ps1` (a vendored helper that is
auto-synced from this canonical copy to other repos by `tools/Sync-ScriptKit.ps1`). The UI reads
the envelope in `ComponentCard`/`outcome.ts`/`attention.ts`.

## Profiles model

Claude Code "profiles" are isolated config dirs `~/.claude-<name>` with junction/symlink links
to shared content (`skills`, `commands`, `agents`, `plugins`, `projects`, `history.jsonl`)
under `~/.claude`. Castellyn reads health via a read-only `Get-ProfilesStatus.ps1`
(`profiles.last.json`) and mutates via data-driven scripts that read `config/profiles.json`
(install/repair/add/remove/rename/recolor/set-links). Symlinking *folders* needs admin (UAC);
junctions/file-links don't.

## Gotchas

- PowerShell writes JSON with a UTF-8 **BOM** → strip `\u{feff}` before `serde_json`.
- Syncthing folder IDs are per-machine — resolve folders **by path**, never by a hardcoded ID.
- Hidden `.stignore` can't be `CREATE_ALWAYS`-written while Hidden — clear the attribute first.
- claude-code-router (ccr) may fail to start on some Node versions — that's a ccr issue, not
  Castellyn's.
