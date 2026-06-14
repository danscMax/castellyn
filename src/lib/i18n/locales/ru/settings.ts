export default {
  title: 'Настройки',

  // Flash messages
  savedPath: 'Путь сохранён',
  savedTimeouts: 'Таймауты сохранены',
  autostartOn: 'Автозапуск включён',
  autostartOff: 'Автозапуск выключен',
  saved: 'Сохранено',

  // Theme
  theme: 'Тема',
  themeDesc: 'Оформление интерфейса',
  themeDark: 'Тёмная',
  themeLight: 'Светлая',
  themeDarkTip: 'Тёмная тема',
  themeLightTip: 'Светлая тема',

  // Language
  language: 'Язык',
  languageDesc: 'Язык интерфейса',
  languageTip: 'Переключить язык интерфейса — применяется сразу, без перезапуска',

  // Scripts root
  scriptsRoot: 'Путь к скриптам (SCRIPTS_ROOT)',
  scriptsRootDesc:
    'Где лежат скрипты обслуживания. Пусто = по умолчанию E:\\Scripts (или переменная окружения SCRIPTS_ROOT).',
  scriptsRootInputTip: 'Абсолютный путь к корню скриптов',
  savePathTip: 'Сохранить путь',
  currentlyUsed: 'Сейчас используется: {path}',

  // Launch
  launch: 'Запуск',
  startWithWindows: 'Запускать при входе в Windows',
  startWithWindowsDesc: 'Реестр HKCU\\…\\Run; указывает на этот exe',
  startWithWindowsTip: 'Автозапуск с Windows',
  startHidden: 'Стартовать свёрнутым в трей',
  startHiddenDesc: 'Окно не показывается при запуске, висит в трее',
  startHiddenTip: 'Старт в трее',

  // Timeouts
  timeouts: 'Таймауты (форки)',
  timeoutsDesc: 'Для медленных сетей. Пусто = значения по умолчанию скрипта.',
  fetchTimeout: 'git fetch, сек',
  fetchTimeoutTip: 'Таймаут git fetch',
  ghTimeout: 'gh запросы, сек',
  ghTimeoutTip: 'Таймаут запросов gh',
  saveTimeoutsTip: 'Сохранить таймауты',

  // About
  about: 'О программе',
  version: 'Версия',
  scripts: 'Скрипты',
  config: 'Конфиг',
  app: 'Приложение',
  openScriptsFolder: 'Открыть папку скриптов',
  openScriptsFolderTip: 'Открыть папку скриптов в Проводнике'
};
