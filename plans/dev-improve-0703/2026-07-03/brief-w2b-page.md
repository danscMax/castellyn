# Brief: Wave 2B — page/console/backend cluster (items 7, 14)

Repo `E:\Scripts\Castellyn`. Comments English. Do NOT commit. OWNED files ONLY:
`src/routes/+page.svelte`, `src/lib/components/Console.svelte`, `src-tauri/src/lib.rs`.
i18n locale files are READ-ONLY (keys pre-added). Do NOT modify existing tests (you MAY add a rust test). No new dependency.

## Read first
- `src/routes/+page.svelte`: the ~8 sites doing `log = [...log, X].slice(-MAX_LOG)` (grep `\[...log,`); the `run-log` listener ~1702; `MAX_LOG` const; command-palette actions ~1560-1600 with `run:` handlers (item 14); the run-lock (`running` state) and `opName(running)`.
- `src/lib/components/Console.svelte`: the autoscroll `$effect` at ~62-65 (item 7 rAF).
- `src-tauri/src/lib.rs`: `spawn_streamed` + where it emits `run-log` LogLine events (grep `run-log`, `LogLine`, `emit`); this is the batch point (item 7). Keep the `{component, stream, line}` event shape.

## Contract (FROZEN)
- run-log event shape STAYS `{ component: string, stream: string, line: string }` — do NOT change it to an array (that would touch ipc.ts, out of scope). Batching coalesces multiple lines into ONE event by joining with `\n` in the `line` field.
- i18n key to USE (already added): `page.busy_running` with `{name}`.
- FIFO ordering of log lines MUST be preserved end-to-end.

## Tasks

### 7 — log append perf (3 parts)
1. **Frontend appendLog helper** (`+page.svelte`): add one helper `function appendLog(line: string) { log.push(line); if (log.length > MAX_LOG) log.splice(0, log.length - MAX_LOG); }` (Svelte 5 `$state` array — push/splice mutate reactively; verify `log` is declared with `$state`). Replace the ~8 `log = [...log, X].slice(-MAX_LOG)` sites with `appendLog(X)`. For a batched run-log `line` containing `\n`, split and append each: `for (const ln of p.line.split('\n')) appendLog(...)` — preserving order and the `⚠ ` err prefix per line.
2. **Backend batch** (`lib.rs` `spawn_streamed`): coalesce rapid stdout/stderr lines and flush as one `run-log` event at ~30ms cadence (or when the buffer hits a sane size, e.g. 64 lines), joining lines with `\n`. Preserve FIFO: buffer is an ordered Vec; stdout and stderr must NOT interleave out of order within a flush — keep per-stream buffers OR tag each line and flush in arrival order (simplest correct: a single ordered buffer of (stream,line), flushed by grouping consecutive same-stream runs into events; if that's complex, flush one event per stream per tick with the lines in order). Do NOT drop lines. Ensure the final lines flush on process exit (flush before emitting run-done). Add a rust unit test for the batching/ordering helper (extract the buffer-coalesce into a testable fn).
3. **rAF autoscroll** (`Console.svelte`): wrap the `logEl.scrollTop = logEl.scrollHeight` in `requestAnimationFrame(...)` so rapid appends don't thrash layout; guard the ref still exists inside the callback.

If the backend batch (part 2) proves risky to keep FIFO/interleave-correct, implement parts 1+3 fully and for part 2 do the MINIMAL safe version (per-tick flush per stream, lines in order) — flag it in your report rather than shipping an ordering bug.

### 14 — palette busy toast instead of silent no-op
- The command palette `run:` handlers call `startRun`/`startForks`/etc. which self-guard on the run lock and silently no-op while busy. Add a small wrapper `runOrToast(fn)` (or inline guard) used by the run-starting palette actions: if a run is in progress (`running` is non-null), `pushToast({kind:'info', title: t('page.busy_running', {name: opName(running)})})` and return; else call `fn()`. Apply to the high-frequency run verbs (checkall, forks, backup, per-component check/apply, stack start/stop) — NOT to pure UI toggles (theme/density/open-log/hotkeys) which are always safe. Do not change the underlying startRun guards (defense in depth stays).

## Verify (all must pass)
```
npm run check      # 0/0
npm test           # vitest green
cd src-tauri && C:\Users\User\.cargo\bin\cargo.exe test   # green + new batch test
```
Live sanity (if you can, else note skipped): during a run, the console lines stay in order.

## Report back (first completion — I will NOT re-query you)
Per item: changed file:line ranges, the batch-helper test name, gate outputs (real summary lines), and — importantly — whether part-2 backend batch is full or the minimal fallback, with why.
