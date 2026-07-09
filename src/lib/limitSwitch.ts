import type { ProfileInfo, LimitsStatusEvent } from './ipc';

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
 * Pure + deterministic → unit-tested; the switch I/O (kill + respawn) lives in SessionsTab.
 */
export function pickResumeCandidate(
  current: string,
  profiles: ProfileInfo[],
  limits: Record<string, LimitsStatusEvent>,
  exclude?: ReadonlySet<string>
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
    .map((p) => ({ name: p.name, h5: limits[p.name]?.h5 ?? null }))
    .filter((p): p is { name: string; h5: number } => p.h5 != null && p.h5 < 85)
    .sort((a, b) => a.h5 - b.h5);
  return eligible.length ? eligible[0].name : null;
}
