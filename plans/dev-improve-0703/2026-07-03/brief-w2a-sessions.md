# Brief: Wave 2A — sessions cluster (items 9, 12, 16, 20)

Repo `E:\Scripts\Castellyn`. Svelte 5 runes. Comments English. Do NOT commit. Do NOT touch files outside this OWNED set:
`src/lib/components/SessionsTab.svelte`, `src/lib/components/TerminalPane.svelte`, `src/app.css`,
`src-tauri/src/agent_status.rs`, `src/lib/ipc.ts`.
i18n locale files are READ-ONLY — all keys you need already exist (listed per item). Do NOT add/edit i18n.
Do NOT modify existing tests (you MAY add agent_status.rs tests). No new dependency. No new props beyond what's listed.

## Read first
- `src/lib/components/EmptyState.svelte` (props: `icon, title, description, action, actionLabel`) — reuse target for item 12.
- `src/lib/components/SessionsTab.svelte`: empty block ~1461 (item 12); launch/atLimit ~500-540 + `restoreLast`/`launchWorkspace` (item 16); `paneLabel`/`Pane` type + `LivePane` type ~339 + persist effect ~349 + `Fav`/favorites (item 20); inline `color:#3fb950` ~1298 and CSS `#2dd4bf` ~1510 (item 9); `agentStates`/dot rendering + tooltip (item 20 elapsed).
- `src/lib/components/TerminalPane.svelte`: `#2dd4bf` ~764 (item 9); xterm `term` init in `onMount`/`start` — add `term.onBell` (item 20).
- `src/app.css`: `:root` status tokens ~96-102 and `.light` block ~131+ (item 9).
- `src-tauri/src/agent_status.rs`: `struct StatusEvent` line ~109-121, its push site ~301-308, `Track` has `spawned_at`/`last_output` already (item 20).
- `src/lib/ipc.ts`: `AgentStatusEvent` type (item 20 — add the new field).

## Contract (FROZEN)
- i18n keys to USE (already added): `sessions.emptyTitle`, `sessions.emptyHint`, `sessions.phLaunch` (item 12); `sessions.restoredPartial` `{n}{m}` (item 16); `sessions.activeFor` `{d}` (item 20).
- StatusEvent serializes `#[serde(rename_all="camelCase")]` → a new rust field `spawned_at: u64` reaches the frontend as `spawnedAt`.
- Session cap constant: reuse the existing limit used by the grid (grep for the 12-pane / MAX cap in SessionsTab — do NOT hardcode a new one).

## Tasks

### 9 — status color tokens
- In `src/app.css` `:root`: add `--sw-status-warn: var(--sw-warn);` and `--sw-status-done: #2dd4bf;`. In `.light`: add a `--sw-status-done` override with adequate contrast on the near-white surface (pick a darker teal, ~#0d9488) and a `--sw-status-done`-consistent value; keep `--sw-status-warn` inheriting `--sw-warn` (light already overrides `--sw-warn`? verify — if not, the var chain still resolves).
- Replace inline `color:#3fb950` (SessionsTab ~1298 ssh-test-ok) with `var(--sw-status-up)` (green "ok" already has a token — reuse it; #3fb950 ≈ up-green) OR `--sw-status-done` if semantically "done". Pick `--sw-status-up` for an ssh OK check.
- Replace CSS `#2dd4bf` occurrences (TerminalPane ~764 done-dot, SessionsTab ~1510) with `var(--sw-status-done)`.
- Replace the `var(--sw-status-warn, #e0b341)` fallbacks (TerminalPane ~750/756, SessionsTab ~1480/1507) — the token now exists, so the literal fallback is dead; keep `var(--sw-status-warn)` (drop the `, #e0b341`) OR leave the fallback (harmless). Prefer dropping it for cleanliness.
- Verify: `npm run check` 0/0; both themes render (screenshot not required from you — just keep tokens valid).

### 12 — empty state → EmptyState.svelte
- Replace the manual `.empty` block (~1461-1470) with `<EmptyState icon={...} title={t('sessions.emptyTitle')} description={t('sessions.emptyHint')} action={launchPhrase} actionLabel={t('sessions.phLaunch')} />`. Import EmptyState and a suitable Lucide icon (e.g. `Terminal` or `SquareTerminal` — match what other tabs import). Remove the now-dead `.empty`/`.empty-icon` CSS if nothing else uses it (grep first).

### 16 — atLimit: disable launch + partial-restore toast
- Find the session-cap gate (`atLimit` or the count-vs-max check). Add `disabled={atLimit}` to the launch (▶) buttons so the user can't exceed the cap silently.
- In `restoreLast`/`launchWorkspace` (whatever restores/launches a set), when the cap would truncate the set, count the dropped panes and after restoring show `pushToast({kind:'info', title: t('sessions.restoredPartial', {n: restored, m: total})})`. Only toast when `restored < total`.

### 20 — pane name persist + bell→unread + elapsed
- **Name persist:** add optional `name?: string` to the `LivePane` type and include `name: p.name` in the persist map (~349) and in restore (respawn keeps the name). If favorites (`Fav`) store launch configs, add `name?` there too so a saved favorite keeps its rename. `Pane.name` already exists.
- **Bell → unread:** in TerminalPane `start()`/onMount after `term` is created, add `term.onBell(() => onActivity?.(paneKey));` so a shell BEL marks the pane unread (same `onActivity` used for off-screen output). Guard: only fire when not visible? No — bell is an explicit attention signal; fire always (the unread marker self-clears on focus).
- **Elapsed:** add `spawned_at: u64` to `StatusEvent` (agent_status.rs) populated from `t.spawned_at` at the push site (~301). Add `spawnedAt?: number` to `AgentStatusEvent` in ipc.ts. In SessionsTab, where the status dot has a `title=`, append the elapsed: compute `t('sessions.activeFor', {d: <humanized now - spawnedAt>})` — humanize to `Nm`/`Nh Mm` (small local helper; minute granularity). No new backend events for elapsed — it's derived on render from the static spawnedAt. Add a rust unit test that the emitted StatusEvent carries a non-zero spawned_at.

## Verify (all must pass)
```
npm run check      # 0 errors 0 warnings
npm test           # vitest incl. i18n parity — stays green
cd src-tauri && C:\Users\User\.cargo\bin\cargo.exe test   # stays green + your new test
```

## Report back (first completion — I will NOT re-query you)
Per item: changed file:line ranges, new agent_status test name, gate outputs (paste the real summary lines), any contract deviation flagged explicitly.
