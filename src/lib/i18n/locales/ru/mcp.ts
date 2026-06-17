export default {
  // Header
  title: 'MCP-серверы',
  subtitle: 'Источник истины (config/.mcp.json) и развёртывание по профилям',
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
  commandTitle: 'Команда запуска сервера',
  pluginBadge: 'из плагина',
  pluginBadgeTitle: 'Сервер приходит из плагин-маркетплейса — в профили не разворачивается',
  deployedCountTitle: 'Развёрнут в {n} из {total} профилей',
  pluginNote: 'Глобально через плагин-маркетплейс — не разворачивается в профили (это нормально).',
  profileDeployedTitle: 'Развёрнут в профиле {p}',
  profileNotDeployedTitle: 'НЕ развёрнут в {p} — нажми «Развернуть во все профили», чтобы добавить',
  deployToProfileTip: 'Развернуть все MCP-серверы в профиль {p}',

  // Empty state
  emptyTitle: 'Нет данных',
  emptyHint: 'config/.mcp.json не найден или пуст.',

  // Extras (out of source of truth)
  extrasHeading: 'Вне источника истины',
  extrasNote: 'Серверы, найденные в профилях, но отсутствующие в config/.mcp.json:',
  extrasProfileTitle: 'Есть в этом профиле, но нет в источнике истины'
};
