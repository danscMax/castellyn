# Plan 3b — OmniRoute integration Ф6+Ф7 (Codex→OmniRoute + supervisor seams)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> **Part A** (Tasks A1-A4) is executable now — unit-tested, regression-free, inert until OmniRoute
> is enabled live. **Part B** is the owner's live-session checklist (NOT for implementers).
> Owner-approved scope (2026-07-07): Ф6 = Codex→OmniRoute + docs (the `direct/openai` arm relax was
> dropped as obsolete — Claude uses the anthropic arm to :20128, OpenCode already reaches any base
> URL). Ф7 = supervisor code seams now, live registration/health/enable later.

## Global Constraints

- **Two repos.** `stack.json` (`dependsOn` field examples only — real omniroute values set in Part B)
  lives in **llm-stack** (`E:\Scripts\llm-stack`, branch `master`); `git add stack.json` ONLY
  (foreign dirty files `start-stack.ps1`, `start-stack.ps1.nobom.bak` must never be swept). lib.rs /
  ipc.ts / i18n / DESIGN.md live in **Castellyn** (`E:\Scripts\Castellyn`, branch
  `feat/omniroute-integration-3b`).
- **Regression-free:** every new runtime path is opt-in via a manifest field that is ABSENT until
  Part B (`dependsOn`, `healthTimeoutSec`, omniroute `enabled:true`). `run_codex_providers`
  (freellmapi) output must stay byte-identical.
- **No secrets, no AI attribution.** JSON as UTF-8 without BOM. i18n ru/en/zh parity
  (`npm run check:i18n`); never shadow the `t` translation fn.
- **cargo:** `C:\Users\User\.cargo\bin\cargo.exe` (not on PATH). Gates each task: `cargo test`,
  `cargo clippy` (0), `npm run check` (0/0), `npm run check:i18n`.
- Reuse: `omniroute_base_url()` (`lib.rs:5929`) + dangling `omnirouteBaseUrl` ipc wrapper;
  `kill_tree` (`1436`) + `StackProcs` (`2936`) + `save_stack_procs` for teardown; `ready_timeout_secs`
  shape (`3053`) for the health-timeout reader.

---

## Part A — executable now

### Task A1 — Generalize the Codex provider patch (Ф6 code)

**Files:** `src-tauri/src/lib.rs` (`patch_codex_gateway` ~8873, `run_codex_providers` ~8911, handler
list ~14138, test module ~9025), `src-tauri/src/i18n.rs`, `src/lib/ipc.ts`.

1. Parameterize `patch_codex_gateway(toml_text, base_url)` →
   `patch_codex_provider(toml_text, provider_id, display_name, base_url, env_key, seed_model)`.
   The four literals (`"freellmapi"`, `"FreeLLMAPI"`, `"FREELLMAPI_API_KEY"`, `"kimi-k2-thinking"`)
   become params. Everything else unchanged: `base_url` still gets `/v1` appended, profile `model`
   seeded only when absent, top-level `model`/`model_provider` untouched.
2. `run_codex_providers` calls
   `patch_codex_provider(text, "freellmapi", "FreeLLMAPI", &base, "FREELLMAPI_API_KEY", "kimi-k2-thinking")`
   — **byte-identical** output; existing test `codex_gateway_patch_preserves_config_and_user_model`
   (`lib.rs:9025`) must stay green.
3. New command `run_codex_omniroute()` mirroring `run_codex_providers` but:
   `base = omniroute_base_url().ok_or(tr("err.omniroute_missing", cur_lang()))?`;
   `patch_codex_provider(text, "omniroute", "OmniRoute", &base, "OMNIROUTE_API_KEY", "kimi-k2-thinking")`;
   **no key mirror** (OmniRoute's key comes from `omniroute keys` — Part B); write config, return
   `Ok(false)`. Register in `generate_handler!`; add `ipc.ts` wrapper `runCodexOmniroute`.
4. i18n `err.omniroute_missing` (ru/en/zh) mirroring `err.gateway_missing`.

**Test:** `codex_omniroute_patch_writes_provider_and_profile` (mirror `…preserves_config…` at 9025):
`[model_providers.omniroute]` has `name`/`base_url=…/v1`/`env_key="OMNIROUTE_API_KEY"`;
`[profiles.omniroute]` has `model_provider="omniroute"` + seeded model; an existing user `model`
under the profile is preserved on re-patch. Freellmapi test stays green.

> ⚠ Do NOT surface a "Deploy Codex→OmniRoute" UI button — the config only works once `:20128` serves
> `/v1/responses` (Part B). Land command + test only.

### Task A2 — Dependency-ordered start (Ф7 code)

**Files:** `src-tauri/src/lib.rs` (`native_stack_start` loop head ~3143; new pure helper + test).

Add optional `"dependsOn": ["id", …]` to stack.json entries (real values in Part B). Extract:
```rust
fn order_services(services: &[serde_json::Value]) -> Vec<serde_json::Value>
```
Stable topological sort (Kahn): a service comes after every id in its `dependsOn` that is present in
the list; ties break by original manifest index; a cycle or a dep on a missing id → the affected
node falls back to manifest order, never panics (log via return or a side channel is not needed —
keep it pure; the caller logs if it wants). Replace `for svc in stack_services()` at `lib.rs:3143`
with `for svc in order_services(&stack_services())`. Only the start loop consumes ordering.

**Tests (pure):** deps precede dependents; missing/disabled dep → dependent still emitted, no hang;
cycle a→b→a → manifest-order fallback, no panic; no `dependsOn` anywhere → output order == input
order (regression guard).

### Task A3 — Teardown-on-critical-failure + configurable health timeout (Ф7 code)

**Files:** `src-tauri/src/lib.rs` (`native_stack_start` failure arms 3213/3238 + spawn insert 3202;
`native_wait_ready` health-wait constant `15` at 3098; new pure helpers + tests). Reuse `kill_tree`,
`StackProcs`, `save_stack_procs`.

1. Track this run's starts: `let mut started_pids: Vec<(String, u32)> = vec![];` push `(sid, pid)` at
   the successful-spawn insert (`lib.rs:3202`).
2. Teardown gate: when a `critical:true` service fails (`Readiness::Down` 3213 or spawn `None` 3238),
   after the `[fail]` log, if `should_teardown(is_critical)` → `kill_tree` each tracked pid, remove
   from `procs`, `save_stack_procs`, log `[teardown] critical <name> failed — rolled back N`, then
   `break`. Non-critical failure = today's behavior (log, continue). Read `critical` from `svc`
   (`svc.get("critical").and_then(as_bool).unwrap_or(false)`).
   `fn should_teardown(failed_is_critical: bool) -> bool { failed_is_critical }`.
3. `fn health_timeout_secs(svc: &Value) -> u64` = `svc.healthTimeoutSec` (u64, filter >0) else 15;
   use it at `lib.rs:3098` instead of the literal `15` (mirror `ready_timeout_secs` at 3053).

**Tests:** `should_teardown(true|false)`; `health_timeout_secs` present>0 / absent→15 / zero→15
(mirror `ready_timeout_secs` test at 4292). Real pid-kill/timing = live (Part B).

### Task A4 — Docs: pointing each client at OmniRoute

**Files:** `plans/omniroute-stack/DESIGN.md` — fix the §6 Ф6 bullet to the corrected premise, and add
a short "Clients → OmniRoute" subsection:
- **Claude Code:** my-provider `protocol=anthropic`, `baseUrl=http://localhost:20128/v1`,
  `connectVia=direct` → bind profile (existing `direct/anthropic` arm). Zero code.
- **OpenCode:** add OmniRoute as engine/myprovider → Connect to OpenCode (existing
  `run_opencode_provider`). Zero code.
- **Codex:** `run_codex_omniroute` (A1) — after Part B `/v1/responses` check; set `OMNIROUTE_API_KEY`
  from `omniroute keys`.

---

## Part B — live session checklist (owner; NOT for implementers)

Needs a running, configured OmniRoute:
1. `npm i -g omniroute`; `omniroute serve --no-open --no-tray` on real DATA_DIR; change `CHANGEME`.
2. Pin the real unauthorized-2xx health route → stack.json `omniroute.health` (+ `healthTimeoutSec`
   if slow). Closes the wedge risk (DESIGN §10).
3. Add `env` DATA_DIR var (exact name confirmed live), set `enabled:true` + `dependsOn:[backends]`
   + `teardownOnFailure:true` (opt-in supervisor rollback; A3 decoupled it from the health-`critical`
   flag so only the front rolls the stack back on its own start failure).
4. `omniroute providers add/keys add/nodes add` for 16 key-providers + local engines; disable
   provider-retry for unstable upstreams, timeout > worst-case.
5. Verify `/v1/responses`; set `OMNIROUTE_API_KEY`; surface the Codex→OmniRoute UI trigger; smoke.
6. Full live smoke (DESIGN §8) → push both repos.

## Verification (Part A)

`cargo test` (new: `order_services`, `should_teardown`, `health_timeout_secs`,
`codex_omniroute_patch…`; existing freellmapi codex test green) · `cargo clippy` 0 · `npm run check`
0/0 · `npm run check:i18n` (+1 key ×3). Regression proof: no `dependsOn`/`healthTimeoutSec`/critical
failure + unchanged `run_codex_providers` → runtime identical to pre-3b.
