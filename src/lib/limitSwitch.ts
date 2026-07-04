import type { ProfileInfo, LimitsStatusEvent } from './ipc';

/**
 * #21e: pick the profile to resume a rate-limited conversation under.
 *
 * Eligible = a DIFFERENT profile that (a) exists, (b) has OAuth credentials, (c) has intact shared
 * links — so `--resume <id>` reaches the shared ~/.claude/projects transcript — and (d) has a KNOWN
 * 5h utilisation below 85%. The least-utilised eligible profile wins; `null` when none qualify
 * (caller then falls back to "wait"). Utilisation must be known: a profile with no limits-status
 * datapoint is excluded rather than switched to blindly.
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
    .filter((p) => p.name !== current && p.exists && p.credentialsPresent && p.linksIntact)
    // L11: skip a profile already claimed by an earlier pane in the same auto-continue pass, so two
    // panes limited in the same tick don't both pile onto the one free profile (defeating balancing).
    .filter((p) => !exclude?.has(p.name))
    .map((p) => ({ name: p.name, h5: limits[p.name]?.h5 ?? null }))
    .filter((p): p is { name: string; h5: number } => p.h5 != null && p.h5 < 85)
    .sort((a, b) => a.h5 - b.h5);
  return eligible.length ? eligible[0].name : null;
}
