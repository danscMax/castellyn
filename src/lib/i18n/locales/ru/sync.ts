export default {
  // Header
  title: 'Синхронизация между компьютерами',
  subtitle:
    'Держит настройки Claude Code одинаковыми на всех твоих ПК: история, сессии (/resume), скиллы, агенты, команды и горячие клавиши автоматически копируются через Syncthing (P2P, без облака). Ниже — что именно синхронизировать.',
  refreshTitle: 'Перечитать статус Syncthing и настройки синхронизации',
  refreshing: 'Идёт…',
  refresh: 'Обновить',

  // Byte units (KB/MB/… ladder), comma-joined so t() returns a string.
  byteUnits: 'Б,КБ,МБ,ГБ,ТБ',

  // Syncthing state labels
  stateIdle: 'в норме (idle)',
  stateSyncing: 'синхронизация…',
  stateScanning: 'сканирование…',
  stateError: 'ошибка',

  // Syncthing status card
  syncthing: 'Syncthing',
  daemonTitle: 'Демон Syncthing доступен по локальному REST',
  connected: 'подключён',
  notFoundTitle: 'Syncthing не найден или не запущен — синхронизация не активна на этой машине',
  notFound: 'не найден',
  folder: 'Папка',
  folderIdTitle: 'ID папки: {id}',
  state: 'Состояние',
  completion: 'Готовность',
  completionTitle: 'Доля синхронизированных данных',
  connectedDevices: 'Других машин на связи',
  connectedDevicesTitle:
    'Сколько ДРУГИХ машин Syncthing сейчас видит на связи. Само это устройство не считается, поэтому одна вторая машина = 1.',
  folderNotShared: 'Папка ~/.claude не добавлена в Syncthing на этой машине.',
  noSyncthingYet: 'Настройки ниже сохранятся в .stignore и применятся, как только Syncthing появится.',

  // Drift warning
  needsApplyBadge: 'требует применения',
  driftWarning: 'Развёрнутый .stignore не совпадает с настройками ниже — нажмите «Применить».',

  // Item toggles
  whatToSync: 'Что синхронизировать',
  itemTitle: 'Включает строку «{path}» в whitelist .stignore',
  itemToggleTip: 'Вкл/выкл синхронизацию этого элемента между машинами; локальные файлы не трогаются',
  applyTitle:
    'Сохранить выбор в sync-config.json, перегенерировать .stignore и запросить пересканирование Syncthing',
  apply: 'Применить',
  unsavedChanges: 'есть несохранённые изменения',
  allApplied: 'всё применено',
  footnote:
    'Отключение пункта только перестаёт синхронизировать его между машинами — локальные файлы не удаляются. Секреты, settings.json и кэш плагинов не синхронизируются никогда.',

  // Items
  itemHistoryLabel: 'История команд',
  itemHistoryDesc: 'Список введённых команд',
  itemProjectsLabel: 'Сессии и память',
  itemProjectsDesc: 'Сессии (/resume) и нативная память проектов',
  itemSkillsLabel: 'Скиллы',
  itemSkillsDesc: 'Личные скиллы',
  itemAgentsLabel: 'Агенты',
  itemAgentsDesc: 'Кастомные сабагенты',
  itemCommandsLabel: 'Команды',
  itemCommandsDesc: 'Слэш-команды',
  itemKeybindingsLabel: 'Горячие клавиши',
  itemKeybindingsDesc: 'Раскладка клавиш',

  // Empty state
  emptyTitle: 'Нет данных',
  emptyHint: 'Нажми «Обновить», чтобы собрать статус синхронизации.'
};
