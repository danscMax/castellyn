# Architecture

Castellyn is a **Tauri v2** desktop app: a Svelte 5 frontend talking to a Rust backend over
Tauri IPC. The backend orchestrates the user's PowerShell maintenance scripts under
`SCRIPTS_ROOT` (default `E:\Scripts`) and exposes their state to the UI — but increasingly does
the work **natively in Rust**: many former PS surfaces (sync, providers, router, opencode,
plugins, engine, config-drift) are now native commands. No database, no sidecar process,
single binary.

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
  `hasToken`), `read_engines` (+ TCP `port_listening`), `read_config_drift` (shared-config link
  health), `list_skills`, `list_plugin_contents`.
- **`CREATE_NO_WINDOW`** is set on every `Command` (pwsh/reg/taskkill/explorer) so no console
  window flashes. Required.
- **Config** — `HubConfig { scriptsRoot, startHidden, fetchTimeoutSec, ghTimeoutSec }` at
  `%APPDATA%\castellyn\config.json`. `read_config_file()` reads the current path and falls back
  through `agenthub_config_path()` then `legacy_config_path()` (pre-rename locations) so settings
  survive the renames. Writes always go to the new path (`write_config` → `config_path()`).
- **`scripts_root()`** — `$SCRIPTS_ROOT` env → `config.scriptsRoot` → default `E:\Scripts`.
- **Tray / window** — `build_tray` (Show / Check-all / Quit), close-to-tray, autostart via
  HKCU\…\Run value `Castellyn` (migrated once from the old `AgentHub` value). Tray menu labels are
  localized via `src-tauri/src/i18n.rs` (`tr("tray.*", lang)`) and relabeled live on a locale change
  (`set_language`); the tooltip is the brand name `Castellyn`.

**Registered commands** — the canonical, authoritative list is the `tauri::generate_handler![…]`
block at the bottom of `lib.rs` (~80 commands; frontend calls them via typed wrappers in
`lib/ipc.ts`). Don't maintain a copy here — it rots. They group roughly as:

- **components / updates** — `list_components`, `read_status`, `run_component`, `cancel_run`
- **forks** — `run_forks`, `run_fork_repo`, `cancel_fork_repo`, `read_fork_repo_status`, `list_github_repos`
- **backup / restore** — `list_backups`, `run_backup`
- **profiles** — `read_profiles`, `run_profiles`, `run_profile_mgmt`, `repair_profile_elevated`,
  `relaunch_as_admin` (UAC for folder symlinks), `open_profile_dir`, `launch_profile`, `read_profile_usage`
- **sync + config-drift** — `read_sync`, `run_sync`, `read_config_drift`, `run_config_drift`
- **providers / engines / router / opencode** — `read_providers`, `run_provider`, `read_engines`,
  `run_engine`, `run_router`, `run_connect_router`, `read_stack`, `run_stack`, my-provider CRUD, key rotation
- **MCP / plugins / skills** — `read_mcp`, `run_mcp`, `list_plugins`, `run_plugin`, `list_skills`, `delete_skill`
- **environments** (cross-harness coverage) — `read_environments` (per-harness skills/providers/MCP/RTK
  overview), `read_skill_matrix` (per-skill × harness diff), `share_skills` (junction every skill into
  `~/.agents/skills`, the folder OpenCode + Codex both scan), `run_opencode_rtk` (write/remove the
  Windows-safe OpenCode RTK plugin), plus one-click canonical fan-outs: `run_opencode_mcp`
  (.mcp.json → opencode.json `mcp`), `run_opencode_providers` (myproviders.json → `provider`,
  keys as `{env:…}` refs only), `run_opencode_instructions` (canonical CLAUDE.md/RTK.md paths →
  `instructions[]`), `run_codex_mcp` (.mcp.json → Codex via the official `codex mcp add` CLI),
  `run_codex_providers` (freellmapi gateway → Codex `[model_providers]`+`[profiles]` via toml_edit —
  gateway-only because Codex is Responses-API-only; also mirrors the gateway key into the user env)
- **schedules** — `read_schedules`, `run_schedule`
- **sessions** (PTY) — `session_spawn`, `session_write`, `session_resize`, `session_kill`
- **config / shell** — `read_config`, `write_config`, `export_config`, `import_config`, `app_paths`,
  `open_path`, `open_terminal`, `get_autostart`, `set_autostart`, `set_toggle_hotkey`

## Frontend (`src/`)

- **`routes/+page.svelte`** is the orchestrator. It owns tab state (`active`), holds the data
  for every tab, makes all `read_*`/`run_*` calls, lazy-loads heavy tabs on first open
  (`$effect`), and centralizes the **confirm dialog** (`askConfirm`/`doConfirm`) and the
  **run lifecycle** (`run-log` appends to the console log; `run-done` reloads the relevant
  tab's data and surfaces a toast via `lib/outcome.ts`).
- **Components** (`lib/components/`, ~42 files): one per tab — `HomeTab` (USE-1 health overview),
  `UpdatesTab`, `ForksTab`, `BackupTab`, `ProfilesTab`, `McpTab`, `EnvironmentsTab` (the «Среды»
  cross-harness tab), `SyncTab`, `ProvidersTab`,
  `PluginsTab`, `ScheduleTab`, `SessionsTab`, `SubagentsTab` (the «Субагенты» agents tab),
  `AnalyticsTab`, `SettingsTab` — plus dialogs (all
  built on `ModalShell`: `ConfirmDialog`, `RestoreDialog`, `ProfileEditDialog`, `LaunchConfigDialog`,
  `SessionLaunchDialog`, `ProviderEditDialog`, `MyProviderEditDialog`, `RouterConnectDialog`,
  `HotkeyHelp`), shell (`Sidebar`, `Console`, `WindowTitleBar`, `ToastHost`, `CommandPalette`),
  and shared widgets (`Toggle`, `Select`, `FolderField`, `DropdownMenu`, `DataTable`,
  `Sparkline`, `Spinner`, `SecretInput`, `TerminalPane`, `ComponentCard`, `StackHealthCard`).
  Popovers (`DropdownMenu`/`Select`/`FolderField`) pin to their anchor via `lib/floating.ts`
  (`use:anchored`, `position: fixed`) so they escape overflow-clipping tables/modals.
- **Support modules**: `lib/ipc.ts` (typed invoke + types), `lib/i18n/` (localization),
  `lib/outcome.ts` (run → toast), `lib/attention.ts` (sidebar badges), `lib/glossary.ts`
  (per-component help), `lib/theme.ts` (dark/light), `lib/toast.svelte.ts` (toast store),
  `lib/floating.ts` (anchored popovers), `lib/relativeTime.ts` (locale-aware “N ago”),
  `lib/running.svelte.ts` (run-state store).
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
(`profiles.last.json`, incl. a backup-freshness canary) and mutates via data-driven scripts that
read `config/profiles.json` (install/repair/add/remove/rename/recolor/set-links). Symlinking
*folders* needs admin (UAC); junctions/file-links don't — so a half-built profile can be finished
with a one-off elevated repair (`repair_profile_elevated`, or `relaunch_as_admin` to elevate the
whole app). Separately, **shared-config file links** (settings/keybindings/etc.) have their own
drift check: `Check-Integrity.ps1` → `links.last.json`, surfaced via `read_config_drift` and fixed
with `run_config_drift` (`relink` / `sync-now`).

## Gotchas

- PowerShell writes JSON with a UTF-8 **BOM** → strip `\u{feff}` before `serde_json`.
- Syncthing folder IDs are per-machine — resolve folders **by path**, never by a hardcoded ID.
- Hidden `.stignore` can't be `CREATE_ALWAYS`-written while Hidden — clear the attribute first.
- claude-code-router (ccr) may fail to start on some Node versions — that's a ccr issue, not
  Castellyn's.
