// Pure helpers computing per-tab "needs attention" indicators for the sidebar.
// Kept side-effect-free and import-light so they're unit-testable.

import type { Component, ForkStatus, BackupList, ProfilesStatus, SyncStatus, StackDriftItem } from './ipc';

export type Attention = { level: 'info' | 'warn' | 'danger' | 'done'; count?: number };

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

/** A profile has a broken shared link when a folder it shares is MISSING its link (status null).
 *  A folder holding real data ("none") or a present link is NOT broken. Single source of truth,
 *  shared with the Profiles tab so the sidebar badge and the recommendations card never disagree. */
export function profileHasMissingLink(p: { sharedLinks?: Record<string, string | null> | null }): boolean {
  return !!p.sharedLinks && Object.values(p.sharedLinks).some((s) => s === null);
}

/** Profiles with broken links / missing dirs / sync conflicts. */
export function profilesAttention(data: ProfilesStatus | null | undefined): Attention | null {
  if (!data?.profiles) return null;
  const broken = data.profiles.filter((p) => p.exists && profileHasMissingLink(p)).length;
  const missing = data.profiles.filter((p) => !p.exists).length;
  const conflicts = (data.syncConflicts?.count ?? 0) > 0 ? 1 : 0;
  const total = broken + missing + conflicts;
  // count must match `total` — otherwise a repo with ONLY sync conflicts shows a "0" badge.
  return total > 0 ? { level: 'warn', count: total } : null;
}

/** Plugins with an available update. */
export function pluginsAttention(updateCount: number): Attention | null {
  return updateCount > 0 ? { level: 'info', count: updateCount } : null;
}

/**
 * Sessions: agents waiting for input, or finished-but-unseen. #10: use the SAME herdr palette the
 * pane already uses (blocked = danger/red "waiting for you", done = teal "finished, unseen") so one
 * fact is one colour across the sidebar and the tab — previously the sidebar showed warn/info.
 */
export function sessionsAttention(s: { blocked: number; done: number }): Attention | null {
  if (s.blocked > 0) return { level: 'danger', count: s.blocked };
  if (s.done > 0) return { level: 'done', count: s.done };
  return null;
}

/** Ф1: stack ownership drift surfaced on Home. Any non-ok item badges Home; an `error` item
 *  (a check that itself failed) escalates the level to danger, otherwise warn. */
export function stackDriftAttention(items: StackDriftItem[] | null | undefined): Attention | null {
  if (!items) return null;
  const bad = items.filter((i) => i.state !== 'ok');
  if (!bad.length) return null;
  return { level: bad.some((i) => i.state === 'error') ? 'danger' : 'warn', count: bad.length };
}

/** Deployed .stignore drifted from the configured sync whitelist. */
export function syncAttention(data: SyncStatus | null | undefined): Attention | null {
  if (!data || !data.stignoreExists) return null;
  return data.stignoreMatches === false ? { level: 'warn' } : null;
}
