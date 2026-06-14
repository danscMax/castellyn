// AgentHub — Tauri backend.
//   * Component manifest (embedded) → run a component's PowerShell script in -Check or -Apply
//     mode, streaming stdout/stderr to the UI.
//   * Single-run guard + cancel.
//   * System tray with minimize-to-tray.
// Paths resolve from $SCRIPTS_ROOT (fallback E:\Scripts) so the app survives a disk move.

use std::os::windows::process::CommandExt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Windows CREATE_NO_WINDOW — keep spawned console apps (pwsh/reg/taskkill) from flashing
/// a black console window in front of the GUI.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// Canonical manifest, embedded as a fallback. The live source of truth is the
// same file on disk (read at runtime by `manifest_text`) so the dashboard and
// the PowerShell tooling never desync.
const MANIFEST_FALLBACK: &str = include_str!("../../manifest/maintenance-manifest.json");

/// Persistent hub settings (%APPDATA%\agenthub\config.json).
#[derive(Serialize, Deserialize, Default, Clone)]
struct HubConfig {
    #[serde(rename = "scriptsRoot", default, skip_serializing_if = "Option::is_none")]
    scripts_root: Option<String>,
    #[serde(rename = "startHidden", default)]
    start_hidden: bool,
    #[serde(rename = "fetchTimeoutSec", default, skip_serializing_if = "Option::is_none")]
    fetch_timeout_sec: Option<u32>,
    #[serde(rename = "ghTimeoutSec", default, skip_serializing_if = "Option::is_none")]
    gh_timeout_sec: Option<u32>,
}

fn config_path() -> Option<String> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| format!("{a}\\agenthub\\config.json"))
}

/// Legacy config location from before the AgentHub rename; read as a fallback so a
/// user's saved scriptsRoot/timeouts survive the rename. Writes always go to config_path().
fn legacy_config_path() -> Option<String> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| format!("{a}\\claude-maintenance-hub\\config.json"))
}

fn read_config_at(path: Option<String>) -> Option<HubConfig> {
    let p = path?;
    let c = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(c.trim_start_matches('\u{feff}')).ok()
}

fn read_config_file() -> HubConfig {
    read_config_at(config_path())
        .or_else(|| read_config_at(legacy_config_path()))
        .unwrap_or_default()
}

/// Scripts root: $SCRIPTS_ROOT env → config.scriptsRoot → default E:\Scripts.
fn scripts_root() -> String {
    if let Ok(v) = std::env::var("SCRIPTS_ROOT") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Some(r) = read_config_file().scripts_root {
        if !r.trim().is_empty() {
            return r;
        }
    }
    "E:\\Scripts".to_string()
}

/// Read the canonical manifest from disk; fall back to the embedded copy if the
/// file is missing or unreadable (e.g. relocated exe without the repo).
fn manifest_text() -> String {
    let path = format!(
        "{}\\AgentHub\\manifest\\maintenance-manifest.json",
        scripts_root()
    );
    std::fs::read_to_string(&path).unwrap_or_else(|_| MANIFEST_FALLBACK.to_string())
}

#[derive(Deserialize, Clone)]
struct RawManifest {
    components: Vec<RawComponent>,
}

#[derive(Deserialize, Clone)]
struct RawComponent {
    id: String,
    name: String,
    group: String,
    #[serde(rename = "scriptRel")]
    script_rel: String,
    #[serde(rename = "checkArgs")]
    check_args: Vec<String>,
    #[serde(rename = "applyArgs")]
    apply_args: Vec<String>,
    #[serde(rename = "supportsApply")]
    supports_apply: bool,
    #[serde(rename = "lastJsonRel")]
    last_json_rel: Option<String>,
}

/// Component as sent to the UI (absolute paths, camelCase).
#[derive(Serialize, Clone)]
struct Component {
    id: String,
    name: String,
    group: String,
    #[serde(rename = "lastJson")]
    last_json: Option<String>,
    #[serde(rename = "supportsApply")]
    supports_apply: bool,
}

fn raw_components() -> Vec<RawComponent> {
    serde_json::from_str::<RawManifest>(&manifest_text())
        .map(|m| m.components)
        .unwrap_or_default()
}

fn abs(rel: &str) -> String {
    format!("{}\\{}", scripts_root(), rel)
}

#[tauri::command]
fn list_components() -> Vec<Component> {
    raw_components()
        .into_iter()
        .map(|c| Component {
            id: c.id,
            name: c.name,
            group: c.group,
            last_json: c.last_json_rel.as_deref().map(abs),
            supports_apply: c.supports_apply,
        })
        .collect()
}

/// Parse JSON tolerating a leading UTF-8 BOM — PowerShell writes some configs (e.g.
/// .backup-state.json) with one, and serde_json rejects a BOM otherwise.
fn parse_json_bom(content: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(content.trim_start_matches('\u{feff}'))
}

/// Read and parse a *.last.json status file. Returns null if it doesn't exist yet.
#[tauri::command]
fn read_status(path: String) -> Result<Option<serde_json::Value>, String> {
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_json_bom(&content)
            .map(Some)
            .map_err(|e| format!("parse {path}: {e}")),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {path}: {e}")),
    }
}

// Tracks the PID of the currently-running child (Some(0) = reserved/starting).
#[derive(Default)]
struct RunState(Mutex<Option<u32>>);

#[derive(Serialize, Clone)]
struct LogLine {
    component: String,
    stream: String,
    line: String,
}

#[derive(Serialize, Clone)]
struct RunDone {
    component: String,
    code: i32,
}

/// Spawn `pwsh -File <script> <args>`, streaming each output line to the UI via "run-log"
/// and finishing with "run-done" (component = `id`). Only one run at a time (RunState guard).
async fn spawn_streamed(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
    script: String,
    args: Vec<String>,
) -> Result<i32, String> {
    // Reserve the single run slot (guard dropped before any await).
    {
        let mut g = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return Err("Уже идёт другой прогон — дождись завершения или отмени.".into());
        }
        *g = Some(0);
    }

    let mut cmd = Command::new("pwsh");
    cmd.arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script);
    for a in &args {
        cmd.arg(a);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            *state.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(format!("не удалось запустить pwsh для {script}: {e}"));
        }
    };

    if let Some(pid) = child.id() {
        *state.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    }

    let stdout = child.stdout.take().ok_or("нет stdout")?;
    let stderr = child.stderr.take().ok_or("нет stderr")?;

    let (app_o, id_o) = (app.clone(), id.clone());
    let h_out = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_o.emit(
                "run-log",
                LogLine { component: id_o.clone(), stream: "out".into(), line },
            );
        }
    });
    let (app_e, id_e) = (app.clone(), id.clone());
    let h_err = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_e.emit(
                "run-log",
                LogLine { component: id_e.clone(), stream: "err".into(), line },
            );
        }
    });

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let _ = h_out.await;
    let _ = h_err.await;

    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let code = status.code().unwrap_or(-1);
    let _ = app.emit("run-done", RunDone { component: id, code });
    Ok(code)
}

/// Run a component's script in `mode` ("check" | "apply"). Only one run at a time.
#[tauri::command]
async fn run_component(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
    mode: String,
) -> Result<i32, String> {
    let comp = raw_components()
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("неизвестный компонент {id}"))?;

    let args = if mode == "apply" {
        if !comp.supports_apply {
            return Err(format!("компонент {} не поддерживает применение", comp.name));
        }
        comp.apply_args.clone()
    } else {
        comp.check_args.clone()
    };

    let script = abs(&comp.script_rel);
    spawn_streamed(app, state, id, script, args).await
}

/// Map a Forks-tab action to update-forks.ps1 args (without -Paths). Mutations carry
/// `-Yes -Unattended` because the script otherwise prompts (Read-Host) and would hang.
fn forks_action_args(action: &str) -> Option<Vec<String>> {
    let v: Vec<&str> = match action {
        "check" => vec!["-Unattended"],
        "plan" => vec![
            "-FfMain", "-DeleteMerged", "-NormalizeRemotes", "-Rebase", "-DryRun", "-Unattended",
        ],
        "ff" => vec!["-FfMain", "-Yes", "-Unattended"],
        "delete" => vec!["-DeleteMerged", "-Yes", "-Unattended"],
        "rebase" => vec!["-Rebase", "-Yes", "-Unattended"],
        "normalize" => vec!["-NormalizeRemotes", "-Yes", "-Unattended"],
        _ => return None,
    };
    Some(v.into_iter().map(String::from).collect())
}

/// Run a Forks-tab action. `path` (a repo path) scopes the action to one repo via -Paths;
/// omit it for the global read actions (check / plan).
#[tauri::command]
async fn run_forks(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    path: Option<String>,
) -> Result<i32, String> {
    let comp = raw_components()
        .into_iter()
        .find(|c| c.id == "forks")
        .ok_or("компонент forks не найден в манифесте")?;
    let mut args =
        forks_action_args(&action).ok_or_else(|| format!("неизвестное действие forks: {action}"))?;
    if let Some(p) = path {
        let mut full = vec!["-Paths".to_string(), p];
        full.append(&mut args);
        args = full;
    }
    // Optional timeouts from hub settings.
    let cfg = read_config_file();
    if let Some(t) = cfg.fetch_timeout_sec {
        args.push("-FetchTimeoutSec".into());
        args.push(t.to_string());
    }
    if let Some(t) = cfg.gh_timeout_sec {
        args.push("-GhTimeoutSec".into());
        args.push(t.to_string());
    }
    let script = abs(&comp.script_rel);
    spawn_streamed(app, state, "forks".to_string(), script, args).await
}

const BACKUP_DIR_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Backups";
const BACKUP_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Backup-ClaudeSetup.ps1";
const RESTORE_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Restore-ClaudeSetup.ps1";

#[derive(Serialize)]
struct BackupList {
    snapshots: Vec<String>,
    weeklies: Vec<String>,
    state: Option<serde_json::Value>,
}

/// Snapshot dir name format: yyyy-MM-dd_HHmmss.
fn is_snapshot_name(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 17 {
        return false;
    }
    let d = |i: usize| b[i].is_ascii_digit();
    d(0) && d(1) && d(2) && d(3) && b[4] == b'-' && d(5) && d(6) && b[7] == b'-' && d(8) && d(9)
        && b[10] == b'_' && d(11) && d(12) && d(13) && d(14) && d(15) && d(16)
}

/// List backup snapshots + weekly archives and parse .backup-state.json — one call so the
/// frontend never needs the absolute Backups path.
#[tauri::command]
fn list_backups() -> BackupList {
    let dir = abs(BACKUP_DIR_REL);
    let mut snapshots: Vec<String> = Vec::new();
    let mut weeklies: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && is_snapshot_name(&name) {
                snapshots.push(name);
            } else if !is_dir && name.starts_with("weekly-") && name.ends_with(".zip") {
                weeklies.push(name);
            }
        }
    }
    snapshots.sort();
    snapshots.reverse();
    weeklies.sort();
    weeklies.reverse();
    let state = std::fs::read_to_string(format!("{dir}\\.backup-state.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok());
    BackupList { snapshots, weeklies, state }
}

/// Run a Backup-tab action: create a snapshot, preview a restore (-WhatIf), or restore.
/// Restore is scoped by `timestamp`/`profiles`; credentials only with `include_credentials`.
#[tauri::command]
async fn run_backup(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    timestamp: Option<String>,
    profiles: Option<Vec<String>>,
    include_credentials: Option<bool>,
) -> Result<i32, String> {
    let (script_rel, mut args): (&str, Vec<String>) = match action.as_str() {
        "backup" => (BACKUP_SCRIPT_REL, vec!["-Force".to_string()]),
        "restore-preview" => (RESTORE_SCRIPT_REL, vec!["-WhatIf".to_string()]),
        "restore" => (RESTORE_SCRIPT_REL, Vec::new()),
        _ => return Err(format!("неизвестное действие backup: {action}")),
    };
    if action == "restore-preview" || action == "restore" {
        if let Some(t) = timestamp {
            if !t.is_empty() {
                args.push("-Timestamp".into());
                args.push(t);
            }
        }
        if let Some(ps) = profiles {
            if !ps.is_empty() {
                args.push("-Profiles".into());
                for p in ps {
                    args.push(p);
                }
            }
        }
    }
    if action == "restore" && include_credentials.unwrap_or(false) {
        args.push("-IncludeCredentials".into());
    }
    let script = abs(script_rel);
    spawn_streamed(app, state, "backup".to_string(), script, args).await
}

const PROFILES_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Get-ProfilesStatus.ps1";
const INSTALL_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Install-ClaudeProfiles.ps1";
const REPAIR_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Repair-ProfileLinks.ps1";
const PROFILES_JSON_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\profiles.last.json";

/// Read the cached profiles health snapshot (profiles.last.json). Null until first check.
#[tauri::command]
fn read_profiles() -> Result<Option<serde_json::Value>, String> {
    match std::fs::read_to_string(abs(PROFILES_JSON_REL)) {
        Ok(content) => parse_json_bom(&content)
            .map(Some)
            .map_err(|e| format!("parse profiles.last.json: {e}")),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read profiles.last.json: {e}")),
    }
}

/// Run a Profiles-tab action: refresh status, clean sync-conflict files, reinstall all profiles,
/// or repair the links of a single profile (`repair` requires `name`).
#[tauri::command]
async fn run_profiles(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    name: Option<String>,
) -> Result<i32, String> {
    let (script_rel, args): (&str, Vec<String>) = match action.as_str() {
        "check" => (PROFILES_SCRIPT_REL, Vec::new()),
        "clean-conflicts" => (PROFILES_SCRIPT_REL, vec!["-CleanConflicts".to_string()]),
        "reinstall" => (INSTALL_SCRIPT_REL, vec!["-Force".to_string()]),
        "repair" => {
            let n = name.unwrap_or_default();
            if !PROFILE_NAMES.contains(&n.as_str()) {
                return Err(format!("неизвестный профиль: {n}"));
            }
            (REPAIR_SCRIPT_REL, vec!["-Name".to_string(), n])
        }
        _ => return Err(format!("неизвестное действие profiles: {action}")),
    };
    let script = abs(script_rel);
    spawn_streamed(app, state, "profiles".to_string(), script, args).await
}

const PROFILES_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\profiles.json";
const PROFILE_MGMT_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Manage-Profiles.ps1";

/// Read the canonical profile config (config\profiles.json): names, colours, descriptions,
/// and each profile's linkedFolders. Null until the file exists.
#[tauri::command]
fn read_profiles_config() -> Result<Option<serde_json::Value>, String> {
    match std::fs::read_to_string(abs(PROFILES_CONFIG_REL)) {
        Ok(content) => parse_json_bom(&content)
            .map(Some)
            .map_err(|e| format!("parse profiles.json: {e}")),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read profiles.json: {e}")),
    }
}

/// Profile name validation: `[A-Za-z0-9][A-Za-z0-9_-]{0,31}` — keeps the shell call safe
/// (no spaces, quotes, path separators) and mirrors Manage-Profiles.ps1.
fn valid_profile_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars().next().map(|c| c.is_ascii_alphanumeric()).unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Profile lifecycle: add / remove / rename / recolor / set-links via Manage-Profiles.ps1.
#[tauri::command]
async fn run_profile_mgmt(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    name: String,
    new_name: Option<String>,
    color: Option<String>,
    description: Option<String>,
    enabled: Option<Vec<String>>,
) -> Result<i32, String> {
    if !valid_profile_name(&name) {
        return Err(format!("недопустимое имя профиля: {name}"));
    }
    let mut args: Vec<String> = vec!["-Action".into(), action.clone(), "-Name".into(), name];
    match action.as_str() {
        "add" => {
            args.push("-Color".into());
            args.push(color.unwrap_or_else(|| "White".into()));
            args.push("-Description".into());
            args.push(description.unwrap_or_default());
        }
        "remove" => {}
        "rename" => {
            let nn = new_name.unwrap_or_default();
            if !valid_profile_name(&nn) {
                return Err(format!("недопустимое новое имя: {nn}"));
            }
            args.push("-NewName".into());
            args.push(nn);
        }
        "recolor" => {
            args.push("-Color".into());
            args.push(color.ok_or("не указан цвет")?);
        }
        "set-links" => {
            args.push("-Enabled".into());
            args.push(enabled.unwrap_or_default().join(","));
        }
        _ => return Err(format!("неизвестное действие профиля: {action}")),
    }
    let script = abs(PROFILE_MGMT_SCRIPT_REL);
    spawn_streamed(app, state, "profiles".to_string(), script, args).await
}

const SYNC_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Manage-Sync.ps1";
const SYNC_JSON_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\sync.last.json";
const SYNC_ITEMS: [&str; 6] = [
    "history",
    "projects",
    "skills",
    "agents",
    "commands",
    "keybindings",
];

/// Read the cached sync status (sync.last.json). Null until first query.
#[tauri::command]
fn read_sync() -> Result<Option<serde_json::Value>, String> {
    match std::fs::read_to_string(abs(SYNC_JSON_REL)) {
        Ok(content) => parse_json_bom(&content)
            .map(Some)
            .map_err(|e| format!("parse sync.last.json: {e}")),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read sync.last.json: {e}")),
    }
}

/// Run a Sync-tab action: query status, or set the synced-items whitelist.
/// `set` takes `enabled` = the list of items to keep syncing (everything else is dropped).
#[tauri::command]
async fn run_sync(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    enabled: Option<Vec<String>>,
) -> Result<i32, String> {
    let args: Vec<String> = match action.as_str() {
        "query" => vec!["-Action".into(), "query".into()],
        "set" => {
            let items = enabled.unwrap_or_default();
            // Validate against the known item keys (ignore anything unexpected).
            let valid: Vec<String> = items
                .into_iter()
                .filter(|i| SYNC_ITEMS.contains(&i.as_str()))
                .collect();
            vec![
                "-Action".into(),
                "set".into(),
                "-Enabled".into(),
                valid.join(","),
            ]
        }
        _ => return Err(format!("неизвестное действие sync: {action}")),
    };
    let script = abs(SYNC_SCRIPT_REL);
    spawn_streamed(app, state, "sync".to_string(), script, args).await
}

// --- LLM provider per profile + local engine launcher ---
const ENGINES_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\engines.json";
const ENGINE_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Manage-Engine.ps1";
const PROVIDER_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Manage-Provider.ps1";
/// Per-profile launch config (full vs lean mode + which tools to re-include when lean).
const LAUNCH_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\profile-launch.json";

#[derive(Serialize)]
struct EngineStatus {
    id: String,
    name: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    protocol: String,
    port: u16,
    #[serde(rename = "dashboardUrl")]
    dashboard_url: String,
    #[serde(rename = "hasCommand")]
    has_command: bool,
    /// True for the claude-code-router bridge entry (gets install/configure controls in the UI).
    router: bool,
    /// For the router entry: is the `ccr` CLI present on PATH? (None for plain engines.)
    installed: Option<bool>,
    running: bool,
}

/// Is an executable `name` (with common Windows extensions) found on PATH?
fn cmd_on_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else { return false };
    let exts = ["", ".cmd", ".exe", ".ps1", ".bat"];
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            if dir.join(format!("{name}{ext}")).is_file() {
                return true;
            }
        }
    }
    false
}

/// Fast TCP probe: is something listening on 127.0.0.1:port?
fn port_listening(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(250)).is_ok()
}

/// Engine registry (config\engines.json) + live running status (port probe). Read-only.
#[tauri::command]
fn read_engines() -> Vec<EngineStatus> {
    let content = match std::fs::read_to_string(abs(ENGINES_CONFIG_REL)) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let v = match parse_json_bom(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(arr) = v.get("engines").and_then(|e| e.as_array()) else {
        return Vec::new();
    };
    let s = |e: &serde_json::Value, k: &str| {
        e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let mut engines: Vec<EngineStatus> = arr
        .iter()
        .map(|e| {
            let port = e.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16;
            let is_router = e.get("router").and_then(|r| r.as_bool()).unwrap_or(false);
            EngineStatus {
                id: s(e, "id"),
                name: s(e, "name"),
                base_url: s(e, "baseUrl"),
                protocol: s(e, "protocol"),
                port,
                dashboard_url: s(e, "dashboardUrl"),
                has_command: !s(e, "command").is_empty() || !s(e, "start").is_empty(),
                router: is_router,
                installed: if is_router { Some(cmd_on_path("ccr")) } else { None },
                running: false,
            }
        })
        .collect();
    // Probe ports concurrently: each port_listening blocks up to 250ms, so doing them
    // sequentially would be N*250ms. thread::scope keeps it bounded by the slowest single probe.
    let running: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = engines
            .iter()
            .map(|e| {
                let p = e.port;
                scope.spawn(move || port_listening(p))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(false)).collect()
    });
    for (e, r) in engines.iter_mut().zip(running) {
        e.running = r;
    }
    engines
}

/// Patch an engine's baseUrl + port in config\engines.json (user can change ports when
/// something else occupies the default). Read-modify-write, preserves everything else.
#[tauri::command]
fn update_engine(id: String, base_url: String, port: u16) -> Result<(), String> {
    let path = abs(ENGINES_CONFIG_REL);
    let content = std::fs::read_to_string(&path).map_err(|e| format!("read engines.json: {e}"))?;
    let mut v = parse_json_bom(&content).map_err(|e| format!("parse engines.json: {e}"))?;
    let arr = v
        .get_mut("engines")
        .and_then(|e| e.as_array_mut())
        .ok_or("engines.json: нет массива engines")?;
    let mut found = false;
    for e in arr.iter_mut() {
        if e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()) {
            e["baseUrl"] = serde_json::Value::String(base_url.clone());
            e["port"] = serde_json::Value::Number(port.into());
            found = true;
            break;
        }
    }
    if !found {
        return Err(format!("движок '{id}' не найден"));
    }
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write engines.json: {e}"))?;
    Ok(())
}

/// Start / stop a local engine via Manage-Engine.ps1 (streamed).
#[tauri::command]
async fn run_engine(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    id: String,
) -> Result<i32, String> {
    if !matches!(action.as_str(), "start" | "stop") {
        return Err(format!("неизвестное действие engine: {action}"));
    }
    let script = abs(ENGINE_SCRIPT_REL);
    spawn_streamed(
        app,
        state,
        "engine".to_string(),
        script,
        vec!["-Action".into(), action, "-Id".into(), id],
    )
    .await
}

const ROUTER_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Setup-Router.ps1";
const CONNECT_ROUTER_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Connect-Router.ps1";
const MODELS_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Get-EngineModels.ps1";

/// Install or configure claude-code-router (ccr) via Setup-Router.ps1 (streamed).
/// `configure` needs `backend` (engine baseUrl) + `model`.
#[tauri::command]
async fn run_router(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    backend: Option<String>,
    model: Option<String>,
    name: Option<String>,
) -> Result<i32, String> {
    if !matches!(action.as_str(), "install" | "configure") {
        return Err(format!("неизвестное действие router: {action}"));
    }
    let mut args: Vec<String> = vec!["-Action".into(), action.clone()];
    if action == "configure" {
        let b = backend.unwrap_or_default();
        let m = model.unwrap_or_default();
        if b.is_empty() || m.is_empty() {
            return Err("для configure нужны backend и model".into());
        }
        args.push("-Backend".into());
        args.push(b);
        args.push("-Model".into());
        args.push(m);
        if let Some(n) = name.filter(|s| !s.is_empty()) {
            args.push("-Name".into());
            args.push(n);
        }
    }
    let script = abs(ROUTER_SCRIPT_REL);
    spawn_streamed(app, state, "engine".to_string(), script, args).await
}

/// Turnkey: configure+start ccr for `backend`/`model` and bind `profile` to it (streamed).
#[tauri::command]
async fn run_connect_router(
    app: AppHandle,
    state: State<'_, RunState>,
    backend: String,
    model: String,
    profile: String,
    name: Option<String>,
) -> Result<i32, String> {
    if backend.is_empty() || model.is_empty() {
        return Err("нужны backend и model".into());
    }
    if !valid_profile_name(&profile) {
        return Err(format!("недопустимый профиль: {profile}"));
    }
    let mut args: Vec<String> = vec![
        "-Backend".into(),
        backend,
        "-Model".into(),
        model,
        "-Profile".into(),
        profile,
    ];
    if let Some(n) = name.filter(|s| !s.is_empty()) {
        args.push("-Name".into());
        args.push(n);
    }
    let script = abs(CONNECT_ROUTER_SCRIPT_REL);
    spawn_streamed(app, state, "provider".to_string(), script, args).await
}

/// Fetch model ids from an OpenAI-compatible engine (GET <baseUrl>/models). Empty on error.
#[tauri::command]
async fn read_engine_models(base_url: String) -> Vec<String> {
    if base_url.is_empty() {
        return Vec::new();
    }
    let script = abs(MODELS_SCRIPT_REL);
    let out = tokio::process::Command::new("pwsh")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script, "-BaseUrl", &base_url])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<Vec<String>>(stdout.trim()).unwrap_or_default()
}

#[derive(Serialize)]
struct ProfileProvider {
    name: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    model: String,
    #[serde(rename = "smallModel")]
    small_model: String,
    #[serde(rename = "hasToken")]
    has_token: bool,
}

/// Profile names from config\profiles.json (fallback to the built-in list).
fn profile_names() -> Vec<String> {
    if let Ok(c) = std::fs::read_to_string(abs(PROFILES_CONFIG_REL)) {
        if let Ok(v) = parse_json_bom(&c) {
            if let Some(arr) = v.get("profiles").and_then(|p| p.as_array()) {
                let names: Vec<String> = arr
                    .iter()
                    .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect();
                if !names.is_empty() {
                    return names;
                }
            }
        }
    }
    PROFILE_NAMES.iter().map(|s| s.to_string()).collect()
}

/// Per-profile provider, read natively from each settings.json env.
/// The token VALUE is never returned — only `hasToken`.
#[tauri::command]
fn read_providers() -> Vec<ProfileProvider> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for name in profile_names() {
        let path = format!("{home}\\.claude-{name}\\settings.json");
        let mut p = ProfileProvider {
            name: name.clone(),
            base_url: String::new(),
            model: String::new(),
            small_model: String::new(),
            has_token: false,
        };
        if let Ok(c) = std::fs::read_to_string(&path) {
            if let Ok(v) = parse_json_bom(&c) {
                if let Some(env) = v.get("env").and_then(|e| e.as_object()) {
                    let g = |k: &str| env.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    p.base_url = g("ANTHROPIC_BASE_URL");
                    p.model = g("ANTHROPIC_MODEL");
                    p.small_model = g("ANTHROPIC_SMALL_FAST_MODEL");
                    p.has_token = !g("ANTHROPIC_AUTH_TOKEN").is_empty();
                }
            }
        }
        out.push(p);
    }
    out
}

/// Bind (set) or unbind (clear) a profile's provider via Manage-Provider.ps1 (streamed).
#[tauri::command]
async fn run_provider(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    name: String,
    base_url: Option<String>,
    token: Option<String>,
    model: Option<String>,
    small_model: Option<String>,
    keep_token: Option<bool>,
) -> Result<i32, String> {
    if !matches!(action.as_str(), "set" | "clear") {
        return Err(format!("неизвестное действие provider: {action}"));
    }
    if !valid_profile_name(&name) {
        return Err(format!("недопустимое имя профиля: {name}"));
    }
    let mut args: Vec<String> = vec!["-Action".into(), action.clone(), "-Name".into(), name];
    if action == "set" {
        let b = base_url.unwrap_or_default();
        if b.is_empty() {
            return Err("для set нужен baseUrl".into());
        }
        args.push("-BaseUrl".into());
        args.push(b);
        // Model/small are readable → always authoritative (empty removes the override).
        args.push("-Model".into());
        args.push(model.unwrap_or_default());
        args.push("-SmallModel".into());
        args.push(small_model.unwrap_or_default());
        // Token: keep existing, or set/remove by the supplied value.
        if keep_token.unwrap_or(false) {
            args.push("-KeepToken".into());
        } else {
            args.push("-Token".into());
            args.push(token.unwrap_or_default());
        }
    }
    let script = abs(PROVIDER_SCRIPT_REL);
    spawn_streamed(app, state, "provider".to_string(), script, args).await
}

const MCP_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\.mcp.json";
const MCP_DEPLOY_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Deploy-Mcp.ps1";
const PROFILE_NAMES: [&str; 6] = ["ccmy", "cc1", "cc2", "cc3", "cc4", "cc5"];

#[derive(Serialize)]
struct McpServer {
    name: String,
    command: String,
    #[serde(rename = "deployedIn")]
    deployed_in: Vec<String>,
}

#[derive(Serialize)]
struct McpExtra {
    name: String,
    #[serde(rename = "presentIn")]
    present_in: Vec<String>,
}

#[derive(Serialize)]
struct McpStatus {
    source: Vec<McpServer>,
    extras: Vec<McpExtra>,
    /// Profile names whose .claude.json exists (i.e. could be inspected).
    profiles: Vec<String>,
}

/// Top-level (user-scope) mcpServers keys from a profile's .claude.json; None if unreadable.
fn profile_mcp_servers(name: &str) -> Option<Vec<String>> {
    let home = std::env::var("USERPROFILE").ok()?;
    let path = format!("{home}\\.claude-{name}\\.claude.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let v = parse_json_bom(&content).ok()?;
    let obj = v.get("mcpServers")?.as_object()?;
    Some(obj.keys().cloned().collect())
}

/// Inspect MCP config: source-of-truth servers (config/.mcp.json) vs what's actually deployed
/// per profile (.claude.json top-level mcpServers). Read-only.
#[tauri::command]
fn read_mcp() -> Result<McpStatus, String> {
    // Source-of-truth servers.
    let mut source_defs: Vec<(String, String)> = Vec::new(); // (name, command)
    if let Ok(content) = std::fs::read_to_string(abs(MCP_CONFIG_REL)) {
        if let Ok(v) = parse_json_bom(&content) {
            if let Some(obj) = v.get("mcpServers").and_then(|m| m.as_object()) {
                for (name, def) in obj {
                    let cmd = def
                        .get("command")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    source_defs.push((name.clone(), cmd));
                }
            }
        }
    }

    // Per-profile deployed sets.
    let mut existing_profiles: Vec<String> = Vec::new();
    let mut per_profile: Vec<(String, Vec<String>)> = Vec::new();
    for p in PROFILE_NAMES {
        if let Some(servers) = profile_mcp_servers(p) {
            existing_profiles.push(p.to_string());
            per_profile.push((p.to_string(), servers));
        }
    }

    let source: Vec<McpServer> = source_defs
        .iter()
        .map(|(name, cmd)| {
            let deployed_in = per_profile
                .iter()
                .filter(|(_, servers)| servers.iter().any(|s| s == name))
                .map(|(p, _)| p.clone())
                .collect();
            McpServer { name: name.clone(), command: cmd.clone(), deployed_in }
        })
        .collect();

    // Servers found in a profile but absent from the source-of-truth.
    let source_names: std::collections::HashSet<&str> =
        source_defs.iter().map(|(n, _)| n.as_str()).collect();
    let mut extras_map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (p, servers) in &per_profile {
        for s in servers {
            if !source_names.contains(s.as_str()) {
                extras_map.entry(s.clone()).or_default().push(p.clone());
            }
        }
    }
    let extras = extras_map
        .into_iter()
        .map(|(name, present_in)| McpExtra { name, present_in })
        .collect();

    Ok(McpStatus { source, extras, profiles: existing_profiles })
}

const SCHEDULE_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Schedule-Hub.ps1";
const SCHEDULES_JSON_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\schedules.last.json";

/// Refresh (run the helper's query) and read schedules.last.json. Not streamed.
#[tauri::command]
async fn read_schedules() -> Result<Option<serde_json::Value>, String> {
    let script = abs(SCHEDULE_SCRIPT_REL);
    let _ = tokio::process::Command::new("pwsh")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script,
            "-Action",
            "query",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    match std::fs::read_to_string(abs(SCHEDULES_JSON_REL)) {
        Ok(c) => parse_json_bom(&c).map(Some).map_err(|e| format!("parse schedules: {e}")),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read schedules: {e}")),
    }
}

/// Known schedule actions (whitelist mirrors the ScheduleTab UI + Schedule-Hub.ps1).
fn valid_schedule_action(a: &str) -> bool {
    matches!(a, "enable" | "disable" | "run" | "create" | "delete")
}

/// Manage a scheduled task: enable / disable / run / create / delete (streamed).
#[tauri::command]
async fn run_schedule(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    id: Option<String>,
    time: Option<String>,
) -> Result<i32, String> {
    if !valid_schedule_action(&action) {
        return Err(format!("неизвестное действие schedule: {action}"));
    }
    let mut args = vec!["-Action".to_string(), action];
    if let Some(i) = id {
        args.push("-Id".into());
        args.push(i);
    }
    if let Some(t) = time {
        args.push("-Time".into());
        args.push(t);
    }
    let script = abs(SCHEDULE_SCRIPT_REL);
    spawn_streamed(app, state, "schedule".to_string(), script, args).await
}

/// Run an MCP-tab action: deploy shared MCP servers into all profiles (Deploy-Mcp.ps1).
#[tauri::command]
async fn run_mcp(app: AppHandle, state: State<'_, RunState>, action: String) -> Result<i32, String> {
    let script_rel = match action.as_str() {
        "deploy" => MCP_DEPLOY_SCRIPT_REL,
        _ => return Err(format!("неизвестное действие mcp: {action}")),
    };
    let script = abs(script_rel);
    spawn_streamed(app, state, "mcp".to_string(), script, Vec::new()).await
}

const PLUGIN_MGR_SCRIPT_REL: &str = "claude-plugin-updater\\Manage-Plugin.ps1";

/// List installed plugins via `claude plugin list --json`.
#[tauri::command]
async fn list_plugins() -> Result<serde_json::Value, String> {
    let out = tokio::process::Command::new("pwsh")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "claude plugin list --json"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map_err(|e| format!("запуск claude: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_json_bom(stdout.trim()).map_err(|e| format!("parse plugins: {e}"))
}

#[derive(Serialize)]
struct SkillInfo {
    name: String,
    description: String,
    version: String,
    dir: String,
}

/// First-block YAML frontmatter (between the first `---` pair) of a SKILL.md.
fn extract_frontmatter(content: &str) -> String {
    let t = content.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            return rest[..end].to_string();
        }
    }
    String::new()
}

fn fm_value(fm: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in fm.lines() {
        if let Some(rest) = line.strip_prefix(&prefix) {
            let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// List skills under ~/.claude/skills with name/description/version from SKILL.md.
#[tauri::command]
fn list_skills() -> Vec<SkillInfo> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let root = format!("{home}\\.claude\\skills");
    let mut out: Vec<SkillInfo> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = e.file_name().to_string_lossy().to_string();
            let mut name = dir_name.clone();
            let mut description = String::new();
            let mut version = String::new();
            if let Ok(content) = std::fs::read_to_string(e.path().join("SKILL.md")) {
                let fm = extract_frontmatter(&content);
                if let Some(v) = fm_value(&fm, "name") {
                    name = v;
                }
                if let Some(v) = fm_value(&fm, "description") {
                    description = v;
                }
                if let Some(v) = fm_value(&fm, "version") {
                    version = v;
                }
            }
            out.push(SkillInfo { name, description, version, dir: e.path().display().to_string() });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[derive(Serialize)]
struct PluginUpdate {
    id: String,
    installed: String,
    available: String,
}

/// Detect plugins with an available update by comparing installed_plugins.json versions
/// against the on-disk marketplace manifests. Fast, read-only, no network.
#[tauri::command]
fn list_plugin_updates() -> Vec<PluginUpdate> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let plugins_dir = format!("{home}\\.claude\\plugins");
    let installed = match std::fs::read_to_string(format!("{plugins_dir}\\installed_plugins.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
    {
        Some(v) => v,
        None => return Vec::new(),
    };
    let markets = std::fs::read_to_string(format!("{plugins_dir}\\known_marketplaces.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .unwrap_or(serde_json::Value::Null);

    let mut out: Vec<PluginUpdate> = Vec::new();
    let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) else {
        return out;
    };
    for (id, arr) in po {
        let inst = arr
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e.get("version"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if inst.is_empty() || inst == "unknown" {
            continue;
        }
        let Some(at) = id.rfind('@') else { continue };
        let plugin_id = &id[..at];
        let mp_name = &id[at + 1..];
        let loc = markets
            .get(mp_name)
            .and_then(|m| m.get("installLocation"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{plugins_dir}\\marketplaces\\{mp_name}"));
        for cand in [
            format!("{loc}\\plugins\\{plugin_id}\\.claude-plugin\\plugin.json"),
            format!("{loc}\\external_plugins\\{plugin_id}\\.claude-plugin\\plugin.json"),
        ] {
            if let Ok(c) = std::fs::read_to_string(&cand) {
                if let Ok(m) = parse_json_bom(&c) {
                    let latest = m.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    if !latest.is_empty() && latest != inst {
                        out.push(PluginUpdate {
                            id: id.clone(),
                            installed: inst.to_string(),
                            available: latest.to_string(),
                        });
                    }
                }
                break;
            }
        }
    }
    out
}

#[derive(Serialize)]
struct PluginContents {
    id: String,
    skills: Vec<String>,
    commands: Vec<String>,
    agents: Vec<String>,
}

/// Resolve the on-disk content directory of an installed plugin.
/// Prefers the reported installPath (github plugins live in the cache); falls back to the
/// marketplace source `<installLocation>\plugins\<plugin_id>` for directory-source marketplaces
/// (e.g. max-marketplace), whose cache installPath may not exist.
fn plugin_content_dir(id: &str, install_path: &str, markets: &serde_json::Value) -> Option<String> {
    if !install_path.is_empty() && std::path::Path::new(install_path).is_dir() {
        return Some(install_path.to_string());
    }
    let at = id.rfind('@')?;
    let plugin_id = &id[..at];
    let mp_name = &id[at + 1..];
    let loc = markets
        .get(mp_name)
        .and_then(|m| m.get("installLocation"))
        .and_then(|v| v.as_str())?;
    let cand = format!("{loc}\\plugins\\{plugin_id}");
    if std::path::Path::new(&cand).is_dir() {
        Some(cand)
    } else {
        None
    }
}

/// Collect `*.md` stems under a directory recursively (used for commands/agents).
/// Nested paths are joined with `:` to mirror Claude Code's namespaced naming.
fn collect_md_names(root: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("md") {
                if let Ok(rel) = p.strip_prefix(base) {
                    let name = rel
                        .with_extension("")
                        .to_string_lossy()
                        .replace(['\\', '/'], ":");
                    if !name.is_empty() {
                        out.push(name);
                    }
                }
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

/// Skill names under `<dir>/skills` (one subdir per skill, name from SKILL.md frontmatter
/// when present, else the directory name).
fn collect_skill_names(skills_root: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skills_root) {
        for e in entries.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = e.file_name().to_string_lossy().to_string();
            let name = std::fs::read_to_string(e.path().join("SKILL.md"))
                .ok()
                .and_then(|c| fm_value(&extract_frontmatter(&c), "name"))
                .unwrap_or(dir_name);
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Itemize the skills / commands / agents bundled inside each installed plugin.
/// Read-only filesystem scan; no network, no claude CLI spawn.
#[tauri::command]
fn list_plugin_contents() -> Vec<PluginContents> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let plugins_dir = format!("{home}\\.claude\\plugins");
    let installed = match std::fs::read_to_string(format!("{plugins_dir}\\installed_plugins.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
    {
        Some(v) => v,
        None => return Vec::new(),
    };
    let markets = std::fs::read_to_string(format!("{plugins_dir}\\known_marketplaces.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .unwrap_or(serde_json::Value::Null);

    let mut out: Vec<PluginContents> = Vec::new();
    let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) else {
        return out;
    };
    for (id, arr) in po {
        let install_path = arr
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e.get("installPath"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Some(dir) = plugin_content_dir(id, install_path, &markets) else {
            continue;
        };
        let base = std::path::Path::new(&dir);
        let skills = collect_skill_names(&base.join("skills"));
        let commands = collect_md_names(&base.join("commands"));
        let agents = collect_md_names(&base.join("agents"));
        if skills.is_empty() && commands.is_empty() && agents.is_empty() {
            continue;
        }
        out.push(PluginContents { id: id.clone(), skills, commands, agents });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// Manage one plugin: enable / disable / update (streamed via Manage-Plugin.ps1).
#[tauri::command]
async fn run_plugin(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    id: String,
) -> Result<i32, String> {
    if !matches!(action.as_str(), "enable" | "disable" | "update") {
        return Err(format!("неизвестное действие plugin: {action}"));
    }
    let script = abs(PLUGIN_MGR_SCRIPT_REL);
    spawn_streamed(
        app,
        state,
        "plugin-mgr".to_string(),
        script,
        vec!["-Action".into(), action, "-Id".into(), id],
    )
    .await
}

#[tauri::command]
fn read_config() -> HubConfig {
    read_config_file()
}

#[tauri::command]
fn write_config(config: HubConfig) -> Result<(), String> {
    let p = config_path().ok_or("APPDATA не найден")?;
    if let Some(dir) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&p, json).map_err(|e| format!("запись config: {e}"))?;
    Ok(())
}

/// Resolved paths for the About section.
#[tauri::command]
fn app_paths() -> serde_json::Value {
    serde_json::json!({
        "scriptsRoot": scripts_root(),
        "configPath": config_path(),
        "exe": std::env::current_exe().ok().map(|p| p.display().to_string()),
    })
}

/// A profile's settings.json `env` block as (key, value) pairs. Launching with these in the
/// real process environment (not just settings.json) is what lets Claude Code skip the
/// onboarding "Select login method" screen for a custom provider — its auth check reads the
/// process env, while settings.json `env` is only applied to outgoing requests.
fn profile_env_pairs(name: &str) -> Vec<(String, String)> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let path = format!("{home}\\.claude-{name}\\settings.json");
    let mut out = Vec::new();
    if let Ok(c) = std::fs::read_to_string(&path) {
        if let Ok(v) = parse_json_bom(&c) {
            if let Some(env) = v.get("env").and_then(|e| e.as_object()) {
                for (k, val) in env {
                    if let Some(s) = val.as_str() {
                        out.push((k.clone(), s.to_string()));
                    }
                }
            }
        }
    }
    out
}

#[derive(Serialize)]
struct ProfileLaunch {
    name: String,
    /// "full" (default) | "lean".
    mode: String,
    /// MCP server names to re-include when lean (subset of config\.mcp.json).
    mcp: Vec<String>,
    #[serde(rename = "claudeMd")]
    claude_md: bool,
    /// True when the profile has a token/API-key provider → lean uses `--bare`; else `--safe-mode`.
    #[serde(rename = "tokenAuth")]
    token_auth: bool,
}

#[derive(Serialize)]
struct LaunchConfigStatus {
    profiles: Vec<ProfileLaunch>,
    #[serde(rename = "availableMcp")]
    available_mcp: Vec<String>,
}

/// MCP server names declared in the canonical config\.mcp.json.
fn read_mcp_server_names() -> Vec<String> {
    let content = std::fs::read_to_string(abs(MCP_CONFIG_REL)).unwrap_or_default();
    parse_json_bom(&content)
        .ok()
        .and_then(|v| {
            v.get("mcpServers")
                .and_then(|m| m.as_object())
                .map(|o| o.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// One profile's launch config from profile-launch.json → (mode, mcp, claude_md).
fn read_profile_launch(name: &str) -> (String, Vec<String>, bool) {
    let content = std::fs::read_to_string(abs(LAUNCH_CONFIG_REL)).unwrap_or_default();
    if let Ok(v) = parse_json_bom(&content) {
        if let Some(p) = v.get("profiles").and_then(|p| p.get(name)) {
            let mode = p.get("mode").and_then(|x| x.as_str()).unwrap_or("full").to_string();
            let mcp = p
                .get("mcp")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let claude_md = p.get("claudeMd").and_then(|x| x.as_bool()).unwrap_or(false);
            return (mode, mcp, claude_md);
        }
    }
    ("full".into(), Vec::new(), false)
}

/// Does the profile use a token/API-key provider (so `--bare` works)? OAuth-only profiles don't
/// expose a token in env and would break under `--bare` (which never reads OAuth/keychain).
fn profile_uses_token_auth(name: &str) -> bool {
    profile_env_pairs(name)
        .iter()
        .any(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN" || k == "ANTHROPIC_API_KEY")
}

/// Write a temp MCP config holding only the selected servers (placeholders substituted), for
/// `--mcp-config`. Returns its path, or None when nothing usable was selected.
fn write_temp_mcp_config(name: &str, servers: &[String]) -> Option<String> {
    let home = std::env::var("USERPROFILE").ok()?;
    let src = std::fs::read_to_string(abs(MCP_CONFIG_REL)).ok()?;
    let src = src.replace("{{USERPROFILE_FWD}}", &home.replace('\\', "/"));
    let v = parse_json_bom(&src).ok()?;
    let all = v.get("mcpServers")?.as_object()?;
    let mut chosen = serde_json::Map::new();
    for s in servers {
        if let Some(entry) = all.get(s) {
            chosen.insert(s.clone(), entry.clone());
        }
    }
    if chosen.is_empty() {
        return None;
    }
    let out = serde_json::json!({ "mcpServers": chosen });
    let tmp = std::env::temp_dir().join(format!("claude-hub-mcp-{name}.json"));
    std::fs::write(&tmp, serde_json::to_string_pretty(&out).ok()?).ok()?;
    Some(tmp.to_string_lossy().to_string())
}

/// Base lean flag by auth type: `--bare` works only with token/API-key auth (it never reads
/// OAuth/keychain); OAuth profiles fall back to `--safe-mode`.
fn lean_base_flag(token_auth: bool) -> &'static str {
    if token_auth {
        "--bare"
    } else {
        "--safe-mode"
    }
}

/// Extra `claude` CLI flags for a profile's lean launch (shared by launch + measure).
/// `--bare` for token-auth profiles (skips plugins/hooks/LSP/auto-memory), else `--safe-mode`.
fn lean_flags(name: &str) -> Vec<String> {
    let (_, mcp, claude_md) = read_profile_launch(name);
    let token_auth = profile_uses_token_auth(name);
    let mut flags = vec![lean_base_flag(token_auth).to_string()];
    if token_auth {
        // --bare already loads no MCP. Only when specific servers are chosen do we add them via
        // --mcp-config (+ --strict-mcp-config so ONLY those load). NB: --strict-mcp-config WITHOUT
        // --mcp-config makes claude skip the request entirely (usage all zeros) — never emit it alone.
        if !mcp.is_empty() {
            if let Some(path) = write_temp_mcp_config(name, &mcp) {
                flags.push("--strict-mcp-config".into());
                flags.push("--mcp-config".into());
                flags.push(path);
            }
        }
        if claude_md {
            if let Ok(home) = std::env::var("USERPROFILE") {
                flags.push("--add-dir".into());
                flags.push(format!("{home}\\.claude-{name}"));
            }
        }
    }
    flags
}

/// Per-profile launch config + available MCP servers (for the tool-set UI). Read-only.
#[tauri::command]
fn read_launch_config() -> LaunchConfigStatus {
    let available_mcp = read_mcp_server_names();
    let profiles = profile_names()
        .iter()
        .map(|n| {
            let (mode, mcp, claude_md) = read_profile_launch(n);
            ProfileLaunch {
                name: n.clone(),
                mode,
                mcp,
                claude_md,
                token_auth: profile_uses_token_auth(n),
            }
        })
        .collect();
    LaunchConfigStatus { profiles, available_mcp }
}

/// Set a profile's launch config (mode + selected MCP + CLAUDE.md). Backup + UTF-8 no BOM.
#[tauri::command]
fn set_launch_config(
    name: String,
    mode: String,
    mcp: Vec<String>,
    claude_md: bool,
) -> Result<(), String> {
    if !valid_profile_name(&name) {
        return Err(format!("недопустимое имя профиля: {name}"));
    }
    if !matches!(mode.as_str(), "full" | "lean") {
        return Err(format!("неизвестный режим: {mode}"));
    }
    let path = abs(LAUNCH_CONFIG_REL);
    let mut v = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({ "schemaVersion": 1, "profiles": {} }));
    let obj = v.as_object_mut().unwrap();
    obj.entry("schemaVersion").or_insert(serde_json::json!(1));
    let profiles = obj
        .entry("profiles")
        .or_insert_with(|| serde_json::json!({}));
    if !profiles.is_object() {
        *profiles = serde_json::json!({});
    }
    profiles.as_object_mut().unwrap().insert(
        name,
        serde_json::json!({ "mode": mode, "mcp": mcp, "claudeMd": claude_md }),
    );
    if std::path::Path::new(&path).exists() {
        let _ = std::fs::copy(&path, format!("{path}.bak"));
    }
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write profile-launch.json: {e}"))?;
    Ok(())
}

/// Measure a profile's effective system-prompt size: run `claude [lean flags] -p ok
/// --output-format json` and return usage.input_tokens. Lean is fast; full hits the model
/// with the big prompt (slow on a local engine), so this is invoked on demand only.
#[tauri::command]
async fn measure_context(name: String, lean: bool) -> Result<i64, String> {
    if !valid_profile_name(&name) {
        return Err(format!("недопустимое имя профиля: {name}"));
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let dir = format!("{home}\\.claude-{name}");
    let mut argline: Vec<String> = vec!["/c".into(), "claude".into()];
    if lean {
        argline.extend(lean_flags(&name));
    }
    argline.extend([
        "-p".into(),
        "ok".into(),
        "--output-format".into(),
        "json".into(),
    ]);
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(&argline).env("CLAUDE_CONFIG_DIR", &dir);
    for (k, v) in profile_env_pairs(&name) {
        cmd.env(k, v);
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = tokio::time::timeout(std::time::Duration::from_secs(180), cmd.output())
        .await
        .map_err(|_| "измерение превысило 180с — модель не ответила".to_string())?
        .map_err(|e| format!("claude не запустился: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    // claude may print startup/log lines before the single JSON result (esp. with MCP servers),
    // so extract the outermost {...} rather than assuming the whole output is JSON.
    let raw = stdout.trim();
    let json_str = match (raw.find('{'), raw.rfind('}')) {
        (Some(a), Some(b)) if b > a => &raw[a..=b],
        _ => raw,
    };
    let v = parse_json_bom(json_str).map_err(|_| {
        format!(
            "не удалось разобрать ответ claude: {}",
            raw.chars().take(200).collect::<String>()
        )
    })?;
    v.get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|t| t.as_i64())
        .ok_or_else(|| "в ответе нет usage.input_tokens".to_string())
}

/// Launch a profile: open a console with CLAUDE_CONFIG_DIR set and `claude` running under it.
/// `mode` is accepted for API compatibility but only "terminal" is supported (the VS Code launch
/// was removed — `code` CLI can't reliably pass env to an already-running instance nor auto-open
/// a terminal). Honors the profile's saved launch config (full vs lean → lean CLI flags inline).
#[tauri::command]
fn launch_profile(name: String, mode: String) -> Result<(), String> {
    if !PROFILE_NAMES.contains(&name.as_str()) && !valid_profile_name(&name) {
        return Err(format!("недопустимый профиль: {name}"));
    }
    if mode != "terminal" {
        return Err(format!("неподдерживаемый режим запуска: {mode}"));
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let dir = format!("{home}\\.claude-{name}");
    let (launch_mode, _, _) = read_profile_launch(&name);
    let lean = launch_mode == "lean";
    // Lean mode → append the lean CLI flags (--bare/--safe-mode + selected MCP).
    let claude_cmd = if lean {
        format!("claude {}", lean_flags(&name).join(" "))
    } else {
        "claude".to_string()
    };
    // New console that starts claude with the profile's provider env + config dir set as REAL
    // environment variables (inherited by the window `start` spawns), rather than inlined into a
    // `set K=V&&` cmd string. This avoids any cmd-metacharacter handling on env values and is what
    // makes a custom-provider profile skip the login screen (claude's auth check reads the env).
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "start", &format!("Claude {name}"), "cmd", "/k", &claude_cmd])
        .env("CLAUDE_CONFIG_DIR", &dir);
    for (k, v) in profile_env_pairs(&name) {
        cmd.env(k, v);
    }
    cmd.creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("не удалось открыть терминал: {e}"))?;
    Ok(())
}

/// Open a terminal (cmd) at `path` — e.g. a repo dir, to resolve a conflict with Claude Code.
#[tauri::command]
fn open_terminal(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("каталог не найден: {path}"));
    }
    // Open the new console directly in `path` via current_dir, rather than inlining the path
    // into a `cmd /k cd /d {path}` string (which an attacker-named dir with cmd metacharacters
    // like `&` could break out of). `start` inherits this process's working directory.
    std::process::Command::new("cmd")
        .args(["/c", "start", "Repo", "cmd", "/k"])
        .current_dir(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("не удалось открыть терминал: {e}"))?;
    Ok(())
}

/// Open an arbitrary folder/file in Explorer.
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("не удалось открыть {path}: {e}"))?;
    Ok(())
}

const AUTOSTART_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const AUTOSTART_NAME: &str = "AgentHub";

/// Is the app registered to start with Windows (HKCU Run key)?
#[tauri::command]
fn get_autostart() -> bool {
    std::process::Command::new("reg")
        .args(["query", AUTOSTART_KEY, "/v", AUTOSTART_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Enable/disable start-with-Windows via the HKCU Run key (points at this exe).
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .display()
            .to_string();
        let out = std::process::Command::new("reg")
            .args(["add", AUTOSTART_KEY, "/v", AUTOSTART_NAME, "/t", "REG_SZ", "/d", &exe, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
    } else {
        let _ = std::process::Command::new("reg")
            .args(["delete", AUTOSTART_KEY, "/v", AUTOSTART_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    Ok(())
}

/// Open a profile's config dir (%USERPROFILE%\.claude-<name>) in Explorer.
#[tauri::command]
fn open_profile_dir(name: String) -> Result<(), String> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("недопустимое имя профиля".into());
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let path = format!("{home}\\.claude-{name}");
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("не удалось открыть {path}: {e}"))?;
    Ok(())
}

/// Kill the currently-running child process tree (Windows: taskkill /T /F).
#[tauri::command]
fn cancel_run(state: State<'_, RunState>) -> Result<(), String> {
    let pid = { *state.0.lock().unwrap_or_else(|e| e.into_inner()) };
    match pid {
        Some(p) if p != 0 => {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &p.to_string(), "/T", "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            Ok(())
        }
        _ => Err("Нет активного прогона".into()),
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Показать окно", true, None::<&str>)?;
    let check_all = MenuItem::with_id(app, "check_all", "Проверить всё", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &check_all, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("AgentHub")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal(app),
            "check_all" => {
                reveal(app);
                let _ = app.emit("tray-check-all", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn reveal(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(RunState::default())
        .invoke_handler(tauri::generate_handler![
            list_components,
            read_status,
            run_component,
            run_forks,
            list_backups,
            run_backup,
            read_profiles,
            run_profiles,
            read_profiles_config,
            run_profile_mgmt,
            open_profile_dir,
            launch_profile,
            read_launch_config,
            set_launch_config,
            measure_context,
            read_sync,
            run_sync,
            read_engines,
            update_engine,
            run_engine,
            run_router,
            run_connect_router,
            read_engine_models,
            read_providers,
            run_provider,
            read_mcp,
            run_mcp,
            list_plugins,
            list_skills,
            list_plugin_updates,
            list_plugin_contents,
            run_plugin,
            read_schedules,
            run_schedule,
            read_config,
            write_config,
            app_paths,
            open_path,
            open_terminal,
            get_autostart,
            set_autostart,
            cancel_run
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            // Start minimized to tray if configured.
            if read_config_file().start_hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button minimizes to tray instead of quitting.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_name_format() {
        assert!(is_snapshot_name("2026-06-12_100002"));
        assert!(!is_snapshot_name("weekly-2026-06-11.zip"));
        assert!(!is_snapshot_name("2026-6-12_100002"));
        assert!(!is_snapshot_name(".backup-state.json"));
        assert!(!is_snapshot_name("2026-06-12_10000")); // too short
    }

    #[test]
    fn forks_args_known_and_unknown() {
        assert_eq!(forks_action_args("check"), Some(vec!["-Unattended".to_string()]));
        let ff = forks_action_args("ff").unwrap();
        assert!(ff.contains(&"-FfMain".to_string()));
        assert!(ff.contains(&"-Yes".to_string())); // mutations must be unattended
        assert!(forks_action_args("bogus").is_none());
        // "plan" must be a dry-run — never mutating.
        let plan = forks_action_args("plan").unwrap();
        assert!(plan.contains(&"-DryRun".to_string()));
        assert!(!plan.contains(&"-Yes".to_string()));
    }

    #[test]
    fn json_bom_tolerant() {
        let v = parse_json_bom("\u{feff}{\"a\":1}").unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(parse_json_bom("{\"b\":2}").unwrap()["b"], 2);
        assert!(parse_json_bom("{not json").is_err());
    }

    #[test]
    fn forks_actions_all_unattended() {
        // Every fork action runs unattended (no interactive Read-Host hang).
        for a in ["check", "plan", "ff", "delete", "rebase", "normalize"] {
            let args = forks_action_args(a).unwrap();
            assert!(args.contains(&"-Unattended".to_string()), "{a} must be unattended");
        }
    }

    #[test]
    fn profile_name_validation() {
        assert!(valid_profile_name("ccmy"));
        assert!(valid_profile_name("cc6"));
        assert!(valid_profile_name("A_b-1"));
        assert!(!valid_profile_name("")); // empty
        assert!(!valid_profile_name("-bad")); // leading non-alnum
        assert!(!valid_profile_name("bad name")); // space
        assert!(!valid_profile_name("a/b")); // path sep
        assert!(!valid_profile_name("a\"b")); // quote
        assert!(!valid_profile_name(&"x".repeat(33))); // too long
    }

    #[test]
    fn schedule_action_whitelist() {
        for a in ["enable", "disable", "run", "create", "delete"] {
            assert!(valid_schedule_action(a), "{a} must be allowed");
        }
        assert!(!valid_schedule_action("")); // empty
        assert!(!valid_schedule_action("drop")); // unknown
        assert!(!valid_schedule_action("delete; rm -rf")); // injection-shaped
    }

    #[test]
    fn lean_base_flag_by_auth() {
        // Token/API-key profiles get the tiny --bare; OAuth profiles fall back to --safe-mode.
        assert_eq!(lean_base_flag(true), "--bare");
        assert_eq!(lean_base_flag(false), "--safe-mode");
    }

    #[test]
    fn plugin_contents_scan() {
        // Build a throwaway plugin tree and verify the scanners pick up nested items
        // and namespace nested paths with ':'.
        let root = std::env::temp_dir().join("cmh_plugin_contents_scan");
        let _ = std::fs::remove_dir_all(&root);
        let mk = |p: &std::path::Path, body: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        mk(&root.join("skills\\max-dedup\\SKILL.md"), "---\nname: max-dedup\n---\nx");
        mk(&root.join("skills\\plain\\SKILL.md"), "no frontmatter here");
        mk(&root.join("commands\\check.md"), "c");
        mk(&root.join("commands\\sub\\nested.md"), "c");
        mk(&root.join("agents\\dev-researcher.md"), "a");

        let skills = collect_skill_names(&root.join("skills"));
        assert!(skills.contains(&"max-dedup".to_string())); // from frontmatter
        assert!(skills.contains(&"plain".to_string())); // fallback to dir name

        let commands = collect_md_names(&root.join("commands"));
        assert!(commands.contains(&"check".to_string()));
        assert!(commands.contains(&"sub:nested".to_string())); // nested -> ':'

        let agents = collect_md_names(&root.join("agents"));
        assert_eq!(agents, vec!["dev-researcher".to_string()]);

        // Directory-source fallback: empty installPath resolves via marketplace installLocation.
        let markets = serde_json::json!({
            "mp": { "installLocation": root.to_string_lossy() }
        });
        // <installLocation>\plugins\<plugin_id> must exist for the fallback to resolve.
        let plug = root.join("plugins").join("toolkit");
        std::fs::create_dir_all(&plug).unwrap();
        let resolved = plugin_content_dir("toolkit@mp", "", &markets);
        assert_eq!(resolved, Some(plug.to_string_lossy().to_string()));
        // Missing dir + missing marketplace -> None.
        assert!(plugin_content_dir("ghost@nope", "", &markets).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }
}
