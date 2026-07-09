export default {
  // ── UpdatesTab: header ──
  title: 'Обновления',
  subtitle: 'Проверка и применение обновлений всего стека Claude Code',
  groupHasUpdate: 'Есть обновление ({count})',
  groupUpToDate: 'Актуально ({count})',
  groupHeld: 'На удержании ({count})',
  groupErrors: 'С ошибками ({count})',
  groupAllClear: 'всё актуально',
  staleOldest: 'самая старая проверка: {time}',
  statusCorrupt: 'статус повреждён',
  summaryChecked: 'проверено {time}',
  updatingNow: 'сейчас: {step}',
  checkAllBtn: 'Проверить всё',
  updateAllBtn: 'Обновить всё',

  // ── ComponentCard: forks summary ──
  forkConflicts: '{count} с конфликтами',
  forkToDelete: '{count} к удалению',
  forkOpenPr: '{count} открытых PR',
  forkAllSynced: 'всё синхронизировано',

  // ── ComponentCard: health badges ──
  healthNoStatus: 'без статуса',
  healthNoData: 'нет данных',
  healthFailedCount: '{count} с ошибкой',
  healthError: 'ошибка',
  healthHeld: 'на удержании',
  healthNeedsAttentionOne: '{count} требует внимания',
  healthNeedsAttentionMany: '{count} требуют внимания',
  healthUpToDate: 'актуально',
  healthUnknown: 'статус «{status}» неизвестен',

  // ── ComponentCard: details ──
  lastRun: 'Последний запуск',
  duration: 'Длительность',
  durationSeconds: '{count} с',

  // ── ComponentCard: actions ──
  checkTip: 'Проверить наличие обновлений (только чтение, ничего не устанавливает)',
  checking: 'Идёт…',
  checkBtn: 'Проверить',
  openForksBtn: 'Открыть Форки',
  openForksTip: 'Перейти на вкладку «Форки» — там действия по каждому репозиторию',
  openPluginsBtn: 'Открыть Расширения',
  openPluginsTip: 'Перейти на вкладку «Расширения» — там плагины, скилы и агенты',
  updateBtn: 'Обновить',
  updateBtnCount: 'Обновить ({count})',
  updateTip: 'Установить доступные обновления этого компонента (с подтверждением)',
  applyBtn: 'Применить',
  applyTip:
    'Запустить обновление (статус ещё не проверялся — нажми «Проверить», чтобы узнать, есть ли что обновлять)',
  upToDate: 'актуально',
  upToDateTip: 'Обновлений нет — всё актуально'
};
