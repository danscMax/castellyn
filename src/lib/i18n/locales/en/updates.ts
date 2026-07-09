export default {
  // ── UpdatesTab: header ──
  title: 'Updates',
  subtitle: 'Check and apply updates for the whole Claude Code stack',
  groupHasUpdate: 'Update available ({count})',
  groupUpToDate: 'Up to date ({count})',
  groupHeld: 'Held ({count})',
  groupErrors: 'With errors ({count})',
  groupAllClear: 'all up to date',
  staleOldest: 'oldest check: {time}',
  statusCorrupt: 'status corrupt',
  summaryChecked: 'checked {time}',
  updatingNow: 'now: {step}',
  checkAllBtn: 'Check all',
  updateAllBtn: 'Update all',

  // ── ComponentCard: forks summary ──
  forkConflicts: '{count} with conflicts',
  forkToDelete: '{count} to delete',
  forkOpenPr: '{count} open PRs',
  forkAllSynced: 'all in sync',

  // ── ComponentCard: health badges ──
  healthNoStatus: 'no status',
  healthNoData: 'no data',
  healthFailedCount: '{count} failed',
  healthError: 'error',
  healthHeld: 'on hold',
  healthNeedsAttentionOne: '{count} needs attention',
  healthNeedsAttentionMany: '{count} need attention',
  healthUpToDate: 'up to date',
  healthUnknown: 'unknown status “{status}”',

  // ── ComponentCard: details ──
  lastRun: 'Last run',
  duration: 'Duration',
  durationSeconds: '{count} s',

  // ── ComponentCard: actions ──
  checkTip: 'Check for updates (read-only, installs nothing)',
  checking: 'Working…',
  checkBtn: 'Check',
  openForksBtn: 'Open Forks',
  openForksTip: 'Go to the “Forks” tab — actions per repository live there',
  openPluginsBtn: 'Open Extensions',
  openPluginsTip: 'Go to the “Extensions” tab — plugins, skills and agents live there',
  updateBtn: 'Update',
  updateBtnCount: 'Update ({count})',
  updateTip: 'Install the available updates for this component (with confirmation)',
  applyBtn: 'Apply',
  applyTip:
    'Run the update (status not checked yet — click “Check” to find out whether there is anything to update)',
  upToDate: 'no updates',
  upToDateTip: 'No updates — everything is up to date'
};
