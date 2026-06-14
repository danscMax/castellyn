// Pure helpers computing per-tab "needs attention" indicators for the sidebar.
// Kept side-effect-free and import-light so they're unit-testable.

import type { Component, ForkStatus, BackupList, ProfilesStatus, SyncStatus } from './ipc';

export type Attention = { level: 'info' | 'warn'; count?: number };

/** Sum of available updates across update components. */
export function updatesAttention(
  components: Component[],
  statuses: Record<string, any>
): Attention | null {
  let changed = 0;
  for (const c of components) {
    const n = statuses?.[c.id]?.counts?.changed;
    if (typeof n === 'number') changed += n;
  }
  return changed > 0 ? { level: 'info', count: changed } : null;
}

/** Forks needing manual work. */
export function forksAttention(s: ForkStatus | null | undefined): Attention | null {
  const n = s?.summary?.needHands ?? 0;
  return n > 0 ? { level: 'warn', count: n } : null;
}

/** Backup older than 2 days (matches Test-Installation thresholds). */
export function backupAttention(data: BackupList | null | undefined): Attention | null {
  const last = data?.state?.lastRun;
  if (!last) return null;
  const days = (Date.now() - new Date(last).getTime()) / 86_400_000;
  if (Number.isNaN(days)) return null;
  return days > 2 ? { level: 'warn' } : null;
}

/** Profiles with broken links / missing dirs / sync conflicts. */
export function profilesAttention(data: ProfilesStatus | null | undefined): Attention | null {
  if (!data?.profiles) return null;
  const broken = data.profiles.filter((p) => p.exists && !p.linksIntact).length;
  const missing = data.profiles.filter((p) => !p.exists).length;
  const conflicts = (data.syncConflicts?.count ?? 0) > 0 ? 1 : 0;
  const total = broken + missing + conflicts;
  return total > 0 ? { level: 'warn', count: broken + missing } : null;
}

/** Plugins with an available update. */
export function pluginsAttention(updateCount: number): Attention | null {
  return updateCount > 0 ? { level: 'info', count: updateCount } : null;
}

/** Deployed .stignore drifted from the configured sync whitelist. */
export function syncAttention(data: SyncStatus | null | undefined): Attention | null {
  if (!data || !data.stignoreExists) return null;
  return data.stignoreMatches === false ? { level: 'warn' } : null;
}
