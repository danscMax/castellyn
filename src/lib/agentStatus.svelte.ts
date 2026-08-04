// Live agent-status summary for the Sessions grid. SessionsTab (the only writer) keeps
// it current from `agent-status` events; +page reads it for the sidebar attention badge.
// Module store instead of prop-drilling across the layout boundary — same pattern as
// running.svelte.ts.

/** Semantic pane states. `done` is frontend-derived: working/blocked → idle while the
 *  pane was not focused, cleared once the user looks at it (herdr's Idle+!seen). */
export type AgentPaneState = 'working' | 'blocked' | 'idle' | 'done' | 'limited' | 'unknown';

// `live` is the count of ALL active session panes regardless of tool/status — shell panes and
// hook-less Claude panes have no semantic state, so blocked+working+done alone undercounts what
// is really running (clicker-audit #1: Analytics said "no sessions" with 3 shells live).
export const agentSummary = $state({ blocked: 0, working: 0, done: 0, limited: 0, live: 0 });

// ── Backlog 25: how long did agents wait for ME ────────────────────────────────────────────────
// The status engine only ever knows a pane's CURRENT state, so this has to be accumulated as the
// transitions happen. It is accumulated HERE, next to the summary the Analytics tab already reads,
// rather than in the backend Track: a `Track.state_since` field was added for this once and removed
// by review as dead weight because nothing read it. `AnalyticsTab` reads every field below.
export type WaitStats = {
  /** Total ms panes spent in `blocked` — i.e. stopped, waiting on the user. */
  totalMs: number;
  /** How many completed waits that total is made of (so the average is honest). */
  count: number;
  longestMs: number;
  /** When counting started — a total means nothing without the window it covers. */
  since: number;
};

const WAIT_KEY = 'cmh-agent-wait';
const emptyWait = (): WaitStats => ({ totalMs: 0, count: 0, longestMs: 0, since: Date.now() });

function loadWait(): WaitStats {
  try {
    // Same defensive read as runHistory: a corrupted / colliding key must not break the tab.
    const v = JSON.parse(localStorage.getItem(WAIT_KEY) ?? 'null');
    if (v && typeof v.totalMs === 'number' && typeof v.count === 'number') {
      return {
        totalMs: v.totalMs,
        count: v.count,
        longestMs: typeof v.longestMs === 'number' ? v.longestMs : 0,
        since: typeof v.since === 'number' ? v.since : Date.now()
      };
    }
  } catch {
    /* unreadable or unavailable (SSR) — start fresh */
  }
  return emptyWait();
}

export const agentWait = $state<WaitStats>(loadWait());

// Panes currently blocked → when they entered that state. Deliberately NOT persisted: a wait that
// was still open when the app closed has no end, and inventing one would inflate the total.
const blockedSince = new Map<string, number>();

/**
 * Feed one pane state transition. Called from SessionsTab's `agent-status` listener, which is the
 * single place a pane's state change is observed.
 *
 * Only `blocked` counts: `limited` is the agent parked on quota, which is not the user's to answer,
 * and lumping the two together would make the number unusable for the question it exists to answer.
 */
export function noteWait(id: string, state: AgentPaneState) {
  if (state === 'blocked') {
    // Re-entering an already-open wait keeps the original start (repeat events are normal).
    if (!blockedSince.has(id)) blockedSince.set(id, Date.now());
    return;
  }
  const started = blockedSince.get(id);
  if (started === undefined) return;
  blockedSince.delete(id);
  // Clamped: `Date.now()` is wall time, so a backward clock adjustment (NTP step, DST-less manual
  // set) mid-wait would otherwise bill a negative delta into a total that is never recomputed.
  const ms = Math.max(0, Date.now() - started);
  agentWait.totalMs += ms;
  agentWait.count += 1;
  if (ms > agentWait.longestMs) agentWait.longestMs = ms;
  try {
    localStorage.setItem(WAIT_KEY, JSON.stringify(agentWait));
  } catch {
    /* quota / unavailable — a metric is not worth failing a session over */
  }
}

export function clearAgentWait() {
  blockedSince.clear();
  Object.assign(agentWait, emptyWait());
  try {
    localStorage.removeItem(WAIT_KEY);
  } catch {
    /* ignore */
  }
}
