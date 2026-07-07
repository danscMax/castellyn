# OmniRoute как единый шлюз LLM-стека Castellyn — дизайн (черновик на ревью)

> Дата: 2026-07-07 · Статус: **DRAFT, ждёт подтверждения владельца** · Ветка реализации: ещё не создана
> Метод: заземлено мультиагентной верификацией (41 гипотеза, 30 подтв. по `file:line`) + офиц. доки OmniRoute/OpenCode/freellmapi (2026).

## 1. Цель

Сделать **OmniRoute (`:20128`) единственным входом** для Claude Code, OpenCode и Codex, за которым живёт весь наш free-стек. Требования владельца: максимальная стабильность и удобство; тривиальное добавление новых провайдеров/эндпоинтов; **мониторинг, где пользователь прямо в Castellyn сразу видит, что аккаунт/ключ отвалился или исчерпан и его надо пересоздать**; поддерживаемость вторым разработчиком; наши расширения (мультиаккаунтность) переживают апдейты апстримов.

## 2. Целевая топология (ревизия Опции 1)

```
Claude Code   ─ ANTHROPIC_BASE_URL=http://localhost:20128/v1 ─┐
OpenCode/Codex ─ OpenAI baseURL   =http://localhost:20128/v1 ─┼─► OmniRoute :20128  (единый фронт + мозг ротации)
                                                              │      ├─ 16 key-based провайдеров (Google/Groq/Cerebras/NVIDIA/Mistral/
                                                              │      │   Cohere/Cloudflare/HF/GitHub Models/Zhipu/… ) с мульти-ключом
                                                              │      ├─ 90+ прочих free/paid провайдеров OmniRoute + RTK/Caveman-сжатие
                                                              │      ├─ Qwen  :3264  (custom OpenAI-провайдер; внутр. ротация аккаунтов — в движке)
                                                              │      ├─ DeepSeek :9655 (custom; внутр. ротация)
                                                              │      ├─ GLM-Kimi :9766 (custom; внутр. ротация)
                                                              │      ├─ g0i :8788 (custom; captcha-budget)
                                                              │      └─ zcode :3847 (после git-клона приватного репо; custom)
                                                              └─ freellmapi :13001 = FALLBACK на время перехода → затем ретайр
```

**Принцип ротации:** OmniRoute владеет ротацией на уровне **провайдер/ключ**. Ротация *скрытых* аккаунтов ВНУТРИ каждого локального движка (браузерные z.ai/Qwen/DeepSeek-аккаунты за одним портом) остаётся в самом движке — OmniRoute до них не дотягивается. Это не дубль и не костыль, это неизбежно. Костыль freellmapi (его собственный multi-key pool на ветке `wip-local`) — убираем.

## 3. Решения владельца (зафиксированы)

| # | Вопрос | Решение |
|---|--------|---------|
| D1 | Реакция супервизора на падение | **Аларм (toast+бейдж) обязателен + опт-ин авто-рестарт** с backoff и лимитом попыток |
| D2 | Ротация мультиаккаунтов | OmniRoute — на уровне провайдер/ключ; движки держат внутр. per-account ротацию; **ретаир rotation-костыля freellmapi** |
| D3 | Как добавлять провайдера | **Дашборд/API OmniRoute — канон**; форму `addKey` перенацелить на provider-API OmniRoute; stack.json правится руками только для редкого нового *сервиса* |
| D4 | Судьба freellmapi | **Полный ретайр**: 16 key-провайдеров → в OmniRoute с мульти-ключом; freellmapi держим fallback'ом до подтверждения ёмкости на живом гейте |
| D5 | zcode | **git-клон приватного `Asati-Privatka` + `Update-Zcode.ps1`**, перенос живых `data/`+`.env`, потом вешать за OmniRoute |
| D6 | Форк-апдейт | **Generic `fork-updater` — канон**; `sync-freellmapi.cmd` убрать; `Update-FreeLLMAPI.ps1` → только build+restart (с фиксом false-green и порта) |

## 4. Заземлённые факты (ключевое из верификации)

- **OmniRoute запускается как обычная запись stack.json** — супервизор спавнит через `cmd /c chcp 65001>nul & <command>` с полным PATH (вкл. `%APPDATA%\npm`), env/health/dashboard уже поддержаны как чистые данные манифеста (`lib.rs:3002-3008, 3609-3646, 3192-3200`). Требуется: реальный `dir` (иначе `[skip]`), предварительная установка (`where omniroute` сейчас пусто), и подтверждённый long-running serve-подкоманд.
- **«Критичный фронт» требует кода:** `StackHealthCard.svelte:48/57/64` хардкодит `id==='gateway'` единственным критичным хопом → новый сервис рисуется нейтральным серым при падении. `id 'gateway'` **перегружен**: `gateway_base_url()` (`lib.rs:5866`) кормит И регистрацию upstream'ов freellmapi (`:13001`, строка 6099), И OpenAI base_url Codex (строка 8851). Нельзя переиспользовать/переименовать — нужен отдельный id `omniroute` и caller-aware разводка.
- **OpenAI-клиенты:** арм `("direct","openai")` (`lib.rs:6086`) жёстко отклонён — надо разблокировать, чтобы OpenCode/Codex били в `:20128/v1` как чистый конфиг. Claude Code кода не требует (реюз арма `direct/anthropic`).
- **Мониторинг сейчас = только порт+один 2xx** (`StackHealth`, `lib.rs:3648-3660`): нет поля аккаунт/ключ/квота; **нет даже опроса-пока-открыто** (`StackHealthCard` грузит `load()` один раз на mount + ручной refresh); **нет пост-стартовой супервизии/авто-рестарта** (`native_stack_start` одноразовый, `StackProcs` читается только на stop).
- **Детект здоровья аккаунтов уже машинно-читаем на бэкендах:** freellmapi `GET :13001/api/health` (per-key статус, авто-дизейбл после 3× 401/403); DeepSeek `GET :9655/api/accounts` (`OK|WAIT|EXPIRED|INVALID` + `POST /:id/check`); GLM-Kimi `GET :9766/admin/accounts` (`ok/failCount/lastError`); g0i — captcha-budget через `solver.py balance()`. **Castellyn их не опрашивает.** Плюс нет плитки/тоста для аккаунтов (`attention.ts` не знает про ключи).
- **«Половина через chrome»** = MV3-расширение `FreeDeepseekAPI\chrome-extension\` (тянет токен DeepSeek → `:9655/api/accounts/import`) + браузерная ре-авторизация Qwen (puppeteer), GLM-Kimi (`npm run auth:browser`), zcode (Playwright). **Детект машинный, RECOVERY ручной** (браузерный ритуал). Под OmniRoute эти per-account сигналы НЕВИДИМЫ через `:20128` — опрашивать надо порты движков напрямую.
- ⚠ **`Update-FreeLLMAPI.ps1` — сломан:** на `wip-local` сравнивает с несуществующим `origin/wip-local` → крэш до записи статуса → `finally` пишет **false-green `ok`**, апстрим-фиксы молча не приезжают. Плюс хардкод `:3001` (сервис на `:13001`) — при включении Scheduled Task ложный smoke-OK + collateral-kill чужого процесса на 3001.
- **Нет единого реестра «что за сервисы/провайдеры есть»:** `maintenance-manifest.json` — реестр task'ов (без port/health), реальные сервисы — в отдельном репо `llm-stack\stack.json`, custom-провайдеры — в `myproviders.json`+Credential Manager, upstream'ы freellmapi — в его SQLite, DATA_DIR OmniRoute — пятый стор. Backend **read-only над stack.json** (write-пути нет).

## 5. Мониторинг здоровья аккаунтов (ядро запроса #3)

Новый слой, независимый от OmniRoute-миграции (ценен сам по себе):

1. **Фоновый health-loop** — таймер, переиспользующий `read_stack_health_blocking` (`lib.rs:3672`) по паттерну `limits.rs:241`; эмитит run-log/toast на переходе-в-down (сейчас падение видно только на mount/refresh).
2. **Per-account ридеры** (бьют по портам движков НАПРЯМУЮ, т.к. через `:20128` невидимы): freellmapi `/api/health` (ре-логин из keyring за Bearer), DeepSeek `/api/accounts`, GLM-Kimi `/admin/accounts`, g0i captcha-balance, и — когда поднимется — `~/.omniroute` SQLite (DATA_DIR). Реюз `balance_get/extract_balance` (`lib.rs:6389`).
3. **Attention+toast** — `accountsAttention()` по образцу `forksAttention` → в сайдбар-роллап + `outcome.ts`; отдельное состояние **«нужна ре-авторизация»** с deep-link на нужный ритуал (клик расширения для DeepSeek; `npm run auth:browser` для GLM/Kimi/zcode).
4. **Health-path OmniRoute:** объявить НЕ-пустой health на роут, который 2xx-ит ТОЛЬКО когда реально маршрутизирует (кандидат `/v1/models`, не дашборд-рут `CHANGEME`, который 200-ит даже заклинив). Проверить на гейте.

## 6. Фазированный план (порядок = приоритет доверия к мониторингу → потом миграция)

- **Ф0. Pre-integration гейт (перед любой проводкой):** `npm i -g omniroute`; подтвердить `where omniroute` и long-running serve-подкоманд (`launch` vs one-shot); пробить, какой роут даёт неавторизованный 2xx только-при-маршрутизации (`/v1/models`); **проверить ёмкость мульти-ключа** для key-based провайдеров. Пока не установлено — ничего не предполагаем.
- **Ф1. Починить «врущий» мониторинг (независимо от OmniRoute):** де-хардкод `Update-FreeLLMAPI.ps1` `:3001→:13001` + null-guard `git rev-parse` (стоп false-green); фоновый health-loop.
- **Ф2. Слой per-account health** (см. §5, п.2) — ридеры по портам движков + OmniRoute SQLite.
- **Ф3. Attention+toast** (§5 п.3) + «нужна ре-авторизация» с deep-link.
- **Ф3.5. Supervisor hardening (portfolio-аудит 2026-07-06) — ПРЕРЕКВИЗИТ к единому фронту.** `stackNative` уже дефолт (`unwrap_or(true)`) и несмокан, а аудит нашёл 5 багов, помеченных «fix BEFORE enabling» → они уже в дефолтном пути и при едином фронте бьют сильнее: **CAST-2** старт рапортует `code:0` даже если сервис не поднялся (false-green; пробросить реальный exit через `native_stack_start`→`run_stack`) — стартовый близнец Ф1-фикса; **CAST-1** двойной `run-done` при restart; **CAST-3** single-stop ставит глобальный `STACK_CANCEL`, гася full-start (только при `only=None` / per-id set); **CAST-4** port-kill после успешного pid-kill (гейт `if !killed`); **CAST-5** `STACK_CANCEL` не проверяется в цикле `native_wait_ready`. + **`readyTimeoutSec` на сервис в stack.json** (llm-stack-2), читаемый И нативным `native_wait_ready` (не только PS-лаунчером) — нужно OmniRoute (движки cold-start >25с). + `glm-router` `ROUTER_SECRET` (llm-stack-1), раз держим его fallback'ом.
- **Ф4. ✅ СДЕЛАНО (Plan 3):** развели перегруженный `id 'gateway'` — добавлен параллельный `omniroute_base_url()` + запись `id 'omniroute'` в stack.json (`enabled:false`, port 20128, `critical:true`); `gateway_base_url()`=:13001 нетронут (аддитивно).
- **Ф5. ✅ СДЕЛАНО (Plan 3):** хардкод `id==='gateway'` в `StackHealthCard` заменён на data-driven `critical` в `StackHealth`; `gateway` помечен `critical:true` (регрессии ноль).
- **Ф6. ⚠ РЕВИЗИЯ (Plan 3b) — премисса была неточной, см. §13.** «Разблокировать арм `("direct","openai")`» СНЯТО как обсолетное: арм таргетит Claude-профиль, а не OpenCode/Codex; Claude идёт через `direct/anthropic` на :20128, OpenCode уже умеет любой base_url. Реальная дыра — **только Codex** (`patch_codex_gateway` был захардкожен на freellmapi:13001). ✅ Код-часть сделана в Plan 3b: `patch_codex_provider(...)` параметризован + `run_codex_omniroute` (за живым `/v1/responses`-чеком). Проводка Claude/OpenCode — ноль кода (§13).
- **Ф7. ✅ Код-сеамы сделаны (Plan 3b), живое отложено:** `order_services()` (dependsOn топо-сорт, фронт после upstream'ов) + teardown-on-critical-failure (kill_tree только своего run'а) + конфигурируемый `healthTimeoutSec` — всё юнит-тестировано, opt-in через manifest-поля (инертно без них). **Живое (владелец):** `omniroute providers add/keys add` для движков + 16 key-провайдеров, отключить provider-retry для нестабильных upstream'ов, таймаут > worst-case, реальный health-роут + DATA_DIR-env + `enabled:true` + `dependsOn`.
- **Ф8. Опт-ин авто-рестарт** фронта (backoff+cap) — по D1.
- **Ф9. Консолидация форк-апдейта (D6):** generic `fork-updater` каноном; `sync-freellmapi.cmd` убрать; `Update-FreeLLMAPI.ps1` → build+restart; запушить `wip-local` (бэкап для 2-го дева).
- **Ф10. zcode (D5):** git-клон приватного репо + `Update-Zcode.ps1` + перенос `data/`+`.env`; пока не готово — `enabled:false`.
- **Ф11. Ретайр freellmapi** (D4) — только после того, как OmniRoute докажет покрытие 16 провайдеров вживую. До этого — fallback.
- **Ф12. Доки 2-му деву (дёшево):** таблица 4 деревьев (Castellyn / `llm-stack`=stack.json / `External\<svc>`=код+.env / `SettingsMCP\ClaudeProfiles`=Update-*.ps1) + резолв токенов `{{PROFILES}}`/`{{SCRIPTS_ROOT}}`; поправить заголовок «adding a provider = one entry» (называет оба шага).

## 7. Что может сломаться / роллбек

- **Единый фронт = единая точка отказа.** Митигация: Ф1-Ф3 (мониторинг+аларм) и Ф8 (авто-рестарт) идут ДО ретайра запасных путей; `glm-router :4000` и freellmapi держим живыми как fallback, пока OmniRoute не докажет себя.
- **Fidelity Anthropic-перевода OmniRoute для Claude Code** (tool-use/streaming/thinking) — риск №1, чисто эмпирический. Живой смоук до доверия; иначе Claude Code остаётся на glm-router/прямом Anthropic.
- **Двойной retry-шторм** (OmniRoute×движок): freellmapi капит 20 попыток, но без wall-clock дедлайна. Отключить provider-retry OmniRoute для нестабильных upstream'ов; таймаут > worst-case.
- **Миграция ключей 16 провайдеров** в OmniRoute — разовая; при потере — freellmapi ещё жив (fallback).
- **zcode re-clone** может затереть живые `data/`+`.env` — переносить, не перезаписывать.
- **Split id 'gateway'** — caller-aware правка в 5+ сайтах; ошибка = misrouting freeapi.db/регистрации. Точечно, с тестом.

## 8. Верификация (живой смоук, не только гейты)

Claude Code → `:20128` реальный запрос (с tool-use); OpenCode/Codex → `:20128`; запрос, уходящий в движок (Qwen/DeepSeek); запрос в key-based провайдер (Groq/Cerebras); проверить RTK/Caveman-сжатие; **убить аккаунт вручную → Castellyn показывает бейдж/тост «пересоздать»**; Castellyn показывает карточку OmniRoute + health + «Открыть дашборд». Плюс зелёные гейты: `npm run check` 0/0, `npm test`, `cargo test`, `build_all.ps1`.

## 9. Отложено / открытые вопросы

- Точный serve-подкоманд и health-роут OmniRoute — решается на Ф0.
- Ёмкость мульти-ключа OmniRoute для наших free-tier'ов — Ф0.
- Нужен ли `glm-router` после доказанного Anthropic-перевода OmniRoute — решить после Ф-смоука.
- Онбординг-визард первого запуска OmniRoute (пароль `CHANGEME`→смена, ключ, провайдеры) — отдельным шагом.
- ⚠ **Plan 2 (из финального ревью Ф1):** `prev_down` в `stack_health.rs` стартует пустым → первый тик (~30с после старта) эмитит `stack-service-down` для КАЖДОГО уже-down enabled-сервиса (ложный всплеск «только что упал»). Сейчас инертно (нет потребителя события). При подключении тоста/бейджа в Plan 2 — засеять `prev_down` из первого поллинга (эмитить только со 2-го тика) или явно подавить baseline первого тика.
- ⚠ **fork-updater = ДВЕ расходящиеся копии** (найдено 2026-07-07): `Castellyn\tools\fork-updater\ForkSync.psm1` (75KB, + runtime `*.last.json`/logs — эту гоняет Castellyn через манифест) vs `E:\Scripts\fork-updater\ForkSync.psm1` (54KB, + `tests/`/`repos.json`/`README` — цель portfolio-аудита). Файлы ОТЛИЧАЮТСЯ. **Ф9/D6 обязан сперва свести их в один канон** (вендор-синк по образцу `ScriptKit.ps1`), иначе 8 аудит-фиксов fork-updater лягут не в ту копию, что использует Castellyn.

## 10. Результаты Ф0 (pre-integration гейт, 2026-07-07) — вживую

OmniRoute установлен глобально (`npm i -g omniroute`, v3.8.45, bin `%APPDATA%\npm\omniroute[.cmd]`). Прогон на изолированном DATA_DIR.

**Подтверждено (меняет/уточняет спек):**
- **serve-команда = `omniroute serve --no-open --no-tray`** (foreground, supervisor-owned; `--no-open` = не открывать браузер каждый старт; `--daemon` НЕ использовать — отвяжет pid от нашего супервизора). Стартует ~8с, бинд :20128. Порт-флаг `--port`.
- **Мульти-ключ/ротация есть** → ретайр freellmapi обоснован: `keys add/list/rotate`, `providers rotate` (ротация upstream-ключа), `providers status` (**key health: age/expiry/cooldown**), `nodes add/list` (несколько эндпоинтов на провайдера).
- **Version-preservation упрощается:** нативные `omniroute backup create --encrypt --retention N` + `backup auto` (расписание) + `sync` (между инстансами). ⇒ **НЕ писать `Backup-OmniRoute.ps1`** — обернуть `omniroute backup` (правка §5/D4).
- **Мониторинг-фид готов:** `omniroute providers status --json` = per-key health по всем провайдерам ⇒ Ф2 читает это, а НЕ сырой SQLite (правка §5 п.2).
- **Добавление провайдера скриптуемо:** `providers`/`keys`/`nodes` CLI ⇒ D3 можно реализовать через CLI, не только дашборд.
- **Авто-детект клиентов:** `omniroute status` видит Claude Code / Codex / OpenCode установленными → `setup-*` их пропишет.
- **DATA_DIR обязателен и сквозной:** CLI привязан к DATA_DIR; при несовпадении CLI «не видит» запущенный сервер. Castellyn обязан передавать тот же DATA_DIR во ВСЕ вызовы `omniroute` (env в stack.json + при shell-out).

**⚠ Риски, вскрытые вживую (важно):**
- **WEDGE подтверждён эмпирически:** свежий zero-config сервер после серии проб **завис, оставаясь `LISTENING` на 0.0.0.0:20128, но отдавая `000` на ВСЕ роуты (вкл. root)**. Это ровно «wedged-but-bound» — **port-open health показал бы зелёный на мёртвом сервисе**. Вывод жёсткий: health в stack.json = реальный 2xx на маршрутизирующий роут (или вызов `omniroute health`). ВАЖНО: собственный `--max-restarts` OmniRoute это НЕ ловит (зависание ≠ краш).
- **`omniroute health` КОРРЕКТНО отличил wedge** (сказал «server not running», пока порт был LISTENING) → это лучший сигнал, чем port-open; кандидат на источник health для Castellyn. Но **точный неавторизованный HTTP 2xx-роут не закреплён** (API-роуты `000`-или под wedge; `/v1/models` требует Bearer; `/health` = SPA-404). Закрыть чистым ре-тестом на СКОНФИГУРИРОВАННОМ сервере (Ф7-смоук).
- **`omniroute stop` (с DATA_DIR) — надёжен** (остановил зависший PID). Для Castellyn: наш `kill_tree` по tracked-pid должен так же чисто снимать дерево — проверить на Ф7.
- **Оговорка честности:** wedge получен на голом сервере без провайдеров под burst-пробами — это НЕ доказывает, что нормально сконфигурированный OmniRoute нестабилен. Нужен ре-тест с провайдерами. Но для единого фронта это усиливает приоритет мониторинга/аларма/fallback ДО ретайра запасных путей.

**Итог гейта:** зелёный на «можно строить», с двумя открытыми пунктами к Ф7-смоуку (точный health-роут + стабильность под конфигом). Спек-дельты выше применить при реализации.

## 11. Модель провижнинга (лучшая практика 2025-26) — решение

**Вопрос владельца:** зашить все движки/апдейтеры прямо в Castellyn «без внешних зависимостей», чтобы юзеру не ставить всё по отдельности?

**Решение: НЕ реимплементировать и НЕ бандлить — Castellyn = провижнер и владелец жизненного цикла внешних движков.**
- [Tauri sidecar](https://v2.tauri.app/develop/sidecar/) требует self-contained бинарь БЕЗ рантайм-зависимостей → наши движки (Node+Chrome/Playwright, Python, OmniRoute=1174 npm-пакета) им не забандлить. Реимплементация в Rust = стать мейнтейнером upstream + потерять их апдейты (ровно то, ради чего fork-sync). Антипаттерн.
- **Паттерн (uv tool / Ollama / VS Code):** один установ Castellyn → детект отсутствующего (`where omniroute`, `omniroute doctor`, git-статус движков) → **install-on-demand с согласия** (`npm i -g omniroute`, git-клон движков, `npm install`, `npx playwright install chromium`) → persist (receipt в `%APPDATA%\castellyn`) → авто-апдейт. Движки остаются внешним кодом; юзер их руками НЕ трогает — «без внешних зависимостей» с его точки зрения.
- **Два рычага сократить поверхность установки:** (1) OmniRoute поглощает провайдеров → меньше движков (аудит Опции 4); (2) в Rust переносим только ТОНКУЮ оркестрацию (лаунчеры/апдейтеры — начали нативным супервизором, заменившим `start/stop-stack.ps1`), не тяжёлые движки.
- **Реализация:** отдельная поздняя фаза — first-run dependency-wizard (детект → install-with-consent → receipt), поверх уже существующего онбординга. НЕ блокирует Plans 1-3.

## 12. Что НЕ обсолетится (ответ на «llm-stack и fork-updater больше не нужны?»)

Castellyn — нативный ШЕЛЛ/оркестратор, не замена. Ретайрится только ПЕРЕСЕЧЕНИЕ:
- **llm-stack ОСТАЁТСЯ:** `stack.json` = канонический реестр сервисов, который читает нативный супервизор (и аудит его расширяет — `readyTimeoutSec`); `glm-router/` (fallback Anthropic); `extension/` (DeepSeek-креды). Ретайрятся только PS-лаунчеры `start/stop-stack.ps1` (нативный супервизор — дефолт; но их держат 3 внешних потребителя).
- **fork-updater ОСТАЁТСЯ и становится КАНОНОМ** (D6) — самостоятельный движок с тест-сьютом; Castellyn лишь вызывает `update-forks.ps1`. Ретайрится `sync-freellmapi.cmd`. (Сперва свести две расходящиеся копии — см. §9.)

## 13. Клиенты → OmniRoute (Ф6-ревизия, заземлено 2026-07-07)

Как каждый клиент указывается на единый фронт `:20128`. Исходная формулировка Ф6 («разблокировать
арм `direct/openai`, чтобы OpenCode/Codex били в :20128») оказалась неточной: арм `connect_my_provider`
`("direct","openai")` таргетит **Claude-профиль** (`targetProfile`), а не OpenCode/Codex. Заземлённая карта:

| Клиент | Как указать на :20128 | Код |
|--------|------------------------|-----|
| **Claude Code** | my-provider `protocol=anthropic`, `baseUrl=http://localhost:20128/v1`, `connectVia=direct` → бинд профиля существующим армом `("direct","anthropic")` (`ANTHROPIC_BASE_URL`). OmniRoute сам делает Anthropic-трансляцию. | **ноль** |
| **OpenCode** | добавить OmniRoute движком/my-provider → «Connect to OpenCode» (`run_opencode_provider`/`run_opencode_providers`, `lib.rs:6910`/`8673`). Уже умеет любой OpenAI base_url. | **ноль** |
| **Codex** | `run_codex_omniroute` (Plan 3b A1) — обобщённый `patch_codex_provider` пишет `[model_providers.omniroute]`+`[profiles.omniroute]`. | **новый (сделан)** |

⚠ **Codex-жёсткое ограничение:** Codex говорит ТОЛЬКО Responses wire API (WireApi без `chat` с 2026-02),
поэтому `:20128/v1` обязан отдавать `/v1/responses` shim — иначе регистрация пройдёт, но запросы молча
падают. Живой чек `/v1/responses` + установка `OMNIROUTE_API_KEY` (из `omniroute keys`) + UI-триггер
«Deploy Codex→OmniRoute» — это Part B (живой сеанс), не Part A. «Разблокировку арма `direct/openai`»
под этой топологией НЕ делаем (Claude покрыт anthropic-армом).
