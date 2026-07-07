# Monitoring Trust (Ф1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the LLM-stack dashboard from lying — kill the `Update-FreeLLMAPI.ps1` false-green + `:3001` landmine, and make service death visible without a manual refresh via a background health-poll loop.

**Architecture:** Two independent backend fixes + one small frontend wire. (1) Harden the PowerShell updater so a missing remote ref reports `error` instead of crashing into a stale `ok`, and read the port from `.env` instead of the hardcoded `:3001`. (2) A Rust background thread (mirroring `limits::start`) polls `read_stack_health_blocking()` every 30s, emits the full health list to the UI, and logs services that transition to down. (3) `StackHealthCard.svelte` subscribes to that event so its dots update live.

**Tech Stack:** Rust (Tauri v2, `std::thread` + `tauri::Emitter`), Svelte 5 runes + `@tauri-apps/api/event`, PowerShell 5.1.

## Plan set (this is Plan 1 of 5)

This subsystem-decomposed effort ships one working slice per plan (see `plans/omniroute-stack/DESIGN.md`):
1. **Monitoring Trust (Ф1)** — THIS PLAN. Independent of OmniRoute.
2. Per-account health + attention/toasts (Ф2+Ф3) — read `providers status --json` / engine health endpoints, surface "recreate account".
3. OmniRoute integration (Ф4 split `id 'gateway'`, Ф5 front-critical health, Ф6 relax direct-openai arm, Ф7 register providers).
4. Fork-update consolidation (Ф9) + zcode git-clone migration (Ф10).
5. freellmapi retire (Ф11) + second-dev docs (Ф12).

## Global Constraints

- **Comments in English.** (per `~/.claude/CLAUDE.md`)
- **All `Command` spawns set `CREATE_NO_WINDOW`** (0x08000000). (No new spawns in this plan, but honor it if one is added.)
- **i18n parity ru/en/zh** enforced by `npm run check:i18n` + `src/lib/i18n/index.test.ts`. **This plan adds NO user-facing strings** (the live dot update reuses existing labels; the prominent toast/badge is Plan 2) — keep it that way so `check:i18n` stays green.
- **Never name an `{#each … as t}` var or a param `t`** — shadows the translation function.
- **Green gates before "done":** `npm run check` (0/0), `npm test`, `cargo test`, `cargo clippy` (0 warnings), `npm run build`.
- **Config path:** `%APPDATA%\castellyn\config.json` via `config_path()`; `HubConfig` toggles are `Option<bool>` defaulting on (mirror `limits_monitor`).
- Cargo is not on PATH — invoke via full path; trust `$LASTEXITCODE`, not PowerShell's false exit-1 (see `[[cargo-windows-invocation]]`).

---

### Task 1: Harden Update-FreeLLMAPI.ps1 (kill false-green + `:3001` landmine)

**Files:**
- Modify: `E:\Scripts\SettingsMCP\ClaudeProfiles\Update-FreeLLMAPI.ps1` (port `:59`, rev-parse guard `:84-86`, blind kill `:176-178`)

**Interfaces:**
- Consumes: nothing (standalone maintenance script).
- Produces: an honest `freellmapi.last.json` status envelope (`error` on a missing remote ref, never a stale `ok`); health/smoke targets the real `.env` `PORT` (13001), not `:3001`.

**Context — the two confirmed bugs (from the analysis):**
1. On branch `wip-local` the script runs `git rev-parse --short origin/wip-local`, but `origin/wip-local` does not exist → stdout is empty → `.Trim()` throws → the `finally` block writes `freellmapi.last.json` with the **default `status='ok'`** (a false green; upstream fixes silently never land).
2. `$healthUrl` is hardcoded `http://localhost:3001/` and a `Stop-Process` blindly kills whoever holds `:3001` — but the service binds `13001`. Dormant today (Scheduled Task disabled), arms the moment it's enabled: false-OK smoke against Docker/Grafana on `:3001` + a collateral kill.

- [ ] **Step 1: Read the port from `.env` (replace the `:3001` hardcode)**

In the Configuration block, replace:

```powershell
$healthUrl  = 'http://localhost:3001/'
```

with:

```powershell
# Port comes from the service's own .env (PORT=13001); never hardcode — it moved off 3001 (Grafana collision).
$port = 13001
$envFile = Join-Path $projectDir '.env'
if (Test-Path -LiteralPath $envFile) {
    $m = Select-String -LiteralPath $envFile -Pattern '^\s*PORT\s*=\s*(\d+)' | Select-Object -First 1
    if ($m) { $port = [int]$m.Matches[0].Groups[1].Value }
}
$healthUrl  = "http://localhost:$port/"
```

- [ ] **Step 2: Null-guard the remote-ref lookup (kill the false-green crash)**

Replace:

```powershell
    & git fetch --quiet origin
    $remoteCommit = (& git rev-parse --short "origin/$localBranch").Trim()
    Write-Host "  Remote:  origin/$localBranch @ $remoteCommit" -ForegroundColor Gray
```

with:

```powershell
    & git fetch --quiet origin
    $remoteCommit = (& git rev-parse --short "origin/$localBranch" 2>$null)
    $remoteCommit = if ($remoteCommit) { "$remoteCommit".Trim() } else { $null }
    if (-not $remoteCommit) {
        Write-Host ""
        Write-Host "  ERROR: origin/$localBranch not found — cannot compare (push the branch or fix the remote)." -ForegroundColor Red
        # Honest status: fail CLOSED instead of the finally block writing a stale 'ok'.
        $skStatus = 'error'; $skFailed = 1; $skSummary = "origin/$localBranch missing"
        return
    }
    Write-Host "  Remote:  origin/$localBranch @ $remoteCommit" -ForegroundColor Gray
```

- [ ] **Step 3: Drop the blind `:3001` `Stop-Process` (collateral-kill risk)**

The Scheduled Task `/End` already stops the service; the defensive port-kill can murder an unrelated process. Remove:

```powershell
        & schtasks.exe /End /TN $taskName 2>&1 | Out-Null
        Start-Sleep -Seconds 2
        # Also kill anything still bound to :3001 (defensive)
        Get-NetTCPConnection -LocalPort 3001 -ErrorAction SilentlyContinue | ForEach-Object {
            try { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue } catch {}
        }
        Start-Sleep -Seconds 1
        & schtasks.exe /Run /TN $taskName 2>&1 | Out-Null
```

with:

```powershell
        & schtasks.exe /End /TN $taskName 2>&1 | Out-Null
        Start-Sleep -Seconds 2
        & schtasks.exe /Run /TN $taskName 2>&1 | Out-Null
```

- [ ] **Step 4: Verify it now reports honestly (runnable check)**

Run (PowerShell, UTF-8):

```powershell
powershell.exe -ExecutionPolicy Bypass -Command "[Console]::OutputEncoding=[Text.Encoding]::UTF8; & 'E:\Scripts\SettingsMCP\ClaudeProfiles\Update-FreeLLMAPI.ps1' -Check; Get-Content -LiteralPath 'E:\Scripts\SettingsMCP\ClaudeProfiles\freellmapi.last.json' -Raw"
```

Expected: the run does NOT throw; `freellmapi.last.json` shows `"status":"error"` with `"summary":"origin/wip-local missing"` (or `"ok"/"changes"` if the branch *does* have a matching origin ref) — **never a stale `ok` after a crash**. Confirm no `3001` remains: `Select-String -LiteralPath 'E:\Scripts\SettingsMCP\ClaudeProfiles\Update-FreeLLMAPI.ps1' -Pattern '3001'` returns nothing.

- [ ] **Step 5: Commit**

```bash
git -C E:/Scripts/Castellyn add -A   # NOTE: the .ps1 lives outside the repo; commit it in its own tree if versioned there.
git commit -m "fix(freellmapi-updater): honest status on missing origin ref + de-hardcode :3001->.env PORT"
```

> If `Update-FreeLLMAPI.ps1` is not tracked in the Castellyn repo, commit it wherever `SettingsMCP\ClaudeProfiles` is versioned; otherwise note the edit in the plan's completion and move on.

---

### Task 2: Background stack-health monitor (Rust)

**Files:**
- Create: `E:\Scripts\Castellyn\src-tauri\src\stack_health.rs`
- Modify: `E:\Scripts\Castellyn\src-tauri\src\lib.rs` — make `StackHealth` + `read_stack_health_blocking` `pub(crate)` (`:3648`, `:3672`); add `mod stack_health;` and call `stack_health::start(...)` in `setup()` (near `limits::start`, ~`:14124`); add `stack_health_monitor` to `HubConfig`.
- Modify: `E:\Scripts\Castellyn\src\lib\ipc.ts` — add `stackHealthMonitor?: boolean` to the `HubConfig` type (keep Rust↔TS parity).

**Interfaces:**
- Consumes: `crate::read_stack_health_blocking() -> Vec<StackHealth>` (made `pub(crate)`); `StackHealth { id, name, group, port, enabled, port_open, healthy }` (fields made `pub(crate)`).
- Produces:
  - `pub(crate) fn newly_down(prev_down: &mut HashSet<String>, curr: &[(String, bool)]) -> Vec<String>` — pure; returns ids that transitioned **into** down this tick (were not down last tick), and updates `prev_down` to the current down-set. Recovery re-arms.
  - `pub fn start(app: tauri::AppHandle)` — spawns the poll thread.
  - Tauri events: `stack-health` (payload `Vec<StackHealth>`, every tick) and `stack-service-down` (payload `{ id: String, name: String }`, once per transition-to-down).

- [ ] **Step 1: Write the failing unit test for the pure transition detector**

Create `src-tauri/src/stack_health.rs` with only the test + a stub:

```rust
//! Background liveness monitor for the llm-stack. Polls read_stack_health_blocking() on a timer,
//! pushes the full list to the UI, and flags services that transition to down (once per transition).

use std::collections::HashSet;

/// Ids that went down THIS tick (were not down last tick). Mutates `prev_down` to the current
/// down-set so a still-down service does not re-fire and a recovered one re-arms. Pure + testable.
pub(crate) fn newly_down(prev_down: &mut HashSet<String>, curr: &[(String, bool)]) -> Vec<String> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_per_transition_and_rearms() {
        let mut prev: HashSet<String> = HashSet::new();
        let up = |ids: &[(&str, bool)]| ids.iter().map(|(i, d)| (i.to_string(), *d)).collect::<Vec<_>>();

        // First tick: gateway down, qwen up → gateway newly-down.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", false)])), vec!["gateway"]);
        // Still down → does NOT re-fire.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", false)])), Vec::<String>::new());
        // Gateway recovers → nothing fires, but it re-arms.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", false), ("qwen", false)])), Vec::<String>::new());
        // Gateway drops again → fires again (re-armed).
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", false)])), vec!["gateway"]);
        // A second service drops in the same tick → only the newly-down one.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", true)])), vec!["qwen"]);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml stack_health::` (set `$CARGO` to the full cargo path, e.g. `C:/Users/User/.cargo/bin/cargo.exe`).
Expected: FAIL — panics with `not implemented` / `unimplemented`.

- [ ] **Step 3: Implement `newly_down`**

Replace the stub body:

```rust
pub(crate) fn newly_down(prev_down: &mut HashSet<String>, curr: &[(String, bool)]) -> Vec<String> {
    let now_down: HashSet<String> = curr
        .iter()
        .filter(|(_, down)| *down)
        .map(|(id, _)| id.clone())
        .collect();
    // Newly down = down now but not down last tick, in stable input order.
    let fired: Vec<String> = curr
        .iter()
        .filter(|(id, down)| *down && !prev_down.contains(id))
        .map(|(id, _)| id.clone())
        .collect();
    *prev_down = now_down;
    fired
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml stack_health::`
Expected: PASS (`fires_once_per_transition_and_rearms ... ok`).

- [ ] **Step 5: Add the poll loop `start(app)`**

Append to `stack_health.rs`:

```rust
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const POLL_SECS: u64 = 30;

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ServiceDown {
    id: String,
    name: String,
}

/// Start the stack-health poll thread. Called once from `setup()`. Respects the
/// `stackHealthMonitor` config toggle (default on). First poll runs after one interval so startup
/// isn't blocked; the health card already loads once on mount.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut prev_down: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(POLL_SECS));
            if !crate::read_config_file().stack_health_monitor.unwrap_or(true) {
                continue;
            }
            let health = crate::read_stack_health_blocking();
            // Only enabled services count as "outages"; a disabled service being down is expected.
            let curr: Vec<(String, bool)> = health
                .iter()
                .filter(|h| h.enabled)
                .map(|h| (h.id.clone(), !h.port_open || h.healthy == Some(false)))
                .collect();
            let fired = newly_down(&mut prev_down, &curr);
            // Push the full list every tick so the UI updates live without a manual refresh.
            let _ = app.emit("stack-health", &health);
            for id in fired {
                if let Some(h) = health.iter().find(|h| h.id == id) {
                    let _ = app.emit("stack-service-down", ServiceDown { id: h.id.clone(), name: h.name.clone() });
                }
            }
        }
    });
}
```

- [ ] **Step 6: Wire into lib.rs — visibility, module, config, setup**

In `lib.rs`: (a) make the health reader + struct crate-visible:

```rust
// was: #[derive(Serialize)] struct StackHealth { id: String, ... }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StackHealth {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: String,
    pub(crate) port: u16,
    pub(crate) enabled: bool,
    pub(crate) port_open: bool,
    pub(crate) healthy: Option<bool>,
}
```

```rust
// was: fn read_stack_health_blocking() -> Vec<StackHealth> {
pub(crate) fn read_stack_health_blocking() -> Vec<StackHealth> {
```

(b) declare the module near the other `mod` lines: `mod stack_health;`

(c) add the toggle to `HubConfig` (find the struct; mirror `limits_monitor`):

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_health_monitor: Option<bool>,
```

(d) start it in `setup()` next to `limits::start(...)`:

```rust
        limits::start(app.handle().clone());
        stack_health::start(app.handle().clone());
```

- [ ] **Step 7: Keep Rust↔TS config parity**

In `src/lib/ipc.ts`, add to the `HubConfig` type (near `stackNative`):

```ts
  stackHealthMonitor?: boolean;
```

- [ ] **Step 8: Build + test + clippy**

Run:
```
"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml
"$CARGO" clippy --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml -- -D warnings
```
Expected: tests PASS (incl. `stack_health::`); clippy 0 warnings.

- [ ] **Step 9: Commit**

```bash
git -C E:/Scripts/Castellyn add src-tauri/src/stack_health.rs src-tauri/src/lib.rs src/lib/ipc.ts
git commit -m "feat(stack): background health monitor — emit stack-health + flag transition-to-down"
```

---

### Task 3: Live-update the health card (no manual refresh)

**Files:**
- Modify: `E:\Scripts\Castellyn\src\lib\components\StackHealthCard.svelte` (the `$effect` at `:25-27`)

**Interfaces:**
- Consumes: the `stack-health` Tauri event (payload `StackHealth[]`) from Task 2.
- Produces: a card whose dots reflect the latest poll without the user clicking refresh.

- [ ] **Step 1: Subscribe to the `stack-health` event**

Add the import at the top of the `<script>` block:

```ts
  import { listen } from '@tauri-apps/api/event';
```

Replace the mount effect:

```ts
  $effect(() => {
    load();
  });
```

with:

```ts
  $effect(() => {
    load();
    // Live updates from the backend health-poll loop — no manual refresh needed.
    const un = listen<StackHealth[]>('stack-health', (e) => {
      items = e.payload;
      loadedOnce = true;
    });
    return () => {
      un.then((f) => f());
    };
  });
```

- [ ] **Step 2: Type + i18n gate**

Run: `cd E:/Scripts/Castellyn && npm run check && npm run check:i18n`
Expected: svelte-check 0 errors / 0 warnings; i18n parity unchanged (no new keys). `StackHealth` is already imported (`:2`), so the `listen<StackHealth[]>` generic resolves.

- [ ] **Step 3: Live smoke (the whole point of Ф1)**

Run the app (`npm run tauri dev` or the built `castellyn.exe`), open the tab with the System Health card, then stop a running stack service (e.g. via its Stop button or kill its port). Within ~30s the service's dot must turn (down/degraded) **without clicking Refresh**. Confirm the console/network shows a `stack-health` event arriving.

- [ ] **Step 4: Commit**

```bash
git -C E:/Scripts/Castellyn add src/lib/components/StackHealthCard.svelte
git commit -m "feat(health-card): live-update dots from the stack-health event (no manual refresh)"
```

---

## Self-Review notes

- **Spec coverage (Ф1):** false-green `Update-FreeLLMAPI.ps1` → Task 1 steps 2, 4; `:3001` de-hardcode → Task 1 steps 1, 3; background health-poll loop (mirror `limits.rs:241`) → Task 2; "death seen without manual refresh" → Task 3. ✓
- **Deferred (NOT this plan, by design):** per-account health readers + `accountsAttention()` + prominent toast/badge = Plan 2 (Ф2/Ф3); front-critical health `role:"front"` replacing `id==='gateway'` = Plan 3 (Ф5); opt-in auto-restart = Plan 3 (Ф8, and must coordinate with OmniRoute's own `--max-restarts`).
- **Type consistency:** `newly_down(&mut HashSet<String>, &[(String,bool)]) -> Vec<String>` used identically in test (Task 2 step 1) and loop (step 5); `stack_health_monitor`/`stackHealthMonitor` paired across Rust `HubConfig` and `ipc.ts`; `stack-health` event payload `Vec<StackHealth>` (Rust) ↔ `StackHealth[]` (Svelte listen generic).
- **No new spawns**, so `CREATE_NO_WINDOW` is not exercised. No new i18n keys, so `check:i18n` stays green.
