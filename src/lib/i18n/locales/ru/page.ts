// Strings owned by the top-level page orchestrator (+page.svelte) and the
// run-outcome mapper (outcome.ts): confirm dialogs, run-log lines, toasts.
export default {
  // Home/Overview tab keys
  home_title: 'Обзор',
  home_subtitle: 'Здоровье профилей, бэкапа, синка и конфига одним взглядом.',
  home_allOk: 'всё ок',
  home_noData: 'пока нет данных',
  home_issues: 'проблем: {n}',
  home_backup: 'Бэкап',
  home_backupAge: 'последний {time}',
  home_config: 'Файлы конфига',
  home_configDrifted: '{n} разошлось',
  home_configUnlinked: '{n} без ссылки',
  home_ok: 'в синхроне',
  home_profiles: 'Профили',
  home_profilesOk: '{n} ок',
  home_profilesBroken: '{n} битых связей',
  home_conflicts: 'Конфликты',
  home_conflictsN: '{n} файлов',
  home_sync: 'Syncthing',
  home_syncOffline: 'не подключён',
  home_tasks: 'Задачи',
  home_tasksFailing: '{n} с ошибкой',
  home_tasksOff: '{n} выключено',
  // Wave-4 config-drift / time keys
  drift_log: 'Дрейф конфига: {verb}…',
  drift_verb_check: 'проверка связей',
  drift_verb_relink: 'починка связей',
  drift_verb_sync: 'синхронизация конфига',
  confirm_relink_title: 'Починить связи конфига',
  confirm_relink_msg: 'Заново установить symlink общих файлов конфига (statusline, CLAUDE.md, hooks…). Требуются права администратора (UAC).',
  confirm_relink_btn: 'Починить',
  confirm_driftsync_title: 'Синхронизировать конфиг',
  confirm_driftsync_msg: 'Запустить бэкап — живой конфиг отзеркалится в источник деплоя.',
  confirm_driftsync_btn: 'Синхр.',
  // Operational display names (toasts / log)
  op_backup: 'Бэкап',
  op_profiles: 'Профили',
  op_mcp: 'MCP',
  op_sync: 'Синхронизация',
  op_engine: 'Движок',
  op_provider: 'Провайдер',
  op_schedule: 'Расписание',
  op_plugins: 'Плагины',
  op_forks: 'Форки',

  // Generic run-log lines
  log_component: '▶ {name}: {verb}…',
  log_error: '✖ Ошибка: {e}',
  log_warn: '⚠ {e}',
  log_done: '■ Готово (код {code}).',
  verb_apply: 'применение',
  verb_check: 'проверка',

  // Component apply
  confirm_apply_title: 'Применить обновление?',
  confirm_apply_msg: 'Компонент «{name}» будет РЕАЛЬНО обновлён (-Apply). Продолжить?',
  confirm_apply_btn: 'Применить',

  // Forks
  forks_verb_check: 'проверка',
  forks_verb_plan: 'план (dry-run)',
  forks_verb_action: 'действие «{action}»',
  forks_verb_syncwip: 'синхронизация wip-local',
  forks_log: '▶ Форки: {verb}{path}…',
  forks_starting: 'запуск…',
  toast_fork_done: '{name}: обновлено',
  toast_fork_error: '{name}: ошибка (код {code})',
  forks_recheck: '▶ Форки: перепроверка…',
  confirm_fork_title: 'Изменить форк?',
  confirm_fork_msg: '{label}. Это РЕАЛЬНО изменит репозиторий. Продолжить?',
  confirm_fork_btn: 'Выполнить',
  confirm_batchff_title: 'Подтянуть все обновления?',
  confirm_batchff_msg:
    'Будет выполнен безопасный fast-forward для {n} форков: {names}. Это только перемотка вперёд (без слияния и force-push). Продолжить?',
  confirm_batchff_btn: 'Подтянуть',

  // Backup
  backup_verb_snapshot: 'создание снапшота',
  backup_verb_restore_preview: 'план восстановления (-WhatIf)',
  backup_verb_restore: 'восстановление',
  backup_log: '▶ Бэкап: {verb}…',
  backup_snap_last: 'последний',
  confirm_restore_title: 'Восстановить конфиги?',
  confirm_restore_msg:
    'Снапшот «{snap}» перезапишет живые конфиги выбранных профилей — необратимо. Продолжить?',
  confirm_restore_btn: 'Восстановить',

  // Profiles
  prof_verb_add: 'добавление профиля {name}',
  prof_verb_remove: 'удаление профиля {name}',
  prof_verb_rename: 'переименование {name} → {newName}',
  prof_verb_recolor: 'смена цвета {name}',
  prof_verb_setlinks: 'общие папки {name}',
  prof_log: '▶ Профили: {verb}…',
  prof_verb_check: 'проверка',
  prof_verb_clean: 'удаление sync-конфликтов',
  prof_verb_repair: 'починка связей {name}',
  prof_verb_reinstall: 'переустановка профилей',
  confirm_prof_remove_title: 'Удалить профиль «{name}»?',
  confirm_prof_remove_msg:
    'Каталог ~/.claude-{name} будет удалён вместе с сохранённым входом и настройками этого профиля. Общий контент (skills/projects и т.д.) не пострадает. Действие необратимо.',
  confirm_prof_remove_btn: 'Удалить',
  confirm_reinstall_title: 'Переустановить профили?',
  confirm_reinstall_msg:
    'Install-ClaudeProfiles.ps1 -Force пересоздаст junction’ы/симлинки всех профилей и потребует прав администратора (UAC). Продолжить?',
  confirm_reinstall_btn: 'Переустановить',
  confirm_reinstall_word: 'ПЕРЕУСТАНОВИТЬ',
  confirm_clean_title: 'Удалить sync-конфликты?',
  confirm_clean_msg:
    'Будут удалены файлы-дубли *.sync-conflict-* (оригиналы не тронутся; Syncthing хранит версии). Продолжить?',
  confirm_clean_btn: 'Удалить',

  // MCP
  mcp_log: '▶ MCP: развёртывание во все профили…',
  confirm_mcp_title: 'Развернуть MCP во все профили?',
  confirm_mcp_msg:
    'Серверы из config/.mcp.json будут добавлены в каждый профиль (user-scope, идемпотентно). Существующие перезапишутся теми же значениями. Продолжить?',
  confirm_mcp_btn: 'Развернуть',

  // Sync
  sync_log_set: '▶ Синхронизация: применение настроек…',
  sync_log_query: '▶ Синхронизация: проверка…',
  sync_apply_off:
    'Перестанут синхронизироваться между машинами: {off} (локальные файлы не удаляются). .stignore будет перегенерирован.',
  sync_apply_all: 'Все элементы будут синхронизироваться. .stignore будет перегенерирован.',
  confirm_sync_title: 'Применить настройки синхронизации?',
  confirm_sync_btn: 'Применить',

  // Engines / providers / router
  engine_log: '▶ Движок {id}: {verb}…',
  engine_verb_start: 'запуск',
  engine_verb_stop: 'остановка',
  confirm_engine_stop_title: 'Остановить движок?',
  confirm_engine_stop_msg: 'Будет остановлен процесс, слушающий порт движка «{id}». Продолжить?',
  confirm_engine_stop_btn: 'Остановить',
  stack_log: '▶ LLM-стек: {verb}…',
  stack_verb_start: 'запуск',
  stack_verb_stop: 'остановка',
  confirm_stack_stop_title: 'Остановить весь стек?',
  confirm_stack_stop_msg:
    'Будут остановлены все сервисы LLM-стека (шлюз и бэкенды). Открытые дашборды перестанут отвечать.',
  confirm_stack_stop_btn: 'Остановить всё',
  opencode_log: '▶ opencode ← {engine} ({model})…',
  confirm_opencode_title: 'Подключить к opencode?',
  confirm_opencode_msg:
    'opencode будет использовать «{engine}» (модель {model}). Запишется в opencode.json (бэкап .bak).',
  confirm_opencode_btn: 'Подключить',
  provider_log: '▶ Провайдер {name}: {verb}…',
  provider_verb_set: 'привязка',
  provider_verb_clear: 'сброс',
  confirm_provider_clear_title: 'Сбросить провайдера?',
  confirm_provider_clear_msg:
    'Профиль «{name}» вернётся к стандартному Anthropic-логину (провайдерский env будет очищен). Продолжить?',
  confirm_provider_clear_btn: 'Сбросить',
  router_install_log: '▶ Роутер: установка claude-code-router (npm)…',
  confirm_router_title: 'Подключить через роутер?',
  confirm_router_msg:
    'Профиль «{profile}» будет переключён на «{engine}» (модель «{model}») через ccr: настрою и запущу claude-code-router и привяжу профиль к http://127.0.0.1:3456. Перезапусти профиль после. Продолжить?',
  confirm_router_btn: 'Подключить',
  router_log: '▶ Роутер: {engine} ({model}) → профиль {profile}…',

  // Schedule
  sched_verb_enable: 'включение',
  sched_verb_disable: 'выключение',
  sched_verb_run: 'запуск',
  sched_verb_create: 'создание расписания',
  sched_verb_delete: 'удаление расписания',
  sched_log: '▶ Расписание ({id}): {verb}…',
  confirm_sched_delete_title: 'Удалить задание?',
  confirm_sched_delete_msg: 'Задание «{id}» будет удалено из планировщика Windows. Продолжить?',
  confirm_sched_delete_btn: 'Удалить',

  // Plugins
  plugin_verb_update: 'обновление',
  plugin_verb_enable: 'включение',
  plugin_verb_disable: 'выключение',
  plugin_verb_remove: 'удаление',
  plugin_log: '▶ Плагин {id}: {verb}…',
  confirm_plugin_disable_title: 'Выключить плагин?',
  confirm_plugin_disable_msg: '«{id}» будет выключен во всех профилях. Продолжить?',
  confirm_plugin_disable_btn: 'Выключить',
  confirm_plugin_remove_title: 'Удалить плагин?',
  confirm_plugin_remove_msg: '«{id}» будет удалён (claude plugin remove). Продолжить?',
  confirm_plugin_remove_btn: 'Удалить',
  confirm_plugin_bulk_remove_msg: 'Будет удалено плагинов: {count} (claude plugin remove). Продолжить?',
  confirm_skill_delete_title: 'Удалить скилл?',
  confirm_skill_delete_msg: 'Папка скилла «{name}» будет удалена безвозвратно. Продолжить?',
  confirm_skill_delete_btn: 'Удалить',

  // Operational toasts
  toast_op_done: '{name}: готово',
  toast_op_error: '{name}: ошибка (код {code})',
  toast_op_error_detail: 'Подробности — в логе выполнения.',
  toast_open_log: 'Открыть лог',
  toast_generic_error: 'Что-то пошло не так',

  // Misc
  load_error: 'Ошибка загрузки: {e}',
  wip: 'Раздел в разработке — появится в следующих итерациях.',

  // Run outcomes (outcome.ts)
  out_duration: 'за {d}',
  out_sec: '{n} с',
  out_fork_conflicts: '{n} с конфликтами',
  out_fork_merged: '{n} веток к удалению',
  out_fork_open: '{n} открытых PR',
  out_forks_need: 'Форки: требуют действий — {need}',
  out_forks_synced: 'Форки: всё синхронизировано',
  out_open_forks: 'Открыть Форки',
  out_failed_count: '{name}: требуют внимания — {failed}',
  out_failed_problems: '{name}: есть проблемы',
  out_applied: '{name}: обновлено',
  out_changes_count: '{name}: доступно обновлений — {changed}',
  out_changes_any: '{name}: есть обновления',
  out_changes_detail: 'Нажми «Обновить» на карточке.',
  out_uptodate: '{name}: актуально',
  hkTitle: 'Горячие клавиши',
  hkPalette: 'Палитра команд / переход по вкладкам',
  hkNewSession: 'Новая сессия (на вкладке «Сессии»)',
  hkColumns: 'Колонок: 1 / 2 / 3',
  hkFocusPane: 'Фокус на следующую / предыдущую панель',
  hkFind: 'Поиск в терминале',
  hkCopyPaste: 'Копировать / вставить в терминале',
  hkHelp: 'Эта справка'
};
