# Cluster C — Sessions, Personalization, Backup, Hooks, Cross-machine

Read-only investigation, 2026-07-03. Every claim carries `file:line` evidence. Items that cannot
be confirmed from code in *this* repo are marked **UNVERIFIED** with the reason.

Two structural facts frame the whole cluster:

- **Sessions personalization + live-session restore live ONLY in webview `localStorage`** (keys
  `cmh-*`). None of it is in `HubConfig` (`%APPDATA%\castellyn\config.json`) and none of it is under
  `~/.claude`, so it is neither backed up nor cross-machine synced. This is the item-18 gap and it
  compounds across topics 2, 3 and 5.
- **Castellyn is a control panel, not a sync/backup engine.** The actual backup is an external
  PowerShell script under `SCRIPTS_ROOT`; the actual cross-machine transfer is Syncthing (external).
  Castellyn spawns/reads them.

---

## 1. Session transcripts / restore across an app restart

**WHAT** — Live Sessions panes survive a webview reload and offer a restore bar after a full app
restart; Claude panes resume their conversation via a captured `--resume <id>`.

**WHERE**
- `LIVE_KEY = 'cmh-sessions-live'` — `src/lib/components/SessionsTab.svelte:344`.
- Persisted shape `LivePane` (tool, profile, cwd, args, remoteDir, sshTarget, id, **claudeSid**, name)
  — `SessionsTab.svelte:345`.
- Snapshot captured synchronously at init, before the persist effect overwrites it — `:348-354`.
- Persist effect: writes live panes; while a restore offer is pending it *also* keeps the last run's
  dead panes in `LIVE_KEY` so a second reload doesn't lose the offer — `:355-371`.
- On mount: `sessionList()` (backend) splits saved panes into **alive → re-attach as owner** and
  **dead → `restorable`** (a dead set == full app restart, the PTYs died with the process) — `:136-152`.
- Restore bar UI (`restoreOffer` / `restoreLast` / dismiss) — `:1351-1358`.
- `claudeSids` captured from the run event payload — `:246`, `:254`
  (`if (e.payload.claudeSessionId) claudeSids = {...}`).
- `--resume` injection at launch — `:1088-1094`: only for `tool==='claude' && !sshTarget`, a
  `claudeSid` matching `/^[\w-]{1,64}$/`, and only when the user's args don't already carry
  `--resume`/`--continue`; then `args = \`${args} --resume ${s.claudeSid}\``.
- The Claude session id is filled by the status hook: `castellyn_status.py:44` writes
  `claudeSessionId` from the hook stdin; the pane pairs its own id via `CASTELLYN_SESSION_ID`
  (`lib.rs:9373`).

**SHARED?** — Live-restore state is per-webview `localStorage`, i.e. per-machine, per-app-data-dir.
NOT in HubConfig, NOT backed up, NOT synced. The **transcripts** themselves live in Claude Code's own
store `~/.claude/projects/…`; `--resume <sid>` just hands the id to `claude`, which resolves the
transcript from that store.

Cross-**profile** transcript sharing (memory note "projects = junction → --resume works
cross-profile"): the only in-repo evidence is that `projects` is a Syncthing/link-managed item
(`lib.rs:1795`, `("projects", "!/projects")`) and profiles share config via
`Relink-SharedConfig.ps1` / `Install-ClaudeProfiles.ps1` (`lib.rs:1353,1358`). **UNVERIFIED here**
that `~/.claude/projects` is specifically a junction — that link is created by those external profile
scripts, not by any code in this cluster.

**AUTO** — Automatic: reload re-attach and the persist effect run on their own. The restore bar after
a full restart is a **one-click prompt**, deliberately not auto-spawned (comment `:145-146`,
`:189-190`). Stale `blocked`/idle badges and the claude-sid feed come from the hook automatically.

**GAP**
- Restore is best-effort and local only; a machine migration or a cleared webview store loses the
  entire live-session set and every captured claude-sid.
- Cross-profile resume depends on an external junction this cluster's code neither creates nor checks.

---

## 2. Sessions personalization — the `cmh-*` localStorage keys (item-18)

**WHAT** — Every Sessions preference is a `localStorage` key. Complete inventory
(all in `src/lib/components/SessionsTab.svelte` unless noted):

| Key | Line | Holds |
|-----|------|-------|
| `cmh-sessions-folders` (FKEY) | `:94`, read `:120`, write `:228` | last-used folder per tool/profile |
| `cmh-sessions-cols` (CKEY) | `:95`, `:126`, write `:219` | grid column count |
| `cmh-sessions-workspaces` (WKEY) | `:96`, `:121`, write `:1056` | **saved workspaces** (named session sets) |
| `cmh-sessions-defargs` (DAKEY) | `:97`, `:122`, write `:569` | default launch args |
| `cmh-remote-recent` (RRKEY) | `:98`, `:125`, write `:843` | recently-used SSH remote start dirs |
| `cmh-monitor-layout` (MLKEY) | `:99`, `:192`, write `:481` | "разнести по мониторам" arrangement |
| `cmh-sessions-favorites` (VKEY) | `:908`, `:123`, write `:944` | favorites |
| `cmh-projects-root` (ROOT) | `:721`, `:124`, write `:731,:1281` | projects-root quick-pick |
| `cmh-sessions-colfr` (COLFR_KEY) | `:616`, `:619`, write `:630` | per-column width fractions |
| `cmh-sessions-fontsize` | `:128` | terminal font size |
| `cmh-sessions-launcher` | `:130`, write `:562` | launcher panel open/closed |
| `cmh-recent-folders` | `:541-543` | recent cwd folders |
| `cmh-sessions-live` (LIVE_KEY) | `:344` | live-session restore set (topic 1) |
| `cmh-backup-keep` | `BackupTab.svelte:96,:104` | Backup tab retention count |

Note: **saved SSH hosts** are the exception — loaded via `readSshHosts()` (backend, `:154`, imports
`~/.ssh/config`), not localStorage. Only the *recently-used remote dirs* (`cmh-remote-recent`) are
localStorage.

**SHARED?** — No. All `cmh-*` keys are webview `localStorage`. Confirmed as the single storage path
(no `HubConfig`/backup mirror exists for any of them).

**AUTO** — Written automatically on each change; read on mount.

**GAP (item-18)** — Workspaces, favorites, projects-root, column layout, default args, monitor
layout, recent remote dirs — all vanish on OS reinstall, machine migration, or a webview data wipe.
None is in `HubConfig`, none is in the backup snapshot, none is under `~/.claude` (so Syncthing never
sees it). This is the same gap seen from three angles across topics 1/3/5.

---

## 3. Backup / export (BackupTab + backup command)

**WHAT** — The Backup tab creates timestamped snapshots + weekly zip archives of the **Claude Code
setup** and restores them, scoped by profile and (optionally) credentials.

**WHERE**
- UI: `src/lib/components/BackupTab.svelte` — `doBackup()` → `onAction('backup', {keepSnapshots})`
  (`:102-109`); retention default 30 stored in `cmh-backup-keep` (`:93,:96,:104`); snapshot list,
  weekly-archive list (verify/extract/reveal/delete), freshness badge.
- Backend command `run_backup` — `src-tauri/src/lib.rs:1332`; arg builder `backup_args` — `:1272`.
- **The scripts are external, under `SCRIPTS_ROOT`, NOT in this repo:**
  `BACKUP_SCRIPT_REL = "!Настройки и MCP\\ClaudeProfiles\\Backup-ClaudeSetup.ps1"` (`lib.rs:1121`);
  `RESTORE_SCRIPT_REL = "…\\Restore-ClaudeSetup.ps1"` (`:1122`).
- `list_backups` reads `Backups/` snapshots + `weekly-*.zip` + `.backup-state.json` — `:1159-1211`.
- Native ops: `reveal_backup` `:1213`, `delete_backup` `:1225`, `verify_backup` `:1234`,
  `extract_backup` `:1251`.
- Backup folder exposed to Settings → About: `"backupDir": abs(BACKUP_DIR_REL)` — `:8386-8387`.

**Args actually passed** (`backup_args`, `:1279-1326`):
- `backup` → `-Force` [`-KeepSnapshots N`, floored at `max(1)` so "keep none" is impossible].
- `restore-preview` → `-WhatIf` (safe dry-run path first).
- `restore` → optional `-Timestamp`, `-Profiles …`, and `-IncludeCredentials` **only when explicitly
  requested** (`:1323-1325`, defaults OFF).

**SHARED?** — Snapshots land under `Backups/` in `SCRIPTS_ROOT` (`abs(BACKUP_DIR_REL)`), a local dir.
Not itself cross-machine unless that dir is separately synced (out of scope here).

**AUTO** — Manual (button-triggered). Freshness badge in the UI nudges when stale (`BackupTab.svelte:129-143`,
fresh ≤2d / staling ≤7d / stale). No scheduled auto-backup wired in this cluster.

**GAP**
- **WHAT is inside a snapshot is UNVERIFIED from this repo** — the file/dir list lives entirely in the
  external `Backup-ClaudeSetup.ps1`. From the Castellyn side only these are provable: restore is
  profile-scoped, and **credentials are EXCLUDED unless `-IncludeCredentials` is explicitly set** — so
  the default restore does not write secrets. Good, but confirm the backup half's inclusion/exclusion
  by reading the external PS1.
- The backup targets the Claude setup; it does **not** include Castellyn's own Sessions `cmh-*`
  localStorage prefs (they live in the webview store, outside `~/.claude` and outside `Backups/`).
  So item-18 personalization is neither synced nor backed up.

---

## 4. The two hooks — `plugin_sync.py` and `castellyn_status.py`

### 4a. `plugin_sync.py` (SessionStart)

**WHAT** — Propagates newly-installed plugins across all CC profiles: any plugin enabled (`True`) in
ANY profile is added to every profile **missing the key entirely**; an explicit `False` is an
intentional opt-out and is never touched; marketplaces propagate via `setdefault`. Atomic +
only-if-changed writes, fail-open. — `src-tauri/assets/plugin_sync.py:1-104` (reconcile `:33-77`).

**WHERE / wiring**
- Wired into **SessionStart only** — `plugin_sync_wire` marker `"plugin_sync.py"` (`lib.rs:7975-7982`),
  cmd `PLUGIN_SYNC_HOOK_CMD = "py -X utf8 ~/.claude/hooks/plugin_sync.py"` (`:7815`).
- `PROFILES` list is generated by Castellyn (`plugin_sync.py:23` `PROFILES = [".claude"]  # castellyn:profiles`),
  rendered by `render_plugin_sync_script` (`lib.rs:7836`). A directory scan is deliberately avoided so
  sibling dirs like `~/.claude-mem` are never touched (`plugin_sync.py:14-15`).
- Version-gated install: `ensure_plugin_sync_script` writes only if disk version < embedded
  (`lib.rs:7859-…`); header `# plugin-sync-version: 2`.
- Toggle `plugin_sync_set(enabled)` (`lib.rs:8026`), run-now `run_plugin_sync … --verbose` (`:8066`).

### 4b. `castellyn_status.py` (5 lifecycle events)

**WHAT** — Reports the semantic state of a Castellyn-owned pane by writing a tiny JSON file the app
watches: `%APPDATA%\castellyn\agent-status\<sid>.json` with `{state,event,claudeSessionId,ts}`. State
map `SessionStart→idle, UserPromptSubmit→working, Notification→blocked, Stop→idle, SessionEnd→ended`
(`assets/castellyn_status.py:16-22`, write `:25-51`). **No-op unless `CASTELLYN_SESSION_ID` is in the
env** and alnum ≤32 chars (`:26-27`) — so ordinary Claude use outside Castellyn is unaffected.

**WHERE / wiring**
- `STATUS_HOOK_EVENTS = [SessionStart, UserPromptSubmit, Notification, Stop, SessionEnd]`
  (`lib.rs:8108-8114`); cmd `STATUS_HOOK_CMD` (`:8106`), marker `"castellyn_status.py"` (`:8107`).
- Toggle `agent_status_hook_set(enabled)` wires/unwires all 5 events across profiles, continue-on-error
  (`lib.rs:8165-8204`); state report `agent_status_hook_state` requires ALL five wired to count a
  profile "wired" (`:8135-8155`).
- Consumed by `src-tauri/src/agent_status.rs` (poll thread → pane badges + OS/sound notifications).

**SHARED?** — Both hooks operate on `~/.claude/…/settings.json` per profile; the status output dir is
local (`%APPDATA%\castellyn\agent-status`). Not cross-machine.

**AUTO** — Enable/disable is manual (toggles). Once wired, both fire automatically on the profile's
lifecycle events. Status files older than 7 days are pruned automatically at app start
(`agent_status.rs:349-365`); recent ones are kept because their claude-sid feeds session restore.

**GAP (item-#5, refined by evidence)**
- Settings.json entries **are** cleaned on disable: `hook_cmd_unwire` removes the marker-matching
  command *and* collapses any entry left empty (`lib.rs:7940-7969`, the `retain` at `:7960-7967`). So
  there is **no leftover empty hook stanza** for tracked profiles — contrary to the naive expectation.
- The genuine leftovers on disable:
  1. **The `.py` script files are never deleted.** `plugin_sync_set(false)` / `agent_status_hook_set(false)`
     only unwire settings; `ensure_*_script` only ever writes, never removes. So
     `~/.claude/hooks/plugin_sync.py` and `…/castellyn_status.py` stay on disk after disable. Harmless
     (unwired ⇒ never invoked; the status hook also self-no-ops without the env var) but orphaned.
  2. **Unwire only sweeps CURRENT profiles** (`plugin_sync_profiles(&home)`, `:8035`, `:8174`). If a
     profile was wired and later removed from Castellyn's profile list (or its dir renamed outside
     Castellyn), disabling never reaches that orphaned `settings.json` → its hook entry is stranded.
     This is the real "leftover hook entries" case and it is not handled.

---

## 5. Cross-machine sync — Syncthing (in-scope control panel vs external engine)

**WHAT** — Castellyn does **not** bundle, install, or run Syncthing. It (a) reads Syncthing's local
config + REST API for a read-mostly status view, and (b) owns the `.stignore` whitelist that decides
*which* parts of `~/.claude` sync.

**WHERE — read-side (status)**
- `syncthing_api_key` reads `%LOCALAPPDATA%\Syncthing\config.xml` `<apikey>` — `lib.rs:2001-2008`.
- `st_agent` ureq client, 1.5s timeout — `:2010-2015`; `st_get` hits `http://127.0.0.1:8384` with
  `X-API-Key` (**host+port hardcoded**, `:2018`).
- `st_claude_folder` finds the Syncthing folder whose path == `~/.claude` — `:2035-2049`.
- `syncthing_status` reports `available / keyConfigured / version / folderShared / folderLabel /
  folderId / state / completion / connectedDevices` — `:2051-2125` (`keyConfigured:true` lets the UI
  tell "configured but daemon down" from "not configured").
- The ONLY write to Syncthing: `syncthing_rescan` → `POST /rest/db/scan`, best-effort — `:2128-2139`.

**WHERE — write-side (what syncs)**
- Whitelist items `sync_item_lines()`: `history.jsonl`, **`projects`**, `skills`, `agents`,
  `commands`, `keybindings.json` — `lib.rs:1792-1801`.
- `build_stignore` = volatile-only ignores (`.git/index.lock`, `*.sync-conflict-*`, `~syncthing~*`,
  `.stversions`) + the enabled whitelist — `:1831-…`.
- `sync_set` writes `config\sync-config.json` (backed up first) + canonical `config\.stignore` +
  live `~/.claude\.stignore`, then rescans — `:2175-2204`.
- Source-of-truth config `SYNC_CONFIG_REL = "…\\ClaudeProfiles\\config\\sync-config.json"` (`:1788`).

**SHARED? / division of labour**
- **Castellyn's scope:** the `.stignore` whitelist (what syncs) + a read-only status dashboard + a
  best-effort rescan trigger.
- **External (out of scope):** the actual file transfer, device pairing, and conflict resolution —
  all Syncthing's job. Castellyn never moves bytes between machines itself.
- The `projects` whitelist line (`:1795`) is what lets `~/.claude/projects` (Claude Code transcripts)
  sync between machines, which is how a `--resume` (topic 1) can find a conversation started on
  another machine — ties topics 1 and 5 together.

**AUTO** — Status is read on demand (`read_sync`, `:2143`); toggling the whitelist is manual and
triggers an immediate rescan. Transfers themselves run on Syncthing's own schedule.

**GAP**
- Sessions `cmh-*` localStorage prefs are **not** under `~/.claude`, so Syncthing never carries them —
  no cross-machine personalization (item-18 again).
- Syncthing config path and REST endpoint are hardcoded (`%LOCALAPPDATA%\Syncthing`,
  `127.0.0.1:8384`, `:2003,:2018`); a non-default install/port makes Castellyn report "not
  configured" even when Syncthing is running.

---

## Cross-cutting takeaway (item-18)

The recurring theme: **Sessions personalization has no home outside webview localStorage.** It is
absent from `HubConfig`, from the backup snapshot, and from the Syncthing whitelist (it isn't under
`~/.claude`). Closing item-18 means giving `cmh-*` prefs a durable home — either mirror them into
`HubConfig` (then they ride the backup) or into a synced file under `~/.claude` (then Syncthing carries
them). Either path also needs an import on first run so an existing machine's prefs survive migration.

### Unverified / to confirm outside this cluster
- Exact file/dir list a backup snapshot contains — read the external `Backup-ClaudeSetup.ps1`.
- That `~/.claude/projects` is a *junction* enabling cross-profile resume — created by external
  profile scripts (`Install-ClaudeProfiles.ps1` / `Relink-SharedConfig.ps1`), not this code.
