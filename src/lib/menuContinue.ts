/**
 * #21f — pure decision for the auto-continue state machine, one action per tick per pane. Split out
 * of SessionsTab.maybeAutoContinue so the two-phase logic (pick option 1 on a blocking menu, then
 * inject "continue") is deterministic and unit-tested, instead of buried in a Svelte component that
 * only a live rate limit could exercise. The keystrokes and guard bookkeeping stay in the caller;
 * this only decides WHAT to do this tick.
 *
 * Two menus are auto-driven the same way (pick option 1, then continue):
 *   - the limit menu   "What do you want to do? / 1. Stop and wait for limit to reset / …"
 *   - the resume menu  "1. Resume from summary / 2. Resume full session as-is / …"
 * plus the passive limit banner (no menu → no keypress, just "continue").
 *
 * TIMING is driven by RELIABLE signals, never the live agent-status `limited` flag (which flickers —
 * a keypress or banner floods PTY output and the flag clears mid-episode):
 *   - `sawRateLimit` — STICKY: a real rate limit was seen this episode (agent-status went 'limited',
 *     which the backend sets only for the limit menu/banner, NOT a bare resume menu). Once set it
 *     persists until the episode is re-armed, so a mid-episode flicker can't drop the flow or make a
 *     limit look like a resume.
 *   - `resetMs` — the endpoint monitor's reset time (limits-status), the reliable "when does it clear".
 * A rate limit waits for `resetMs`; a bare resume menu (healthy profile) continues after a short settle.
 */
export type MenuContinueState = {
  /** STICKY: a real rate limit was detected for this pane this episode (agent-status 'limited').
   *  Distinct from a bare resume menu. Drives "wait for the reset"; survives the flickering flag. */
  sawRateLimit: boolean;
  /** A limit/resume menu is currently up → press option 1. */
  menuUp: boolean;
  /** Option 1 was already pressed this episode. */
  menuDismissed: boolean;
  /** "continue" was already sent this episode. */
  continued: boolean;
  /** User is at the pane (focused/typing) — defer so we never fight their keystrokes. */
  busy: boolean;
  /** Endpoint reset time (epoch ms) for the exhausted window, or null when unknown/none. Reliable —
   *  from the limits-status poll, not PTY-derived. */
  resetMs: number | null;
  /** Per-pane jitter added to the reset so N panes don't fire in lockstep. */
  jitterMs: number;
  /** When option 1 was pressed (epoch ms), or null if not yet. */
  menuDismissedAtMs: number | null;
  nowMs: number;
};

/** `rearm` = episode over, clear guards. `press1` = send option-1 keypress. `continue` = inject the
 *  continuation. `wait` = active but nothing to do this tick. */
export type MenuContinueAction = 'rearm' | 'press1' | 'continue' | 'wait';

/** Resume menu (no rate limit): let the summary start loading after option 1 before continuing. */
const RESUME_SETTLE_MS = 6_000;

export function decideMenuContinue(s: MenuContinueState): MenuContinueAction {
  // In an episode while: a rate limit is (was) active, OR a menu is up, OR we pressed option 1 and
  // haven't continued yet. The sticky `sawRateLimit` + `menuDismissed` keep the pane active across the
  // live-flag flicker; only `continued` (with no menu left) or a truly idle pane re-arms.
  const inEpisode = s.sawRateLimit || s.menuUp || (s.menuDismissed && !s.continued);
  if (!inEpisode || (s.continued && !s.menuUp)) return 'rearm';
  if (s.continued || s.busy) return 'wait';

  // Phase 1 — pick option 1 on whichever blocking menu is up (once per episode).
  if (s.menuUp && !s.menuDismissed) return 'press1';

  // Phase 2 — inject "continue". Keyed on the STICKY sawRateLimit and the endpoint reset, never the
  // flickering live `limited` flag.
  if (s.sawRateLimit) {
    // Real rate limit: wait for the endpoint reset (its own wait dwarfs any settle), then continue —
    // but never into a still-open menu.
    if (s.resetMs == null || s.nowMs < s.resetMs + s.jitterMs) return 'wait';
    return s.menuUp ? 'wait' : 'continue';
  }
  // Resume menu (no rate limit): continue once the menu has cleared + a short settle.
  const settled = s.menuDismissedAtMs == null || s.nowMs - s.menuDismissedAtMs >= RESUME_SETTLE_MS;
  return !s.menuUp && settled ? 'continue' : 'wait';
}

/** One usage window's exhaustion + its reset time already parsed to epoch ms (NaN when unknown). */
export type ResetWindow = { util: number; resetMs: number };

/**
 * The endpoint reset a rate-limited pane should wait for (`resetMs` fed to {@link decideMenuContinue}).
 * It's the LATER reset among the EXHAUSTED (≥99%) windows — V-18: a pane capped on 7d still has a
 * near-future 5h reset, so the *binding* one is whichever resets last. When the endpoint hasn't caught
 * up to a just-hit limit (5-min poll lag → no window reads ≥99 yet), fall back to the 5h reset. Returns
 * null when nothing is finite. Extracted from SessionsTab.bindingResetMs so it's unit-testable (#18).
 */
export function pickBindingResetMs(windows: ResetWindow[], h5FallbackMs: number): number | null {
  const capped = windows.filter((w) => w.util >= 99 && Number.isFinite(w.resetMs)).map((w) => w.resetMs);
  if (capped.length) return Math.max(...capped);
  return Number.isFinite(h5FallbackMs) ? h5FallbackMs : null;
}
