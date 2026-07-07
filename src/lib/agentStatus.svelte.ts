// Live agent-status summary for the Sessions grid. SessionsTab (the only writer) keeps
// it current from `agent-status` events; +page reads it for the sidebar attention badge.
// Module store instead of prop-drilling across the layout boundary — same pattern as
// running.svelte.ts.

/** Semantic pane states. `done` is frontend-derived: working/blocked → idle while the
 *  pane was not focused, cleared once the user looks at it (herdr's Idle+!seen). */
export type AgentPaneState = 'working' | 'blocked' | 'idle' | 'done' | 'limited' | 'unknown';

export const agentSummary = $state({ blocked: 0, working: 0, done: 0, limited: 0 });
