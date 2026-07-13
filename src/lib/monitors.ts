// Shared monitor list — fetched once and reused across every pane and window, instead of each
// TerminalPane calling list_monitors() on its own (which fired N identical Win32 enumerations).
// Monitors rarely change; call invalidateMonitors() (e.g. after a hotplug) to force a refresh.
import { listMonitors, openMonitorWindow, prepareDetach, type DetachPane, type MonitorInfo } from '$lib/ipc';

let cache: MonitorInfo[] | null = null;
let inflight: Promise<MonitorInfo[]> | null = null;
// L14: after a failed enumeration we deliberately don't cache (so a transient error retries), but a
// caller that re-requests whenever the list is empty (e.g. TerminalPane's monitors-length effect)
// would then spin with no backoff. Suppress retries for a short window after a failure.
let coolUntil = 0;
const FAIL_COOLDOWN_MS = 5000;
// A single stable empty result for the no-data paths (cooldown / failure). Returning a FRESH `[]`
// each call made a caller that assigns it to reactive state (TerminalPane's monitors-length effect)
// re-run forever — a new reference reads as "changed" every tick. One shared constant is `===` to
// itself, so the assignment settles instead of spinning. Treated as read-only; callers never mutate.
const EMPTY: MonitorInfo[] = [];

/** The monitor list, cached. Concurrent callers share one in-flight request. */
export async function getMonitors(): Promise<MonitorInfo[]> {
  if (cache) return cache;
  if (Date.now() < coolUntil) return EMPTY; // L14: in post-failure cooldown — don't hammer list_monitors
  if (!inflight) {
    inflight = listMonitors()
      .then((m) => {
        cache = m;
        return m;
      })
      .catch(() => {
        coolUntil = Date.now() + FAIL_COOLDOWN_MS; // transient failure → brief cooldown, then retry
        return EMPTY;
      })
      .finally(() => {
        inflight = null;
      });
  }
  return inflight;
}

/** Drop the cache so the next getMonitors() re-enumerates (after a monitor hotplug / layout change). */
export function invalidateMonitors(): void {
  cache = null;
  coolUntil = 0; // a deliberate refresh must not be blocked by a stale failure cooldown
}

/**
 * Stash a detached-window spec and open a frameless window on monitor `idx`. The single place the
 * prepareDetach → openMonitorWindow → "did it open?" sequence lives (was duplicated in SessionsTab's
 * distribute/restore and TerminalPane's send-to-monitor). Returns false if the window/monitor was
 * unavailable so the caller can leave the pane(s) where they are.
 */
export async function openDetached(label: string, idx: number, panes: DetachPane[]): Promise<boolean> {
  try {
    await prepareDetach(label, { panes });
    await openMonitorWindow(label, idx);
    return true;
  } catch {
    return false; // monitor/window unavailable
  }
}
