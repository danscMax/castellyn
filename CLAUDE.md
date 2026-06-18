# CLAUDE.md — Castellyn

Project-specific guidance for Claude Code working in this repository. Supplements the
user's global `~/.claude/CLAUDE.md` (security, PowerShell-Cyrillic, DRY, workflow rules).

> Display brand **Castellyn** (renamed from AgentHub 2026-06-18). Folder renamed too:
> `E:\Scripts\Castellyn` (code/manifest path-literals now resolve `<SCRIPTS_ROOT>\Castellyn\tools\…`).
> These internal identifiers are deliberately KEPT on the old `agenthub` name to preserve user
> data: npm/crate name `agenthub` (registries require lowercase) + binary `agenthub.exe`, Tauri
> identifier `com.danscmax.agenthub`, config dir `%APPDATA%\agenthub`, keyring keys `agenthub.*`.

## What this is

**Castellyn** is a desktop control center for a local AI-coding dev environment. Today it unifies the
maintenance of a Claude Code stack (updates, GitHub forks, profiles, MCP servers, sync,
providers/engines, plugins, schedules); it is growing toward **multiple agents** (Claude
Code, Codex, opencode…) and **remote control of open sessions**.

It is a thin, native shell around the user's existing PowerShell maintenance scripts under
`SCRIPTS_ROOT` (default `E:\Scripts`): the Rust backend spawns those scripts and streams their
output; the Svelte UI renders their `*.last.json` status envelopes.

## Stack & architecture

- **Tauri v2** single binary. Frontend **SvelteKit (static adapter, SPA) + Svelte 5 runes**;
  backend **Rust** (`src-tauri/`). No DB, no sidecar process.
- **Backend** is essentially one file: `src-tauri/src/lib.rs` — all `#[tauri::command]`s.
  - `spawn_streamed(...)` is the single DRY entry for running a script and streaming
    `run-log` / `run-done` events to the UI. One run at a time; `cancel_run` kills the tree.
  - Native (no-script) readers exist where cheaper: `read_mcp`, `read_providers`,
    `port_listening`, plugin/skill scans.
  - **All process spawns set `CREATE_NO_WINDOW`** (0x08000000) — otherwise a black console
    flashes. Keep this on every new `Command`.
  - Config: `HubConfig` in `%APPDATA%\agenthub\config.json` (`config_path()`), with a
    legacy-path read fallback (`legacy_config_path`) kept for the pre-rename location.
  - Autostart: HKCU\…\Run value `AgentHub` (`AUTOSTART_NAME`).
  - Tray menu strings are **hardcoded Russian** and NOT internationalized (separate surface).
- **Frontend** (`src/`):
  - `routes/+page.svelte` — the orchestrator: tab state, all `run_*`/`read_*` calls,
    the confirm dialog (`askConfirm`/`doConfirm`), run-log + toasts.
  - `lib/components/*.svelte` — one component per tab + dialogs + shell (Sidebar, Console,
    WindowTitleBar, ConfirmDialog, ToastHost, Toggle, DropdownMenu).
  - `lib/ipc.ts` — typed `invoke` wrappers + shared types.
  - `lib/i18n/` — localization (see `docs/I18N.md`).
  - `lib/outcome.ts` — maps a finished run to a toast; `lib/attention.ts` — sidebar badges;
    `lib/glossary.ts` — per-component help text; `lib/theme.ts` — dark/light.
- **Custom window chrome**: `decorations: false` + `WindowTitleBar.svelte` (drag region,
  min/max/close), repaints with the theme.

## The component model

`manifest/maintenance-manifest.json` is the canonical list of maintenance components, read
**from disk at runtime** (`manifest_text()`), with an embedded copy as fallback. Current
components: `all` (orchestrator) + `plugins`, `forks`, `rtk`, `speckit`, `opencode`,
`ccrrouter`, `freellmapi`, `cargo`, `bomfix`.

**Status envelope** (the contract every script writes to `<id>.last.json`):
`{ schemaVersion, component, status: ok|changes|error|held, timestamp, mode, durationSec,
counts:{changed,failed,total}, summary }`. Scripts emit it via `Write-StatusJson` in
`tools/ScriptKit.ps1` (the vendored, auto-synced helper — canonical copy lives here).

## Conventions (follow these)

- **DRY**: search before adding. Backend → reuse `spawn_streamed`; never add a second
  streaming path. Frontend → reuse `common.*` i18n keys, `askConfirm`, existing components.
- **i18n**: every user-facing string goes through `t('ns.key')`. Keep ru/en/zh in parity
  (enforced by `npm run check:i18n` + `src/lib/i18n/index.test.ts`). **Never** name an
  `{#each … as t}` loop var or a function param `t` — it shadows the translation function.
- **JSON from PowerShell**: scripts may write UTF-8 **with BOM**; strip `\u{feff}` before
  `serde_json` (helpers already do). Castellyn's own writers use UTF-8 **without** BOM.
- **Destructive actions**: gate behind a confirm dialog; scripts must run non-interactively
  (`-Yes -Unattended`, never `Read-Host`). Prefer a `-WhatIf`/preview path first.
- **Don't click-test the GUI blind**: validate via builds/tests + reading `*.last.json`;
  real destructive runs (install/restore/reinstall) are left to the user.
- **No AI-attribution** anywhere (per global rules).

## Build / dev / verify

```bash
npm install            # first time
npm run dev            # vite dev (frontend only)
npm run tauri dev      # full app, hot reload
npm run check          # svelte-check (type + i18n shape gate) — keep 0/0
npm test               # vitest (i18n parity, outcome, attention)
npm run check:i18n     # ru/en/zh leaf-key parity (tsx)
npm run build          # frontend → build/
.\build_all.ps1        # release exe (agenthub.exe) + desktop shortcut (Castellyn.lnk)
```

Green gates before declaring done: `npm run check` (0/0), `npm test`, `npm run build`, and a
release build via `build_all.ps1`. See `docs/BUILD.md`.

## Icon / branding

App icon master is `src-tauri/icons/icon.png` (1024). Regenerate all formats with
`python tools/make-icon.py` → `npm run tauri -- icon <printed path>`. Brand blue
`#3b82f6 → #2563eb`.

## Docs

- `docs/ARCHITECTURE.md` — layout, IPC, tabs, status envelope, profiles model.
- `docs/I18N.md` — how localization works + adding strings/locales.
- `docs/BUILD.md` — build, release, icon, troubleshooting.
- `plans/` — design specs (historical).
