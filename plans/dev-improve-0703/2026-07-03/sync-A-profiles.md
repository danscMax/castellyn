# Sync-карта, кластер A: профили + шеринг скиллов

Read-only исследование Castellyn (`E:\Scripts\Castellyn`), 2026-07-03. Все факты
подтверждены `file:line`. Помечено «НЕ ПРОВЕРЕНО» то, что нельзя подтвердить из кода
(живое состояние диска, поведение внешних инструментов).

> Важно: фактическая механика junction/symlink живёт в PowerShell-скриптах ВНЕ репозитория,
> под `E:\Scripts\!Настройки и MCP\ClaudeProfiles\` (Rust только их запускает через
> `spawn_streamed`). Шеринг скиллов между харнессами (`~/.agents/skills`) — наоборот, нативный
> Rust (`share_skills`), без скрипта.

---

## 0. Что такое «профиль»

- Профиль = каталог `~/.claude-<name>`, используемый как `CLAUDE_CONFIG_DIR`. Мастер-конфиг —
  `~/.claude`. Доказательства: `lib.rs:3464` (`{home}\\.claude-{name}\\settings.json`),
  `Install-ClaudeProfiles.ps1:289` (`$profileDir = "$HOME\.claude-$($profile.Name)"`),
  cmd-обёртки задают `CLAUDE_CONFIG_DIR=%USERPROFILE%\.claude-<name>` (`Install:696`).
- Имена по умолчанию: `PROFILE_NAMES = ["ccmy","cc1","cc2","cc3","cc4","cc5"]` (`lib.rs:5406`),
  переопределяются `config\profiles.json → profiles[].name`.
- **`profile_names()`** (`lib.rs:3437-3452`): читает
  `!Настройки и MCP\ClaudeProfiles\config\profiles.json` (`PROFILES_CONFIG_REL`, `lib.rs:1619`),
  берёт `profiles[].name`; при отсутствии/ошибке — 6 дефолтов. Это канонический список,
  на который завязаны все fan-out'ы (провайдеры `lib.rs:3463`, MCP `lib.rs:5467`, плагины
  `lib.rs:7599`, репейр `lib.rs:1412/1440`).
- **`plugin_sync_profiles(home)`** (`lib.rs:7880-7893`): `.claude` + `.claude-<name>` для каждого
  профиля, отфильтровано по наличию РЕАЛЬНОГО (не symlink) `settings.json`. Скан домашнего каталога
  сознательно НЕ используется — иначе `~/.claude-mem` и т.п. приняли бы за профиль (`lib.rs:7876-7879`).
- **ProfilesTab.svelte**: создание — кнопка «Reinstall» (`onAction('reinstall')` →
  `run_profiles "reinstall"` → `INSTALL_SCRIPT_REL -Force`, `lib.rs:1403`); add/rename/recolor/
  set-links — `run_profile_mgmt` → `Manage-Profiles.ps1` (`lib.rs:1643-1702`); репейр линков —
  `run_profiles "repair"` (не-админ путь) либо `repair_profile_elevated` → `Repair-ProfileLinks.ps1`
  (`lib.rs:1705-1764`); `repair_all_profiles` — «Repair All» с Home (`lib.rs:1434-1480`).
- UI-список общих папок: `ALL_FOLDERS = ['agents','commands','hooks','plugins','skills','projects','history.jsonl']`
  (`ProfilesTab.svelte:88`); отображаемый тип линка на профиль — `p.sharedLinks` (Junction /
  SymbolicLink / HardLink / none / null) (`ProfilesTab.svelte:236-253, 473-474`).

---

## Механизм 1 — Общие ПАПКИ профиля (agents, commands, hooks, plugins, skills) → SymbolicLink

- **ЧТО**: содержимое каталогов `agents / commands / hooks / plugins / skills`.
- **ОТКУДА → КУДА**: `~/.claude/<folder>` (мастер) → `~/.claude-<name>/<folder>`.
- **КАК**: директорный **SymbolicLink** (`New-Item -ItemType SymbolicLink`), нужен админ/Developer Mode.
  `Install-ClaudeProfiles.ps1:392-472` (Step 4), `Repair-ProfileLinks.ps1:174-181`;
  тип задаётся `Get-SharedItemLinkKind` → `'SymbolicLink'` для всего кроме projects/history
  (`ProfileLib.ps1:104-112`). Набор — `Get-ClaudeSharedFolders` из `config\profiles.json →
  sharedFoldersDefault`, fallback — тот же список из 7 (`ProfileLib.ps1:68-79`).
- **АВТО/РУЧНОЕ**: **user-triggered** (Reinstall / Repair). Не-админ прогон Install само-элевейтится
  одним UAC (`Install:423-448`), либо копирует (`-CopySharedFolders`), либо пропускает (`-SkipSymlinks`).
- **GAP**:
  - Требуется админ. При отказе UAC профили молча НЕ шарят эти папки (`Install:445-447`).
  - Реальная (не-link) папка с данными репейром НИКОГДА не перезаписывается — репортится и
    оставляется под полную переустановку, которая мержит (`Repair:150-158`, `Install:491-530`).
  - `Repair` не-админ пропускает symlink-папки с предупреждением (`Repair:175-179`) — отсюда
    двухступенчатый UX «Finish (admin)» в UI (`ProfilesTab.svelte:78-85, 418-420`).

## Механизм 2 — projects/ (сессии/транскрипты) → Junction

- **ЧТО**: транскрипты сессий (`projects/*`) → это и даёт кросс-профильный `claude --resume`.
- **ОТКУДА → КУДА**: `~/.claude/projects` (общий) → `~/.claude-<name>/projects` (junction).
- **КАК**: **Junction** (`New-Item -ItemType Junction`), **без админа**. `Install:474-560` (Step 4.5),
  `Repair:172-173` (ветка `$isJunction`), `Get-SharedItemLinkKind` → `'Junction'` (`ProfileLib.ps1:109`).
  При первичной установке существующие per-profile `projects` сначала МЕРЖАТСЯ в общий
  (newest-wins по файлам), потом каталог заменяется junction'ом (`Install:491-560`).
- **АВТО/РУЧНОЕ**: user-triggered (Reinstall/Repair). Не нужен админ → самый надёжный из линков.
- **GAP**: слияние происходит только в момент установки; далее это один общий каталог. Существенных
  проблем нет. (Совпадает с памятью проекта: «projects = junction → --resume кросс-профильно».)

## Механизм 3 — history.jsonl → файловый линк (symlink→hardlink)

- **ЧТО**: история промптов. **ОТКУДА → КУДА**: `~/.claude/history.jsonl` (мерж+дедуп всех профилей) →
  `~/.claude-<name>/history.jsonl`.
- **КАК**: `New-FileLink` = `cmd /c mklink` (symlink), fallback → HardLink (`Install:375-390, 562-630`;
  `Repair:71-80, 169-171`). `Get-SharedItemLinkKind` → `'File'` (`ProfileLib.ps1:110`).
- **АВТО/РУЧНОЕ**: user-triggered.

## Механизм 4 — keybindings.json → общий линк

- **ЧТО**: горячие клавиши. **ОТКУДА → КУДА**: `~/.claude/keybindings.json` (golden copy, в
  Syncthing-allowlist) → каждый профиль. **КАК**: symlink → hardlink → copy fallback (`Install:314-348`,
  Step 2.5). **АВТО**: user-triggered (Install).

## Механизм 5 — Общие CONFIG-ФАЙЛЫ → symlink/hardlink

- **ЧТО**: `settings.local.json, CLAUDE.md, statusline.py, infra-probe.ps1, cleanup_nul.ps1,
  subagent-monitor.ps1, RTK.md` (`Get-ClaudeSharedFiles -Kind shared`, `ProfileLib.ps1:87`).
- **ОТКУДА → КУДА**: `config/<file>` → `~/.claude/<file>` (первичная копия) → линкуется в каждый профиль.
  `Install:716-845` (Step 6). `mainOnly` файлы (`cclsp.json`, `.stignore`) кладутся только в `~/.claude`
  (`ProfileLib.ps1:88`, `Install:739-755`).
- **КАК**: symlink с fallback на hardlink (`New-FileLink`).
- **АВТО/РУЧНОЕ**: user-triggered. Отдельный путь — `Relink-SharedConfig.ps1` (`RELINK_SCRIPT_REL`,
  само-элевейтится, `lib.rs:1489-1512`); дрейф этих файлов сторожит `Check-Integrity.ps1` →
  `links.last.json` (`read_config_drift`, `lib.rs:1484-1487`; diff — `read_drift_diff`, `lib.rs:1590-1617`).
- **GAP**: `settings.json` в этот список НЕ входит намеренно (см. Механизм 6).

## Механизм 6 — settings.json → per-profile STUB (НЕ шарится)

- **ЧТО**: мягкие дефолты (model/theme/effortLevel…). Деплоится НЕЗАВИСИМОЙ КОПИЕЙ в мастер + каждый
  профиль, НЕ линком — потому что Claude Code перезаписывает `settings.json` (issues #2688/#9234),
  что рвёт линки (`Install:847-874`, Step 6, коммент `Install:726-733`).
- **Enforced** общий конфиг (plugins/permissions/hooks/statusLine/marketplaces) вместо этого лежит в
  `managed-settings.json` в `%ProgramFiles%\ClaudeCode\` (наивысший приоритет, CC не перезаписывает),
  `Install:876-933` (Step 6.7), нужен админ (само-элевейт).
- **GAP**: `settings.json` сознательно per-profile и может расходиться; единообразие обеспечивает только
  `managed-settings.json`. Это by design, не баг.

## Механизм 7 — .credentials.json → per-profile (НЕ шарится)

- **ЧТО**: токены логина. Копируется из бэкапа на install и НИКОГДА не перезаписывается — даже с `-Force`
  (`Install:355-372`, Step 3). Каждый профиль логинится независимо; `credentialsPresent` репортится на
  профиль (`ProfilesTab.svelte:333, 489-492`). **АВТО**: нет — только на install из бэкапа.

## Механизм 8 — Enable-state плагинов → reconcile-хук (plugin_sync.py)

- **ЧТО**: ключи `enabledPlugins` + `extraKnownMarketplaces` в `settings.json` профилей. Сам КОНТЕНТ
  каталога `plugins/` шарится symlink'ом (Механизм 1), но состояние вкл/выкл — per-profile в каждом
  `settings.json`, поэтому его синхронизируют отдельно.
- **ОТКУДА → КУДА**: плагин, включённый (`True`) в ЛЮБОМ профиле, добавляется в каждый профиль, где ключ
  ОТСУТСТВУЕТ целиком; явный `False` — намеренный per-profile opt-out, НЕ трогается. Marketplaces —
  так же через `setdefault`. Атомарно, only-if-changed, fail-open (`plugin_sync.py:1-16, 33-60`).
- **КАК**: SessionStart-хук, встроенный ассет (`PLUGIN_SYNC_SCRIPT`, `lib.rs:7812`); команда хука
  `py -X utf8 ~/.claude/hooks/plugin_sync.py` (`PLUGIN_SYNC_HOOK_CMD`, `lib.rs:7815`). Список `PROFILES`
  внутри скрипта генерит Castellyn из `plugin_sync_profiles` (`render_plugin_sync_script`, `lib.rs:7836-7854`).
- **АВТО/РУЧНОЕ**: **автоматически на SessionStart**, ЕСЛИ хук вписан — тумблер на вкладке Plugins
  вписывает команду в `settings.json` каждого профиля (идемпотентно, пропускает symlink'нутые settings,
  `lib.rs:7804-7811`). Плюс «Sync now» — разовый ручной прогон.
- Дополнительно: `manage_plugin_native` (`lib.rs:7580-7611`) — enable/disable гоняет по каждому профилю
  через `CLAUDE_CONFIG_DIR`; `update` — один раз (кэш `plugins/` общий).
- **Расхождение в доках (мелкое)**: коммент `lib.rs:7581` называет общий `plugins/` «shared … via
  junction», но `Get-SharedItemLinkKind` и `Install` Step 4 создают для `plugins` **SymbolicLink**, а не
  junction (junction — только `projects`). Живой тип линка на диске НЕ ПРОВЕРЕН; по коду это SymbolicLink.

## Механизм 9 — MCP-серверы → деплой per-profile (user scope)

- **ЧТО**: 5 общих MCP из `config/.mcp.json`. **ОТКУДА → КУДА**: `.mcp.json` → `claude mcp add-json
  --scope user` в `.claude.json` каждого профиля (`Install:936-980`, Step 6.8). Пропускаются
  `context7`/`serena` — их уже даёт enabled-плагин, дубль конфликтует (`Install:961-967`).
- **КАК**: remove-then-add через `claude mcp`, не нужен админ. Нативный read: `read_mcp` +
  `profile_mcp_servers` per-profile (`lib.rs:5467-5471`). **АВТО**: user-triggered (Install).
- Примечание: `managed-mcp.json` НЕ используется — он включает «exclusive MCP control» и ломает `--chrome`
  (`Install:882-889`).

---

## Шеринг СКИЛЛОВ — две разные оси

### Ось A (per-profile, только Claude): папка skills → SymbolicLink

- Это часть Механизма 1: `~/.claude/skills` symlink'ается в каждый `~/.claude-<name>/skills`, поэтому
  все профили Claude видят один набор скиллов. Именно поэтому `read_environments` для Claude ставит
  `shareable_gap = 0` с комментом «sharing targets ~/.agents/skills, which Claude does not read»
  (`lib.rs:6252`). Установка скилла — копия из `config/skills/` в `~/.claude/skills` (`Install:184-204`).

### Ось B (кросс-харнесс): share_skills → junction'ы в ~/.agents/skills

- **ЧТО**: каждый РЕАЛЬНЫЙ (не-symlink) скилл из `~/.claude/skills` + каждый скилл, встроенный в
  установленный плагин (`shareable_skill_sources`, `lib.rs:6009-6033`).
- **ОТКУДА → КУДА**: `~/.claude/skills/<skill>` и content-dir плагинов → `~/.agents/skills/<skill>` —
  единственный каталог, который OpenCode И Codex сканируют на user-уровне (`lib.rs:5963-5969, 6360-6363`).
- **КАК**: `mklink /J` (junction) на каждый скилл, **без админа** (`share_skills`, `lib.rs:6364-6422`).
  Идемпотентный «ensure»: создать отсутствующие, пересоздать висячие (устаревшие цели в plugin-кэше
  после апдейта), пропустить живые (`lib.rs:6389-6408`). Имя скилла charset-guard `[A-Za-z0-9._-]`
  против инъекции в argv `mklink` (`lib.rs:6378-6388`).
- **АВТО/РУЧНОЕ**: **user-triggered** — кнопка «Share skills» (`shareSkills`, `+page.svelte:836`;
  ipc `share_skills`, `lib.rs:746/750` в ipc.ts). НЕ автоматически: ничто не перезапускает шеринг при
  установке нового скилла/плагина.
- **Модель видимости** (`lib.rs:5966-5969`, `skill_sets` `lib.rs:6036-6088`):
  - Claude: `~/.claude/skills` + скиллы плагинов.
  - OpenCode: `~/.claude/skills` + `~/.agents/skills` + `~/.config/opencode/skills`.
  - Codex: `~/.agents/skills` + `~/.codex/skills` (Claude-каталог НЕ читает).
  Матрица/гэп: `read_skill_matrix` (`lib.rs:6320-6348`), `read_environments`/`shareable_gap =
  source_names − <harness>_visible` (`lib.rs:6185-6187`), мемо `skill_sets_cached` (TTL 2 c, `lib.rs:6094-6107`).
- **GAP**:
  - Codex НЕ видит `~/.claude/skills` вообще — скиллы Claude/плагинов доходят до Codex только ПОСЛЕ
    ручного `share_skills` (junction в `~/.agents/skills`).
  - `share_skills` линкует только РЕАЛЬНЫЕ скиллы (symlink'нутые «own» пропускаются — они уже лежат в
    коллекции и в `~/.agents`, `lib.rs:6019, 6038-6040`).
  - Скиллы, живущие ТОЛЬКО в OpenCode/Codex, никогда не пушатся обратно в Claude — постоянный
    неустранимый остаток; `shareableGap` его намеренно исключает, чтобы не читался как незакрытый гэп
    (`CONTEXT.md:69-73`).
  - Ручной one-shot: после установки нового скилла/плагина junction'ы надо пересоздавать вручную;
    апдейт плагина может оставить junction висячим до повторного «Share skills».
- Смежное: `delete_skill` (`lib.rs:7779-7802`) у symlink'нутого «own» скилла удаляет ТОЛЬКО линк, не
  трогая исходную коллекцию; plugin-скиллы удалять отказывается.

---

## Сводка «что шарится / чем / авто ли»

| Артефакт | Механизм линка | Направление | Авто? | Нужен админ |
|---|---|---|---|---|
| agents, commands, hooks, plugins, skills (папки) | SymbolicLink | `~/.claude` → профиль | Нет (Reinstall/Repair) | Да (или UAC-элевейт) |
| projects (сессии) | Junction | `~/.claude` → профиль | Нет (Reinstall/Repair) | Нет |
| history.jsonl | symlink→hardlink | `~/.claude` → профиль | Нет | Нет |
| keybindings.json | symlink→hardlink→copy | `~/.claude` → профиль | Нет | Нет |
| CLAUDE.md/statusline.py/RTK.md/… | symlink→hardlink | config → `~/.claude` → профиль | Нет (Reinstall/Relink) | Relink элевейтится |
| settings.json | КОПИЯ (не линк) | config → каждый профиль | Нет | Нет |
| managed-settings.json | КОПИЯ в ProgramFiles | config → машина | Нет | Да |
| .credentials.json | КОПИЯ из бэкапа | бэкап → профиль (не перезапис.) | Нет | Нет |
| enabledPlugins/marketplaces | reconcile-хук | любой профиль → недостающие | **Да** (SessionStart, если хук вписан) | Нет |
| MCP-серверы | `claude mcp add-json --scope user` | .mcp.json → каждый профиль | Нет (Install) | Нет |
| Скиллы для OpenCode/Codex | Junction (`mklink /J`) | `~/.claude/skills`+плагины → `~/.agents/skills` | **Нет** (кнопка «Share skills») | Нет |

Единственный по-настоящему АВТОМАТИЧЕСКИЙ шеринг — reconcile enable-state плагинов на SessionStart
(при вписанном хуке). Всё остальное — user-triggered (Reinstall / Repair / Share skills / Install).
