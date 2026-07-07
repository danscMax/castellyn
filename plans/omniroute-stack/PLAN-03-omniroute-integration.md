# Plan 3 — OmniRoute integration, phase Ф4+Ф5 (split id + data-driven critical front)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> Scope confirmed by owner 2026-07-07: **Ф4+Ф5 now** (safe, unit-testable, zero live-OmniRoute
> dependency). Ф6 (relax `direct/openai` arm) and Ф7 (register providers + dependsOn/teardown)
> are DEFERRED to a live session where the owner installs & configures OmniRoute — see DESIGN §7/§8.

**Goal:** Give OmniRoute (`:20128`) its own distinct identity in the stack registry and make the
"critical front" of the health card data-driven, so the single-front topology can later attach
without renaming or misrouting the overloaded `gateway` id.

**Architecture:** Two additive, regression-free changes across two repos.
1. **Ф4 (split):** `gateway_base_url()` (freellmapi `:13001`) stays *untouched* — it feeds
   freellmapi-backend sites (custom-provider registration `lib.rs:6144`, Codex-freellmapi
   `lib.rs:8896`). Add a *parallel* `omniroute_base_url()` + an `omniroute` service entry in
   stack.json. No existing site changes.
2. **Ф5 (critical front):** add a `critical` flag to `StackHealth` (read from stack.json), mark
   `gateway` **and** `omniroute` critical, and replace the hardcoded `id === 'gateway'` checks in
   `StackHealthCard.svelte` with `.critical`. Because `gateway` stays `critical:true`, today's
   behavior is preserved 1:1; the generalization only activates when a second critical service is
   enabled.

**Tech Stack:** Rust (`src-tauri/src/lib.rs`) + Tauri commands, SvelteKit/Svelte 5 runes
(`StackHealthCard.svelte`, `ipc.ts`), JSON registry (`E:\Scripts\llm-stack\stack.json`).

## Global Constraints

- **Two repos.** `stack.json` lives in the **llm-stack** repo (`E:\Scripts\llm-stack`, branch
  `master`) — `STACK_CONFIG_REL = "llm-stack\\stack.json"`, read at runtime. lib.rs / svelte / ipc.ts
  live in **Castellyn** (`E:\Scripts\Castellyn`, branch `feat/omniroute-integration-f4f5`).
  Commit each file to its own repo. `git add` **only** the files you changed — llm-stack has
  foreign dirty files (`start-stack.ps1`, `start-stack.ps1.nobom.bak`) that must NOT be swept in.
- **`omniroute` entry is `enabled:false`.** It must not be spawned/probed until the owner installs
  and configures OmniRoute live (Ф7). Live-only fields (the real `health` 2xx route, the DATA_DIR
  env var name) are deliberately left for that session per DESIGN §10 — do not fabricate them.
- **No new user-facing strings.** `critical` is internal; `i18n` leaf-key parity must stay
  unchanged (`npm run check:i18n`).
- **DRY:** `omniroute_base_url()` mirrors `gateway_base_url()` exactly (same shape, `#[tauri::command]`,
  registered + ipc.ts wrapper). Do not invent a different pattern.
- **Green gates each task:** `cargo test` (from `src-tauri`, full path cargo), `cargo clippy`
  (0 warnings), `npm run check` (0/0), `npm test`, `npm run check:i18n`.

---

### Task 1: Ф4 — `omniroute` identity (stack.json entry + `omniroute_base_url()`)

**Files:**
- Modify: `E:\Scripts\llm-stack\stack.json` (add one service entry after `gateway`, before `glm-router`)
- Modify: `E:\Scripts\Castellyn\src-tauri\src\lib.rs` (add `omniroute_base_url()` after
  `gateway_base_url()` ~`:5917`; register in the `invoke_handler!` list next to `gateway_base_url`
  ~`:14138`; add a unit test)
- Modify: `E:\Scripts\Castellyn\src\lib\ipc.ts` (add `omnirouteBaseUrl` wrapper next to any
  `gatewayBaseUrl` wrapper, or after `readStackHealth` if none exists)

**Interfaces:**
- Produces: `fn omniroute_base_url() -> Option<String>` (Tauri command) returning
  `Some("http://localhost:20128")` when the `omniroute` entry exists, `None` otherwise. Ф6 (later)
  will consume it to wire OpenCode/Codex clients to the front.

- [ ] **Step 1: Add the `omniroute` entry to stack.json**

Insert this object into the `services` array immediately after the `gateway` entry (the one with
`"port": 13001`) and before `glm-router`:

```json
    {
      "id": "omniroute",
      "name": "OmniRoute gateway",
      "group": "core",
      "enabled": false,
      "dir": "{{SCRIPTS_ROOT}}\\llm-stack",
      "command": "omniroute serve --no-open --no-tray",
      "port": 20128,
      "readyTimeoutSec": 40,
      "health": "",
      "protocol": "openai+anthropic",
      "dashboard": "http://localhost:20128",
      "openDashboard": false,
      "critical": true,
      "note": "Single-front gateway (Claude Code + OpenCode/Codex -> :20128/v1). DISABLED until live setup: install with `npm i -g omniroute`, then set enabled:true, add the DATA_DIR env var, and set `health` to the real unauthorized-2xx route (pinned in the Ф7 live smoke — port-only would show green on a wedged-but-bound server, see DESIGN §10). serve cmd is foreground/supervisor-owned; do NOT add --daemon."
    },
```

Keep JSON valid (trailing comma after this object since `glm-router` follows). Save as UTF-8
**without** BOM.

- [ ] **Step 2: Add `omniroute_base_url()` (write the code, then a test that guards it)**

In `lib.rs`, immediately after `gateway_base_url()` (ends ~`:5917`), add:

```rust
/// OmniRoute front base URL from the `omniroute` service port in stack.json. None if absent.
/// Parallel to `gateway_base_url` (freellmapi :13001) and deliberately distinct: `gateway` feeds
/// freellmapi-backend registration + Codex-freellmapi; `omniroute` feeds the single client front.
#[tauri::command]
fn omniroute_base_url() -> Option<String> {
    let port = stack_services()
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some("omniroute"))
        .and_then(|e| e.get("port").and_then(|x| x.as_u64()))?;
    Some(format!("http://localhost:{port}"))
}
```

Register it in the `tauri::generate_handler!` list next to `gateway_base_url` (~`:14138`):

```rust
            gateway_base_url,
            omniroute_base_url,
```

- [ ] **Step 3: Add the guard test**

In the `#[cfg(test)]` module of `lib.rs`, add (this reads the real stack.json and transitively
guards: the entry parses, has the right id, and the helper resolves the port):

```rust
    #[test]
    fn omniroute_base_url_resolves_from_stack_json() {
        // The `omniroute` entry (Ф4) must be present and expose port 20128.
        let url = super::omniroute_base_url().expect("omniroute entry present in stack.json");
        assert_eq!(url, "http://localhost:20128");
    }
```

- [ ] **Step 4: Add the ipc.ts wrapper**

In `src/lib/ipc.ts`, after `readStackHealth` (~`:368`), add:

```ts
// OmniRoute single-front base URL (Ф4). null until the `omniroute` entry exists in stack.json.
export const omnirouteBaseUrl = () => invoke<string | null>('omniroute_base_url');
```

- [ ] **Step 5: Gates**

Run from `E:\Scripts\Castellyn`:
- `cargo test` (in `src-tauri`) → new test passes, all pass, 0 failed.
- `cargo clippy` → 0 warnings (the new command is "used" via the handler macro — no dead-code).
- `npm run check` → 0/0.
- `npm run check:i18n` → unchanged.

Expected: all green.

- [ ] **Step 6: Commit (two repos)**

llm-stack (add ONLY stack.json — foreign files present):
```bash
cd E:/Scripts/llm-stack && git add stack.json && git commit -m "feat(stack): add omniroute front service entry (disabled until live setup)"
```
Castellyn:
```bash
cd E:/Scripts/Castellyn && git add src-tauri/src/lib.rs src/lib/ipc.ts && git commit -m "feat: omniroute_base_url() parallel to gateway_base_url (Ф4 split)"
```

---

### Task 2: Ф5 — data-driven `critical` front in the health card

**Files:**
- Modify: `E:\Scripts\llm-stack\stack.json` (add `"critical": true,` to the `gateway` entry; the
  `omniroute` entry from Task 1 already has it)
- Modify: `E:\Scripts\Castellyn\src-tauri\src\lib.rs` (`StackHealth` struct + the `Row` struct and
  its construction in `read_stack_health_blocking`, ~`:3695`/`:3721`/`:3765`)
- Modify: `E:\Scripts\Castellyn\src\lib\ipc.ts` (`StackHealth` type, ~`:359`)
- Modify: `E:\Scripts\Castellyn\src\lib\components\StackHealthCard.svelte` (replace `id === 'gateway'`
  with `.critical`; ~`:55`/`:66`/`:70`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `StackHealth.critical: bool` (Rust) / `critical: boolean` (TS) — true for services the
  registry marks as the client-facing front. The card treats "any enabled critical service not up"
  as the overall `down` alarm.

- [ ] **Step 1: Rust — add `critical` to the struct + reader**

In `StackHealth` (~`:3695`), after `healthy`:
```rust
    /// This service is the client-facing front — its outage is the overall alarm (data-driven,
    /// replaces the old hardcoded `id == "gateway"`). From stack.json `critical` (default false).
    pub(crate) critical: bool,
```

In the local `Row` struct (~`:3721`), after `health: String,`:
```rust
        critical: bool,
```

In the `Row { ... }` construction (~`:3731`), after `health: s(e, "health"),`:
```rust
            critical: e.get("critical").and_then(|x| x.as_bool()).unwrap_or(false),
```

In the final `StackHealth { ... }` map (~`:3765`), after `healthy,`:
```rust
            critical: r.critical,
```

- [ ] **Step 2: stack.json — mark `gateway` critical**

Add `"critical": true,` to the `gateway` entry (the `:13001` one). Place it after `"openDashboard": true,`.
(The `omniroute` entry already carries `critical:true` from Task 1.) UTF-8, no BOM.

- [ ] **Step 3: ipc.ts — extend the type**

In `StackHealth` (~`:359`), after `healthy: boolean | null; ...`:
```ts
  critical: boolean; // client-facing front — its outage drives the overall 'down' alarm
```

- [ ] **Step 4: StackHealthCard.svelte — replace the hardcodes**

(a) `dotColor` (~`:54-59`) — the comment and the branch:
```svelte
  // A stopped non-critical backend is neutral grey; a dead critical front / sick service keep alarm colours.
  function dotColor(s: StackHealth): string {
    const st = statusOf(s);
    if (st === 'down') return s.critical ? dot.down : dot.off;
    return dot[st];
  }
```

(b) Replace the single-`gateway` derived (~`:66`) with a critical-set derived:
```svelte
  const criticals = $derived(enabled.filter((i) => i.critical));
```

(c) `overall` (~`:70-80`) — swap the gateway clause for "any critical not up":
```svelte
  // ok=all up · degraded=a service is sick (port open, /health fails) · stopped=some backends are
  // just off (normal) · down=a critical front is unreachable (the only real outage).
  const overall = $derived<'ok' | 'degraded' | 'stopped' | 'down'>(
    total === 0
      ? 'down'
      : criticals.some((c) => statusOf(c) !== 'up')
        ? 'down'
        : anySick
          ? 'degraded'
          : ups === total
            ? 'ok'
            : 'stopped'
  );
```

(d) Update the block comment at ~`:38-40` ("only the gateway (the critical hop)...") to read
"only a critical front being down is treated as an alarm." No other logic changes.

- [ ] **Step 5: Gates**

- `cargo test` (in `src-tauri`) → all pass (struct/reader compile; existing tests green).
- `cargo clippy` → 0 warnings.
- `npm run check` → 0/0 (svelte-check validates the new `critical` field usage).
- `npm test` → unchanged (no outcome/attention change).
- `npm run check:i18n` → unchanged.

**Test rationale (why no new unit test):** the change is a data-driven generalization of an
existing inline `$derived`. It is regression-free *by construction* — `gateway` stays
`critical:true`, so during transition the enabled-critical set is exactly `{gateway}` and the
`overall` output is identical to today. The only new behavior (a second critical service) is inert
until `omniroute` is enabled live. svelte-check + the preserved-invariant is the check; extracting
the derivation into a testable module would be speculative scaffolding (YAGNI). If the final review
judges a test Important, extract `stackOverall()` then.

- [ ] **Step 6: Commit (two repos)**

llm-stack (ONLY stack.json):
```bash
cd E:/Scripts/llm-stack && git add stack.json && git commit -m "feat(stack): mark gateway as critical front"
```
Castellyn:
```bash
cd E:/Scripts/Castellyn && git add src-tauri/src/lib.rs src/lib/ipc.ts src/lib/components/StackHealthCard.svelte && git commit -m "feat: data-driven critical front in health card (Ф5)"
```

---

## Self-review (author)

- **Spec coverage:** Ф4 = entry + `omniroute_base_url()` (Task 1). Ф5 = `critical` field + card
  swap (Task 2). Ф6/Ф7 explicitly out of scope (owner decision).
- **Type consistency:** `critical` is `bool`(Rust)/`boolean`(TS), added to both `StackHealth`
  shapes; `omniroute_base_url` returns `Option<String>`/`string|null` — matches `gateway_base_url`.
- **No placeholders:** every step has exact code. The only intentionally-empty fields (`health:""`,
  no DATA_DIR env) are documented live-config deferrals, not TODOs.
