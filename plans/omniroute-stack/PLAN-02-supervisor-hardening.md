# Supervisor Hardening (Ф3.5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the native LLM-stack supervisor (`stackNative`, now the default `unwrap_or(true)` and unsmoked) trustworthy enough to sit under the single-front OmniRoute integration: a single-service Stop must not abort a concurrent full Start, a successful pid-kill must not trigger a second port-based kill, an in-flight readiness wait must be interruptible by Stop, and cold-starting engines (>25s) must get a per-service readiness budget from `stack.json` honored by the **native** waiter. Plus close the paid `glm-router` fallback's missing client auth.

**Architecture:** Three independent slices. (1) **Rust supervisor reliability** — three surgical fixes in `native_stack_stop` / `native_wait_ready` (`src-tauri/src/lib.rs`), each backed by a pure testable helper or a concrete runnable check: gate the global cancel flag on stop-all only (CAST-3), gate the port-kill fallback behind `!killed` (CAST-4), and check the cancel flag inside the readiness poll (CAST-5). (2) **`readyTimeoutSec`** — optional per-service field in `stack.json` read by the native `native_wait_ready` (fallback 25s), so an OmniRoute engine that binds its port slowly is not reported dead. (3) **glm-router auth** — an `X-Router-Secret` header check in `glm-router/router.py` (Python + `.env`), independent of the Rust work.

**Tech Stack:** Rust (Tauri v2, `std::sync::atomic`, `serde_json`), JSON manifest (`stack.json`), Python 3 (FastAPI/Starlette, `httpx`, `hmac`).

## Plan set (this is Plan 2 of 5)

This subsystem-decomposed effort ships one working slice per plan (see `plans/omniroute-stack/DESIGN.md`). This was "supervisor-hardening" (DESIGN §6, Ф3.5) — a **prerequisite to the single OmniRoute front**: the five audit items were tagged "fix BEFORE enabling `stackNative`", but `stackNative` already defaults on, so the still-open ones now live in the default path and bite harder under a unified front.
1. **Monitoring Trust (Ф1)** — Plan 1. Independent of OmniRoute.
2. **Supervisor Hardening (Ф3.5)** — THIS PLAN. Reliability of the native supervisor + `readyTimeoutSec` + glm-router auth.
3. OmniRoute integration (Ф4 split `id 'gateway'`, Ф5 front-critical health, Ф6 relax direct-openai arm, Ф7 register providers).
4. Fork-update consolidation (Ф9) + zcode git-clone migration (Ф10).
5. freellmapi retire (Ф11) + second-dev docs (Ф12).

## Global Constraints

- **Comments in English.** (per `~/.claude/CLAUDE.md`)
- **All `Command` spawns set `CREATE_NO_WINDOW`** (0x08000000). No **new** spawns in this plan (the native supervisor's existing `spawn_service_native` / `tasklist` / `netstat` calls already set it) — honor it if one is added.
- **i18n parity ru/en/zh** enforced by `npm run check:i18n` + `src/lib/i18n/index.test.ts`. **This plan adds NO `t()` keys** — the stack-log lines (`[cancel]`, `[fail]`, `[skip]`) are raw English `stack_emit` strings, not translated keys, exactly like the surrounding code. Keep it that way so `check:i18n` stays green (last count: unchanged).
- **Never name an `{#each … as t}` var or a param `t`** — shadows the translation function. (No Svelte in this plan.)
- **Green gates before "done":** `npm run check` (0/0), `npm test`, `cargo test`, `cargo clippy -- -D warnings` (0 warnings), `npm run build`. For the Python task also: `pytest glm-router/`.
- **Cargo is not on PATH** — invoke via its full path `C:/Users/User/.cargo/bin/cargo.exe` (referred to as `$CARGO` below); trust `$LASTEXITCODE`, not PowerShell's false exit-1 (see `[[cargo-windows-invocation]]`).

---

## Audit reconciliation (verified against current `lib.rs`, HEAD `68f0dcb`)

The portfolio audit is dated 2026-07-06; commit `9a40b2e` and the later hardening (`0c9c8ab`, `68f0dcb`) landed since. Each item was re-checked against the **current** code before writing a task.

| Item | Verdict | Evidence (current code) |
|------|---------|-------------------------|
| **CAST-1** double `run-done` on native restart | **ALREADY FIXED** | `native_stack_stop` takes `emit_done: bool` (`lib.rs:3269`); the restart branch calls it with `false` (`lib.rs:3415`) so only the concluding `native_stack_start` emits `run-done`. No task. |
| **CAST-2** native start false-green (`code:0` on failure) | **ALREADY FIXED** | `native_stack_start` tracks `failed`, computes `let code = if failed > 0 { 1 } else { 0 };`, emits `run-done{code}` and returns it (`lib.rs:3122,3189,3214,3226-3234`); `run_stack` returns `Ok(code)` (`lib.rs:3399,3408`). Skips (disabled/requires-missing/already-up) are intentionally not failures. No task. |
| **CAST-3** single-service stop sets global `STACK_CANCEL`, aborting a concurrent full start | **STILL A BUG** | `native_stack_stop` sets `STACK_CANCEL.store(true, …)` **unconditionally** at `lib.rs:3271`, regardless of `only`. A single-service Stop (which `reserve_preempt`s the slot but leaves the native start task running) flips the flag and the start loop aborts at `lib.rs:3124`. → **Task 1**. |
| **CAST-4** port-kill fallback runs even after a successful tracked-pid kill | **STILL A BUG** | The port fallback at `lib.rs:3317-3324` runs whenever `port != 0`, with no `!killed` guard, so a service already killed by its tracked pid gets a second `kill_tree` on whatever holds the port (ownership-guarded, but still a redundant kill). → **Task 1**. |
| **CAST-5** `STACK_CANCEL` not checked inside `native_wait_ready` poll | **STILL A BUG** | `native_wait_ready`'s port loop (`lib.rs:3063-3072`) and health loop (`lib.rs:3082-3091`) never read `STACK_CANCEL`, so a Stop issued mid-wait is only noticed after the full budget (up to 25s + 15s) elapses. → **Task 1**. |
| **llm-stack-2** `readyTimeoutSec` per service, honored by the **native** waiter | **STILL NEEDED (native gap)** | `native_wait_ready` hardcodes the port budget to `Duration::from_secs(25)` at `lib.rs:3061`; there is no per-service override on the native path. The audit item names `start-stack.ps1`, but DESIGN §6 Ф3.5 requires the native waiter to honor it too (engines cold-start >25s). → **Task 2**. |
| **llm-stack-1** glm-router `ROUTER_SECRET` client auth | **STILL A BUG** | `router.py` `proxy_all` (`router.py:100-117`) proxies every request with the paid Z.AI key injected and no client-auth check. → **Task 3**. |

**Out of scope for this plan** (Castellyn fix_plan Wave 2/3, distinct subsystems — not supervisor hardening): CAST-6 (usage-poll dedup, `limits.rs`), CAST-7 (`read_stack_drift` off-thread), CAST-8 (`WindowTitleBar` empty strip), CAST-9/CAST-10 (security Low, by-design/not-queued). Leave for their own audit follow-ups.

---

### Task 1: Native supervisor reliability — cancel-scope, port-kill gate, interruptible wait (CAST-3/4/5)

**Files:**
- Modify: `E:\Scripts\Castellyn\src-tauri\src\lib.rs`
  - `native_wait_ready` — `Readiness` enum (`:3043-3047`), port loop (`:3063-3072`), health loop (`:3082-3091`).
  - `native_stack_start` — the `native_wait_ready` match arm (`:3187-3211`).
  - `native_stack_stop` — the unconditional `STACK_CANCEL.store` (`:3271`), the port fallback (`:3317-3324`).
  - Add a pure helper `stop_aborts_start` near `native_stack_stop`.
  - Add a unit test to the existing `#[cfg(test)] mod tests` at `:14216`.

**Interfaces:**
- Consumes: `static STACK_CANCEL: AtomicBool` (`:2941`); `enum Readiness` (`:3043`); `Ordering` (already in scope, used at `:3110`).
- Produces:
  - `fn stop_aborts_start(only: Option<&str>) -> bool` — pure; `true` iff this is a stop-all (`only.is_none()`). The single gate deciding whether a Stop flips the global start-abort flag.
  - `Readiness::Cancelled` — new variant returned by `native_wait_ready` when `STACK_CANCEL` is observed mid-poll; the start loop treats it as an immediate, non-failure break.
  - Behavioral (no signature change): `native_stack_stop`'s port fallback runs only when the tracked-pid kill did not already succeed.

#### CAST-3 — gate the global cancel flag on stop-all only

- [ ] **Step 1: Write the failing unit test for `stop_aborts_start`**

In `lib.rs`, inside the existing `#[cfg(test)] mod tests { use super::*; … }` block at `:14216`, add:

```rust
    #[test]
    fn stop_aborts_only_on_stop_all() {
        // A stop-all (only=None) must flip STACK_CANCEL to abort a concurrent full start.
        assert!(stop_aborts_start(None));
        // A targeted single-service stop must NOT — it would otherwise cancel the full start
        // of every OTHER service (CAST-3).
        assert!(!stop_aborts_start(Some("gateway")));
    }
```

- [ ] **Step 2: Run the test — expect a COMPILE failure (helper absent)**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml stop_aborts_only_on_stop_all`
Expected: FAIL — `cannot find function `stop_aborts_start` in this scope` (RED; the helper doesn't exist yet).

- [ ] **Step 3: Add the pure helper**

Immediately above `native_stack_stop` (before `:3265`), add:

```rust
/// Whether a Stop should flip the global `STACK_CANCEL` (which aborts an in-flight native start).
/// Only a stop-ALL should — a targeted single-service stop must leave a concurrent full start of the
/// OTHER services running (CAST-3). Pure so the cancel scope has a real assert behind it.
fn stop_aborts_start(only: Option<&str>) -> bool {
    only.is_none()
}
```

- [ ] **Step 4: Apply it in `native_stack_stop`**

Replace the unconditional flag set at `lib.rs:3271`:

```rust
    STACK_CANCEL.store(true, Ordering::SeqCst);
```

with:

```rust
    // Only a stop-all aborts a concurrent full start; a single-service stop leaves it running (CAST-3).
    if stop_aborts_start(only) {
        STACK_CANCEL.store(true, Ordering::SeqCst);
    }
```

- [ ] **Step 5: Run the test — expect PASS**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml stop_aborts_only_on_stop_all`
Expected: PASS (`stop_aborts_only_on_stop_all ... ok`).

#### CAST-4 — gate the port-kill fallback behind `!killed`

- [ ] **Step 6: Guard the port fallback (code read → no separate test)**

This is a control-flow gate on an imperative process-kill (no pure logic to unit-test). Replace the port fallback in `native_stack_stop` at `lib.rs:3315-3324`:

```rust
        // Port fallback (service started outside the app / stale pid): only kill a holder that IS one
        // of our service processes (node/python) — never a foreign app that holds a manifest port.
        if port != 0 {
            if let Some(pid) = by_port.get(&port) {
                if pid_image_name(*pid).map(|n| is_ours_process(&n)).unwrap_or(false) {
                    let _ = kill_tree(*pid);
                    killed = true;
                }
            }
        }
```

with (add the `!killed` guard so the tracked-pid success short-circuits the port lookup — CAST-4):

```rust
        // Port fallback ONLY when the tracked-pid kill didn't already succeed (CAST-4): a service
        // started outside the app / with a stale pid. Guarded so we never kill a foreign app that now
        // holds a manifest port after our own process already died.
        if !killed && port != 0 {
            if let Some(pid) = by_port.get(&port) {
                if pid_image_name(*pid).map(|n| is_ours_process(&n)).unwrap_or(false) {
                    let _ = kill_tree(*pid);
                    killed = true;
                }
            }
        }
```

- [ ] **Step 7: Concrete check for CAST-4 (compile + reasoning assertion)**

There is no pure seam here (the effect is `kill_tree` on live PIDs). The verification is: (a) it compiles under `-D warnings`; (b) read the block and confirm `!killed` short-circuits the `by_port` lookup when the tracked-pid arm already set `killed = true`. Runnable proof deferred to Task 1's Step 11 live smoke (single stop of a tracked service → `kill_tree` fires at most once). Run `"$CARGO" clippy --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml -- -D warnings` and expect 0 warnings.

#### CAST-5 — make the readiness wait interruptible by Stop

- [ ] **Step 8: Add the `Cancelled` variant + cancel checks in `native_wait_ready`**

Extend the `Readiness` enum at `lib.rs:3043-3047`:

```rust
enum Readiness {
    Down,      // port never listened within its budget — a real failure
    PortUp,    // port listens, but a declared health path didn't answer 2xx in time (still running)
    Healthy,   // port listens + 2xx health
    Cancelled, // a Stop set STACK_CANCEL mid-wait — abort, do NOT count as a failure (CAST-5)
}
```

In `native_wait_ready`, add a cancel check as the first line of BOTH poll loop bodies. Port loop (`lib.rs:3063`):

```rust
    while std::time::Instant::now() < port_deadline {
        if STACK_CANCEL.load(Ordering::SeqCst) {
            return Readiness::Cancelled;
        }
        if tokio::task::spawn_blocking(move || port_listening(port))
            .await
            .unwrap_or(false)
        {
            listening = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
```

Health loop (`lib.rs:3082`):

```rust
    while std::time::Instant::now() < health_deadline {
        if STACK_CANCEL.load(Ordering::SeqCst) {
            return Readiness::Cancelled;
        }
        let h = health.clone();
        if tokio::task::spawn_blocking(move || http_health_ok(port, &h))
            .await
            .unwrap_or(false)
        {
            return Readiness::Healthy;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
```

- [ ] **Step 9: Handle `Cancelled` in the start loop**

In `native_stack_start`, the match on `native_wait_ready(&svc).await` at `lib.rs:3187`. Add a `Cancelled` arm that breaks WITHOUT incrementing `failed` (so a Stop doesn't produce a spurious `[fail]`). Replace:

```rust
                match native_wait_ready(&svc).await {
                    Readiness::Down => {
                        failed += 1;
                        stack_emit(app, format!("[fail] {name} did not come up — see stack-logs\\{sid}.log"));
                    }
                    ready => {
```

with:

```rust
                match native_wait_ready(&svc).await {
                    Readiness::Cancelled => {
                        // Stop preempted this start mid-wait — abort cleanly, not a failure (CAST-5).
                        stack_emit(app, format!("[cancel] {name}: start aborted"));
                        break;
                    }
                    Readiness::Down => {
                        failed += 1;
                        stack_emit(app, format!("[fail] {name} did not come up — see stack-logs\\{sid}.log"));
                    }
                    ready => {
```

> The `ready => { started += 1; … matches!(ready, Readiness::PortUp) … }` arm is unchanged: `Cancelled` and `Down` are handled explicitly above it, so `ready` now binds only `PortUp | Healthy` exactly as before.

- [ ] **Step 10: Build, test, clippy (whole crate)**

Run:
```
"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml
"$CARGO" clippy --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml -- -D warnings
```
Expected: all tests PASS (incl. `stop_aborts_only_on_stop_all`); clippy 0 warnings. If clippy flags the new `Cancelled` arm as unreachable or the enum as having an unused variant, it means Step 9 wasn't applied — fix before proceeding.

- [ ] **Step 11: Live smoke (process-kill ordering isn't unit-testable)**

With `stackNative` on (default), run the built app or `npm run tauri dev`:
1. **CAST-3:** Start the whole stack; while services are still coming up, issue a **single-service** Stop (one service's Stop button). Assert the console shows the other services continue to `[ ok ]` / `[fail]` and are NOT interrupted by `[cancel] … aborting start`.
2. **CAST-5:** Start a service whose port is slow to bind (or point one at a dead port); while it shows `[ .. ] … starting…`, issue a **stop-all**. Assert the start loop emits `[cancel] … start aborted` within ~500ms (one poll tick), not after the full 25s.
3. **CAST-4:** Start a tracked service, then Stop it; confirm exactly one `[stop] <name>` and no double-kill of an unrelated port holder (check `stack-logs` / no foreign process died).

- [ ] **Step 12: Commit**

```bash
git -C E:/Scripts/Castellyn add src-tauri/src/lib.rs
git -C E:/Scripts/Castellyn commit -m "fix(stack-native): scope STACK_CANCEL to stop-all, gate port-kill on !killed, interruptible native_wait_ready (CAST-3/4/5)"
```

---

### Task 2: `readyTimeoutSec` per service, honored by the native waiter (llm-stack-2)

**Files:**
- Modify: `E:\Scripts\Castellyn\src-tauri\src\lib.rs` — `native_wait_ready` port-deadline (`:3061`); add a pure helper `ready_timeout_secs` above `native_wait_ready` (before `:3049`); add a unit test to `#[cfg(test)] mod tests` at `:14216`.
- Modify: `E:\Scripts\llm-stack\stack.json` — add `"readyTimeoutSec"` to the `qwen` entry (`:5-20`).

**Interfaces:**
- Consumes: `svc: &serde_json::Value` (a `stack.json` service object).
- Produces: `fn ready_timeout_secs(svc: &serde_json::Value) -> u64` — pure; returns the service's `readyTimeoutSec` when it is a positive integer, else the historical 25s default. Used only for the port-bind budget (the soft health confirmation keeps its own 15s).

**Why native, not just PS:** DESIGN §6 Ф3.5 + §12 — `stack.json` is the canonical registry the **native supervisor** reads, and `stackNative` is the default path. OmniRoute's engines cold-start >25s (headless-Chrome bring-up for Qwen), so the 25s hardcode in `native_wait_ready` gives a false `[fail]` on a service that is merely slow to bind. The PS launcher's own `Wait-Ready` fix (audit llm-stack-2, `start-stack.ps1:71`) is a separate consumer and is NOT this task.

- [ ] **Step 1: Write the failing unit test for `ready_timeout_secs`**

In `lib.rs`, inside `#[cfg(test)] mod tests` at `:14216`, add:

```rust
    #[test]
    fn ready_timeout_reads_override_else_default() {
        // No field → historical 25s default.
        assert_eq!(ready_timeout_secs(&serde_json::json!({})), 25);
        // A positive override wins (Qwen cold start).
        assert_eq!(ready_timeout_secs(&serde_json::json!({ "readyTimeoutSec": 60 })), 60);
        // Nonsense (0 / non-number) falls back to the default, never a zero budget.
        assert_eq!(ready_timeout_secs(&serde_json::json!({ "readyTimeoutSec": 0 })), 25);
        assert_eq!(ready_timeout_secs(&serde_json::json!({ "readyTimeoutSec": "x" })), 25);
    }
```

- [ ] **Step 2: Run the test — expect a COMPILE failure**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml ready_timeout_reads_override_else_default`
Expected: FAIL — `cannot find function `ready_timeout_secs`` (RED).

- [ ] **Step 3: Add the pure helper**

Immediately above `async fn native_wait_ready` (before `lib.rs:3049`), add:

```rust
/// Per-service port-readiness budget in seconds. An OmniRoute/Qwen engine can cold-start >25s, so
/// stack.json may declare `readyTimeoutSec`; anything missing or non-positive falls back to the
/// historical 25s default (never a zero budget). Pure so the calibration knob has a real assert.
fn ready_timeout_secs(svc: &serde_json::Value) -> u64 {
    svc.get("readyTimeoutSec")
        .and_then(|x| x.as_u64())
        .filter(|&n| n > 0)
        .unwrap_or(25)
}
```

- [ ] **Step 4: Use it for the port deadline**

In `native_wait_ready`, replace the hardcoded port budget at `lib.rs:3061`:

```rust
    let port_deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
```

with:

```rust
    // Port-bind budget from stack.json (readyTimeoutSec), default 25s. Slow-binding engines
    // (headless-Chrome cold start) need longer or they read as a false [fail].
    let port_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(ready_timeout_secs(svc));
```

> Leave the health-confirmation deadline (`from_secs(15)` at `:3081`) as-is — it is a soft upgrade with its own fresh budget and never fails a service.

- [ ] **Step 5: Run the test — expect PASS**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml ready_timeout_reads_override_else_default`
Expected: PASS.

- [ ] **Step 6: Add the field to `stack.json` (Qwen)**

In `E:\Scripts\llm-stack\stack.json`, add `readyTimeoutSec` to the `qwen` service. Change (`:11-15`):

```json
      "port": 3264,
      "health": "/health",
      "protocol": "openai",
      "dashboard": "http://localhost:3264/dashboard",
      "openDashboard": false,
```

to:

```json
      "port": 3264,
      "readyTimeoutSec": 60,
      "health": "/health",
      "protocol": "openai",
      "dashboard": "http://localhost:3264/dashboard",
      "openDashboard": false,
```

> Only Qwen needs it today (headless-Chrome cold start). Other services keep the 25s default via the `unwrap_or(25)` fallback. When Ф4 adds the `omniroute` service (serve starts ~8s but can be slower under load), give it its own `readyTimeoutSec` there — not here.

- [ ] **Step 7: Validate the JSON still parses**

Run: `"$CARGO" test --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml` (the full suite; `stack_services()`-backed tests, if any, load the manifest) and, as a direct check, `node -e "JSON.parse(require('fs').readFileSync('E:/Scripts/llm-stack/stack.json','utf8')); console.log('ok')"`.
Expected: `ok` (valid JSON); tests PASS.

- [ ] **Step 8: Clippy + commit**

Run: `"$CARGO" clippy --manifest-path E:/Scripts/Castellyn/src-tauri/Cargo.toml -- -D warnings` (expect 0 warnings), then:

```bash
git -C E:/Scripts/Castellyn add src-tauri/src/lib.rs
git -C E:/Scripts/llm-stack add stack.json   # NOTE: stack.json lives in the llm-stack repo, not Castellyn.
git -C E:/Scripts/Castellyn commit -m "feat(stack-native): honor per-service readyTimeoutSec in native_wait_ready (default 25s)"
git -C E:/Scripts/llm-stack commit -m "feat(stack): readyTimeoutSec=60 for qwen (headless-Chrome cold start)"
```

> `stack.json` is versioned in the `E:\Scripts\llm-stack` tree, `lib.rs` in `E:\Scripts\Castellyn` — commit each in its own repo. If `llm-stack` is not a git repo, note the manifest edit in the completion and move on.

---

### Task 3: glm-router client auth — `X-Router-Secret` (llm-stack-1)

**Files:**
- Modify: `E:\Scripts\llm-stack\glm-router\router.py` — config block (`:39-44`), `proxy_all` entry (`:100-102`), upstream header strip (`:126`), startup banner (`:200-207`).
- Modify/Create: `E:\Scripts\llm-stack\glm-router\.env.example` — document `ROUTER_SECRET`.
- Create: `E:\Scripts\llm-stack\glm-router\test_router_auth.py` — the money-path auth check.
- Modify (if present): `E:\Scripts\llm-stack\README.md` (or `glm-router/README*`) — router section, document the header.

**Interfaces:**
- Consumes: env/`.env` `ROUTER_SECRET` (optional; empty = auth disabled, localhost-only mitigates).
- Produces: `proxy_all` rejects any request with a missing/wrong `X-Router-Secret` with `401` **before** proxying (when a secret is configured); the header is stripped from the upstream request so it never leaks to Z.AI; a startup warning when no secret is set.

**Design note (fail-open-when-unset, constant-time compare):** the router already binds `127.0.0.1` only (`router.py:43`), so the residual threat is other *local* processes. When `ROUTER_SECRET` is configured we enforce it fail-closed with `hmac.compare_digest` (timing-safe); when it is empty we log a loud one-line warning at startup and allow (backward-compat for existing setups) rather than hard-failing the paid fallback. Client (Claude Code) sends it via `ANTHROPIC_CUSTOM_HEADERS`.

- [ ] **Step 1: Write the failing auth test**

Create `E:\Scripts\llm-stack\glm-router\test_router_auth.py`:

```python
"""Money-path auth check for the paid GLM router: a configured ROUTER_SECRET must 401 any request
that doesn't present the matching X-Router-Secret header, BEFORE anything is proxied upstream.

Env is set before importing router so its import-time `raise SystemExit(no ZAI key)` is satisfied and
the secret is active. The 401 path returns before any httpx call, so no real upstream is needed."""
import os

os.environ["ZAI_API_KEY"] = "test-key-not-used"   # satisfy router.py import-time guard
os.environ["ROUTER_SECRET"] = "s3cret"

from starlette.testclient import TestClient  # noqa: E402
import router  # noqa: E402

client = TestClient(router.app)


def test_missing_secret_is_rejected():
    r = client.post("/v1/messages", json={"model": "claude-3-5-haiku-20241022"})
    assert r.status_code == 401, r.text


def test_wrong_secret_is_rejected():
    r = client.post("/v1/messages", headers={"X-Router-Secret": "nope"},
                    json={"model": "claude-3-5-haiku-20241022"})
    assert r.status_code == 401, r.text


if __name__ == "__main__":
    test_missing_secret_is_rejected()
    test_wrong_secret_is_rejected()
    print("router auth: ok")
```

- [ ] **Step 2: Run it — expect FAIL (no auth yet)**

Run: `python E:/Scripts/llm-stack/glm-router/test_router_auth.py`
Expected: an `AssertionError` (the current router returns something other than 401 — it tries to proxy and fails on the upstream connect, i.e. 502/500, not 401). RED. (If `starlette`/`httpx` are missing: `pip install starlette httpx` — both are already transitive deps of the running router.)

- [ ] **Step 3: Read `ROUTER_SECRET` in the config block**

In `router.py`, add `import hmac` to the imports (top, after `import os`), and in the config block after `ZAI_API_KEY` (`:44`) add:

```python
# Optional client auth. When set, every request must present a matching X-Router-Secret header.
# Empty = disabled (localhost-only bind mitigates); we warn loudly at startup in that case.
ROUTER_SECRET = os.environ.get("ROUTER_SECRET", "").strip()
```

- [ ] **Step 4: Reject unauthenticated requests at the top of `proxy_all`**

In `proxy_all`, immediately after the `async def proxy_all(request: Request, full_path: str):` line (`:101`), before reading the body, insert:

```python
    # Client auth (before any upstream work): a configured secret must match, timing-safe.
    if ROUTER_SECRET:
        supplied = request.headers.get("x-router-secret", "")
        if not hmac.compare_digest(supplied, ROUTER_SECRET):
            return JSONResponse(
                {"error": "unauthorized", "details": "missing or invalid X-Router-Secret"},
                status_code=401,
            )
```

- [ ] **Step 5: Don't leak the secret upstream**

In the header-strip list at `router.py:126`, add `x-router-secret` so it is never forwarded to Z.AI or Anthropic. Change:

```python
    headers = dict(request.headers)
    for h in ["host", "content-length", "connection", "accept-encoding"]:
        headers.pop(h, None)
```

to:

```python
    headers = dict(request.headers)
    for h in ["host", "content-length", "connection", "accept-encoding", "x-router-secret"]:
        headers.pop(h, None)
```

- [ ] **Step 6: Warn at startup when unset**

In the `__main__` banner (`router.py:200-207`), after the `Z.AI key` print line add:

```python
    if not ROUTER_SECRET:
        print("  WARNING: ROUTER_SECRET not set — client auth DISABLED (localhost-only). "
              "Set it in glm-router/.env to require X-Router-Secret.")
```

- [ ] **Step 7: Run the test — expect PASS**

Run: `python E:/Scripts/llm-stack/glm-router/test_router_auth.py` → prints `router auth: ok`.
Also run the whole router test dir: `python -m pytest E:/Scripts/llm-stack/glm-router/ -q` (picks up `test_model_map.py` too). Expected: all PASS.

- [ ] **Step 8: Document the header (`.env.example` + README)**

In `E:\Scripts\llm-stack\glm-router\.env.example` (create if missing; mirror the existing keys `ZAI_API_KEY`, `ROUTER_PORT`, …), add:

```dotenv
# Client auth: when set, callers must send  X-Router-Secret: <this value>  or get 401.
# Leave empty to disable (the router binds 127.0.0.1 only). Generate one: python -c "import secrets;print(secrets.token_urlsafe(24))"
ROUTER_SECRET=
```

In the router section of `E:\Scripts\llm-stack\README.md` (if it exists), add a line documenting that when `ROUTER_SECRET` is set, Claude Code must forward it, e.g.:

```
set ANTHROPIC_CUSTOM_HEADERS=X-Router-Secret: <your ROUTER_SECRET>
```

If no README/router section exists, skip and note it in the completion.

- [ ] **Step 9: Live smoke (end-to-end, per audit verify)**

Set `ROUTER_SECRET` in `glm-router/.env`, start the router (`python router.py`), then:
1. `curl` any path with **no** header → expect HTTP `401`.
   `curl -s -o NUL -w "%{http_code}\n" -X POST http://localhost:4000/v1/messages -H "content-type: application/json" -d "{\"model\":\"claude-3-5-haiku-20241022\"}"` → `401`.
2. A real Claude Code request with `ANTHROPIC_BASE_URL=http://localhost:4000` + `ANTHROPIC_CUSTOM_HEADERS=X-Router-Secret: <secret>` → proxies through to Z.AI end-to-end (a normal completion), and the router log shows `[Z.AI] POST …`, not a 401.

- [ ] **Step 10: Commit**

```bash
git -C E:/Scripts/llm-stack add glm-router/router.py glm-router/.env.example glm-router/test_router_auth.py README.md
git -C E:/Scripts/llm-stack commit -m "fix(glm-router): require X-Router-Secret client auth (timing-safe, fail-open when unset)"
```

> All Task 3 files live in the `E:\Scripts\llm-stack` repo. `.env` itself is git-ignored — never commit the real secret, only `.env.example`.

---

## Self-Review notes

- **Audit coverage:** CAST-3 (Task 1 §CAST-3, pure `stop_aborts_start` + unit test), CAST-4 (Task 1 §CAST-4, `!killed` gate + clippy/read), CAST-5 (Task 1 §CAST-5, `Readiness::Cancelled` + poll-loop checks + live smoke), llm-stack-2 native (Task 2, pure `ready_timeout_secs` + unit test + `stack.json`), llm-stack-1 (Task 3, `X-Router-Secret` + pytest). CAST-1 and CAST-2 verified ALREADY FIXED — no task (see reconciliation table). ✓
- **Real asserts behind the logic:** two new pure helpers (`stop_aborts_start`, `ready_timeout_secs`) each unit-tested mirroring `limits.rs::take_alert` / `stack_health.rs::newly_down`; the auth check has a `starlette` TestClient 401 test. The two genuinely process-bound fixes (CAST-4 double-kill ordering, CAST-5 mid-wait interrupt) specify concrete live-smoke checks since they can't be unit-tested without real ports/PIDs.
- **No i18n keys, no new spawns:** stack-log lines are raw `stack_emit` English (consistent with existing `[fail]`/`[skip]`), so `check:i18n` stays green; `CREATE_NO_WINDOW` unchanged (no new `Command`).
- **Sequencing:** Task 1 and Task 2 both edit `native_wait_ready` (Task 1 adds cancel checks inside the loops; Task 2 changes the deadline computed above the loops — non-overlapping lines). Do Task 1 first; Task 2's shown code assumes Task 1's `Cancelled` variant already exists (the `match` arms in Task 2 aren't retouched). Task 3 is fully independent (Python, separate repo).
- **Deferred (NOT this plan):** CAST-6/7/8 (Wave 2 cleanup, unrelated subsystems); CAST-9/10 (by-design security Low); the PS `start-stack.ps1:71` `readyTimeoutSec` read (separate consumer — this plan does the native waiter only); the `omniroute` service's own `readyTimeoutSec` (arrives with Ф4 split).
