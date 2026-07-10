// Live per-profile Anthropic usage (5h/7d utilization). The backend emits a `limits-status` event
// every poll for each OAuth profile (see LimitsStatusEvent) — this store is the single listener sink
// so the Analytics "Claude usage" panel and the title-bar status strip read the same live data
// without any extra backend calls. Mirrors agentStatus.svelte.ts / running.svelte.ts (module store,
// no prop-drilling across the layout boundary). +page wires the listener once on mount.

import type { LimitsStatusEvent } from '$lib/ipc';

/** A stored reading plus the moment it arrived. A transport error emits no event at all, so without
 *  an arrival stamp the last successful numbers look current forever — and auto-switch would trust
 *  them (see `LIMITS_STALE_MS` in limitSwitch.ts). */
export type LimitsEntry = LimitsStatusEvent & { receivedAt: number };

export const limitsStore = $state<{ byProfile: Record<string, LimitsEntry> }>({
  byProfile: {}
});

export function pushLimits(e: LimitsStatusEvent) {
  limitsStore.byProfile[e.profile] = { ...e, receivedAt: Date.now() };
}

/** Peak utilization across all polled profiles (max of every 5h/7d/model-scoped %), or null if
 *  nothing polled. Used by the title-bar strip to surface "closest to a limit" at a glance.
 *  `window` is '5h' | '7d' | the scoped cap's model label (rendered verbatim in the strip). */
export function peakUtilization(): { profile: string; pct: number; window: string } | null {
  let best: { profile: string; pct: number; window: string } | null = null;
  for (const e of Object.values(limitsStore.byProfile)) {
    if (e.expired) continue;
    const cands: Array<[number | null, string]> = [
      [e.h5, '5h'],
      [e.d7, '7d'],
      // A per-model weekly cap can exceed d7 — then it IS the number closest to a limit.
      [e.scoped ?? null, e.scopedLabel ?? '7d']
    ];
    for (const [pct, window] of cands) {
      if (pct != null && (!best || pct > best.pct)) best = { profile: e.profile, pct, window };
    }
  }
  return best;
}
