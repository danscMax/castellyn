# Промпт для Castellyn: полное владение управлением Claude Code стеком

> Готовый бриф для агента (Claude Code), работающего в репозитории `E:\Scripts\Castellyn`.
> **Цель:** Castellyn становится единым «пультом», который полностью владеет управлением локального
> Claude Code стека — профили, marketplace, плагины, скиллы, команды, агенты, хуки, настройки, MCP,
> синхронизация, расписания, бэкапы — заменяя ручной запуск PowerShell-скриптов.
> Как использовать: открыть сессию Claude Code в `E:\Scripts\Castellyn` и вставить весь текст ниже.

---

## 0. Прочитать ПЕРВЫМ (источники правды — ничего не выдумывать)

- **`E:\Scripts\!Настройки и MCP\CLAUDE-STACK-ARCHITECTURE-AND-CASTELLYN.md`** — полная карта текущей
  реализации стека, карта конфликтов и правила владения. **Это главный источник.**
  (На MiniPC — `C:\Scripts\…`; при проблемах с кириллицей в Bash — ASCII-junction `…\SettingsMCP\…`.)
- В самом Castellyn: `CLAUDE.md`, `manifest\maintenance-manifest.json` (уже оборачивает наши
  `Update-*.ps1`), `src-tauri\assets\{plugin_sync.py, castellyn_status.py}`, `src-tauri\src\lib.rs`,
  `src\lib\components\{PluginsTab,McpTab,SessionsTab}.svelte`.
- Внешний стек: `…\ClaudeProfiles\Install-ClaudeProfiles.ps1`, `…\ClaudeProfiles\config\`
  (`managed-settings.json`, `settings.json`, `profiles.json`, `.mcp.json`, `.stignore`, `hooks\`,
  `skills/commands/agents`), `Schedule-Hub.ps1`, `Configure-Syncthing.ps1`, `Backup-ClaudeSetup.ps1`;
  `…\ClaudeMarketplace\Check-MarketplaceVersions.ps1`, `.claude-plugin\marketplace.json`,
  `plugins\{max,speckit}\`.

## 1. Архитектурное решение (рекомендация)

**«Обернуть и владеть», а не переписывать с нуля.** Существующие PowerShell-скрипты — проверенный
движок логики; Castellyn уже часть из них вызывает (см. манифест). Расширить это: Castellyn становится
ЕДИНСТВЕННЫМ фронтом/владельцем, который вызывает эти скрипты и добавляет недостающие поверхности
управления в GUI. **Не** портировать всю логику в Rust сразу — это создаст две расходящиеся реализации.
Постепенный порт в Rust — только для стабильных кусков, отдельной фазой, по одному, со скриптом-fallback.
> Если решишь иначе (полный нативный порт) — сделай это ЯВНЫМ решением и опиши миграционный план.
> По умолчанию — обёртка.

## 2. Что должно оказаться под управлением Castellyn (контур; детали каждого — в арх-документе)

1. **Профили** (`~/.claude-*`): создание/удаление/список из `config\profiles.json` (источник правды),
   symlink-раскладка shared-папок, CMD-врапперы, `$PROFILE`-лаунчеры, креденшелы.
2. **Marketplace + версии:** локальный `max-marketplace` (плагины `max`, `speckit`); **двойной bump
   версии** (`marketplace.json` + `plugin.json` синхронно) — обернуть `Check-MarketplaceVersions.ps1 -Bump`;
   рефреш маркетплейса.
3. **Плагины / скиллы / команды / агенты:** включение по профилям, деплой из `config\{skills,commands,agents}`,
   неймспейсы (`/max:<cmd>`, `max:<agent>`, скиллы — префикс `max-`).
4. **Хуки:** установка файлов из `config\hooks\` в `~/.claude\hooks\` + разводка событий в
   `managed-settings.json` (список и события хуков — в доке).
5. **managed-settings.json:** единый писатель — деплой в `%ProgramFiles%\ClaudeCode\` (нужен admin/UAC).
6. **MCP:** user-scope из единого `config\.mcp.json`; **НИКОГДА** не `managed-mcp.json`.
7. **Синхронизация (Syncthing):** 3 папки + versioning (обернуть `Configure-Syncthing.ps1`).
8. **Расписания и бэкапы:** scheduled tasks (обернуть `Schedule-Hub.ps1`), снапшоты (`Backup-ClaudeSetup.ps1`).
9. **Кросс-машина:** Main (`E:\Scripts`, user `User`) ↔ MiniPC (`C:\Scripts`, user `dansc`); мост `max-bridge`.

## 3. Правила владения / БЕЗ конфликтов (критично — см. док §11–13)

- **`~/.claude\hooks\plugin_sync.py` — один владелец = Castellyn.** Castellyn генерирует список профилей
  из `profiles.json` (его маркер `# castellyn:profiles` уже для этого) и владеет файлом. **Одновременно
  договориться, чтобы внешний инсталлятор перестал деплоить свой `config\hooks\plugin_sync.py`** — иначе
  `Install -Force` перезатрёт версию Castellyn. Логики РАЗНЫЕ (наш недавно сузили до `.claude` + `.claude-cc*`,
  чтобы не трогать `.claude-mem`; версия Castellyn использует явный список).
- **`managed-settings.json` — один писатель.** Сейчас его переразвёртывает ONLOGON-задача
  `ClaudeProfiles-ManagedSettings`. Если Castellyn берёт это на себя — либо он правит НАШ источник
  `config\managed-settings.json` и зовёт деплой, либо отключает эту задачу. **Не два прямых писателя** в
  `%ProgramFiles%`.
- **Scheduled tasks Castellyn — свой префикс `Castellyn-*`.** Не пересекать `ClaudeProfiles-*` /
  `ClaudeMaintenanceHub-*`, иначе взаимное удаление задач.
- **MCP — единый источник `config\.mcp.json`.** Никогда не создавать `managed-mcp.json` (переводит CC
  в exclusive-control, ломает `--chrome`/claude-in-chrome, CC #15494; наш инсталлятор его и так удаляет).
- **Хуки Castellyn — свой префикс** (кроме сознательно-общего `plugin_sync`). `castellyn_status.py` — ок.

## 4. Guardrails (обязательные)

- **Machine-agnostic:** ни одного хардкода `E:\` — выводить корень в рантайме (`$env:COMPUTERNAME`,
  `installLocation` из `known_marketplaces.json`, `%USERPROFILE%`). Работать и на Main, и на MiniPC.
- **JSON без BOM** (парсер CC давится BOM); из PowerShell 5.1 читать UTF-8 ЯВНО
  (`[IO.File]::ReadAllText` / `-Encoding UTF8`), писать через `UTF8Encoding($false)`.
- **Аддитивно и обратимо:** не ломать работающую установку; каждое действие — с dry-run/preview;
  не трогать `.credentials.json` и прочие секреты.
- **`plugin_sync` никогда не пишет в не-CC каталоги** (`.claude-mem` и подобные) — только профили из
  `profiles.json`.
- **Учитывать pending ASCII-переезд** `!Настройки и MCP` → `Settings-MCP`: `manifest\maintenance-manifest.json`
  держит path-literals со старым именем — перейти на динамический вывод корня или обновить после переименования.
- **Elevation:** запись в `%ProgramFiles%` требует admin — поднимать UAC один раз (как инсталлятор),
  не чаще.

## 5. Фазы (каждая — отгружаемая и обратимая)

- **Ф0. Recon + решения владения:** прочитать док + скрипты; зафиксировать владельца каждого общего
  ресурса (таблица §11 дока). Снять «before»-снимок: `managed-settings.json`, набор хуков, профили, задачи.
- **Ф1. Хуки:** Castellyn владеет `plugin_sync` (список из `profiles.json`) + своими хуками; внешний
  инсталлятор перестаёт деплоить `plugin_sync`. Проверка: нет двойной разводки, `.claude-mem` не трогается.
- **Ф2. managed-settings:** единый писатель (Castellyn правит источник + деплоит, либо отключает
  ONLOGON-задачу).
- **Ф3. Marketplace / плагины / скиллы / команды:** двойной bump версии, включение/деплой, неймспейсы.
- **Ф4. MCP + Syncthing + расписания + бэкапы:** обернуть скрипты; задачи с префиксом `Castellyn-*`.
- **Ф5. Профили + инсталлятор:** полный жизненный цикл профиля из GUI; ручной запуск PS остаётся fallback.

## 6. Критерии приёмки

- Всё управление доступно из Castellyn; ручной запуск PS больше НЕ обязателен (но работает как fallback).
- Ноль двойных срабатываний хуков; ноль двойных писателей managed-settings; ноль коллизий имён задач.
- `plugin_sync` не пишет в `.claude-mem`; профили — из `profiles.json`.
- Работает на обеих машинах без правок (machine-agnostic; проверить и на MiniPC).
- Существующая установка не сломана: сравнить «before/after» — `managed-settings.json`, набор хуков,
  список профилей, scheduled tasks.

## 7. Что НЕ делать

- НЕ создавать `managed-mcp.json`.
- НЕ переименовывать/переносить `!Настройки и MCP` (это отдельная офлайн-операция; скрипт
  `Migrate-ToAscii.ps1` уже готов и запускается вручную).
- НЕ дублировать `plugin_sync` (один владелец).
- НЕ хардкодить пути конкретной машины.
- НЕ трогать `.credentials.json` / секреты.

---

### Первым делом от агента ожидается
Прочитать арх-документ (§0), затем вернуть: (а) карту «ресурс → текущий владелец → предлагаемый владелец»,
(б) план по фазам с оценкой, (в) явное решение «обёртка vs нативный порт», (г) список того, что нужно
согласовать с внешним стеком (в первую очередь — отключить деплой `plugin_sync` из инсталлятора).
До реализации — показать план на подтверждение.
