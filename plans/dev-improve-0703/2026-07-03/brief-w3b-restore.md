# Brief: Wave 3B — SessionsTab.svelte (item 3)

Repo `E:\Scripts\Castellyn`. OWNED file: `src/lib/components/SessionsTab.svelte` ONLY. Svelte 5 runes. Comments English.
Do NOT commit. Do NOT touch other files. i18n is READ-ONLY (needed keys exist: `sessions.restoreOffer`, `restoreDo`, `restoreDismiss`). No new deps.

## The bug (item 3)
`restore-набор затирается до решения пользователя`. Flow:
- On mount, `savedLive` is read synchronously from `localStorage[LIVE_KEY]` (`cmh-sessions-live`) — see ~342.
- The persist `$effect` (~349-358) writes LIVE_KEY = current live panes on every change. On a fresh mount panes start empty, then the async re-attach (~136-152) adds back only the STILL-ALIVE sessions and sets `restorable` = the DEAD entries (a full app restart's sessions) to offer a "restore the set" bar.
- Problem: the persist effect keeps overwriting LIVE_KEY with only the live/re-attached panes. The DEAD `restorable` entries are dropped from LIVE_KEY immediately. So if the user reloads the webview AGAIN before acting on the restore bar (or just doesn't act), the restore offer is permanently lost — `savedLive` on the next mount no longer contains them.

## Read first
- ~340-358: `savedLive` init + the persist `$effect` + `LIVE_KEY`, `LivePane` type.
- ~134-152: the async re-attach that sets `restorable = savedLive.filter(dead)`.
- ~1080-1096: `restoreLast()` (accepts — spawns the set, then `restorable = []`).
- Find the DISMISS handler (the `restoreDismiss` button in the restore bar markup ~1230s) — it should set `restorable = []`.

## Task
Make the persisted LIVE_KEY retain the pending `restorable` entries until the user resolves the bar (accept or dismiss), WITHOUT sticking on a stale set forever:
- In the persist `$effect`, when `restorable.length > 0`, write LIVE_KEY = **current live panes PLUS the still-pending `restorable` entries** (dedupe by `id` so a re-attached pane isn't duplicated). This way a second webview reload still reconstructs the same dead set and re-offers it.
- When `restorable` is empty (user accepted via `restoreLast` or dismissed), the effect resumes writing ONLY the live panes (normal behavior) — the stale set clears. Confirm the dismiss handler sets `restorable = []` (if a dismiss path doesn't exist or doesn't clear it, add/ensure it, using the existing `restoreDismiss` button).
- Keep the shape of persisted entries = `LivePane` (so the next mount's `savedLive` parse + alive-check still works; dead restorable entries will again be classified dead → re-offered).
- Do NOT change how `restorable` is initially computed, or the accept path's resume-args logic.

Edge to honor (risk note): the set must NOT become permanent — a dismiss clears `restorable` → the very next effect run overwrites LIVE_KEY without the dead entries. Verify that ordering (dismiss sets restorable=[] and the effect re-runs).

## Verify
```
npm run check   # 0 errors 0 warnings
npm test        # vitest green (incl. i18n parity)
```
Reason through the reload scenario in your report: restart → mount → restore bar shows → reload again → bar STILL shows (dead set preserved) → dismiss → reload → bar gone.

## Report back (ONE completion — no re-query)
Changed line ranges, how you merged restorable into the persisted set (dedupe key), the dismiss-clears-it confirmation, and the two gate summary lines.
