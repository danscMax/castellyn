# Performance Findings — Castellyn (2026-07-05, HEAD 1c02e3d)

Axis: Performance. Scope: measurable/observable effects only (UI jank, memory growth, event
flood, startup delay). Micro-optimizations excluded per briefing.

**Headline:** The genuinely hot paths — the PowerShell run-log stream, the PTY byte stream, and
app startup — are already well-engineered (coalesced batching, binary Channels + WebGL, off-thread
warm-ups, a cached config). No Critical/High issues. The reportable items are three: a class of
**heavy synchronous Tauri commands that run on the main/UI thread** (the biggest), an
**unvirtualized console log** that churns the DOM when open during a verbose run, and a **serial
limits poll** that a hung profile can stall.

Counts: Critical 0 · High 0 · Medium 2 · Low 1.

---

## [PERF-1] Heavy directory-scan / multi-read commands run synchronously on the Tauri main thread — Medium
**File:** `src-tauri\src\lib.rs` — `list_skills` (6825), `read_environments` (7126),
`read_skill_matrix` (7316), `read_profile_matrix` (4097), `list_plugin_contents` (8561),
`read_stack_drift` (9918), `read_mcp` (6233), `read_opencode` (5925), `read_codex_profiles` (6631)

**Description:**
In Tauri v2, commands *without* the `async` keyword execute on the **main thread**; async commands
run on a separate async task (verified against the Tauri v2 docs, see Sources). The codebase already
knows this — every HTTP probe and the other native readers are deliberately pushed off-thread with
`tokio::task::spawn_blocking` (`read_engines` 2467, `read_stack` 2555, `read_stack_health` 2907,
`read_sync` 2300, all the `ureq` probes). But the directory-walking readers above are plain `fn`
commands, so their file-system work runs on the UI event-loop thread.

Several of them do real work, not a single small read. `read_profile_matrix` loops every profile ×
every shared folder doing a `symlink_metadata` stat each, plus per-profile `settings.json`,
`enabledPlugins`, and MCP reads — dozens of syscalls per open. `list_skills` walks the skills tree
and reads a `SKILL.md` front-matter per skill; `read_environments`/`read_skill_matrix` union skill
sets across three harness roots (`skill_names_in` does a `read_dir` + `SKILL.md` `is_file` probe per
dir). On a power-user install with many profiles/plugins/skills this is tens-to-hundreds of ms of
blocking fs work on the thread that also paints the window — a brief freeze each time the
Skills/Environments/Matrix/Plugins tab is opened or refreshed (e.g. after a `run-done`
`reloadExtensions`).

**Evidence:**
```rust
// list_skills — plain `fn` command → runs on the main thread; walks the tree + reads SKILL.md each
fn list_skills() -> Vec<SkillInfo> {
    ...
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            let p = e.path();
            ...
            out.push(read_skill_info(&p, ...));   // reads SKILL.md front-matter per skill
        }
    }
    out.extend(plugin_bundled_skills());
```
```rust
// read_profile_matrix — plain `fn`; per profile × folder does a symlink stat, plus per-profile reads
fn read_profile_matrix() -> Result<Vec<MatrixRow>, String> {
    ...
    let folders = defaults.iter().map(|folder| MatrixFolder {
        ...
        actual: classify_link(&profile_dir.join(folder)).to_string(),  // symlink_metadata per folder
    }).collect();
    ...
    let deployed_all = profile_mcp_servers(name).unwrap_or_default();   // another file read per profile
```
Contrast with the already-correct pattern the same file uses elsewhere:
```rust
#[tauri::command]
async fn read_stack() -> Vec<StackService> {
    tokio::task::spawn_blocking(read_stack_blocking)   // <- off the UI thread, on purpose
        .await.unwrap_or_default()
}
```

**Fix suggestion:** Make the heavy readers `async fn` and move the body into
`tokio::task::spawn_blocking`, exactly mirroring `read_stack`/`read_engines`. Prioritize
`read_profile_matrix`, `list_skills`, `read_environments`, `read_skill_matrix`, `list_plugin_contents`
(the ones doing real dir walks); the light single-read ones (`read_mcp`, `read_opencode`,
`read_codex_profiles`) are optional. This is a mechanical change with an existing in-repo template and
no behavior change.

---

## [PERF-2] Console renders the full log unvirtualized (up to 5000 DOM nodes) and re-diffs the whole non-keyed list on each append at cap — Medium
**File:** `src\lib\components\Console.svelte:151-158`; buffer cap in `src\routes\+page.svelte:183-190`

**Description:**
The console renders one `<div>` per log line via a **non-keyed** `{#each log as line}` with no
virtualization. The buffer is capped at `MAX_LOG = 5000` (good — it is *not* unbounded memory), but
5000 live DOM nodes is already heavy, and the trimming strategy makes updates O(n): once the buffer
is full, each append does `log.splice(0, log.length - MAX_LOG)` — a **front** removal. In a non-keyed
each, shifting every element by one index changes the value bound to every block, so Svelte updates
the text of *all* rendered lines, not just the one added. Svelte 5 batches the per-line `appendLog`
calls within one coalesced `run-log` event (≤64 lines) into a single reconciliation, which softens
it, but at ~33 events/sec (the backend's 30 ms flush cadence) with the dock open during a firehose
run (e.g. a long `Update-All`), that is on the order of 10^5 text-node writes per second — visible
jank while the log is expanded. The dock is collapsed by default and gated behind `{#if !collapsed}`,
so the cost only materializes when the user opens it during a heavy run — which is exactly when they
open it.

**Evidence:**
```svelte
<!-- Console.svelte: no key, no virtualization; every line is a live DOM node -->
{#each log as line}
  <div
    class="log-line"
    class:log-warn={line.startsWith('⚠')}
    ...
  >{line}</div>
{/each}
```
```ts
// +page.svelte: front-splice trim → shifts every index → non-keyed each re-diffs the whole list
const MAX_LOG = 5000;
function appendLog(line: string) {
  log.push(line);
  if (log.length > MAX_LOG) log.splice(0, log.length - MAX_LOG);
}
```

**Fix suggestion:** Cheapest: give each entry a stable id and key the each
(`{#each log as entry (entry.id)}`), so a front `splice` removes exactly one DOM node and appends one
— O(1) per change instead of O(n). Or lower the rendered window (only the last ~1000–2000 lines are
ever read on screen) / add windowing. Keying is the smallest diff and removes the churn entirely.

---

## [PERF-3] Usage-limit monitor polls profiles serially with an 8 s timeout — one hung profile stalls the rest — Low
**File:** `src-tauri\src\limits.rs:190-254`

**Description:**
`limits::start` polls every profile's OAuth usage sequentially each cycle; `fetch_usage` has an 8 s
global timeout. A profile whose endpoint hangs (network, throttling) blocks the loop for up to 8 s
before the next profile is even attempted, so later profiles' `limits-status` events are delayed by
up to `8 s × (# preceding slow profiles)`. This is on a background thread (never blocks the UI) and
the cycle only runs every 5 minutes, so the impact is a laggy/stale limits chip, not a freeze —
hence Low. It becomes noticeable only with several profiles whose tokens are expired/slow.

**Evidence:**
```rust
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(POLL_SECS));   // 300 s
        ...
        for (name, _settings) in crate::plugin_sync_profiles(&home) {
            let cred = format!("{home}\\{name}\\.credentials.json");
            poll_profile(&app, &name, &cred);                 // serial; each can block up to 8 s
        }
    });
}
```

**Fix suggestion:** Either lower `HTTP_TIMEOUT_SECS` (8 s is generous for a JSON GET), or fan the
per-profile polls out onto short-lived threads / a small pool and join, so one slow profile can't
delay the others. Given the 5-minute cadence and background nature, this is optional polish.

---

## Clean areas (checked, no action)

- **PowerShell run-log streaming (`pump_stream`, lib.rs:798-852).** Already coalesces rapid lines
  into one `run-log` event on a 30 ms cadence / 64-line batch instead of one IPC event per line, with
  cancellation-safe reads. This is the correct fix for event-flood to the webview — no change needed.
- **PTY byte streaming (`session_spawn` reader thread, lib.rs:12161-12205).** 32 KiB read buffer,
  raw bytes over a binary `tauri::ipc::Channel` (no base64/JSON per chunk), a bounded scrollback ring,
  and dead-channel pruning via `retain`. `TerminalPane.svelte` writes straight to xterm with the WebGL
  renderer. Per-chunk work (`on_output` atomics, a 512-byte `scan_limit` tail, one ring push) is
  trivial. Well-optimized.
- **Console log buffer is capped** at `MAX_LOG = 5000` and uses reactive push/splice — no unbounded
  memory growth on long runs (the only issue is the render churn in PERF-2, not the buffer itself).
- **`runHistory`** and toast stores are bounded stores, not accumulate-forever arrays.
- **Startup (`setup()`, lib.rs:13057-13114).** The window is created by the Tauri config (shown
  immediately); `setup` warms `is_elevated()` on a background thread, builds the tray, and starts the
  two monitors — no heavy blocking work before the window paints.
- **`read_config_file` (lib.rs:237-244)** is backed by `CONFIG_CACHE` (RwLock), so the many call
  sites (window events, poll loops, alert paths) don't re-read/parse `config.json` from disk each time.
- **Frontend polling is minimal:** only `ProfileUsageBadge` (60 s) and `SessionsTab` auto-continue
  (12 s). The heavy readers are called on-demand (tab open / `run-done`), not on an interval — so
  PERF-1's cost is per-interaction, not continuous.
- **`TerminalPane` data-in** writes `term.write(new Uint8Array(buf))` directly; xterm does its own
  internal write batching. No per-byte reactivity.

## Sources (Tauri threading model, load-bearing for PERF-1)
- [Calling Rust from the Frontend — Tauri v2](https://v2.tauri.app/develop/calling-rust/) — "Commands
  without the async keyword are executed on the main thread unless defined with
  `#[tauri::command(async)]`"; async commands run on a separate task via `async_runtime::spawn`.
