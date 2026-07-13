import type { ProfileInfo, LimitsStatusEvent } from './ipc';

/** A datapoint older than this is not evidence of anything: the backend polls every 300 s, so two
 *  missed rounds mean the network (or the endpoint) has been failing and the numbers are guesses. */
export const LIMITS_STALE_MS = 660_000; // 2 poll intervals + a minute of slack

/** Utilisation at/above this (%) gates a profile out of BOTH the resume auto-switch and the launch
 *  recommendation — the shared "too hot to hand work to" line. */
export const UTIL_THRESHOLD = 85;

/** Per-profile usage store: the last `limits-status` event plus when the UI received it (freshness). */
export type LimitsMap = Record<string, LimitsStatusEvent & { receivedAt?: number }>;

/** Why a profile is not a candidate — structured so the launch advisor can explain it in the UI
 *  (the resume switch discards it and only keeps the winner). First failing check wins. */
export type RejectReason =
  | 'claimed'
  | 'missing'
  | 'no-credentials'
  | 'invalid-credentials'
  | 'needs-onboarding'
  | 'broken-links'
  | 'usage-unknown'
  | 'over-threshold';

export type EligibleProfile = { name: string; util: number };
export type RejectedProfile = { name: string; reason: RejectReason };

/**
 * Shared eligibility + utilisation scoring behind both `pickResumeCandidate` (resume a rate-limited
 * conversation elsewhere) and `launchAdvisor` (recommend a profile for a fresh session). Pure &
 * deterministic → unit-tested; keeping ONE ranker means the two features can never drift apart.
 *
 * A profile is eligible only if it (a) exists, (b) has USABLE OAuth credentials — present, not
 * positively dead (`credentialsValid !== false`), onboarding not stranded by `/logout` — (c) has
 * intact shared links so `--resume`/shared transcripts reach it, (d) is not in `exclude` (a profile
 * already claimed this tick, or the current one for a resume), and (e) has a KNOWN, RECENT
 * utilisation below `UTIL_THRESHOLD` — unless `extraEnabled` pay-as-you-go credits keep it working
 * past the plan cap. `util = max(5h, model-scoped weekly)` — an exhausted model week gates a calm 5h.
 * Unknown usage (null / stale / 429 / no datapoint) is a rejection, never zero.
 *
 * Both optional health fields are absent in older `profiles.last.json` snapshots, so absence means
 * "don't know" and stays eligible — only a positive `false` disqualifies.
 *
 * Returns eligibles least-utilised first (deterministic tie-break by name) plus every rejection with
 * its reason, so a caller can pick `eligible[0]` or explain an empty result.
 */
export function evaluateProfiles(
  profiles: ProfileInfo[],
  limits: LimitsMap,
  exclude: ReadonlySet<string>,
  now: number = Date.now()
): { eligible: EligibleProfile[]; rejected: RejectedProfile[] } {
  const eligible: EligibleProfile[] = [];
  const rejected: RejectedProfile[] = [];
  for (const p of profiles) {
    let reason: RejectReason | null = null;
    if (exclude.has(p.name)) reason = 'claimed';
    else if (!p.exists) reason = 'missing';
    else if (!p.credentialsPresent) reason = 'no-credentials';
    else if (p.credentialsValid === false) reason = 'invalid-credentials';
    else if (p.needsOnboarding) reason = 'needs-onboarding';
    else if (!p.linksIntact) reason = 'broken-links';
    if (reason) {
      rejected.push({ name: p.name, reason });
      continue;
    }

    const l = limits[p.name];
    // `receivedAt` is absent only for a store filled before this field existed — treat that as fresh
    // rather than silently disabling the feature on the first launch after an update.
    const fresh = l != null && now - (l.receivedAt ?? now) <= LIMITS_STALE_MS;
    const h5 = fresh ? (l?.h5 ?? null) : null;
    if (h5 == null) {
      rejected.push({ name: p.name, reason: 'usage-unknown' });
      continue;
    }
    // A model-scoped weekly cap gates real work even when 5h is calm. Absent = 0 (no cap).
    const util = Math.max(h5, l?.scoped ?? 0);
    // extra_usage (pay-as-you-go) keeps the profile working past the plan cap — but only WHILE its
    // credits remain (extraPct < 100). An exhausted extra-cap (extraPct == 100) is over-threshold like
    // any other; unknown extraPct (null) is treated as "credit remains" so we don't over-block.
    const extraCovers = !!l?.extraEnabled && (l?.extraPct ?? 0) < 100;
    if (util >= UTIL_THRESHOLD && !extraCovers) {
      rejected.push({ name: p.name, reason: 'over-threshold' });
      continue;
    }
    eligible.push({ name: p.name, util });
  }
  eligible.sort((a, b) => a.util - b.util || a.name.localeCompare(b.name));
  return { eligible, rejected };
}

/**
 * #21e: pick the profile to resume a rate-limited conversation under. The least-utilised eligible
 * profile OTHER than `current` (and other than any already `exclude`d this pass, so two panes limited
 * in the same tick don't both pile onto the one free profile). `null` when none qualify → the caller
 * falls back to "wait". Thin wrapper over the shared `evaluateProfiles`; the switch I/O (kill +
 * respawn) lives in SessionsTab.
 */
export function pickResumeCandidate(
  current: string,
  profiles: ProfileInfo[],
  limits: LimitsMap,
  exclude?: ReadonlySet<string>,
  now: number = Date.now()
): string | null {
  const excludeAll = new Set<string>([current, ...(exclude ?? [])]);
  const { eligible } = evaluateProfiles(profiles, limits, excludeAll, now);
  return eligible.length ? eligible[0].name : null;
}
