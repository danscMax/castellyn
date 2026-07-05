# Forks tab — diagnosis (2026-07-05)

Cluster: **"Форки и репозитории"** — `src/lib/components/ForksTab.svelte`, `src/lib/components/ForkRepoCard.svelte`,
Rust fork commands in `src-tauri/src/lib.rs`, and the PowerShell fork-sync layer
(`tools/fork-updater/update-forks.ps1` + `tools/fork-updater/ForkSync.psm1` + `tools/fork-updater/repos.json`).

How discovery actually runs (verified):
- Manifest `forks` component → `Castellyn\tools\fork-updater\update-forks.ps1`
  (`manifest/maintenance-manifest.json:28`).
- `update-forks.ps1:53` calls `Invoke-ForkSync -Root $PSScriptRoot`, so config is read from
  `E:\Scripts\Castellyn\tools\fork-updater\repos.json` (`ForkSync.psm1:460`, `Get-ForkSyncConfig`).
- `Find-ManagedRepos` (`ForkSync.psm1:193`) = every `.git` dir under each `roots/*` entry **plus** each
  explicit `paths` entry. `ownPaths` are added as own (non-fork) repos.

---

## Issue 1 — Card layout: truncated names, crowded chips, ~4 columns, wasted space

### Symptom
Fork names clip to "FreeQw…"; status chips crowd the header; only ~4 cards per row; short/collapsed cards
leave large empty gaps below them.

### Root causes (probability)

**A. Grid min-column too wide → few columns (95%).** `src/app.css:420`

```css
.card-grid {
  display: grid;
  gap: var(--sw-space-4);
  grid-template-columns: repeat(auto-fill, minmax(330px, 1fr));
  align-items: start;
}
```

`minmax(330px, 1fr)` forces a 330px floor. On a ~1150px content pane (window minus sidebar) with a 1rem gap
that is only 3 columns; each column then stretches (`1fr`) to ~380px, so cards are simultaneously **few** and
**wider than their content needs**. This is the "only 4 per row + lots of empty horizontal space" complaint.

**B. Name truncates because the header row shares width with two badges (90%).** `ForkRepoCard.svelte:248-265`

The name lives in `<h3 class="truncate">` inside a `min-w-0` flex button that also holds the caret **and** the
`fork`/`own` badge (`:256`), and the row's far end holds the `health` badge (`:264`, `shrink-0`). With both
badges present the name column is squeezed and `truncate` clips it to "FreeQw…". Widening the card (fewer
columns) would relieve this but fights the density goal — the name should not have to compete with badges for
the same line.

**C. `align-items: start` + grid rows = masonry gaps (85%).** `src/app.css:424`

Grid rows are as tall as their tallest member. One expanded card (open details + branch list, `:278-333`) makes
its whole row tall, so a collapsed neighbour shows a big empty gap beneath it. `align-items:start` stops cards
from stretching vertically but does nothing about the row-height coupling. This is inherent to CSS grid; true
density needs either a masonry/columns layout or a compact list mode.

### Root-cause fix (trade-offs)

The lazy, highest-leverage fix is three small CSS/markup changes, not a rewrite:

1. **Drop the min column to ~260-280px** in `.card-grid` (`app.css:423`): `minmax(270px, 1fr)`. Immediately
   yields 4-5 columns on the same pane and shrinks the wasted 1fr stretch.
   Trade-off: a card with many branches gets narrower; branch rows already wrap (`flex-wrap`, `:306`) so this is
   safe. This is the single most effective change.
2. **Give the name its own line / stop it competing with badges** (`ForkRepoCard.svelte:253-257`): move the
   `fork`/`own` chip to the subtitle line (`:258`) or allow the title to wrap (`break-words` instead of
   `truncate`, with `title={repo.Name}` for the tooltip). Full names show without widening the card.
   Trade-off: wrapping adds a line for long names; truncate keeps a fixed height. A 2-line clamp is the middle
   ground.
3. **For the "wasted vertical space" specifically**, add a **compact/table view toggle** for many repos (reuse
   the existing `DataTable` already imported at `ForksTab.svelte:8` and used for the GitHub-only list) OR switch
   the card container to CSS columns (masonry). Trade-off: columns/masonry reorders cards top-to-bottom-per-column
   (harder to scan by name); a DataTable list is denser and sortable but loses the rich per-card recommendation
   UI. Recommendation: keep cards, do (1)+(2) now; offer a list toggle only if the owner still wants more density.

---

## Issue 2 — "not-a-fork" badge (bifrost, ccs) with "— · 0 веток"

### Symptom
Some repos render as dead cards: `not-a-fork` muted badge, subtitle "— · 0 веток", no action buttons.

### Root cause (verified, 95%) — `ForkSync.psm1:483-485`

```powershell
if (-not $IsOwn -and -not $roles.IsFork -and $roles.Guessed -eq $false) {
    return [pscustomobject]@{ Name = $name; Path = $RepoPath; Skipped = 'not-a-fork' }
}
```

`not-a-fork` fires when **all three** hold:
- the repo is **not** in `ownPaths` (`-not $IsOwn`), and
- `gh repo view` returned **no `parent`** → `IsFork = [bool]$parent = $false` (`ForkSync.psm1:186`), and
- `gh` **succeeded** → `Guessed -eq $false` (`:136`).

So bifrost/ccs are git repos sitting under a `roots` entry (`E:\Scripts\External`, `repos.json:3`) whose GitHub
`origin` has **no parent** — i.e. they are **not GitHub forks** (cloned straight from the source, or the fork
link isn't recorded on GitHub), and they are **not declared** in `ownPaths` (`repos.json:5` lists only DeskRift
and Slidio).

**This is correct detection, not a mislabel.** They are genuinely not forks. Two consequences worth flagging:

1. **UX bug, not a classification bug.** Because `roots` is a catch-all directory scan (`Find-ManagedRepos:198`),
   *any* non-fork clone dropped into `External` becomes a dead "not-a-fork" card (subtitle "— · 0 веток" because
   `Skipped` repos carry no `defaultBranch`/branches; rendered at `ForkRepoCard.svelte:258-261`, actions hidden by
   `{#if !repo.Skipped}` `:335`). Nothing is *wrong* — they just clutter the grid with un-actionable cards.
2. **The label is gh-dependent (inconsistent).** If `gh` is unavailable, `Guessed` stays `$true` (`:124`), so the
   `not-a-fork` guard does **not** fire, and the same repo is instead processed as a heuristic "guessed" fork
   (upstream falls back to `origin`, `:170`) and gets a full — but meaningless — card. So the same folder flips
   between "dead not-a-fork card" and "guessed fork card" purely on `gh` availability.

### Fix
This is really the same problem as Issue 4: give the user a way to **exclude** or **reclassify** these repos
(move to `ownPaths`, or drop the `External` root and list only real fork paths). No detection code needs to
change. Cheap presentational mitigation: **hide `Skipped='not-a-fork'` cards behind a collapsed "Not forks (N)"
group** (mirror the existing GitHub-only `<section>` collapse at `ForksTab.svelte:287`) so they stop crowding the
actionable cards.

---

## Issue 3 — "Синхронизировать wip-local" greyed while the card recommends it

### Symptom
FreeQw… card shows *"Рекомендуется: синхронизировать wip-local (отстаёт на 5)"* but the primary button is
disabled/greyed.

### Root cause (verified, 90%) — the button gates on the GLOBAL run slot; the recommendation does not

The recommendation text and the button use **different conditions**:

- Recommendation shown when `canSyncWip` (`ForkRepoCard.svelte:177`), which is
  `wipBehind > 0 && !repo.dirty && safeTree` (`:147`) — **no run-state gate**.
- Button `disabled={rec.disabled}` (`:344`); for syncwip `rec.disabled = anyRunning || busy` (`:178`).
  `anyRunning = !!running` (`ForksTab.svelte:41`), and `running` is the app-wide **single-slot component id**
  passed straight through (`+page.svelte:2482`).

So the button greys out whenever **any** run holds the global slot — a whole-stack fork *check*
(`running === 'forks'`), **or an entirely unrelated component** (rtk / plugins / cargo update). During the
owner's screenshot the global slot was almost certainly held (a fork "Проверить" refresh — the card also shows
the `.fork-refreshing` shimmer, `ForkRepoCard.svelte:430`, which sets `pointer-events:none`).

**Why this is a genuine over-gating bug, not just transient:** per-repo fork actions do **not** use the global
slot. `onForkAction` with a `path` routes to `startForkRepo → runForkRepo` (`+page.svelte:526,533,543`), which
hits the Rust `run_fork_repo` (`lib.rs:1101`). That command only rejects when a **global fork sweep** is active or
**the same repo** is already busy (`ForkRepoSlot::reserve`, `lib.rs:1125`). It does **not** care whether rtk or
plugins is running. So the frontend disabling the wip-sync button on `anyRunning` (any global run) is stricter
than the backend requires — the action would run fine.

Secondary smell (low): the recommended-button branch for `syncwip`/`delwip` has **no `isOwn` guard**
(`:175-178`), while the same actions in the "⋯ more" dropdown are wrapped in `repo.isOwn ? [] : [...]`
(`:363-374`). So an own repo can surface a wip-sync *recommendation button* it can't reach from the dropdown.

### Root-cause fix (trade-offs)
Gate per-repo action buttons on what the backend actually blocks on, not the global slot:
`disabled = busy || running === 'forks'` (this repo running, or a global fork sweep in flight) instead of
`anyRunning || busy`. Apply to the `rec.disabled` computations (`:171-185`) and the dropdown items
(`:367-373`). Effect: an unrelated rtk/plugins run no longer greys every fork action; the recommendation and its
button finally agree. Trade-off: none functionally — the backend already enforces the real mutual-exclusion; this
just stops the UI over-disabling. (While `running === 'forks'` the card is `.fork-refreshing`/non-interactive
anyway, so the button state there is moot.) Also add the missing `!repo.isOwn` guard to the syncwip/delwip
recommendation branch for consistency.

---

## Issue 4 — De-hardcode fork discovery (add/remove forks, set roots, auto-discover)

### Current state (verified)
Discovery is **already config-driven**, just not surfaced in the UI:

- `tools/fork-updater/repos.json` (`Get-ForkSyncConfig`, `ForkSync.psm1:458-475`) supports:
  - `roots` — parent dirs; every `.git` subdir is scanned (default `E:\Scripts\External`).
  - `paths` — explicit individual fork dirs (default `C:\Users\User\rtk-windows-hook-pr\rtk`).
  - `ownPaths` — your own (non-fork) repos, reported for PR/CI + merged-branch cleanup only.
  - `fetchTimeoutSec`, `ghTimeoutSec`.
- Config is read from the **script's own folder** (`-Root $PSScriptRoot`, `update-forks.ps1:53`), i.e. a vendored
  `tools/` file. There is **no Castellyn IPC/UI** to edit it — the user must hand-edit JSON. There is **no
  auto-discovery** across the PC.

So the ask is **surface + extend the existing config**, not build discovery from scratch.

### Proposed design (options + trade-offs)

The three approaches the owner named, ranked lazy-first:

**Option A — Expose the existing `roots`/`paths`/`ownPaths` config in the UI (recommended).**
Add a small settings surface on the Forks tab (or in Settings) that reads/writes the roots/paths/ownPaths lists,
plus "add this folder" / "remove". Back it with two new Rust commands (`read_fork_config` / `write_fork_config`)
that read/write the JSON.
- Trade-off: **store the JSON in `%APPDATA%\castellyn`, not `tools/fork-updater/repos.json`.** The `tools/` copy
  is vendored/auto-synced and can be clobbered on script update; user config must live in the durable config dir
  (same pattern as `config.json`, `lib.rs:265`). Requires passing the config path to the script (a
  `-ConfigPath`/env param) instead of the current `$PSScriptRoot`-relative read.
- Cost: ~1 IPC pair + 1 small component. Reuses everything else. Solves add/remove + set-root + own-vs-fork in one
  move, and gives Issue 2's dead cards a home (reclassify to `ownPaths` or drop the root).

**Option B — Auto-discover forks across configured roots (opt-in scan).**
Add a "Scan folder…" action: pick a parent dir, enumerate `.git` subdirs (the scan logic already exists in
`Find-ManagedRepos`), resolve each via `gh` (`Resolve-RepoRoles`), and present a checklist of what's a real fork
vs own vs not-a-fork so the user chooses what to track.
- Trade-off: a targeted scan of one chosen root is cheap; each repo still costs a `gh repo view` + `git fetch`,
  so a root with 30 repos is slow. Bound it to user-chosen roots, not the whole disk.

**Option C — Whole-PC auto-discovery.**
Walk all drives for `.git` dirs.
- Trade-off: **expensive and noisy** — a full-disk walk is slow, hits permission errors, and surfaces every
  throwaway repo. `gh`/`git` per hit makes it minutes-long. Not recommended as a default; at most a one-off
  "deep scan" behind an explicit button with a progress indicator and a review-before-add checklist.

**Recommendation:** ship **A** (surface the config that already exists, moved to `%APPDATA%\castellyn`), add **B**
(scan a user-picked folder → checklist) as the discovery affordance. Skip **C** unless asked — whole-PC scan is
cost the owner rarely needs, and B covers "I keep forks in a couple of folders" without it.

---

## Highest-probability root causes (summary)

- **not-a-fork (bifrost, ccs):** *correct* detection, not a mislabel — repos under the `E:\Scripts\External`
  root whose GitHub origin has **no parent** and which aren't in `ownPaths` (`ForkSync.psm1:483`). The real
  defect is the catch-all root scan surfacing un-actionable dead cards; fix via Issue-4 config (reclassify/exclude)
  or collapse them into a "Not forks (N)" group.
- **wip-sync disabled while recommended:** the button gates on the **global** run slot
  (`rec.disabled = anyRunning || busy`, `ForkRepoCard.svelte:178`) while the recommendation uses only
  `canSyncWip`. Per-repo actions run on an **independent** per-repo slot (`run_fork_repo`, `lib.rs:1101`) that the
  backend does not gate on unrelated runs — so `anyRunning` over-disables. Fix: gate on
  `busy || running === 'forks'`.
