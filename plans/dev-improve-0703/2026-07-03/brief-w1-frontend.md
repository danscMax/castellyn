# Brief: Wave 1 — frontend zone (items 1c, 1d)

Repo: `E:\Scripts\Castellyn`. Files: `src/lib/components/SessionsTab.svelte` (1c), `src/lib/components/TerminalPane.svelte` (1d). Svelte 5 runes. Comments in English. Do NOT commit. Do NOT touch other files, i18n files (no new strings needed), or any test.

## Read first
- `src/lib/components/SessionsTab.svelte:560-600` — closePane/closeAll (precedent: they already clear `maximized`), `activePanes`/`bgPanes` deriveds
- `src/lib/components/SessionsTab.svelte:676-682` — `toggleMax` / `toggleBackground`
- `src/lib/components/SessionsTab.svelte:1339-1352` — the maximized-mode switcher bar (`maxbar`, iterates `panes`)
- `src/lib/components/SessionsTab.svelte:325-390` — `sessionIds`, `onIdChange`, persist effect (LIVE_KEY), `broadcastInput`, `sendToAll`/`doSendToAll`
- `src/lib/components/TerminalPane.svelte:342-392` — spawn/attach, `onIdChange?.(paneKey, id)` at :363, `pty:exit` listener at :365-369, `relaunch()`
- `src/lib/components/TerminalPane.svelte:540-550` — the existing `onIdChange?.(paneKey, null)` call (teardown precedent)

## Contract (FROZEN)
- `onIdChange` prop signature stays `(key: string, id: string | null) => void`.
- No new i18n strings, no new props, no markup redesign.

## Tasks

### 1c — maximize + background must not leave a dead grid (SessionsTab.svelte)
Bug: maximize pane A → toggle A to background (from the maxbar / hover controls) → `maximized` still = A's key, but A is no longer in `activePanes` → the grid renders nothing ("dead grid").
- In `toggleBackground(key)`: if the pane is being sent TO background and `maximized === key`, reset `maximized = null` (mirror the `closePane` precedent at :566).
- The maximized switcher bar (`{#if maximized}` block at :1341): iterate `activePanes` instead of `panes`, so backgrounded panes don't appear as maxchips (clicking a bg pane's chip would set `maximized` to a pane the grid doesn't render — same dead grid).
- Sanity-check any other write of `maximized` for the same trap (drag/drop at :948 and :1001 read it — leave those).

### 1d — dead PTY must leave the send-to-all/broadcast target set (TerminalPane.svelte)
Bug: when the child process exits (`pty:exit` event at :365), the pane keeps its entry in SessionsTab's `sessionIds`, so `broadcastInput`, `sendToAll`, and the status counts still target a dead session.
- In the `pty:exit:{id}` listener callback, after setting `exited = true` and printing the "ended" line: call `onIdChange?.(paneKey, null);` and set the local `id = null` ONLY IF that does not break `relaunch()` — check: `relaunch()` uses `id` to `sessionKill` before respawn; after pty:exit the child is already dead, so nulling `id` just skips a redundant kill. Keep `attachId` panes in mind: for an attached mirror the exit event also fires — nulling is correct there too (the mirror can't write to a dead session).
- Known accepted consequence (do NOT "fix" it): the persist effect in SessionsTab (:349-358) filters by `sessionIds[p.key]`, so a dead pane drops out of the restore set — that is intended (nothing live to re-attach).
- Do NOT add extra state or events.

## Out of scope
Everything else in both files; other components; stores; i18n; tests.

## Verify (must pass)
```
npm run check   # svelte-check — must stay 0 errors 0 warnings
npm test        # vitest — 37 passing must stay passing
```

## Definition of done
- Maximize→background leaves a live grid (maximized cleared) and the maxbar never lists background panes.
- After a session's process exits, that pane is absent from `sessionIds` (excluded from broadcast/send-to-all/status counts); relaunch of that pane still works (code-trace it).
- Gates green.

## Report back
Per item: exact diff summary (file:line), the code-trace for relaunch-after-exit, gate outputs, anything that didn't fit the contract.
