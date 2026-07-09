// P6: one shared usage poll per profile (ref-counted), so N ProfileUsageBadges of the same profile
// don't each fire their own interval.
//
// The cadence matches the backend's limits poller (300s) and its shared request cache. It used to be
// 70s against a 60s cache TTL — every tick missed the cache by ten seconds, so each visible badge was
// its own stream of requests to Anthropic, on top of the limits poller and the user's statusline.
// Together they earned a real 429. Usage figures move on the scale of hours; polling them five times
// as often bought nothing.
import { readProfileUsage, type ProfileUsage } from '$lib/ipc';

const POLL_MS = 300_000;

// Reactive per-profile usage — components read `usageStore[profile]` in a $derived to stay live.
export const usageStore = $state<Record<string, ProfileUsage | null>>({});

type Entry = { timer: ReturnType<typeof setInterval> | undefined; refs: number };
const entries = new Map<string, Entry>();

async function load(profile: string) {
  try {
    usageStore[profile] = await readProfileUsage(profile);
  } catch {
    usageStore[profile] = null;
  }
}

/** Subscribe a badge to a profile's usage. Starts the shared poll on the first subscriber and stops
 *  it when the last one unsubscribes. Returns an unsubscribe function. */
export function subscribeUsage(profile: string): () => void {
  let e = entries.get(profile);
  if (!e) {
    e = { timer: undefined, refs: 0 };
    entries.set(profile, e);
    load(profile);
    e.timer = setInterval(() => load(profile), POLL_MS);
  }
  e.refs++;
  return () => {
    const en = entries.get(profile);
    if (!en) return;
    en.refs--;
    if (en.refs <= 0) {
      if (en.timer) clearInterval(en.timer);
      entries.delete(profile);
    }
  };
}
