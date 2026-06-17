export default {
  // Header
  title: 'Backup',
  subtitle: 'Snapshots of Claude Code profile configs',
  createTitle: 'Create a new snapshot of all profile configs now',
  makeBackup: 'Make backup',
  retention: 'Keep:',
  retentionTip: 'How many recent snapshots to keep (older ones pruned on backup)',

  // Freshness badge
  fresh: 'fresh',
  staling: 'getting stale',
  stale: 'stale',
  relToday: 'today',
  relYesterday: 'yesterday',
  relDaysAgo: '{n} days ago',

  // Status card
  lastBackup: 'Last backup',
  lastSnapshot: 'Last snapshot',
  snapshotsWeekly: 'Snapshots / weekly',
  weeklyArchive: 'Weekly archive',

  // Snapshots list
  snapshotsHeading: 'Snapshots ({n})',
  latest: 'latest',
  restoreItemTitle: 'Restore configs from this snapshot (shows a preview first)',
  restore: 'Restore',
  emptyTitle: 'No snapshots',
  emptyHint: 'Press “Make backup” to create the first one.',

  // Restore dialog
  dialogTitle: 'Restore from snapshot',
  profiles: 'Profiles',
  profileToggleTip: 'Include this profile in the restore; an unchecked profile is left untouched',
  includeCreds: 'Restore credentials (won’t overwrite existing)',
  includeCredsTip: 'Fill in missing credentials from the snapshot; existing tokens are left untouched',
  warn: 'A real restore overwrites the live configs of the selected profiles — irreversible. Preview the plan first.',
  closeTitle: 'Close without changes',
  previewTitle: 'Preview (-WhatIf): show what would be overwritten — changes nothing',
  showPlan: 'Show plan',
  restoreTitle: 'Restore the selected profiles from the snapshot (irreversible)',
  restoreNeedsPreview: 'Press “Show plan” first'
};
