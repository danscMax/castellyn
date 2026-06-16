export default {
  // Header
  title: 'Profiles',
  health: 'Health of {n} Claude Code {profiles}',
  checkTip: 'Rebuild profile status (read-only)',
  checking: 'Working…',
  addProfile: '+ Add profile',
  addProfileTip: 'Create a new ~/.claude-<name> profile and link the shared folders',
  reinstall: 'Reinstall',
  reinstallTip:
    'Reinstall profiles (Install -Force): recreates junctions/symlinks. Requires administrator rights (UAC)',

  // Recommendations
  recommendations: 'Recommendations',
  brokenLinks: '{n} {profiles} have broken shared links.',
  repairName: 'Repair {name}',
  repairNameTip: 'Repair the “{name}” profile links without a full reinstall',
  missingDirs: 'Missing profile directories: {names}.',
  create: 'Create',
  createTip: 'Reinstall profiles — will create the missing ones',
  syncConflicts: 'Syncthing sync conflicts: {n} (duplicate *.sync-conflict-* files).',
  cleanConflicts: 'Delete duplicates',
  cleanConflictsTip:
    'Delete conflict-duplicate files (originals are untouched; Syncthing keeps versions)',
  allGood: 'all good',
  allGoodHint: 'All profiles present, links intact, no conflicts.',

  // Card header
  colorDot: 'Profile color',

  // Status row
  noDir: 'no directory',
  noDirTip: 'Directory ~/.claude-{name} not found — profile not installed',
  loggedIn: 'logged in ✓',
  loggedInTip: 'Saved login (.credentials.json)',
  noLogin: 'not logged in',
  noLoginTip: 'No saved login — authorization required',
  lean: 'lean',
  leanTip: 'Launches in lean mode ({flag}) — trimmed system prompt',
  links: 'folders {linked}/{total}',
  linksTip: 'Connected {linked} of {total} shared folders (junction/symlink). Click to expand details.',

  // Provider section
  providerLabel: 'Provider:',
  providerDefault: 'Anthropic (default)',
  providerStdTip: 'Standard Anthropic login',
  providerEdit: 'change',
  providerOpenTip: 'Open the Providers tab — configure this provider and its keys there',
  providerEditTip:
    'Assign/change this profile’s LLM provider (LM Studio, router, custom)',
  providerClear: 'reset',
  providerClearTip: 'Reset to the standard Anthropic login',

  // Link kinds / tips
  linkJunction: 'junction',
  linkSymlink: 'symlink',
  linkHardlink: 'hardlink',
  linkNotLink: 'not a link',
  linkNone: 'none',
  linkTipOk: '“{folder}” is shared across all profiles via {kind} — this is normal',
  linkTipNone: '“{folder}” exists but is NOT linked (a copy) — shared content won’t sync',
  linkTipMissing:
    '“{folder}” is missing — shared content unavailable; “Reinstall profiles” helps',

  // Shared-folders editor
  sharedFolders: 'Profile shared folders',
  sharedFoldersTip:
    'Which shared folders to link for this profile (junction/symlink to ~/.claude)',
  applyLinks: 'Apply',
  applyLinksTip: 'Save the selection and recreate the profile links',
  linksCancelTip: 'Close the editor without changing the profile’s current links',
  cancel: 'Cancel',

  // Main actions
  launch: 'Launch',
  launchTip:
    'Open a terminal with claude running under this profile (CLAUDE_CONFIG_DIR=~/.claude-{name})',
  folder: 'Folder',
  folderTip: 'Open the profile directory ~/.claude-{name} in Explorer',

  // Empty state
  noData: 'No data',
  noDataHint: 'Press “Check” to gather the profile status.',

  // Overflow menu
  menuTitle: 'Profile actions',
  menuTools: 'Tools / size',
  menuToolsTip: 'Lean mode, MCP/CLAUDE.md selection and system-prompt size measurement',
  menuRepair: 'Repair links',
  menuRepairTip: 'Recreate broken/missing junctions/symlinks without a full reinstall',
  menuResetProvider: 'Reset provider',
  menuResetProviderTip: 'Revert this profile to the Anthropic default (remove the custom provider)',
  menuSharedFolders: 'Shared folders…',
  menuSharedFoldersTip: 'Choose which shared folders to link for this profile',
  menuColor: 'Color…',
  menuColorTip: 'Change the profile color',
  menuRename: 'Rename…',
  menuRenameTip: 'Rename the profile and its directory',
  menuDelete: 'Delete',
  menuDeleteTip: 'Delete the profile and the ~/.claude-{name} directory',

  // ProfileEditDialog
  dlgClose: 'Close',
  dlgAddTitle: 'Add profile',
  dlgRenameTitle: 'Rename “{name}”',
  dlgRecolorTitle: 'Color of “{name}”',
  dlgNewName: 'New name',
  dlgName: 'Profile name',
  dlgNamePlaceholder: 'e.g. cc6',
  dlgNameTip: 'Becomes part of the ~/.claude-<name> directory; on rename the directory is renamed too',
  dlgNameError: 'Letters/digits/_/-, starts with a letter or digit, up to 32 characters',
  dlgColor: 'Color',
  dlgDescription: 'Description (optional)',
  dlgDescriptionPlaceholder: 'e.g. Tests',
  dlgDescriptionTip: 'A note for yourself (e.g. what the profile is for); does not affect claude',
  dlgCancelTip: 'Close without changes — the profile won’t be created/renamed/recolored',
  dlgSubmitTip: 'Confirm: create the profile, or apply the new name/color',
  dlgCancel: 'Cancel',
  dlgAdd: 'Add',
  dlgRename: 'Rename',
  dlgApply: 'Apply',

  // LaunchConfigDialog
  lcClose: 'Close',
  lcTitle: 'Tools and size · profile “{name}”',
  lcLeanToggle: 'Lean mode',
  lcLeanHeading: 'Lean mode (less context)',
  lcLeanDesc:
    'Trims the system prompt via launch flags (reversible, without editing the global managed file). Useful for local models — fewer tokens, faster, more accurate tool calls.',
  lcBareNote:
    '{bare} will be applied (≈1k tokens): no plugins/hooks/LSP/auto-memory. Hooks (incl. RTK) won’t work. Pick below what to bring back.',
  lcSafeModeNote:
    'The profile has no token provider (OAuth) → {safeMode} will be applied (≈28k): disables plugins/MCP/skills. Granular MCP/CLAUDE.md selection is unavailable here.',
  lcMcpLabel: 'MCP servers (what to connect in lean mode)',
  lcMcpEmpty: 'MCP list is empty (config\\.mcp.json).',
  lcClaudeMdToggle: 'CLAUDE.md',
  lcClaudeMd: 'Attach the profile’s CLAUDE.md (--add-dir)',
  lcSizeLabel: 'System prompt size · measure:',
  lcMeasuring: 'Measuring…',
  lcMeasureLean: 'Lean',
  lcMeasureLeanTip: 'Save the selection and measure the lean set (fast)',
  lcMeasureFull: 'Full (slow)',
  lcMeasureFullTip: 'Measure full mode (slow on a local model — tens of seconds)',
  lcLeanResult: 'Lean: ',
  lcFullResult: 'Full: ',
  lcTokensUnit: 'tok.',
  lcMeasureNote:
    'Measuring runs {cmd} against the profile’s provider and reads usage.input_tokens.',
  lcCancelTip: 'Close without saving — the profile’s mode and tool set stay unchanged',
  lcApplyTip: 'Save the launch mode and selected tool set for this profile',
  lcCancel: 'Cancel',
  lcApply: 'Apply'
};
