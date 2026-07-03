# Волна 1 — Hardening (security + подтверждённые баги)

Все 6 багов лично верифицированы 2026-07-03. Ветка `improve-0703/wave-1`.

## 1.1 Инъекция cmd-метасимволов при деплое MCP на Codex — SECURITY
**Файл:** `src-tauri/src/lib.rs` ~:6470-6535 (функция деплоя MCP на codex).
**Сейчас:** env-значения из `.mcp.json` кладутся в argv как `{k}={val}` без фильтра символов,
затем `Command::new("cmd").arg("/C").arg("codex").args(&argv)` (:6528-6529). cmd повторно
парсит аргументы: `& | < > ^ %` исполняются (класс BatBadBut). Отравленный `.mcp.json` с env
`"X":"a&calc.exe"` запускает calc при «деплой MCP на Codex».
**Сделать:** отклонять значения с `& | < > ^ " %` и управляющими символами ДО сборки argv —
по образцу существующего charset-фильтра plugin-id (см. ~:7130, найди по `plugin`+валидация);
ошибка через `tr(...)` (новый ключ `err.codex_env_chars` в `src-tauri/src/i18n.rs`, паритет
ru/en/zh там же — это НЕ frontend-локали).
**Не делать:** не переходить на прямой запуск codex.exe (это npm `.cmd`-шим, ему нужен cmd /C
— комментарий :6526-6527 объясняет).
**Тест:** юнит в `lib.rs` tests: значение `a&b` → Err; чистые значения (url, путь, npx-строка)
→ Ok. **Verify:** cargo test; вручную не запускать codex.

## 1.2 Утечка API-ключа на невалидированный base_url — SECURITY
**Файл:** `src-tauri/src/lib.rs`, `fn probe_provider` :4449-4463 и его вызовы
(`run_provider_check`, `check_my_provider` ~:4536, balance-проверки).
**Сейчас:** `probe_provider(base_url, protocol, api_key)` шлёт `Authorization: Bearer <ключ из
Credential Manager>` на URL из myproviders.json БЕЗ валидации: `valid_base_url` не вызывается
вовсе, а сама она (~:3693) пропускает `http://`. Подменённый (например, синком) baseUrl →
ключ уходит открытым текстом.
**Сделать:** в начале `probe_provider` — парс URL: схема обязана быть `https`, ЛИБО `http` только
для хостов `localhost`/`127.0.0.1`/`[::1]` (локальные шлюзы типа freellmapi легитимны). Иначе —
вернуть JSON-ошибку как у существующих error-путей функции. Тот же гейт добавить в
balance-путь, если он строит запрос отдельно (найди `fetch_provider_balance`).
**Тест:** юнит: `http://evil.com` → отказ; `http://localhost:3999` → проходит; `https://…` → проходит.
**Verify:** cargo test + живой «проверить провайдера» на существующем https-провайдере.

## 1.3 Maximize + «в фон» = мёртвый грид ⚠ same-file (с 1.4 не пересекается)
**Файл:** `src/lib/components/SessionsTab.svelte`.
**Сейчас:** `toggleBackground` (:680-682) не сбрасывает `maximized`; грид рендерит
`activePanes = panes.filter(p => !p.background)` (:596, `{#each activePanes}` :1363), панели
скрываются `class:hidden={maximized != null && maximized !== pane.key}` (:1364). Максимизировал
панель A → отправил A в фон → `maximized` всё ещё A, A не в activePanes → все остальные скрыты,
грид пуст до ручного «Restore». Плюс maxbar (:1343) итерирует ВСЕ `panes`: клик по чипу фоновой
панели ставит `maximized` на панель вне грида — тот же мёртвый экран.
**Сделать:** (а) в `toggleBackground`: если `maximized === key` → `maximized = null`;
(б) maxbar итерирует `activePanes` вместо `panes`.
**Verify:** ?shot: 3 панели → maximize A → фон A → грид показывает B,C; maxbar без чипа A.

## 1.4 Send-to-all пишет в завершённые сессии
**Файл:** `src/lib/components/TerminalPane.svelte` :363-368.
**Сейчас:** листенер `pty:exit:` ставит `exited = true`, но `onIdChange?.(paneKey, null)`
вызывается только в onDestroy (:545). Мёртвая, но открытая панель остаётся в `sessionIds`
родителя → send-to-all (`SessionsTab.svelte` `doSendToAll` ~:384) целит её и завышает счётчик
в confirm.
**Сделать:** в pty:exit-листенере после `exited = true` вызвать `onIdChange?.(paneKey, null)`.
**Проверь побочку:** persist live-набора (`SessionsTab.svelte` :349-354) фильтрует по
`sessionIds[p.key]` — завершённая панель выпадет из LIVE_KEY; это корректно (умершую сессию
не re-attach'ат), но убедись, что restore-бар и duplicate не ломаются.
**Verify:** ?shot/юнит: панель с exited → не входит в цели send-to-all; счётчик confirm верный.

## 1.5 Busy-state форков течёт при дропе future
**Файл:** `src-tauri/src/lib.rs`, `run_fork_repo` :851-934 (снятие :929-932), `run_forks`
:802-850 (аналог с FORKS_GLOBAL ~:820-826), `cancel_fork_repo` :956.
**Сейчас:** путь удаляется из `ForkRuns` ПОСЛЕ `.await` — не RAII. Tauri дропает futures
in-flight команд при перезагрузке webview (F5 / WebView2-recovery) → запись остаётся навсегда
→ вечный `err.fork_busy` для этого репо (и блок глобального прогона).
**Сделать:** RAII-guard (struct с `Drop`, снимающий запись из ForkRuns / флаг FORKS_GLOBAL) по
образцу существующих `RunSlot`/`BulkSlot` (найди их в lib.rs — канон уже в кодовой базе).
Учти оба места + ранний return при spawn-ошибке (:915-919 станет не нужен — Drop покроет).
**Тест:** юнит: guard создан → запись есть; drop → записи нет (без запуска pwsh — вынеси
guard в отдельную тестируемую единицу).
**Verify:** cargo test.

## 1.6 Откат ключа провайдера теряет мигрированный legacy-слот
**Файл:** `src-tauri/src/lib.rs`, `add_provider_key` :3992-4020, `remove_provider_key` :4026+.
**Сейчас:** при первом добавлении legacy-ключ `provider:{id}` мигрируется в `:0` с УДАЛЕНИЕМ
оригинала (:4003-4005); если `write_myproviders_raw` падает (:4015), rollback удаляет только
новый верхний слот (:4016) — legacy уже потерян, `active_provider_key` читает удалённый ключ.
**Сделать:** не удалять legacy до успешной записи JSON: порядок — `kr_set(:0)` → `kr_set(top)`
→ `write_myproviders_raw` → только при Ok: `kr_delete(legacy)`. При Err — удалить созданные
слоты, legacy не тронут. Проверь `remove_provider_key` на тот же класс (survivors-перезапись
:4034+): падение записи не должно оставлять слоты в полустёртом виде — восстанови исходные
слоты при Err.
**Тест:** юнит с mock/fake keyring-слоем, если он есть; иначе — логический тест порядка через
рефакторинг в чистую функцию. **Verify:** cargo test.

## Гейт волны
cargo test / clippy / npm run check / npm test / npm run build / build_all.ps1 → ff-merge в main.
