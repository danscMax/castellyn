//! Backend-side localization for user-facing strings the Rust layer produces:
//! command errors (shown as toasts), the run-log stream (console panel), and the
//! tray menu (a native surface the JS i18n tables can't reach).
//!
//! The frontend owns the locale (localStorage `cmh-language`); it mirrors the choice
//! into HubConfig.language via the `set_language` command, and lib.rs keeps the current
//! `Lang` in a process global that `tr`/`trv` read.
//!
//! Parity is STRUCTURAL: every entry is `[ru, en, zh]`, so a key cannot exist for one
//! language and not another (the array type forbids it). New strings: add one row.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
    Zh,
}

impl Lang {
    /// Map a frontend locale code to a Lang (unknown → Ru, the project's source language).
    pub fn parse(s: &str) -> Lang {
        match s {
            "en" => Lang::En,
            "zh" => Lang::Zh,
            _ => Lang::Ru,
        }
    }
    fn idx(self) -> usize {
        match self {
            Lang::Ru => 0,
            Lang::En => 1,
            Lang::Zh => 2,
        }
    }
}

// [ru, en, zh]. ru = the original hardcoded literal. Keep namespaces: tray.*, err.*, log.*, det.*.
#[rustfmt::skip]
const TABLE: &[(&str, [&str; 3])] = &[
    // ── tray ────────────────────────────────────────────────────────────────
    ("tray.show",      ["Показать окно", "Show window", "显示窗口"]),
    ("tray.check_all", ["Проверить всё", "Check all", "全部检查"]),
    ("tray.refresh_forks", ["Обновить форки", "Refresh forks", "刷新分叉"]),
    ("tray.refresh_providers", ["Обновить провайдеров", "Refresh providers", "刷新提供商"]),
    ("tray.stack_start", ["Запустить стек", "Start stack", "启动技术栈"]),
    ("tray.stack_stop", ["Остановить стек", "Stop stack", "停止技术栈"]),
    ("tray.open_backup", ["Открыть бэкапы", "Open Backup", "打开备份"]),
    ("tray.open_settings", ["Открыть настройки", "Open Settings", "打开设置"]),
    ("tray.cancel_all", ["Отменить всё", "Cancel all", "全部取消"]),
    ("tray.quit",      ["Выход", "Quit", "退出"]),
    ("tray.tooltip_sessions", ["Castellyn — активных сессий: {n}", "Castellyn — active sessions: {n}", "Castellyn — 活动会话: {n}"]),

    // ── agent-status notifications (Sessions) ────────────────────────────────
    ("status.blocked_title", ["Агент ждёт решения", "Agent needs a decision", "代理等待决定"]),
    ("status.blocked_body",  ["{label}: нужен ваш ответ в терминале", "{label}: your input is needed in the terminal", "{label}: 终端中需要你的确认"]),
    ("status.done_title",    ["Агент закончил", "Agent finished", "代理已完成"]),
    ("status.done_body",     ["{label}: ход завершён — можно смотреть результат", "{label}: the turn finished — ready for review", "{label}: 本轮已结束 — 可以查看结果"]),
    ("limits.crit_title",    ["Лимит почти исчерпан", "Usage limit almost reached", "用量即将达到上限"]),
    ("limits.crit_body",     ["{profile}: {window} на {pct}% — лимит почти исчерпан", "{profile}: {window} at {pct}% — usage limit almost reached", "{profile}: {window} 已用 {pct}% — 即将达到上限"]),

    // ── config write (touched by language-preserving write_config) ───────────
    ("err.no_appdata",    ["APPDATA не найден", "APPDATA not found", "未找到 APPDATA"]),
    ("err.write_config",  ["запись config: {e}", "writing config: {e}", "写入 config: {e}"]),

    // ── command errors (surface as toasts) ───────────────────────────────────
    ("err.run_in_progress", ["Уже идёт другой прогон — дождись завершения или отмени.", "Another run is in progress — wait for it to finish or cancel.", "已有任务在运行 — 请等待其完成或取消。"]),
    ("err.spawn_failed", ["не удалось запустить {program}: {e}", "failed to launch {program}: {e}", "无法启动 {program}: {e}"]),
    ("err.unknown_component", ["неизвестный компонент {id}", "unknown component {id}", "未知组件 {id}"]),
    ("err.component_no_apply", ["компонент {name} не поддерживает применение", "component {name} does not support apply", "组件 {name} 不支持应用"]),
    ("err.forks_missing", ["компонент forks не найден в манифесте", "forks component not found in the manifest", "清单中未找到 forks 组件"]),
    ("err.unknown_forks_action", ["неизвестное действие forks: {action}", "unknown forks action: {action}", "未知的 forks 操作: {action}"]),
    ("err.repo_dir_missing", ["каталог репозитория не найден: {path}", "repository directory not found: {path}", "未找到仓库目录: {path}"]),
    ("err.fork_busy", ["этот форк уже обновляется", "this fork is already updating", "该 fork 正在更新中"]),
    ("err.pwsh_failed", ["не удалось запустить pwsh: {e}", "failed to launch pwsh: {e}", "无法启动 pwsh: {e}"]),
    ("err.unknown_backup_action", ["неизвестное действие backup: {action}", "unknown backup action: {action}", "未知的 backup 操作: {action}"]),
    ("err.unknown_profile", ["неизвестный профиль: {name}", "unknown profile: {name}", "未知配置: {name}"]),
    ("err.unknown_profiles_action", ["неизвестное действие profiles: {action}", "unknown profiles action: {action}", "未知的 profiles 操作: {action}"]),
    ("err.unknown_configdrift_action", ["неизвестное действие config-drift: {action}", "unknown config-drift action: {action}", "未知的 config-drift 操作: {action}"]),

    // ── MCP canonical-config CRUD + OpenCode fan-out (L2: were hardcoded English) ────────────
    ("err.unknown_action", ["неизвестное действие: {action}", "unknown action: {action}", "未知操作: {action}"]),
    ("err.no_userprofile", ["переменная USERPROFILE не задана", "USERPROFILE is not set", "未设置 USERPROFILE"]),
    ("err.mcp_name_required", ["укажите имя MCP-сервера", "MCP server name is required", "需要填写 MCP 服务器名称"]),
    ("err.mcp_invalid_json", ["некорректный JSON сервера: {e}", "invalid server JSON: {e}", "服务器 JSON 无效: {e}"]),
    ("err.mcp_def_not_object", ["определение сервера должно быть JSON-объектом", "server definition must be a JSON object", "服务器定义必须是 JSON 对象"]),
    ("err.mcp_write", ["запись .mcp.json: {e}", "writing .mcp.json: {e}", "写入 .mcp.json: {e}"]),
    ("err.mcp_read", ["чтение .mcp.json: {e}", "reading .mcp.json: {e}", "读取 .mcp.json: {e}"]),
    ("err.mcp_parse", ["разбор .mcp.json: {e}", "parsing .mcp.json: {e}", "解析 .mcp.json: {e}"]),
    ("err.mcp_no_servers", ["в .mcp.json нет mcpServers", "no mcpServers in .mcp.json", ".mcp.json 中没有 mcpServers"]),
    ("err.mcp_unsafe_chars", ["{name}: значения содержат небезопасные для cmd символы", "{name}: values contain characters unsafe for cmd", "{name}: 值包含对 cmd 不安全的字符"]),
    ("err.opencode_missing", ["opencode.json не найден (OpenCode установлен?)", "opencode.json not found (is OpenCode installed?)", "未找到 opencode.json（OpenCode 是否已安装？）"]),
    ("err.opencode_parse", ["разбор opencode.json: {e}", "parsing opencode.json: {e}", "解析 opencode.json: {e}"]),
    ("err.opencode_not_object", ["opencode.json не является JSON-объектом", "opencode.json is not a JSON object", "opencode.json 不是 JSON 对象"]),
    ("err.myproviders_read", ["чтение myproviders.json: {e}", "reading myproviders.json: {e}", "读取 myproviders.json: {e}"]),
    ("err.myproviders_parse", ["разбор myproviders.json: {e}", "parsing myproviders.json: {e}", "解析 myproviders.json: {e}"]),
    ("err.no_providers", ["в реестре нет провайдеров — добавьте их на вкладке «Провайдеры»", "no providers in the registry — add them on the Providers tab", "注册表中没有提供商 — 请在“提供商”标签页添加"]),
    ("err.canon_rules_missing", ["канонические файлы правил не найдены (config\\CLAUDE.md / RTK.md)", "canonical rule files not found (config\\CLAUDE.md / RTK.md)", "未找到规范规则文件（config\\CLAUDE.md / RTK.md）"]),
    ("err.gateway_missing", ["сервис gateway не найден в stack.json", "gateway service not found in stack.json", "在 stack.json 中未找到 gateway 服务"]),
    ("err.codex_missing", ["config.toml не найден (Codex установлен?)", "config.toml not found (is Codex installed?)", "未找到 config.toml（Codex 是否已安装？）"]),
    ("err.rtk_not_found", ["бинарник rtk не найден (сначала установите rtk)", "rtk binary not found (install rtk first)", "未找到 rtk 可执行文件（请先安装 rtk）"]),
    ("err.rtk_not_managed", ["rtk.ts создан не Castellyn — файл не тронут", "rtk.ts is not Castellyn-managed — left untouched", "rtk.ts 并非由 Castellyn 管理 — 未做改动"]),
    ("err.fs_create", ["создание {path}: {e}", "creating {path}: {e}", "创建 {path}: {e}"]),
    ("err.fs_write", ["запись {path}: {e}", "writing {path}: {e}", "写入 {path}: {e}"]),
    ("err.fs_remove", ["удаление {path}: {e}", "removing {path}: {e}", "删除 {path}: {e}"]),
    ("err.invalid_profile_name", ["недопустимое имя профиля: {name}", "invalid profile name: {name}", "配置名称无效: {name}"]),
    ("err.invalid_profile_name_plain", ["недопустимое имя профиля", "invalid profile name", "配置名称无效"]),
    ("err.invalid_new_name", ["недопустимое новое имя: {nn}", "invalid new name: {nn}", "新名称无效: {nn}"]),
    ("err.no_color", ["не указан цвет", "no color specified", "未指定颜色"]),
    ("err.unknown_profile_action", ["неизвестное действие профиля: {action}", "unknown profile action: {action}", "未知的配置操作: {action}"]),
    ("err.orphan_is_canon", ["«{n}» — канонический профиль; удаляйте его во вкладке «Профили», а не как сироту", "'{n}' is a canon profile; delete it from the Profiles tab, not as an orphan", "「{n}」是规范配置；请在「配置」标签中删除，而非作为孤立目录"]),
    ("err.orphan_not_found", ["посторонний каталог «{n}» не найден или больше не сирота", "orphan directory '{n}' not found or no longer an orphan", "未找到孤立目录「{n}」或它已不再是孤立目录"]),
    ("err.orphan_bad_name", ["недопустимое имя сироты: {n}", "invalid orphan name: {n}", "孤立目录名称无效: {n}"]),
    ("err.orphan_session_active", ["профиль «{n}» сейчас используется активной сессией — закройте её и повторите", "profile '{n}' is in use by a live session — close it and retry", "配置「{n}」正被活动会话使用——请关闭后重试"]),
    ("err.orphan_has_links", ["каталог «.claude-{n}» содержит junction-ссылки в общие данные — удалите его вручную (через путь \\\\?\\), чтобы не задеть общий контент", "'.claude-{n}' contains junction links into shared data — delete it manually (via a \\\\?\\ path) so the shared content is not swept in", "「.claude-{n}」包含指向共享数据的 junction 链接——请手动删除（使用 \\\\?\\ 路径），以免波及共享内容"]),
    ("err.elevation_cancelled", ["Повышение прав отменено.", "Elevation cancelled.", "已取消提权。"]),
    ("err.unknown_sync_action", ["неизвестное действие sync: {action}", "unknown sync action: {action}", "未知的 sync 操作: {action}"]),
    ("err.invalid_service_id", ["недопустимый id сервиса: {id}", "invalid service id: {id}", "服务 id 无效: {id}"]),
    ("err.unknown_stack_action", ["неизвестное действие стека: {action}", "unknown stack action: {action}", "未知的 stack 操作: {action}"]),
    ("err.engines_no_array", ["engines.json: нет массива engines", "engines.json: no engines array", "engines.json: 缺少 engines 数组"]),
    ("err.engine_not_found", ["движок '{id}' не найден", "engine '{id}' not found", "未找到引擎 '{id}'"]),
    ("err.engine_not_found_json", ["движок '{id}' не найден в engines.json", "engine '{id}' not found in engines.json", "engines.json 中未找到引擎 '{id}'"]),
    ("err.unknown_engine_action", ["неизвестное действие engine: {action}", "unknown engine action: {action}", "未知的 engine 操作: {action}"]),
    ("err.launch_file_missing", ["файл запуска не найден: {cmd}", "launch file not found: {cmd}", "未找到启动文件: {cmd}"]),
    ("err.unknown_router_action", ["неизвестное действие router: {action}", "unknown router action: {action}", "未知的 router 操作: {action}"]),
    ("err.configure_needs_backend_model", ["для configure нужны backend и model", "configure requires backend and model", "configure 需要 backend 和 model"]),
    ("err.needs_backend_model", ["нужны backend и model", "backend and model are required", "需要 backend 和 model"]),
    ("err.invalid_profile", ["недопустимый профиль: {profile}", "invalid profile: {profile}", "配置无效: {profile}"]),
    ("err.unknown_provider_action", ["неизвестное действие provider: {action}", "unknown provider action: {action}", "未知的 provider 操作: {action}"]),
    ("err.set_needs_baseurl", ["для set нужен baseUrl", "set requires baseUrl", "set 需要 baseUrl"]),
    ("err.url_scheme", ["URL должен начинаться с http:// или https://", "URL must start with http:// or https://", "URL 必须以 http:// 或 https:// 开头"]),
    ("err.empty_host", ["пустой хост в URL", "empty host in URL", "URL 中主机为空"]),
    ("err.blocked_host", ["адрес заблокирован (SSRF/cloud-metadata): {host}", "address blocked (SSRF/cloud-metadata): {host}", "地址被阻止 (SSRF/cloud-metadata): {host}"]),
    ("err.https_required", ["нужен https (http разрешён только для localhost)", "https required (http allowed only for localhost)", "需要 https（http 仅允许用于 localhost）"]),
    ("err.invalid_provider_name", ["недопустимое имя провайдера (1–64 символа, без управляющих)", "invalid provider name (1–64 chars, no control characters)", "提供商名称无效 (1–64 个字符，无控制字符)"]),
    ("err.invalid_protocol", ["protocol должен быть anthropic или openai", "protocol must be anthropic or openai", "protocol 必须为 anthropic 或 openai"]),
    ("err.invalid_connectvia", ["connectVia должен быть freellmapi или direct", "connectVia must be freellmapi or direct", "connectVia 必须为 freellmapi 或 direct"]),
    ("err.empty_key", ["пустой ключ", "empty key", "密钥为空"]),
    ("err.provider_not_found", ["провайдер не найден", "provider not found", "未找到提供商"]),
    ("err.key_not_found", ["ключ не найден", "key not found", "未找到密钥"]),
    ("err.freellmapi_creds_needed", ["укажите email+пароль или токен дашборда freellmapi", "provide an email+password or a freellmapi dashboard token", "请提供 email+密码或 freellmapi 仪表板令牌"]),
    ("err.freellmapi_key_invalid", ["можно удалять только email, password или token", "only email, password, or token can be deleted", "只能删除 email、password 或 token"]),
    ("err.provider_no_apikey", ["для этого провайдера не задан API-ключ", "no API key is set for this provider", "该提供商未设置 API 密钥"]),
    ("err.direct_needs_profile", ["для прямого подключения укажите корректный целевой профиль", "direct connection needs a valid target profile", "直连需要有效的目标配置"]),
    ("err.freellmapi_login_first", ["сначала задайте вход в freellmapi (email+пароль или токен) — кнопка «Вход freellmapi»", "sign in to freellmapi first (email+password or token) — the “freellmapi login” button", "请先登录 freellmapi (email+密码或令牌) — “freellmapi 登录”按钮"]),
    ("err.no_gateway", ["не найден gateway в stack.json", "gateway not found in stack.json", "stack.json 中未找到 gateway"]),
    ("err.unknown_connectvia_protocol", ["неизвестная комбинация connectVia/protocol: {via}/{protocol}", "unknown connectVia/protocol combination: {via}/{protocol}", "未知的 connectVia/protocol 组合: {via}/{protocol}"]),
    ("err.single_key", ["у провайдера только один ключ — добавьте ещё для ротации", "the provider has only one key — add another to rotate", "该提供商只有一个密钥 — 请再添加一个以轮换"]),
    ("err.unknown_opencode_action", ["неизвестное действие opencode: {action}", "unknown opencode action: {action}", "未知的 opencode 操作: {action}"]),
    ("err.invalid_provider_id", ["недопустимый provider id: {id}", "invalid provider id: {id}", "provider id 无效: {id}"]),
    ("err.set_needs_base_url", ["для set нужен base_url", "set requires base_url", "set 需要 base_url"]),
    ("err.unknown_schedule_action", ["неизвестное действие schedule: {action}", "unknown schedule action: {action}", "未知的 schedule 操作: {action}"]),
    ("err.unknown_mcp_action", ["неизвестное действие mcp: {action}", "unknown mcp action: {action}", "未知的 mcp 操作: {action}"]),
    ("err.claude_launch", ["запуск claude: {e}", "launching claude: {e}", "启动 claude: {e}"]),
    ("err.invalid_plugin_id", ["недопустимый id плагина: {id}", "invalid plugin id: {id}", "插件 id 无效: {id}"]),
    ("err.unknown_plugin_action", ["неизвестное действие plugin: {action}", "unknown plugin action: {action}", "未知的 plugin 操作: {action}"]),
    ("err.bad_path", ["неверный путь", "invalid path", "路径无效"]),
    ("err.skills_dir", ["папка скиллов: {e}", "skills folder: {e}", "技能文件夹: {e}"]),
    ("err.skill_not_found", ["скилл не найден: {e}", "skill not found: {e}", "未找到技能: {e}"]),
    ("err.skill_not_in_skills", ["скилл не в ~/.claude/skills (скиллы из плагинов удаляются вместе с плагином)", "skill is not in ~/.claude/skills (plugin skills are removed with their plugin)", "技能不在 ~/.claude/skills 中 (插件技能随插件一起删除)"]),
    ("err.remove_link", ["удаление ссылки: {e}", "removing link: {e}", "删除链接: {e}"]),
    ("err.remove", ["удаление: {e}", "removing: {e}", "删除: {e}"]),
    ("err.write", ["запись: {e}", "writing: {e}", "写入: {e}"]),
    ("err.read", ["чтение: {e}", "reading: {e}", "读取: {e}"]),
    ("err.bad_config_file", ["неверный файл настроек: {e}", "invalid settings file: {e}", "设置文件无效: {e}"]),
    ("err.unknown_mode", ["неизвестный режим: {mode}", "unknown mode: {mode}", "未知模式: {mode}"]),
    ("err.measure_timeout", ["измерение превысило 180с — модель не ответила", "measurement exceeded 180s — the model did not respond", "测量超过 180 秒 — 模型未响应"]),
    ("err.claude_failed", ["claude не запустился: {e}", "claude failed to start: {e}", "claude 启动失败: {e}"]),
    ("err.parse_claude", ["не удалось разобрать ответ claude: {e}", "failed to parse claude response: {e}", "无法解析 claude 响应: {e}"]),
    ("err.no_usage_tokens", ["в ответе нет usage.input_tokens", "no usage.input_tokens in the response", "响应中没有 usage.input_tokens"]),
    ("err.unsupported_launch_mode", ["неподдерживаемый режим запуска: {mode}", "unsupported launch mode: {mode}", "不支持的启动模式: {mode}"]),
    ("err.open_terminal", ["не удалось открыть терминал: {e}", "failed to open terminal: {e}", "无法打开终端: {e}"]),
    ("err.dir_not_found", ["каталог не найден: {path}", "directory not found: {path}", "未找到目录: {path}"]),
    ("err.open_path", ["не удалось открыть {path}: {e}", "failed to open {path}: {e}", "无法打开 {path}: {e}"]),
    ("err.no_active_run", ["Нет активного прогона", "No active run", "没有正在进行的任务"]),
    ("err.kill_failed", ["Не удалось остановить процесс (возможно, он запущен от администратора): {e}", "Could not stop the process (it may be running elevated): {e}", "无法停止进程（可能以管理员身份运行）：{e}"]),
    ("err.bad_url_scheme", ["Отклонён небезопасный URL (разрешены только http/https): {url}", "Refused an unsafe URL (only http/https are allowed): {url}", "已拒绝不安全的 URL（仅允许 http/https）：{url}"]),
    ("err.clone_target_exists", ["Папка назначения уже существует: {path}", "Target folder already exists: {path}", "目标文件夹已存在：{path}"]),
    ("err.git_failed", ["Не удалось запустить git: {e}", "Could not run git: {e}", "无法运行 git：{e}"]),
    ("err.tar_failed", ["Не удалось запустить tar: {e}", "Could not run tar: {e}", "无法运行 tar：{e}"]),
    ("err.bad_hotkey", ["неверная комбинация: {e}", "invalid shortcut: {e}", "快捷键无效: {e}"]),
    ("err.unknown_tool", ["неизвестный инструмент: {tool}", "unknown tool: {tool}", "未知工具: {tool}"]),
    ("err.session_limit", ["достигнут предел сессий ({max})", "session limit reached ({max})", "已达到会话上限 ({max})"]),
    ("err.session_not_found", ["сессия не найдена", "session not found", "未找到会话"]),

    // ── run-log stream (console panel) ───────────────────────────────────────
    ("log.detached_launch", ["Запрошен запуск в отдельном окне. Статус обновится по проверке порта.", "Launch in a separate window requested. Status will update on the next port check.", "已请求在独立窗口中启动。状态将在下次端口检查时更新。"]),
    ("log.spawn_failed_indent", ["    не удалось запустить: {e}", "    failed to launch: {e}", "    无法启动: {e}"]),
    ("log.cfg_need_backend", ["ОШИБКА: для configure нужен -Backend (URL движка).", "ERROR: configure needs -Backend (the engine URL).", "错误: configure 需要 -Backend (引擎 URL)。"]),
    ("log.cfg_need_model", ["ОШИБКА: для configure нужен -Model (например, из «Загрузить модели»).", "ERROR: configure needs -Model (e.g. from “Load models”).", "错误: configure 需要 -Model (例如来自“加载模型”)。"]),
    ("log.provider_line", ["  Провайдер '{name}' -> {api_base}  (модель {model}); Router.default = {name},{model}", "  Provider '{name}' -> {api_base}  (model {model}); Router.default = {name},{model}", "  提供商 '{name}' -> {api_base}  (模型 {model}); Router.default = {name},{model}"]),
    ("log.ser_config", ["ОШИБКА сериализации config.json: {e}", "ERROR serializing config.json: {e}", "序列化 config.json 出错: {e}"]),
    ("log.write_config_err", ["ОШИБКА записи config.json: {e}", "ERROR writing config.json: {e}", "写入 config.json 出错: {e}"]),
    ("log.config_written", ["  config.json записан (бэкап .bak).", "  config.json written (.bak backup).", "  已写入 config.json (.bak 备份)。"]),
    ("log.ccr_already", ["  ccr уже установлен.", "  ccr is already installed.", "  ccr 已安装。"]),
    ("log.npm_missing", ["  ОШИБКА: npm не найден на PATH (нужен Node.js).", "  ERROR: npm not found on PATH (Node.js required).", "  错误: PATH 中未找到 npm (需要 Node.js)。"]),
    ("log.ccr_installed", ["  ccr установлен.", "  ccr installed.", "  ccr 已安装。"]),
    ("log.ccr_unconfirmed", ["  Не удалось подтвердить установку ccr.", "  Could not confirm the ccr installation.", "  无法确认 ccr 安装。"]),
    ("log.ccr_done_hint", ["  Готово. Навесь на профиль пресет «Claude Code Router» (http://127.0.0.1:3456).", "  Done. Attach the “Claude Code Router” preset to a profile (http://127.0.0.1:3456).", "  完成。为配置附加“Claude Code Router”预设 (http://127.0.0.1:3456)。"]),
    ("log.router_connect_header", ["=== Подключение через роутер: {name} → профиль {profile} ===", "=== Connecting via router: {name} → profile {profile} ===", "=== 通过路由连接: {name} → 配置 {profile} ==="]),
    ("log.aborted_ccr_setup", ["Прервано: не удалось настроить ccr.", "Aborted: could not configure ccr.", "已中止: 无法配置 ccr。"]),
    ("log.ccr_listening", ["  ccr слушает :3456 ✓", "  ccr is listening on :3456 ✓", "  ccr 正在监听 :3456 ✓"]),
    ("log.ccr_port_warn", ["  [ВНИМАНИЕ] ccr не поднял порт :3456. Конфиг и привязка сделаны, но сервер не запущен.", "  [WARNING] ccr did not open port :3456. Config and binding are done, but the server is not running.", "  [警告] ccr 未开启端口 :3456。配置和绑定已完成，但服务器未运行。"]),
    ("log.ccr_port_hint", ["            Попробуй обновить ccr (вкладка «Обновления») или запусти «ccr code» в терминале.", "            Try updating ccr (the “Updates” tab) or run “ccr code” in a terminal.", "            尝试更新 ccr (“更新”标签) 或在终端运行“ccr code”。"]),
    ("log.aborted_bind", ["Прервано: не удалось привязать профиль.", "Aborted: could not bind the profile.", "已中止: 无法绑定配置。"]),
    ("log.router_done", ["Готово. Профиль '{profile}' → {ccr_base} (ccr) → {name} ({model}). Перезапусти профиль.", "Done. Profile '{profile}' → {ccr_base} (ccr) → {name} ({model}). Restart the profile.", "完成。配置 '{profile}' → {ccr_base} (ccr) → {name} ({model})。请重启配置。"]),
    ("log.profile_not_found", ["ОШИБКА: профиль '{name}' не найден ({known}).", "ERROR: profile '{name}' not found ({known}).", "错误: 未找到配置 '{name}' ({known})。"]),
    ("log.profile_running_warn", ["⚠️ Профиль '{name}' похоже сейчас запущен (недавняя активность сессии). Закрой его сессию Claude и повтори — смена привязки провайдера у живой сессии может её сломать (как было с cc3). Если профиль не запущен, подожди ~2 минуты и повтори.", "⚠️ Profile '{name}' looks like it is running (recent session activity). Close its Claude session and retry — rebinding the provider of a live session can break it (as happened with cc3). If the profile is not running, wait ~2 minutes and retry.", "⚠️ 配置 '{name}' 似乎正在运行 (近期有会话活动)。请关闭其 Claude 会话后重试 — 对活动会话重新绑定提供商可能导致其损坏 (如 cc3 的情况)。若配置未运行，请等待约 2 分钟后重试。"]),
    ("log.no_userprofile", ["USERPROFILE не задан", "USERPROFILE is not set", "未设置 USERPROFILE"]),
    ("log.read_settings", ["ОШИБКА: не удалось прочитать settings.json ({e}).", "ERROR: could not read settings.json ({e}).", "错误: 无法读取 settings.json ({e})。"]),
    ("log.unchanged", ["(без изменений)", "(unchanged)", "(无变化)"]),
    ("log.set_value", ["(задан)", "(set)", "(已设置)"]),
    ("log.literal", ["(литерал)", "(literal)", "(字面值)"]),
    ("log.provider_reset", ["  Провайдер сброшен на стандартный Anthropic-логин.", "  Provider reset to the default Anthropic login.", "  提供商已重置为默认 Anthropic 登录。"]),
    ("log.ser_settings", ["ОШИБКА сериализации settings.json: {e}", "ERROR serializing settings.json: {e}", "序列化 settings.json 出错: {e}"]),
    ("log.write_settings", ["ОШИБКА записи settings.json: {e}", "ERROR writing settings.json: {e}", "写入 settings.json 出错: {e}"]),
    ("log.settings_updated", ["  settings.json обновлён (бэкап .bak). Перезапустите профиль '{name}', чтобы провайдер применился.", "  settings.json updated (.bak backup). Restart profile '{name}' for the provider to take effect.", "  已更新 settings.json (.bak 备份)。请重启配置 '{name}' 以使提供商生效。"]),
    ("log.freellmapi_no_creds", ["ОШИБКА: нет токена и неполные email/пароль freellmapi.", "ERROR: no token and incomplete freellmapi email/password.", "错误: 没有令牌且 freellmapi email/密码不完整。"]),
    ("log.freellmapi_login", ["  Вход в freellmapi (email+пароль)…", "  Signing in to freellmapi (email+password)…", "  正在登录 freellmapi (email+密码)…"]),
    ("log.freellmapi_no_token", ["  ОШИБКА входа в freellmapi: login не вернул токен", "  freellmapi login error: login did not return a token", "  freellmapi 登录错误: login 未返回令牌"]),
    ("log.freellmapi_401", ["  ОШИБКА входа (401): неверный email или пароль freellmapi.", "  Login error (401): wrong freellmapi email or password.", "  登录错误 (401): freellmapi email 或密码错误。"]),
    ("log.freellmapi_429", ["  ОШИБКА входа (429): слишком много попыток, подождите ~15 мин.", "  Login error (429): too many attempts, wait ~15 min.", "  登录错误 (429): 尝试次数过多，请等待约 15 分钟。"]),
    ("log.freellmapi_login_err", ["  ОШИБКА входа в freellmapi: {e}", "  freellmapi login error: {e}", "  freellmapi 登录错误: {e}"]),
    ("log.freellmapi_register_header", ["=== freellmapi: регистрация custom-провайдера ===", "=== freellmapi: registering a custom provider ===", "=== freellmapi: 注册自定义提供商 ==="]),
    ("log.provider_registered", ["  OK: провайдер зарегистрирован (keyId={key_id}, platform={platform}).", "  OK: provider registered (keyId={key_id}, platform={platform}).", "  OK: 提供商已注册 (keyId={key_id}, platform={platform})。"]),
    ("log.models_list", ["  Модели: {names}", "  Models: {names}", "  模型: {names}"]),
    ("log.freellmapi_done", ["  Готово. Провайдер доступен через freellmapi (:13001) для Claude Code (ccr) и opencode.", "  Done. The provider is available via freellmapi (:13001) for Claude Code (ccr) and opencode.", "  完成。该提供商可通过 freellmapi (:13001) 供 Claude Code (ccr) 和 opencode 使用。"]),
    ("log.freellmapi_auth_invalid", ["  ОШИБКА авторизации ({code}): сессия freellmapi недействительна — переавторизуйтесь (Вход freellmapi).", "  Authorization error ({code}): the freellmapi session is invalid — sign in again (freellmapi login).", "  授权错误 ({code}): freellmapi 会话无效 — 请重新登录 (freellmapi 登录)。"]),
    ("log.freellmapi_400", ["  ОШИБКА (400): freellmapi отклонил baseUrl или тело запроса.", "  ERROR (400): freellmapi rejected the baseUrl or the request body.", "  错误 (400): freellmapi 拒绝了 baseUrl 或请求体。"]),
    ("log.freellmapi_req_err", ["  ОШИБКА запроса к freellmapi: {e}", "  freellmapi request error: {e}", "  freellmapi 请求错误: {e}"]),
    ("log.read_opencode", ["ОШИБКА: не удалось прочитать opencode.json ({e}).", "ERROR: could not read opencode.json ({e}).", "错误: 无法读取 opencode.json ({e})。"]),
    ("log.provider_removed", ["  Провайдер '{provider_id}' удалён из opencode.json.", "  Provider '{provider_id}' removed from opencode.json.", "  已从 opencode.json 中移除提供商 '{provider_id}'。"]),
    ("log.ser_opencode", ["ОШИБКА сериализации opencode.json: {e}", "ERROR serializing opencode.json: {e}", "序列化 opencode.json 出错: {e}"]),
    ("log.write_opencode", ["ОШИБКА записи opencode.json: {e}", "ERROR writing opencode.json: {e}", "写入 opencode.json 出错: {e}"]),
    ("log.opencode_updated", ["  opencode.json обновлён (бэкап .bak): {cfg_path}", "  opencode.json updated (.bak backup): {cfg_path}", "  已更新 opencode.json (.bak 备份): {cfg_path}"]),
    ("log.claude_spawn", ["    не удалось запустить claude: {e}", "    failed to launch claude: {e}", "    无法启动 claude: {e}"]),
    ("log.claude_not_found", ["claude CLI не найден на PATH.", "claude CLI not found on PATH.", "PATH 中未找到 claude CLI。"]),
    ("log.plugin_header", ["=== Плагин: {action} {id} ===", "=== Plugin: {action} {id} ===", "=== 插件: {action} {id} ==="]),
    ("log.plugin_skip", ["  [skip] {p} (нет каталога)", "  [skip] {p} (no directory)", "  [skip] {p} (无目录)"]),
    ("log.done", ["Готово.", "Done.", "完成。"]),
    ("log.bulk_cancelled", ["Массовая операция отменена.", "Bulk operation cancelled.", "批量操作已取消。"]),

    // ── relink (elevated, embedded in a PowerShell Write-Host — keep apostrophe-free) ──
    ("log.relink_start", ["Запуск починки связей от администратора (подтвердите UAC)…", "Running link repair as administrator (confirm UAC)…", "正在以管理员身份修复链接 (请确认 UAC)…"]),
    ("log.relink_error_code", ["Ошибка починки, код ", "Repair failed, code ", "修复失败，代码 "]),
    ("log.relink_cancelled", ["Повышение прав отменено или не удалось.", "Elevation cancelled or failed.", "提权已取消或失败。"]),

    // ── provider-test JSON details (shown in the provider test UI) ────────────
    ("det.responded_models", ["ответил (моделей: {n})", "responded (models: {n})", "已响应 (模型数: {n})"]),
    ("det.key_rejected", ["ключ отклонён ({code})", "key rejected ({code})", "密钥被拒绝 ({code})"]),
    ("det.responds_http", ["отвечает (HTTP {code})", "responds (HTTP {code})", "有响应 (HTTP {code})"]),
    ("det.no_response", ["не отвечает: {e}", "no response: {e}", "无响应: {e}"]),
    ("det.no_balance_number", ["не нашёл число баланса в ответе", "could not find a balance number in the response", "在响应中未找到余额数字"]),
    ("det.balance_no_response", ["balance-URL не ответил", "balance-URL did not respond", "balance-URL 无响应"]),
    ("det.limit", ["лимит", "limit", "额度"]),
    ("det.balance_unavailable", ["баланс недоступен — задайте balance-URL в настройках провайдера", "balance unavailable — set a balance-URL in the provider settings", "余额不可用 — 请在提供商设置中设置 balance-URL"]),
];

/// Translate a key to `lang`. Unknown key → the key itself (visible-but-stable, like the JS t()).
pub fn tr(key: &str, lang: Lang) -> &'static str {
    for (k, vals) in TABLE {
        if *k == key {
            let v = vals[lang.idx()];
            return if v.is_empty() {
                vals[Lang::En.idx()]
            } else {
                v
            };
        }
    }
    // Leak nothing: return a 'static fallback. The key isn't 'static here, so callers that
    // need the key echoed use trv (owned String). For tr, an unknown key is a bug — surface it.
    "?"
}

/// Translate with `{name}` interpolation, mirroring the frontend interpolate(). Returns an owned
/// String. Values are any Display (so call sites pass `&id`, `&e`, `&n` without `.to_string()`).
/// Unknown key → echoes the key so the miss is visible in the UI/log.
pub fn trv(key: &str, lang: Lang, vars: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut found = None;
    for (k, vals) in TABLE {
        if *k == key {
            let v = vals[lang.idx()];
            found = Some(if v.is_empty() {
                vals[Lang::En.idx()]
            } else {
                v
            });
            break;
        }
    }
    let mut s = match found {
        Some(t) => t.to_string(),
        None => key.to_string(),
    };
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_language() {
        assert_eq!(tr("tray.quit", Lang::Ru), "Выход");
        assert_eq!(tr("tray.quit", Lang::En), "Quit");
        assert_eq!(tr("tray.quit", Lang::Zh), "退出");
    }

    #[test]
    fn trv_interpolates() {
        assert_eq!(
            trv("tray.tooltip_sessions", Lang::En, &[("n", &3)]),
            "Castellyn — active sessions: 3"
        );
    }

    #[test]
    fn unknown_key_echoes_in_trv() {
        assert_eq!(trv("err.nope", Lang::En, &[]), "err.nope");
    }

    #[test]
    fn parse_maps_codes() {
        assert!(matches!(Lang::parse("en"), Lang::En));
        assert!(matches!(Lang::parse("zh"), Lang::Zh));
        assert!(matches!(Lang::parse("ru"), Lang::Ru));
        assert!(matches!(Lang::parse("xx"), Lang::Ru)); // unknown → source language
    }

    // The `{token}` set in a string (no regex crate — a tiny scan over `{…}` pairs).
    fn placeholders(s: &str) -> std::collections::BTreeSet<&str> {
        let mut out = std::collections::BTreeSet::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            rest = &rest[i + 1..];
            match rest.find('}') {
                Some(j) => {
                    out.insert(&rest[..j]);
                    rest = &rest[j + 1..];
                }
                None => break,
            }
        }
        out
    }

    // Gate the Rust locale table the same way parity.ts guards the JS locales: every row's ru/zh
    // must carry the exact same {placeholder} set as en, so a translator typo (e.g. {err} where en
    // has {e}) can never leak a literal "{err}" into a toast for one language. Empty = inherits en.
    #[test]
    fn table_placeholder_parity() {
        for (key, vals) in TABLE {
            let en = placeholders(vals[Lang::En.idx()]);
            for (lang, v) in [("ru", vals[Lang::Ru.idx()]), ("zh", vals[Lang::Zh.idx()])] {
                if v.is_empty() {
                    continue; // empty cell falls back to English in tr()/trv()
                }
                assert_eq!(placeholders(v), en, "placeholder drift in '{key}' [{lang}]");
            }
        }
    }
}
