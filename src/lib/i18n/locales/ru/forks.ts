export default {
  // Core helper keys consumed by forkMode() / outcomeLabel() in the i18n module.
  mode_readonly: 'только чтение',
  mode_readonly_nofetch: 'только чтение (без fetch)',
  mode_dryrun: 'предпросмотр плана',
  mode_apply: 'применение изменений',
  outcome_merged: 'влита',
  outcome_clean: 'чисто',
  outcome_conflict: 'конфликт',
  outcome_closed_unmerged: 'закрыта без слияния',
  outcome_local_only: 'только локально',

  // ── ForksTab: header ──
  title: 'Форки и репозитории',
  intro:
    'Следит за твоими форками на GitHub: насколько они отстали от оригинала (upstream), какие ветки/PR влиты, где конфликты. Действия (подтянуть, перебазировать и т.п.) — только для форков; «свои» репозитории показаны для статуса.',
  introGhUnavailable: ' gh недоступен — PR по эвристике.',
  checkTip: 'Пересобрать статус форков (только чтение, ничего не меняет)',
  planBtn: 'Показать план',
  planTip:
    'Показать план безопасных действий (dry-run: ff/удаление влитых/rebase/normalize) — ничего не меняет',
  ffAllBtn: 'Подтянуть все обновления',
  ffAllTip: 'Безопасно подтянуть обновления из upstream для {n} форков (только fast-forward)',
  ffAllNoneTip: 'Нет форков, отстающих от upstream',

  // ── ForksTab: KPIs ──
  kpiRepos: 'репозиториев',
  kpiReposTip: 'Всего управляемых репозиториев',
  kpiMerged: 'влито',
  kpiMergedTip: 'Веток влито в upstream',
  kpiOpen: 'открыто',
  kpiOpenTip: 'Открытых веток / PR в работе',
  kpiConflicts: 'конфликтов',
  kpiConflictsTip: 'Веток с конфликтами слияния',
  kpiNeedHands: 'требуют действий',
  kpiNeedHandsTip: 'Сколько репозиториев/веток требуют ручного вмешательства',
  updatedAt: 'обновлено: {time}',

  // ── ForksTab: empty state ──
  emptyTitle: 'Нет данных',
  emptyHint: 'Нажми «Проверить», чтобы собрать статус форков.',

  // ── ForkRepoCard: AI prompt ──
  promptBranchLine: '- ветка «{name}»',
  promptPrSuffix: ' (PR #{n})',
  promptConflictFiles: '; конфликтные файлы: {files}',
  promptRepo: 'Репозиторий: {name}  ({path})',
  promptRemotes: 'upstream: {upstream} | форк: {fork} | ветка по умолчанию: {branch}',
  promptTask: 'Задача: разрешить конфликты слияния с upstream для веток:',
  promptInstructions:
    'Для каждой ветки: переключись на неё, влей/перебазируй на свежий upstream/{branch}, аккуратно разреши конфликты (сохранив осмысленные изменения с обеих сторон), прогони сборку/тесты, сделай коммит. Не делай force-push без подтверждения.',

  // ── ForkRepoCard: recommended action ──
  recManualPlain: 'разобраться вручную (идёт незавершённая git-операция)',
  recManualLabel: 'Открыть терминал',
  recManualTip: 'Незавершённая git-операция / detached HEAD — разреши вручную в терминале',
  recConflictPlain: 'разрешить конфликты слияния с оригиналом',
  recConflictCopied: '✓ Промпт скопирован',
  recConflictLabel: 'Скопировать AI-промпт',
  recConflictTip: 'Скопируй готовый промпт и попроси Claude Code разрешить конфликты',
  recFfPlain: 'подтянуть обновления из оригинала (отстаёт на {n})',
  recFfLabel: 'Подтянуть из upstream',
  recDeletePlain: 'удалить ветки, уже влитые в оригинал',
  recDeleteLabel: 'Удалить влитые ветки',

  // ── ForkRepoCard: health badge ──
  healthAnalysisError: 'ошибка анализа',
  healthAnalysisErrorTip: 'Не удалось проанализировать репозиторий',
  healthSkippedTip: 'Репозиторий пропущен',
  healthOpName: 'операция',
  healthOpTip: 'В репозитории идёт незавершённая git-операция — действия заблокированы',
  healthDetached: 'detached HEAD',
  healthDetachedTip: 'HEAD не на ветке (detached) — действия заблокированы',
  healthConflictTip: 'Есть ветки, которые не вольются без ручного разрешения конфликтов',
  healthBehind: 'отстаёт на {n}',
  healthBehindTip:
    'Ветка по умолчанию отстаёт от upstream на {n} — можно подтянуть (FF)',
  healthClean: 'чисто',
  healthCleanTip: 'Всё синхронизировано, действий не требуется',

  // ── ForkRepoCard: PR badges ──
  prOpen: 'PR открыт',
  prMerged: 'PR влит',
  prClosed: 'PR закрыт',

  // ── ForkRepoCard: action tips ──
  ffTipNotBehind: 'Недоступно: ветка не отстаёт от upstream',
  ffTipDirty: 'Недоступно: есть незакоммиченные изменения',
  ffTipDiverged: 'Недоступно: ветка разошлась — fast-forward невозможен',
  ffTipUnavailable: 'Недоступно',
  ffTip: 'Fast-forward: подтянуть «{branch}» к upstream (безопасно, без слияния)',
  delTip: 'Удалить ветки, уже влитые в upstream (локально и на форке)',
  delTipUnavailable: 'Недоступно: нет влитых веток',
  rebaseTip: 'Перебазировать открытые ветки на свежий upstream (локально; при конфликте — отмена)',
  rebaseTipDirty: 'Недоступно: грязное рабочее дерево',
  rebaseTipUnavailable: 'Недоступно: нет открытых веток для rebase',
  normTip: 'Привести remotes к канону: origin = ваш форк, upstream = оригинал',

  // ── ForkRepoCard: card body ──
  collapseTip: 'Свернуть детали',
  expandTip: 'Показать ветки и PR',
  badgeOwn: 'свой',
  badgeFork: 'форк',
  badgeOwnTip: 'Ваш собственный репозиторий',
  badgeForkTip: 'Форк чужого репозитория',
  onBranch: ' · на {branch}',
  badgeDirty: 'изменённые файлы',
  badgeDirtyTip: 'Есть незакоммиченные изменения в отслеживаемых файлах',
  badgeUntracked: 'неотслеживаемые',
  badgeUntrackedTip: 'Есть новые файлы вне контроля версий',
  badgeRolesGuessed: 'роли по эвристике',
  badgeRolesGuessedTip: 'gh недоступен — роли remote определены по эвристике',
  upstream: 'upstream',
  upstreamTip: 'Оригинальный репозиторий',
  fork: 'форк',
  forkTip: 'Ваш форк',
  outcomeTip: 'Исход интеграции ветки в upstream',
  prLinkTip: 'Открыть PR на GitHub',
  ciTip: 'Статус CI-проверок',
  ciLabel: 'CI: {checks}',
  conflictInFiles: 'конфликт в файлах: {files}',
  noTopicBranches: 'Топик-веток нет.',

  // ── ForkRepoCard: action row ──
  recommended: 'Рекомендуется:',
  terminal: 'Терминал',
  terminalTip:
    'Открыть терминал в каталоге репозитория (там можно запустить claude и работать вручную)',
  moreActionsTip: 'Ещё действия с форком',
  actionFf: 'Подтянуть из upstream',
  actionDelete: 'Удалить влитые ветки',
  actionRebase: 'Перебазировать на upstream',
  actionNormalize: 'Исправить имена remotes',

  // ── Labels passed to onAction (shown in confirm/log UI) ──
  labelFf: '{name}: fast-forward «{branch}» к upstream',
  labelDelete: '{name}: удалить влитые ветки (локально + форк)',
  labelRebase: '{name}: перебазировать открытые ветки на upstream',
  labelNormalize: '{name}: нормализовать remotes'
};
