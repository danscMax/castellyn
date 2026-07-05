# План: Castellyn как control-plane Claude Code стека (реестр-центричный)

> Ответ на `plans\STACK-OWNERSHIP-PROMPT.md` + форварднутый эко-аудит скиллов/плагинов.
> Разворот после уточнения владельца: это **одна цепь** — Castellyn становится единой панелью,
> которая ВЛАДЕЕТ всем стеком (профили, навыки, плагины, команды, хуки, MCP, sync, задачи, бэкапы);
> аудит — это **инвентарь накопленного долга**, заложенный прямо в архитектуру, а не отдельная уборка.
> Составлен после Ф0-рекона по двум деревьям (Castellyn `src-tauri\src\lib.rs` + внешний PS-стек
> `SettingsMCP\ClaudeProfiles\`), file:line подтверждены. **До кода — на подтверждение.**

---

## 0. Рамка: от «карты писателей» к control-plane

Первый черновик плана останавливался на «кто в какой файл пишет». Этого мало для *владения*.
Платформенный принцип: control-plane — это **реестр ресурсов + reconcile-петля (desired→actual)
+ детект дрейфа + политика жизненного цикла**, а не набор кнопок-обёрток над скриптами.

Три следствия:
- **Аудит = первое заполнение реестра.** Нельзя владеть тем, что не каталогизировано. Ф0.5.
- **Дубли/мусор — не разовая чистка, а класс в реестре** (`blessed / deprecated / quarantined / cruft`)
  + GC как постоянная фича Castellyn, а не кнопка Delete в руках владельца.
- **Фазы режем по жизненному циклу ресурса** (профиль, плагин, навык…), а не по механизму
  (managed-settings.json, hooks…), потому что владелец думает ресурсами.

---

## 1. Модель данных: Реестр ресурсов

Единая таблица управляемых объектов — источник правды Castellyn о самом стеке.

**Типы ресурсов:** `profile`, `plugin`, `marketplace`, `skill`, `command`, `agent`, `hook`,
`mcp_server`, `scheduled_task`, `sync_folder`, `managed_setting_block`, `backup`.

**Поля на запись:**
| Поле | Смысл |
|---|---|
| `id`, `type`, `name` | идентификация |
| `owner` | кто авторитетно пишет: `castellyn` / `external_installer` / `shared` |
| `sourceOfTruth` | путь источника (напр. `config\profiles.json`, `config\.mcp.json`) |
| `deployVia` | механизм: нативно / `Deploy-*.ps1` / `Install-ClaudeProfiles.ps1` / plugin_sync |
| `platform` | `all` / `win` / `mac` (для отсева iOS/darwin-мёртвого) |
| `health` | `ok` / `broken` / `dead-on-windows` / `unknown` |
| `class` | `blessed` / `deprecated(→преемник)` / `quarantined` / `cruft` |
| `provenance` | `universal` / `project:<name>` (тег против утечек типа SellCoach) |
| `driftState` | `in-sync` / `drift` / `not-deployed` (заполняет reconcile) |

Реализация: сначала JSON-снимок (`config\stack-registry.json` или нативно в Rust), позже — живой
скан. Реестр — то, что рендерит будущий «Стек»-таб и на чём стоит GC/health.

---

## 2. Reconcile-примитив (ядро владения)

Один DRY-примитив: **desired → deploy → verify → drift**. У Castellyn он **уже есть для MCP**
(`config\.mcp.json` канон → `Deploy-Mcp.ps1` → `claude mcp add-json`, `lib.rs:5646,5774,5901`) —
обобщить его на остальные ресурсы. Каждое действие: dry-run/preview → запись источника → деплой →
верификация фактического состояния → показ дрейфа. Никогда не «зелёный exit = готово» без проверки
реального эффекта (managed-settings в `%ProgramFiles%`, набор хуков, задачи).

---

## 3. Карта владельцев (ресурс → кто пишет сегодня → предлагаемый) — семя реестра

**Главная поправка к арх-документу §11: почти все «конфликты» уже сняты** (проверено рекон-кодом):

| Ресурс | Пишет сегодня | Владелец (цель) | Статус |
|---|---|---|---|
| `~/.claude\hooks\plugin_sync.py` | **ОБА** (installer `Install…ps1:225-243` + Castellyn `lib.rs:8732,8759`) | **Castellyn** | 🔴 живая коллизия — Ф1 |
| SessionStart-разводка `plugin_sync` | per-profile (Castellyn `:8647-8657`) **и** managed (installer) | одна точка (Castellyn) | 🔴 двойной запуск — Ф1 |
| `castellyn_status.py` | Castellyn | Castellyn | ✅ |
| прочие хуки (`rtk_guard`,`session_health`,`cleanup_nul`,`backup-trigger`,`subagent-monitor`) | installer | external | не трогать |
| `%ProgramFiles%\…\managed-settings.json` | installer (894-915)+ONLOGON `ClaudeProfiles-ManagedSettings` | источник→Castellyn, деплой→`Deploy-ManagedSettings.ps1` | Castellyn НЕ пишет сегодня (greenfield) — Ф1 |
| `config\.mcp.json` (+deploy) | **Castellyn** (канон) | Castellyn | ✅ готово |
| `managed-mcp.json` | никто (installer удаляет) | никто | ✅ |
| scheduled tasks | `Schedule-Hub.ps1` (external), Castellyn оборачивает | external-скрипт, Castellyn=фронт | ✅ нет коллизии |
| Syncthing-папки | `Configure-Syncthing.ps1` (external); Castellyn read-only+rescan | обернуть в Castellyn (Ф4) | gap |
| marketplace bump (`marketplace.json`+`plugin.json`) | `Check-MarketplaceVersions.ps1 -Bump` (dual-write `:12,47-58`) | обернуть | gap Ф3 |
| профили lifecycle (create/symlink/wrapper/launcher/creds) | `Install-ClaudeProfiles.ps1` | обернуть/GUI | gap Ф5 |
| skills/commands/agents (per-profile enable+deploy) | plugin_sync (fill enabledPlugins) + installer | Castellyn (реестр) | частично |
| backups | `Backup-ClaudeSetup.ps1` | обернуть | gap Ф4 |

Баг §12 арх-документа (`plugin_sync`→`.claude-mem`) **закрыт с обеих сторон**: внешний v3 glob
`.claude`+`.claude-cc*` структурно исключает `.claude-mem` (`plugin_sync.py:14-15,28-31`); Castellyn —
явный список из `profiles.json` с тем же предупреждением (`lib.rs:8347-8362`).

---

## 4. Решение: **обёртка, не нативный порт**

Castellyn уже обёртка над тяжёлой PS-логикой (манифест→`Update-*.ps1`, `run_schedule`→`Schedule-Hub.ps1`,
MCP→`Deploy-Mcp.ps1`); нативно делает только дешёвое (читатели, авторство хуков, in-place `.mcp.json`).
Порт логики профилей/managed-settings/marketplace в Rust = вторая расходящаяся реализация — отклоняем.
Нативно — только реестр, reconcile-примитив, health/GC-детект (это новое, не дубль PS).

---

## 5. Фазы (по жизненному циклу ресурса; каждая отгружаемая и обратимая)

> **Коррекция 2026-07-04 (сверка с видением владельца):** два трека, сходящихся в одном UI —
> **Control-plane** (эти фазы: реестр+reconcile+владение, «мозг») и **Cockpit** (Сессии/лаунчер/
> уведомления, herdr-трек, «руки»). Реестр питает оба. Добавлена **Ф2.5 «Матрица per-profile»** —
> главный UX-запрос владельца (провайдеры/прокси/shared-папки/плагины/движки НА ПРОФИЛЬ, выбирает
> пользователь в UI, не хардкод) = desired-state-редактор поверх reconcile-примитива Ф1.
> Порядок хвоста: Ф5 (профили GUI) поднята ВЫШЕ Ф4 (syncthing/backups — работает и так).
> Тип ресурса `engine` (claude/opencode/codex/…) — добавление нового движка = строка конфига, не код.

| Ф | Содержание | Объём |
|---|---|---|
| **Ф0** ✅ | Рекон + карта владельцев (эта секция) | сделано |
| **Ф0.5** ✅ | **Инвентарь = верификация аудита.** Выполнено 2026-07-04: 10 находок сверены по сырым файлам (~половина ложняк/переоценка), реестр записан → `plans\stack-registry.json` (19 строк). Детали: план-файл `shiny-baking-heron.md` + память `stack-ownership-f05`. | сделано |
| **Ф1** ✅ | **Reconcile-ядро + single-owner.** Выполнено 2026-07-04 (eba04af): plugin_sync single-owner закрыт целиком (installer исключает файл, managed-разводка снята, файл = Castellyn v4), read_stack_drift + карточка на Главной, run_managed_deploy (один UAC, верификация пересравнением). | сделано |
| **Ф2** ✅ | **Консолидация дублей + Health/GC.** Чистка исполнена 2026-07-04 (blessed-карантины, ENABLE_STOP_REVIEW=0, claude-reflect/dream/iOS снесены, ~0.5 ГБ мусора). GC-фича: карточка «Мусор стека» на Главной (adbe281) — живой скан stale-версий/temp_git/.bak (удаление в корзину с preview+confirm), darwin/linux — report-only. | сделано |
| **Ф2.5** ✅ | **Матрица per-profile.** Выполнено 2026-07-04 (b9118ae): таб Профили → матрица провайдер/прокси/shared-папки, batch Apply с предпросмотром, верификация перечиткой. Плагины/MCP-колонки = V2. | сделано |
| **Ф3** ✅ | **Marketplace lifecycle.** Выполнено 2026-07-05 (7f1d38e): bump patch/minor/major на «своих» строках таба плагинов (обёртка Check-MarketplaceVersions -Bump + авто plugin update), 4-й drift-пункт «Версии маркетплейса» на Главной (нативная сверка). MCP-dedup исполнен разово (context7/playwright/chrome-devtools; канон очищен от chrome-devtools). Provenance-перенос SellCoach — отложен решением владельца. | сделано |
| **Ф5** ✅ | **Профили lifecycle из GUI.** Рекон 2026-07-05: CRUD уже существовал целиком (add/remove/rename/recolor/redescribe/set-links → Manage-Profiles.ps1; repair/сироты/запуск). Добавлен hardening: (1) remove в Manage-Profiles.ps1 снимает junction/symlink ПЕРЕД удалением (риск сноса общих ~/.claude-папок через рекурсию PS5.1) + каталог в корзину вместо hard-delete; (2) live-session guard на remove/rename в run_profile_mgmt (016e727). | сделано |
| **Ф4** ✅ | Бэкапы обёрнуты (run_backup: backup/restore-preview/restore/delete), синк-папки нативно (run_sync/sync_set), расписания/MCP были готовы. Остаток — обёртка bootstrap `Configure-Syncthing.ps1` — отложен до онбординг-визарда новой машины (низкая ценность: syncthing автономен). | сделано |

Guardrails на все фазы: dry-run/preview перед записью; JSON без BOM (читать UTF-8 явно из PS 5.1,
писать `UTF8Encoding($false)`); machine-agnostic (корень из рантайма, ноль хардкода `E:\`);
не трогать `.credentials.json`/секреты; каждое удаление — обратимо/подтверждаемо.

---

## 6. Согласовать с внешним стеком (пункт «г», приоритет)

1. **ГЛАВНОЕ — installer перестаёт деплоить `plugin_sync.py`** (`Install-ClaudeProfiles.ps1:225-243`:
   исключить файл из `Copy -Force`-цикла, ИЛИ удалить из `config\hooks\`). Иначе `Install -Force`
   затирает версию Castellyn.
2. **Одна точка SessionStart-разводки `plugin_sync`** — убрать из managed ИЛИ из per-profile, не обе.
3. **managed-settings deploy** — Castellyn правит только источник + зовёт `Deploy-ManagedSettings.ps1`;
   ONLOGON-задача остаётся единственным машинным ре-деплоером. Ни одного второго писателя в `%ProgramFiles%`.
4. **ASCII-переезд** `!Настройки и MCP`→`Settings-MCP`: манифест + `*_REL`-константы в `lib.rs` держат
   кириллический path-literal — вывести корень динамически или обновить после переезда.

---

## 7. Открытые решения (нужен твой выбор, в Ф0.5/Ф2)

- **Дубли — кого назначить blessed:** ревью (`code-review` vs sc/comprehensive vs сломанные max:check/review),
  аудит (`max:audit` vs `max:tech-debt` vs goal-audit), память (claude-mem vs claude-reflect vs dream),
  LSP (serena vs cclsp), context7/playwright дубли-регистрации. По каждому — рекомендация в Ф0.5.
- **Латентность-хуки:** отключать ли `claude-mem PostToolUse:*` и `security-guidance Stop-LLM` (это МОИ
  текущие хуки сессии — трогаем осознанно).
- **Мёртвое на Windows:** iOS-скиллы, claude-reflect (хардкод `python3` без `commandWindows`) — снести
  или починить `commandWindows`.

---

## Что дальше

**ПЛАН ЗАВЕРШЁН 2026-07-05** (все фазы ✅, main = 016e727). Остаточная очередь вне этого плана:
- UI-трек **V9/V3** лаунчера (Cockpit; владелец хочет, дизайн не утверждён — нужен гейт).
- Матрица V2: колонки плагины/MCP.
- Онбординг-визард новой машины (включая обёртку `Configure-Syncthing.ps1` и ASCII-переезд §6.4).
- Provenance-перенос SellCoach-скиллов — при появлении второго потребителя маркетплейса.
