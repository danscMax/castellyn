export default {
  // Header
  title: 'Бэкап',
  subtitle: 'Снапшоты конфигов профилей Claude Code',
  createTitle: 'Создать новый снапшот конфигов всех профилей сейчас',
  makeBackup: 'Сделать бэкап',
  retention: 'Хранить:',
  retentionTip: 'Сколько последних снапшотов оставлять (старые удаляются при бэкапе)',

  // Freshness badge
  fresh: 'свежий',
  staling: 'устаревает',
  stale: 'устарел',
  relToday: 'сегодня',
  relYesterday: 'вчера',
  relDaysAgo: '{n} дн. назад',

  // Status card
  lastBackup: 'Последний бэкап',
  lastSnapshot: 'Последний снапшот',
  snapshotsWeekly: 'Снапшотов / weekly',
  weeklyArchive: 'Weekly архив',

  // Snapshots list
  snapshotsHeading: 'Снапшоты ({n})',
  latest: 'последний',
  restoreItemTitle: 'Восстановить конфиги из этого снапшота (сначала покажет план)',
  deleteItemTitle: 'Безвозвратно удалить этот снапшот',
  weekliesHeading: 'Недельные архивы ({n})',
  revealItemTitle: 'Показать этот архив в проводнике',
  verify: 'Проверить',
  verifyItemTitle: 'Проверить, что архив не повреждён (вывести список содержимого)',
  verifyOk: 'Архив в порядке — {n} записей',
  verifyFail: 'Архив повреждён или нечитаем',
  extract: 'Извлечь',
  extractItemTitle: 'Извлечь архив в выбранную папку (живой ~/.claude не затрагивается)',
  extractOk: 'Извлечено',
  extractFail: 'Не удалось извлечь архив',
  importZip: 'Импорт zip…',
  importZipTip: 'Проверить и извлечь zip-бэкап с произвольного пути (например, с другой машины)',
  importOk: 'Импортировано: {n} файлов',
  importFail: 'Не удалось импортировать архив',
  deleteWeeklyTitle: 'Удалить этот недельный архив',
  deleteWeeklyMsg: 'Безвозвратно удаляет этот недельный архив (skills/agents/commands). Снапшоты и живой конфиг не затрагиваются.',
  restore: 'Восстановить',
  emptyTitle: 'Снапшотов нет',
  emptyHint: 'Нажми «Сделать бэкап», чтобы создать первый.',

  // Restore dialog
  dialogTitle: 'Восстановление из снапшота',
  profiles: 'Профили',
  profileToggleTip: 'Включить этот профиль в восстановление; снятый профиль остаётся нетронутым',
  includeCreds: 'Восстановить учётные данные (не перезапишет существующие)',
  includeCredsTip: 'Доложить недостающие учётные данные из снапшота; существующие токены не трогаются',
  warn: 'Реальное восстановление перезапишет живые конфиги выбранных профилей — необратимо. Сначала посмотри план.',
  closeTitle: 'Закрыть без изменений',
  previewTitle: 'Предпросмотр (-WhatIf): показать, что будет перезаписано — ничего не меняет',
  showPlan: 'Показать план',
  restoreTitle: 'Восстановить выбранные профили из снапшота (необратимо)',
  restoreNeedsPreview: 'Сначала нажми «Показать план»',

  // In-dialog plan summary (human-readable; raw script output tucked under details)
  planWhat: 'Что сделает восстановление',
  planProfiles: 'Перезапишет конфиги профилей ({n}): {list}',
  planCredsOn: 'Недостающие учётные данные заполнятся из снапшота (существующие не трогаются).',
  planCredsOff: 'Учётные данные не затрагиваются.',
  planUntouched: 'Общие и системные файлы не изменяются.',
  planDetails: 'Технический вывод',
  restoreDone: 'Восстановление завершено'
};
