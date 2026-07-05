# Diagnosis — Profiles / Matrix / MCP cluster (2026-07-05)

Scope: `ProfilesTab.svelte`, `ProfileEditDialog.svelte`, `ProfilesMatrix.svelte`, `McpTab.svelte`,
`DataTable.svelte`, `DropdownMenu.svelte`, `Select.svelte`, `floating.ts`, `+page.svelte`, `lib.rs`.

## TL;DR / headline finding

**Nothing in this cluster is stubbed, disabled-by-a-dead-condition, or missing a handler.** Every
row-menu item, every matrix cell, and every MCP button is wired end-to-end: UI → `ipc.ts` wrapper →
a real `#[tauri::command]` in `lib.rs`. I verified each command exists (`run_profile_mgmt`,
`set_profile_proxy`, `set_profile_folders`, `set_profile_plugins`, `run_profile_relink`,
`run_provider`, `read_profile_matrix`).

The single mechanism that makes the **row menu, the matrix, and the MCP tab all go dead at once** is
the **global `running` busy-gate**. While any run holds the lock, `busy = !!running` disables every
interactive control across all three surfaces simultaneously — with **no "why is this disabled?"
affordance**. The owner's list of dead menu items is an exact match for the `disabled: busy` items.

The table cut-off (symptom 1) is a separate, fully-static CSS bug: hardcoded fixed column widths sum
wider than the window, so `overflow-x: auto` kicks in and the last column (Действия) is clipped.

---

## Issue 1 — Profiles table overflows / Действия column clipped

**Symptom:** horizontal scrollbar appears; the Действия column is cut off on the right.

**Root cause (probability 95%): fixed column widths that sum wider than the viewport, under
`table-layout: fixed`.**

`ProfilesTab.svelte:338-345` declares six fixed-width columns:

```
name (grow → 260px default), status 150px, usage 170px, provider 200px, links 92px, actions 240px
```

`DataTable.svelte:214-215` resolves the width:

```js
const colWidth = (c) => colW[c.key] ?? c.width ?? (c.grow ? '260px' : '160px');
```

and the table is `table-layout: fixed` (`DataTable.svelte:358`) with a trailing auto spacer column
plus a persistent `.dt-scroll { overflow-x: auto }` wrapper (`DataTable.svelte:350-352`). The six
columns total **~1112px** before cell padding (`var(--sw-space-3)` each side) and the card chrome. On
a ~1280–1366px window minus the sidebar, that overflows → horizontal scroll → the 240px Действия
column (last) sits past the right edge.

Evidence it is purely width, not layout: `table-layout: fixed` never shrinks columns below their
declared width, so there is no responsive collapse. The `usage` (170px) and `provider` (200px)
columns are the fattest non-essential ones.

**Fix (root-cause) with trade-offs:**
- **A (recommended): make the table responsive** — drop the fixed `usage`/`links` columns into the
  expand panel below a breakpoint, or let `provider`/`usage` shrink. Cheapest concrete version:
  trim `usage` 170→130, `provider` 200→160, `actions` 240→200 and rely on truncation. Trade-off:
  buttons in Действия get tighter; may need to fold Launch/Folder behind the ⋯ menu.
- **B: sticky Действия column** (`position: sticky; right: 0`) so it is always reachable even when
  the table scrolls. Trade-off: adds CSS complexity to the shared DataTable; z-index/border seams.
- **C: move the whole row-actions set into the ⋯ menu** (Launch/Folder become menu items), leaving
  one narrow Действия column. Trade-off: Launch is the primary action — burying it costs a click.

Recommend A now (it also relieves symptom 2b), consider B for DataTable generally.

---

## Issue 2 — Row 3-dot ("⋯") menu items appear inactive

**Symptom:** «починить связи», «сбросить провайдера», «цвет», «переименовать», «изменить описание»
none work / all disabled. Single biggest functional complaint.

**Every handler exists and is wired.** `ProfilesTab.svelte:275-335` `menuItems()` builds items with
live `onClick`s (`openDlg('recolor'|'rename'|'redescribe')`, `finishProfile` (repair/fix-links),
`onProviderClear` (reset provider)). `DropdownMenu.svelte:46-50` `pick()` calls `it.onClick()` after
closing. `openDlg` → `onDlgSubmit` (`:147-153`) → `onMgmt` → `+page onProfileMgmt` (`:662`) →
`runProfileMgmt` → **Rust `run_profile_mgmt` (`lib.rs:1793`, streamed via `stream_id::PROFILES`
:1854)**. So this is **not** a wiring or stub bug.

**Candidate root causes:**

- **(2a) Busy-gate — probability 55%.** Every one of the owner's named items carries
  `disabled: busy` (`ProfilesTab.svelte:294, 304, 311, 317, 322, 330`), where
  `busy = $derived(!!running)` (`:85`). The **only** menu item WITHOUT `disabled: busy` is «view
  config» (`:287-289`) — and the owner did not list it as dead. That exact split is the fingerprint
  of `running != null` at screenshot time (a `profiles`/`mcp`/`all` run holding the lock).
  - The lock does clear correctly on `run-done` (`+page:2181 if (running === id) running = null`),
    and there is no periodic auto-check loop (no `setInterval`; `startRun('all','check')` only fires
    from onboarding/tray/palette/chained run-done). So it is an **in-flight** run, not a permanently
    stuck lock — but during that window the entire cluster is dead with no explanation.
- **(2b) Действия column clipped off-screen — probability 30%.** Per Issue 1, the ⋯ trigger lives in
  the last, clipped column; it is partially/fully past the scroll edge, so it reads as
  missing/unclickable until the user scrolls right.
- **(2c) Genuine runtime error breaking interactivity — probability 15%.** Cannot be excluded from
  static reading; needs a live check (the matrix renders real data, see Issue 3, which argues the
  tab is not crashed).

**Fix with trade-offs:**
- For **2a (recommended):** stop hard-disabling *lifecycle* menu items on unrelated runs, and/or add
  a visible reason. Two options: (i) only disable items that truly conflict with the in-flight op
  (rename/remove of a profile, not «color»/«view»); (ii) keep the gate but show a tooltip/inline
  "busy — a task is running" so a disabled control is legible. Trade-off: (i) risks queuing a mgmt
  run while a script runs — but `onProfileMgmt` already early-returns on `if (running) return`
  (`:663`), so the click would silently no-op; better to let the item stay enabled and surface the
  busy toast on click than to grey it out mutely.
- For **2b:** fixed by Issue 1's responsive fix (or a sticky actions column).

---

## Issue 3 — Matrix: "ничего не выбирается, поменять нельзя"

**Symptom:** provider dropdowns / plugin / MCP cells look non-interactive. MCP cell shows "2/2 +1"
(orange).

**The matrix renders real data and is fully wired.** The "2/2 +1" the owner saw is
`ProfilesMatrix.svelte:407` (`deployed/canon +extras`, amber via `mcpWarn` because `extras.length>0`,
`:180-182`) — proof the section is live, not crashed. The provider `<Select>` is bound
(`bind:value={d.provider}`, `:356-360`), edits accumulate into `draft` and dirty-track
(`rowDirty` :184-186), and Apply → preview → `confirmApply` → `onApplyMatrix` (`+page:923`) runs
provider/proxy/folders/relink through real commands. All present.

**Candidate root causes:**

- **(3a) Busy-gate — probability 45%.** Select `disabled={busy || applying}` (`:359`), proxy input
  `:369`, all three chips `:379, 392, 403`, so a run in flight makes every cell dead — matches "нельзя
  поменять" exactly. Same mechanism as 2a.
- **(3b) Discoverability of the accumulate-then-apply model — probability 45%.** By design, changing
  a cell does **not** take effect; it only sets a draft with a tiny amber dirty-dot (`:349`), and the
  actual write is the far-away Apply bar at the bottom (`:490-498`). A user changing a provider and
  seeing "nothing happened" (no immediate write, dirty-dot easy to miss) reads it as "не меняется".
  The header hint «Правки копятся и применяются одной кнопкой» is small muted text (`:314`).
- **(3c) Select panel mispositioned by `anchored` — probability 10%.** The panel is
  `position: fixed` placed by `floating.ts`; inside `.sw-card` (backdrop-filter) it relies on the
  self-correct delta (`floating.ts:43-55`). If a containing-block ancestor defeats that, the panel
  could land off the trigger. Lower probability (same code powers working selects elsewhere).

**Fix with trade-offs:**
- **3b (recommended):** make dirty state loud — a per-row "unsaved" badge and a sticky/floating Apply
  bar that appears the moment any row is dirty, near the edited row rather than only at the bottom.
  Trade-off: more chrome; but it converts a silent model into a visible one.
- **3a:** same treatment as 2a (don't mute the whole matrix on unrelated runs, or explain why).

---

## Issue 4 — MCP tab "ничего не кликается"; plugins hard to discover

**Symptom:** MCP tab feels non-interactive; plugins list opaque.

**Root cause (probability 60%): busy-gate.** Every actionable control in `McpTab.svelte` is
`disabled={busy}`: refresh `:124`, add `:128`, deploy-all `:132`, per-profile deploy chips `:187`,
edit `:195`, delete `:197`, remove-extra `:224`. The **only** thing not gated is the bulk profile
selector chips in the toolbar (`:154`, `toggleBulk`). So during any run the tab is dead except those
toggles — a precise match for "nothing clicks". Same unifying `busy` mechanism as 2a/3a.

Secondary (plugins discoverability) belongs to the Plugins tab (out of this file set), but note the
MCP tab's own "плагины" column just shows `context7`/`serena` as an un-deployable note
(`McpTab.svelte:81, 178-179`) with no link to what's actually installed.

**Fix:** same busy-gate treatment; and give the MCP deployed/plugin badges a tooltip or drill-in.

---

## Issue 5 — Zero-state / de-hardcode

**Zero-state add-profile flow EXISTS and works.** «+ Добавить профиль» (`ProfilesTab.svelte:371`) →
`openDlg('add')` → `onDlgSubmit` → `onMgmt({action:'add', name, color, description})` →
`run_profile_mgmt` add. A fresh user with zero profiles gets `EmptyState` (`:618-619`) plus the
`OnboardingWizard` (`+page:2634`, `profileCount=0`), and the matrix is correctly hidden until at
least one profile exists (`{#if profiles.length}` `:622`). Profile names are **not** hardcoded here —
they come from `data.profiles`.

**But two real hardcodes remain in `McpTab.svelte`:**

- **`ALL_PROFILES` fallback (`:77-79`):**
  ```js
  const ALL_PROFILES = $derived(data?.profiles?.length ? data.profiles
      : ['ccmy', 'cc1', 'cc2', 'cc3', 'cc4', 'cc5']);
  ```
  On first paint (before `read_mcp` resolves) and for any user whose profiles are NOT named
  cc1..cc5/ccmy, the profile chips, the bulk selector, and the `n/total` deployed badge show the
  wrong set. Root-cause fix: fall back to the profiles already loaded elsewhere (`profilesData`) or
  render nothing until `data.profiles` is known, instead of a canned list.
- **`PLUGIN_PROVIDED = ['context7', 'serena']` (`:81`):** hardcodes which servers are
  marketplace-provided (skipped by the installer). If the canon changes, this silently misclassifies.
  Lower urgency (matches the installer's own skip list) but should derive from the same source of
  truth, not a literal. (Cross-ref: the memory note `mcp_deployable_canon` — Deploy-Mcp skips
  context7/serena; this literal must stay in sync with that.)

**Fix with trade-offs:** thread the real profile list (already in `+page` as `profilesData`) into
McpTab as a prop so both the fallback and the badge use one source; drop the `cc*` literal. Trade-off:
one more prop, but kills the last name-hardcode a fresh user would hit.

---

## Cross-cutting recommendation

The dead-menu / dead-matrix / dead-MCP triad is **one root cause wearing three hats**: an
all-or-nothing `busy` gate with no legibility. Fixing that one pattern (scope the disable to truly
conflicting ops, and/or explain the disabled state) resolves the "single biggest functional
complaint" across all three surfaces at once. The table cut-off and the `cc*` hardcode are the two
independent, fully-static bugs to fix alongside.

**Confidence caveat:** everything above is from static reading. The busy-gate hypotheses (2a/3a/4)
are ~50-60% each and should be confirmed with one live observation of `running`'s value while the
owner reproduces — because in the idle state (`running == null`) the code makes every control enabled
and functional.
