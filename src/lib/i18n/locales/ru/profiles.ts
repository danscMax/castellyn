export default {
  // Wave-4 config-drift / time keys
  checkedAt: 'проверено {time}',
  // Header
  title: 'Профили',
  health: 'Здоровье {n} {profiles} Claude Code',
  checkTip: 'Пересобрать статус профилей (только чтение)',
  checking: 'Идёт…',
  addProfile: '+ Добавить профиль',
  addProfileTip: 'Создать новый профиль ~/.claude-<имя> и подключить общие папки',
  reinstall: 'Переустановить',
  reinstallTip:
    "Переустановить профили (Install -Force): пересоздаёт junction'ы/симлинки. Требует прав администратора (UAC)",

  // Recommendations
  recommendations: 'Рекомендации',
  brokenLinks: 'Нарушены общие связи: {n} {profiles}.',
  repairName: 'Починить {name}',
  repairNameTip: 'Починить связи профиля «{name}» без полной переустановки',
  finishAdmin: 'Завершить {name} (админ)',
  finishAdminTip: 'У профиля «{name}» не хватает общих папок — их создание требует прав администратора',
  elevateTitle: 'Завершить профиль «{name}»',
  elevateMsg: 'Общие папки (agents, commands, hooks, plugins, skills) создаются как symlink и требуют прав администратора. Выберите способ.',
  elevateRepairOnce: 'Разово починить от администратора',
  elevateRepairOnceTip: 'Запросить права (UAC) и создать недостающие ссылки только для этого профиля',
  elevateRelaunch: 'Перезапустить приложение от администратора',
  elevateRelaunchTip: 'Перезапустить Castellyn с правами администратора — дальше всё работает без отдельных запросов UAC',
  missingDirs: 'Отсутствуют каталоги профилей: {names}.',
  create: 'Создать',
  createTip: 'Переустановить профили — создаст недостающие',
  syncConflicts: 'Sync-конфликты Syncthing: {n} (файлы-дубли *.sync-conflict-*).',
  cleanConflicts: 'Удалить дубли',
  cleanConflictsTip:
    'Удалить файлы-дубли конфликтов (оригиналы не тронутся; Syncthing хранит версии)',
  allGood: 'всё в порядке',
  allGoodHint: 'Все профили на месте, связи целы, конфликтов нет.',

  // Card header
  colorDot: 'Цвет профиля',
  colName: 'Профиль',
  colStatus: 'Статус',
  colUsage: 'Лимиты',
  colProvider: 'Провайдер',
  colLinks: 'Ссылки',
  colActions: 'Действия',
  searchPlaceholder: 'Поиск профиля…',

  // Status row
  noDir: 'нет каталога',
  noDirTip: 'Каталог ~/.claude-{name} не найден — профиль не установлен',
  loggedIn: 'вход ✓',
  loggedInTip: 'Сохранён вход (.credentials.json)',
  usage5h: '5ч:',
  usage7d: 'нед:',
  usageReset: 'сброс через {time}',
  usageTip: 'Остаток лимитов Claude Code: 5-часовое и недельное окно (процент свободно). Сброс недельного через указанное время.',
  noLogin: 'нет входа',
  noLoginTip: 'Нет сохранённого входа — потребуется авторизация',
  lean: 'лёгкий',
  leanTip: 'Запуск в лёгком режиме ({flag}) — урезанный системный промпт',
  links: 'папки {linked}/{total}',
  linksTip: 'Подключено {linked} из {total} общих папок (junction/symlink). Нажми, чтобы развернуть детали.',

  // Provider section
  providerLabel: 'Провайдер:',
  providerDefault: 'Anthropic (по умолчанию)',
  providerStdTip: 'Стандартный Anthropic-логин',
  providerEdit: 'изменить',
  providerOpenTip: 'Открыть вкладку «Провайдеры» — там настройка и ключи этого провайдера',
  providerEditTip:
    'Назначить/изменить LLM-провайдера этого профиля (LM Studio, роутер, кастомный)',
  providerClear: 'сбросить',
  providerClearTip: 'Сбросить на стандартный Anthropic-логин',

  // Link kinds / tips
  linkJunction: 'junction',
  linkSymlink: 'symlink',
  linkHardlink: 'hardlink',
  linkNotLink: 'не связь',
  linkNone: 'нет',
  linkTipOk: '«{folder}» общая для всех профилей через {kind} — это нормально',
  linkTipNone: '«{folder}» существует, но НЕ связана (копия) — общий контент не синхронизируется',
  linkTipMissing:
    '«{folder}» отсутствует — общий контент недоступен; помогает «Переустановить профили»',

  // Shared-folders editor
  sharedFolders: 'Общие папки профиля',
  sharedFoldersTip:
    'Какие общие папки связывать у этого профиля (junction/symlink на ~/.claude)',
  applyLinks: 'Применить',
  applyLinksTip: 'Сохранить выбор и пересоздать связи профиля',
  linksCancelTip: 'Закрыть редактор, не меняя текущие связи профиля',
  cancel: 'Отмена',

  // Main actions
  launch: 'Запустить',
  launchTip:
    'Открыть терминал с запущенным claude под этим профилем (CLAUDE_CONFIG_DIR=~/.claude-{name})',
  folder: 'Папка',
  folderTip: 'Открыть каталог профиля ~/.claude-{name} в Проводнике',

  // Empty state
  noData: 'Нет данных',
  noDataHint: 'Нажми «Проверить», чтобы собрать статус профилей.',

  // Overflow menu
  menuTitle: 'Действия с профилем',
  menuTools: 'Инструменты / размер',
  menuToolsTip:
    'Лёгкий режим, выбор MCP/CLAUDE.md и измерение размера системного промпта',
  menuViewConfig: 'Просмотр конфига',
  menuViewConfigTip: 'Открыть CLAUDE.md / settings.json профиля (только чтение)',
  viewSettings: 'settings.json',
  viewClaudeMd: 'CLAUDE.md',
  menuRepair: 'Починить связи',
  menuRepairTip:
    'Пересоздать битые/отсутствующие junction/symlink без полной переустановки',
  menuResetProvider: 'Сбросить провайдера',
  menuResetProviderTip: 'Вернуть профиль к Anthropic по умолчанию (убрать кастомный провайдер)',
  menuSharedFolders: 'Общие папки…',
  menuSharedFoldersTip: 'Выбрать, какие общие папки связывать у этого профиля',
  menuColor: 'Цвет…',
  menuColorTip: 'Изменить цвет профиля',
  menuRename: 'Переименовать…',
  menuRenameTip: 'Переименовать профиль и его каталог',
  menuDescribe: 'Изменить описание',
  menuDescribeTip: 'Задать понятную подпись профиля (например, «рабочий», «эксперименты»)',
  menuDelete: 'Удалить',
  menuDeleteTip: 'Удалить профиль и каталог ~/.claude-{name}',

  // ProfileEditDialog
  dlgClose: 'Закрыть',
  dlgAddTitle: 'Добавить профиль',
  dlgRenameTitle: 'Переименовать «{name}»',
  dlgRecolorTitle: 'Цвет профиля «{name}»',
  dlgRedescribeTitle: 'Описание профиля «{name}»',
  dlgNewName: 'Новое имя',
  dlgName: 'Имя профиля',
  dlgNamePlaceholder: 'например, cc6',
  dlgNameTip: 'Часть имени каталога ~/.claude-<имя>; для переименования каталог будет переименован',
  dlgNameError: 'Буквы/цифры/_/-, начинается с буквы или цифры, до 32 символов',
  dlgColor: 'Цвет',
  dlgDescription: 'Описание (необязательно)',
  dlgDescriptionPlaceholder: 'например, Тесты',
  dlgDescriptionTip: 'Заметка для себя (например, назначение профиля); на работу claude не влияет',
  dlgCancelTip: 'Закрыть без изменений — профиль не будет создан/переименован/перекрашен',
  dlgSubmitTip: 'Подтвердить: создать профиль, либо применить новое имя/цвет',
  dlgCancel: 'Отмена',
  dlgAdd: 'Добавить',
  dlgRename: 'Переименовать',
  dlgApply: 'Применить',

  // LaunchConfigDialog
  lcClose: 'Закрыть',
  lcTitle: 'Инструменты и размер · профиль «{name}»',
  lcLeanToggle: 'Лёгкий режим',
  lcLeanHeading: 'Лёгкий режим (меньше контекста)',
  lcLeanDesc:
    'Урезает системный промпт флагами запуска (обратимо, без правки глобального managed). Полезно для локальных моделей — меньше токенов, быстрее, точнее вызовы инструментов.',
  lcBareNote:
    'Применится {bare} (≈1k токенов): без плагинов/hooks/LSP/авто-памяти. Hooks (в т.ч. RTK) не работают. Ниже выбери, что вернуть.',
  lcSafeModeNote:
    'У профиля нет токен-провайдера (OAuth) → применится {safeMode} (≈28k): отключает плагины/MCP/скиллы. Точечный выбор MCP/CLAUDE.md тут недоступен.',
  lcMcpLabel: 'MCP-серверы (что подключить в лёгком режиме)',
  lcMcpEmpty: 'Список MCP пуст (config\\.mcp.json).',
  lcClaudeMdToggle: 'CLAUDE.md',
  lcClaudeMd: 'Подключить CLAUDE.md профиля (--add-dir)',
  lcSizeLabel: 'Размер системного промпта · измерить:',
  lcMeasuring: 'Измеряю…',
  lcMeasureLean: 'Лёгкий',
  lcMeasureLeanTip: 'Сохранить выбор и измерить лёгкий набор (быстро)',
  lcMeasureFull: 'Полный (медленно)',
  lcMeasureFullTip: 'Измерить полный режим (медленно на локальной модели — десятки секунд)',
  lcLeanResult: 'Лёгкий: ',
  lcFullResult: 'Полный: ',
  lcTokensUnit: 'ток.',
  lcMeasureNote:
    'Измерение запускает {cmd} против провайдера профиля и читает usage.input_tokens.',
  lcCancelTip: 'Закрыть без сохранения — режим и набор инструментов профиля не изменятся',
  lcApplyTip: 'Сохранить режим запуска и выбранный набор инструментов для этого профиля',
  lcCancel: 'Отмена',
  lcApply: 'Применить'
};
