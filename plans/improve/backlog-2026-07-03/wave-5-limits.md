# Волна 5 — Лимиты Anthropic + автопродолжение (утверждено владельцем 2026-07-03)

Ветка `improve-0703/wave-5`. Цель: программа работает ночь/дни без присмотра; упёрлись в
лимит → сами продолжаем после сброса окна, либо переключаемся на свободный профиль.

**Зависимости:** 5.2 → после 5.1 (нужно состояние `limited`); 5.3 → после 5.1+5.2;
5.4 → после 5.3; 5.5 → после 5.1 (нужны данные utilization). Порядок: 5.1 → 5.2 → 5.3 → 5.5 → 5.4.

**Референс (DRY, НЕ копировать — читать как образец контракта):**
`C:\Users\User\.claude\statusline.py` :55-89 — рабочий запрос oauth/usage.

## Общие решения владельца (вшиты в задачи)
- Пороги предупреждений: **85%** (тост) и **99%** (тост+звук+OS-уведомление).
- Автопродолжение: **ВКЛ по умолчанию, без UI-тумблера** (аварийный выключатель — только поле
  в config.json). Текст по языку UI: ru «продолжай» / en «continue» / zh «继续» (i18n-ключ).
- Режим после лимита — настройка `limitMode: wait (default) | switchProfile`.
- ФАКТ проверен на диске 2026-07-03: `~/.claude-*/projects` — JUNCTION на общий
  `~/.claude/projects`, поэтому `claude --resume <sid>` находит транскрипт КРОСС-профильно.

## 5.1 Backend-монитор лимитов (limits.rs)
**Новый файл:** `src-tauri/src/limits.rs`; регистрация в `lib.rs` (mod + start() в setup рядом с
`agent_status::start()`); поля в `HubConfig` (:94-151); команды в generate_handler! + `ipc.ts`.
**Сделать:**
- Фоновый поток, опрос каждые ~5 мин по каждому профилю с OAuth-кредами:
  читать `<home>\<profileDir>\.credentials.json` → `claudeAiOauth.accessToken`;
  `GET https://api.anthropic.com/api/oauth/usage` с заголовками
  `Accept: application/json`, `Authorization: Bearer <token>`,
  `anthropic-beta: oauth-2025-04-20`; таймаут 8 с (ureq, как в кодовой базе).
  Парсить `five_hour.{utilization,resets_at}` и `seven_day.{utilization,resets_at}`.
- Профильные дирки — из `plugin_sync_profiles`/`profile_names()` (НЕ сканировать ~/.claude*).
  Профиль без `.credentials.json` или без accessToken → статус «N/A» (API-key/gateway-профили).
- Токен ТОЛЬКО читать; refresh НЕ делать (риск разлогинить CC). 401 → пометить профиль
  «токен протух», не ретраить агрессивно.
- Событие `limits-status` в UI: `{ profile, h5:{util,resetsAt}, d7:{util,resetsAt}, state }`.
  НЕ логировать токен нигде.
- Предупреждения: пороги 85% (тост) / 99% (тост+звук+OS) — переиспользовать каналы
  statusSounds/statusNotify и notify-инфраструктуру agent_status (notify_transition-образец).
  Антиспам: одно уведомление на порог на окно (ключ = profile+window+resets_at); при смене
  resets_at (новое окно) — сбросить.
- Config: `limitsPoll: bool`(default true), `limitMode: "wait"|"switchProfile"`(default wait),
  `autoContinue: bool`(default true, скрытый аварийный выключатель).
**Тест:** юнит на парс ответа oauth/usage (фикстура JSON) → h5/d7; антиспам (второй раз тот же
порог+окно → без уведомления; новое resets_at → снова можно). **Verify:** cargo test; живой
профиль — % совпадает со statusline. `check:i18n` для новых ключей.

## 5.2 Детект упирания в лимит + состояние `limited`
**Файл:** `src-tauri/src/agent_status.rs` (reader-путь `on_output` :87 / compute :160+;
StatusEvent :109; состояние в enum/строках рядом с working/blocked/idle).
**Сделать:**
- В reader-потоке PTY (там, где `on_output`) сканировать хвост чанка на строки лимита CC:
  «5-hour limit reached ∙ resets», «You're out of extra usage», «usage limit reached» и вариации
  (regex, устойчивый к пробелам/пунктуации). Извлечь время сброса, если присутствует.
- Пометить трек состоянием `limited` (новое) + сохранить `resets_at`. Приоритет: `limited`
  выше working/idle, ниже явного exited. Подтверждение/резерв времени сброса — из данных 5.1
  (endpoint utilization≥100% и его resets_at).
- StatusEvent += `limited` + `resetsAt`; пробросить в ipc.ts + agentStatus.svelte.ts.
- Формат строки может меняться между версиями CC → парс защитить (fail-open, не паниковать),
  fallback на endpoint-детект (5.1).
**Тест:** юнит: реальные строки лимита → detect+parse resets; мусор → нет ложняка.
**Verify:** cargo test; в панель `echo` строки лимита → точка становится `limited`.

## 5.3 Автопродолжение после сброса
**Файл:** новый модуль-петля (в limits.rs или agent_status.rs) + фронт для текста/тостов;
session_write уже есть (backend команда записи в PTY); respawn — инфраструктура restore
(`SessionsTab.svelte` restoreLast :1051, claudeSids map).
**Сделать (ВКЛ по умолчанию):**
- Когда панель `limited` и наступил `resets_at` + джиттер(30–90 с, детерминированный по id — в
  скриптовой среде без rand; в rust rand допустим):
  - живой PTY панели → `session_write` текста продолжения;
  - мёртвой (exited) claude-панели → respawn `claude --resume <claudeSid>` (если claudeSid
    валиден — гейт `^[\w-]{1,64}$`, как в restoreLast) + текст продолжения.
- Текст — по языку UI (`ru «продолжай» / en «continue» / zh «继续»`), i18n-ключ
  `sessions.autoContinueMsg`. Фронт передаёт актуальную локаль (или backend читает config.language).
- МАКСИМУМ 1 авто-продолжение на панель на окно сброса (счётчик по profile+window) — защита от
  цикла. Каждый факт — в лог (spawn_streamed/console) + тост.
- Управляется `autoContinue` из config (default true); off → только помечать `limited`, не слать.
**Тест:** юнит на «пора ли продолжать» (limited + now≥resets+jitter + счётчик<1 → да).
**Verify:** искусственно выставить limited + resets в прошлом → панель получает текст один раз.

## 5.4 Режим switchProfile (переключение на свободный профиль)
**Файл:** limits.rs (выбор кандидата) + respawn-путь (как 5.3).
**Сделать (только при `limitMode == "switchProfile"`):**
- При упирании панели в лимит выбрать кандидата: OAuth-профиль с наименьшей utilization <85%,
  с `linksIntact` (иначе `projects` не junction → --resume не найдёт разговор), не сам себя.
- Kill текущей панели → respawn под кандидатом: `claude --resume <claudeSid>` в env этого
  профиля + текст продолжения (5.3). claudeSid общий через junction projects (проверено).
- Кандидатов нет → fallback на `wait` (поведение 5.3).
- Разные профили = разные аккаунты/подписки — это осознанный выбор пользователя (настройка).
**Тест:** юнит выбора кандидата (набор профилей с util → верный/пусто). **Verify:**
искусственный лимит при switchProfile → панель перезапущена под другим профилем, разговор идёт.

## 5.5 UI: чип лимитов в Сессиях + колонка в Профилях
**Файл:** `src/lib/components/SessionsTab.svelte` (шапка, рядом со status-чипами :1113),
`src/lib/components/ProfilesTab.svelte` (колонка), `src/lib/ipc.ts`, i18n ru/en/zh.
**Сделать:**
- Чип в шапке Сессий: худшее окно среди профилей активных панелей — «5h NN% · сброс HH:MM»
  (или «7d …» если оно ближе к лимиту). Цвет по порогам (85/99) — токены статуса (после 2.2).
- ProfilesTab: колонка utilization (h5/d7 %) + время сброса; «N/A» для не-OAuth.
- Формат времени — существующий relative/date-formatter (DRY, см. `dedup-audit` про
  date/relative-time хелперы — переиспользовать, не плодить).
**Verify:** ?shot обе темы; check:i18n.

## Гейт волны
Все гейты + build_all.ps1. Живой смоук: включить профиль, увидеть чип с реальным %; искусственно
смоделировать limited → авто-продолжение. ff-merge в main.
