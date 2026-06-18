# Castellyn

**Control center for AI coding agents.** A native desktop app (Tauri + Svelte 5 + Rust) that
unifies the upkeep and operation of a Claude Code stack today, and is growing toward managing
**multiple agents** (Claude Code, Codex, opencode…) with **remote control of open sessions**.

Castellyn is a thin native shell over the maintenance scripts under `SCRIPTS_ROOT` (default
`E:\Scripts`): the Rust backend runs them and streams output; the UI renders their status.

## Features

Ten tabs:

| Tab | What it does |
|---|---|
| **Updates** | Check / apply updates across the whole stack (plugins, forks, RTK, SpecKit, opencode, ccr, FreeLLMAPI, Cargo bins, BOM-fix) — per-component cards + an "update all" orchestrator. |
| **Forks** | Status of your GitHub forks vs. upstream (merged/open/conflict branches, PR + CI), with safe per-repo actions (fast-forward, delete-merged, rebase, normalize remotes). |
| **Backup** | Config snapshots of all profiles + restore (with a mandatory `-WhatIf` preview gate). |
| **Profiles** | Full lifecycle of Claude Code profiles (`~/.claude-<name>`): add/remove/rename/recolor, repair junction/symlink health, shared-folder links, launch in terminal/VS Code. |
| **MCP** | Source-of-truth `.mcp.json` and a per-profile deployment matrix; one-click deploy to all profiles. |
| **Sync** | What syncs between machines via Syncthing (history/projects/skills/agents/commands/keybindings) — edits `.stignore`, shows Syncthing status. |
| **Providers** | Local LLM engines (start/stop, port status) and binding a provider per profile; claude-code-router integration for OpenAI-style engines. |
| **Plugins & skills** | Update / enable / disable plugins per profile + a read-only skills overview and plugin-contents breakdown. |
| **Schedule** | Run maintenance automatically via Windows Task Scheduler. |
| **Settings** | Theme, **language (RU / EN / 简体中文)**, scripts path, autostart, timeouts, about. |

Plus: custom themed window chrome, collapsible run-log console, system tray, sidebar
"needs attention" badges, and a fully internationalized UI (live switch, no restart).

## Quick start

```bash
npm install
npm run tauri dev      # run the app with hot reload
```

Build a release exe + desktop shortcut:

```powershell
.\build_all.ps1                 # standalone exe
.\build_all.ps1 -Bundle         # + NSIS/MSI installers
```

The exe reads `SCRIPTS_ROOT` (env → Settings → default `E:\Scripts`), so it runs from anywhere
as long as the scripts are reachable.

## Project layout

```
src/                     SvelteKit frontend (Svelte 5 runes)
  routes/+page.svelte    orchestrator (tabs, IPC calls, confirm, toasts)
  lib/components/        one component per tab + dialogs + shell
  lib/i18n/              localization (ru/en/zh, per-namespace dicts)
  lib/ipc.ts             typed invoke() wrappers + types
src-tauri/               Rust backend
  src/lib.rs             all #[tauri::command]s (spawn_streamed, readers, config, tray)
  icons/                 app icons (master: icon.png)
manifest/                canonical component manifest (read at runtime)
tools/                   ScriptKit.ps1 (status helper), make-icon.py
docs/                    architecture, i18n, build
build_all.ps1 / .bat     one-command release build
```

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — how it fits together.
- [docs/I18N.md](docs/I18N.md) — localization and adding strings/locales.
- [docs/BUILD.md](docs/BUILD.md) — build, release, icon, troubleshooting.
- [CLAUDE.md](CLAUDE.md) — guidance for AI assistants working in this repo.

## Tech

Tauri v2 · SvelteKit (static/SPA) · Svelte 5 runes · TypeScript · Tailwind · Rust
(serde, tokio) · PowerShell maintenance scripts. Windows-first.
