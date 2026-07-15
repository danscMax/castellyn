export default {
  // Wave-4 config-drift / time keys
  configDrift: 'Config files',
  configDriftDesc: 'Shared config (statusline, CLAUDE.md, RTK.md, hooks…) linked to the deploy source synced across machines.',
  driftedBadge: '{n} drifted',
  unlinkedBadge: '{n} not linked',
  configOk: 'in sync',
  checkedAt: 'checked {time}',
  driftCheckBtn: 'Re-check',
  driftCheckTip: 'Re-scan shared config files for drift',
  syncNowBtn: 'Sync now',
  syncNowTip: 'Mirror live config to the deploy source (backup)',
  relinkBtn: 'Repair links',
  relinkTip: 'Re-establish shared config symlinks (needs admin)',
  conflictsDesc: 'Syncthing conflict files detected.',
  fstateOk: 'ok',
  fstateLinked: 'linked',
  fstateMaster: 'source',
  fstateDrifted: 'drifted',
  fstateUnlinked: 'not linked',
  cleanConflictsBtn: 'Clean',
  cleanConflictsTip: 'Delete the *.sync-conflict-* files',
  conflictShow: 'Show…',
  keepLocal: 'Keep local',
  keepOther: 'Take other machine’s',
  stConfiguredButDown: 'configured but unreachable',
  memoTitle: 'What does NOT sync',
  memoBody:
    '• Secrets and keys — Credential Manager, settings*.json, .credentials.json (kept locally on each machine).\n• Plugins and marketplaces — installed separately on every PC.\n• OpenCode/Codex fan-outs and “Share skills” — redo on each machine.',
  // Header
  title: 'Sync across machines',
  subtitle:
    'Keeps Claude Code settings identical on all your PCs: history, sessions (/resume), skills, agents, commands and keybindings are copied automatically via Syncthing (P2P, no cloud). Below — exactly what to sync.',
  refreshTitle: 'Re-read Syncthing status and sync settings',
  refreshing: 'Working…',
  refresh: 'Refresh',

  // Byte units (KB/MB/… ladder), comma-joined so t() returns a string.
  byteUnits: 'B,KB,MB,GB,TB',

  // Syncthing state labels
  stateIdle: 'OK (idle)',
  stateSyncing: 'syncing…',
  stateScanning: 'scanning…',
  stateError: 'error',

  // Syncthing status card
  syncthing: 'Syncthing',
  daemonTitle: 'Syncthing daemon reachable over local REST',
  connected: 'connected',
  openWebUi: 'Web UI',
  openWebUiTip: 'Open the Syncthing web interface (localhost:8384)',
  notFoundTitle: 'Syncthing not found or not running — sync is not active on this machine',
  togglesInertNote: 'Syncthing is not running — these toggles have no effect yet: they apply once the daemon is up.',
  notFound: 'not found',
  folder: 'Folder',
  folderIdTitle: 'Folder ID: {id}',
  state: 'State',
  completion: 'Readiness',
  completionTitle: 'Share of synced data',
  connectedDevices: 'Other machines connected',
  connectedDevicesTitle:
    'How many OTHER machines Syncthing currently sees online. This device is not counted, so one second machine = 1.',
  folderNotShared: 'The ~/.claude folder is not added to Syncthing on this machine.',
  noSyncthingYet: 'The settings below are saved to .stignore and apply as soon as Syncthing appears.',

  // Drift warning
  needsApplyBadge: 'needs applying',
  driftWarning: 'The deployed .stignore does not match the settings below — click “Apply”.',

  // Item toggles
  whatToSync: 'What to sync',
  itemTitle: 'Includes the line “{path}” in the .stignore whitelist',
  itemToggleTip: 'Turn syncing of this item across machines on/off; local files are left untouched',
  applyTitle:
    'Save the selection to sync-config.json, regenerate .stignore and request a Syncthing rescan',
  apply: 'Apply',
  unsavedChanges: 'unsaved changes',
  allApplied: 'all applied',
  footnote:
    'Disabling an item only stops syncing it across machines — local files are not deleted. Secrets, settings.json and the plugin cache are never synced.',

  // Items
  itemHistoryLabel: 'Command history',
  itemHistoryDesc: 'List of entered commands',
  itemProjectsLabel: 'Sessions and memory',
  itemProjectsDesc: 'Sessions (/resume) and native project memory',
  itemSkillsLabel: 'Skills',
  itemSkillsDesc: 'Personal skills',
  itemAgentsLabel: 'Agents',
  itemAgentsDesc: 'Custom subagents',
  itemCommandsLabel: 'Commands',
  itemCommandsDesc: 'Slash commands',
  itemKeybindingsLabel: 'Keybindings',
  itemKeybindingsDesc: 'Key layout',
  itemCastellynLabel: 'Sessions settings',
  itemCastellynDesc: 'Saved sets, favorites, layout, folders, args',

  // Drift diff (Phase 3.2)
  showDiff: 'Show diff',
  hideDiff: 'Hide diff',
  diffTitle: 'Differences from shared copy',

  // Empty state
  emptyTitle: 'No data',
  emptyHint: 'Click “Refresh” to collect the sync status.'
};
