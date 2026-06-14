export default {
  title: 'Settings',

  // Flash messages
  savedPath: 'Path saved',
  savedTimeouts: 'Timeouts saved',
  autostartOn: 'Autostart enabled',
  autostartOff: 'Autostart disabled',
  saved: 'Saved',

  // Theme
  theme: 'Theme',
  themeDesc: 'Interface appearance',
  themeDark: 'Dark',
  themeLight: 'Light',
  themeDarkTip: 'Dark theme',
  themeLightTip: 'Light theme',

  // Language
  language: 'Language',
  languageDesc: 'Interface language',
  languageTip: 'Switch the interface language — applied instantly, no restart',

  // Scripts root
  scriptsRoot: 'Scripts path (SCRIPTS_ROOT)',
  scriptsRootDesc:
    'Where the maintenance scripts live. Empty = default E:\\Scripts (or the SCRIPTS_ROOT environment variable).',
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
  openScriptsFolderTip: 'Open the scripts folder in Explorer'
};
