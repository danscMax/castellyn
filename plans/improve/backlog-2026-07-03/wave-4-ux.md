# Волна 4 — UX-переделки

Ветка `improve-0703/wave-4`. 4.4/4.5 требуют выполненной 2.2 (токены статуса). 4.2/4.5/4.6
правят SessionsTab — сериализуй.

## 4.1 Не уничтожать состояние вкладки при переключении — USABILITY (impact HIGH)
**Файл:** `src/routes/+page.svelte` :1982 (`{#key active}` вокруг рендера вкладок),
прецедент keep-mounted — Sessions (:2101, монтируется отдельно и переживает переключения).
**Сейчас:** `{#key active}` пересоздаёт компонент активной вкладки при каждом переключении
(ради fade). Теряется транзиентное состояние: ProvidersTab — результаты health/balance
(`health`, `balance`), раскрытые Details (`advOpen`), панели ключей (`keysOpen`), инлайн-редактор
эндпоинта; ProfilesTab — раскрытые строки, редактор shared-folders; PluginsTab — раскрытые
строки, модалка changelog.
**Сделать:** держать посещённые вкладки смонтированными, переключать видимость через
`display:none` (не unmount). Fade перенести на CSS-класс по смене `active`, без `{#key}`.
ВНИМАНИЕ: часть вкладок рассчитывает на remount для refresh данных (focus-refresh логика,
подписки на события) — проверь каждую: данные грузятся в `+page`, но если компонент имеет
onMount-загрузку, добавь reactive-refresh при повторной активации, чтобы данные не устаревали.
**needs_confirmation:** keep-mounted ВСЕ вкладки vs только дорогие (Providers/Profiles/Plugins).
Рекомендую все с лёгким состоянием mount'ить лениво и не размонтировать после первого показа;
если какая-то вкладка тяжёлая по памяти — оставь ей remount. Отметь решение.
**Verify:** health-check провайдера → уйти на другую вкладку → вернуться → результат на месте.
Проверь, что данные не «замерзают» устаревшими. npm run check.

## 4.2 SSH-статус: emoji-светофор → канон .dot ⚠ same-file — VISUAL
**Файл:** `src/lib/components/SessionsTab.svelte` :827 (дропдаун локации),
:1281 (чип сервера в ⚙); класс `.dot`/`dot-pulse` уже есть (TerminalPane / app.css).
**Сейчас:** `sshReach ? '🟢' : … '🔴' : '⚪'` — единственный не-темизируемый emoji-статус в
приложении; ⚪ «проверяется» читается как «выключено».
**Сделать:** заменить emoji на `<span class="dot ok/fail">` (те же токены статуса, что после
2.2), а на время активной проверки (`checkReach` в полёте) — класс `dot-pulse` (пульс =
«идёт проверка»), а не серый кружок. Оба места (:827, :1281).
**Verify:** ?shot обе темы: ok/fail/checking различимы, checking пульсирует.

## 4.3 Один факт — единый цвет/словарь blocked/done (сайдбар ↔ вкладка) — VISUAL
**Файл:** `src/lib/attention.ts` :60-63 (`sessionsAttention`: blocked→warn, done→info),
`src/lib/components/Sidebar.svelte` (`att-warn`=--sw-warn, `att-info`=--sw-accent ~:433/440),
внутри вкладки: blocked=`--sw-danger` (TerminalPane :761, SessionsTab :1503), done=teal;
словарь: тултип `ru/sessions.ts:8` «Агент закончил — не просмотрено», чип «{n} готово»
(`sumDone`), сайдбар — молчаливая точка.
**Сейчас:** blocked = амбер в сайдбаре / красный во вкладке; done = синий / teal; три
формулировки одного состояния.
**Сделать:** согласовать. Рекомендую: (а) цвет — blocked→danger-уровень в сайдбаре (добавить
`att-danger` если нет), done→свой teal-уровень; ЛИБО перекрасить вкладку под сайдбар — выбери
меньший диф, но результат: один факт = один цвет везде. (б) единая формулировка «готово»
(убрать «не просмотрено» из тултипа ИЛИ применить везде) — ru/en/zh.
**needs_confirmation:** расширять Attention-уровни vs перекрасить вкладку. Отметь.
**Verify:** ?shot: точка в сайдбаре и во вкладке одного цвета для blocked и для done.

## 4.4 «Обновить всё»: авто-recheck + прогресс по компонентам — USABILITY
**Файл:** `src/routes/+page.svelte` :1748 (авто-recheck `if (… && c.id !== 'all')`),
`src/lib/components/UpdatesTab.svelte` :26-27 (all-карточка убрана из грида, спиннеры
`busy={running === c.id}` :94), refreshing-пилюля только для forks (:1317-1321).
**Сейчас:** после «Обновить всё» карточки остаются с устаревшим «доступно обновление» (recheck
исключает `all`); whole-stack прогон выглядит зависшим — видны только 2 серые кнопки.
**Сделать:** (а) после успешного all-apply запустить `startRun('all', 'check')`, чтобы карточки
самолечились (как одиночный apply). (б) прогресс: парсить `[<component>]`-префиксы из стрим-лога
`all`-прогона и подсвечивать текущий компонент (спиннер/подпись) либо показывать «проверяю
<name>…» в roll-up-заголовке.
**Verify:** живой «Обновить всё» → по завершении карточки актуальны, во время — видно движение.

## 4.5 Создать один профиль без reinstall всех — USABILITY ⚠ same-file (SessionsTab не трогает)
**Файл:** `src/lib/components/ProfilesTab.svelte` :428 (кнопка «Создать» → `onAction('reinstall')`),
`src/routes/+page.svelte` `onProfileAction('reinstall')` :609-615 (danger-confirm с requireText).
**Сейчас:** «Создать» для missing-профиля запускает полный whole-stack reinstall ВСЕХ профилей
с вводом слова-подтверждения.
**Сделать:** точечное создание одного профиля. Проверь, поддерживает ли PS-установщик параметр
имени профиля (grep в `SCRIPTS_ROOT` / manifest / существующие вызовы reinstall); если да —
новый backend-путь/аргумент «создать профиль <name>» + лёгкий confirm. Если нет — Rust-native
создание каталога профиля по модели существующих (junction `projects`→`~/.claude/projects`,
симлинки shared-элементов как у других профилей — см. sync/линки-логику). Начни с рекона:
как вообще создаётся профиль сейчас.
**needs_confirmation:** PS-параметр vs native. Отметь по результату рекона.
**Verify:** удалить каталог тест-профиля → «Создать» восстанавливает ТОЛЬКО его, остальные не
тронуты. cargo test / npm run check.

## 4.6 Sessions-персонализация в экспорт/бэкап — FUNCTIONAL (impact HIGH) ⚠ same-file
**Файл:** `src/lib/components/SettingsTab.svelte` :169-192 (export/import сериализует
`HubConfig`), `src-tauri/src/lib.rs` `HubConfig` :94-151, localStorage-ключи в
`SessionsTab.svelte`: workspaces (WKEY :94), favorites (VKEY :881), projects root (:694),
default args (:95), ширины колонок (:603), layout мониторов (:97), recent dirs, scrollback
(`SettingsTab.svelte` :207-216).
**Сейчас:** вся персонализация Sessions — только в per-webview localStorage; export/backup
переносят один HubConfig → на второй машине / после переустановки всё теряется молча.
**Сделать:** добавить блок `sessionsPrefs` (workspaces, favorites, projectsRoot, defaultArgs,
columnWidths, monitorLayout, scrollback) в экспортируемую модель. Вариант А: поля в HubConfig
(бэкенд знает). Вариант Б: отдельный экспортируемый sidecar-файл + захват в backup/restore.
При первом чтении — миграция из localStorage (не потерять текущие настройки пользователя).
Не создавай двойной источник истины навсегда: определи, что localStorage — кэш, а канон — в
конфиге (или наоборот), и синхронизируй в одну сторону.
**needs_confirmation:** HubConfig vs sidecar-файл. Рекомендую sidecar (Sessions-специфика, не
раздувает HubConfig), захваченный существующим backup/restore. Отметь.
**Verify:** настроить workspace+favorite → экспорт → чистый импорт (или др. профиль appdata) →
настройки на месте. npm run check / cargo test.

## 4.7 Клавиатурный scrollback + прыжок к панели — FUNCTIONAL
**Файл:** `src/lib/components/TerminalPane.svelte` :484 (`attachCustomKeyEventHandler` — сейчас
copy/paste/find/new-session), `src/lib/components/SessionsTab.svelte` onKey :1008-1016
(Alt+1/2/3 = число колонок, Ctrl+]/[ = цикл фокуса).
**Сейчас:** прокрутка буфера только колесом мыши (недоступно, когда TUI на alt-screen забирает
wheel — grep `scrollLines|PageUp|copyMode` пуст); к панели 5 из 12 — только циклом.
**Сделать:** (а) в `attachCustomKeyEventHandler`: Shift+PageUp/PageDown → `term.scrollLines(∓N)`;
Ctrl+Home/End → `term.scrollToTop()/scrollToBottom()`. ПРОПУСКАТЬ, когда активен alt-screen
(приложение вроде vim/claude-TUI управляет экраном) — проверь `term.buffer.active.type` или
DECSET 1049, чтобы не перехватывать у TUI. (б) Alt+N (N=1..9) → фокус панели N; текущие колонки
переехать на Ctrl+Alt+N (обнови подсказку хоткеев + i18n).
**needs_confirmation:** куда переносить Alt+1-3 (колонки). Рекомендую Ctrl+Alt+N. Отметь.
**Verify:** живая панель (npm run tauri dev): Shift+PgUp прокручивает историю, но vim внутри
панели получает свой PgUp; Alt+3 фокусирует 3-ю панель. npm run check.

## Гейт волны
Все гейты + build_all.ps1 → ff-merge в main.
