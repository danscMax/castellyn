# Diagnosis — Schedule tab + Home false status

Machine: MAIN. Investigated 2026-07-05 against the LIVE Task Scheduler state, the
PowerShell producer (`Schedule-Hub.ps1`), the Rust bridge (`lib.rs`), and both consumers
(`HomeTab.svelte`, `ScheduleTab.svelte`). Every claim below is backed by a verbatim quote.

## Ground truth (verified, not from the UI)

Real Task Scheduler state (queried via `Get-ScheduledTask` / `Get-ScheduledTaskInfo`):

| Task | Exists | State | LastTaskResult | Meaning |
|------|--------|-------|----------------|---------|
| ClaudeProfiles-Backup | **MISSING** | — | — | genuinely not registered |
| ClaudeProfiles-Integrity | yes | Ready | 0 (0x0) | ran OK today |
| ClaudeMaintenanceHub-UpdateAll | yes | Ready | 267011 (0x41303) | **SCHED_S_TASK_HAS_NOT_RUN — never run yet** |
| ClaudeMaintenanceHub-Forks | yes | Ready | 267011 (0x41303) | **never run yet** |
| ClaudeProfiles-InfraHealth | yes | Ready | 0 (0x0) | ran OK today |

The stored envelope `E:\Scripts\!Настройки и MCP\ClaudeProfiles\schedules.last.json` matches
this reality exactly (`backup.exists=false`; `updates`/`forks` → `status:"never-run", ok:false,
lastResult:267011`). So the data layer is accurate; the bug is in **interpretation**.

`0x41303` is not an error. It is the documented Task Scheduler sentinel for "the task has
never run". Both UpdateAll and Forks are scheduled for tomorrow 11:00/11:30 and simply have not
fired yet.

---

## Symptom 2 (HIGHEST priority) — Home says "ЗАДАЧИ: 2 с ошибкой", Schedule tab says all green

This is the false-status bug and its single root cause is proven.

### Root cause (probability 95%): `ok` conflates "never run" with "failed", and Home keys off `ok`

Producer — `E:\Scripts\!Настройки и MCP\ClaudeProfiles\Schedule-Hub.ps1:71`:

```powershell
$ok = ($lastResult -eq 0 -or $lastResult -eq 267009)
```

`267011` (never-run) is **not** in that set, so a never-run task gets `ok = $false`. Note the
same file *already* decodes 267011 correctly one function up (`Schedule-Hub.ps1:46`
`267011  { 'never-run' }`) — the `status` string is right, only the `ok` boolean is wrong.

Consumer — `E:\Scripts\Castellyn\src\lib\components\HomeTab.svelte:142`:

```js
const failing = schedules.tasks.filter((x) => x.ok === false).length;
```

With UpdateAll + Forks both `ok:false`, `failing = 2` → `HomeTab.svelte:151`
`value: failing > 0 ? t('page.home_tasksFailing', { n: failing }) : …` →
`ru/page.ts:34 home_tasksFailing: '{n} с ошибкой'` → **"2 с ошибкой"**, and
`HomeTab.svelte:152 level: failing > 0 ? 'bad'` → the red chip. `n=2` also lines up with the
"проблем: 2" roll-up.

The other consumer never looks at `ok` at all. `ScheduleTab.svelte:56-62` decides its badge
purely from `exists`/`enabled`:

```svelte
{#if !task.exists}      … statusNotCreatedBadge
{:else if task.enabled} … statusEnabledBadge   ← "включено", green
{:else}                 … statusDisabledBadge
```

So an existing+enabled task is always green there regardless of `ok`/`status`. **That is the
divergence**: Home trusts `ok`, ScheduleTab ignores it, and `ok` itself is wrong for never-run.

### Root-cause FIX

Fix at the source so every consumer benefits (DRY). In `Schedule-Hub.ps1:71`, `ok` should mean
"not a real failure", i.e. only a genuine `failed(0x…)` is not-ok:

```powershell
# never-run / ready / running are benign — only a decoded failure is not-ok
$ok = ($status -notlike 'failed*')
```

(`$status` is already computed on the line above via `Get-TaskResultStatus`.) After this,
UpdateAll/Forks become `ok:true`, `failing` drops to 0, and Home shows the honest state ("all
OK" or, if you want never-run surfaced, a neutral note — see below). ScheduleTab is unaffected
because it never read `ok`.

- Trade-off A (recommended, smallest): the one-line `ok` fix above. A task that has genuinely
  failed (non-zero, non-sentinel HRESULT) still counts as failing and still turns Home red —
  correct. Downside: "never run yet" is no longer distinguished on Home; it just isn't an error
  (which is right).
- Trade-off B (optional polish): if a never-run task *should* be visible on Home, add a 4th
  neutral bucket in `HomeTab.svelte` (`neverRun = tasks.filter(x => x.status === 'never-run')`)
  shown as `warn`/info text, never `bad`. More code, more i18n keys; only if the owner wants it.
- Do **not** "fix" this by making ScheduleTab also colour on `ok` — that would spread the wrong
  semantics to a second surface instead of correcting the source.

### Secondary contributor (probability 5%)
None material. The chip could theoretically diverge if `read_schedules` served a stale JSON, but
the stored JSON matches live reality here, so staleness is not in play for this symptom.

---

## Symptom 1 — "Бэкап конфигов" shows "не создано" and clicking «Создать расписание» doesn't stick

### What "не создано" means
`ScheduleTab.svelte:56 {#if !task.exists}` → `schedule.statusNotCreatedBadge`. It renders when
`task.exists === false`, which comes straight from `Schedule-Hub.ps1:54 $exists = [bool]$t` where
`$t = Get-ScheduledTask -TaskName 'ClaudeProfiles-Backup'`. **Verified true**: that task is
genuinely absent from Task Scheduler. So the badge is honest — unlike symptom 2.

### The create logic itself is NOT broken (proven)
I reproduced the exact owner flow by invoking the real script
`Schedule-Hub.ps1 -Action create -Id backup -Time 10:00`. Result: `Создано:
ClaudeProfiles-Backup в 10:00`, child `exit 0`, and `Get-ScheduledTask` afterwards returned the
task in state `Ready`. I then deleted it to restore the prior MISSING state. So registration,
the Cyrillic script path (`Backup-ClaudeSetup.ps1`, confirmed present), the trigger/settings —
all work. There is no permanent, backup-specific code defect blocking creation.

That means the owner's "clicked create, still не создано" was produced by one of the paths below,
each of which is a real design defect that makes such a failure **invisible and unrecoverable**.

### Root cause candidates

**(A) probability 55% — false success: the PS create swallows its own error, so a failed
registration still returns exit 0 and the UI toasts "done".**
`Schedule-Hub.ps1:100-117` wraps `Register-ScheduledTask` in a `try`, and the surrounding
`catch` at `:119-120` only writes a red line and lets the script fall through to
`Write-SchedulesJson` (`:123`) and exit 0:

```powershell
} catch {
    Write-Host "Ошибка: $($_.Exception.Message)" -ForegroundColor Red
}
```

The Rust bridge returns that exit code unchanged (`run_schedule` →
`spawn_streamed(..., stream_id::SCHEDULE, …)`, `lib.rs:6534`), and the UI treats exit 0 as
success. On the follow-up `reloadSchedules` (`+page.svelte:2208 if (id === STREAM_IDS.SCHEDULE)
await reloadSchedules();`) the card correctly flips back to "не создано" — producing exactly
"I created it but it stayed не создано, with no error". This is the classic exit-0-but-did-nothing
false success. It is also why I couldn't see the underlying reason: on the owner's machine
`Register-ScheduledTask` presumably failed once (transient elevation/AV/policy at setup time — the
other 4 were registered in the same batch at default times, backup was the one that missed), and
the swallow hid it.

FIX: make the catch fail loudly — re-throw / `exit 1` after logging, so a failed create surfaces
a real error toast instead of a success one. Root-cause, one block:
```powershell
} catch {
    Write-Host "Ошибка: $($_.Exception.Message)" -ForegroundColor Red
    Write-SchedulesJson   # keep the JSON honest
    exit 1                # <-- stop reporting success on a failed op
}
```
Trade-off: enable/disable/run/delete share this catch, so they too will now report failure
honestly (desirable). Verify the create path additionally asserts the task exists post-register
before exit 0 (belt-and-suspenders against a Register that "succeeds" but registers nothing).

**(B) probability 30% — the click was silently dropped because something else held the run lock.**
`+page.svelte:1604-1605`:
```js
function startSchedule(action, id, time) {
    if (running) return;   // <-- no toast, no feedback; the click is a no-op
```
If any run is in flight (including the schedule tab's own lazy query, an auto-refresh, or a
background op) when the owner clicks, the create is dropped with zero feedback. Repeated across a
session this reads as "the button doesn't work". FIX: mirror the pattern already used at
`+page.svelte:1976` (`pushToast({ kind:'info', title: t('page.busy_running', …) })`) so a blocked
click tells the user why instead of vanishing.

**(C) probability 15% — historical/transient only.** The other 4 exist; backup missed at
first-run and was never successfully retried. No standing code bug; folded into (A)/(B) because
those are what make it un-diagnosable and un-retryable for the user.

Highest-probability actionable root cause for symptom 1: **(A) the swallowed-error / false-success
in the create path** — it both hides why creation failed and lets the UI claim success.

---

## Symptom 3 — DE-HARDCODE: the task set, names, scripts, args and times are baked in

Everything schedulable is a fixed 5-row table in `Schedule-Hub.ps1:30-36`:

```powershell
$defs = @(
  @{ Id='backup';   TN='ClaudeProfiles-Backup';   … Script=(Join-Path $scriptDir 'Backup-ClaudeSetup.ps1'); Args='-Quiet'; Time='10:00' },
  @{ Id='integrity';TN='ClaudeProfiles-Integrity'; … }
  @{ Id='updates';  TN='ClaudeMaintenanceHub-UpdateAll'; … Script=(Join-Path $scriptDir 'Update-All.ps1'); Args='-Check'; Time='11:00' },
  @{ Id='forks';    TN='ClaudeMaintenanceHub-Forks';     … Script=(Join-Path $scriptsRoot 'fork-updater\update-forks.ps1'); Args='-Unattended'; Time='11:30' },
  @{ Id='infra-health'; TN='ClaudeProfiles-InfraHealth'; … Script=(Join-Path $env:USERPROFILE '.claude\hooks\infra_health.py'); Args=''; Time='09:30' }
)
```

Problems for a fresh downloader:
- **Fixed identities.** Task names, labels, scripts, args, default times are all literal. A user
  with no `fork-updater`, a different profile layout, or no `infra_health.py` gets tasks pointing
  at scripts that don't exist (Register won't complain — see symptom 1 — they'll just fail at run).
- **Path divergence already exists in-repo.** The Forks task points at
  `$scriptsRoot\fork-updater\update-forks.ps1`, while `maintenance-manifest.json:28` points the
  *same* forks component at `Castellyn\tools\fork-updater\update-forks.ps1`. Two hardcoded truths
  for one action — a canonical single source is missing.
- **No user control.** Can't add a custom maintenance action, can't remove ones they don't use;
  times are editable per-task in the UI (`ScheduleTab` `bind:value={times[task.id]}`) but the *set*
  of tasks is frozen.

### Recommended de-hardcode (root, DRY)
The app already has a declarative registry read from disk at runtime:
`manifest/maintenance-manifest.json` (`id`, `scriptRel`, `checkArgs`, `name`, `lastJsonRel`) —
the same shape the maintenance tabs consume. Fold scheduling into that model rather than a second
hardcoded table:

1. Add optional `schedulable: true` + `defaultTime` (+ optional `taskName`) fields to manifest
   components. The 3 that are components (all/updates, forks, plus any others) come for free;
   `scriptRel` is already the canonical path (kills the Forks path divergence above).
2. For the ClaudeProfiles-only actions that aren't maintenance components (backup, integrity,
   infra-health), either promote them into the manifest as `group:"ClaudeProfiles"` entries, or
   keep a small user-editable `schedules.json` (id/label/scriptRel/args/defaultTime) under
   `%APPDATA%\castellyn`. Either way the source becomes data, not a `$defs` literal.
3. `Schedule-Hub.ps1` builds `$defs` by reading that registry (resolve `scriptRel` against
   `$env:SCRIPTS_ROOT`) instead of hardcoding. Task name defaults to `Castellyn-<id>` when not
   overridden, so a fresh machine self-names without collisions.
4. UI: the existing per-task time input stays; add "enable which actions" (a component with no
   task yet = "create") — which ScheduleTab already renders via the `!task.exists → create`
   branch. Minimal UI change: the tab just iterates whatever the registry yields.

Trade-off: promoting the ClaudeProfiles scripts into the manifest is the cleanest single-source
option but touches the manifest schema (a `schemaVersion` bump + the embedded fallback copy in
`lib.rs`). The lighter option (2b, a `schedules.json` the user can edit) ships faster and keeps
the maintenance manifest focused on updates; recommend 2b for v1, manifest-merge later. Do **not**
build a full "custom arbitrary command" scheduler UI now — YAGNI; data-driven registry + the
existing create/time UI covers the real need.

---

## One-line summary of fixes
1. `Schedule-Hub.ps1:71` — `$ok = ($status -notlike 'failed*')` (kills "2 с ошибкой" false status). **Do this first.**
2. `Schedule-Hub.ps1:119` — re-throw/`exit 1` on catch so a failed create stops reporting success.
3. `+page.svelte:1605` — toast when a schedule click is dropped because busy.
4. De-hardcode the `$defs` table onto the on-disk registry (manifest and/or a user `schedules.json`).
