# Reliability Findings — Castellyn (2026-07-05, HEAD 1c02e3d)

Axis: Reliability (panics, races, error handling, data loss, failure modes).
Scope per briefing. This codebase already went through a full goal-audit (2026-07-04); it is
unusually hardened — locks are poison-tolerant everywhere (`unwrap_or_else(|e| e.into_inner())`),
config writes are atomic temp+rename with `.bak` recovery, all `ureq` agents carry
`timeout_global`, PTY sessions kill their tree on close/exit, and the frontend `.catch()`es its
invoke calls. Findings below are what survived that bar.

---

## [REL-1] `expand_ssh_config` panics on a non-ASCII line in `~/.ssh/config` — String byte-slice not on a UTF-8 boundary — High
**File:** `src-tauri/src/lib.rs:12545-12547` (reached from the `read_ssh_hosts` command via `read_ssh_config_hosts` → `expand_ssh_config`, lines 12594-12530)
**Description:**
`expand_ssh_config` classifies every line of the user's `~/.ssh/config` (and any `Include`d file)
by slicing the first 7 **bytes** with `t[..7]` / `t[7..]`. The only guard is `t.len() > 7`, which
checks byte *length* but not that byte index 7 lands on a UTF-8 character boundary. Any line whose
7th byte falls inside a multi-byte character makes `t[..7]` (and `t[7..]`) panic
(`byte index 7 is not a char boundary`).

This is very reachable for **this** user: `expand_ssh_config` does **not** skip comment lines (unlike
`parse_ssh_config`, which skips `#`), so a Cyrillic comment such as `# рабочий сервер` is enough —
`# р` = `#`(0) ` `(1) `р`(2,3) `а`(4,5) `б`(6,7): byte 7 is the second byte of `б`, so the slice
splits it and panics. A Cyrillic `HostName`/`User`/`IdentityFile` value on a long-enough line does the
same. Given the environment is Cyrillic-heavy, a hand-edited ssh config with a Russian comment is a
plausible real file. The panic takes down the `read_ssh_hosts` command (the Sessions/SSH host list),
and a panic on a tokio blocking/worker thread can leave the invoke unresolved rather than cleanly
rejected.

**Evidence:**
```rust
for line in text.lines() {
    let t = line.trim();
    // Match the `Include` keyword followed by whitespace/'=' (not e.g. "IncludeFoo").
    let is_include = t.len() > 7
        && t[..7].eq_ignore_ascii_case("include")
        && t[7..].starts_with(|c: char| c.is_whitespace() || c == '=');
```
(No `#`-comment skip precedes this loop; every line reaches the slice.)

**Fix suggestion:**
Slice safely instead of by raw byte index. Minimal change:
```rust
let is_include = t.get(..7).is_some_and(|h| h.eq_ignore_ascii_case("include"))
    && t.get(7..).is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '='));
```
`str::get(..7)` returns `None` (rather than panicking) when 7 isn't a char boundary, so a Cyrillic
line simply isn't treated as an `Include` and is passed through as ordinary config text. Add a test
with a `# рабочий` comment line to lock it in.

---

## [REL-2] PTY reader thread `Child::wait` can block forever if a grandchild keeps the pty open — leaked session slot — Low
**File:** `src-tauri/src/lib.rs:12169-12205` (reader thread in `session_spawn`)
**Description:**
The reader thread breaks its loop on `read() == Ok(0)` (EOF), then calls blocking
`Child::wait(&mut *child)` and only afterwards removes the session from the map and frees its
`SESSION_LIMIT` slot. EOF on the master arrives when the last writer to the pty slave closes. If the
launched `pwsh` exits but a **grandchild** it spawned (a backgrounded `node`/`ssh`/detached process)
still holds the slave, EOF never arrives, the reader parks in `read()`/`wait()` indefinitely, and the
session keeps its slot, master handle and ring buffer until the pane is explicitly closed
(`session_kill`) or the app exits (the RunEvent::Exit drain at 13141-13146). A user who leaves the
"finished" pane open silently burns one of `SESSION_LIMIT` slots.

(Caveat: exact EOF timing under Windows ConPTY / portable-pty 0.8 depends on how the pseudoconsole
signals slave closure with lingering grandchildren; the leak is real when it doesn't, which is the
worst case worth guarding.)

**Evidence:**
```rust
loop {
    match reader.read(&mut buf) {
        Ok(0) | Err(_) => break,
        Ok(n) => { /* fan-out */ }
    }
}
// EOF means the child has exited; surface its real exit code (-1 if wait() fails).
let code = Child::wait(&mut *child)
    .map(|s| s.exit_code() as i32)
    .unwrap_or(-1);
```
**Fix suggestion:**
This is bounded already by the kill-on-close Job Object (`assign_to_kill_job`) at app exit, so it is
not a headless-orphan risk — only a slot leak while the app runs. If it proves real in practice,
either kill the tree (the Job Object / `killer.kill()`) once the direct child's exit status is
observed via a non-blocking `try_wait` poll loop with a short cap, or reap the map entry on the
`read()` EOF rather than after `wait()`. Low priority — leave unless slot exhaustion is observed.

---

## [REL-3] No automatic timeout on a script run — a hung PowerShell script holds the single run slot indefinitely — Low
**File:** `src-tauri/src/lib.rs:728-768` (`pump_and_wait`, `let status = child.wait().await;`)
**Description:**
The single-slot streamed runner awaits `child.wait()` with no timeout. A script that hangs (infinite
loop, a network call with no timeout of its own, an unexpected interactive `Read-Host` that no
`-Yes -Unattended` covered) never returns, so the one run slot stays reserved and every other
component's check/apply is blocked until the user notices and hits Cancel. The design is
one-run-at-a-time by intent and `cancel_run` (taskkill /T /F) does clear it, so this is a
UX-degradation / silent-stall rather than a leak — but there is no backstop if the user isn't
watching (e.g. a tray-triggered `check_all`).

**Evidence:**
```rust
let status = child.wait().await;
// Await the pumps so their final coalesced flush lands BEFORE run-done — no lost tail lines.
for h in handles { let _ = h.await; }
let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
```
**Fix suggestion:**
Optional: wrap `child.wait()` in `tokio::time::timeout` with a generous per-run ceiling (config-driven,
e.g. 10-15 min), killing the tree and emitting an error `run-done` on expiry so the slot always frees
itself. Given scripts are the user's own and Cancel exists, this is a defensive nicety — leave unless
a stuck `check_all` is reported.

---

## [REL-4] Unbounded in-memory line buffering in `pump_stream` for newline-less output — Low
**File:** `src-tauri/src/lib.rs:805-851` (`pump_stream`, `lines.next_line().await`)
**Description:**
`pump_stream` reads the child's stdout/stderr with `tokio::io::Lines::next_line()`, which accumulates
bytes until a newline or EOF. A script that prints a very large chunk with **no** newline (e.g.
`Get-Content` of a minified single-line JSON, or a base64 blob dumped raw) forces `next_line` to buffer
the entire thing in memory before it can emit — a memory spike proportional to the largest line, with
no cap. Scripts here are trusted and normally newline-terminated, so this is a robustness edge, not an
active bug.

**Evidence:**
```rust
let read = match deadline {
    None => lines.next_line().await,
    Some(dl) => match tokio::time::timeout_at(dl, lines.next_line()).await { /* … */ },
};
```
(`next_line` has no length bound; the 32 KiB cap in the *PTY* reader at 12168 does not apply to this
pipe-based path.)
**Fix suggestion:**
If it ever matters, switch to a bounded byte read (like the PTY reader's fixed buffer) and split on
newlines yourself, capping the carry buffer. Not worth doing pre-emptively.

---

## Clean areas (verified, no finding)

- **Lock poisoning:** every `Mutex`/`RwLock` acquisition uses `unwrap_or_else(|e| e.into_inner())`
  (RunState, ForkRuns, SessionState, CONFIG_CACHE, CUR_LANG, agent_status TRACKS, limits FIRED,
  MYPROVIDERS/MCP/DEPLOY_CFG/SSHHOSTS locks). A panic under one lock cannot cascade into a
  poisoned-lock panic elsewhere. (`lib.rs` lines 82,85,261,443,532,4785,6328,10944,12128,… )
- **Config data-loss:** `write_config_file` → `write_json_atomic` is temp+rename (never blanks the
  target) and leaves a `.bak`; `read_config_at` uses `read_json_or_recover`, which restores from
  `.bak` when the live file is corrupt, so a damaged config.json does **not** silently reset to
  defaults. (`lib.rs:224-230, 2003-2059, 10937-10946`)
- **Session-limit race:** `session_spawn` re-checks `SESSION_LIMIT` and inserts under the *same*
  map lock, so two concurrent spawns can't both slip past the ceiling; the loser's child is killed.
  (`lib.rs:12127-12149`)
- **Session-id collision:** `gen_session_id` mixes a process-wide monotonic counter into the nanos
  so same-tick ids differ (R3-04). (`lib.rs:11885-11900`)
- **HTTP timeouts:** every `ureq::Agent` sets `timeout_global` (1.5s status probes, 6-30s provider
  probes, 8s limits, 10-12s engines). No unbounded blocking HTTP. (`lib.rs:2144,5123,5453,5510,5664,
  5780,8661`, `limits.rs:99-102`)
- **Shutdown hygiene:** `RunEvent::Exit` drains and kills every live PTY session; child trees are
  tied to a kill-on-close Job Object at spawn, so nothing outlives the app. (`lib.rs:13141-13146,
  12089-12091`)
- **Secret-file writes:** settings.json / opencode.json / .claude.json skip the `.bak` copy so a
  rotated cleartext token isn't stranded, while temp+rename still guarantees crash-safety.
  (`lib.rs:2027-2039`)
- **PTY reader lifecycle (frontend):** `TerminalPane.svelte` guards a `listen()` that resolves
  post-destroy (`if (destroyed) exitUn()`), drains `unlisteners`, disposes the WebGL addon on GPU
  context loss (falls back to DOM renderer), and `term.dispose()`s on destroy; drag `pointermove`/
  `pointerup` window listeners are removed in their `up` handler. (`TerminalPane.svelte:367-380,
  443-459,575-594`; `DataTable.svelte:123-137`)
- **Untrusted-input parsing:** SSRF host extraction, base64 `ps_encoded_command`
  (`c.get(1).unwrap_or(&0)`), envelope `read_json_or_recover`, agent_status/limits JSON reads all use
  `unwrap_or`/`get`/`ok()` — no panics on malformed script output or hostile URLs.
  (`lib.rs:4569-4599,12653-12677,412-424`; `agent_status.rs:337-365`; `limits.rs:86-138`)
- **Frontend invoke rejections:** `+page.svelte` routes spawn/action calls through
  `.catch(onSpawnErr)`/`.catch(toastErr)` and wraps `readStatus` in try/catch. (`+page.svelte:297-304,
  430,505,538,…`)
