import type { ProfileInfo, LimitsStatusEvent } from './ipc';

/** A datapoint older than this is not evidence of anything: the backend polls every 300 s, so two
 *  missed rounds mean the network (or the endpoint) has been failing and the numbers are guesses. */
export const LIMITS_STALE_MS = 660_000; // 2 poll intervals + a minute of slack

/**
 * #21e: pick the profile to resume a rate-limited conversation under.
 *
 * Eligible = a DIFFERENT profile that (a) exists, (b) has USABLE OAuth credentials, (c) has intact
 * shared links — so `--resume <id>` reaches the shared ~/.claude/projects transcript — and (d) has a
 * KNOWN 5h utilisation below 85%. The least-utilised eligible profile wins; `null` when none qualify
 * (caller then falls back to "wait"). Utilisation must be known: a profile with no limits-status
 * datapoint is excluded rather than switched to blindly.
 *
 * "Usable" is stricter than "has a .credentials.json": a profile whose tokens are empty/expired, or
 * whose onboarding flag was cleared by /logout, drops the resumed session into a login wizard
 * instead of continuing the conversation. Both fields are optional in older profiles.last.json
 * snapshots, so absence means "don't know" and stays eligible — only a positive `false` disqualifies.
 *
 * A datapoint must also be RECENT. A 429 emits `h5: null` deliberately (unknown ≠ zero), but a plain
 * transport error emits nothing at all, so the store keeps the last successful reading — and a stale
 * "12%" would send the session onto a profile that has been exhausted since. Age is the guard.
 *
 * Pure + deterministic → unit-tested; the switch I/O (kill + respawn) lives in SessionsTab.
 */
export function pickResumeCandidate(
  current: string,
  profiles: ProfileInfo[],
  limits: Record<string, LimitsStatusEvent & { receivedAt?: number }>,
  exclude?: ReadonlySet<string>,
  now: number = Date.now()
): string | null {
  const eligible = profiles
    .filter(
      (p) =>
        p.name !== current &&
        p.exists &&
        p.credentialsPresent &&
        p.credentialsValid !== false &&
        !p.needsOnboarding &&
        p.linksIntact
    )
    // L11: skip a profile already claimed by an earlier pane in the same auto-continue pass, so two
    // panes limited in the same tick don't both pile onto the one free profile (defeating balancing).
    .filter((p) => !exclude?.has(p.name))
    .map((p) => {
      const l = limits[p.name];
      // `receivedAt` is absent only for a store filled before this field existed — treat that as fresh
      // rather than silently disabling auto-switch on first launch after an update.
      const fresh = l != null && now - (l.receivedAt ?? now) <= LIMITS_STALE_MS;
      const h5 = fresh ? (l?.h5 ?? null) : null;
      // A model-scoped weekly cap (limits[] `scoped`) gates real work even when 5h is calm — a
      // profile whose Opus/Fable week is exhausted is not a resume destination. Absent = 0 (no cap).
      const util = h5 == null ? null : Math.max(h5, l?.scoped ?? 0);
      return { name: p.name, util };
    })
    .filter((p): p is { name: string; util: number } => p.util != null && p.util < 85)
    .sort((a, b) => a.util - b.util);
  return eligible.length ? eligible[0].name : null;
}
