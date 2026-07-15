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
