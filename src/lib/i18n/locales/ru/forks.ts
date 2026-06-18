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
  needHands_one: 'требует действия',
  needHands_few: 'требуют действия',
  needHands_many: 'требуют действий',
  kpiNeedHandsTip:
    'Репозиториев/веток, где нужно ваше вмешательство: разрешить конфликты, разобрать незакоммиченные изменения, подтянуть обновления. Конкретное действие — на карточке репозитория ниже.',
  updatedAt: 'обновлено: {time}',
  modeLine: 'последний прогон: {mode}',
  refreshing: 'обновляется…',
  filterAll: 'Все',
  filterForks: 'Форки',
  filterOwn: 'Свои',
  filterTip: 'Фильтр: все репозитории / только форки / только свои',
  sortTip: 'Сортировка: по имени / по отставанию',
  sortName: 'по имени',
  sortBehind: 'по отставанию',
  githubOnlyHeading: 'Ещё на GitHub — не клонированы ({n})',
  githubOnlyTip:
    'Твои репозитории на GitHub (включая закрытые), которых нет локально. Действия недоступны, пока не клонируешь.',
  githubOnlyEmptyFilter: 'Нет репозиториев под текущий фильтр.',
  ghPrivate: 'приватный',
  ghPrivateTip: 'Закрытый репозиторий на GitHub',
  ghOpen: 'Открыть на GitHub',
  ghOpenTip: 'Открыть страницу репозитория на GitHub',
  ghColName: 'Репозиторий',
  ghColRepo: 'owner/repo',
  ghColKind: 'Тип',
  ghColActions: 'Действия',

  // ── ForksTab: empty state ──
  emptyTitle: 'Нет данных',
  emptyHint: 'Нажми «Проверить», чтобы собрать статус форков.',

  // ── ForkRepoCard: AI prompt ──
  promptBranchLine: '- ветка «{name}»',
  promptPrSuffix: ' (PR #{n})',
  promptConflictFiles: '; конфликтные файлы: {files}',
  promptRepo: 'Репозиторий: {name}  ({path})',
  promptRemotes: 'upstream: {upstream} | форк: {fork} | ветка по умолчанию: {branch}',
  promptTask: 'Задача: проверить и при необходимости разрешить конфликты слияния с upstream для веток:',
  promptInstructions:
    'Сначала установи факты, не доверяя статусу слепо: выполни `git fetch upstream` и убедись, что ссылка upstream/{branch} существует (если нет — посмотри `git remote -v` и используй реальную ветку отслеживания). Для каждой ветки проверь, есть ли НАСТОЯЩИЙ конфликт со свежим upstream/{branch} через `git merge-tree` (или тестовый merge/rebase). Если конфликта нет — НЕ выдумывай работу: не делай пустых коммитов, слияний «для галочки» и force-push, просто сообщи, что разрешать нечего. Если конфликт реальный — переключись на ветку, влей/перебазируй на свежий upstream/{branch}, аккуратно разреши конфликты (сохранив осмысленные изменения с обеих сторон), прогони сборку/тесты, сделай коммит. Никогда не делай force-push без подтверждения.',
  promptTaskDirty:
    'Задача: в рабочем дереве есть незакоммиченные изменения (и/или новые файлы вне git). Нужно разобраться с ними и аккуратно завершить.',
  promptInstructionsDirty:
    'Просмотри изменения (git status, git diff). Пойми, что это: сгруппируй связанные правки и сделай понятные коммиты; временное/мусор — добавь в .gitignore или удали. Особый случай — вендоренные/авто-синхронизируемые файлы (в шапке файла есть пометка VENDORED/CANON или для него существует инструмент синхронизации): НЕ коммить локальную копию вслепую — сначала сверь её с каноническим источником и при расхождении обнови из канона (или прогони инструмент синхронизации), и только потом коммить, иначе зафиксируешь устаревшую версию. Прогони сборку/тесты, если они есть. Не делай force-push и не пушь без подтверждения. Если назначение каких-то изменений непонятно — не трогай их и сообщи.',

  // ── ForkRepoCard: recommended action ──
  recManualPlain: 'разобраться вручную (идёт незавершённая git-операция)',
  recManualLabel: 'Открыть терминал',
  recManualTip: 'Незавершённая git-операция / detached HEAD — разреши вручную в терминале',
  recConflictPlain: 'разрешить конфликты слияния с оригиналом',
  recConflictCopied: '✓ Промпт скопирован',
  recConflictLabel: 'Скопировать AI-промпт',
  recConflictTip: 'Скопируй готовый промпт и попроси Claude Code разрешить конфликты',
  recDirtyPlain: 'разобрать незакоммиченные изменения',
  recDirtyCopied: '✓ Промпт скопирован',
  recDirtyLabel: 'Скопировать AI-промпт',
  recDirtyTip: 'Скопируй промпт и попроси Claude Code разобрать и закоммитить изменения',
  recFfPlain: 'подтянуть обновления из оригинала (отстаёт на {n} {commits})',
  recFfLabel: 'Подтянуть из upstream',
  recDeletePlain: 'удалить ветки, уже влитые в оригинал',
  recDeleteLabel: 'Удалить влитые ветки',

  // ── ForkRepoCard: health badge ──
  healthAnalysisError: 'ошибка анализа',
  healthAnalysisErrorTip: 'Не удалось проанализировать репозиторий',
  healthSkippedTip: 'Репозиторий пропущен',
  healthOpName: 'операция',
  healthOpTip: 'В репозитории идёт незавершённая git-операция — действия заблокированы',
  healthDetached: 'HEAD вне ветки',
  healthDetachedTip: 'HEAD не на ветке (detached HEAD) — действия заблокированы, разреши вручную в терминале',
  healthConflictTip: 'Есть ветки, которые не вольются без ручного разрешения конфликтов',
  healthBehind: 'отстаёт на {n} {commits}',
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
  badgeUntracked: 'новые файлы',
  badgeUntrackedTip: 'Есть новые файлы вне контроля версий (untracked) — ещё не добавлены в git',
  badgeRolesGuessed: 'remote — приблизительно',
  badgeRolesGuessedTip:
    'gh недоступен — какой remote оригинал (upstream), а какой ваш форк (origin), определено по догадке',
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
  branchAhead: '+{n} от upstream',
  branchAheadTip: 'Коммитов в этой ветке сверх upstream: {n}',

  // ── ForkRepoCard: wip-local (личная рабочая ветка) ──
  wipBehind: 'wip-local отстал на {n} {commits}',
  wipBehindTip:
    'Личная рабочая ветка wip-local отстаёт от upstream на {n} {commits} — стоит синхронизировать',
  wipLabel: 'wip-local',
  wipBehindRow: 'отстаёт на {n} {commits}',
  wipMergedPatches: 'влито патчей: {n}',

  // ── ForkRepoCard: action row ──
  recommended: 'Рекомендуется:',
  terminal: 'Терминал',
  terminalTip:
    'Открыть сессию в папке репозитория: выбор инструмента (Claude / opencode / shell) и профиля (= провайдера)',
  externalTerminal: 'Внешний терминал (cmd)',
  externalTerminalTip: 'Открыть обычный системный cmd в папке репозитория (для ручных git-операций)',
  moreActionsTip: 'Ещё действия',
  actionFf: 'Подтянуть из upstream',
  actionDelete: 'Удалить влитые ветки',
  actionRebase: 'Перебазировать на upstream',
  actionNormalize: 'Исправить имена remotes',

  // ── Labels passed to onAction (shown in confirm/log UI) ──
  labelFf: '{name}: fast-forward «{branch}» к upstream',
  labelDelete: '{name}: удалить влитые ветки (локально + форк)',
  labelRebase: '{name}: перебазировать открытые ветки на upstream',
  labelNormalize: '{name}: нормализовать remotes',
  labelSyncWip: '{name}: синхронизировать wip-local с upstream',

  // ── ForkRepoCard: sync wip-local action ──
  recSyncWipPlain: 'синхронизировать wip-local с оригиналом (отстаёт на {n})',
  recSyncWipLabel: 'Синхронизировать wip-local',
  recSyncWipTip:
    'Перебазировать личную ветку wip-local на свежий upstream (локально, без push; при конфликте — отмена)',
  actionSyncWip: 'Синхронизировать wip-local',
  syncWipTip: 'Перебазировать wip-local на свежий upstream (локально, без push)',
  syncWipTipSynced: 'Недоступно: wip-local уже синхронизирован',
  syncWipTipDirty: 'Недоступно: есть незакоммиченные изменения',
  syncWipTipUnavailable: 'Недоступно: нет ветки wip-local',
  runStarting: 'запуск…',
  runDone: 'обновлено',
  runFailed: 'ошибка (код {code})',
  runCancel: 'Отмена',
  runCancelTip: 'Прервать обновление этого репозитория'
};
