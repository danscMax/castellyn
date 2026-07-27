export default {
  // Header
  title: 'MCP-серверы',
  subtitle: 'Источник истины (config/.mcp.json) и развёртывание по профилям',
  colName: 'Сервер',
  colCommand: 'Команда',
  colDeployed: 'Развёрнут',
  colProfiles: 'Профили',
  colActions: 'Действия',
  refreshTitle: 'Перечитать конфиги и обновить матрицу (только чтение)',
  refreshing: 'Идёт…',
  refresh: 'Обновить',
  deployTitle:
    'Развернуть все серверы из config/.mcp.json во все профили (user-scope, идемпотентно). Заполнит серые ячейки.',
  deployAll: 'Развернуть во все профили',
  selectProfiles: 'Выбрать профили:',
  selectProfileTip: 'Отметить «{p}» для массового развёртывания',
  bulkDeploy: 'Развернуть в выбранные',
  bulkDeployTip: 'Развернуть все MCP-серверы в отмеченные профили',

  // Server card
  pluginBadge: 'из плагина',
  pluginBadgeTitle: 'Сервер приходит из плагин-маркетплейса — в профили не разворачивается',
  deployedCountTitle: 'Развёрнут в {n} из {total} профилей',
  pluginNote: 'Глобально через плагин-маркетплейс — не разворачивается в профили (это нормально).',
  profileDeployedTitle: 'Развёрнут в профиле {p}',
  deployToProfileTip: 'Развернуть все MCP-серверы в профиль {p}',

  // Liveness probe (запускает реальный процесс — только по явному клику)
  colHealth: 'Связь',
  probeGo: 'проверить',
  probeTip: 'Запустить сервер, выполнить рукопожатие MCP и получить список инструментов. Стартует реальный процесс и сразу же его гасит.',
  probeAll: 'Проверить связь',
  probeAllTip: 'Проверить рукопожатие у всех серверов из config/.mcp.json. Каждый сервер реально запускается.',
  probeOkBadge: 'инстр.: {n}',
  probeOkTitle: '{server} — инструментов: {n}, ответил за {ms} мс',
  probeFail: 'нет связи',

  // Empty state
  emptyTitle: 'Нет данных',
  emptyHint: 'config/.mcp.json не найден или пуст.',

  // Extras (out of source of truth)
  extrasHeading: 'Вне источника истины',
  extrasNote: 'Серверы, найденные в профилях, но отсутствующие в config/.mcp.json:',
  removeExtraTitle: 'Убрать этот сервер из профиля {p}',

  // Add / edit a canonical server
  addServer: 'Добавить сервер',
  addServerTitle: 'Добавить сервер в config/.mcp.json',
  editServerTitle: 'Изменить этот сервер в config/.mcp.json',
  removeServerTitle: 'Удалить этот сервер из config/.mcp.json',
  formName: 'Имя сервера',
  formJson: 'Определение (JSON)',
  errEmptyName: 'Нужно имя сервера',
  errBadJson: 'Некорректный JSON',
  savedServer: 'Сервер «{name}» сохранён',
  removedServer: 'Сервер «{name}» удалён',
  removedExtra: '«{name}» убран из {profile}',
  unsavedTitle: 'Несохранённые изменения',
  unsavedMsg: 'Закрыть форму и потерять введённое?',
  discardEdits: 'Закрыть без сохранения',
  keepEditing: 'Продолжить правку'
};
