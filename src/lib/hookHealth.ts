import type { AgentStatusHookState } from './ipc';

/** Overall health of the agent-status lifecycle hook, ranked worst-actionable first so the UI can
 *  pick one message/colour. `unavailable` = backend didn't answer; `off` = nothing wired anywhere
 *  (nothing to fix); `script-missing` = a profile references the hook but the script file is gone,
 *  so it silently no-ops (the real trap); `partial` = some profiles aren't fully wired (drift or a
 *  fresh profile); `healthy` = every profile fully wired and the script is present. */
export type HookHealthStatus = 'unavailable' | 'off' | 'script-missing' | 'partial' | 'healthy';

export type HookHealth = {
  status: HookHealthStatus;
  /** Profiles with all five lifecycle events wired. */
  wired: number;
  /** Wired + unwired = every profile Castellyn manages. */
  total: number;
  /** Profiles wired for some-but-not-all events (event-level drift). */
  drift: number;
};

/** Pure classification of `agent_status_hook_status`. First failing check wins, mirroring the
 *  backend's reason ordering, so the caller renders exactly one status. */
export function hookHealth(state: AgentStatusHookState | null): HookHealth {
  if (!state) return { status: 'unavailable', wired: 0, total: 0, drift: 0 };
  const wired = state.wired.length;
  const total = wired + state.unwired.length;
  const drift = state.partial.length;
  let status: HookHealthStatus;
  // Only "off" when there's truly nothing wired AND no drift to fix — otherwise a
  // partially-wired profile (0 fully wired, some partial) would wrongly read as "nothing to fix".
  if (wired === 0 && drift === 0) status = 'off';
  else if (!state.scriptPresent) status = 'script-missing';
  // `drift > 0` is NOT redundant with unwired: a profile can have event-level drift (some-but-not-all
  // events) while none are fully unwired, and that must still read as 'partial' (see hookHealth.test).
  else if (state.unwired.length > 0 || drift > 0) status = 'partial';
  else status = 'healthy';
  return { status, wired, total, drift };
}
