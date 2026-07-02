# План: порт функционала herdr в «Сессии» + встроенный plugin-sync

Дата: 2026-07-02. Решение: **нативный порт** (не встраивание herdr — Windows-бета апстрима,
AGPL-3.0, второй фоновый процесс). herdr — образец UX; код и TOML-манифесты **не копируем**.

## Разрыв (herdr → Castellyn)

Уже есть (переиспользуем): PTY-стек (`session_spawn` + Channel + ring 256KiB), grid со
сплиттерами, maximize, background-панели, workspaces/favorites, attach/detach/move,
broadcast, send-to-all, SSH, поиск, мультимонитор, reload-survival.

Не хватает четырёх подсистем herdr:
1. **Статусы агентов** (blocked / working / done / idle / unknown)
2. **Уведомления** (звук + системные) на переходы состояний
3. **Roll-up статусов** (заголовок панели → тулбар → бейдж сайдбара)
4. **Restore/Resume** сессий после перезапуска приложения (`claude --resume <id>`)

Плюс отдельно: **plugin-sync между профилями** (ручной хук пользователя → в настройки Castellyn).

## Архитектура статусов (ядро порта)

Модель herdr: `AgentState = Blocked | Working | Idle | Unknown` + флаг `seen`
(«done» = Idle + !seen). Три источника истины, по убыванию авторитета:

### 1. Lifecycle-hooks для Claude Code (наш козырь, у herdr этого нет)
Castellyn владеет профилями → ставим свой хук-скрипт (по образцу plugin_sync):
- Спавн панели получает env `CASTELLYN_SESSION_ID=<id панели>`.
- Хук `castellyn_status.py` в профилях (SessionStart/UserPromptSubmit/PreToolUse →
  working; Notification (permission) → blocked; Stop → done; SessionEnd → idle).
  Скрипт мгновенно no-op, если env отсутствует (обычные сессии вне Castellyn не трогаем).
- Хук пишет `%APPDATA%\castellyn\agent-status\<CASTELLYN_SESSION_ID>.json`
  `{state, claude_session_id, ts}` (атомарно, fail-open). `claude_session_id` из
  входного JSON хука — он же ключ для `--resume`.
- Backend следит за папкой (poll 500 мс в reader-цикле или notify-watcher) и
  эмитит `agent-status:<id>` во фронт.

### 2. PTY-активность (для всех инструментов)
Байты из PTY идут → working (с антидребезгом herdr: working→idle только после
паузы вывода ~1–2 с + подтверждение; startup grace 3 с; exit → idle мгновенно).

### 3. Скрин-эвристики (fallback: codex/opencode; claude — самолечение)
Ring-буфер уже в backend: strip ANSI → последние N непустых строк → свои правила
(Rust, по мотивам подхода herdr, БЕЗ копирования его TOML):
- blocked: видимые промпты подтверждения («do you want to proceed?», «esc to cancel»,
  y/n-меню) — строго, при сомнении idle;
- idle: видимая строка ввода (`❯`, `>`) без блокера.
Правила — данные (JSON в бинарнике + возможность локального оверрайда в config dir).

Арбитраж: hook-authority (свежий TTL) > скрин-правило > PTY-активность. Диагностика:
скрытая команда «объяснить статус» (какое правило сматчилось) — аналог `agent explain`.

## Волны

### Волна 0 — Plugin-sync в Castellyn (маленькая, независимая)
- Нативный Rust-порт reconcile-логики `plugin_sync.py` (enabledPlugins=true из любого
  профиля → добиваем в профили, где ключ ОТСУТСТВУЕТ; False = осознанный opt-out, не
  трогаем; extraKnownMarketplaces через setdefault; запись атомарная, only-if-changed,
  BOM-safe) — кнопка «Синхронизировать плагины сейчас» + отчёт что куда добавлено.
- Тумблер «Авто-синхронизация при старте сессий»: ставит/снимает SessionStart-хук
  `plugin_sync.py` во всех профилях (порт wire_plugin_sync.py, идемпотентно; сам
  скрипт кладём в `~/.claude/hooks/`, версионируем маркером `plugin-sync-version`).
- Место: вкладка «Среды» (там уже живут fan-out'ы) или «Плагины» — решить при
  реализации по имеющейся структуре UI.

### Волна 1 — Статусы агентов
- Backend: модуль `agent_status` (hook-watcher + PTY-активность + скрин-правила +
  арбитраж + антидребезг), событие `agent-status:<id>`, env-инжект, установка/снятие
  статус-хука в профили (переиспользуя механику Волны 0).
- Frontend: точка/спиннер состояния в заголовке панели (blocked=красный, working=жёлтый
  спиннер, done=teal, idle=зелёный, unknown=серый), seen-логика (фокус панели гасит
  done→idle), сводка в тулбаре («2 blocked · 1 working · 1 done»), бейдж вкладки
  Sessions в сайдбаре через существующий `attention.ts`.

### Волна 2 — Уведомления
- Переходы: →blocked = звук «request» + уведомление; working/blocked→idle(done) =
  звук «done» + уведомление. Подавление: панель видима и окно в фокусе → тихо.
- Системные уведомления: `tauri-plugin-notification` (клик → фокус окна и панели —
  насколько позволит плагин). Звук: свои короткие mp3/wav (НЕ ассеты herdr, AGPL),
  плеер — PowerShell MediaPlayer с CREATE_NO_WINDOW (паттерн уже в проекте).
- Настройки: вкл/выкл звук, вкл/выкл системные, per-tool отключение, задержка (0–N с).

### Волна 3 — Restore / Resume
- Снапшот формы сессий на диск (`%APPDATA%\castellyn\sessions-snapshot.json`):
  панели (tool, profile, cwd, args, ssh), layout, раскладка колонок — при изменениях.
- При старте приложения: баннер «Восстановить прошлую сессию? (N панелей)».
- Claude-панели: `claude --resume <claude_session_id>` (id уже приходит из хука В1).
  opencode/codex — проверить актуальные флаги resume при реализации; без id — обычный
  respawn в сохранённом cwd.
- Существующий localStorage-механизм reload-survival остаётся (окно-уровень);
  снапшот — уровень приложения.

### Волна 4 (опционально, отдельное решение) — Группы/табы
herdr-слой Tab (несколько layout'ов в workspace): вкладки-группы панелей внутри
«Сессий» с roll-up статуса на каждую. Большая UI-работа; ценность есть при 6+ панелях.

## Не переносим (осознанно)
- Клиент-серверную архитектуру / detach выживание после закрытия Castellyn (у нас
  Job Object намеренно убивает дерево — иное поведение = отдельное решение).
- Socket API для внешнего управления, remote over SSH как в herdr, live handoff.
- BSP-сплиты произвольной вложенности (наш grid покрывает сценарии), copy-mode,
  prefix-клавиатуру (у нас GUI + хоткеи).
- Детекцию 17 агентов — только claude / codex / opencode (+shell без статусов).

## Гейты
`npm run check` 0/0, `npm test`, `npm run build`, cargo build, `build_all.ps1`;
по Волнам 1–2 — обязательный живой смоук Sessions (запуск claude, увидеть переходы
статусов и уведомление) до объявления готовности.

## Юридическая рамка
herdr = AGPL-3.0: не копировать код, TOML-манифесты, mp3. Реализация «по мотивам»
(своя выборка правил по реальному выводу CLI, свои звуки, своя модель состояний
с теми же 4 значениями enum) — чисто.
