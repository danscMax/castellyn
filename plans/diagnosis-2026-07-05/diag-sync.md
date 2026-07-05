# Sync tab diagnosis — 2026-07-05

Cluster: "Синхронизация между компьютерами" (`SyncTab.svelte`) + stuck "● идёт: Синхронизация" status.

Files inspected:
- `src/lib/components/SyncTab.svelte`
- `src/routes/+page.svelte` (sync wiring: `startSync`, `onSyncApply`, `run-done` listener)
- `src/lib/running.svelte.ts` (title-bar mirror `runningStore.op`)
- `src-tauri/src/lib.rs` (`run_sync`, `read_sync`, `sync_set`, `run_config_drift`, `sync_item_lines`, path constants)

---

## Issue #2 (do first — it is the root of #1 too): stuck "● идёт: Синхронизация"

### Symptom
The window title bar shows "● идёт: Синхронизация" permanently, starting the moment the Sync
tab is first opened, and never clears across other screens. `busy` stays true app-wide.

### Root cause — 95%: `run_sync` never emits `run-done`, but `startSync` waits for one
`startSync` sets the run lock and relies on the `run-done` event to release it:

`src/routes/+page.svelte:1291`
```
function startSync(action: 'query' | 'set', enabled?: string[]) {
  if (running) return;
  running = 'sync';
  log = [action === 'set' ? t('page.sync_log_set') : t('page.sync_log_query')];
  runSync(action, enabled).catch(onSpawnErr);   // fire-and-forget, no .then/.finally
}
```
`running` is cleared ONLY inside the `run-done` listener:

`src/routes/+page.svelte:2180`
```
const id = e.payload.component;
if (running === id) running = null;
```

But the backend `run_sync` is a plain command that returns a value synchronously and does
**not** go through `spawn_streamed`, so it never emits `run-log`/`run-done`:

`src-tauri/src/lib.rs:2427`
```
/// Run a Sync-tab action: `query` (no-op; UI re-reads via read_sync) or `set` the whitelist.
#[tauri::command]
async fn run_sync(action: String, enabled: Option<Vec<String>>) -> Result<i32, String> {
    match action.as_str() {
        "query" => Ok(0),                                  // <-- returns, no run-done
        "set" => {
            let enabled = enabled.unwrap_or_default();
            tokio::task::spawn_blocking(move || sync_set(&enabled)) // <-- also just returns Ok(0)
                .await ...
        }
        ...
    }
}
```
`sync_set` (lib.rs:2396) likewise returns `Ok(0)` — no event.

Flow that traps the lock: opening the Sync tab runs the first-open effect
(`+page.svelte:1299`) → `startSync('query')` → `running='sync'` → `runSync('query')` resolves
`0` with **no run-done** → nothing ever sets `running=null`. `runningStore.op` mirrors
`running` (`running.svelte.ts:1-6`, `opName('sync')` → "Синхронизация"), so the title bar sticks
forever. The same happens on the "Обновить" button (`onSyncRefresh` → `startSync('query')`) and
on **Применить** (`onSyncApply` → `startSync('set', …)`).

Contrast: `run_config_drift` (lib.rs:1641) DOES go through `spawn_streamed(... stream_id::SYNC ...)`
(lib.rs:1659), so the drift buttons emit a proper run-done and release the lock correctly. Only the
`run_sync` path is broken. That is why drift actions look fine but plain sync/apply/refresh hang.

Note the vague label "· 1 строк" = `console.lines` count of the log array (`log=[…one line…]`),
unrelated to real progress — see UX note below.

### Fix — root cause, lazy, one place
`run_sync` is native and synchronous; do NOT bolt on a fake streamed run. Instead release the lock
where the call actually completes. `startSync` is the single caller of `runSync`, so fix it there:

```
function startSync(action: 'query' | 'set', enabled?: string[]) {
  if (running) return;
  running = 'sync';
  log = [action === 'set' ? t('page.sync_log_set') : t('page.sync_log_query')];
  runSync(action, enabled)
    .then(async () => {
      await reloadSync();
      await reloadConfigDrift();
      await reloadProfiles();
    })
    .catch(onSpawnErr)
    .finally(() => { if (running === 'sync') running = null; });
}
```
This mirrors the `run-done` handler's SYNC-branch reloads (`+page.svelte:2200-2204`) that currently
never fire for `run_sync`. Trade-off: `run_config_drift` still legitimately uses the `run-done`
path — since `startSync` never launches drift (that's `startConfigDrift`), there is no double-clear;
but confirm no other caller invokes `runSync` (grep: only `startSync` does). Keep the guard
`if (running === 'sync')` so a concurrent drift run isn't unlocked by mistake.

Alternative (heavier, not recommended): make `run_sync` go through a native streamed runner
(`run_native_streamed`, lib.rs:910) so it emits run-done like the pluginsync path. More code, no
extra benefit here since read_sync already refreshes state.

---

## Issue #1: banner says "нажмите «Применить»" but the button is disabled & far away

### Symptom
Yellow banner "требует применения — Развёрнутый .stignore не совпадает с настройками ниже — нажмите
«Применить»" is shown, but the **Применить** button (far below, under the toggles) is greyed out
and not discoverable.

### Root cause (a) — button disabled: two independent gates, neither satisfied by the banner
Banner condition (`SyncTab.svelte:253`):
```
{#if data.stignoreExists && data.stignoreMatches === false}
```
`stignoreMatches` is computed in the backend by comparing the **deployed** `~/.claude/.stignore`
against `build_stignore(saved config)` (lib.rs:2370-2373). So the banner means: *the live .stignore
on disk drifted from the saved config* — independent of any UI toggle edits.

Button condition (`SyncTab.svelte:278`):
```
<button class="sw-btn" disabled={busy || !dirty} onclick={apply} ...>
```
where `dirty` (SyncTab.svelte:54) = "local toggle selection differs from the **saved** config":
```
const dirty = $derived.by(() => {
  const items = (data?.items ?? {}) as Record<string, boolean>;
  return ITEMS.some((i) => (sel[i.key] ?? true) !== (items[i.key] !== false));
});
```
Two reasons the button is disabled while the banner is up:
1. **`busy` is stuck true** because of Issue #2 (`running='sync'` never clears). This disables the
   button AND every toggle regardless of anything else — the dominant cause in the live screenshot
   (~70%). Fixing #2 unsticks this.
2. **`!dirty`** — even with `busy` false: the banner fires on `stignoreMatches===false`, but the
   user hasn't changed any toggle, so `dirty===false` and the button stays disabled (~30%). The
   apply gate never accounts for "deployed .stignore drifted from saved config"; you literally
   cannot re-apply the already-saved config to regenerate the drifted live file.

### Fix (a) — enable when apply is genuinely needed
Include the stignore drift in the enable condition:

`SyncTab.svelte:278`
```
<button class="sw-btn" disabled={busy || (!dirty && data.stignoreMatches !== false)} onclick={apply} ...>
```
i.e. enabled when there are local edits (`dirty`) OR the deployed .stignore drifted
(`stignoreMatches === false`). `apply()` already re-sends the current selection (which, unedited,
equals the saved config) → `sync_set` regenerates canonical + live `.stignore` and rescans
(lib.rs:2417-2423), which is exactly what the banner asks for. Trade-off: none functionally — this
only widens the enable set to the case the banner already advertises. Must land together with the
#2 fix, otherwise `busy` keeps it disabled.

### Root cause (b) / UX — action is orphaned at the bottom, banner is inert text
The banner (SyncTab.svelte:253-258) is pure text + a badge; the actionable button lives ~25 lines
lower (SyncTab.svelte:277-284), after the "ЧТО СИНХРОНИЗИРОВАТЬ" grid. Telling the user to press a
button they must scroll to find, that is also disabled, is the discoverability half of the bug.

### Fix (b) — put the Применить button in the banner
Add the same `apply` action inside the drift banner so the call-to-action is where the instruction
is:
```
{#if data.stignoreExists && data.stignoreMatches === false}
  <div class="sw-card mb-sw-4 border border-amber-500/40 text-sw-sm flex items-center gap-sw-2">
    <span class="badge badge-warn">{t('sync.needsApplyBadge')}</span>
    <span>{t('sync.driftWarning')}</span>
    <button class="sw-btn ml-auto shrink-0" disabled={busy} onclick={apply}
      title={t('sync.applyTitle')}>{t('common.apply')}</button>
  </div>
{/if}
```
In the banner the button need only gate on `busy` (the banner itself already proves apply is
needed). Keep the bottom button too (it serves the `dirty` toggle-edit case). No new i18n keys
needed — reuses `common.apply`, `sync.applyTitle`. Trade-off: two apply buttons on screen when both
conditions hold; acceptable, they trigger the same `apply()`.

---

## Issue #3 (cross-cutting): hardcoded file set + Syncthing paths, fresh user has none

### What is actually hardcoded (and what is not)
- The synced-**item** whitelist (`history, projects, skills, agents, commands, keybindings,
  castellyn`) is baked into `sync_item_lines()` (lib.rs:1944-1956). These are structural
  `~/.claude/*` subpaths and are reasonable universal defaults — NOT per-user file names.
- The **config-drift file list** the task cites (statusline.py, CLAUDE.md, RTK.md, cleanup_nul.ps1,
  subagent-monitor.ps1) is **not** hardcoded in Castellyn. It is read from a cached JSON produced by
  external PowerShell (`links.last.json`, lib.rs:1483/1632), written by
  `Relink-SharedConfig.ps1` / `Check-Integrity.ps1`. So that set is defined by the user's
  ClaudeProfiles scripts, not the app.
- The genuinely user-specific hardcodes are the **paths into one particular ClaudeProfiles tree**:
  - `SYNC_CONFIG_REL = "!Настройки и MCP\\ClaudeProfiles\\config\\sync-config.json"` (lib.rs:1940)
  - `SYNC_CANON_STIGNORE_REL = "…\\config\\.stignore"` (lib.rs:1941)
  - `RELINK_SCRIPT_REL`, `CONFIG_DRIFT_JSON_REL` (lib.rs:1481-1483)
  All resolve via `abs()` = `scripts_root()` + this literal. `scripts_root()` IS configurable
  (env `SCRIPTS_ROOT` → `config.scriptsRoot` → default `E:\Scripts`, lib.rs:266), but the
  `!Настройки и MCP\ClaudeProfiles\…` suffix is fixed. A fresh user without that exact tree has no
  `sync-config.json`, no canonical `.stignore`, and no `links.last.json` — the config-drift card and
  the canonical-stignore write path silently target a directory that does not exist.
- The Syncthing side (`syncthing_status()`, folder id/label) is read from the running Syncthing
  daemon, so it is auto-detected, not hardcoded — but which folder maps to `~/.claude` still assumes
  the user's Syncthing was configured by `Configure-Syncthing.ps1` (lib.rs:10097).

### Fix direction (analysis; larger change — recommend a follow-up, not this pass)
1. **Make the ClaudeProfiles config root a config value**, not a literal suffix: add e.g.
   `config.profilesConfigDir` alongside `scripts_root`, defaulting to the current literal, so the
   sync-config / canonical-stignore / links paths derive from it. One place, mirrors the existing
   `scripts_root` pattern.
2. **Degrade gracefully when the tree is absent**: `read_sync` already tolerates a missing
   `sync-config.json` (defaults all-on, lib.rs:1965-1982); extend the same tolerance to the
   canonical `.stignore` write (skip, don't error) and hide the config-drift card when
   `links.last.json` is missing (today `read_config_drift` returns `None`, so the card is hidden —
   verify SyncTab's `{#if driftData}` already handles this: it does, SyncTab.svelte:171).
3. **The live `.stignore` whitelist is already correctly universal** (built from `sync_item_lines`
   into `~/.claude/.stignore`) — no change needed there.

Priority: (1) is the real de-hardcode; the item whitelist and the drift file list do not need
per-user config (they are structural / script-defined respectively). This is a design change worth
its own task, not folded into the #1/#2 bugfix.

---

## Recommended landing order
1. Fix #2 (`startSync` releases the lock in `.finally`) — unsticks `busy` and, with it, most of #1.
2. Fix #1a (enable button on `stignoreMatches===false`) + #1b (button in the banner).
3. Defer #3 to a follow-up (profilesConfigDir config value); low urgency for the current single-user
   setup, required before OSS release.
