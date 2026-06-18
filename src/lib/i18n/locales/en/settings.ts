export default {
  title: 'Settings',
  searchPlaceholder: 'Search settings…',
  confirmDestructive: 'Confirm destructive actions',
  confirmDestructiveDesc: 'Ask before delete/reset (restore and reinstall always confirm)',

  // Flash messages
  savedPath: 'Path saved',
  savedTimeouts: 'Timeouts saved',
  autostartOn: 'Autostart enabled',
  autostartOff: 'Autostart disabled',
  saved: 'Saved',

  // View
  view: 'View',
  density: 'Density',
  densityComfortable: 'Comfortable',
  densityCompact: 'Compact',
  fullWidth: 'Full-width content',
  fullWidthDesc: 'Don’t cap content at 1600px (for wide screens)',
  termScrollback: 'Terminal scrollback',
  termScrollbackDesc: 'How many output lines each session pane keeps (1000–50000)',
  termScrollbackTip: 'Applies to newly opened panes; more lines use more memory',
  // Theme
  theme: 'Theme',
  themeDesc: 'Interface appearance',
  themeDark: 'Dark',
  themeLight: 'Light',
  themeDarkTip: 'Dark theme',
  themeLightTip: 'Light theme',
  themeSystem: 'System',
  themeSystemTip: 'Follow the OS theme',
  resetView: 'Reset view',
  resetViewTip: 'Restore density and width to defaults',

  // Language
  language: 'Language',
  languageDesc: 'Interface language',
  languageTip: 'Switch the interface language — applied instantly, no restart',

  // Scripts root
  scriptsRoot: 'Scripts path (SCRIPTS_ROOT)',
  scriptsRootDesc:
    'Folder holding the maintenance PowerShell scripts Castellyn runs (updates, forks, backup, etc.). This is NOT the sessions working folder — the Sessions "Default folder" is set separately. Empty = E:\\Scripts (or the SCRIPTS_ROOT environment variable).',
  scriptsRootInputTip: 'Absolute path to the scripts root',
  savePathTip: 'Save path',
  currentlyUsed: 'Currently used: {path}',

  // Launch
  launch: 'Launch',
  startWithWindows: 'Start on Windows sign-in',
  startWithWindowsDesc: 'HKCU\\…\\Run registry key; points to this exe',
  startWithWindowsTip: 'Start with Windows',
  startHidden: 'Start minimized to tray',
  startHiddenDesc: 'The window stays hidden at startup, living in the tray',
  startHiddenTip: 'Start in tray',
  closeToTray: 'Close to tray',
  closeToTrayDesc: 'The ✕ button hides the window to the tray. Turn off to make ✕ quit the app.',
  closeToTrayTip: 'Behavior of the window close button',
  toggleHotkey: 'Global show/hide hotkey',
  toggleHotkeyDesc: 'A system-wide combo to show/hide the window from anywhere. Empty = off.',
  toggleHotkeyTip: 'e.g. CommandOrControl+Shift+H',
  toggleHotkeyPlaceholder: 'CommandOrControl+Shift+H',
  toggleHotkeyError: 'Could not register the hotkey',

  // Timeouts
  timeouts: 'Timeouts (forks)',
  timeoutsDesc: 'For slow networks. Empty = the script’s default values.',
  fetchTimeout: 'git fetch, sec',
  fetchTimeoutTip: 'git fetch timeout',
  ghTimeout: 'gh requests, sec',
  ghTimeoutTip: 'gh request timeout',
  saveTimeoutsTip: 'Save timeouts',

  // About
  about: 'About',
  version: 'Version',
  scripts: 'Scripts',
  config: 'Config',
  app: 'Application',
  openScriptsFolder: 'Open scripts folder',
  openScriptsFolderTip: 'Open the scripts folder in Explorer',
  openConfigFile: 'Open config.json',
  openConfigFileTip: 'Open the Castellyn settings file',
  openStackFile: 'Open stack.json',
  openStackFileTip: 'Open the LLM stack config',
  backupSection: 'Settings backup',
  backupSectionDesc: 'Export/import Castellyn settings (config.json) — to move to another PC',
  exportConfig: 'Export settings',
  exportTip: 'Save the current settings to a file',
  importConfig: 'Import settings',
  importTip: 'Load settings from a file (applied immediately)',
  configExported: 'Settings exported',
  configImported: 'Settings imported'
};
