# Code Quality Findings — Castellyn (HEAD 1c02e3d, 2026-07-05)

Axis: Code Quality (real bugs + maintainability debt, not style). Grounding: every
finding below was read with Read/Grep and carries a verbatim quote. Scope-excluded
items (agenthub fallbacks, portable-pty pin, one-run-at-a-time) are honored.

**Headline:** No Critical or High correctness bug was found. The prior goal-audit
(0 Crit/High) still holds against current HEAD. What remains is maintainability debt:
two god-file hotspots with a stringly-typed run contract, one genuine envelope-read
drift, and two minor logic imperfections. Counts: 0 Critical, 0 High, 3 Medium, 2 Low.

---

## [QUAL-1] `updatesAttention` bypasses the shared `countOf` envelope helper — count drift — Medium
**File:** `E:\Scripts\Castellyn\src\lib\attention.ts:9-19` (vs `E:\Scripts\Castellyn\src\lib\envelope.ts:7-13`)
**Description:** `envelope.ts` exists specifically so the envelope-reading rules "can't
drift apart between the toast layer and the cards" (its own header comment). `countOf`
reads `counts.changed`, then falls back to a legacy `changed[]` array length, then
`plugins_changed`. But `updatesAttention` — which computes the sidebar "Updates" badge —
re-implements the read inline and only understands `counts.changed`. A component whose
script still emits the legacy shape (the fallbacks in `countOf` exist precisely because
such writers are anticipated) would be counted in the toast/card (via `countOf`) but
contribute **0** to the sidebar badge. The two surfaces silently disagree. Currently
latent because `Write-StatusJson` always emits `counts`, but the whole point of `countOf`
is defense against writers that don't — and this caller opted out of that defense.
**Evidence:**
```ts
// attention.ts:14-18
for (const c of components) {
  const n = statuses?.[c.id]?.counts?.changed;   // <-- no legacy fallback
  if (typeof n === 'number') changed += n;
}
```
```ts
// envelope.ts:7-13 — the helper this should have used
export function countOf(s: any, key: 'changed' | 'failed'): number {
  if (s?.counts && typeof s.counts[key] === 'number') return s.counts[key] as number;
  const arr = s?.[key];
  if (Array.isArray(arr)) return arr.length;
  const num = s?.[`plugins_${key}`];
  return typeof num === 'number' ? num : 0;
}
```
**Fix suggestion:** `import { countOf } from './envelope'` and use
`countOf(statuses?.[c.id], 'changed')` in the loop, so the badge and the toast read the
envelope through one code path.

---

## [QUAL-2] `+page.svelte` run-lifecycle: stringly-typed component-id contract + a web of module run flags — Medium (god-file hotspot)
**File:** `E:\Scripts\Castellyn\src\routes\+page.svelte:2069-2209` (the `run-done` listener)
**Description:** The single most dangerous state tangle in the 101 KB orchestrator is the
`run-done` handler. It dispatches purely on an **untyped string** `e.payload.component`
that must match, by convention, the id passed at ~7 unrelated backend `spawn_streamed`
sites (`"backup"`, `"profiles"`, `"sync"`, `"engine"`, `"mcp"`, `"schedule"`, plus
native-streamed `"provider"`/`"engine"`). The same handler juggles seven module-level
mutable flags whose reset points are scattered between spawn sites and the listener:
`running`, `pendingRun`, `bulkActive`, `lastRunMode`, `lastForkAction`, `allProgress`,
`pendingUndo`. There is no type binding the emitted id to the reload/label/outcome maps —
a renamed or new stream id fails **silently** (no reload, no toast, raw id leaks through
`opName`'s fallback). This is not a bug today (all ids line up — I verified `run_stack`→
`"engine"`, `run_config_drift`→`"sync"`, `run_provider`→`"provider"`), but it is the
hotspot most likely to grow one on the next tab.
**Evidence:**
```ts
// +page.svelte:2085-2114 — id is an untyped string; every consumer is a manual `if`
const id = e.payload.component;
if (running === id) running = null;
...
if (id === 'backup') await reloadBackup();
if (id === 'profiles') await reloadProfiles();
if (id === 'mcp') await reloadMcp();
if (id === 'sync') { await reloadSync(); await reloadConfigDrift(); await reloadProfiles(); }
if (id === 'engine' || id === 'provider') await reloadProviders();
```
**Fix suggestion:** Define a `StreamComponentId` union type in `ipc.ts`, type both the
`spawn_streamed` id argument (frontend wrappers) and the `run-done` payload against it,
and replace the `if (id === ...)` ladder with one `Record<StreamComponentId, () => void>`
reload map so a missing entry is a compile error, not a silent no-op. No behavior change,
removes the whole class of "new tab forgot to wire its reload."

---

## [QUAL-3] `lib.rs`: 147 commands in one 14.2k-line file share an untyped run/streaming contract — Medium (god-file hotspot)
**File:** `E:\Scripts\Castellyn\src-tauri\src\lib.rs` (147 `#[tauri::command]`, single file)
**Description:** The backend counterpart of QUAL-2. `spawn_streamed(...)` is called with a
free-form `String` component id at many sites, and there are **multiple parallel streaming
domains** with their own id conventions (`run-log`/`run-done`, `fork-log`/`fork-done`
keyed by repo path, plugin-bulk under `"plugin-mgr"`, `run_native_streamed`). The
"component id" is a stringly-typed cross-boundary contract with no single source of truth
on the Rust side either — each spawn site hard-codes its literal. Combined with the file
size (readers must Grep to find anything), this is the backend maintainability hotspot: a
mismatched literal between a spawn site and the frontend's `run-done` switch is invisible
to the compiler on both ends.
**Evidence:**
```rust
// lib.rs — the same untyped id literal, spread across unrelated commands:
1409:    spawn_streamed(app, state, "backup".to_string(), script, args).await
1504:    spawn_streamed(app, state, "profiles".to_string(), script, args).await
1596:    spawn_streamed(app, state, "sync".to_string(), script, args).await   // run_config_drift streams as "sync"
2678:    spawn_streamed(app, state, "engine".to_string(), script, args).await // run_stack streams as "engine"
3489:    run_native_streamed(app, state, "provider".to_string(), move |out, err| {
```
**Fix suggestion:** Not "split the file" generically. Concretely: introduce a
`const`/`enum` for the streaming component ids in Rust (e.g. `mod stream_id { pub const
BACKUP: &str = "backup"; ... }`) shared by all spawn sites, and mirror it as the
`StreamComponentId` union in QUAL-2 so both ends reference one canonical set. Extracting
the ~7 `spawn_streamed` command groups into topic modules (`profiles.rs`, `stack.rs`,
`sessions.rs`) would also shrink the Grep surface without touching behavior.

---

## [QUAL-4] `ForksTab.repoHasConflict` calls `.length` on a `string | string[]` union — Low (drift with sibling normalizer)
**File:** `E:\Scripts\Castellyn\src\lib\components\ForksTab.svelte:64-65`
**Description:** `ForkBranch.conflictFiles` is typed `string | string[] | null` because
PowerShell's `Select-Object -Unique` yields a **scalar string** for a single file and an
array for 2+ (documented at `ipc.ts:39`). `ForkRepoCard.svelte` handles this correctly
with a `confFiles()` normalizer (`Array.isArray(cf) ? cf : cf ? [cf] : []`, line 42-44).
`ForksTab` instead does `conflictFiles?.length` directly: for the common single-file case
this is the **character count of the filename**, not the file count. It happens to work
here because it's only compared `> 0` (any non-empty filename has length ≥ 1), so the
conflict tile filters correctly — but it is a latent copy-that-diverged: reuse this
pattern anywhere that needs the actual count and it silently reports characters.
**Evidence:**
```ts
// ForksTab.svelte:64-65
const repoHasConflict = (r: import('$lib/ipc').ForkRepo) =>
  (r.branches ?? []).some((b) => b.outcome === 'conflict' || (b.conflictFiles?.length ?? 0) > 0);
```
```ts
// ForkRepoCard.svelte:42-44 — the correct normalizer that should be shared
// (array only for 2+), so conflictFiles arrives as string | string[] | null — normalize it.
const confFiles = (cf: string | string[] | null) =>
  Array.isArray(cf) ? cf : cf ? [cf] : [];
```
**Fix suggestion:** Export `confFiles()` from a shared module (or `ipc.ts`) and use
`confFiles(b.conflictFiles).length > 0` in `ForksTab`, killing the second interpretation
of the union type.

---

## [QUAL-5] Sessions auto-continue keys resume solely on the 5-hour reset, ignoring the 7-day window — Low
**File:** `E:\Scripts\Castellyn\src\lib\components\SessionsTab.svelte:363-369`
**Description:** `maybeAutoContinue` decides when a usage-limited pane may be nudged to
resume. It reads **only** `h5Reset` and waits until that timestamp (+ jitter). A pane
that is limited on the **7-day** window (`d7` at 100%) still has an `h5Reset` in the
future for its 5-hour window; when that unrelated 5h reset passes, the code auto-sends the
continue text even though the 7-day quota is still exhausted. The pane immediately
re-hits the limit, re-flags `limited`, and the `autoContinued.delete` path re-arms it — so
it is self-correcting and not data-loss, but it is a wrong-trigger: an unattended
"continue" is sent into a still-limited session, and the user gets a misleading
"auto-continued" toast.
**Evidence:**
```ts
// SessionsTab.svelte:363-369
const resetStr = limitsByProfile[p.profile]?.h5Reset;      // only the 5h window
const reset = resetStr ? Date.parse(resetStr) : NaN;
if (!Number.isFinite(reset)) continue;
if (contJitterMs[p.key] == null) contJitterMs[p.key] = 30_000 + Math.floor(Math.random() * 60_000);
if (Date.now() < reset + contJitterMs[p.key]) continue;
autoContinued.add(p.key);
sessionWrite(id, t('sessions.autoContinueText') + '\r');
```
**Fix suggestion:** Resume no earlier than `max(h5Reset, d7Reset)` when both are known
(the pane isn't free until the binding window resets), or gate the nudge on the
`limited` flag having actually cleared (agent-status clears it on an output flood) rather
than on a clock alone.

---

## Clean areas (audited, no quality defects found)

- **`limits.rs`** (usage-limit monitor): threshold/antispam logic (`take_alert`), scalar
  coercion (`util_of`/`json_scalar_str`), and 401 handling are correct and covered by
  three focused unit tests incl. the De-Morgan re-arm case. No token ever reaches a log
  or event.
- **`agent_status.rs`** (PTY state machine): the `working|blocked|idle|limited|unknown`
  transitions, hook-authority precedence, blocked-resume byte/time backstops, and
  self-heal ceilings are all exercised by 6 unit tests; the tricky `limited`-outranks and
  blocked-byte-burst cases are pinned. Solid.
- **i18n key integrity**: flattened the full `en` dict (1833 keys) and scanned every
  `.svelte`/`.ts` for literal `t('...')` keys — **zero** typo'd/missing static keys (only
  a dynamic `t('nav.' + …)` prefix, a false positive). The template-literal onboarding
  keys (`page.onb_step_${step.id}`) all resolve (`page.ts:347-358`). Parity's blind spot
  (a key absent from all three locales) is empty here.
- **Envelope contract**: `Write-StatusJson` (`ScriptKit.ps1:238-284`) always emits the
  full `{schemaVersion, status, counts:{changed,failed,total}, ...}` shape and guards
  `-Extra` against clobbering reserved keys; `outcome.ts`/`envelope.ts` parse it
  faithfully incl. the `held` branch. No writer↔reader field drift.
- **BOM / JSON reads**: centralized through `read_json_opt` / `read_json_or_recover`
  (`lib.rs:387-429`, BOM-tolerant + `.bak` recovery). The few inline
  `trim_start_matches('\u{feff}')` sites read raw text (not parse) and don't diverge.
- **`ProfilesMatrix` plugin tri-state** (`pluginOn`/`pluginDirty`, lines 148-160):
  `on`/`off`/`unset` dirty detection is correct, including the `unset`→explicit-override
  case (`return true`).
- **`onbRunAll` ordering** (`+page.svelte:827-833`): the `order` array ids match
  `onboarding_scan`'s emitted ids exactly; `backup_tab` steps are correctly filtered out.
- **`opName` mapping** (`running.svelte.ts:9-21`): every operational stream id that
  reaches the toast path is mapped; verified no live unmapped id leaks its raw string.

---

## Summary
- **Critical:** 0
- **High:** 0
- **Medium:** 3 (QUAL-1 envelope-read drift; QUAL-2/QUAL-3 the two god-file
  stringly-typed run-contract hotspots)
- **Low:** 2 (QUAL-4 fork union-`.length`; QUAL-5 5h-only auto-continue)

The two Medium god-file findings are one root cause (an untyped `component-id` string
crossing the Rust↔TS boundary at many sites) — fixing them together (a shared id
enum/union) is the highest-leverage quality investment and removes a whole class of
silent "new tab didn't wire up" regressions.
