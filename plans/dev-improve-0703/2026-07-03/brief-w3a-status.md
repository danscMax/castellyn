# Brief: Wave 3A — agent_status.rs (items 6, 8)

Repo `E:\Scripts\Castellyn`. OWNED file: `src-tauri/src/agent_status.rs` ONLY. Comments English. Do NOT commit.
Do NOT edit `src-tauri/src/lib.rs` — the Lead owns the single `on_output` caller there and will update it to the new signature (contract below). Do NOT modify existing tests (ADD tests). No new dependency (stdlib atomics/fs only).

## Read first
- Whole file, especially: `struct Track` (~37-49), `on_output` (~87-95), `compute` (~186-225), `apply_hook_report` (~229-248), the poll loop (~269-316) incl. the hook-file read (~282-291), constants (~21-28).

## Contract (FROZEN — coordinate with Lead)
- `on_output` signature BECOMES: `pub fn on_output(id: &str, bytes: usize)`. Keep the function name; only add the `bytes` param.
- **The Lead has ALREADY finished all lib.rs edits for this wave and is NOT touching lib.rs anymore.** You own the SINGLE caller line: update `agent_status::on_output(&id_r)` at lib.rs ~9411 → `agent_status::on_output(&id_r, n)` (the chunk length `n` is in scope: `Ok(n) => { let bytes = &buf[..n]; ... }`). Touch ONLY that one line in lib.rs — nothing else in that file. This is the ONLY lib.rs edit you make.
- `StatusEvent` already has `spawned_at` (Wave 2) — leave it.

## Tasks

### 6 — `blocked` must not clear on a mere prompt repaint (reliability)
Today `compute` flips `blocked → working` as soon as `t.last_output > t.hook_ts + BLOCKED_RESUME_MS` — i.e. ANY PTY output after 1.5s (a resize repaint of the prompt box, a spinner tick) ends the blocked state even though the user hasn't answered.
Fix — hook-first, with a byte-burst fallback (approved decision):
- Primary: `blocked` clears when a NEW hook event supersedes it — `apply_hook_report` already updates `hook_state` (e.g. UserPromptSubmit → "working", Stop → "idle"). So if the hook authority moves off "blocked", compute naturally leaves blocked. Keep that.
- Fallback (no hook fires when the user answers a permission prompt in-terminal): clear `blocked` only on a substantial **byte burst** since the block, not on the first trickle. Track bytes emitted since the current blocked state began; flip to "working" only once that exceeds a threshold (define `BLOCKED_RESUME_BYTES`, e.g. 512 or 1024 — a repaint of a prompt box is small; a resumed agent turn floods far more). Keep a time backstop so it can't stick forever if bytes never arrive: if `now - hook_ts` exceeds a generous ceiling (e.g. `BLOCKED_RESUME_MS * some factor`, or a new `BLOCKED_STUCK_MS ~ 20s`) AND there's been ANY output, allow the flip — so an Esc-answer (which produces little output) still recovers.
- Implementation: add a byte counter to `Track` (see item 8 — make it `AtomicU64`), reset/snapshot it when `hook_state` becomes "blocked" (in `apply_hook_report`), and read the delta in `compute`. `compute` takes `&Track` so reading an atomic is fine.
- Unit test: simulate a Track in blocked with a small post-block byte delta + output → stays blocked; with a byte delta over threshold → working; the time-backstop path → working. Model the existing tests' style (they construct a Track and call compute directly).

### 8 — mtime gate for hook-file reads + atomic last_output (perf)
- **mtime gate (primary win):** add a per-Track field for the last-seen hook-file mtime (e.g. `hook_mtime: u64`). In the poll loop, before `read_to_string({id}.json)`, `fs::metadata(path).modified()` → compare to the stored mtime; if unchanged, SKIP the read+parse entirely (the JSON hasn't changed). On a changed mtime, read+parse+`apply_hook_report` and store the new mtime. This removes a JSON parse per claude pane per 500ms poll when nothing changed. Keep correctness: a missing file (never written yet) reads as before.
- **atomic last_output:** make `Track.last_output` an `AtomicU64` (and the item-6 byte counter an `AtomicU64`). `on_output(id, bytes)` then updates them via a SHARED borrow (`map.get(id)` + `last_output.store(now, Relaxed)` + `bytes_counter.fetch_add(bytes, Relaxed)`) instead of `get_mut`. NOTE: this removes the `&mut` exclusivity but the `TRACKS.lock()` to find the entry stays — a fully lock-free per-session handoff would require passing an Arc into the lib.rs PTY reader and is OUT OF SCOPE for this wave; the mtime gate is the primary perf win. Say so in your report. Everywhere else that reads `last_output` (compute, the STARTUP_GRACE/silent math) switches to `.load(Relaxed)`.
- Keep `now_ms()` as the clock. Ensure `Track` construction in `on_spawn` and the tests initializes the new atomic fields.

## Verify
```
cd src-tauri && C:\Users\User\.cargo\bin\cargo.exe test
```
Must be GREEN + your new tests, zero new warnings. Because you update the single lib.rs caller (see contract) to the 2-arg signature, the whole crate compiles and `cargo test` runs normally. If it doesn't compile, the mismatch is between your `on_output` signature and that one caller line — reconcile them.

## Report back (ONE completion — no re-query)
Per item: changed line ranges, new field names, new consts, new test names, the cargo test summary line, whether you touched the lib.rs caller line, and the item-8 lock-free-scope note.
