# Волна 2 — Quick wins (effort S, высокая отдача)

Ветка `improve-0703/wave-2`. Задачи 2.4/2.6 правят SessionsTab — сериализуй их между собой
и с любыми задачами волны 4, если пойдут параллельно.

## 2.1 Единый appendLog() + backend-батч + rAF-автоскролл — PERF
**Файлы:** `src/routes/+page.svelte` (8+ мест `log = [...log, …].slice(-MAX_LOG)`:
:257, :266, :455, :1151, :1158, :1704, :1709, :1839, :1848, :1858 и без slice :1412/1476/1486/1754/1901),
`src/lib/components/Console.svelte` :62-65, `src-tauri/src/lib.rs` эмиттер run-log ~:596-623.
**Сейчас:** каждая строка пересоздаёт массив до MAX_LOG=5000 (O(n) на строку); backend шлёт
одно IPC-событие на строку без батча; Console автоскролл читает `scrollHeight` (forced reflow)
на каждую строку.
**Сделать:**
(а) добавить `function appendLog(line: string) { log.push(line); if (log.length > MAX_LOG)
log.splice(0, log.length - MAX_LOG); }` — заменить ВСЕ `log = [...log, X].slice(-MAX_LOG)` и
голые `log = [...log, X]` на `appendLog(X)`. (Svelte 5: `log` должен быть `$state` массивом —
если сейчас переприсваивание было ради реактивности, проверь, что мутация `.push` реактивна в
рунах; если нет — используй `log = [...]` только там, где массив реально меняется целиком, а
для горячего пути введи батч-буфер, см. б.)
(б) backend: буферизировать run-log строки ~30 мс и слать пачкой (Vec<String> в payload);
фронт добавляет пачку одним `log.push(...batch)`. НЕ ломай формат `run-done`.
(в) Console.svelte: автоскролл через один `requestAnimationFrame`, не на каждую строку.
**Verify:** живой `forks check` (много строк): консоль плавная, порядок строк сохранён (FIFO),
`run-done` приходит. npm run check / npm test.
**needs_confirmation:** батч на backend (б) можно отложить, если (а)+(в) достаточно — реши по
замеру, отметь в отчёте.

## 2.2 Токены статус-цветов ⚡ (блокер для 4.4/4.5) — VISUAL
**Файлы:** `src/app.css` (~:95-101 блок токенов + light-override секция),
`src/lib/components/TerminalPane.svelte` :750, :756, :764,
`src/lib/components/SessionsTab.svelte` :1298, :1480, :1507, :1510.
**Сейчас:** `var(--sw-status-warn, #e0b341)` в 4 местах — токен `--sw-status-warn` НЕ определён
в app.css (есть только `--sw-warn: #f59e0b` и `--sw-status-degraded: #f59e0b`) → всегда фолбэк
#e0b341, рядом с реальным #f59e0b два разных амбера. `#2dd4bf` (done) захардкожен (:1510, :764)
без light-override — ровно антипаттерн, задокументированный в `src/lib/statusColor.ts:1-6` как
УЖЕ чинённый WCAG-баг. `#3fb950` (ssh ok, :1298) — off-palette.
**Сделать:** в app.css определить `--sw-status-warn: var(--sw-warn);` и
`--sw-status-done: #2dd4bf;` + light-override `.light { --sw-status-done: #0d9488; }` (teal-600,
проверь контраст ≥4.5:1 на `--sw-bg` светлой темы). Заменить хардкоды #2dd4bf → var(--sw-status-done),
#3fb950 → существующий `--sw-status-up` (#10b981) или новый токен с light-override. Канон —
`.status-*` из statusColor.ts (dark *-400 / light *-700).
**Verify:** npm run check; ?shot обе темы — амберы совпадают, «готово» читаемо на светлой.

## 2.3 Empty state Sessions → общий EmptyState.svelte ⚡ (DRY) — VISUAL
**Файл:** `src/lib/components/SessionsTab.svelte` :1458-1467 (+ CSS `.empty`/`.empty-icon` ~:1894),
образец — `src/lib/components/EmptyState.svelte` (Lucide `<Icon size={32} strokeWidth={1.5}>`,
классы `.empty-title`/`.empty-desc`), как в Forks/MCP/Analytics.
**Сделать:** заменить ручной `<div class="empty"><div class="empty-icon">▦</div>…` на
`<EmptyState>` с подходящей Lucide-иконкой (напр. `TerminalSquare`), title `sessions.emptyTitle`,
desc `sessions.emptyHint` и кнопкой запуска (слот или проп). Удалить осиротевший CSS `.empty*`,
если больше не используется (grep по файлу перед удалением).
**Verify:** ?shot пустой вкладки обе темы; npm run check.

## 2.4 Ctrl+K-палитра: тост «занято» вместо молчаливого no-op ⚠ same-file (+page) — USABILITY
**Файл:** `src/routes/+page.svelte` :1571-1593 (verbs палитры зовут startRun/onStack/startBackup),
раннеры с `if (running) return;` (:339, :564, :1079, :1128, :1233 и т.д.).
**Сейчас:** verbs палитры вызывают раннеры напрямую; при активном прогоне — тихий no-op
(комментарий :1571-1572 признаёт «harmless»). Пользователь набрал команду → Enter → ничего.
**Сделать:** обёртка `function runOrToast(fn) { if (running) { toast(t('page.busy_running',
{ id: running })); return; } fn(); }` (используй существующий toast-механизм проекта) —
обернуть ею verbs палитры, которые запускают прогоны. Новый i18n-ключ `page.busy_running`
(ru/en/zh). НЕ трогай кнопки во вкладках (там `disabled={busy}` уже корректен).
**needs_confirmation:** тост (рекомендую) vs очередь команд — бери тост, очередь избыточна.
**Verify:** запусти долгий прогон, Ctrl+K → «apply …» → виден тост; npm run check:i18n.

## 2.5 Запуск при лимите панелей: disable ▶ + тост о частичном restore — USABILITY
**Файл:** `src/lib/components/SessionsTab.svelte` :538 (`if (atLimit && !v.attachId) return;`),
кнопки запуска (:1210 launchPhrase, :901 launchFav, :1045 launchWorkspace, :937 duplicate),
`restoreLast` :1051-1065, `restoreLayout`/`launchWorkspace` (циклы addPane).
**Сейчас:** кнопка ▶ активна при `atLimit`, клик — тихий no-op; restore/workspace при упоре в
MAX_PANES/SESSION_LIMIT молча отбрасывают лишние панели.
**Сделать:** (а) `disabled={atLimit}` (или `atLimit && !attach`) на launch-кнопках + tooltip
с причиной; (б) в restoreLast/launchWorkspace считать отброшенные и, если >0, тост
`sessions.restorePartial` c «N из M» (ru/en/zh).
**Verify:** ?shot при 12 панелях: ▶ задизейблена; частичный restore даёт тост.

## 2.6 Три мелочи сессий на готовых данных ⚠ same-file (Sessions/TerminalPane/agent_status) — FUNCTIONAL
**Файлы:** `src/lib/components/SessionsTab.svelte` (LivePane :339, WsConfig :105, Fav :880),
`src/lib/components/TerminalPane.svelte` (onBell — отсутствует; unread через onActivity),
`src-tauri/src/agent_status.rs` (Track.spawned_at/last_output :74/:87, StatusEvent :109),
`src/lib/agentStatus.svelte.ts`, `src/lib/ipc.ts` (AgentStatusEvent).
**Сделать 3 независимых под-правки (можно 3 коммита):**
(2.6a) Персист имени панели: добавить `name?: string` в `LivePane` (:339 + маппинг :353),
`WsConfig` (:105) и `Fav` (:880 + сборка/применение). Rename уже есть (:81 renamePane);
цель — чтобы имя переживало рестарт (restoreLast), workspace и favorite.
(2.6b) `term.onBell`: подписать `term.onBell(() => onActivity?.(paneKey))` в TerminalPane
(где создаётся term, рядом с :525 focusin) → BEL от shell/SSH-панелей (agent-status их не
покрывает) поднимает unread-маркер (SessionsTab :234).
(2.6c) Elapsed в статусе: добавить в StatusEvent поля `spawnedAt`/`lastOutput` (или
готовые «работает N мин»/«тишина N мин»), пробросить в ipc.ts + agentStatus.svelte.ts,
показать в тултипе точки TerminalPane. НЕ спамить: отдавать при смене минуты, не каждый тик.
**Verify:** rename → рестарт приложения → имя на месте; `printf '\a'` в shell-панели →
подсветка; тултип точки показывает время. cargo test / npm run check / check:i18n.

## Гейт волны
Все гейты + build_all.ps1 → ff-merge в main.
