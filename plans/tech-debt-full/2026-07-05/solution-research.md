# Solution Research — non-trivial tech-debt fixes

Scope: best-practice implementations for the 5 non-trivial verified findings only
(V-4, V-5, V-7, V-10, V-14). The one-liners (V-1 `str::get`, V-3 guard swap, V-6
`countOf`, V-11 denylist chars, V-12 `max_redirects(0)`, V-17 shared normalizer) need no
research. Grounded against HEAD `1c02e3d` where the code is quoted.

Lead pre-verified (not re-researched here): plist 1.10.0 needs quick-xml ^0.41 (so
`cargo update -p plist` is the V-2 candidate); ureq 3 defaults `max_redirects=10` and
`redirect_auth_headers=Never`; Tauri v2 sync commands run on the main thread.

---

## V-5 — Console log rendering (Svelte 5)

**Problem (Console.svelte:151, +page.svelte:185-191):** `{#each log as line}` over a
`string[]` with no key, plus `log.splice(0, log.length - MAX_LOG)` (MAX_LOG=5000) front-
trimming inside a resizable dock. Non-keyed + front-splice is the pathological combination:
every splice shifts every remaining line's positional index, so Svelte's non-keyed diff
re-renders (or at minimum re-checks) the entire list on each append+trim cycle.

### Options

**A. Windowed render — "last N + show-all" (zero-dep).**
Keep the full `log: string[]` buffer for search/copy, but bind the each-block to a derived
slice: `const view = $derived(showAll ? log : log.slice(-500))`. Render only the visible
tail; a "show all 5000" toggle swaps to the full buffer on demand. 500 DOM nodes is trivial
for any browser; the append path only ever adds/removes at the end of a ≤500 array.
- *Pros:* no dependency, no per-line wrapper allocation, smallest diff, kills the O(n) splice
  churn outright (the trim now happens on data the DOM doesn't mirror). Copy/search still see
  the whole buffer.
- *Cons:* scrollback capped at N unless the user hits "show all"; "show all" briefly mounts
  5000 nodes (acceptable — it's an explicit, rare action, and a console log-dump legitimately
  is 5000 nodes).

**B. Keyed each with a stable-id wrapper.**
Change the buffer to `{ id: number, text: string }[]` (monotonic counter as id) and key the
block `{#each log as line (line.id)}`. Svelte then moves/keeps existing DOM nodes across the
front-splice instead of re-rendering them.
- *Pros:* full 5000-line scrollback stays live; idiomatic Svelte fix for the exact "no key"
  finding. Keying is the [documented](https://svelte.dev/docs/svelte/each) answer for
  add/remove-in-the-middle lists.
- *Cons:* still mounts up to 5000 real DOM nodes (keying makes updates cheap, not the node
  count small); a wrapper object + id counter to thread through every `pushLog` call site;
  more churn than A for the same user-visible result at this size.

**C. Virtual-list library (svelte-virtual-list / svelte-tiny-virtual-list / @tanstack/virtual).**
Only render rows in the viewport.
- *Pros:* scales past 5000 without node-count cost.
- *Cons:* new dependency; most Svelte-5-native virtual-list crates are thin/young; variable-
  height rows (wrapped long log lines) fight fixed-height virtualization; overkill for a
  hard 5000-line cap. Fails the ladder — a windowed slice is a few lines and covers it.

### Recommendation
Option **A (windowed render, zero-dep)**. It removes the actual cost (whole-list diff on
every splice) with the smallest diff and no dependency, and a 5000-cap console never needs
true virtualization. Keep the full buffer for copy/search; render `log.slice(-500)` with a
"show all" escape hatch. If, and only if, live full-scrollback with cheap updates is a hard
requirement, fall back to B (keyed wrapper) — do not add C for a capped log.
**Source:** https://svelte.dev/docs/svelte/each (keyed-each semantics),
https://svelte.dev/tutorial/svelte/keyed-each-blocks

---

## V-4 — Heavy dir-scan commands on the main thread

**Problem:** `read_profile_matrix` (:4098), `list_skills` (:6825), `read_environments`
(:7126), `read_skill_matrix` (:7316), `list_plugin_contents` (:8561) are plain `fn` under
`#[tauri::command]`. Tauri v2 runs a *synchronous* command on the main thread (lead-verified),
so these fs-walks block the UI event loop. The in-repo template already exists:
`check_provider_balance` (:5713) is `async fn` + `tokio::task::spawn_blocking`.

**Panic-unwind constraint (confirmed at Cargo.toml:71-78):** the release profile is
deliberately **not** `panic="abort"` precisely because "native ops rely on unwinding a
panicked `spawn_blocking` closure into a `JoinError` (the `.unwrap_or` fallbacks) — abort
would crash the whole app instead." So the JoinError path is load-bearing and must be
preserved by any conversion.

### The idiomatic Tauri v2 pattern
```rust
#[tauri::command]
async fn read_profile_matrix(/* args: owned/Clone, no borrowed State across await */)
    -> Result<Matrix, String>
{
    tokio::task::spawn_blocking(move || {
        // the existing synchronous body, verbatim
    })
    .await
    .map_err(|e| format!("task failed: {e}"))?   // JoinError → surfaced, not swallowed
    // inner Result<Matrix,String> flows through
}
```

### Gotchas when converting `fn` → `async fn` (Tauri v2)
1. **`State` is not `Send` across `spawn_blocking`.** Read what you need from
   `State<'_, _>` (config path, roots) *before* the closure and move owned copies in. The
   five targets are readers — pull `scripts_root()` / config once, then walk. Do not hold a
   `MutexGuard` across the `.await`.
2. **Return type is unchanged.** An `async` command still returns `Result<T, String>` (or
   `T`); Tauri serializes the awaited value identically. No signature change visible to TS.
3. **`#[tauri::command(async)]` is a different tool.** The `(async)` attribute on a *sync*
   `fn` merely tells Tauri to run that sync body on a worker thread from the async runtime —
   it does **not** give you an async body or `spawn_blocking`'s dedicated blocking pool, and
   it still can't hold non-`Send` state across anything. For fs-walk offload, prefer explicit
   `async fn` + `spawn_blocking` (matches the existing `check_provider_balance` house style
   and keeps the JoinError-unwrap contract visible in one place).
4. **JoinError handling must stay unwind-based.** Map the outer `JoinError` to an error
   string (as above) or, where the code wants the "return a default on failure" behavior,
   keep the existing `.unwrap_or(...)` on the *inner* result. Either way the closure panic
   unwinds into a `JoinError` rather than aborting — which only holds because the profile is
   not `panic="abort"` (Cargo.toml:73). Do not change that profile.

### Recommendation
Convert the five readers to `async fn` + `tokio::task::spawn_blocking(move || { …body… })`,
mirroring `check_provider_balance` exactly (same crate, same style). Snapshot any needed
`State`/config values *before* the closure; map the `JoinError` to a `String` error (or keep
the inner `.unwrap_or` default). Preserve `panic != "abort"`.
**Source:** https://v2.tauri.app/develop/calling-rust/ ,
https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html ,
https://docs.rs/tokio/latest/tokio/task/struct.JoinError.html ,
in-repo precedent `check_provider_balance` lib.rs:5713.

---

## V-7 — Stringly-typed stream-id contract (Rust ↔ TS)

**Problem:** 18 spawn sites pass hard-coded `run-done`-family id literals (forks:1058 …
onboarding:10102); the frontend `run-done` ladder (+page.svelte:2069-2209) dispatches on an
untyped string. No compile-time binding on either side; a renamed/typo'd id fails silently at
runtime. Constraint: `lib.rs` is one 14k-line file and the project values minimal deps.

### Options

**A. Hand-mirrored constants + TS union + one parity test (zero-dep).**
Rust: a small `mod stream_id { pub const FORKS: &str = "forks-done"; … }` (or an enum with a
`&str` mapping) used at all 18 spawn sites. TS: `export const STREAM_IDS = [...] as const;
export type StreamId = typeof STREAM_IDS[number];` and key the reload map by `StreamId`. One
Vitest asserting the TS array equals a Rust-emitted list.
- *Pros:* no dependency; compile-time exhaustiveness on the *TS* side (the reload map must
  cover the union); grep-able single source per language; fits the existing i18n-parity-test
  idiom the repo already runs (`check:i18n`). Smallest footprint in a 14k-line file — a
  `const` module, not a codegen build step.
- *Cons:* two lists to keep in sync (mitigated by the test); the test is the enforcement, not
  the compiler across the boundary.

**B. Codegen — tauri-specta or ts-rs.**
Generate the TS types/bindings from Rust annotations.
- *tauri-specta* is still **`2.0.0-rc.24`** as of 2026 — years in RC, no 1.0-for-v2 stable
  line. Adopting an RC as a boundary-critical build dependency in an app that "values minimal
  deps" is a poor trade for 18 string constants.
- *ts-rs* generates TS *types* from Rust structs but is aimed at data types, not command-id
  string unions; you'd still hand-write the id set — it doesn't solve *this* problem.
- *Pros:* eliminates hand-mirroring for large type surfaces.
- *Cons:* new build-step dependency, RC stability (specta), macro annotations across 14k
  lines, generated-file review noise. Disproportionate to a fixed set of ~18 ids.

**C. Minimal `as const` + generated JSON equality (variant of A).**
Rust `build.rs`/test writes the id list to a JSON; TS test imports it and asserts equality.
- *Pros:* single source of truth is the Rust list.
- *Cons:* adds a build/test artifact and a codegen step for marginal gain over A's two-list-
  plus-test; more moving parts in a repo that keeps parity via plain unit tests already.

### Recommendation
Option **A**: a Rust `stream_id` const module (used at all 18 sites) + a TS `as const` array
with a derived `StreamId` union keying the reload map, enforced by one parity unit test in the
existing test idiom. Do **not** pull in tauri-specta (RC in 2026) or ts-rs for 18 string ids —
that fails the minimal-deps bar and the ladder. Codegen earns its keep when the shared surface
is dozens of evolving structs, not a fixed id enum.
**Source:** https://github.com/specta-rs/tauri-specta/releases (still 2.0.0-rc.* in 2026),
https://github.com/Aleph-Alpha/ts-rs (type-focused, not command-id unions),
in-repo precedent: i18n parity test (`src/lib/i18n/index.test.ts`, `npm run check:i18n`).

---

## V-10 — freellmapi key via `setx` → plaintext HKCU\Environment

**Problem (lib.rs:7989-7994):** `Command::new("setx").args(["FREELLMAPI_API_KEY", &key])`
persists the key as plaintext in `HKCU\Environment` (persistent at-rest) and exposes it on
argv (transient). The stated goal: the `codex` CLI, run in **fresh user-opened terminals**,
needs `FREELLMAPI_API_KEY` present.

### Options

**A. Codex-native env config — `env_key` + `shell_environment_policy` (best fit).**
Codex reads its provider API key from the env var **named by** the provider's `env_key` in
`~/.codex/config.toml`, and `shell_environment_policy` (with a `set = { … }` table) controls
what env subprocesses see. Two sub-routes:
  - If Castellyn already spawns codex itself (it does — `run_codex_mcp`, and codex provider
    wiring exists per the env-platform work), inject the key **per-spawn** via
    `Command::env("FREELLMAPI_API_KEY", &key)` on that child only. Nothing persists to the
    registry; the key never touches HKCU. This is the clean fix for Castellyn-launched codex.
  - For the "codex in a terminal the *user* opened by hand" case, point codex's provider at a
    var and let codex's own config carry it, rather than a global setx. (Note: putting the raw
    key in config.toml is also plaintext-at-rest, so it's not strictly better than setx for
    at-rest exposure — but it scopes to codex instead of every process on the machine.)
- *Pros:* removes the machine-wide plaintext env var; scopes the secret to codex; per-spawn
  `.env()` leaves nothing at rest at all for the paths Castellyn controls.
- *Cons:* per-spawn `.env()` only covers Castellyn-launched codex, not a terminal the user
  opens independently.

**B. Per-spawn `.env()` only (scope to what Castellyn launches).**
Drop the setx entirely; set the var on each codex `Command` Castellyn spawns.
- *Pros:* zero persistence, zero argv exposure (env is not argv); smallest, safest diff.
- *Cons:* does **not** serve a user who opens their own terminal and runs `codex` there.

**C. Keep setx but document it (status-quo).**
If "codex in an arbitrary user-opened fresh terminal" is a genuine hard requirement, Windows
offers no non-persistent, non-plaintext way to inject an env var into *future, externally-
launched* shells — `setx` (or a shell-profile hook, equally plaintext) is the only robust
route. Then the finding is **documentation-only**: note the plaintext-at-rest tradeoff and
that the secret already lives in Credential Manager as the source of truth.

### Recommendation
Prefer **B for Castellyn-launched codex** (per-spawn `Command::env(...)`, delete the `setx`
call) — it eliminates both the argv and the at-rest exposure for everything the app controls,
smallest diff. If product genuinely needs the key in **user-opened** terminals, that is the
one case with no clean Windows primitive: keep it **documentation-only** (option C), because
setx/profile-hook are the only mechanisms and both are plaintext-at-rest — do not pretend a
"secure setx" exists. Confirm which case is required before coding; V-10 is Low and may be
doc-only.
**Source:** https://developers.openai.com/codex/config-reference (`env_key`,
`shell_environment_policy`, `set`),
https://developers.openai.com/codex/config-advanced ,
https://codex.danielvaughan.com/2026/06/03/codex-cli-environment-variables-runtime-configuration-headless-ci-container-deployment/

---

## V-14 — No run-timeout backstop

**Problem (lib.rs:754):** `child.wait().await` in `pump_and_wait` is untimed; a wedged script
holds the single run slot forever. `cancel_run` exists but is user-initiated.

**Confirmed in-repo primitives:**
- The pid is stored in the run slot *before* `pump_and_wait` (`slot.set_pid(child.id())`,
  :662-664); `child.id()` is also still readable at the top of `pump_and_wait`.
- Tree-kill already exists: `kill_tree(pid)` runs `taskkill /PID <pid> /T /F`
  (lib.rs:1131-1147), tolerates exit 128 (already-gone), and is the shared kill path for
  `cancel_run` / `cancel_fork_repo` / `measure_context` timeout.
- `tokio` already has the `time` feature enabled (Cargo.toml:36).

**Why `child.kill()` is the wrong primitive here:** tokio's `Child::kill()` terminates only
the *direct* child. Castellyn's children are trees (`pwsh` → `claude`/`node`, `ssh.exe`), so
the existing `taskkill /T /F` tree-kill is required — do not substitute `child.kill()`.

### The composition
```rust
let pid = child.id();                       // grab before the wait consumes it
let status = match tokio::time::timeout(RUN_MAX, child.wait()).await {
    Ok(s) => s,                             // normal exit — exit-code path unchanged
    Err(_elapsed) => {                      // backstop fired
        if let Some(p) = pid { let _ = kill_tree(p); }  // tree-kill, reuse existing path
        child.wait().await                  // reap the now-killed child → no zombie/handle leak
    }
};
```

### Pitfalls (and how the above avoids them)
1. **Losing the exit-code path:** on the normal branch `status` is the real `ExitStatus`; the
   existing `status.ok().and_then(|s| s.code()).unwrap_or(-1)` (:759) is untouched. On timeout,
   the reaped status yields the kill's code (or -1), which is the correct "was killed" signal.
2. **Double-kill / cancel race:** `kill_tree` already tolerates "process not found" (exit 128
   → Ok), so a user `cancel_run` landing at the same instant as the timeout can't produce a
   false error. Idempotent by construction.
3. **Zombie / handle leak:** the timeout future drop does *not* reap the child; the explicit
   second `child.wait().await` after `kill_tree` reaps it. (tokio drops the OS handle on
   `Child` drop, but reaping keeps the status path clean and avoids relying on drop order with
   the still-running stdout/stderr pump tasks — those are awaited right after, as today.)
4. **Optional, not mandatory:** make `RUN_MAX` a generous const (e.g. tens of minutes) or an
   `Option<Duration>` so legitimately long runs (big fork syncs) aren't guillotined; a backstop
   is a safety net for *wedged* runs, not a QoS timeout. Given one-run-at-a-time, a wedge is
   what it protects against.

### Recommendation
Wrap the existing `child.wait().await` (:754) in `tokio::time::timeout(RUN_MAX, …)`; on
`Elapsed`, call the existing `kill_tree(child.id())` then re-`wait()` to reap. Reuse
`kill_tree` (not `child.kill()`) for the tree; keep the exit-code extraction as-is. Make
`RUN_MAX` generous/configurable so it only catches genuine wedges. All primitives already
exist — this is composition, not new machinery.
**Source:** https://docs.rs/tokio/latest/tokio/time/fn.timeout.html ,
https://docs.rs/tokio/latest/tokio/process/struct.Child.html (kill kills only the direct
child), in-repo `kill_tree` lib.rs:1131 and slot pid at :662.

---

## Summary table

| ID  | Fix | New dep? | Diff size |
|-----|-----|----------|-----------|
| V-5 | Windowed render: full buffer for copy/search, render `log.slice(-500)` + "show all" toggle | no | small |
| V-4 | `async fn` + `spawn_blocking(move || body)` on the 5 readers; snapshot State first; map JoinError; keep `panic!="abort"` | no | medium (5 fns) |
| V-7 | Rust `stream_id` const module + TS `as const` union keying the reload map + 1 parity test; **no** specta/ts-rs | no | medium |
| V-10 | Delete `setx`; per-spawn `Command::env()` for Castellyn-launched codex. If user-opened terminals required → doc-only (no clean Windows primitive) | no | small |
| V-14 | `tokio::time::timeout(RUN_MAX, child.wait())`; on Elapsed reuse `kill_tree(pid)` + re-`wait()`; keep exit-code path | no | small |

None of the five recommendations adds a dependency. Every one either reuses an existing
in-repo pattern (`check_provider_balance` spawn_blocking, `kill_tree`, i18n parity test) or a
stdlib/native primitive.
