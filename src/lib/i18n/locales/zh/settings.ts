export default {
  title: '设置',

  // Flash messages
  savedPath: '路径已保存',
  savedTimeouts: '超时已保存',
  autostartOn: '已启用开机自启',
  autostartOff: '已禁用开机自启',
  saved: '已保存',

  // Theme
  theme: '主题',
  themeDesc: '界面外观',
  themeDark: '深色',
  themeLight: '浅色',
  themeDarkTip: '深色主题',
  themeLightTip: '浅色主题',

  // Language
  language: '语言',
  languageDesc: '界面语言',
  languageTip: '切换界面语言——即时生效，无需重启',

  // Scripts root
  scriptsRoot: '脚本路径（SCRIPTS_ROOT）',
  scriptsRootDesc:
    '维护脚本所在位置。留空 = 默认 E:\\Scripts（或环境变量 SCRIPTS_ROOT）。',
  scriptsRootInputTip: '脚本根目录的绝对路径',
  savePathTip: '保存路径',
  currentlyUsed: '当前使用：{path}',

  // Launch
  launch: '启动',
  startWithWindows: '登录 Windows 时启动',
  startWithWindowsDesc: '注册表 HKCU\\…\\Run；指向此 exe',
  startWithWindowsTip: '随 Windows 启动',
  startHidden: '启动时最小化到托盘',
  startHiddenDesc: '启动时不显示窗口，常驻托盘',
  startHiddenTip: '在托盘中启动',

  // Timeouts
  timeouts: '超时（复刻）',
  timeoutsDesc: '用于慢速网络。留空 = 脚本默认值。',
  fetchTimeout: 'git fetch，秒',
  fetchTimeoutTip: 'git fetch 超时',
  ghTimeout: 'gh 请求，秒',
  ghTimeoutTip: 'gh 请求超时',
  saveTimeoutsTip: '保存超时',

  // About
  about: '关于',
  version: '版本',
  scripts: '脚本',
  config: '配置文件',
  app: '应用程序',
  openScriptsFolder: '打开脚本文件夹',
  openScriptsFolderTip: '在资源管理器中打开脚本文件夹'
};
