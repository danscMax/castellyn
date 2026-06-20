// Castellyn — Tauri backend.
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

mod i18n;
use i18n::{tr, trv, Lang};

/// Windows CREATE_NO_WINDOW — keep spawned console apps (pwsh/reg/taskkill) from flashing
/// a black console window in front of the GUI.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Current UI locale, mirrored from the frontend via `set_language` and read by `tr`/`trv`
/// when the backend produces user-facing text (command errors, run-log, tray).
// ponytail: one process-global lock; locale changes are rare so contention is a non-issue.
static CUR_LANG: std::sync::RwLock<Lang> = std::sync::RwLock::new(Lang::Ru);
fn cur_lang() -> Lang {
    *CUR_LANG.read().unwrap_or_else(|e| e.into_inner())
}
fn set_cur_lang(l: Lang) {
    *CUR_LANG.write().unwrap_or_else(|e| e.into_inner()) = l;
}

// Canonical manifest, embedded as a fallback. The live source of truth is the
// same file on disk (read at runtime by `manifest_text`) so the dashboard and
// the PowerShell tooling never desync.
const MANIFEST_FALLBACK: &str = include_str!("../../manifest/maintenance-manifest.json");

/// Persistent hub settings (%APPDATA%\castellyn\config.json).
#[derive(Serialize, Deserialize, Default, Clone)]
struct HubConfig {
    #[serde(rename = "scriptsRoot", default, skip_serializing_if = "Option::is_none")]
    scripts_root: Option<String>,
    #[serde(rename = "startHidden", default)]
    start_hidden: bool,
    // None = default (true): the ✕ button hides to tray. false = ✕ actually quits the app.
    #[serde(rename = "closeToTray", default, skip_serializing_if = "Option::is_none")]
    close_to_tray: Option<bool>,
    #[serde(rename = "fetchTimeoutSec", default, skip_serializing_if = "Option::is_none")]
    fetch_timeout_sec: Option<u32>,
    #[serde(rename = "ghTimeoutSec", default, skip_serializing_if = "Option::is_none")]
    gh_timeout_sec: Option<u32>,
    // OS-level accelerator (e.g. "CommandOrControl+Shift+H") that toggles the window. None/empty = off.
    #[serde(rename = "toggleHotkey", default, skip_serializing_if = "Option::is_none")]
    toggle_hotkey: Option<String>,
    // UI locale ("ru"/"en"/"zh") mirrored from the frontend so the backend (errors, log, tray) can
    // localize too. Owned by set_language; write_config preserves it (never clobbered by a settings save).
    #[serde(rename = "language", default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}

fn config_path() -> Option<String> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| format!("{a}\\castellyn\\config.json"))
}

/// Pre-Castellyn config location (the `agenthub` rename tier). Read as a fallback so a user's
/// saved scriptsRoot/timeouts survive the rename; the first write_config migrates it forward.
fn agenthub_config_path() -> Option<String> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| format!("{a}\\agenthub\\config.json"))
}

/// Oldest legacy config location (pre-AgentHub `claude-maintenance-hub`). Read-only fallback.
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
        .or_else(|| read_config_at(agenthub_config_path()))
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

/// Expand manifest path placeholders the same way the PowerShell executors do, so paths surfaced
/// to the UI match what actually runs. `{{SCRIPTS_ROOT}}` → scripts_root(), `{{USERPROFILE}}` → home.
fn expand_placeholders(s: &str) -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    s.replace("{{SCRIPTS_ROOT}}", &scripts_root()).replace("{{USERPROFILE}}", &home)
}

/// Read the canonical manifest from disk; fall back to the embedded copy if the
/// file is missing or unreadable (e.g. relocated exe without the repo).
fn manifest_text() -> String {
    let path = format!(
        "{}\\Castellyn\\manifest\\maintenance-manifest.json",
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

// fork-updater is now vendored under Castellyn\tools\; these are the pre-rename external
// locations, used as a fallback if the vendored copy is absent (e.g. a relocated exe).
const FORKS_SCRIPT_FALLBACK: &str = "fork-updater\\update-forks.ps1";
const FORKS_LASTJSON_FALLBACK: &str = "fork-updater\\fork-sync.last.json";

/// Resolve `rel` under scripts_root, preferring it but falling back to `fallback_rel`
/// when the primary file doesn't exist (vendored-first, external-second).
fn abs_with_fallback(rel: &str, fallback_rel: &str) -> String {
    let primary = abs(rel);
    if std::path::Path::new(&primary).exists() {
        return primary;
    }
    let fb = abs(fallback_rel);
    if std::path::Path::new(&fb).exists() {
        return fb;
    }
    primary
}

#[tauri::command]
fn list_components() -> Vec<Component> {
    raw_components()
        .into_iter()
        .map(|c| {
            let is_forks = c.id == "forks";
            Component {
                last_json: c.last_json_rel.as_deref().map(|rel| {
                    if is_forks {
                        abs_with_fallback(rel, FORKS_LASTJSON_FALLBACK)
                    } else {
                        abs(rel)
                    }
                }),
                id: c.id,
                name: c.name,
                group: c.group,
                supports_apply: c.supports_apply,
            }
        })
        .collect()
}

/// Parse JSON tolerating a leading UTF-8 BOM — PowerShell writes some configs (e.g.
/// .backup-state.json) with one, and serde_json rejects a BOM otherwise.
fn parse_json_bom(content: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(content.trim_start_matches('\u{feff}'))
}

/// Read+parse a JSON file, NotFound -> Ok(None). The single home for the
/// "read_to_string -> parse_json_bom -> None on missing" envelope (was copy-pasted 5×).
fn read_json_opt(
    path: impl AsRef<std::path::Path>,
    label: &str,
) -> Result<Option<serde_json::Value>, String> {
    match std::fs::read_to_string(path.as_ref()) {
        Ok(c) => parse_json_bom(&c)
            .map(Some)
            .map_err(|e| format!("parse {label}: {e}")),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {label}: {e}")),
    }
}

/// Read and parse a *.last.json status file. Returns null if it doesn't exist yet.
#[tauri::command]
fn read_status(path: String) -> Result<Option<serde_json::Value>, String> {
    read_json_opt(&path, &path)
}

// Tracks the PID of the currently-running child (Some(0) = reserved/starting).
#[derive(Default)]
struct RunState(Mutex<Option<u32>>);

// Per-repo fork runs (path -> pid). Lets each fork update run concurrently and independently,
// keyed by repo path, without the single RunState slot blocking the whole Forks tab.
#[derive(Default)]
struct ForkRuns(Mutex<std::collections::HashMap<String, u32>>);

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
    spawn_streamed_io(app, state, id, script, args, None).await
}

/// Like `spawn_streamed`, but optionally feeds `stdin_payload` to the script's STDIN. Secrets
/// (e.g. provider tokens) are passed this way so they never appear in the process command line —
/// on Windows any process can read another's argv via WMI / Get-CimInstance Win32_Process.
async fn spawn_streamed_io(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
    script: String,
    args: Vec<String>,
    stdin_payload: Option<String>,
) -> Result<i32, String> {
    // Run the PowerShell script through the generic program runner.
    let mut full = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script,
    ];
    full.extend(args);
    spawn_streamed_prog(app, state, id, "pwsh".to_string(), full, stdin_payload).await
}

/// Core single-slot streamed runner: run `program args`, stream stdout/stderr to the console log,
/// wait for exit. Optionally feeds `stdin_payload` (secrets go here, never argv). Exports the
/// resolved SCRIPTS_ROOT so a child's {{SCRIPTS_ROOT}} expansion matches the backend's.
async fn spawn_streamed_prog(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
    program: String,
    args: Vec<String>,
    stdin_payload: Option<String>,
) -> Result<i32, String> {
    // Reserve the single run slot (guard dropped before any await).
    {
        let mut g = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return Err("Уже идёт другой прогон — дождись завершения или отмени.".into());
        }
        *g = Some(0);
    }

    let mut cmd = Command::new(&program);
    for a in &args {
        cmd.arg(a);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    if stdin_payload.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    cmd.env("SCRIPTS_ROOT", scripts_root());
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            *state.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(format!("не удалось запустить {program}: {e}"));
        }
    };

    if let Some(pid) = child.id() {
        *state.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    }

    // Feed the secret to STDIN and close it so the script's [Console]::In.ReadToEnd() returns.
    if let Some(payload) = stdin_payload {
        if let Some(mut sin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = sin.write_all(payload.as_bytes()).await;
            let _ = sin.shutdown().await;
        }
    }

    let code = pump_and_wait(app, id, child, "run-log", "run-done").await;
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(code)
}

/// The single shared streaming path: pump a child's stdout/stderr to `log_event`
/// (component = `id`), wait for exit, then emit `done_event`. Used by both the single-slot
/// runner (spawn_streamed_io) and the concurrent per-repo fork runner — only slot/registry
/// bookkeeping differs between them.
async fn pump_and_wait(
    app: AppHandle,
    id: String,
    mut child: tokio::process::Child,
    log_event: &'static str,
    done_event: &'static str,
) -> i32 {
    let mut handles = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let (a, i) = (app.clone(), id.clone());
        handles.push(tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = a.emit(log_event, LogLine { component: i.clone(), stream: "out".into(), line });
            }
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let (a, i) = (app.clone(), id.clone());
        handles.push(tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = a.emit(log_event, LogLine { component: i.clone(), stream: "err".into(), line });
            }
        }));
    }
    let status = child.wait().await;
    for h in handles {
        let _ = h.await;
    }
    let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
    let _ = app.emit(done_event, RunDone { component: id, code });
    code
}

/// Run a NATIVE job under the single-slot RunState guard, mirroring `spawn_streamed`'s contract so a
/// command can drop its PowerShell layer without any frontend change: it emits `run-log` lines
/// (component = `id`) and a final `run-done`. The job runs on a blocking thread (file IO / ureq /
/// subprocess), receives `out`/`err` line emitters, and returns the exit code. Secrets are passed
/// into `job` as ordinary captured values (process memory) — no argv/STDIN dance needed natively.
async fn run_native_streamed<F>(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
    job: F,
) -> Result<i32, String>
where
    F: FnOnce(&dyn Fn(&str), &dyn Fn(&str)) -> i32 + Send + 'static,
{
    {
        let mut g = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return Err("Уже идёт другой прогон — дождись завершения или отмени.".into());
        }
        *g = Some(0);
    }
    let app_job = app.clone();
    let id_job = id.clone();
    let code = tokio::task::spawn_blocking(move || {
        let out = |line: &str| {
            let _ = app_job.emit(
                "run-log",
                LogLine { component: id_job.clone(), stream: "out".into(), line: line.to_string() },
            );
        };
        let err = |line: &str| {
            let _ = app_job.emit(
                "run-log",
                LogLine { component: id_job.clone(), stream: "err".into(), line: line.to_string() },
            );
        };
        job(&out, &err)
    })
    .await
    .unwrap_or(-1);
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        "sync-wip" => vec!["-SyncWipLocal", "-Yes", "-Unattended"],
        "normalize" => vec!["-NormalizeRemotes", "-Yes", "-Unattended"],
        _ => return None,
    };
    Some(v.into_iter().map(String::from).collect())
}

/// Per-repo status JSON path that a `-Single` run writes (next to the fork-sync script). Read back
/// after a per-repo run to merge just that repo's fresh state into the UI — no shared-file race.
fn fork_repo_out_file(path: &str) -> Option<String> {
    let comp = raw_components().into_iter().find(|c| c.id == "forks")?;
    let script = abs_with_fallback(&comp.script_rel, FORKS_SCRIPT_FALLBACK);
    let dir = std::path::Path::new(&script).parent()?.to_string_lossy().to_string();
    let safe: String =
        path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    Some(format!("{dir}\\fork-sync.{safe}.last.json"))
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
    let script = abs_with_fallback(&comp.script_rel, FORKS_SCRIPT_FALLBACK);
    spawn_streamed(app, state, "forks".to_string(), script, args).await
}

/// Run a Forks action scoped to ONE repo, concurrently and independently of other repos and of
/// the single-slot runner. Streams to `fork-log` / `fork-done` (component = repo path) so the UI
/// can show per-repo progress without blocking the whole tab. Rejects a second run on the same repo.
#[tauri::command]
async fn run_fork_repo(
    app: AppHandle,
    runs: State<'_, ForkRuns>,
    action: String,
    path: String,
) -> Result<i32, String> {
    let mut args =
        forks_action_args(&action).ok_or_else(|| format!("неизвестное действие forks: {action}"))?;
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("каталог репозитория не найден: {path}"));
    }
    let comp = raw_components()
        .into_iter()
        .find(|c| c.id == "forks")
        .ok_or("компонент forks не найден в манифесте")?;
    let script = abs_with_fallback(&comp.script_rel, FORKS_SCRIPT_FALLBACK);
    // Reserve this repo (reject a second concurrent run on the same one).
    {
        let mut m = runs.0.lock().unwrap_or_else(|e| e.into_inner());
        if m.contains_key(&path) {
            return Err("этот форк уже обновляется".into());
        }
        m.insert(path.clone(), 0);
    }
    // Strict single-repo run: only this repo is processed, and its result is written to a per-repo
    // JSON (not the shared fork-sync.last.json) — so concurrent repo runs never race the file.
    let out_file = fork_repo_out_file(&path).unwrap_or_default();
    // `args` (from forks_action_args) already carries -Unattended for every action — don't repeat it
    // here, or pwsh fails with "parameter 'Unattended' specified more than once".
    let mut full = vec!["-Single".to_string(), path.clone(), "-OutFile".to_string(), out_file];
    full.append(&mut args);
    let cfg = read_config_file();
    if let Some(t) = cfg.fetch_timeout_sec {
        full.push("-FetchTimeoutSec".into());
        full.push(t.to_string());
    }
    if let Some(t) = cfg.gh_timeout_sec {
        full.push("-GhTimeoutSec".into());
        full.push(t.to_string());
    }
    let mut cmd = Command::new("pwsh");
    cmd.arg("-NoProfile").arg("-ExecutionPolicy").arg("Bypass").arg("-File").arg(&script);
    for a in &full {
        cmd.arg(a);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.env("SCRIPTS_ROOT", scripts_root());
    cmd.creation_flags(CREATE_NO_WINDOW);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            runs.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
            return Err(format!("не удалось запустить pwsh: {e}"));
        }
    };
    if let Some(pid) = child.id() {
        runs.0.lock().unwrap_or_else(|e| e.into_inner()).insert(path.clone(), pid);
    }
    let code = pump_and_wait(app, path.clone(), child, "fork-log", "fork-done").await;
    runs.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
    Ok(code)
}

/// Cancel the in-flight fork run for `path` (kills its process tree). No-op if none is running.
#[tauri::command]
fn cancel_fork_repo(runs: State<'_, ForkRuns>, path: String) -> Result<(), String> {
    let pid = { runs.0.lock().unwrap_or_else(|e| e.into_inner()).get(&path).copied() };
    if let Some(p) = pid {
        if p != 0 {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &p.to_string(), "/T", "/F"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    }
    Ok(())
}

/// Read the single repo's fresh state from the per-repo JSON a `-Single` run wrote. The UI merges
/// this into its repo list after `fork-done`, so only that card updates (no full rescan, no race).
#[tauri::command]
fn read_fork_repo_status(path: String) -> Option<serde_json::Value> {
    let out_file = fork_repo_out_file(&path)?;
    let content = std::fs::read_to_string(&out_file).ok()?;
    let v = parse_json_bom(&content).ok()?;
    v.get("repos").and_then(|r| r.as_array()).and_then(|a| a.first()).cloned()
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
    keep_snapshots: Option<u32>,
) -> Result<i32, String> {
    let (script_rel, mut args): (&str, Vec<String>) = match action.as_str() {
        "backup" => {
            let mut a = vec!["-Force".to_string()];
            if let Some(k) = keep_snapshots {
                a.push("-KeepSnapshots".into());
                a.push(k.max(1).to_string());
            }
            (BACKUP_SCRIPT_REL, a)
        }
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
// Config-drift (FUN-7): shared-config FILE link health. links.last.json is written by
// Check-Integrity.ps1; Relink self-elevates; sync-now reuses the Backup mirror.
const RELINK_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Relink-SharedConfig.ps1";
const INTEGRITY_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Check-Integrity.ps1";
const CONFIG_DRIFT_JSON_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\links.last.json";

/// Read the cached profiles health snapshot (profiles.last.json). Null until first check.
#[tauri::command]
fn read_profiles() -> Result<Option<serde_json::Value>, String> {
    read_json_opt(abs(PROFILES_JSON_REL), "profiles.last.json")
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
            if !profile_names().iter().any(|x| x == &n) {
                return Err(format!("неизвестный профиль: {n}"));
            }
            (REPAIR_SCRIPT_REL, vec!["-Name".to_string(), n])
        }
        _ => return Err(format!("неизвестное действие profiles: {action}")),
    };
    let script = abs(script_rel);
    spawn_streamed(app, state, "profiles".to_string(), script, args).await
}

/// Read the cached shared-config link-drift snapshot (links.last.json from Check-Integrity.ps1).
/// Null until the first integrity check has run. Shape: {generatedAt, drifted, unlinked, ok, items}.
#[tauri::command]
fn read_config_drift() -> Result<Option<serde_json::Value>, String> {
    read_json_opt(abs(CONFIG_DRIFT_JSON_REL), "links.last.json")
}

/// Run a config-drift action: `check` (refresh links.last.json), `relink` (re-establish the shared
/// config-file symlinks; the script self-elevates via UAC and returns a real exit code), or
/// `sync-now` (Backup mirror live -> config). Uses the "sync" run slot so the existing outcome/
/// toast + run-done reload apply.
#[tauri::command]
async fn run_config_drift(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
) -> Result<i32, String> {
    let (script_rel, args): (&str, Vec<String>) = match action.as_str() {
        "check" => (INTEGRITY_SCRIPT_REL, Vec::new()),
        "relink" => (RELINK_SCRIPT_REL, vec!["-NonInteractive".to_string()]),
        "sync-now" => (BACKUP_SCRIPT_REL, vec!["-Force".to_string()]),
        _ => return Err(format!("неизвестное действие config-drift: {action}")),
    };
    let script = abs(script_rel);
    spawn_streamed(app, state, "sync".to_string(), script, args).await
}

const PROFILES_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\profiles.json";
const PROFILE_MGMT_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Manage-Profiles.ps1";

/// Read the canonical profile config (config\profiles.json): names, colours, descriptions,
/// and each profile's linkedFolders. Null until the file exists.
#[tauri::command]
fn read_profiles_config() -> Result<Option<serde_json::Value>, String> {
    read_json_opt(abs(PROFILES_CONFIG_REL), "profiles.json")
}

/// Profile name validation: `[A-Za-z0-9][A-Za-z0-9_-]{0,31}` — keeps the shell call safe
/// (no spaces, quotes, path separators) and mirrors Manage-Profiles.ps1.
fn valid_profile_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars().next().map(|c| c.is_ascii_alphanumeric()).unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Profile lifecycle: add / remove / rename / recolor / redescribe / set-links via Manage-Profiles.ps1.
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
        // Description may be empty (clearing) — pass whatever the dialog sent, as a separate argv.
        "redescribe" => {
            args.push("-Description".into());
            args.push(description.unwrap_or_default());
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

/// Repair ONE profile's shared-folder links with admin rights (folder symlinks need UAC).
/// Launches Repair-ProfileLinks.ps1 elevated via `Start-Process -Verb RunAs` and waits.
/// The elevated child's output isn't piped back (UAC severs inherited pipes); the repair
/// script refreshes profiles.last.json itself, so the UI reloads on `run-done`.
#[tauri::command]
async fn repair_profile_elevated(
    app: AppHandle,
    state: State<'_, RunState>,
    name: String,
) -> Result<i32, String> {
    // Charset-validate FIRST: `name` gets interpolated into an *elevated* PowerShell string
    // below, and profile_names() reads names verbatim from profiles.json (no charset check),
    // so a single quote there would be admin-level command injection. valid_profile_name()
    // (mirrors run_profile_mgmt) makes the "name is validated" guarantee real, not assumed.
    if !valid_profile_name(&name) {
        return Err(format!("недопустимое имя профиля: {name}"));
    }
    if !profile_names().iter().any(|x| x == &name) {
        return Err(format!("неизвестный профиль: {name}"));
    }
    let repair = abs(REPAIR_SCRIPT_REL);
    // name is validated ([A-Za-z0-9_-]); repair path carries no single quotes — safe in 'literals'.
    // NB: Start-Process does NOT quote -ArgumentList elements, so a `-File <path with spaces/!>`
    // silently breaks (elevated pwsh can't find the script) while `-Wait` swallows the child's
    // exit code → false success. Pass the script via `-Command "& '<path>' -Name '<n>'"` (single-
    // quoted path survives) and check the real ExitCode via -PassThru.
    let inner = format!(
        "Write-Host 'Запуск починки связей от администратора (подтвердите UAC)…'; \
         try {{ $p = Start-Process -FilePath pwsh -Verb RunAs -PassThru -Wait -ArgumentList \
         @('-NoProfile','-ExecutionPolicy','Bypass','-Command',\"& '{repair}' -Name '{name}'\") \
         -ErrorAction Stop; \
         if ($p.ExitCode -eq 0) {{ Write-Host 'Готово.' }} else {{ Write-Host ('Ошибка починки, код ' + $p.ExitCode); exit 1 }} }} \
         catch {{ Write-Host 'Повышение прав отменено или не удалось.'; exit 1 }}"
    );
    let args = vec!["-NoProfile".to_string(), "-Command".to_string(), inner];
    spawn_streamed_prog(app, state, "profiles".to_string(), "pwsh".to_string(), args, None).await
}

/// Relaunch the whole app elevated (so inline symlink creation works). Launches a new
/// elevated instance via `Start-Process -Verb RunAs`; on success quits this instance, on
/// UAC-decline leaves it running (so the user isn't left with nothing).
#[tauri::command]
fn relaunch_as_admin(app: AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_q = exe.display().to_string().replace('\'', "''");
    let inner = format!("try {{ Start-Process -FilePath '{exe_q}' -Verb RunAs -ErrorAction Stop }} catch {{ exit 1 }}");
    let status = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-Command", &inner])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("не удалось запустить pwsh: {e}"))?;
    if status.success() {
        app.exit(0);
        Ok(())
    } else {
        Err("Повышение прав отменено.".into())
    }
}

// --- Sync tab (native; was Manage-Sync.ps1) ---
const SYNC_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\sync-config.json";
const SYNC_CANON_STIGNORE_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\.stignore";

/// Item key -> .stignore whitelist line (order matters; mirrors Manage-Sync.ps1 $ItemLines).
fn sync_item_lines() -> [(&'static str, &'static str); 6] {
    [
        ("history", "!/history.jsonl"),
        ("projects", "!/projects"),
        ("skills", "!/skills"),
        ("agents", "!/agents"),
        ("commands", "!/commands"),
        ("keybindings", "!/keybindings.json"),
    ]
}

/// Absolute path of the live ~/.claude/.stignore.
fn live_stignore_path() -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    format!("{home}\\.claude\\.stignore")
}

/// Read config\sync-config.json → ordered (key, enabled); default all-on, `items.<k>` overrides.
fn read_sync_config() -> Vec<(String, bool)> {
    let mut items: Vec<(String, bool)> =
        sync_item_lines().iter().map(|(k, _)| (k.to_string(), true)).collect();
    if let Some(v) = std::fs::read_to_string(abs(SYNC_CONFIG_REL))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
    {
        if let Some(obj) = v.get("items").and_then(|x| x.as_object()) {
            for (k, on) in items.iter_mut() {
                if let Some(b) = obj.get(k.as_str()).and_then(|x| x.as_bool()) {
                    *on = b;
                }
            }
        }
    }
    items
}

/// Reproduce Build-Stignore: header + volatile rules + enabled whitelist + footer. LF, trailing \n.
fn build_stignore(items: &[(String, bool)]) -> String {
    let mut lines: Vec<String> = vec![
        "// =====================================================".into(),
        "// Syncthing ignore rules for ~/.claude  (generated by Castellyn)".into(),
        "// Whitelist below is driven by config\\sync-config.json (dashboard -> Синхронизация).".into(),
        "// First match wins; \"//\" starts a comment.".into(),
        "// =====================================================".into(),
        "".into(),
        "// --- Volatile only (never real content; cause sync conflicts) ---".into(),
        "**/.git/index.lock".into(),
        "*.sync-conflict-*".into(),
        "~syncthing~*".into(),
        ".stversions".into(),
        "".into(),
        "// --- Synced (whitelist; toggle via dashboard / sync-config.json) ---".into(),
    ];
    for (k, line) in sync_item_lines().iter() {
        if items.iter().any(|(ik, on)| ik == k && *on) {
            lines.push((*line).to_string());
        }
    }
    lines.extend([
        "".into(),
        "// --- Ignore everything else under ~/.claude ---".into(),
        "// settings*.json / .claude.json / .credentials.json = machine-local (secrets, CC-rewritten).".into(),
        "// plugins/ = re-fetched per machine from managed-settings marketplaces (not lost).".into(),
        "/*".into(),
    ]);
    lines.join("\n") + "\n"
}

/// Significant rule lines (non-comment, trimmed) for drift comparison — mirrors Get-RuleLines.
fn rule_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .split('\n')
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .collect()
}

/// Run `attrib <flags> <path>` quietly (no console flash). Best-effort.
fn run_attrib(flags: &[&str], path: &str) {
    let mut c = std::process::Command::new("attrib");
    c.args(flags).arg(path).creation_flags(CREATE_NO_WINDOW);
    let _ = c.status();
}

/// Write UTF-8 without BOM, tolerating a Hidden/ReadOnly target (clear attrs, write, restore Hidden).
fn write_file_no_bom(path: &str, content: &str) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    let meta = std::fs::metadata(path).ok();
    let was_hidden = meta
        .as_ref()
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
        .unwrap_or(false);
    let was_ro = meta.as_ref().map(|m| m.permissions().readonly()).unwrap_or(false);
    if was_hidden || was_ro {
        run_attrib(&["-h", "-r"], path);
    }
    std::fs::write(path, content)?;
    if was_hidden {
        run_attrib(&["+h"], path);
    }
    Ok(())
}

// --- Syncthing local REST (best-effort; mirrors Get-SyncthingStatus) ---
fn syncthing_api_key() -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let cfg = std::fs::read_to_string(format!("{local}\\Syncthing\\config.xml")).ok()?;
    let start = cfg.find("<apikey>")? + "<apikey>".len();
    let end = cfg[start..].find("</apikey>")? + start;
    let key = cfg[start..end].trim().to_string();
    (!key.is_empty()).then_some(key)
}

fn st_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_millis(1500)))
        .build()
        .into()
}

fn st_get(agent: &ureq::Agent, key: &str, path: &str) -> Option<serde_json::Value> {
    let url = format!("http://127.0.0.1:8384{path}");
    let mut resp = agent.get(&url).header("X-API-Key", key).call().ok()?;
    let s = resp.body_mut().read_to_string().ok()?;
    serde_json::from_str(&s).ok()
}

/// canonicalize + strip \\?\ prefix + lowercase + trim trailing slashes (for path equality).
fn normalize_path(p: &str) -> String {
    let c = std::fs::canonicalize(p)
        .map(|pb| pb.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.to_string());
    c.trim_start_matches("\\\\?\\")
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// Syncthing id of the folder whose path == ~/.claude (folder ids are per-machine).
fn st_claude_folder(agent: &ureq::Agent, key: &str) -> Option<serde_json::Value> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let claude = normalize_path(&format!("{home}\\.claude"));
    let folders = st_get(agent, key, "/rest/config/folders")?;
    folders.as_array()?.iter().find(|f| {
        f.get("path")
            .and_then(|p| p.as_str())
            .map(|p| normalize_path(p) == claude)
            .unwrap_or(false)
    }).cloned()
}

fn syncthing_status() -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert("available".into(), serde_json::json!(false));
    let Some(key) = syncthing_api_key() else {
        return serde_json::Value::Object(out);
    };
    let agent = st_agent();
    if st_get(&agent, &key, "/rest/system/ping").is_none() {
        return serde_json::Value::Object(out); // daemon not answering
    }
    out.insert("available".into(), serde_json::json!(true));
    if let Some(ver) = st_get(&agent, &key, "/rest/system/version")
        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
    {
        out.insert("version".into(), serde_json::json!(ver));
    }
    let Some(folder) = st_claude_folder(&agent, &key) else {
        out.insert("folderShared".into(), serde_json::json!(false));
        return serde_json::Value::Object(out); // ~/.claude not a Syncthing folder here
    };
    out.insert("folderShared".into(), serde_json::json!(true));
    if let Some(label) = folder.get("label").and_then(|x| x.as_str()) {
        out.insert("folderLabel".into(), serde_json::json!(label));
    }
    let fid = folder.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    out.insert("folderId".into(), serde_json::json!(fid));
    if let Some(st) = st_get(&agent, &key, &format!("/rest/db/status?folder={fid}")) {
        if let Some(state) = st.get("state").and_then(|x| x.as_str()) {
            out.insert("state".into(), serde_json::json!(state));
        }
        let g = st.get("globalBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let n = st.get("needBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.insert("globalBytes".into(), serde_json::json!(g as i64));
        out.insert("needBytes".into(), serde_json::json!(n as i64));
        let completion = if g > 0.0 { ((100.0 * (g - n) / g) * 10.0).round() / 10.0 } else { 100.0 };
        out.insert("completion".into(), serde_json::json!(completion));
    } else {
        out.insert("state".into(), serde_json::json!("unknown"));
    }
    if let Some(conns) = st_get(&agent, &key, "/rest/system/connections") {
        let connected = conns
            .get("connections")
            .and_then(|c| c.as_object())
            .map(|o| {
                o.values()
                    .filter(|d| d.get("connected").and_then(|x| x.as_bool()).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        out.insert("connectedDevices".into(), serde_json::json!(connected));
    }
    serde_json::Value::Object(out)
}

/// Ask Syncthing to rescan the ~/.claude folder so a fresh .stignore applies now (best-effort).
fn syncthing_rescan() {
    let Some(key) = syncthing_api_key() else { return };
    let agent = st_agent();
    if let Some(fid) = st_claude_folder(&agent, &key)
        .and_then(|f| f.get("id").and_then(|x| x.as_str()).map(String::from))
    {
        let url = format!("http://127.0.0.1:8384/rest/db/scan?folder={fid}");
        let _ = agent.post(&url).header("X-API-Key", &key).send_empty();
    }
}

/// Sync status (items whitelist + .stignore drift + Syncthing). Native; blocking → spawn_blocking.
#[tauri::command]
async fn read_sync() -> Result<Option<serde_json::Value>, String> {
    tokio::task::spawn_blocking(|| {
        let items = read_sync_config();
        let expected = build_stignore(&items);
        let live = std::fs::read_to_string(live_stignore_path());
        let stignore_exists = live.is_ok();
        let stignore_matches = live
            .as_ref()
            .map(|c| rule_lines(c) == rule_lines(&expected))
            .unwrap_or(false);
        let items_obj: serde_json::Map<String, serde_json::Value> =
            items.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect();
        // generatedAt must change per fetch so SyncTab re-seeds its selection from items.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        Some(serde_json::json!({
            "generatedAt": now.to_string(),
            "items": items_obj,
            "stignoreMatches": stignore_matches,
            "stignoreExists": stignore_exists,
            "syncthing": syncthing_status(),
        }))
    })
    .await
    .map_err(|e| format!("{e}"))
}

/// Write sync-config.json (backup first) + regenerate canonical & live .stignore + rescan.
fn sync_set(enabled: &[String]) -> Result<i32, String> {
    let items: Vec<(String, bool)> = sync_item_lines()
        .iter()
        .map(|(k, _)| (k.to_string(), enabled.iter().any(|e| e == k)))
        .collect();
    let content = build_stignore(&items);

    // Backup + write the source-of-truth config.
    let cfg = abs(SYNC_CONFIG_REL);
    if std::path::Path::new(&cfg).exists() {
        let _ = std::fs::copy(&cfg, format!("{cfg}.bak"));
    }
    let items_obj: serde_json::Map<String, serde_json::Value> =
        items.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect();
    let payload = serde_json::json!({
        "schemaVersion": 1,
        "_comment": "Что синхронизируется между машинами (Syncthing claude-config = ~/.claude). Менять через дашборд Castellyn.",
        "items": items_obj,
    });
    let cfg_json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    write_file_no_bom(&cfg, &cfg_json).map_err(|e| format!("write sync-config.json: {e}"))?;

    // Regenerate canonical (config\.stignore, backed up) + live (~/.claude/.stignore).
    let canon = abs(SYNC_CANON_STIGNORE_REL);
    if std::path::Path::new(&canon).exists() {
        let _ = std::fs::copy(&canon, format!("{canon}.bak"));
    }
    write_file_no_bom(&canon, &content).map_err(|e| format!("write config\\.stignore: {e}"))?;
    let live = live_stignore_path();
    if let Some(dir) = std::path::Path::new(&live).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    write_file_no_bom(&live, &content).map_err(|e| format!("write ~/.claude/.stignore: {e}"))?;

    syncthing_rescan();
    Ok(0)
}

/// Run a Sync-tab action: `query` (no-op; UI re-reads via read_sync) or `set` the whitelist.
#[tauri::command]
async fn run_sync(action: String, enabled: Option<Vec<String>>) -> Result<i32, String> {
    match action.as_str() {
        "query" => Ok(0),
        "set" => {
            let enabled = enabled.unwrap_or_default();
            tokio::task::spawn_blocking(move || sync_set(&enabled))
                .await
                .map_err(|e| format!("{e}"))?
        }
        _ => Err(format!("неизвестное действие sync: {action}")),
    }
}

// --- LLM provider per profile + local engine launcher ---
const ENGINES_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\engines.json";
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

/// Resolve a command on PATH to a Windows-LAUNCHABLE path (.exe / .cmd / .bat — what CreateProcess
/// and `std::process::Command` can run; Rust ≥1.77 launches .cmd/.bat via cmd.exe with safe argument
/// escaping). Skips extension-less and .ps1 shims (npm drops all three for the same tool). None if
/// not found. Used to spawn npm-installed CLIs (`claude`, `ccr`, `npm`) directly.
fn exe_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var("PATH").ok()?;
    let exts = [".exe", ".cmd", ".bat"];
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
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

const STACK_CONFIG_REL: &str = "llm-stack\\stack.json";
const STACK_START_REL: &str = "llm-stack\\start-stack.ps1";
const STACK_STOP_REL: &str = "llm-stack\\stop-stack.ps1";

#[derive(Serialize)]
struct StackService {
    id: String,
    name: String,
    group: String,
    port: u16,
    protocol: String,
    dashboard: String,
    dir: String,
    enabled: bool,
    running: bool,
}

/// LLM-stack services from `llm-stack\stack.json` (the single source of truth for the
/// gateway + backend forks) + live running status (port probe). Read-only. Empty if the
/// manifest is missing. `protocol`/`port`/`dashboard` come straight from the manifest —
/// nothing is hardcoded here.
#[tauri::command]
fn read_stack() -> Vec<StackService> {
    let content = match std::fs::read_to_string(abs(STACK_CONFIG_REL)) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let v = match parse_json_bom(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(arr) = v.get("services").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    let s = |e: &serde_json::Value, k: &str| {
        e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let mut svcs: Vec<StackService> = arr
        .iter()
        .map(|e| StackService {
            id: s(e, "id"),
            name: s(e, "name"),
            group: s(e, "group"),
            port: e.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16,
            protocol: s(e, "protocol"),
            dashboard: e.get("dashboard").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            dir: expand_placeholders(&s(e, "dir")),
            enabled: e.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true),
            running: false,
        })
        .collect();
    // Probe ports concurrently (each probe blocks up to 250ms) — same pattern as read_engines.
    let running: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = svcs
            .iter()
            .map(|svc| {
                let p = svc.port;
                scope.spawn(move || p != 0 && port_listening(p))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(false)).collect()
    });
    for (svc, r) in svcs.iter_mut().zip(running) {
        svc.running = r;
    }
    svcs
}

/// A stack service id is a manifest key, passed to PowerShell as a standalone argv element (no
/// shell), so this only needs to reject obviously malformed ids — keep it to the manifest's shape.
fn valid_stack_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Start or stop the LLM stack. With no `only`, acts on the whole stack (start `-Router` includes
/// the paid GLM router on :4000; stop `-All`). With `only=<service id>`, acts on that one service
/// via the launchers' `-Only` switch. Streamed via pwsh.
#[tauri::command]
async fn run_stack(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    only: Option<String>,
) -> Result<i32, String> {
    let only = only.filter(|s| !s.is_empty());
    if let Some(id) = &only {
        if !valid_stack_id(id) {
            return Err(format!("недопустимый id сервиса: {id}"));
        }
    }
    let (script, args) = match action.as_str() {
        "start" => {
            let script = abs(STACK_START_REL);
            let args = match &only {
                Some(id) => vec!["-Only".to_string(), id.clone()],
                None => vec!["-Router".to_string()],
            };
            (script, args)
        }
        "stop" => {
            let script = abs(STACK_STOP_REL);
            let args = match &only {
                Some(id) => vec!["-Only".to_string(), id.clone()],
                None => vec!["-All".to_string()],
            };
            (script, args)
        }
        _ => return Err(format!("неизвестное действие стека: {action}")),
    };
    spawn_streamed(app, state, "engine".to_string(), script, args).await
}

const STACK_PROCS_SCRIPT_REL: &str = "Castellyn\\tools\\stack\\Stack-Procs.ps1";

/// Listening-process info for one stack port: PID + uptime. Frontend joins this onto service cards
/// by port to show "PID 1234 · 2h" without an extra per-service probe.
#[derive(Serialize, Deserialize)]
struct StackProc {
    port: u16,
    pid: u32,
    #[serde(rename = "uptimeSec")]
    uptime_sec: u64,
}

/// PID + uptime for every currently-listening stack port (one process snapshot via pwsh). Ports
/// with no listener are omitted. Read-only; never touches the services. Empty on any failure.
#[tauri::command]
async fn read_stack_procs() -> Vec<StackProc> {
    let ports: Vec<u16> = read_stack().iter().filter(|s| s.port != 0).map(|s| s.port).collect();
    if ports.is_empty() {
        return Vec::new();
    }
    let port_args = ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    let script = abs(STACK_PROCS_SCRIPT_REL);
    let out = tokio::process::Command::new("pwsh")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script, "-Ports", &port_args])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_json_bom(stdout.trim())
        .ok()
        .and_then(|v| serde_json::from_value::<Vec<StackProc>>(v).ok())
        .unwrap_or_default()
}

// ---- freellmapi analytics (read-only via a node helper over its SQLite DB) ----

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AnalyticsTotals {
    total_requests: i64,
    success_rate: f64,
    total_input_tokens: i64,
    total_output_tokens: i64,
    avg_latency_ms: i64,
    estimated_cost_savings: f64,
    first_request_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsModel {
    platform: String,
    model_id: String,
    display_name: String,
    requests: i64,
    success_rate: f64,
    avg_latency_ms: i64,
    total_input_tokens: i64,
    total_output_tokens: i64,
    estimated_cost: f64,
}

/// One time-series bucket for a single model. `bucket` is a unix-epoch second floored to `step_sec`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsSeriesPoint {
    bucket: i64,
    platform: String,
    model_id: String,
    requests: i64,
    total_input_tokens: i64,
    total_output_tokens: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsHelperOut {
    totals: Option<AnalyticsTotals>,
    per_model: Option<Vec<AnalyticsModel>>,
    series: Option<Vec<AnalyticsSeriesPoint>>,
    step_sec: Option<i64>,
    error: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct FreellmapiAnalytics {
    /// False when the gateway DB / node / data is missing — UI shows an empty state.
    available: bool,
    totals: AnalyticsTotals,
    per_model: Vec<AnalyticsModel>,
    /// Per-model usage over time (sparkline source); bucket width is `step_sec`.
    series: Vec<AnalyticsSeriesPoint>,
    step_sec: i64,
}

/// Path to the freellmapi SQLite DB, from the `gateway` service `dir` in stack.json (placeholders
/// expanded). None if the manifest or the entry is missing.
fn gateway_db_path() -> Option<String> {
    let content = std::fs::read_to_string(abs(STACK_CONFIG_REL)).ok()?;
    let v = parse_json_bom(&content).ok()?;
    let dir = v
        .get("services")?
        .as_array()?
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some("gateway"))
        .and_then(|e| e.get("dir"))
        .and_then(|x| x.as_str())?;
    Some(format!("{}\\data\\freeapi.db", expand_placeholders(dir)))
}

/// freellmapi usage analytics for the last `range_hours`, read **read-only** (WAL-safe) by a node
/// helper over the gateway's own better-sqlite3 — never disturbs the live gateway. Returns an empty
/// (available=false) result when node, the helper, or the DB is missing.
#[tauri::command]
async fn read_freellmapi_analytics(range_hours: u32) -> FreellmapiAnalytics {
    let hours = if range_hours == 0 { 168 } else { range_hours };
    let db = match gateway_db_path() {
        Some(p) if std::path::Path::new(&p).exists() => p,
        _ => return FreellmapiAnalytics::default(),
    };
    let helper = abs("Castellyn\\tools\\analytics\\query.cjs");
    let out = tokio::process::Command::new("node")
        .args([&helper, &db, &hours.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    let Ok(out) = out else { return FreellmapiAnalytics::default() };
    if !out.status.success() {
        return FreellmapiAnalytics::default();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: Option<AnalyticsHelperOut> =
        parse_json_bom(stdout.trim()).ok().and_then(|v| serde_json::from_value(v).ok());
    match parsed {
        Some(p) if p.error.is_none() && p.totals.is_some() => FreellmapiAnalytics {
            available: true,
            totals: p.totals.unwrap_or_default(),
            per_model: p.per_model.unwrap_or_default(),
            series: p.series.unwrap_or_default(),
            step_sec: p.step_sec.unwrap_or(0),
        },
        _ => FreellmapiAnalytics::default(),
    }
}

/// Minimal HTTP/1.1 GET to 127.0.0.1:port+path over a plain socket (localhost, no TLS, no extra
/// crate). Returns true iff the status line reports a 2xx — a real "is it actually serving"
/// signal, beyond a bare port being open.
fn http_health_ok(port: u16, path: &str) -> bool {
    use std::io::{Read, Write};
    if port == 0 {
        return false;
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)) {
            Ok(s) => s,
            Err(_) => return false,
        };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(700)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(400)));
    let p = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    let req = format!(
        "GET {p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: Castellyn\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    // Status line looks like "HTTP/1.1 200 OK" — accept any 2xx.
    let head = String::from_utf8_lossy(&buf[..n]);
    head.starts_with("HTTP/1.") && head.split(' ').nth(1).map(|c| c.starts_with('2')).unwrap_or(false)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackHealth {
    id: String,
    name: String,
    group: String,
    port: u16,
    enabled: bool,
    /// TCP port accepts a connection.
    port_open: bool,
    /// HTTP health endpoint returned 2xx. None when the service has no `health` path (port-only).
    healthy: Option<bool>,
}

/// Real health of llm-stack services: a TCP port probe plus — when `health` is set in stack.json —
/// an HTTP GET to that path expecting 2xx. Concurrent, read-only. Powers the System Health card.
#[tauri::command]
fn read_stack_health() -> Vec<StackHealth> {
    let content = match std::fs::read_to_string(abs(STACK_CONFIG_REL)) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let v = match parse_json_bom(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(arr) = v.get("services").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    let s = |e: &serde_json::Value, k: &str| {
        e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    struct Row {
        id: String,
        name: String,
        group: String,
        port: u16,
        enabled: bool,
        health: String,
    }
    let rows: Vec<Row> = arr
        .iter()
        .map(|e| Row {
            id: s(e, "id"),
            name: s(e, "name"),
            group: s(e, "group"),
            port: e.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16,
            enabled: e.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true),
            health: s(e, "health"),
        })
        .collect();
    // Probe all services concurrently — bounded by the slowest single probe, like read_stack.
    let results: Vec<(bool, Option<bool>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = rows
            .iter()
            .map(|r| {
                let port = r.port;
                let health = r.health.clone();
                scope.spawn(move || {
                    let open = port_listening(port);
                    let healthy = if health.is_empty() {
                        None
                    } else {
                        Some(open && http_health_ok(port, &health))
                    };
                    (open, healthy)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or((false, None))).collect()
    });
    rows.into_iter()
        .zip(results)
        .map(|(r, (open, healthy))| StackHealth {
            id: r.id,
            name: r.name,
            group: r.group,
            port: r.port,
            enabled: r.enabled,
            port_open: open,
            healthy,
        })
        .collect()
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
/// Expand engines.json path placeholders (machine-independent config): {{SCRIPTS_ROOT}} and
/// {{USERPROFILE}}. A string without placeholders passes through unchanged.
fn expand_manifest(p: &str) -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    p.replace("{{SCRIPTS_ROOT}}", &scripts_root())
        .replace("{{USERPROFILE}}", &home)
}

struct EngineCfg {
    start: String,
    stop: String,
    command: String,
    port: u16,
}

/// Look up one engine's launch fields in config\engines.json by id.
fn load_engine_cfg(id: &str) -> Option<EngineCfg> {
    let content = std::fs::read_to_string(abs(ENGINES_CONFIG_REL)).ok()?;
    let v = parse_json_bom(&content).ok()?;
    let arr = v.get("engines")?.as_array()?;
    let e = arr
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id))?;
    let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    Some(EngineCfg {
        start: s("start"),
        stop: s("stop"),
        command: s("command"),
        port: e.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16,
    })
}

/// PIDs of processes LISTENING on a local TCP `port`, via `netstat -ano`. A listener is identified
/// by its foreign address being the wildcard `0.0.0.0:0` / `[::]:0` — locale-independent (we never
/// read the localized "LISTENING" state word). Empty on any error.
fn listeners_on_port(port: u16) -> Vec<u32> {
    if port == 0 {
        return Vec::new();
    }
    let out = std::process::Command::new("netstat")
        .args(["-ano"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let suffix = format!(":{port}");
    let mut pids = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // Proto  Local  Foreign  State  PID
        if cols.len() < 5 || !cols[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if !cols[1].ends_with(&suffix) {
            continue;
        }
        if cols[2] != "0.0.0.0:0" && cols[2] != "[::]:0" {
            continue;
        }
        if let Ok(pid) = cols[cols.len() - 1].parse::<u32>() {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Launch a long-running engine DETACHED in its own console window (fire-and-forget). We neither
/// wait for it nor capture its pipes — a server never exits and would otherwise hang the streamed
/// runner forever (locking the run slot). "Running" is derived from the port probe, not from the
/// launcher exiting. `cwd` sets the working directory when given. Dropping the Child does not kill
/// it on Windows, so the engine keeps running after we return.
fn spawn_engine_detached(program: &str, args: &[String], cwd: Option<&str>) -> Result<(), String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .env("SCRIPTS_ROOT", scripts_root())
        .creation_flags(CREATE_NEW_CONSOLE);
    if let Some(d) = cwd.filter(|d| !d.is_empty()) {
        cmd.current_dir(d);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("не удалось запустить {program}: {e}"))
}

/// Console feedback for a detached engine launch (the launch produced no streamed output here —
/// the engine's own window has the live logs). Mirrors the normal run-log/run-done so the UI
/// spinner clears cleanly.
fn emit_engine_started(app: &AppHandle) {
    let _ = app.emit(
        "run-log",
        LogLine {
            component: "engine".into(),
            stream: "out".into(),
            line: "Запрошен запуск в отдельном окне. Статус обновится по проверке порта.".into(),
        },
    );
    let _ = app.emit("run-done", RunDone { component: "engine".into(), code: 0 });
}

/// Start / stop a local LLM engine from config\engines.json (native; was Manage-Engine.ps1).
/// start: launch the engine's `start` shell command (or `command` file) DETACHED in its own
/// console, else status-only no-op. stop: run `stop` (streamed), else kill whatever listens on
/// its port.
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
    let Some(cfg) = load_engine_cfg(&id) else {
        return Err(format!("движок '{id}' не найден в engines.json"));
    };

    if action == "start" {
        // A server never exits, so launch it DETACHED in its own console and return immediately —
        // streaming-until-exit would hang the run slot forever. Status comes from the port probe.
        // Shell start command (ccr, llmstack): run via `cmd /c` in a fresh console.
        if !cfg.start.trim().is_empty() {
            let sh = expand_manifest(&cfg.start);
            spawn_engine_detached("cmd", &["/c".to_string(), sh], None)?;
            emit_engine_started(&app);
            return Ok(0);
        }
        // File-based engine: run the file directly in its own console (.py via python), cwd = its dir.
        if !cfg.command.trim().is_empty() {
            let cmd = expand_manifest(&cfg.command);
            let path = std::path::Path::new(&cmd);
            if !path.is_file() {
                return Err(format!("файл запуска не найден: {cmd}"));
            }
            let dir = path.parent().map(|p| p.display().to_string());
            if cmd.to_lowercase().ends_with(".py") {
                spawn_engine_detached("python", &[cmd.clone()], dir.as_deref())?;
            } else {
                spawn_engine_detached(&cmd, &[], dir.as_deref())?;
            }
            emit_engine_started(&app);
            return Ok(0);
        }
        // Status-only engine (no launch command) — nothing to start.
        return Ok(0);
    }

    // stop
    if !cfg.stop.trim().is_empty() {
        let sh = expand_manifest(&cfg.stop);
        return spawn_streamed_prog(app, state, "engine".into(), "cmd".into(), vec!["/c".into(), sh], None).await;
    }
    // Fallback: kill whatever listens on the engine's port.
    let pids = listeners_on_port(cfg.port);
    if pids.is_empty() {
        return Ok(0); // nobody listening — already stopped
    }
    let mut args: Vec<String> = vec!["/F".into()];
    for pid in pids {
        args.push("/PID".into());
        args.push(pid.to_string());
    }
    spawn_streamed_prog(app, state, "engine".into(), "taskkill".into(), args, None).await
}

/// Run a child process to completion, forwarding stdout/stderr to the UI log (indented, mirroring the
/// PS `| ForEach Write-Host "    $_"`). Returns the exit code (None if it failed to launch). Uses
/// `.output()` (deadlock-free); the npm/ccr commands here are non-interactive. Simple args only — no
/// JSON, so .cmd shims (npm.cmd / ccr.cmd) launch cleanly under Rust's escaping.
fn stream_output(
    prog: &std::path::Path,
    args: &[&str],
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> Option<i32> {
    match std::process::Command::new(prog).args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                out(&format!("    {line}"));
            }
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                err(&format!("    {line}"));
            }
            o.status.code()
        }
        Err(e) => {
            err(&format!("    не удалось запустить: {e}"));
            None
        }
    }
}

/// The config.json merge of Setup-Router `configure` (testable; explicit path). Writes
/// ~/.claude-code-router/config.json so ccr forwards Claude Code to `backend`/`model` under provider
/// `name`, preserving other providers. Returns the exit code; streams via out/err.
fn apply_router_config(
    cfg_path: &str,
    backend: &str,
    model: &str,
    name: &str,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    use serde_json::{json, Value};
    if backend.is_empty() {
        err("ОШИБКА: для configure нужен -Backend (URL движка).");
        return 1;
    }
    if model.is_empty() {
        err("ОШИБКА: для configure нужен -Model (например, из «Загрузить модели»).");
        return 1;
    }
    // ccr wants the full chat-completions URL.
    let mut api_base = backend.trim_end_matches('/').to_string();
    if !api_base.ends_with("/chat/completions") {
        api_base = format!("{api_base}/chat/completions");
    }
    // Load existing config (preserve other providers/keys) or start fresh.
    let mut cfg: Value = std::fs::read_to_string(cfg_path)
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| json!({}));
    let obj = cfg.as_object_mut().unwrap();
    if !obj.get("Providers").map(|p| p.is_array()).unwrap_or(false) {
        obj.insert("Providers".into(), json!([]));
    }
    out(&format!(
        "  Провайдер '{name}' -> {api_base}  (модель {model}); Router.default = {name},{model}"
    ));
    let provider =
        json!({ "name": name, "api_base_url": api_base, "api_key": "not-needed", "models": [model] });
    {
        let providers = obj.get_mut("Providers").unwrap().as_array_mut().unwrap();
        let mut found = false;
        for p in providers.iter_mut() {
            if p.get("name").and_then(|n| n.as_str()) == Some(name) {
                *p = provider.clone();
                found = true;
            }
        }
        if !found {
            providers.push(provider);
        }
    }
    if !obj.get("Router").map(|r| r.is_object()).unwrap_or(false) {
        obj.insert("Router".into(), json!({}));
    }
    obj.get_mut("Router")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("default".into(), json!(format!("{name},{model}")));

    // Backup then write (UTF-8 no BOM).
    if let Some(dir) = std::path::Path::new(cfg_path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::path::Path::new(cfg_path).exists() {
        let _ = std::fs::copy(cfg_path, format!("{cfg_path}.bak"));
    }
    let serialized = match serde_json::to_string_pretty(&cfg) {
        Ok(s) => s,
        Err(e) => {
            err(&format!("ОШИБКА сериализации config.json: {e}"));
            return 1;
        }
    };
    if let Err(e) = write_file_no_bom(cfg_path, &serialized) {
        err(&format!("ОШИБКА записи config.json: {e}"));
        return 1;
    }
    out("  config.json записан (бэкап .bak).");
    0
}

/// Native port of Setup-Router.ps1 (install | configure). `install` runs `npm install -g
/// @musistudio/claude-code-router`; `configure` rewrites ccr's config.json then `ccr restart`.
/// Returns the exit code; streams via out/err.
fn setup_router_native(
    action: &str,
    backend: &str,
    model: &str,
    name: &str,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let cfg_path = format!("{home}\\.claude-code-router\\config.json");
    out(&format!("=== Router (ccr): {action} ==="));

    if action == "install" {
        if exe_on_path("ccr").is_some() {
            out("  ccr уже установлен.");
            return 0;
        }
        let Some(npm) = exe_on_path("npm") else {
            err("  ОШИБКА: npm не найден на PATH (нужен Node.js).");
            return 1;
        };
        out("  npm install -g @musistudio/claude-code-router …");
        stream_output(&npm, &["install", "-g", "@musistudio/claude-code-router"], out, err);
        if exe_on_path("ccr").is_some() {
            out("  ccr установлен.");
            0
        } else {
            out("  Не удалось подтвердить установку ccr.");
            1
        }
    } else {
        let code = apply_router_config(&cfg_path, backend, model, name, out, err);
        if code != 0 {
            return code;
        }
        if let Some(ccr) = exe_on_path("ccr") {
            out("  ccr restart …");
            stream_output(&ccr, &["restart"], out, err);
        }
        out("  Готово. Навесь на профиль пресет «Claude Code Router» (http://127.0.0.1:3456).");
        0
    }
}

/// Native port of Connect-Router.ps1: turnkey configure+start ccr then bind a profile to it.
/// Reuses the native Setup-Router (`setup_router_native`) and Manage-Provider
/// (`manage_provider_native`) steps (DRY) — the only extra logic is starting ccr, waiting for
/// :3456, and reading ccr's APIKEY. Returns the exit code; streams via out/err.
fn connect_router_native(
    backend: &str,
    model: &str,
    profile: &str,
    name: &str,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    let ccr_base = "http://127.0.0.1:3456";
    out(&format!("=== Подключение через роутер: {name} → профиль {profile} ==="));

    // 1. Configure ccr for this backend/model (+ ccr restart inside Setup-Router).
    let code = setup_router_native("configure", backend, model, name, out, err);
    if code != 0 {
        err("Прервано: не удалось настроить ccr.");
        return code;
    }

    // 2. Ensure ccr is running and verify the port came up (non-fatal warning, mirrors the script).
    if let Some(ccr) = exe_on_path("ccr") {
        out("  ccr start …");
        stream_output(&ccr, &["start"], out, err);
        std::thread::sleep(std::time::Duration::from_secs(4));
        if port_listening(3456) {
            out("  ccr слушает :3456 ✓");
        } else {
            out("  [ВНИМАНИЕ] ccr не поднял порт :3456. Конфиг и привязка сделаны, но сервер не запущен.");
            out("            Попробуй обновить ccr (вкладка «Обновления») или запусти «ccr code» в терминале.");
        }
    }

    // 3. Read ccr's APIKEY (token the profile must send; empty when ccr is open on localhost).
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let cfg_path = format!("{home}\\.claude-code-router\\config.json");
    let token = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|v| v.get("APIKEY").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_default();

    // 4. Bind the profile to ccr (Anthropic endpoint). Empty token → Manage-Provider writes the
    //    dummy bearer (single source of that rule) so a bare `claude` skips the login screen.
    let token_opt = if token.is_empty() { None } else { Some(token.as_str()) };
    let code = manage_provider_native(
        profile, "set", ccr_base, false, token_opt, Some(model), None, out, err,
    );
    if code != 0 {
        err("Прервано: не удалось привязать профиль.");
        return code;
    }
    out(&format!(
        "Готово. Профиль '{profile}' → {ccr_base} (ccr) → {name} ({model}). Перезапусти профиль."
    ));
    0
}

/// Install or configure claude-code-router (ccr) (native; was Setup-Router.ps1).
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
    let backend = backend.unwrap_or_default();
    let model = model.unwrap_or_default();
    if action == "configure" && (backend.is_empty() || model.is_empty()) {
        return Err("для configure нужны backend и model".into());
    }
    // PS default provider name is 'lmstudio'.
    let name = name.filter(|s| !s.is_empty()).unwrap_or_else(|| "lmstudio".to_string());
    run_native_streamed(app, state, "engine".to_string(), move |out, err| {
        setup_router_native(&action, &backend, &model, &name, out, err)
    })
    .await
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
    // PS default provider name is 'lmstudio'.
    let name = name.filter(|s| !s.is_empty()).unwrap_or_else(|| "lmstudio".to_string());
    run_native_streamed(app, state, "provider".to_string(), move |out, err| {
        connect_router_native(&backend, &model, &profile, &name, out, err)
    })
    .await
}

/// Fetch model ids from an OpenAI-compatible engine (GET <baseUrl>/models). Empty on error.
#[tauri::command]
async fn read_engine_models(base_url: String) -> Vec<String> {
    // Native HTTP (was Get-EngineModels.ps1). Blocking ureq → spawn_blocking off the async runtime.
    tokio::task::spawn_blocking(move || fetch_engine_models(&base_url))
        .await
        .unwrap_or_default()
}

#[derive(Serialize)]
struct GithubRepo {
    owner: String,
    name: String,
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "isPrivate")]
    is_private: bool,
    #[serde(rename = "isFork")]
    is_fork: bool,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

/// All of the authenticated user's GitHub repos (incl. private), via `gh repo list`.
/// Lets the UI surface repos that aren't locally cloned. Empty if gh is missing or
/// unauthenticated; read-only (no network writes).
#[tauri::command]
async fn list_github_repos() -> Vec<GithubRepo> {
    let out = tokio::process::Command::new("gh")
        .args([
            "repo", "list", "--limit", "1000", "--json",
            "name,owner,nameWithOwner,isPrivate,isFork,url,updatedAt",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(stdout.trim()) else {
        return Vec::new();
    };
    arr.iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let b = |k: &str| r.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
            GithubRepo {
                owner: r
                    .get("owner")
                    .and_then(|o| o.get("login"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: s("name"),
                name_with_owner: s("nameWithOwner"),
                is_private: b("isPrivate"),
                is_fork: b("isFork"),
                url: s("url"),
                updated_at: s("updatedAt"),
            }
        })
        .collect()
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
                    // Prefer the current tier env vars; fall back to the legacy single-value keys
                    // so profiles bound by an older AgentHub still display their model.
                    let g_or = |new: &str, old: &str| {
                        let v = g(new);
                        if v.is_empty() { g(old) } else { v }
                    };
                    p.base_url = g("ANTHROPIC_BASE_URL");
                    p.model = g_or("ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_MODEL");
                    p.small_model = g_or("ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_SMALL_FAST_MODEL");
                    p.has_token = !g("ANTHROPIC_AUTH_TOKEN").is_empty();
                }
            }
        }
        out.push(p);
    }
    out
}

/// Provider env keys written to a profile's settings.json. The last two are legacy single-value
/// keys, kept here only so `clear` (and the set-migration) scrub them too.
const PROVIDER_ENV_KEYS: [&str; 7] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

/// Best-effort check: were any of a profile dir's hot session paths written within `recent_secs`?
/// A running `claude` constantly rewrites .claude.json / sessions / shell-snapshots, so fresh
/// activity ≈ an open session. Pure (takes the dir) so it is unit-testable.
fn dir_recently_written(base: &std::path::Path, recent_secs: u64) -> bool {
    let now = std::time::SystemTime::now();
    [".claude.json", "sessions", "shell-snapshots", "session-env", "todos", "projects"]
        .iter()
        .any(|p| {
            std::fs::metadata(base.join(p))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .map(|age| age.as_secs() <= recent_secs)
                .unwrap_or(false)
        })
}

/// Guard for the cc3-class footgun: rebinding a profile whose `claude` session is live rewrites a
/// settings.json the running session reads, which can break it (the cc3→ccr 429 incident).
/// ponytail: mtime heuristic, not a lock — may briefly false-positive just after a session closes,
/// or when Syncthing just pulled the profile from the other machine (itself a "don't rebind now"
/// case). Upgrade to real process-env inspection only if the false positives ever annoy.
fn profile_session_active(name: &str) -> bool {
    let Ok(home) = std::env::var("USERPROFILE") else { return false };
    dir_recently_written(&std::path::Path::new(&home).join(format!(".claude-{name}")), 120)
}

/// Native port of Manage-Provider.ps1: merge the provider env block of ONE profile's
/// `~/.claude-<name>/settings.json` (preserving every other setting). `model`/`small_model`:
/// `None` = leave untouched, `Some("")` = remove the override, `Some(v)` = set it. Token: `keep_token`
/// keeps the existing bearer; otherwise a non-empty `token` is written, an empty one falls back to the
/// dummy bearer (so a bare `claude` skips the login screen). Returns the exit code; streams via out/err.
#[allow(clippy::too_many_arguments)]
fn manage_provider_native(
    name: &str,
    action: &str,
    base_url: &str,
    keep_token: bool,
    token: Option<&str>,
    model: Option<&str>,
    small_model: Option<&str>,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    // Validate against the canonical profile list (mirrors the script's Get-ClaudeProfiles check).
    let known = profile_names();
    if !known.iter().any(|n| n == name) {
        err(&format!("ОШИБКА: профиль '{name}' не найден ({}).", known.join(", ")));
        return 1;
    }
    // cc3-class guard: never rewrite a settings.json a live session is reading (see
    // profile_session_active). All provider-bind paths funnel here, so one check covers them all.
    if profile_session_active(name) {
        err(&format!(
            "⚠️ Профиль '{name}' похоже сейчас запущен (недавняя активность сессии). Закрой его \
             сессию Claude и повтори — смена привязки провайдера у живой сессии может её сломать \
             (как было с cc3). Если профиль не запущен, подожди ~2 минуты и повтори."
        ));
        return 1;
    }
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => {
            err("USERPROFILE не задан");
            return 1;
        }
    };
    let settings_path = format!("{home}\\.claude-{name}\\settings.json");
    apply_provider_env(
        &settings_path,
        name,
        action,
        base_url,
        keep_token,
        token,
        model,
        small_model,
        out,
        err,
    )
}

/// The settings.json merge of `manage_provider_native`, taking an explicit path (testable; no
/// USERPROFILE/profile-list coupling). See `manage_provider_native` for the parameter semantics.
#[allow(clippy::too_many_arguments)]
fn apply_provider_env(
    settings_path: &str,
    name: &str,
    action: &str,
    base_url: &str,
    keep_token: bool,
    token: Option<&str>,
    model: Option<&str>,
    small_model: Option<&str>,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    use serde_json::{json, Value};
    let mut settings: Value = match std::fs::read_to_string(settings_path) {
        Ok(ref c) if !c.trim().is_empty() => match parse_json_bom(c) {
            Ok(v) => v,
            Err(e) => {
                err(&format!("ОШИБКА: не удалось прочитать settings.json ({e})."));
                return 1;
            }
        },
        _ => json!({}),
    };
    if !settings.is_object() {
        settings = json!({});
    }
    let sobj = settings.as_object_mut().unwrap();
    if !sobj.get("env").map(|e| e.is_object()).unwrap_or(false) {
        sobj.insert("env".into(), json!({}));
    }

    out(&format!("=== Provider: {action} {name} ==="));

    let env_empty = {
        let env = sobj.get_mut("env").unwrap().as_object_mut().unwrap();
        if action == "set" {
            env.insert("ANTHROPIC_BASE_URL".into(), json!(base_url));
            // Token: keep, set the supplied one, or write a dummy for a keyless endpoint.
            if keep_token {
                // leave ANTHROPIC_AUTH_TOKEN as-is
            } else if let Some(t) = token.filter(|s| !s.is_empty()) {
                env.insert("ANTHROPIC_AUTH_TOKEN".into(), json!(t));
            } else {
                env.insert("ANTHROPIC_AUTH_TOKEN".into(), json!("agenthub-local"));
            }
            // Legacy single-value keys are always scrubbed on set (tier vars are the source of truth).
            env.remove("ANTHROPIC_MODEL");
            env.remove("ANTHROPIC_SMALL_FAST_MODEL");
            if let Some(m) = model {
                if m.is_empty() {
                    env.remove("ANTHROPIC_DEFAULT_SONNET_MODEL");
                    env.remove("ANTHROPIC_DEFAULT_OPUS_MODEL");
                } else {
                    env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), json!(m));
                    env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), json!(m));
                }
            }
            if let Some(sm) = small_model {
                if sm.is_empty() {
                    env.remove("ANTHROPIC_DEFAULT_HAIKU_MODEL");
                } else {
                    env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), json!(sm));
                }
            }
            let token_shown = if keep_token {
                "(без изменений)"
            } else if token.filter(|s| !s.is_empty()).is_some() {
                "(задан)"
            } else {
                "(dummy: agenthub-local)"
            };
            out(&format!(
                "  BaseUrl={base_url}  Model={}  SmallModel={}  Token={token_shown}",
                model.filter(|s| !s.is_empty()).unwrap_or("—"),
                small_model.filter(|s| !s.is_empty()).unwrap_or("—")
            ));
        } else {
            for k in PROVIDER_ENV_KEYS {
                env.remove(k);
            }
            out("  Провайдер сброшен на стандартный Anthropic-логин.");
        }
        env.is_empty()
    };
    if env_empty {
        sobj.remove("env");
    }

    // Backup then write (UTF-8 no BOM).
    if let Some(dir) = std::path::Path::new(settings_path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::path::Path::new(settings_path).exists() {
        let _ = std::fs::copy(settings_path, format!("{settings_path}.bak"));
    }
    let serialized = match serde_json::to_string_pretty(&settings) {
        Ok(s) => s,
        Err(e) => {
            err(&format!("ОШИБКА сериализации settings.json: {e}"));
            return 1;
        }
    };
    if let Err(e) = write_file_no_bom(settings_path, &serialized) {
        err(&format!("ОШИБКА записи settings.json: {e}"));
        return 1;
    }
    out(&format!(
        "  settings.json обновлён (бэкап .bak). Перезапустите профиль '{name}', чтобы провайдер применился."
    ));
    0
}

/// Bind (set) or unbind (clear) a profile's provider (native; was Manage-Provider.ps1).
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
    let base_url = base_url.unwrap_or_default();
    if action == "set" && base_url.is_empty() {
        return Err("для set нужен baseUrl".into());
    }
    let keep_token = keep_token.unwrap_or(false);
    // On set the dialog always supplies Model/SmallModel (empty removes the override) — bind them;
    // clear ignores them.
    let model = model.unwrap_or_default();
    let small_model = small_model.unwrap_or_default();
    let token = token.unwrap_or_default();
    run_native_streamed(app, state, "provider".to_string(), move |out, err| {
        let (model_arg, small_arg) = if action == "set" {
            (Some(model.as_str()), Some(small_model.as_str()))
        } else {
            (None, None)
        };
        let token_arg = if keep_token { None } else { Some(token.as_str()) };
        manage_provider_native(
            &name, &action, &base_url, keep_token, token_arg, model_arg, small_arg, out, err,
        )
    })
    .await
}

// --- Custom provider registry (config\myproviders.json + Windows Credential Manager) ---
// A user-owned list of external LLM providers (DeepSeek, Minimax, any OpenAI/Anthropic-compatible
// endpoint). Metadata lives in myproviders.json; the API key lives ONLY in the Credential Manager
// (mirrors the user's Mediafarm api_profiles split — never plaintext in JSON).
const MYPROVIDERS_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\myproviders.json";
/// Credential Manager service names. One entry per provider key (`provider:<id>`) + a single
/// freellmapi dashboard-session token (`dashboard`) used by the "connect via freellmapi" path.
const KR_PROVIDERS: &str = "castellyn.providers";
const KR_FREELLMAPI: &str = "castellyn.freellmapi";

/// Pre-Castellyn keyring service for a current one: `castellyn.X` → `agenthub.X`. Used only for
/// one-time lazy migration of secrets stored under the old brand. None for non-castellyn services.
fn legacy_kr_service(service: &str) -> Option<String> {
    service.strip_prefix("castellyn.").map(|s| format!("agenthub.{s}"))
}

fn kr_get(service: &str, user: &str) -> Option<String> {
    if let Some(v) = keyring::Entry::new(service, user).ok().and_then(|e| e.get_password().ok()) {
        return Some(v);
    }
    // Lazy migration: a secret stored under the old `agenthub.*` service is re-homed under the new
    // name and returned, so the rename never loses stored API keys / dashboard tokens. No recursion
    // (kr_set hits keyring::Entry directly).
    let old = legacy_kr_service(service)?;
    let v = keyring::Entry::new(&old, user).ok()?.get_password().ok()?;
    let _ = kr_set(service, user, &v);
    Some(v)
}
fn kr_set(service: &str, user: &str, secret: &str) -> Result<(), String> {
    keyring::Entry::new(service, user)
        .map_err(|e| format!("credential store: {e}"))?
        .set_password(secret)
        .map_err(|e| format!("save credential: {e}"))
}
fn kr_delete(service: &str, user: &str) {
    if let Ok(e) = keyring::Entry::new(service, user) {
        let _ = e.delete_credential();
    }
    // Also clear any pre-rename copy so deleting a key never leaves an orphaned `agenthub.*` secret.
    if let Some(old) = legacy_kr_service(service) {
        if let Ok(e) = keyring::Entry::new(&old, user) {
            let _ = e.delete_credential();
        }
    }
}

/// Key-pool metadata from a provider's JSON entry: (keyCount, activeKey). A provider may hold
/// several interchangeable keys (e.g. multiple aerolink keys); `activeKey` selects which one is
/// written to the harness on connect. keyCount==0 means the legacy single-key layout (`provider:<id>`).
fn key_pool_meta(e: &serde_json::Value) -> (u64, u64) {
    let count = e.get("keyCount").and_then(|x| x.as_u64()).unwrap_or(0);
    let active = e.get("activeKey").and_then(|x| x.as_u64()).unwrap_or(0);
    (count, active)
}

/// Next index when rotating a pool of `count` keys (wraps around). count<=1 → 0 (no-op rotation).
fn next_key_index(active: u64, count: u64) -> u64 {
    if count <= 1 { 0 } else { (active + 1) % count }
}

/// The currently active API key for a provider: pool slot `provider:<id>:<active>` when a pool
/// exists, otherwise the legacy single entry `provider:<id>`. Read-only.
fn active_provider_key(id: &str, e: &serde_json::Value) -> Option<String> {
    let (count, active) = key_pool_meta(e);
    if count > 0 {
        let idx = if active < count { active } else { 0 };
        kr_get(KR_PROVIDERS, &format!("provider:{id}:{idx}"))
    } else {
        kr_get(KR_PROVIDERS, &format!("provider:{id}"))
    }
}

/// Provider display name: non-empty, bounded, no control chars (it's a label, not a shell arg —
/// the shell-safe identifier is the generated `id`).
fn valid_provider_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.len() <= 64 && !s.chars().any(|c| c.is_control())
}

/// SSRF guard for a provider base URL (ports the intent of Mediafarm's validate_base_url):
/// require http/https and reject link-local + known cloud-metadata pivots. Localhost / RFC1918
/// are allowed on purpose (local engines like LM Studio). Run before storing a key and before connect.
fn valid_base_url(s: &str) -> Result<(), String> {
    let s = s.trim();
    let rest = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .ok_or("URL должен начинаться с http:// или https://")?;
    let host_port = rest.split('/').next().unwrap_or("");
    // strip an optional :port; handle an IPv6 literal ([::1] / [::1]:port) without mistaking its
    // inner colons for the port separator.
    let host = if host_port.starts_with('[') {
        host_port.trim_start_matches('[').split(']').next().unwrap_or("")
    } else {
        host_port.rsplit_once(':').map(|(h, _)| h).unwrap_or(host_port)
    };
    if host.is_empty() {
        return Err("пустой хост в URL".into());
    }
    let hl = host.to_ascii_lowercase();
    let blocked = ["169.254.169.254", "100.100.100.200", "fd00:ec2::254", "metadata.google.internal"];
    if blocked.contains(&hl.as_str()) || hl.starts_with("169.254.") || hl == "metadata" {
        return Err(format!("адрес заблокирован (SSRF/cloud-metadata): {host}"));
    }
    Ok(())
}

fn now_unix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}
fn gen_provider_id() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:012x}", (n as u64) & 0xffff_ffff_ffff)
}

#[derive(Serialize)]
struct MyProvider {
    id: String,
    name: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    protocol: String,
    #[serde(rename = "authScheme")]
    auth_scheme: String,
    model: String,
    #[serde(rename = "smallModel")]
    small_model: String,
    #[serde(rename = "connectVia")]
    connect_via: String,
    #[serde(rename = "targetProfile")]
    target_profile: String,
    /// Optional balance/credits endpoint (full URL) queried with the provider's key (#B4).
    #[serde(rename = "balanceUrl")]
    balance_url: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    /// Computed (never persisted): does a Credential Manager entry exist for this provider?
    #[serde(rename = "hasKey")]
    has_key: bool,
    /// Number of keys in the rotation pool (0 = legacy single key in `provider:<id>`).
    #[serde(rename = "keyCount")]
    key_count: u64,
    /// Index of the active key within the pool (which one connect writes to the harness).
    #[serde(rename = "activeKey")]
    active_key: u64,
}

#[derive(Deserialize)]
struct MyProviderInput {
    #[serde(default)]
    id: Option<String>, // empty/None = create a new record
    name: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    protocol: String,
    #[serde(rename = "authScheme", default)]
    auth_scheme: String,
    #[serde(default)]
    model: String,
    #[serde(rename = "smallModel", default)]
    small_model: String,
    #[serde(rename = "connectVia")]
    connect_via: String,
    #[serde(rename = "targetProfile", default)]
    target_profile: String,
    #[serde(rename = "balanceUrl", default)]
    balance_url: String,
}

fn read_myproviders_raw() -> Vec<serde_json::Value> {
    std::fs::read_to_string(abs(MYPROVIDERS_CONFIG_REL))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|v| v.get("providers").and_then(|p| p.as_array()).cloned())
        .unwrap_or_default()
}

fn write_myproviders_raw(list: &[serde_json::Value]) -> Result<(), String> {
    let path = abs(MYPROVIDERS_CONFIG_REL);
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::path::Path::new(&path).exists() {
        let _ = std::fs::copy(&path, format!("{path}.bak"));
    }
    let v = serde_json::json!({ "schemaVersion": 1, "providers": list });
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write myproviders.json: {e}"))
}

fn myprovider_from_entry(e: &serde_json::Value) -> MyProvider {
    let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let id = s("id");
    let (key_count, active_key) = key_pool_meta(e);
    // A pool (keyCount>0) is authoritative; otherwise fall back to the legacy single entry.
    let has_key = if key_count > 0 {
        true
    } else {
        kr_get(KR_PROVIDERS, &format!("provider:{id}")).is_some()
    };
    MyProvider {
        name: s("name"),
        base_url: s("baseUrl"),
        protocol: s("protocol"),
        auth_scheme: s("authScheme"),
        model: s("model"),
        small_model: s("smallModel"),
        connect_via: s("connectVia"),
        target_profile: s("targetProfile"),
        balance_url: s("balanceUrl"),
        created_at: s("createdAt"),
        has_key,
        key_count,
        active_key,
        id,
    }
}

/// List the user's custom providers (metadata + computed hasKey). Read-only.
#[tauri::command]
fn list_my_providers() -> Vec<MyProvider> {
    read_myproviders_raw().iter().map(myprovider_from_entry).collect()
}

/// Upsert a provider record. `api_key` arrives over the (local) Tauri IPC channel — not argv —
/// and is written to the Credential Manager; an empty/None key keeps any existing one.
#[tauri::command]
fn save_my_provider(p: MyProviderInput, api_key: Option<String>) -> Result<MyProvider, String> {
    if !valid_provider_name(&p.name) {
        return Err("недопустимое имя провайдера (1–64 символа, без управляющих)".into());
    }
    valid_base_url(&p.base_url)?;
    if !matches!(p.protocol.as_str(), "anthropic" | "openai") {
        return Err("protocol должен быть anthropic или openai".into());
    }
    if !matches!(p.connect_via.as_str(), "freellmapi" | "direct") {
        return Err("connectVia должен быть freellmapi или direct".into());
    }
    let id = p.id.clone().filter(|s| !s.is_empty()).unwrap_or_else(gen_provider_id);
    let auth = if !p.auth_scheme.is_empty() {
        p.auth_scheme.clone()
    } else if p.protocol == "anthropic" {
        "x-api-key".to_string()
    } else {
        "bearer".to_string()
    };
    let mut list = read_myproviders_raw();
    let prev = list
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()));
    let created = prev
        .and_then(|e| e.get("createdAt").and_then(|x| x.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(now_unix);
    // Carry the key-pool metadata across edits — the main dialog never reshapes the pool.
    let (key_count, active_key) = prev.map(key_pool_meta).unwrap_or((0, 0));
    let entry = serde_json::json!({
        "id": id,
        "name": p.name.trim(),
        "baseUrl": p.base_url.trim(),
        "protocol": p.protocol,
        "authScheme": auth,
        "model": p.model.trim(),
        "smallModel": p.small_model.trim(),
        "connectVia": p.connect_via,
        "targetProfile": p.target_profile.trim(),
        "balanceUrl": p.balance_url.trim(),
        "createdAt": created,
        "keyCount": key_count,
        "activeKey": active_key,
    });
    list.retain(|e| e.get("id").and_then(|x| x.as_str()) != Some(id.as_str()));
    list.push(entry.clone());
    write_myproviders_raw(&list)?;
    if let Some(k) = api_key {
        if !k.trim().is_empty() {
            // The dialog's key replaces the *active* key: overwrite its pool slot if a pool exists,
            // otherwise write the legacy single entry (pools are created via add_provider_key).
            if key_count > 0 {
                let idx = if active_key < key_count { active_key } else { 0 };
                kr_set(KR_PROVIDERS, &format!("provider:{id}:{idx}"), k.trim())?;
            } else {
                kr_set(KR_PROVIDERS, &format!("provider:{id}"), k.trim())?;
            }
        }
    }
    Ok(myprovider_from_entry(&entry))
}

/// Delete a provider record and its Credential Manager entry.
#[tauri::command]
fn delete_my_provider(id: String) -> Result<(), String> {
    let mut list = read_myproviders_raw();
    let (key_count, _) = list
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .map(key_pool_meta)
        .unwrap_or((0, 0));
    let before = list.len();
    list.retain(|e| e.get("id").and_then(|x| x.as_str()) != Some(id.as_str()));
    if list.len() != before {
        write_myproviders_raw(&list)?;
    }
    // Purge both the legacy single entry and every pool slot.
    kr_delete(KR_PROVIDERS, &format!("provider:{id}"));
    for i in 0..key_count {
        kr_delete(KR_PROVIDERS, &format!("provider:{id}:{i}"));
    }
    Ok(())
}

/// Append a key to a provider's rotation pool. On the first add we migrate the legacy single key
/// (`provider:<id>`) into slot 0 so the pool subsumes it. The new key is appended (it does not
/// become active — rotation is explicit via next_provider_key). `api_key` arrives over Tauri IPC.
#[tauri::command]
fn add_provider_key(id: String, api_key: String) -> Result<MyProvider, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("пустой ключ".into());
    }
    let mut list = read_myproviders_raw();
    let idx = list
        .iter()
        .position(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .ok_or("провайдер не найден")?;
    let (mut count, active) = key_pool_meta(&list[idx]);
    // First add: fold the legacy single key (if any) into slot 0 so nothing is orphaned.
    if count == 0 {
        if let Some(legacy) = kr_get(KR_PROVIDERS, &format!("provider:{id}")) {
            kr_set(KR_PROVIDERS, &format!("provider:{id}:0"), &legacy)?;
            kr_delete(KR_PROVIDERS, &format!("provider:{id}"));
            count = 1;
        }
    }
    kr_set(KR_PROVIDERS, &format!("provider:{id}:{count}"), key)?;
    count += 1;
    list[idx]["keyCount"] = serde_json::json!(count);
    list[idx]["activeKey"] = serde_json::json!(active.min(count - 1));
    write_myproviders_raw(&list)?;
    Ok(myprovider_from_entry(&list[idx]))
}

/// Remove one key from the pool by index and re-pack the remaining slots (keyring has no enum, so
/// we read survivors, rewrite slots 0..n-1, drop the top slot, and clamp activeKey). Returns the
/// updated provider. Removing the last key collapses the pool back to "no key".
#[tauri::command]
fn remove_provider_key(id: String, index: u64) -> Result<MyProvider, String> {
    let mut list = read_myproviders_raw();
    let pos = list
        .iter()
        .position(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .ok_or("провайдер не найден")?;
    let (count, active) = key_pool_meta(&list[pos]);
    if count == 0 || index >= count {
        return Err("ключ не найден".into());
    }
    // Read all surviving secrets in order, then rewrite slots compactly.
    let mut survivors: Vec<String> = Vec::new();
    for i in 0..count {
        if i == index {
            continue;
        }
        if let Some(k) = kr_get(KR_PROVIDERS, &format!("provider:{id}:{i}")) {
            survivors.push(k);
        }
    }
    for i in 0..count {
        kr_delete(KR_PROVIDERS, &format!("provider:{id}:{i}"));
    }
    for (i, k) in survivors.iter().enumerate() {
        kr_set(KR_PROVIDERS, &format!("provider:{id}:{i}"), k)?;
    }
    let new_count = survivors.len() as u64;
    // Keep the active key pointing at a valid slot: shift down if we removed at/below it.
    let new_active = if new_count == 0 {
        0
    } else if active >= index && active > 0 {
        (active - 1).min(new_count - 1)
    } else {
        active.min(new_count - 1)
    };
    list[pos]["keyCount"] = serde_json::json!(new_count);
    list[pos]["activeKey"] = serde_json::json!(new_active);
    write_myproviders_raw(&list)?;
    Ok(myprovider_from_entry(&list[pos]))
}

/// Persist freellmapi dashboard credentials in the Credential Manager: email+password (preferred —
/// Castellyn logs in programmatically via /api/auth/login) and/or a pasted session token (fallback).
/// Empty/None fields are left untouched. Secrets never touch JSON.
#[tauri::command]
fn set_freellmapi_auth(
    email: Option<String>,
    password: Option<String>,
    token: Option<String>,
) -> Result<(), String> {
    let mut any = false;
    for (user, val) in [("email", &email), ("password", &password), ("token", &token)] {
        if let Some(v) = val {
            let v = v.trim();
            if !v.is_empty() {
                kr_set(KR_FREELLMAPI, user, v)?;
                any = true;
            }
        }
    }
    if !any {
        return Err("укажите email+пароль или токен дашборда freellmapi".into());
    }
    Ok(())
}

/// Which freellmapi auth is configured (for the UI). Never returns the secret values themselves.
#[tauri::command]
fn freellmapi_auth_status() -> serde_json::Value {
    serde_json::json!({
        "hasEmail": kr_get(KR_FREELLMAPI, "email").is_some(),
        "hasToken": kr_get(KR_FREELLMAPI, "token").is_some(),
    })
}

/// freellmapi gateway base URL from the `gateway` service port in stack.json. None if absent.
fn gateway_base_url() -> Option<String> {
    let content = std::fs::read_to_string(abs(STACK_CONFIG_REL)).ok()?;
    let v = parse_json_bom(&content).ok()?;
    let port = v
        .get("services")?
        .as_array()?
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some("gateway"))
        .and_then(|e| e.get("port"))
        .and_then(|x| x.as_u64())?;
    Some(format!("http://localhost:{port}"))
}

/// Native port of Connect-CustomProvider.ps1: register a custom OpenAI-compatible provider in the
/// freellmapi gateway. Authenticates with the saved session `token`, else logs in via
/// `/api/auth/login` (email+password), then POSTs `/api/keys/custom`. Returns the exit code; streams
/// progress via out/err. Secrets are ordinary captured args here (process memory) — no STDIN dance.
#[allow(clippy::too_many_arguments)]
fn connect_custom_native(
    gateway: &str,
    base_url: &str,
    model: &str,
    display_name: &str,
    label: &str,
    token: &str,
    email: &str,
    password: &str,
    api_key: &str,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    use serde_json::{json, Value};
    let base = gateway.trim_end_matches('/');
    let token = token.trim();
    let email = email.trim();
    let api_key = api_key.trim();
    // Generous timeout: a cold gateway login/registration can take a few seconds.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();

    // Authenticate: reuse the token, else log in with email+password.
    let token = if token.is_empty() {
        if email.is_empty() || password.is_empty() {
            err("ОШИБКА: нет токена и неполные email/пароль freellmapi.");
            return 1;
        }
        out("  Вход в freellmapi (email+пароль)…");
        let body = json!({ "email": email, "password": password }).to_string();
        match agent
            .post(&format!("{base}/api/auth/login"))
            .header("Content-Type", "application/json")
            .send(body.as_str())
        {
            Ok(mut resp) => {
                let parsed = resp
                    .body_mut()
                    .read_to_string()
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok());
                match parsed.as_ref().and_then(|v| v["token"].as_str()).filter(|t| !t.is_empty()) {
                    Some(t) => t.to_string(),
                    None => {
                        err("  ОШИБКА входа в freellmapi: login не вернул токен");
                        return 1;
                    }
                }
            }
            Err(ureq::Error::StatusCode(401)) => {
                err("  ОШИБКА входа (401): неверный email или пароль freellmapi.");
                return 1;
            }
            Err(ureq::Error::StatusCode(429)) => {
                err("  ОШИБКА входа (429): слишком много попыток, подождите ~15 мин.");
                return 1;
            }
            Err(e) => {
                err(&format!("  ОШИБКА входа в freellmapi: {e}"));
                return 1;
            }
        }
    } else {
        token.to_string()
    };

    // Build the registration payload (mirrors the script's optional fields).
    let mut payload = serde_json::Map::new();
    payload.insert("baseUrl".into(), json!(base_url));
    payload.insert(
        "displayName".into(),
        json!(if display_name.is_empty() { base_url } else { display_name }),
    );
    if !label.is_empty() {
        payload.insert("label".into(), json!(label));
    }
    if !model.is_empty() {
        payload.insert("models".into(), json!([model]));
    }
    if !api_key.is_empty() {
        payload.insert("apiKey".into(), json!(api_key));
    }
    let payload = Value::Object(payload).to_string();

    let uri = format!("{base}/api/keys/custom");
    out("=== freellmapi: регистрация custom-провайдера ===");
    out(&format!(
        "  POST {uri}  (baseUrl={base_url}, model={})",
        if model.is_empty() { "—" } else { model }
    ));

    match agent
        .post(&uri)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(payload.as_str())
    {
        Ok(mut resp) => {
            let v = resp
                .body_mut()
                .read_to_string()
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or(Value::Null);
            let key_id = v["keyId"].as_str().unwrap_or("");
            let platform = v["platform"].as_str().unwrap_or("");
            out(&format!("  OK: провайдер зарегистрирован (keyId={key_id}, platform={platform})."));
            if let Some(models) = v["models"].as_array() {
                let names: Vec<String> =
                    models.iter().filter_map(|m| m.as_str().map(String::from)).collect();
                if !names.is_empty() {
                    out(&format!("  Модели: {}", names.join(", ")));
                }
            }
            out("  Готово. Провайдер доступен через freellmapi (:13001) для Claude Code (ccr) и opencode.");
            0
        }
        Err(ureq::Error::StatusCode(code @ (401 | 403))) => {
            err(&format!(
                "  ОШИБКА авторизации ({code}): сессия freellmapi недействительна — переавторизуйтесь (Вход freellmapi)."
            ));
            1
        }
        Err(ureq::Error::StatusCode(400)) => {
            err("  ОШИБКА (400): freellmapi отклонил baseUrl или тело запроса.");
            1
        }
        Err(e) => {
            err(&format!("  ОШИБКА запроса к freellmapi: {e}"));
            1
        }
    }
}

/// Connect a saved provider to a harness. Dispatches by connectVia/protocol; the key (and the
/// freellmapi dash-token) are read from the Credential Manager and used in-process.
#[tauri::command]
async fn connect_my_provider(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
) -> Result<i32, String> {
    let list = read_myproviders_raw();
    let e = list
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .ok_or("провайдер не найден")?;
    let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let (protocol, via, base_url) = (s("protocol"), s("connectVia"), s("baseUrl"));
    valid_base_url(&base_url)?;
    let api_key = active_provider_key(&id, e)
        .ok_or("для этого провайдера не задан API-ключ")?;

    match (via.as_str(), protocol.as_str()) {
        // Anthropic-native → bind straight to a profile's settings.json (native Manage-Provider).
        ("direct", "anthropic") => {
            let name = s("targetProfile");
            if !valid_profile_name(&name) {
                return Err("для прямого подключения укажите корректный целевой профиль".into());
            }
            let model = s("model");
            let small = s("smallModel");
            run_native_streamed(app, state, "provider".into(), move |out, err| {
                manage_provider_native(
                    &name,
                    "set",
                    &base_url,
                    false,
                    Some(&api_key),
                    Some(model.as_str()),
                    Some(small.as_str()),
                    out,
                    err,
                )
            })
            .await
        }
        // OpenAI direct → would need claude-code-router, which is currently broken.
        ("direct", "openai") => Err(
            "OpenAI-провайдер напрямую требует claude-code-router (сейчас недоступен) — подключите через freellmapi"
                .into(),
        ),
        // Via the freellmapi hub → register as a custom OpenAI-compatible endpoint. The script
        // logs in (email+password → session) if no token is set, then POSTs /api/keys/custom.
        ("freellmapi", _) => {
            let token = kr_get(KR_FREELLMAPI, "token").unwrap_or_default();
            let email = kr_get(KR_FREELLMAPI, "email").unwrap_or_default();
            let password = kr_get(KR_FREELLMAPI, "password").unwrap_or_default();
            if token.is_empty() && (email.is_empty() || password.is_empty()) {
                return Err("сначала задайте вход в freellmapi (email+пароль или токен) — кнопка «Вход freellmapi»".into());
            }
            let gateway = gateway_base_url().ok_or("не найден gateway в stack.json")?;
            let (model, display_name, label) =
                (s("model"), s("name"), format!("agenthub:{}", s("name")));
            run_native_streamed(app, state, "provider".into(), move |out, err| {
                connect_custom_native(
                    &gateway,
                    &base_url,
                    &model,
                    &display_name,
                    &label,
                    &token,
                    &email,
                    &password,
                    &api_key,
                    out,
                    err,
                )
            })
            .await
        }
        _ => Err(format!("неизвестная комбинация connectVia/protocol: {via}/{protocol}")),
    }
}

/// Rotate to the next key in a provider's pool and re-connect (rewrites the target harness with the
/// newly-active key). For manual balance-exhaustion rotation, e.g. aerolink: click → next key → cc2
/// is rebound. Errors if the pool has fewer than two keys. Returns the connect exit code.
#[tauri::command]
async fn next_provider_key(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
) -> Result<i32, String> {
    let mut list = read_myproviders_raw();
    let pos = list
        .iter()
        .position(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .ok_or("провайдер не найден")?;
    let (count, active) = key_pool_meta(&list[pos]);
    if count < 2 {
        return Err("у провайдера только один ключ — добавьте ещё для ротации".into());
    }
    let next = next_key_index(active, count);
    list[pos]["activeKey"] = serde_json::json!(next);
    write_myproviders_raw(&list)?;
    // Re-bind the harness to the now-active key (reuses the full connect dispatch).
    connect_my_provider(app, state, id).await
}

/// Count models in an OpenAI/Anthropic-style /models response (data[] | models[] | bare array).
fn count_models(v: &serde_json::Value) -> usize {
    if let Some(d) = v.get("data").and_then(|x| x.as_array()) {
        return d.len();
    }
    if let Some(m) = v.get("models").and_then(|x| x.as_array()) {
        return m.len();
    }
    if let Some(a) = v.as_array() {
        return a.len();
    }
    0
}

/// Does a URL carry a non-empty path after the host? (mirrors the old PS `[uri].AbsolutePath` check)
fn url_has_path(u: &str) -> bool {
    let after = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
    after
        .split_once('/')
        .map(|(_, p)| !p.trim_matches('/').is_empty())
        .unwrap_or(false)
}

/// Native provider liveness probe (was Check-Provider.ps1). Blocking — call via spawn_blocking.
/// GET {root}/v1/models with the optional key; returns `{ ok, detail, count? }` (same shape as before).
fn probe_provider(base_url: &str, protocol: &str, api_key: &str) -> serde_json::Value {
    // Normalize: strip a trailing /v1, then always query /v1/models (works with or without /v1).
    let root = base_url.trim_end_matches('/');
    let root = root.strip_suffix("/v1").unwrap_or(root);
    let url = format!("{root}/v1/models");

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(12)))
        .build()
        .into();
    let mut req = agent.get(&url);
    if protocol == "anthropic" {
        if !api_key.is_empty() {
            req = req.header("x-api-key", api_key);
        }
        req = req.header("anthropic-version", "2023-06-01");
    } else if !api_key.is_empty() {
        req = req.header("Authorization", &format!("Bearer {api_key}"));
    }

    match req.call() {
        Ok(mut resp) => {
            let n = resp
                .body_mut()
                .read_to_string()
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .map(|v| count_models(&v))
                .unwrap_or(0);
            serde_json::json!({ "ok": true, "detail": format!("ответил (моделей: {n})"), "count": n })
        }
        Err(ureq::Error::StatusCode(code)) => {
            // An HTTP status means the server is ALIVE (it answered). Only auth failure is a real
            // problem; any other status (e.g. 404 — routers/bridges like ccr have no /v1/models)
            // still means "responding".
            if code == 401 || code == 403 {
                serde_json::json!({ "ok": false, "detail": format!("ключ отклонён ({code})") })
            } else {
                serde_json::json!({ "ok": true, "detail": format!("отвечает (HTTP {code})") })
            }
        }
        Err(e) => serde_json::json!({ "ok": false, "detail": format!("не отвечает: {e}") }),
    }
}

/// Native model list (was Get-EngineModels.ps1). Blocking — call via spawn_blocking.
/// GET <base>/models (or /v1/models for a bare host). Returns model ids; empty on any error.
fn fetch_engine_models(base_url: &str) -> Vec<String> {
    if base_url.is_empty() {
        return Vec::new();
    }
    let u = base_url.trim_end_matches('/');
    let url = if u.ends_with("/models") {
        u.to_string()
    } else if url_has_path(u) {
        format!("{u}/models")
    } else {
        format!("{u}/v1/models")
    };

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(6)))
        .build()
        .into();
    // Some servers want an Authorization header even when no real key is needed.
    let body = match agent.get(&url).header("Authorization", "Bearer not-needed").call() {
        Ok(mut resp) => resp.body_mut().read_to_string().unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Vec::new();
    };
    let arr = v.get("data").and_then(|x| x.as_array()).or_else(|| v.as_array());
    match arr {
        Some(items) => items
            .iter()
            .filter_map(|it| it.get("id").and_then(|x| x.as_str()).map(String::from))
            .filter(|s| !s.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

/// Shared liveness check: GET {baseUrl}/v1/models with the API key. Returns `{ ok, detail, count? }`.
/// Native HTTP (ureq) via spawn_blocking — no PowerShell, no run slot taken.
async fn run_provider_check(base_url: &str, protocol: &str, api_key: &str) -> serde_json::Value {
    let (b, p, k) = (base_url.to_string(), protocol.to_string(), api_key.to_string());
    tokio::task::spawn_blocking(move || probe_provider(&b, &p, &k))
        .await
        .unwrap_or_else(|e| serde_json::json!({ "ok": false, "detail": format!("{e}") }))
}

/// Liveness check for a saved custom provider: key read from the Credential Manager.
#[tauri::command]
async fn check_my_provider(id: String) -> serde_json::Value {
    let list = read_myproviders_raw();
    let entry = list
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
        .cloned();
    let Some(e) = entry else {
        return serde_json::json!({ "ok": false, "detail": "провайдер не найден" });
    };
    let base_url = e.get("baseUrl").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let protocol = e.get("protocol").and_then(|x| x.as_str()).unwrap_or("openai").to_string();
    let api_key = active_provider_key(&id, &e).unwrap_or_default();
    run_provider_check(&base_url, &protocol, &api_key).await
}

/// Liveness check for an arbitrary base URL (local engines / stack services — no key needed).
#[tauri::command]
async fn check_provider_url(base_url: String, protocol: String) -> serde_json::Value {
    run_provider_check(&base_url, &protocol, "").await
}

// --- Provider balance (#B4) ------------------------------------------------------------------
// Balance is provider-specific (no universal endpoint). We try, in order: a user-configured
// balanceUrl, then known shapes (DeepSeek /user/balance, OpenAI-billing /dashboard/billing).

/// Follow a dot-path (segments may be array indices) to a numeric value (number or numeric string).
fn json_f64(v: &serde_json::Value, path: &str) -> Option<f64> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = if let Ok(i) = seg.parse::<usize>() { cur.get(i)? } else { cur.get(seg)? };
    }
    match cur {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// GET a balance endpoint with the provider's auth header; parse the JSON body.
fn balance_get(agent: &ureq::Agent, url: &str, protocol: &str, key: &str) -> Option<serde_json::Value> {
    let mut req = agent.get(url);
    if protocol == "anthropic" {
        if !key.is_empty() {
            req = req.header("x-api-key", key);
        }
        req = req.header("anthropic-version", "2023-06-01");
    } else if !key.is_empty() {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    let body = req.call().ok()?.body_mut().read_to_string().ok()?;
    serde_json::from_str(&body).ok()
}

/// Extract (amount, currency) from a balance response across common shapes.
fn extract_balance(v: &serde_json::Value) -> Option<(f64, String)> {
    let cur = v
        .get("currency")
        .and_then(|x| x.as_str())
        .or_else(|| v.pointer("/balance_infos/0/currency").and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    for p in [
        "balance",
        "data.balance",
        "balance_infos.0.total_balance",
        "total_balance",
        "hard_limit_usd",
        "quota",
        "data.quota",
        "remaining",
    ] {
        if let Some(n) = json_f64(v, p) {
            return Some((n, cur));
        }
    }
    None
}

fn fetch_provider_balance(id: &str) -> serde_json::Value {
    let list = read_myproviders_raw();
    let Some(e) = list.iter().find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id)) else {
        return serde_json::json!({ "ok": false, "detail": "провайдер не найден" });
    };
    let base = e.get("baseUrl").and_then(|x| x.as_str()).unwrap_or("");
    let protocol = e.get("protocol").and_then(|x| x.as_str()).unwrap_or("openai");
    let balance_url = e.get("balanceUrl").and_then(|x| x.as_str()).unwrap_or("");
    let key = active_provider_key(id, e).unwrap_or_default();
    let root = base.trim_end_matches('/').trim_end_matches("/v1").trim_end_matches('/');

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();

    // 1) User-configured balance URL — most reliable.
    if !balance_url.is_empty() {
        return match balance_get(&agent, balance_url, protocol, &key) {
            Some(v) => match extract_balance(&v) {
                Some((amt, cur)) => serde_json::json!({ "ok": true, "amount": amt, "currency": cur, "detail": "" }),
                None => serde_json::json!({ "ok": false, "detail": "не нашёл число баланса в ответе" }),
            },
            None => serde_json::json!({ "ok": false, "detail": "balance-URL не ответил" }),
        };
    }
    // 2) DeepSeek-style.
    if base.contains("deepseek") {
        if let Some(v) = balance_get(&agent, &format!("{root}/user/balance"), protocol, &key) {
            if let Some((amt, cur)) = extract_balance(&v) {
                return serde_json::json!({ "ok": true, "amount": amt, "currency": cur, "detail": "" });
            }
        }
    }
    // 3) OpenAI-billing style (one-api / new-api gateways).
    if let Some(v) = balance_get(&agent, &format!("{root}/dashboard/billing/subscription"), protocol, &key) {
        if let Some(amt) = json_f64(&v, "hard_limit_usd").or_else(|| json_f64(&v, "system_hard_limit_usd")) {
            return serde_json::json!({ "ok": true, "amount": amt, "currency": "USD", "detail": "лимит" });
        }
    }
    serde_json::json!({ "ok": false, "detail": "баланс недоступен — задайте balance-URL в настройках провайдера" })
}

/// Best-effort balance/credits for a custom provider (#B4). `{ ok, amount?, currency?, detail }`.
#[tauri::command]
async fn check_provider_balance(id: String) -> serde_json::Value {
    tokio::task::spawn_blocking(move || fetch_provider_balance(&id))
        .await
        .unwrap_or_else(|e| serde_json::json!({ "ok": false, "detail": format!("{e}") }))
}

/// Read-only view of a profile's CLAUDE.md or settings.json (#80). Whitelisted filenames +
/// validated profile name guard against path traversal.
#[tauri::command]
async fn read_profile_file(name: String, which: String) -> Result<String, String> {
    if !valid_profile_name(&name) {
        return Err("invalid profile name".into());
    }
    let file = match which.as_str() {
        "claude" => "CLAUDE.md",
        "settings" => "settings.json",
        _ => return Err("unknown file".into()),
    };
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let path = std::path::Path::new(&home)
        .join(format!(".claude-{name}"))
        .join(file);
    std::fs::read_to_string(&path).map_err(|e| format!("{e}"))
}

// --- Claude Code usage limits (per profile) ---------------------------------------------------
// Mirrors the user's statusline: each profile's OAuth token (~/.claude-<name>/.credentials.json)
// → GET the usage endpoint → 5-hour & 7-day utilization + reset times. Used to show remaining
// budget over each profile (and session).

#[derive(Clone, Serialize)]
struct ProfileUsage {
    #[serde(rename = "fiveHourPct")]
    five_hour_pct: Option<f64>,
    #[serde(rename = "sevenDayPct")]
    seven_day_pct: Option<f64>,
    #[serde(rename = "fiveHourResetsAt")]
    five_hour_resets_at: Option<String>,
    #[serde(rename = "sevenDayResetsAt")]
    seven_day_resets_at: Option<String>,
}

#[derive(Default)]
struct UsageCache(Mutex<std::collections::HashMap<String, (std::time::Instant, ProfileUsage)>>);

/// Blocking: read a profile's OAuth token and query the usage endpoint. None on any failure
/// (not logged in / token expired / offline) so the UI just omits the badge.
fn fetch_profile_usage(profile: &str) -> Option<ProfileUsage> {
    let home = std::env::var("USERPROFILE").ok()?;
    let creds = format!("{home}\\.claude-{profile}\\.credentials.json");
    let content = std::fs::read_to_string(&creds).ok()?;
    let v: serde_json::Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).ok()?;
    let token = v.get("claudeAiOauth")?.get("accessToken")?.as_str()?.to_string();
    if token.is_empty() {
        return None;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();
    let body = agent
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let r: serde_json::Value = serde_json::from_str(&body).ok()?;
    let pct = |k: &str| r.get(k).and_then(|x| x.get("utilization")).and_then(|x| x.as_f64());
    let reset =
        |k: &str| r.get(k).and_then(|x| x.get("resets_at")).and_then(|x| x.as_str()).map(String::from);
    Some(ProfileUsage {
        five_hour_pct: pct("five_hour"),
        seven_day_pct: pct("seven_day"),
        five_hour_resets_at: reset("five_hour"),
        seven_day_resets_at: reset("seven_day"),
    })
}

/// Claude Code usage limits for a profile (5h + 7d). Cached ~60s per profile; null on any error.
#[tauri::command]
async fn read_profile_usage(
    cache: State<'_, UsageCache>,
    profile: String,
) -> Result<Option<ProfileUsage>, String> {
    if !valid_profile_name(&profile) {
        return Ok(None);
    }
    {
        let map = cache.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, u)) = map.get(&profile) {
            if at.elapsed().as_secs() < 60 {
                return Ok(Some(u.clone()));
            }
        }
    }
    let p = profile.clone();
    let fetched = tokio::task::spawn_blocking(move || fetch_profile_usage(&p)).await.ok().flatten();
    if let Some(u) = &fetched {
        cache
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(profile, (std::time::Instant::now(), u.clone()));
    }
    Ok(fetched)
}

/// opencode's global config path: $OPENCODE_CONFIG → $XDG_CONFIG_HOME\opencode → ~/.config/opencode.
fn opencode_config_path() -> String {
    if let Ok(p) = std::env::var("OPENCODE_CONFIG") {
        if !p.is_empty() {
            return p;
        }
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return format!("{x}\\opencode\\opencode.json");
        }
    }
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    format!("{home}\\.config\\opencode\\opencode.json")
}

#[derive(Serialize)]
struct OpencodeProvider {
    id: String,
    name: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    #[serde(rename = "hasKey")]
    has_key: bool,
}

#[derive(Serialize)]
struct OpencodeStatus {
    installed: bool, // does the config file exist?
    model: String,   // active model "<id>/<model>", or ""
    providers: Vec<OpencodeProvider>,
}

/// opencode's global config (custom providers + active model). Read-only; the apiKey VALUE is
/// never returned (only `has_key`). `installed=false` when no config file exists yet.
#[tauri::command]
fn read_opencode() -> OpencodeStatus {
    let content = match std::fs::read_to_string(opencode_config_path()) {
        Ok(c) => c,
        Err(_) => {
            return OpencodeStatus { installed: false, model: String::new(), providers: Vec::new() }
        }
    };
    let v = match parse_json_bom(&content) {
        Ok(v) => v,
        Err(_) => {
            return OpencodeStatus { installed: true, model: String::new(), providers: Vec::new() }
        }
    };
    let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let mut providers = Vec::new();
    if let Some(obj) = v.get("provider").and_then(|p| p.as_object()) {
        for (id, p) in obj {
            let opts = p.get("options");
            providers.push(OpencodeProvider {
                id: id.clone(),
                name: p.get("name").and_then(|x| x.as_str()).unwrap_or(id).to_string(),
                base_url: opts
                    .and_then(|o| o.get("baseURL"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                has_key: opts
                    .and_then(|o| o.get("apiKey"))
                    .and_then(|x| x.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false),
            });
        }
    }
    OpencodeStatus { installed: true, model, providers }
}

/// Bind (`set`) or unbind (`clear`) a custom OpenAI-compatible provider for opencode via
/// Native port of Manage-OpenCode-Provider.ps1: merge-patch opencode.json's `provider.<id>` (and the
/// top-level active `model`), preserving every other key. apiKey precedence (mirrors the script):
/// keep_key → literal key → `{env:VAR}` → keep existing. Returns the exit code; streams via out/err.
#[allow(clippy::too_many_arguments)]
fn opencode_provider_native(
    cfg_path: &str,
    action: &str,
    provider_id: &str,
    name: Option<&str>,
    base_url: &str,
    model: Option<&str>,
    key: Option<&str>,
    env_key: Option<&str>,
    keep_key: bool,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> i32 {
    use serde_json::{json, Value};

    // Load opencode.json (BOM-tolerant) or start minimal; a parse failure aborts (as the script does).
    let mut cfg: Value = match std::fs::read_to_string(cfg_path) {
        Ok(ref c) if !c.trim().is_empty() => match parse_json_bom(c) {
            Ok(v) => v,
            Err(e) => {
                err(&format!("ОШИБКА: не удалось прочитать opencode.json ({e})."));
                return 1;
            }
        },
        _ => json!({}),
    };
    if !cfg.is_object() {
        cfg = json!({});
    }
    let obj = cfg.as_object_mut().unwrap();
    obj.entry("$schema").or_insert_with(|| json!("https://opencode.ai/config.json"));
    if !obj.get("provider").map(|p| p.is_object()).unwrap_or(false) {
        obj.insert("provider".into(), json!({}));
    }

    out(&format!("=== opencode provider: {action} {provider_id} ==="));

    if action == "set" {
        let mut active_model: Option<String> = None;
        {
            let providers = obj.get_mut("provider").unwrap().as_object_mut().unwrap();
            if !providers.get(provider_id).map(|x| x.is_object()).unwrap_or(false) {
                providers.insert(provider_id.to_string(), json!({}));
            }
            let p = providers.get_mut(provider_id).unwrap().as_object_mut().unwrap();
            p.insert("npm".into(), json!("@ai-sdk/openai-compatible"));
            match name.filter(|s| !s.is_empty()) {
                Some(n) => {
                    p.insert("name".into(), json!(n));
                }
                None => {
                    if !p.contains_key("name") {
                        p.insert("name".into(), json!(provider_id));
                    }
                }
            }
            if !p.get("options").map(|x| x.is_object()).unwrap_or(false) {
                p.insert("options".into(), json!({}));
            }
            let opts = p.get_mut("options").unwrap().as_object_mut().unwrap();
            opts.insert("baseURL".into(), json!(base_url));
            // apiKey: keep_key → leave; literal key; {env:VAR}; else leave as-is.
            if keep_key {
                // leave options.apiKey untouched
            } else if let Some(k) = key.filter(|s| !s.is_empty()) {
                opts.insert("apiKey".into(), json!(k));
            } else if let Some(e) = env_key.filter(|s| !s.is_empty()) {
                opts.insert("apiKey".into(), json!(format!("{{env:{e}}}")));
            }
            // Model: register it (preserve curated models) and remember it as the active model.
            if let Some(m) = model.filter(|s| !s.is_empty()) {
                if !p.get("models").map(|x| x.is_object()).unwrap_or(false) {
                    p.insert("models".into(), json!({}));
                }
                let models = p.get_mut("models").unwrap().as_object_mut().unwrap();
                if !models.contains_key(m) {
                    models.insert(m.to_string(), json!({ "name": m }));
                }
                active_model = Some(format!("{provider_id}/{m}"));
            }
        }
        if let Some(am) = &active_model {
            obj.insert("model".into(), json!(am));
        }
        let key_shown = if keep_key {
            "(без изменений)".to_string()
        } else if key.filter(|s| !s.is_empty()).is_some() {
            "(литерал)".to_string()
        } else if let Some(e) = env_key.filter(|s| !s.is_empty()) {
            format!("{{env:{e}}}")
        } else {
            "(без изменений)".to_string()
        };
        out(&format!(
            "  baseURL={base_url}  model={}  apiKey={key_shown}",
            active_model.as_deref().unwrap_or("—")
        ));
    } else {
        {
            let providers = obj.get_mut("provider").unwrap().as_object_mut().unwrap();
            providers.remove(provider_id);
        }
        let points_here = obj
            .get("model")
            .and_then(|m| m.as_str())
            .map(|s| s.starts_with(&format!("{provider_id}/")))
            .unwrap_or(false);
        if points_here {
            obj.remove("model");
        }
        out(&format!("  Провайдер '{provider_id}' удалён из opencode.json."));
    }

    // Backup then write (UTF-8 no BOM).
    if let Some(dir) = std::path::Path::new(cfg_path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::path::Path::new(cfg_path).exists() {
        let _ = std::fs::copy(cfg_path, format!("{cfg_path}.bak"));
    }
    let serialized = match serde_json::to_string_pretty(&cfg) {
        Ok(s) => s,
        Err(e) => {
            err(&format!("ОШИБКА сериализации opencode.json: {e}"));
            return 1;
        }
    };
    if let Err(e) = write_file_no_bom(cfg_path, &serialized) {
        err(&format!("ОШИБКА записи opencode.json: {e}"));
        return 1;
    }
    out(&format!("  opencode.json обновлён (бэкап .bak): {cfg_path}"));
    0
}

/// Manage-OpenCode-Provider (native): merge-patch of opencode.json. apiKey: literal `key`, else
/// `{env:env_key}`, else keep existing.
#[tauri::command]
async fn run_opencode_provider(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    provider_id: String,
    name: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    key: Option<String>,
    env_key: Option<String>,
    keep_key: Option<bool>,
) -> Result<i32, String> {
    if !matches!(action.as_str(), "set" | "clear") {
        return Err(format!("неизвестное действие opencode: {action}"));
    }
    if !valid_profile_name(&provider_id) {
        return Err(format!("недопустимый provider id: {provider_id}"));
    }
    let base_url = base_url.unwrap_or_default();
    if action == "set" && base_url.is_empty() {
        return Err("для set нужен base_url".into());
    }
    let keep_key = keep_key.unwrap_or(false);
    let cfg_path = opencode_config_path();
    run_native_streamed(app, state, "provider".to_string(), move |out, err| {
        opencode_provider_native(
            &cfg_path,
            &action,
            &provider_id,
            name.as_deref(),
            &base_url,
            model.as_deref(),
            key.as_deref(),
            env_key.as_deref(),
            keep_key,
            out,
            err,
        )
    })
    .await
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
    for p in profile_names() {
        if let Some(servers) = profile_mcp_servers(&p) {
            existing_profiles.push(p.clone());
            per_profile.push((p, servers));
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
    read_json_opt(abs(SCHEDULES_JSON_REL), "schedules")
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
/// NOTE: kept on PowerShell deliberately. The deploy must pass each server's JSON to
/// `claude mcp add-json <name> <json>`; PowerShell 7's native arg-passing forwards the quoted JSON
/// to the `claude.cmd` shim intact, but Rust's `std::process::Command` .cmd escaping mangles it
/// (claude then rejects it with "Invalid configuration: Invalid input"). A native port would need to
/// edit each profile's live `.claude.json` directly — too invasive for the gain. The PS path works.
#[tauri::command]
async fn run_mcp(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    only: Option<Vec<String>>,
) -> Result<i32, String> {
    let script_rel = match action.as_str() {
        "deploy" => MCP_DEPLOY_SCRIPT_REL,
        _ => return Err(format!("неизвестное действие mcp: {action}")),
    };
    let script = abs(script_rel);
    // Optional `-Only a,b` limits deployment to specific profiles (Deploy-Mcp.ps1 supports it);
    // empty/None deploys to all.
    let args = match only {
        Some(p) if !p.is_empty() => vec!["-Only".to_string(), p.join(",")],
        _ => Vec::new(),
    };
    spawn_streamed(app, state, "mcp".to_string(), script, args).await
}


/// Map plugin id → description, read from each installed plugin's .claude-plugin/plugin.json
/// (the `claude plugin list --json` output has no description).
fn plugin_descriptions() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(home) = std::env::var("USERPROFILE") else { return map };
    let plugins_dir = format!("{home}\\.claude\\plugins");
    let installed = std::fs::read_to_string(format!("{plugins_dir}\\installed_plugins.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok());
    let markets = std::fs::read_to_string(format!("{plugins_dir}\\known_marketplaces.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .unwrap_or(serde_json::Value::Null);
    let Some(po) = installed.as_ref().and_then(|v| v.get("plugins")).and_then(|v| v.as_object())
    else {
        return map;
    };
    for (id, arr) in po {
        let install_path = arr
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e.get("installPath"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(dir) = plugin_content_dir(id, install_path, &markets) {
            let pj = std::path::Path::new(&dir).join(".claude-plugin").join("plugin.json");
            if let Some(d) = std::fs::read_to_string(pj)
                .ok()
                .and_then(|c| parse_json_bom(&c).ok())
                .and_then(|m| m.get("description").and_then(|v| v.as_str()).map(String::from))
                .filter(|s| !s.is_empty())
            {
                map.insert(id.clone(), d);
            }
        }
    }
    map
}

/// List installed plugins via `claude plugin list --json`, enriched with descriptions from disk.
#[tauri::command]
async fn list_plugins() -> Result<serde_json::Value, String> {
    let out = tokio::process::Command::new("pwsh")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "claude plugin list --json"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map_err(|e| format!("запуск claude: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut v = parse_json_bom(stdout.trim()).map_err(|e| format!("parse plugins: {e}"))?;
    let desc = plugin_descriptions();
    let own = own_marketplaces();
    if let Some(arr) = v.as_array_mut() {
        for item in arr.iter_mut() {
            let id = item.get("id").and_then(|x| x.as_str()).map(String::from);
            if let (Some(id), Some(obj)) = (id, item.as_object_mut()) {
                if let Some(d) = desc.get(&id) {
                    obj.insert("description".into(), serde_json::json!(d));
                }
                let mp = id.rsplit('@').next().unwrap_or("");
                obj.insert("mine".into(), serde_json::json!(own.contains(mp)));
            }
        }
    }
    Ok(v)
}

#[derive(Serialize)]
struct SkillInfo {
    name: String,
    description: String,
    version: String,
    dir: String,
    /// "own" (your symlinked skills), "default" (plain dir in ~/.claude/skills),
    /// or "plugin:<id>" (bundled inside a plugin).
    source: String,
    /// True when authored by you: a symlinked skill OR from a local (directory-source) marketplace
    /// you maintain (e.g. max-marketplace), as opposed to third-party github marketplaces.
    mine: bool,
}

/// Names of marketplaces whose source is a local directory (i.e. authored/maintained by the user),
/// e.g. "max-marketplace". Plugins/skills from these count as "mine".
fn own_marketplaces() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Ok(home) = std::env::var("USERPROFILE") else { return set };
    let p = format!("{home}\\.claude\\plugins\\known_marketplaces.json");
    if let Some(v) = std::fs::read_to_string(p).ok().and_then(|c| parse_json_bom(&c).ok()) {
        let obj = v.get("marketplaces").and_then(|m| m.as_object()).or_else(|| v.as_object());
        if let Some(obj) = obj {
            for (name, m) in obj {
                let stype = m
                    .get("source")
                    .and_then(|s| s.get("source"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if stype == "directory" {
                    set.insert(name.clone());
                }
            }
        }
    }
    set
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

/// Parse one skill directory (name/description/version from SKILL.md, dir name as fallback).
fn read_skill_info(skill_dir: &std::path::Path, source: String, mine: bool) -> SkillInfo {
    let dir_name = skill_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut name = dir_name;
    let mut description = String::new();
    let mut version = String::new();
    if let Ok(content) = std::fs::read_to_string(skill_dir.join("SKILL.md")) {
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
    SkillInfo { name, description, version, dir: skill_dir.display().to_string(), source, mine }
}

/// Skills bundled inside installed plugins (source = "plugin:<id>").
fn plugin_bundled_skills() -> Vec<SkillInfo> {
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
    let own = own_marketplaces();
    let mut out: Vec<SkillInfo> = Vec::new();
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
        let mp = id.rsplit('@').next().unwrap_or("");
        let mine = own.contains(mp);
        if let Ok(entries) = std::fs::read_dir(std::path::Path::new(&dir).join("skills")) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.push(read_skill_info(&p, format!("plugin:{id}"), mine));
                }
            }
        }
    }
    out
}

fn skill_rank(s: &SkillInfo) -> u8 {
    if s.mine {
        0
    } else if s.source == "default" {
        1
    } else {
        2
    }
}

/// All skills: standalone in ~/.claude/skills (own = symlink to your collection, default = plain
/// dir) PLUS skills bundled in installed plugins. `is_dir()` follows symlinks so your symlinked
/// "own" skills are no longer dropped. Sorted by source (own → default → plugin) then name.
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
            let p = e.path();
            if !p.is_dir() {
                continue; // follows symlinks → includes symlinked "own" skills
            }
            let is_link = std::fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            out.push(read_skill_info(
                &p,
                if is_link { "own".into() } else { "default".into() },
                is_link,
            ));
        }
    }
    out.extend(plugin_bundled_skills());
    out.sort_by(|a, b| {
        skill_rank(a)
            .cmp(&skill_rank(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
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

/// Run `claude plugin <action> <id>` once, optionally under a specific CLAUDE_CONFIG_DIR profile.
/// Streams stdout/stderr to the UI log (indented). Simple args only (no JSON) — the `.cmd` shim
/// launches cleanly under Rust's escaping (unlike the Deploy-Mcp add-json case).
fn run_claude_plugin(
    claude: &std::path::Path,
    cfg_dir: Option<&str>,
    action: &str,
    id: &str,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) {
    let mut cmd = std::process::Command::new(claude);
    cmd.args(["plugin", action, id]).creation_flags(CREATE_NO_WINDOW);
    if let Some(d) = cfg_dir {
        cmd.env("CLAUDE_CONFIG_DIR", d);
    }
    match cmd.output() {
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                out(&format!("    {line}"));
            }
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                err(&format!("    {line}"));
            }
        }
        Err(e) => err(&format!("    не удалось запустить claude: {e}")),
    }
}

/// Native port of Manage-Plugin.ps1: enable / disable / update a plugin via the claude CLI.
/// `update` runs once (the plugins/ cache is shared across profiles via junction); enable/disable
/// loop every profile (enabled-state is per-profile, switched via CLAUDE_CONFIG_DIR). Uses the
/// canonical `profile_names()` (vs the script's hardcoded 6) so a 7th profile is covered too.
/// Returns the exit code; streams via out/err.
fn manage_plugin_native(action: &str, id: &str, out: &dyn Fn(&str), err: &dyn Fn(&str)) -> i32 {
    let Some(claude) = exe_on_path("claude") else {
        err("claude CLI не найден на PATH.");
        return 1;
    };
    out(&format!("=== Плагин: {action} {id} ==="));
    if action == "update" {
        run_claude_plugin(&claude, None, action, id, out, err);
    } else {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        for p in profile_names() {
            let dir = format!("{home}\\.claude-{p}");
            if !std::path::Path::new(&dir).exists() {
                out(&format!("  [skip] {p} (нет каталога)"));
                continue;
            }
            out(&format!("  [{p}] claude plugin {action} {id}"));
            run_claude_plugin(&claude, Some(&dir), action, id, out, err);
        }
    }
    out("Готово.");
    0
}

/// Manage one plugin: enable / disable / update (native; was Manage-Plugin.ps1).
#[tauri::command]
async fn run_plugin(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    id: String,
) -> Result<i32, String> {
    // Uninstall runs the claude CLI directly (the vetted script only does enable/disable/update).
    // Guard the id since it reaches `cmd /c` — plugin ids are name@marketplace, never shell metachars.
    if action == "remove" {
        if id.is_empty()
            || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
        {
            return Err(format!("недопустимый id плагина: {id}"));
        }
        return spawn_streamed_prog(
            app,
            state,
            "plugin-mgr".to_string(),
            "cmd".to_string(),
            vec!["/c".into(), "claude".into(), "plugin".into(), "remove".into(), id],
            None,
        )
        .await;
    }
    if !matches!(action.as_str(), "enable" | "disable" | "update") {
        return Err(format!("неизвестное действие plugin: {action}"));
    }
    // id reaches process args natively now — guard it (same rule as the remove branch).
    if id.is_empty()
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
    {
        return Err(format!("недопустимый id плагина: {id}"));
    }
    run_native_streamed(app, state, "plugin-mgr".to_string(), move |out, err| {
        manage_plugin_native(&action, &id, out, err)
    })
    .await
}

/// Delete a standalone skill from ~/.claude/skills. Guard uses the entry's PARENT (not the
/// resolved target) so a symlinked "own" skill only has its LINK removed — the real collection in
/// ~/.agents stays intact. Plugin-bundled skills (parent ≠ ~/.claude/skills) are refused.
#[tauri::command]
fn delete_skill(dir: String) -> Result<(), String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let skills_root = std::path::Path::new(&home).join(".claude").join("skills");
    let target = std::path::Path::new(&dir);
    let parent = target.parent().ok_or("неверный путь")?;
    let canon_root = std::fs::canonicalize(&skills_root).map_err(|e| format!("папка скиллов: {e}"))?;
    let canon_parent = std::fs::canonicalize(parent).map_err(|e| format!("скилл не найден: {e}"))?;
    if canon_parent != canon_root {
        return Err("скилл не в ~/.claude/skills (скиллы из плагинов удаляются вместе с плагином)".into());
    }
    let meta = std::fs::symlink_metadata(target).map_err(|e| format!("скилл не найден: {e}"))?;
    if meta.file_type().is_symlink() {
        // Remove only the symlink, never the linked-to source collection.
        std::fs::remove_dir(target)
            .or_else(|_| std::fs::remove_file(target))
            .map_err(|e| format!("удаление ссылки: {e}"))
    } else {
        std::fs::remove_dir_all(target).map_err(|e| format!("удаление: {e}"))
    }
}

#[tauri::command]
fn read_config() -> HubConfig {
    read_config_file()
}

/// Serialize a config to disk verbatim (no field preservation). The single file-write primitive.
fn write_config_file(config: &HubConfig) -> Result<(), String> {
    let p = config_path().ok_or_else(|| tr("err.no_appdata", cur_lang()).to_string())?;
    if let Some(dir) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&p, json)
        .map_err(|e| trv("err.write_config", cur_lang(), &[("e", &e.to_string())]))?;
    Ok(())
}

#[tauri::command]
fn write_config(mut config: HubConfig) -> Result<(), String> {
    // language is owned by set_language — a generic settings save must never clobber it.
    config.language = read_config_file().language;
    write_config_file(&config)
}

/// Mirror the UI locale into the backend: update the in-process Lang (so errors/log localize),
/// persist it in config (so the tray is correct at next startup too), and relabel the tray now.
#[tauri::command]
fn set_language(app: AppHandle, lang: String) -> Result<(), String> {
    let l = Lang::parse(&lang);
    set_cur_lang(l);
    let mut cfg = read_config_file();
    cfg.language = Some(lang);
    write_config_file(&cfg)?;
    rebuild_tray_menu(&app, l);
    update_tray_tooltip(&app);
    Ok(())
}

/// Resolved paths for the About section.
#[tauri::command]
fn app_paths() -> serde_json::Value {
    let stack = abs(STACK_CONFIG_REL);
    serde_json::json!({
        "scriptsRoot": scripts_root(),
        "configPath": config_path(),
        "exe": std::env::current_exe().ok().map(|p| p.display().to_string()),
        "stackPath": if std::path::Path::new(&stack).exists() { Some(stack) } else { None },
    })
}

/// Export the current Castellyn config to a user-chosen path (#117). Serializes HubConfig so the
/// file is always valid even if config.json was never written.
#[tauri::command]
fn export_config(dest: String) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&read_config_file()).map_err(|e| e.to_string())?;
    std::fs::write(&dest, json).map_err(|e| format!("запись: {e}"))
}

/// Read + validate a config file (#117); returns the parsed HubConfig (the frontend persists it
/// via write_config). Invalid JSON / wrong shape → Err.
#[tauri::command]
fn import_config(src: String) -> Result<HubConfig, String> {
    let text = std::fs::read_to_string(&src).map_err(|e| format!("чтение: {e}"))?;
    // BOM-tolerant like every other file read (PowerShell-written configs often carry one).
    serde_json::from_str::<HubConfig>(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("неверный файл настроек: {e}"))
}

/// A profile's settings.json `env` block as (key, value) pairs. Claude Code (2.1+) applies its
/// settings.json `env` to its own process before the auth check, so a non-empty
/// ANTHROPIC_AUTH_TOKEN there already skips the "Select login method" screen on a bare launch
/// (verified empirically). Re-exporting these at launch is now belt-and-suspenders, not required.
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
    // `set K=V&&` cmd string. This avoids any cmd-metacharacter handling on env values. The login
    // screen is already skipped by the token in settings.json `env`; this re-export is redundant
    // safety (kept harmless) rather than the mechanism.
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
    // `start`'s first token, if unquoted, is taken as the program to run — a bare
    // `start Repo` makes cmd look for a program named "Repo" and fail. Pass an empty
    // quoted title ("") so `start` treats the following `cmd /k` as the command.
    // (Rust quotes an empty arg as `""`; a bare word like "Repo" stays unquoted.)
    std::process::Command::new("cmd")
        .args(["/c", "start", "", "cmd", "/k"])
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
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("не удалось открыть {path}: {e}"))?;
    Ok(())
}

const AUTOSTART_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const AUTOSTART_NAME: &str = "Castellyn";
const LEGACY_AUTOSTART_NAME: &str = "AgentHub";

/// One-time migration of the autostart Run entry from the old `AgentHub` value to `Castellyn`.
/// If autostart was on, re-point it at the current exe under the new name and drop the old value;
/// otherwise do nothing. Idempotent — a no-op once the old value is gone.
fn migrate_autostart() {
    let had_old = std::process::Command::new("reg")
        .args(["query", AUTOSTART_KEY, "/v", LEGACY_AUTOSTART_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !had_old {
        return;
    }
    // Re-point at the current exe under the new name, and drop the old value ONLY once the new one
    // is actually in place. If current_exe() or the add fails, leave the old value alone — autostart
    // is preserved and the migration simply retries next launch (idempotent), never silently lost.
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string();
    let added = std::process::Command::new("reg")
        .args(["add", AUTOSTART_KEY, "/v", AUTOSTART_NAME, "/t", "REG_SZ", "/d", &exe, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if added {
        let _ = std::process::Command::new("reg")
            .args(["delete", AUTOSTART_KEY, "/v", LEGACY_AUTOSTART_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

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
    if !valid_profile_name(&name) {
        return Err("недопустимое имя профиля".into());
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let path = format!("{home}\\.claude-{name}");
    std::process::Command::new("explorer")
        .arg(&path)
        .creation_flags(CREATE_NO_WINDOW)
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

/// Build the tray menu with labels in the given locale. Shared by initial build + relabel-on-switch.
fn tray_menu(app: &AppHandle, lang: Lang) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", tr("tray.show", lang), true, None::<&str>)?;
    let check_all = MenuItem::with_id(app, "check_all", tr("tray.check_all", lang), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr("tray.quit", lang), true, None::<&str>)?;
    Menu::with_items(app, &[&show, &check_all, &quit])
}

/// Relabel the tray menu in-place after a language switch (no app restart needed).
fn rebuild_tray_menu(app: &AppHandle, lang: Lang) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("main-tray"), tray_menu(app, lang)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = tray_menu(app, cur_lang())?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Castellyn")
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

/// Toggle window visibility from the global hotkey: hide when it's the foreground window, else reveal.
fn toggle_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let focused = w.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = w.hide();
        } else {
            reveal(app);
        }
    }
}

/// Reflect the number of open session panes in the tray tooltip.
// ponytail: tooltip count, not a drawn overlay badge — add image-gen only if a visual badge is requested.
fn update_tray_tooltip(app: &AppHandle) {
    let n = app
        .state::<SessionState>()
        .0
        .lock()
        .map(|m| m.len())
        .unwrap_or(0);
    let label = if n == 0 {
        "Castellyn".to_string()
    } else {
        trv("tray.tooltip_sessions", cur_lang(), &[("n", &n.to_string())])
    };
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(&label));
    }
}

/// Register (replacing any previous) the OS-global show/hide accelerator. Errors on a bad/taken combo.
fn register_toggle_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let sc = Shortcut::from_str(accel).map_err(|e| format!("неверная комбинация: {e}"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(sc).map_err(|e| format!("{e}"))
}

/// Apply a new toggle hotkey at runtime. Empty/None clears it. Config persistence is the frontend's job.
#[tauri::command]
fn set_toggle_hotkey(app: AppHandle, accel: Option<String>) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    match accel.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(a) => register_toggle_hotkey(&app, a),
        None => {
            let _ = app.global_shortcut().unregister_all();
            Ok(())
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
// ===================== Parallel terminal sessions (real PTYs) =====================
// Each session runs a profile's `claude` in a true PTY (portable-pty) so its TUI renders in an
// xterm.js pane. Output streams to the frontend as base64 frames on a per-session event; input and
// resize flow back via commands. The live sessions live in Tauri-managed state.

struct PtySession {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

#[derive(Default)]
struct SessionState(Mutex<std::collections::HashMap<String, PtySession>>);

fn gen_session_id() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("s{:015x}", (n as u64) & 0x000f_ffff_ffff_ffff)
}

/// Spawn a tool (claude / opencode / shell) inside a real PTY and stream its output. Returns the
/// session id. Output → event `pty:data:<id>` (base64); termination → `pty:exit:<id>` (exit i32).
#[tauri::command]
fn session_spawn(
    app: AppHandle,
    state: State<'_, SessionState>,
    profile: String,
    tool: Option<String>,
    args: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    on_data: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<String, String> {
    use portable_pty::{CommandBuilder, PtySize};
    let tool = tool.unwrap_or_else(|| "claude".into());
    if !matches!(tool.as_str(), "claude" | "opencode" | "shell") {
        return Err(format!("неизвестный инструмент: {tool}"));
    }
    // The profile only matters for claude (it picks CLAUDE_CONFIG_DIR = ~/.claude-<name>).
    if tool == "claude" && !valid_profile_name(&profile) {
        return Err(format!("недопустимый профиль: {profile}"));
    }
    let size = PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 };
    let pair = portable_pty::native_pty_system()
        .openpty(size)
        .map_err(|e| format!("openpty: {e}"))?;

    // Tools are .cmd shims → launch inside pwsh; -NoExit keeps the pane usable after the tool exits.
    // `shell` opens a bare interactive pwsh. Extra args (e.g. `--effort max`) are the user's own
    // input on their own machine, appended to the launch command verbatim.
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let extra = args.unwrap_or_default();
    let extra = extra.trim();
    let mut cmd = CommandBuilder::new("pwsh");
    cmd.arg("-NoLogo");
    if tool != "shell" {
        let base = if tool == "opencode" { "opencode" } else { "claude" };
        cmd.arg("-NoExit");
        cmd.arg("-Command");
        cmd.arg(if extra.is_empty() { base.to_string() } else { format!("{base} {extra}") });
    }
    if tool == "claude" {
        cmd.env("CLAUDE_CONFIG_DIR", format!("{home}\\.claude-{profile}"));
    }
    let dir = cwd.filter(|c| !c.trim().is_empty()).unwrap_or_else(|| home.clone());
    if !dir.is_empty() {
        cmd.cwd(dir);
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| format!("spawn: {e}"))?;
    drop(pair.slave); // close the slave in the parent so EOF arrives when the child exits
    let mut reader = pair.master.try_clone_reader().map_err(|e| format!("reader: {e}"))?;
    let writer = pair.master.take_writer().map_err(|e| format!("writer: {e}"))?;

    let id = gen_session_id();
    let exit_event = format!("pty:exit:{id}");

    // Reader thread: stream PTY output as raw bytes over the binary channel (no base64/JSON event
    // per chunk) until EOF; then signal termination once.
    let app2 = app.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // Raw body → delivered to JS as an ArrayBuffer (binary, no base64).
                    if on_data
                        .send(tauri::ipc::InvokeResponseBody::Raw(buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = app2.emit(exit_event.as_str(), 0i32);
    });

    state
        .0
        .lock()
        .unwrap()
        .insert(id.clone(), PtySession { master: pair.master, writer, child });
    update_tray_tooltip(&app);
    Ok(id)
}

/// Forward keystrokes (UTF-8) from an xterm pane into the PTY.
#[tauri::command]
fn session_write(state: State<'_, SessionState>, id: String, data: String) -> Result<(), String> {
    use std::io::Write;
    let mut map = state.0.lock().unwrap();
    let s = map.get_mut(&id).ok_or("сессия не найдена")?;
    s.writer.write_all(data.as_bytes()).map_err(|e| format!("write: {e}"))?;
    s.writer.flush().map_err(|e| format!("flush: {e}"))
}

/// Resize the PTY when its pane changes size (xterm fit addon).
#[tauri::command]
fn session_resize(state: State<'_, SessionState>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    use portable_pty::PtySize;
    let map = state.0.lock().unwrap();
    let s = map.get(&id).ok_or("сессия не найдена")?;
    s.master
        .resize(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("resize: {e}"))
}

/// Kill a session's child process and drop it (its reader thread then ends on EOF).
#[tauri::command]
fn session_kill(app: AppHandle, state: State<'_, SessionState>, id: String) -> Result<(), String> {
    if let Some(mut s) = state.0.lock().unwrap().remove(&id) {
        let _ = s.child.kill();
    }
    update_tray_tooltip(&app);
    Ok(())
}

/// Immediate subdirectories of `path` (full paths, sorted, hidden/dot dirs skipped) — powers the
/// "projects root" quick-pick in the session launcher. Read-only; empty on any error.
#[tauri::command]
fn list_subdirs(path: String) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&path) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if !name.starts_with('.') {
                        out.push(p.display().to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Remember window position/size across launches (auto-restores on start, saves on exit).
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // OS-global hotkey to show/hide the window; the actual combo is registered from config in setup.
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    if event.state == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .manage(RunState::default())
        .manage(ForkRuns::default())
        .manage(SessionState::default())
        .manage(UsageCache::default())
        .invoke_handler(tauri::generate_handler![
            list_components,
            read_status,
            run_component,
            run_forks,
            run_fork_repo,
            cancel_fork_repo,
            read_fork_repo_status,
            list_backups,
            run_backup,
            read_profiles,
            run_profiles,
            read_profiles_config,
            run_profile_mgmt,
            repair_profile_elevated,
            relaunch_as_admin,
            open_profile_dir,
            launch_profile,
            read_launch_config,
            set_launch_config,
            measure_context,
            read_sync,
            run_sync,
            read_config_drift,
            run_config_drift,
            read_engines,
            update_engine,
            run_engine,
            run_router,
            run_connect_router,
            read_engine_models,
            list_github_repos,
            read_stack,
            run_stack,
            read_stack_health,
            read_stack_procs,
            session_spawn,
            session_write,
            session_resize,
            session_kill,
            list_subdirs,
            read_freellmapi_analytics,
            read_providers,
            run_provider,
            list_my_providers,
            save_my_provider,
            delete_my_provider,
            set_freellmapi_auth,
            freellmapi_auth_status,
            connect_my_provider,
            check_my_provider,
            check_provider_url,
            check_provider_balance,
            read_profile_file,
            read_profile_usage,
            add_provider_key,
            remove_provider_key,
            next_provider_key,
            read_opencode,
            run_opencode_provider,
            read_mcp,
            run_mcp,
            list_plugins,
            list_skills,
            list_plugin_updates,
            list_plugin_contents,
            run_plugin,
            delete_skill,
            read_schedules,
            run_schedule,
            read_config,
            write_config,
            export_config,
            import_config,
            app_paths,
            open_path,
            open_terminal,
            get_autostart,
            set_autostart,
            set_toggle_hotkey,
            set_language,
            cancel_run
        ])
        .setup(|app| {
            // Seed the backend locale from config so the tray builds in the right language. The
            // frontend also re-syncs on mount (covers a fresh config with no language yet).
            if let Some(lang) = read_config_file().language {
                set_cur_lang(Lang::parse(&lang));
            }
            build_tray(app.handle())?;
            // One-time brand-rename migration of the autostart Run entry (AgentHub → Castellyn).
            migrate_autostart();
            let cfg = read_config_file();
            // Start minimized to tray if configured.
            if cfg.start_hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            // Register the configured show/hide hotkey, if any. A bad/taken combo must not block startup.
            if let Some(accel) = cfg.toggle_hotkey.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                if let Err(e) = register_toggle_hotkey(app.handle(), accel) {
                    eprintln!("toggle hotkey register failed: {e}");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button minimizes to tray instead of quitting.
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Persist geometry before hiding, so it survives even a later kill (the plugin
                // also saves on a clean exit via the tray "Выход").
                use tauri_plugin_window_state::{AppHandleExt, StateFlags};
                let _ = window.app_handle().save_window_state(StateFlags::all());
                // Default: ✕ minimizes to tray. If the user opted out (closeToTray=false),
                // let the close proceed so the app actually quits.
                if read_config_file().close_to_tray.unwrap_or(true) {
                    api.prevent_close();
                    let _ = window.hide();
                }
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
        let wip = forks_action_args("sync-wip").unwrap();
        assert!(wip.contains(&"-SyncWipLocal".to_string()));
        assert!(wip.contains(&"-Yes".to_string()));
        assert!(forks_action_args("bogus").is_none());
        // "plan" must be a dry-run — never mutating.
        let plan = forks_action_args("plan").unwrap();
        assert!(plan.contains(&"-DryRun".to_string()));
        assert!(!plan.contains(&"-Yes".to_string()));
    }

    #[test]
    fn session_guard_detects_recent_activity() {
        // The cc3-class guard: an idle profile dir reads as not-active; a freshly-written hot file
        // reads as active (so a rebind is refused while a session is likely open).
        let dir = std::env::temp_dir().join(format!("castellyn_sess_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!dir_recently_written(&dir, 120)); // empty → idle
        std::fs::write(dir.join(".claude.json"), "{}").unwrap();
        assert!(dir_recently_written(&dir, 120)); // fresh hot file → looks live
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_bom_tolerant() {
        let v = parse_json_bom("\u{feff}{\"a\":1}").unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(parse_json_bom("{\"b\":2}").unwrap()["b"], 2);
        assert!(parse_json_bom("{not json").is_err());
    }

    #[test]
    fn read_json_opt_missing_is_none() {
        // The shared reader envelope: a missing file is Ok(None), not Err.
        let p = std::env::temp_dir().join(format!("castellyn_nope_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert_eq!(read_json_opt(&p, "nope").unwrap(), None);
        // A present file (with BOM) parses through.
        std::fs::write(&p, "\u{feff}{\"k\":7}").unwrap();
        assert_eq!(read_json_opt(&p, "nope").unwrap().unwrap()["k"], 7);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn forks_actions_all_unattended() {
        // Every fork action runs unattended (no interactive Read-Host hang).
        for a in ["check", "plan", "ff", "delete", "rebase", "sync-wip", "normalize"] {
            let args = forks_action_args(a).unwrap();
            assert!(args.contains(&"-Unattended".to_string()), "{a} must be unattended");
        }
    }

    #[test]
    fn opencode_merge_preserves_other_keys() {
        // Core promise of Manage-OpenCode-Provider: set/clear one provider, leave everything else.
        let path = std::env::temp_dir().join(format!("castellyn_oc_{}.json", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);
        let seed = serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "other/keep-model",
            "provider": {
                "other": { "npm": "@ai-sdk/openai-compatible", "name": "Other",
                           "options": { "baseURL": "http://keep", "apiKey": "sekret" } },
                "tgt":   { "name": "Old", "options": { "baseURL": "http://old", "apiKey": "old" },
                           "models": { "curated": { "name": "curated" } } }
            },
            "agent": { "build": { "model": "other/keep-model" } }
        });
        std::fs::write(&p, serde_json::to_string(&seed).unwrap()).unwrap();
        let noop = |_: &str| {};

        // set tgt → new baseURL + name + model + {env:VAR}; other provider & curated model preserved.
        let code = opencode_provider_native(
            &p, "set", "tgt", Some("New"), "http://new", Some("m1"), None, Some("MY_KEY"), false,
            &noop, &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["provider"]["other"]["options"]["apiKey"], "sekret"); // untouched
        assert_eq!(v["agent"]["build"]["model"], "other/keep-model"); // untouched
        assert_eq!(v["provider"]["tgt"]["name"], "New");
        assert_eq!(v["provider"]["tgt"]["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(v["provider"]["tgt"]["options"]["baseURL"], "http://new");
        assert_eq!(v["provider"]["tgt"]["options"]["apiKey"], "{env:MY_KEY}");
        assert_eq!(v["provider"]["tgt"]["models"]["curated"]["name"], "curated"); // preserved
        assert_eq!(v["provider"]["tgt"]["models"]["m1"]["name"], "m1"); // added
        assert_eq!(v["model"], "tgt/m1"); // active model switched

        // clear tgt → removed; top-level model (now points at tgt) cleared; other provider intact.
        let code = opencode_provider_native(
            &p, "clear", "tgt", None, "", None, None, None, false, &noop, &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v["provider"].get("tgt").is_none());
        assert!(v.get("model").is_none()); // pointed at tgt → removed
        assert_eq!(v["provider"]["other"]["options"]["baseURL"], "http://keep"); // intact
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{p}.bak"));
    }

    #[test]
    fn provider_env_merge_set_and_clear() {
        let path = std::env::temp_dir().join(format!("castellyn_prov_{}.json", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);
        // Seed: unrelated setting + legacy keys that `set` must scrub.
        let seed = serde_json::json!({
            "theme": "dark",
            "env": { "ANTHROPIC_MODEL": "legacy", "ANTHROPIC_SMALL_FAST_MODEL": "legacy-s", "FOO": "bar" }
        });
        std::fs::write(&p, serde_json::to_string(&seed).unwrap()).unwrap();
        let noop = |_: &str| {};

        // set with a token + model (no small) → base/token/tier set, legacy scrubbed, others kept.
        let code = apply_provider_env(
            &p, "cc1", "set", "http://localhost:4000", false, Some("sk-x"),
            Some("glm-4.7"), Some(""), &noop, &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark"); // unrelated setting preserved
        assert_eq!(v["env"]["FOO"], "bar"); // unrelated env preserved
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://localhost:4000");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-x");
        assert_eq!(v["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "glm-4.7");
        assert_eq!(v["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "glm-4.7");
        assert!(v["env"].get("ANTHROPIC_DEFAULT_HAIKU_MODEL").is_none()); // small="" → removed
        assert!(v["env"].get("ANTHROPIC_MODEL").is_none()); // legacy scrubbed
        assert!(v["env"].get("ANTHROPIC_SMALL_FAST_MODEL").is_none());

        // set without a token (keyless endpoint) → dummy bearer, never tokenless.
        let code = apply_provider_env(
            &p, "cc1", "set", "http://localhost:4000", false, Some(""), None, None, &noop, &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "agenthub-local");
        assert_eq!(v["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "glm-4.7"); // model None → untouched

        // clear → all provider keys gone; the empty env block is dropped; unrelated setting kept.
        let code = apply_provider_env(&p, "cc1", "clear", "", false, None, None, None, &noop, &noop);
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        // FOO remained, so env stays; but all ANTHROPIC_* keys are gone.
        for k in PROVIDER_ENV_KEYS {
            assert!(v["env"].get(k).is_none(), "{k} should be cleared");
        }
        assert_eq!(v["env"]["FOO"], "bar");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{p}.bak"));
    }

    #[test]
    fn router_config_upsert_preserves_others() {
        let path = std::env::temp_dir().join(format!("castellyn_ccr_{}.json", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);
        // Seed: an unrelated provider + a stale entry for the name we'll upsert + extra top-level key.
        let seed = serde_json::json!({
            "APIKEY": "keep-me",
            "Providers": [
                { "name": "other", "api_base_url": "http://other/v1/chat/completions", "models": ["x"] },
                { "name": "lmstudio", "api_base_url": "http://STALE", "models": ["old"] }
            ],
            "Router": { "default": "other,x" }
        });
        std::fs::write(&p, serde_json::to_string(&seed).unwrap()).unwrap();
        let noop = |_: &str| {};

        // configure: backend without /chat/completions → normalized; upsert lmstudio; others kept.
        let code =
            apply_router_config(&p, "http://localhost:1234/v1", "qwen", "lmstudio", &noop, &noop);
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["APIKEY"], "keep-me"); // unrelated top-level key preserved
        let provs = v["Providers"].as_array().unwrap();
        assert_eq!(provs.len(), 2); // upsert, not append
        let other = provs.iter().find(|x| x["name"] == "other").unwrap();
        assert_eq!(other["api_base_url"], "http://other/v1/chat/completions"); // untouched
        let lm = provs.iter().find(|x| x["name"] == "lmstudio").unwrap();
        assert_eq!(lm["api_base_url"], "http://localhost:1234/v1/chat/completions"); // normalized
        assert_eq!(lm["api_key"], "not-needed");
        assert_eq!(lm["models"][0], "qwen");
        assert_eq!(v["Router"]["default"], "lmstudio,qwen");

        // A backend already ending in /chat/completions must not be doubled.
        let code = apply_router_config(
            &p, "http://h/v1/chat/completions", "m", "lmstudio", &noop, &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let lm = v["Providers"].as_array().unwrap().iter().find(|x| x["name"] == "lmstudio").unwrap();
        assert_eq!(lm["api_base_url"], "http://h/v1/chat/completions");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{p}.bak"));
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
    fn provider_name_validation() {
        assert!(valid_provider_name("DeepSeek"));
        assert!(valid_provider_name("My Provider 2")); // spaces ok — it's a label
        assert!(!valid_provider_name("")); // empty
        assert!(!valid_provider_name("   ")); // whitespace-only
        assert!(!valid_provider_name("bad\nname")); // control char
        assert!(!valid_provider_name(&"x".repeat(65))); // too long
    }

    #[test]
    fn pty_echo_roundtrip() {
        // Verifies the exact PTY plumbing session_spawn relies on: open a PTY, run a command in it,
        // and read its output back through a cloned reader (the reader-thread pattern).
        use portable_pty::{CommandBuilder, PtySize};
        use std::io::Read;
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");
        let mut cmd = CommandBuilder::new("cmd");
        cmd.arg("/c");
        cmd.arg("echo agenthub-pty-probe");
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let mut out = String::new();
        let mut buf = [0u8; 1024];
        for _ in 0..50 {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    out.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if out.contains("agenthub-pty-probe") {
                        break;
                    }
                }
            }
        }
        let _ = child.wait();
        assert!(out.contains("agenthub-pty-probe"), "pty output was: {out:?}");
    }

    #[test]
    fn session_id_unique_shape() {
        let a = gen_session_id();
        assert!(a.starts_with('s') && a.len() == 16, "id shape: {a}");
    }

    #[test]
    fn stack_id_validation() {
        assert!(valid_stack_id("qwen"));
        assert!(valid_stack_id("glm-kimi"));
        assert!(valid_stack_id("gateway_2"));
        assert!(!valid_stack_id("")); // empty
        assert!(!valid_stack_id("a b")); // space
        assert!(!valid_stack_id("a;rm")); // injection-shaped
        assert!(!valid_stack_id(&"x".repeat(41))); // too long
    }

    #[test]
    fn key_pool_rotation() {
        // wraps around the pool
        assert_eq!(next_key_index(0, 3), 1);
        assert_eq!(next_key_index(2, 3), 0);
        // single key or empty pool → stays at 0 (no-op)
        assert_eq!(next_key_index(0, 1), 0);
        assert_eq!(next_key_index(0, 0), 0);
        // active beyond count still produces a valid in-range successor
        assert_eq!(next_key_index(5, 3), 0);
    }

    #[test]
    fn key_pool_meta_defaults() {
        // no metadata → legacy layout (0, 0)
        assert_eq!(key_pool_meta(&serde_json::json!({})), (0, 0));
        assert_eq!(key_pool_meta(&serde_json::json!({ "keyCount": 3, "activeKey": 2 })), (3, 2));
    }

    #[test]
    fn base_url_validation() {
        assert!(valid_base_url("https://api.deepseek.com/v1").is_ok());
        assert!(valid_base_url("http://localhost:1234").is_ok()); // local engine
        assert!(valid_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(valid_base_url("https://[::1]:8080/v1").is_ok()); // ipv6 literal
        assert!(valid_base_url("ftp://x").is_err()); // bad scheme
        assert!(valid_base_url("api.deepseek.com").is_err()); // no scheme
        assert!(valid_base_url("http://169.254.169.254/latest/meta-data").is_err()); // AWS IMDS
        assert!(valid_base_url("http://100.100.100.200/").is_err()); // Alibaba metadata
        assert!(valid_base_url("http://metadata.google.internal/").is_err()); // GCP metadata
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
