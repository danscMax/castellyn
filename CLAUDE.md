# CLAUDE.md — Castellyn

Project-specific guidance for Claude Code working in this repository. Supplements the
user's global `~/.claude/CLAUDE.md` (security, PowerShell-Cyrillic, DRY, workflow rules).

> Display brand **Castellyn** (renamed from AgentHub 2026-06-18). Folder: `E:\Scripts\Castellyn`
> (code/manifest path-literals resolve `<SCRIPTS_ROOT>\Castellyn\tools\…`).
> Internal identifiers were renamed `agenthub` → `castellyn` 2026-06-20: npm/crate name `castellyn`,
> binary `castellyn.exe`, Tauri id `com.danscmax.castellyn`, config dir `%APPDATA%\castellyn`,
> keyring keys `castellyn.*`, autostart Run value `Castellyn`. The old `agenthub`/`AgentHub` names are
> still **read as a migration fallback** so existing user data survives — do NOT remove these:
> config-path chain (`config_path` → `agenthub_config_path` → `legacy_config_path`), lazy keyring
> re-home in `kr_get`/`kr_delete` (`legacy_kr_service`), one-time `migrate_autostart` in `setup()`.
> Sentinels deliberately KEPT on the old name (internal, not brand-visible; must stay in sync with PS
> / tests): `agenthub-local` dummy auth token (matches `Manage-Provider.ps1`), `agenthub:<name>`
> freellmapi label, `agenthub-pty-probe` test string.

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
- **Backend** is still mostly one file: `src-tauri/src/lib.rs` holds the bulk of the
  `#[tauri::command]`s, with coherent groups peeled off into their own modules over time
  (`links.rs`, `profiles_status.rs`, `limits.rs`, `agent_status.rs`, `agent_schedules.rs`,
  `worktree.rs`, `session_bus.rs`, `stack_health.rs`, `schedules_watch.rs`, `mcp_probe.rs`,
  `ssh_hosts.rs`). Peeling off one more group is the established move — `ssh_hosts.rs` is the
  pattern to copy. **The registration list `generate_handler![…]` is the invariant**: a command
  that silently falls out of it breaks at runtime with nothing red beforehand, so count it before
  and after any move.
  - `pump_and_wait(...)` is the single streaming primitive (child stdout/stderr → `run-log`,
    exit → `run-done`). Reach it through an existing wrapper: `spawn_streamed` → `_io` → `_prog`
    for a script under the single-run `RunState` guard, `spawn_pwsh_phase` / `spawn_stack_phase`
    for stack phases, `run_fork_repo` for the concurrent per-repo fork runs. `cancel_run` kills
    the tree. It also joins every child to the kill-on-close Job Object, so a hard crash cannot
    orphan a script tree — don't add a spawn path that bypasses it.
  - Native (no-script) readers exist where cheaper: `read_mcp`, `read_providers`,
    `port_listening`, plugin/skill scans. `probe_mcp` (`mcp_probe.rs`) goes further and actually
    handshakes with a configured MCP server — opt-in only, never on render.
  - **All process spawns set `CREATE_NO_WINDOW`** (0x08000000) — otherwise a black console
    flashes. Keep this on every new `Command`.
  - **Errors that reach the UI go through `tr`/`trv` with `err.*` keys** in `src-tauri/src/i18n.rs`
    (ru/en/zh). That mechanism already exists in ~165 places — do NOT add `thiserror`, an error
    enum or error codes beside it. A raw `format!` string reaching the user is the bug.
  - **Types shared with the frontend are generated, not mirrored**: five structs derive `ts-rs`
    and export into `src/lib/generated/` during `cargo test`; `src/lib/ipc.ts` re-exports them.
    Mirror a struct by hand only if it has no Rust source (the PowerShell-produced payloads).
  - Config: `HubConfig` in `%APPDATA%\castellyn\config.json` (`config_path()`), with a
    legacy-path read fallback (`legacy_config_path`) kept for the pre-rename location.
  - Autostart: HKCU\…\Run value `Castellyn` (`AUTOSTART_NAME`); migrated once from `AgentHub`.
  - Tray menu labels are **localized** via `src-tauri/src/i18n.rs` (`tr("tray.*", lang)`, used in
    `build_tray`); `set_language` relabels the tray live when the UI locale changes.
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

- **DRY**: search before adding. Backend → every streamed run must terminate in `pump_and_wait`;
  never add a second streaming path. Frontend → reuse `common.*` i18n keys, `askConfirm`,
  existing components.
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
npm run verify         # ALL gates in order (verify.ps1) — the single source of truth
npm run smoke          # walk every tab of a RUNNING isolated instance over CDP (see below)
.\build_all.ps1        # release exe (castellyn.exe) + desktop shortcut (Castellyn.lnk)
```

Green gates before declaring done: `npm run verify`, and a release build via `build_all.ps1`.
Don't hand-maintain the gate list here — read `verify.ps1`. See `docs/BUILD.md`.

**The gates are blind to runtime.** Every one of them passes while a tab fails to render, so a
UI change is not done until `tools/smoke.mjs` has walked it:

```bash
pwsh -File tools/iso-test.ps1 -World   # bring up the sandbox (+ -Build after Rust changes)
npm run smoke                          # exit 0 pass · 1 a tab is broken · 2 nothing to attach to
pwsh -File tools/iso-test.ps1 -Stop
```

It prints the rendered character count per tab, which doubles as a cheap refactor check: a pure
move should leave those counts identical.

## Isolated test instance (safe full click-through)

`tools/iso-test.ps1` spins up Castellyn with its OWN `%APPDATA%`/`%LOCALAPPDATA%` (config isolated
in a scratch dir) and a dedicated CDP port (9223; the real dev uses 9222), so a Playwright clicker
can drive the whole UI without corrupting the real Castellyn config or touching the real browser.

```bash
pwsh -File tools/iso-test.ps1            # start (reuses scratch profile); prints CDP + recipe
pwsh -File tools/iso-test.ps1 -World     # FULL sandbox: fake home/scripts/forks/agent-CLIs — ALL buttons safe
pwsh -File tools/iso-test.ps1 -Fresh     # start with a clean profile (onboarding appears)
pwsh -File tools/iso-test.ps1 -Build     # cargo build the exe first, then start
pwsh -File tools/iso-test.ps1 -Stop      # tear down (kills exe + frees vite 1420)
```

- **Isolation boundary (default, БЕЗ `-World`):** only Castellyn's own config lives under
  `%APPDATA%\castellyn`. The profiles it manages (`~/.claude*`, `~/.ssh/config`, forks under
  `SCRIPTS_ROOT`, providers) are the REAL filesystem — nothing that WRITES the real system is safe.
- **`-World` (full sandbox):** `tools/iso-world.ps1` builds a fake world (`%TEMP%\castellyn-iso\world`:
  three `.claude*` profiles with dummy creds, stub maintenance scripts emitting real envelopes with
  `ISO_OUTCOME=ok|changes|error|held`, SettingsMCP tree, two real git fork repos + `fork-sync.last.json`
  fixture, echo-TUI stubs `claude`/`codex`/`opencode` + fixture `gh`) and the harness redirects
  USERPROFILE/SCRIPTS_ROOT/CASTELLYN_SETTINGS_DIR/PATH into it, with `CASTELLYN_ISO=1` making the two
  non-env side channels file-backed (HKCU autostart + OS credential store; window titles
  "[ISO SANDBOX]"). In this mode EVERY button/action is safe — maintenance runs, git actions,
  session launches all hit only the sandbox. Gotcha: WebView2 150+ needs the explicit
  `WEBVIEW2_USER_DATA_FOLDER` the harness sets, or CDP silently never opens.
- **Port constraint:** vite is pinned to 1420 (`strictPort`) and the debug exe loads `devUrl=1420`,
  so the isolated instance and your own `npm run tauri dev` CANNOT run at the same time.
- The full safe click-through procedure (spawn a cheap clicker agent + safety rules + findings
  format) is the `/max:max-castellyn-iso-audit` skill.

## Icon / branding

Production icon master is `src-tauri/icons/icon-master.png` (1024×1024 citadel/gatehouse
emblem). Regenerate every format from it with
`npm run tauri -- icon src-tauri/icons/icon-master.png` — this overwrites `icon.png`,
`icon.ico`, `icon.icns` and the `Square*` set. Brand blue `#3b82f6 → #2563eb`.
`tools/make-icon.py` is **only an offline fallback**: it draws the LEGACY hub-and-nodes mark
with Pillow, not the shipped emblem — feeding its output to `tauri icon` replaces the app icon.
Details in `docs/BUILD.md`.

## Docs

- `docs/ARCHITECTURE.md` — layout, IPC, tabs, status envelope, profiles model.
- `docs/I18N.md` — how localization works + adding strings/locales.
- `docs/BUILD.md` — build, release, icon, troubleshooting.
- `plans/` — local working directory for design specs, audit reports and agent artifacts.
  **Gitignored and untracked**: write here freely, nothing is committed. Older plans live in the
  history (before the untracking commit) if you need them.
