// Castellyn — Tauri backend.
//   * Component manifest (embedded) → run a component's PowerShell script in -Check or -Apply
//     mode, streaming stdout/stderr to the UI.
//   * Single-run guard + cancel.
//   * System tray with minimize-to-tray.
// Paths resolve from $SCRIPTS_ROOT (fallback E:\Scripts) so the app survives a disk move.

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

mod agent_status;
mod limits;
mod i18n;
use i18n::{tr, trv, Lang};

/// Windows CREATE_NO_WINDOW — keep spawned console apps (pwsh/reg/taskkill) from flashing
/// a black console window in front of the GUI.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Assign a spawned PTY child to a process-global Job Object created with KILL_ON_JOB_CLOSE, so the
/// whole tree (pwsh → claude/node, or ssh.exe) is terminated when the app process exits — including a
/// hard crash, where `session_kill` never runs and ConPTY cleanup of grandchildren is only best-effort.
/// The job handle is intentionally never closed: it lives for the app's lifetime and its closure on
/// process death is what triggers the kill. No-op on non-Windows (the app ships Windows-only).
#[cfg(windows)]
fn assign_to_kill_job(pid: u32) {
    use std::sync::OnceLock;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    // Lazily create the single kill-on-close job; store its handle as isize (HANDLE isn't Send/Sync).
    static KILL_JOB: OnceLock<isize> = OnceLock::new();
    let jobval = *KILL_JOB.get_or_init(|| unsafe {
        let job = match CreateJobObjectW(None, windows::core::PCWSTR::null()) {
            Ok(h) => h,
            Err(_) => return 0,
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job.0 as isize
    });
    if jobval == 0 {
        return;
    }
    unsafe {
        let job = HANDLE(jobval as *mut core::ffi::c_void);
        if let Ok(hproc) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) {
            let _ = AssignProcessToJobObject(job, hproc);
            let _ = CloseHandle(hproc);
        }
    }
}

#[cfg(not(windows))]
fn assign_to_kill_job(_pid: u32) {}

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

/// Per-harness list of MCP server NAMES that Castellyn deployed (item Gap-2). Deploy uses it to
/// reconcile: a name we deployed before but that's no longer in canon (.mcp.json) is removed from the
/// harness; a user-added server is never in this list, so it's never touched.
/// LIMITATION (accepted): the ledger is name-keyed, so if Castellyn deployed name N, N is later
/// dropped from canon, and the user independently adds their OWN server also named N, the next deploy
/// removes it. A name collision on a de-canonized name is the (rare) cost of a name-based ledger.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct ManagedMcp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opencode: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex: Option<Vec<String>>,
}

/// Persistent hub settings (%APPDATA%\castellyn\config.json).
#[derive(Serialize, Deserialize, Default, Clone)]
struct HubConfig {
    #[serde(
        rename = "scriptsRoot",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    scripts_root: Option<String>,
    #[serde(rename = "startHidden", default)]
    start_hidden: bool,
    // None = default (true): the ✕ button hides to tray. false = ✕ actually quits the app.
    #[serde(
        rename = "closeToTray",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    close_to_tray: Option<bool>,
    #[serde(
        rename = "fetchTimeoutSec",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    fetch_timeout_sec: Option<u32>,
    #[serde(
        rename = "ghTimeoutSec",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    gh_timeout_sec: Option<u32>,
    // OS-level accelerator (e.g. "CommandOrControl+Shift+H") that toggles the window. None/empty = off.
    #[serde(
        rename = "toggleHotkey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    toggle_hotkey: Option<String>,
    // Full action → accelerator mapping. Keys: "toggle_window" etc. Empty map = no shortcuts.
    // Supersedes toggleHotkey (which is kept for backward compat).
    #[serde(rename = "shortcuts", default, skip_serializing_if = "Option::is_none")]
    shortcuts: Option<HashMap<String, String>>,
    // UI locale ("ru"/"en"/"zh") mirrored from the frontend so the backend (errors, log, tray) can
    // localize too. Owned by set_language; write_config preserves it (never clobbered by a settings save).
    #[serde(rename = "language", default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    // Agent-status notifications (Sessions): None = default (true). Sounds are system
    // beeps (MessageBeep); notify = OS toast on →blocked and background completion.
    #[serde(
        rename = "statusSounds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    status_sounds: Option<bool>,
    #[serde(
        rename = "statusNotify",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    status_notify: Option<bool>,
    // Anthropic OAuth usage-limit monitor (Sessions): None = default (true). Polls each profile's
    // usage every 5 min and alerts at 85% / 99%. Set false to stop the background api.anthropic.com poll.
    #[serde(
        rename = "limitsMonitor",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    limits_monitor: Option<bool>,
    // Auto-continue a limited Claude pane once its 5h window resets (Sessions, item 21c). None =
    // default (true). No UI toggle — a config-only escape hatch for rollback if unattended auto-input
    // is unwanted. The whole loop lives in the frontend (SessionsTab); the backend only persists it.
    #[serde(
        rename = "autoContinueOnReset",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    auto_continue_on_reset: Option<bool>,
    // After-limit behaviour (Sessions, item 21e): "wait" (default) keeps the pane on its profile and
    // auto-continues on reset (21c); "switchProfile" respawns the conversation under a free OAuth
    // profile immediately. Persisted here; the whole loop lives in the frontend.
    #[serde(
        rename = "limitMode",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    limit_mode: Option<String>,
    // Gap-2: MCP server names Castellyn has deployed to each harness — the reconcile ledger (see
    // ManagedMcp). Written by the OpenCode/Codex MCP fan-out; not user-facing.
    #[serde(
        rename = "managedMcp",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    managed_mcp: Option<ManagedMcp>,
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
    // Recover from <path>.bak if the live config is corrupt, so a damaged config.json silently
    // restores instead of resetting every setting to defaults (and the next save overwriting them).
    let v = read_json_or_recover(&p, "config.json").ok().flatten()?;
    serde_json::from_value(v).ok()
}

/// Cached parse of config.json, invalidated by write_config_file (the single writer) so a settings
/// save / language change is reflected at once — while hot paths (scripts_root → abs →
/// expand_placeholders, list_components, run_forks) avoid a disk read + serde parse on every call.
static CONFIG_CACHE: std::sync::RwLock<Option<HubConfig>> = std::sync::RwLock::new(None);

fn read_config_file() -> HubConfig {
    if let Some(c) = CONFIG_CACHE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        return c.clone();
    }
    let mut cfg = read_config_at(config_path())
        .or_else(|| read_config_at(agenthub_config_path()))
        .or_else(|| read_config_at(legacy_config_path()))
        .unwrap_or_default();
    // Migrate toggleHotkey → shortcuts map (one-time: first read after upgrade populates the map).
    if cfg.shortcuts.is_none() {
        let mut m = HashMap::new();
        if let Some(hk) = cfg
            .toggle_hotkey
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            m.insert("toggle_window".to_string(), hk.to_string());
        }
        cfg.shortcuts = if m.is_empty() { None } else { Some(m) };
    }
    *CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(cfg.clone());
    cfg
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
    s.replace("{{SCRIPTS_ROOT}}", &scripts_root())
        .replace("{{USERPROFILE}}", &home)
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
    // A corrupt on-disk manifest (bad JSON) must not silently blank the dashboard — fall back to the
    // embedded copy on a PARSE error too, not just a read error (manifest_text handles missing file).
    serde_json::from_str::<RawManifest>(&manifest_text())
        .or_else(|_| serde_json::from_str::<RawManifest>(MANIFEST_FALLBACK))
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

/// Like read_json_opt but, when the file is present yet corrupt/unreadable, transparently falls back
/// to the `<path>.bak` durable backup that write_json_atomic leaves behind. Returns Err ONLY when the
/// main file is bad AND no usable .bak exists — the signal for a mutating caller to ABORT instead of
/// overwriting (which would turn a corrupt file into a permanent wipe). NotFound -> Ok(None).
fn read_json_or_recover(
    path: impl AsRef<std::path::Path>,
    label: &str,
) -> Result<Option<serde_json::Value>, String> {
    let path = path.as_ref();
    match read_json_opt(path, label) {
        Ok(v) => Ok(v),
        Err(main_err) => match read_json_opt(format!("{}.bak", path.display()), label) {
            Ok(Some(v)) => Ok(Some(v)),
            _ => Err(main_err),
        },
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

/// RAII guard for the single global run slot. `reserve` claims it (Err if a run is already in
/// progress); `Drop` ALWAYS clears it back to None — so a panic or early return anywhere on the run
/// path can't wedge the slot into a permanent "run in progress" that no cancel can clear (the old
/// hand-reset after the await never ran if the future panicked/was dropped).
struct RunSlot<'a>(&'a RunState);
impl<'a> RunSlot<'a> {
    fn reserve(state: &'a RunState) -> Result<Self, String> {
        let mut g = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return Err(tr("err.run_in_progress", cur_lang()).into());
        }
        *g = Some(0);
        Ok(RunSlot(state))
    }
    /// Record the real child pid so cancel_run can target it (slot still resets on drop).
    fn set_pid(&self, pid: u32) {
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    }
}
impl Drop for RunSlot<'_> {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

// Per-repo fork runs (path -> pid). Lets each fork update run concurrently and independently,
// keyed by repo path, without the single RunState slot blocking the whole Forks tab.
#[derive(Default)]
struct ForkRuns(Mutex<std::collections::HashMap<String, u32>>);

// True while a GLOBAL run_forks sweep is in flight. The global sweep and per-repo runs are two
// separate concurrency domains; without this flag they could `git fetch` the SAME repo at once and
// one would die on git's `.lock` file. Used for mutual exclusion (global ⟷ any per-repo), not to
// serialize per-repo runs against each other (that stays independent, by design).
static FORKS_GLOBAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// F17: bulk plugin ops run in their OWN domain (not the single RunState slot) so a 10-plugin sweep
// doesn't block unrelated backup/forks work. ACTIVE rejects a second concurrent bulk; CANCEL is the
// between-items kill switch (set by cancel_all). Plugin mutations stay SEQUENTIAL inside the run —
// concurrent `claude plugin` writes would race the shared ~/.claude config.
static BULK_PLUGINS_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static BULK_PLUGINS_CANCEL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// M1: "Repair All" profiles runs each profile as a sequential spawn_pwsh_phase under one RunSlot.
// cancel_run/cancel_all only kill the CURRENT child's pid, so without this between-items flag the
// loop marches through every remaining profile after a Cancel. Mirrors BULK_PLUGINS_CANCEL.
static PROFILES_BULK_CANCEL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// RAII reservation for the bulk-plugin domain (mirrors RunSlot): `Drop` ALWAYS clears ACTIVE, so a
/// dropped command future (e.g. webview reload mid-sweep) can't wedge it into a permanent "busy".
struct BulkSlot;
impl BulkSlot {
    fn reserve() -> Option<Self> {
        if BULK_PLUGINS_ACTIVE.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(BulkSlot)
        }
    }
}
impl Drop for BulkSlot {
    fn drop(&mut self) {
        BULK_PLUGINS_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// RAII for the GLOBAL forks-sweep flag (mirrors RunSlot/BulkSlot): `reserve` sets FORKS_GLOBAL true;
/// `Drop` ALWAYS stores false — so a dropped `run_forks` future (webview reload mid-await) can't wedge
/// the whole Forks tab into a permanent "busy". Keeps the original ordering: the flag is set FIRST,
/// then `run_forks` checks the per-repo map and returns Err (letting Drop clear) if any repo is active.
struct ForksGlobalSlot;
impl ForksGlobalSlot {
    fn reserve() -> Self {
        FORKS_GLOBAL.store(true, Ordering::SeqCst);
        ForksGlobalSlot
    }
}
impl Drop for ForksGlobalSlot {
    fn drop(&mut self) {
        FORKS_GLOBAL.store(false, Ordering::SeqCst);
    }
}

/// RAII reservation of one repo path in the ForkRuns map (mirrors the guards above): `reserve` runs the
/// locked global-busy + already-running checks and inserts `path`→0; `set_pid` records the child pid so
/// cancel_fork_repo can target it; `Drop` ALWAYS removes the path — so a dropped `run_fork_repo` future
/// (or its spawn-error path) can't strand a repo as permanently "busy".
struct ForkRepoSlot<'a> {
    runs: &'a ForkRuns,
    path: String,
}
impl<'a> ForkRepoSlot<'a> {
    fn reserve(runs: &'a ForkRuns, path: String) -> Result<Self, String> {
        let mut m = runs.0.lock().unwrap_or_else(|e| e.into_inner());
        // A global run_forks sweep is in flight — it processes every repo, so running this one now would
        // double-fetch it. Reject (the residual set-vs-check race is backstopped by git's .lock).
        if FORKS_GLOBAL.load(Ordering::SeqCst) {
            return Err(tr("err.fork_busy", cur_lang()).into());
        }
        if m.contains_key(&path) {
            return Err(tr("err.fork_busy", cur_lang()).into());
        }
        m.insert(path.clone(), 0);
        drop(m);
        Ok(ForkRepoSlot { runs, path })
    }
    /// Record the real child pid so cancel_fork_repo can target it (slot still clears on drop).
    fn set_pid(&self, pid: u32) {
        self.runs
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.path.clone(), pid);
    }
}
impl Drop for ForkRepoSlot<'_> {
    fn drop(&mut self) {
        self.runs
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.path);
    }
}

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

/// The pwsh launcher prefix every script spawn shares: `-NoProfile -ExecutionPolicy Bypass
/// -File <script>` followed by the script's own args. One definition so the contract can't drift.
fn pwsh_file_args(script: String, args: Vec<String>) -> Vec<String> {
    let mut full = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script,
    ];
    full.extend(args);
    full
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
    let full = pwsh_file_args(script, args);
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
    // ponytail (R1-02): between this `Some(0)` reservation and `child.id()` setting the real pid
    // below, the run can't be cancelled — cancel_run treats pid 0 (and None) as "no active run", and
    // the real pid simply doesn't exist until cmd.spawn() returns. Closing this sub-spawn window
    // cleanly would need a cancel-flag the spawn re-checks (then kills the just-spawned child), i.e.
    // extra RunState and a kill path that races spawn — risk to a working cancel for a window of a
    // few ms before any process exists. Deliberately left as-is; cancel works the instant a pid lands.
    let slot = RunSlot::reserve(state.inner())?;

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
            return Err(trv(
                "err.spawn_failed",
                cur_lang(),
                &[("program", &program), ("e", &e)],
            ));
        }
    };

    if let Some(pid) = child.id() {
        slot.set_pid(pid);
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
    drop(slot); // release the single run slot (also happens on any early return / panic above)
    Ok(code)
}

/// Run one pwsh script under an ALREADY-reserved run slot, streaming to "run-log" and emitting
/// `done_event` at the end. Pass an event the UI ignores (e.g. "run-restart-stop") to suppress a
/// premature run-done — the two-phase stack restart uses this so only the final phase signals
/// completion. Sets the live PID so cancel_run can kill whichever phase is running.
async fn spawn_pwsh_phase(
    app: &AppHandle,
    state: &State<'_, RunState>,
    id: &str,
    script: String,
    args: Vec<String>,
    done_event: &'static str,
) -> i32 {
    let full = pwsh_file_args(script, args);
    let mut cmd = Command::new("pwsh");
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
            // L1: surface the spawn failure like spawn_streamed_prog's sibling path (err.spawn_failed).
            // A bulk Repair-All spawn failure was previously silent (`Err(_) => return -1`).
            let program = "pwsh";
            let _ = app.emit(
                "run-log",
                LogLine {
                    component: id.to_string(),
                    stream: "err".into(),
                    line: trv("err.spawn_failed", cur_lang(), &[("program", &program), ("e", &e)]),
                },
            );
            return -1;
        }
    };
    if let Some(pid) = child.id() {
        *state.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    }
    pump_and_wait(app.clone(), id.to_string(), child, "run-log", done_event).await
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
        handles.push(tokio::spawn(pump_stream(
            app.clone(),
            id.clone(),
            log_event,
            "out",
            BufReader::new(stdout).lines(),
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        handles.push(tokio::spawn(pump_stream(
            app.clone(),
            id.clone(),
            log_event,
            "err",
            BufReader::new(stderr).lines(),
        )));
    }
    let status = child.wait().await;
    // Await the pumps so their final coalesced flush lands BEFORE run-done — no lost tail lines.
    for h in handles {
        let _ = h.await;
    }
    let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
    let _ = app.emit(
        done_event,
        RunDone {
            component: id,
            code,
        },
    );
    code
}

/// Coalesce a FIFO batch of one stream's buffered lines into a single run-log payload (joined by
/// '\n'). Pure + extracted so the batching/ordering contract the frontend splits on stays tested.
fn coalesce_batch(lines: &[String]) -> String {
    lines.join("\n")
}

/// Emit one coalesced run-log event for a stream's buffered lines, then clear the buffer.
fn flush_batch(
    app: &AppHandle,
    log_event: &str,
    id: &str,
    stream: &'static str,
    batch: &mut Vec<String>,
) {
    if batch.is_empty() {
        return;
    }
    let _ = app.emit(
        log_event,
        LogLine {
            component: id.to_string(),
            stream: stream.to_string(),
            line: coalesce_batch(batch),
        },
    );
    batch.clear();
}

/// Pump ONE stream, coalescing rapid lines into a single run-log event flushed at ~30ms cadence (or
/// once the buffer hits MAX_BATCH lines) instead of one IPC event per line. Per-stream batching keeps
/// each stream's lines in FIFO order and never interleaves stdout/stderr within an event. Cross-stream
/// merge into one arrival-ordered event would need an mpsc/select! path (tokio `sync` isn't enabled);
/// separate OS pipes carry no cross-pipe ordering guarantee anyway, so per-stream is the honest unit.
/// `Lines::next_line` is cancellation-safe (its partial-line buffer lives in `Lines`, not the future),
/// so wrapping it in `timeout_at` cannot drop a line.
async fn pump_stream<R>(
    app: AppHandle,
    id: String,
    log_event: &'static str,
    stream: &'static str,
    mut lines: tokio::io::Lines<R>,
) where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
{
    const FLUSH_MS: u64 = 30;
    const MAX_BATCH: usize = 64;
    let mut batch: Vec<String> = Vec::new();
    // Deadline for the current (non-empty) batch: flush FLUSH_MS after its FIRST line, bounding
    // console latency regardless of how steadily lines arrive.
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let read = match deadline {
            None => lines.next_line().await,
            Some(dl) => match tokio::time::timeout_at(dl, lines.next_line()).await {
                Ok(r) => r,
                Err(_) => {
                    flush_batch(&app, log_event, &id, stream, &mut batch);
                    deadline = None;
                    continue;
                }
            },
        };
        match read {
            Ok(Some(line)) => {
                batch.push(line);
                if deadline.is_none() {
                    deadline = Some(
                        tokio::time::Instant::now() + std::time::Duration::from_millis(FLUSH_MS),
                    );
                }
                if batch.len() >= MAX_BATCH {
                    flush_batch(&app, log_event, &id, stream, &mut batch);
                    deadline = None;
                }
            }
            // EOF (None) or a read error: flush the tail and stop.
            _ => {
                flush_batch(&app, log_event, &id, stream, &mut batch);
                break;
            }
        }
    }
}

#[cfg(test)]
mod batch_tests {
    use super::coalesce_batch;

    #[test]
    fn coalesce_preserves_order_and_round_trips() {
        // Empty batch → empty payload (flush_batch skips it, but the join must not panic).
        assert_eq!(coalesce_batch(&[]), "");
        // A single line passes through unchanged.
        assert_eq!(coalesce_batch(&["only".to_string()]), "only");
        // Multiple lines join with '\n' in FIFO order; splitting recovers them exactly — the frontend
        // relies on this to re-explode one coalesced run-log event back into ordered rows.
        let lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let payload = coalesce_batch(&lines);
        assert_eq!(payload, "a\nb\nc");
        assert_eq!(payload.split('\n').collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }
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
    let _slot = RunSlot::reserve(state.inner())?;
    let app_job = app.clone();
    let id_job = id.clone();
    let code = tokio::task::spawn_blocking(move || {
        let out = |line: &str| {
            let _ = app_job.emit(
                "run-log",
                LogLine {
                    component: id_job.clone(),
                    stream: "out".into(),
                    line: line.to_string(),
                },
            );
        };
        let err = |line: &str| {
            let _ = app_job.emit(
                "run-log",
                LogLine {
                    component: id_job.clone(),
                    stream: "err".into(),
                    line: line.to_string(),
                },
            );
        };
        job(&out, &err)
    })
    .await
    .unwrap_or(-1);
    drop(_slot); // release the single run slot (also released on early return / panic above)
    let _ = app.emit(
        "run-done",
        RunDone {
            component: id,
            code,
        },
    );
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
        .ok_or_else(|| trv("err.unknown_component", cur_lang(), &[("id", &id)]))?;

    let args = if mode == "apply" {
        if !comp.supports_apply {
            return Err(trv(
                "err.component_no_apply",
                cur_lang(),
                &[("name", &comp.name)],
            ));
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
            "-FfMain",
            "-DeleteMerged",
            "-NormalizeRemotes",
            "-Rebase",
            "-DryRun",
            "-Unattended",
        ],
        "ff" => vec!["-FfMain", "-Yes", "-Unattended"],
        "delete" => vec!["-DeleteMerged", "-Yes", "-Unattended"],
        "rebase" => vec!["-Rebase", "-Yes", "-Unattended"],
        "sync-wip" => vec!["-SyncWipLocal", "-Yes", "-Unattended"],
        "delete-wip" => vec!["-DeleteWip", "-Yes", "-Unattended"],
        "prune" => vec!["-Prune", "-Yes", "-Unattended"],
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
    let dir = std::path::Path::new(&script)
        .parent()?
        .to_string_lossy()
        .to_string();
    // A short readable hint + a stable hash of the FULL path. The old "every non-alphanumeric → '_'"
    // map collapsed distinct paths (Cyrillic, or `a-b` vs `a_b`) onto one file → concurrent per-repo
    // runs raced it. DefaultHasher::new() has fixed keys, so the hash is stable across processes —
    // read_fork_repo_status recomputes the same name. (Same-volume: dir is the script's own folder.)
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    let hint: String = path
        .chars()
        .rev()
        .take_while(|c| *c != '\\' && *c != '/')
        .collect::<String>();
    let hint: String = hint
        .chars()
        .rev()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect();
    Some(format!(
        "{dir}\\fork-sync.{hint}.{:016x}.last.json",
        h.finish()
    ))
}

/// Run a Forks-tab action. `path` (a repo path) scopes the action to one repo via -Paths;
/// omit it for the global read actions (check / plan).
#[tauri::command]
async fn run_forks(
    app: AppHandle,
    state: State<'_, RunState>,
    runs: State<'_, ForkRuns>,
    action: String,
    path: Option<String>,
) -> Result<i32, String> {
    let comp = raw_components()
        .into_iter()
        .find(|c| c.id == "forks")
        .ok_or(tr("err.forks_missing", cur_lang()))?;
    let mut args = forks_action_args(&action).ok_or_else(|| {
        trv(
            "err.unknown_forks_action",
            cur_lang(),
            &[("action", &action)],
        )
    })?;
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
    // Claim the global slot (Drop clears it even if this future is dropped mid-await), then bail if any
    // per-repo run is active (would `git fetch` the same repo concurrently). Order: the flag is set first
    // so a per-repo run starting now sees it.
    let _global = ForksGlobalSlot::reserve();
    if !runs.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        return Err(tr("err.fork_busy", cur_lang()).to_string());
    }
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
    let mut args = forks_action_args(&action).ok_or_else(|| {
        trv(
            "err.unknown_forks_action",
            cur_lang(),
            &[("action", &action)],
        )
    })?;
    if !std::path::Path::new(&path).is_dir() {
        return Err(trv("err.repo_dir_missing", cur_lang(), &[("path", &path)]));
    }
    let comp = raw_components()
        .into_iter()
        .find(|c| c.id == "forks")
        .ok_or(tr("err.forks_missing", cur_lang()))?;
    let script = abs_with_fallback(&comp.script_rel, FORKS_SCRIPT_FALLBACK);
    // Reserve this repo (Drop removes it even if this future is dropped mid-run). Rejects a second run
    // on the same repo, or any run while a global sweep is in flight.
    let slot = ForkRepoSlot::reserve(&runs, path.clone())?;
    // Strict single-repo run: only this repo is processed, and its result is written to a per-repo
    // JSON (not the shared fork-sync.last.json) — so concurrent repo runs never race the file.
    let out_file = fork_repo_out_file(&path).unwrap_or_default();
    // `args` (from forks_action_args) already carries -Unattended for every action — don't repeat it
    // here, or pwsh fails with "parameter 'Unattended' specified more than once".
    let mut full = vec![
        "-Single".to_string(),
        path.clone(),
        "-OutFile".to_string(),
        out_file,
    ];
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
    for a in &pwsh_file_args(script, full) {
        cmd.arg(a);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.env("SCRIPTS_ROOT", scripts_root());
    cmd.creation_flags(CREATE_NO_WINDOW);
    let child = match cmd.spawn() {
        Ok(c) => c,
        // `slot` Drop removes the path on this early return.
        Err(e) => return Err(trv("err.pwsh_failed", cur_lang(), &[("e", &e)])),
    };
    if let Some(pid) = child.id() {
        slot.set_pid(pid);
    }
    let code = pump_and_wait(app, path.clone(), child, "fork-log", "fork-done").await;
    // `slot` Drop removes the path when this function returns.
    Ok(code)
}

/// taskkill /T /F a process tree by PID. Exit 128 = "process not found" (already exited) is benign;
/// any other failure (e.g. a non-elevated app can't kill an elevated child → Access denied) is
/// surfaced instead of a false Ok. Shared by cancel_run / cancel_fork_repo / measure_context timeout.
fn kill_tree(pid: u32) -> Result<(), String> {
    match std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(o) if o.status.success() || o.status.code() == Some(128) => Ok(()),
        Ok(o) => {
            let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(trv("err.kill_failed", cur_lang(), &[("e", &msg)]))
        }
        Err(e) => Err(trv("err.kill_failed", cur_lang(), &[("e", &e)])),
    }
}

/// Cancel the in-flight fork run for `path` (kills its process tree). No-op if none is running.
#[tauri::command]
fn cancel_fork_repo(runs: State<'_, ForkRuns>, path: String) -> Result<(), String> {
    let pid = {
        runs.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&path)
            .copied()
    };
    if let Some(p) = pid {
        if p != 0 {
            kill_tree(p)?;
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
    v.get("repos")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .cloned()
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
    d(0) && d(1)
        && d(2)
        && d(3)
        && b[4] == b'-'
        && d(5)
        && d(6)
        && b[7] == b'-'
        && d(8)
        && d(9)
        && b[10] == b'_'
        && d(11)
        && d(12)
        && d(13)
        && d(14)
        && d(15)
        && d(16)
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
    BackupList {
        snapshots,
        weeklies,
        state,
    }
}

/// Validate a weekly-archive name (no path separators, `weekly-*.zip` shape) and return its absolute
/// path under Backups. Shared by reveal/verify/extract/delete so the guard can't drift between them.
fn weekly_archive_path(name: &str) -> Result<String, String> {
    if name.contains('/')
        || name.contains('\\')
        || !(name.starts_with("weekly-") && name.ends_with(".zip"))
    {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &name)]));
    }
    let path = format!("{}\\{}", abs(BACKUP_DIR_REL), name);
    if !std::path::Path::new(&path).exists() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &path)]));
    }
    Ok(path)
}

/// The SYSTEM bsdtar — a bare `tar` may resolve to Git-Bash GNU tar, which treats the `E:` in the
/// archive path as a remote host ("Cannot connect to E:"). Same reason the Backup script uses it.
fn system_tar() -> String {
    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{windir}\\System32\\tar.exe")
}

/// F9: reveal a weekly archive in Explorer (file selected).
#[tauri::command]
fn reveal_backup(name: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| trv("err.open_path", cur_lang(), &[("path", &path), ("e", &e)]))?;
    Ok(())
}

/// F9: delete a weekly archive (zip). The FE gates this behind a confirm.
#[tauri::command]
fn delete_backup(name: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    std::fs::remove_file(&path)
        .map_err(|e| trv("err.open_path", cur_lang(), &[("path", &path), ("e", &e)]))
}

/// F9: verify a weekly archive by listing it (`tar -tf`). Returns the entry count on success, or the
/// tar stderr if the zip is corrupt/truncated.
#[tauri::command]
fn verify_backup(name: String) -> Result<usize, String> {
    let path = weekly_archive_path(&name)?;
    let out = std::process::Command::new(system_tar())
        .args(["-tf", &path])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.tar_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).lines().count())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// F9: extract a weekly archive to a user-picked folder. NON-destructive — never writes over the live
/// ~/.claude (the weekly archives skills/agents/commands, which are Syncthing-synced + junctioned).
#[tauri::command]
fn extract_backup(name: String, dest: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    if !std::path::Path::new(&dest).is_dir() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &dest)]));
    }
    let out = std::process::Command::new(system_tar())
        .args(["-x", "-f", &path, "-C", &dest])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.tar_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Build the (script, args) for a Backup-tab action. Kept pure so the security-sensitive gating is
/// unit-testable: credentials ride along ONLY on a real `restore` with the explicit flag, a preview
/// is always `-WhatIf` (non-destructive), `-KeepSnapshots` never drops below 1, and an unknown
/// action errors instead of silently running a script.
fn backup_args(
    action: &str,
    timestamp: Option<String>,
    profiles: Option<Vec<String>>,
    include_credentials: Option<bool>,
    keep_snapshots: Option<u32>,
) -> Result<(&'static str, Vec<String>), String> {
    let (script_rel, mut args): (&'static str, Vec<String>) = match action {
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
        "delete-snapshot" => match timestamp.as_deref().filter(|t| !t.is_empty()) {
            // Must carry a non-empty id: an empty -DeleteSnapshot would make the script run a normal
            // backup instead. The PS side additionally pattern-validates the id against traversal.
            Some(id) => (
                BACKUP_SCRIPT_REL,
                vec!["-DeleteSnapshot".to_string(), id.to_string()],
            ),
            None => return Err("delete-snapshot requires a snapshot id".to_string()),
        },
        _ => {
            return Err(trv(
                "err.unknown_backup_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
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
    Ok((script_rel, args))
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
    let (script_rel, args) = backup_args(
        &action,
        timestamp,
        profiles,
        include_credentials,
        keep_snapshots,
    )?;
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

/// Whether THIS process is running elevated (admin). Cached — elevation can't change at runtime.
/// Uses the canonical .NET WindowsPrincipal check via pwsh (no extra crate); CREATE_NO_WINDOW.
fn is_elevated() -> bool {
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ELEVATED.get_or_init(|| {
        let script = "[bool]([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)";
        std::process::Command::new("pwsh")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

/// Read the cached profiles health snapshot (profiles.last.json). Null until first check.
#[tauri::command]
fn read_profiles() -> Result<Option<serde_json::Value>, String> {
    let mut out = read_json_opt(abs(PROFILES_JSON_REL), "profiles.last.json")?;
    // The external status script's `isAdmin` goes stale after an elevated relaunch (it reflects the
    // run that LAST wrote the file). Override it with a live native check so the UI sees the current
    // process's real privileges immediately — no need to re-run the status script first.
    if let Some(serde_json::Value::Object(map)) = out.as_mut() {
        map.insert("isAdmin".into(), serde_json::json!(is_elevated()));
    }
    Ok(out)
}

/// Run a Profiles-tab action: refresh status, clean sync-conflict files, reinstall all profiles,
/// create a single missing profile (`create`), or repair the links of a single profile (`repair`).
/// `create` and `repair` require `name`.
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
            // Charset gate (R1-04): mirror the elevated sibling repair_profile_elevated — validate
            // the name's charset before the membership check, since profile_names() reads names
            // verbatim from profiles.json (no charset guarantee) and `n` becomes a -Name argv.
            if !valid_profile_name(&n) {
                return Err(trv("err.invalid_profile_name", cur_lang(), &[("name", &n)]));
            }
            if !profile_names().iter().any(|x| x == &n) {
                return Err(trv("err.unknown_profile", cur_lang(), &[("name", &n)]));
            }
            (REPAIR_SCRIPT_REL, vec!["-Name".to_string(), n])
        }
        "create" => {
            let n = name.unwrap_or_default();
            if !valid_profile_name(&n) {
                return Err(trv("err.invalid_profile_name", cur_lang(), &[("name", &n)]));
            }
            if !profile_names().iter().any(|x| x == &n) {
                return Err(trv("err.unknown_profile", cur_lang(), &[("name", &n)]));
            }
            // Rust-native dir creation (no admin) so a single missing profile can be created without a
            // full -Force reinstall that re-touches every profile and re-runs the global CLI/RTK steps.
            // Repair-ProfileLinks then makes just this profile's shared-folder links (folder symlinks
            // need admin → surfaced as broken-links + the existing one-UAC elevated repair afterwards).
            let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
            let dir = format!("{home}\\.claude-{n}");
            std::fs::create_dir_all(&dir).map_err(|e| format!("create {dir}: {e}"))?;
            (REPAIR_SCRIPT_REL, vec!["-Name".to_string(), n])
        }
        _ => {
            return Err(trv(
                "err.unknown_profiles_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
    };
    let script = abs(script_rel);
    spawn_streamed(app, state, "profiles".to_string(), script, args).await
}

/// F23: repair the shared-folder links of the given profiles in one run (Home "Repair All"). Loops
/// the single-profile repair script under ONE reserved slot (the per-profile script has no -All mode),
/// streaming a header per profile; the final run-done carries the WORST exit code so a failure in any
/// profile surfaces. Each name is charset-validated + membership-checked (repair is idempotent + never
/// deletes real data, so repairing an already-healthy profile is a safe no-op).
#[tauri::command]
async fn repair_all_profiles(
    app: AppHandle,
    state: State<'_, RunState>,
    names: Vec<String>,
) -> Result<i32, String> {
    let known = profile_names();
    let targets: Vec<String> = names
        .into_iter()
        .filter(|n| valid_profile_name(n) && known.iter().any(|k| k == n))
        .collect();
    let _slot = RunSlot::reserve(state.inner())?;
    PROFILES_BULK_CANCEL.store(false, Ordering::SeqCst);
    let script = abs(REPAIR_SCRIPT_REL);
    let mut worst = 0;
    for name in &targets {
        let _ = app.emit(
            "run-log",
            LogLine {
                component: "profiles".into(),
                stream: "out".into(),
                line: format!("── repair {name} ──"),
            },
        );
        // Intermediate phases emit an event the UI ignores; the real run-done is sent below with `worst`.
        let code = spawn_pwsh_phase(
            &app,
            &state,
            "profiles",
            script.clone(),
            vec!["-Name".into(), name.clone()],
            "run-profiles-phase",
        )
        .await;
        if code != 0 {
            worst = code;
        }
        // M1: honor a mid-run Cancel — cancel_run/cancel_all killed the current child and set the
        // flag; stop instead of marching through the remaining profiles (worst already reflects it).
        if PROFILES_BULK_CANCEL.load(Ordering::SeqCst) {
            break;
        }
    }
    drop(_slot);
    let _ = app.emit(
        "run-done",
        RunDone {
            component: "profiles".into(),
            code: worst,
        },
    );
    Ok(worst)
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
        _ => {
            return Err(trv(
                "err.unknown_configdrift_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
    };
    let script = abs(script_rel);
    spawn_streamed(app, state, "sync".to_string(), script, args).await
}

// --- Config-drift diff (Phase 3.2) ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum DiffLineKind {
    Add,
    Del,
    Same,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiffLine {
    kind: DiffLineKind,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DriftDiff {
    tip_path: String,
    source_path: String,
    source_lines: usize,
    tip_lines: usize,
    lines: Vec<DiffLine>,
}

/// LCS-based line diff. `a` is the reference (source), `b` is the changed file (tip).
/// Returns diff lines with add/del/same markers. O(m*n) time+mem — config files are tiny.
fn compute_diff(a: &[String], b: &[String]) -> Vec<DiffLine> {
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }
    let mut result = Vec::with_capacity(m + n);
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            result.push(DiffLine {
                kind: DiffLineKind::Same,
                text: a[i - 1].clone(),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            result.push(DiffLine {
                kind: DiffLineKind::Add,
                text: b[j - 1].clone(),
            });
            j -= 1;
        } else {
            result.push(DiffLine {
                kind: DiffLineKind::Del,
                text: a[i - 1].clone(),
            });
            i -= 1;
        }
    }
    result.reverse();
    result
}

const CONFIG_SOURCE_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config";

/// Read a unified diff between a drifted live file and its shared config copy.
/// `name` is the filename (e.g. "statusline.py").
/// Returns null if either file is missing.
#[tauri::command]
fn read_drift_diff(name: String) -> Result<Option<DriftDiff>, String> {
    let home = std::env::var("USERPROFILE").map_err(|_| "no USERPROFILE".to_string())?;
    let tip_path = format!("{}\\.claude\\{}", home, name);
    let source_path = format!("{}\\{}\\{}", scripts_root(), CONFIG_SOURCE_REL, name);

    let tip_content = match std::fs::read_to_string(&tip_path) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let source_content = match std::fs::read_to_string(&source_path) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };

    let tip_lines: Vec<String> = tip_content.lines().map(String::from).collect();
    let source_lines: Vec<String> = source_content.lines().map(String::from).collect();

    let lines = compute_diff(&source_lines, &tip_lines);

    Ok(Some(DriftDiff {
        tip_path,
        source_path,
        source_lines: source_lines.len(),
        tip_lines: tip_lines.len(),
        lines,
    }))
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
        && s.chars()
            .next()
            .map(|c| c.is_ascii_alphanumeric())
            .unwrap_or(false)
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Profile lifecycle: add / remove / rename / recolor / redescribe / set-links via Manage-Profiles.ps1.
#[tauri::command]
// command handler: args come from the JS invoke boundary
#[allow(clippy::too_many_arguments)]
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
        return Err(trv(
            "err.invalid_profile_name",
            cur_lang(),
            &[("name", &name)],
        ));
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
                return Err(trv("err.invalid_new_name", cur_lang(), &[("nn", &nn)]));
            }
            args.push("-NewName".into());
            args.push(nn);
        }
        "recolor" => {
            args.push("-Color".into());
            args.push(color.ok_or(tr("err.no_color", cur_lang()))?);
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
        _ => {
            return Err(trv(
                "err.unknown_profile_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
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
        return Err(trv(
            "err.invalid_profile_name",
            cur_lang(),
            &[("name", &name)],
        ));
    }
    if !profile_names().iter().any(|x| x == &name) {
        return Err(trv("err.unknown_profile", cur_lang(), &[("name", &name)]));
    }
    let repair = abs(REPAIR_SCRIPT_REL);
    // name is validated ([A-Za-z0-9_-]); repair path carries no single quotes — safe in 'literals'.
    // NB: Start-Process does NOT quote -ArgumentList elements, so a `-File <path with spaces/!>`
    // silently breaks (elevated pwsh can't find the script) while `-Wait` swallows the child's
    // exit code → false success. Pass the script via `-Command "& '<path>' -Name '<n>'"` (single-
    // quoted path survives) and check the real ExitCode via -PassThru.
    // Write-Host args are localized; escape single quotes (PowerShell '' ) so a future translation
    // containing an apostrophe can't break out of the single-quoted literal in this ELEVATED command.
    // Defense in depth: current translations are apostrophe-free, but that invariant shouldn't rest on
    // a comment alone. (name is charset-validated above; repair is escaped here for completeness.)
    let lang = cur_lang();
    let esc = |s: &str| s.replace('\'', "''");
    let s_start = esc(tr("log.relink_start", lang));
    let s_done = esc(tr("log.done", lang));
    let s_err = esc(tr("log.relink_error_code", lang));
    let s_cancel = esc(tr("log.relink_cancelled", lang));
    let repair = esc(&repair);
    let inner = format!(
        "Write-Host '{s_start}'; \
         try {{ $p = Start-Process -FilePath pwsh -Verb RunAs -PassThru -Wait -ArgumentList \
         @('-NoProfile','-ExecutionPolicy','Bypass','-Command',\"& '{repair}' -Name '{name}'\") \
         -ErrorAction Stop; \
         if ($p.ExitCode -eq 0) {{ Write-Host '{s_done}' }} else {{ Write-Host ('{s_err}' + $p.ExitCode); exit 1 }} }} \
         catch {{ Write-Host '{s_cancel}'; exit 1 }}"
    );
    let args = vec!["-NoProfile".to_string(), "-Command".to_string(), inner];
    spawn_streamed_prog(
        app,
        state,
        "profiles".to_string(),
        "pwsh".to_string(),
        args,
        None,
    )
    .await
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
        .map_err(|e| trv("err.pwsh_failed", cur_lang(), &[("e", &e)]))?;
    if status.success() {
        app.exit(0);
        Ok(())
    } else {
        Err(tr("err.elevation_cancelled", cur_lang()).into())
    }
}

// --- Sync tab (native; was Manage-Sync.ps1) ---
const SYNC_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\sync-config.json";
const SYNC_CANON_STIGNORE_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\.stignore";

/// Item key -> .stignore whitelist line (order matters; mirrors Manage-Sync.ps1 $ItemLines).
fn sync_item_lines() -> [(&'static str, &'static str); 7] {
    [
        ("history", "!/history.jsonl"),
        ("projects", "!/projects"),
        ("skills", "!/skills"),
        ("agents", "!/agents"),
        ("commands", "!/commands"),
        ("keybindings", "!/keybindings.json"),
        // Castellyn's own durable data (item 18): the Sessions-personalization sidecar
        // (~/.claude/castellyn/sessions.json) rides the same ~/.claude sync between machines.
        ("castellyn", "!/castellyn"),
    ]
}

/// Absolute path of the live ~/.claude/.stignore.
fn live_stignore_path() -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    format!("{home}\\.claude\\.stignore")
}

/// Read config\sync-config.json → ordered (key, enabled); default all-on, `items.<k>` overrides.
fn read_sync_config() -> Vec<(String, bool)> {
    let mut items: Vec<(String, bool)> = sync_item_lines()
        .iter()
        .map(|(k, _)| (k.to_string(), true))
        .collect();
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
        "// Whitelist below is driven by config\\sync-config.json (dashboard -> Синхронизация)."
            .into(),
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

/// Write UTF-8 without BOM, tolerating a Hidden/ReadOnly target (clear attrs, write, restore both).
fn write_file_no_bom(path: &str, content: &str) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    let meta = std::fs::metadata(path).ok();
    let was_hidden = meta
        .as_ref()
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
        .unwrap_or(false);
    let was_ro = meta
        .as_ref()
        .map(|m| m.permissions().readonly())
        .unwrap_or(false);
    if was_hidden || was_ro {
        run_attrib(&["-h", "-r"], path);
    }
    std::fs::write(path, content)?;
    // Restore whatever attrs the target carried; previously +r was dropped on every write.
    if was_hidden {
        run_attrib(&["+h"], path);
    }
    if was_ro {
        run_attrib(&["+r"], path);
    }
    Ok(())
}

/// Durable config write: back up the existing target to `<path>.bak`, write the new content to a
/// temp file in the same directory (UTF-8 **no BOM**, attribute-aware via `write_file_no_bom`),
/// then atomically rename it over the target (`std::fs::rename` replaces on Windows). A crash mid-
/// write leaves either the old file or the temp behind — never a half-written/blanked target.
/// Single DRY entry for every Castellyn config writer (myproviders/engines/router/opencode/config).
fn write_json_atomic(path: &str, content: &str) -> std::io::Result<()> {
    let p = std::path::Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Snapshot the target's Hidden/ReadOnly so the rename can replace a RO/Hidden file (a plain
    // rename onto a ReadOnly target fails on Windows) and we can re-stamp the attrs afterwards —
    // preserving the attr-tolerance the direct no-BOM writer gave every target before this helper.
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    let meta = std::fs::metadata(path).ok();
    let was_hidden = meta
        .as_ref()
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
        .unwrap_or(false);
    let was_ro = meta
        .as_ref()
        .map(|m| m.permissions().readonly())
        .unwrap_or(false);
    // Back up the prior good copy before we touch anything (best-effort, mirrors the old writers) —
    // EXCEPT for secret-bearing files: a profile's settings.json carries the ANTHROPIC_AUTH_TOKEN and
    // opencode.json a literal apiKey, both living outside Castellyn's own dir where they may be synced.
    // An in-place .bak would strand a prior cleartext secret after rotation. The atomic temp+rename
    // below already guarantees the target is never blanked, so skipping the .bak costs no crash safety.
    let is_secret_file = p
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            // .claude.json carries the ANTHROPIC auth token + per-server MCP env (may hold keys).
            n.eq_ignore_ascii_case("settings.json")
                || n.eq_ignore_ascii_case("opencode.json")
                || n.eq_ignore_ascii_case(".claude.json")
        })
        .unwrap_or(false);
    if p.exists() && !is_secret_file {
        let _ = std::fs::copy(path, format!("{path}.bak"));
    }
    if was_hidden || was_ro {
        run_attrib(&["-h", "-r"], path);
    }
    // Temp in the SAME dir so the rename is a same-volume atomic replace.
    let tmp = format!("{path}.tmp");
    write_file_no_bom(&tmp, content)?;
    // rename() overwrites the destination on Windows (unlike POSIX hard semantics, std handles this).
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp); // don't leave a stray .tmp on failure
        return Err(e);
    }
    // Re-stamp the attributes the caller's file carried before the swap.
    if was_hidden {
        run_attrib(&["+h"], path);
    }
    if was_ro {
        run_attrib(&["+r"], path);
    }
    Ok(())
}

/// Windows ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33): another process (a Claude
/// session reading its settings.json, an AV scan, SyncThing) has the file open — a transient state
/// that clears in milliseconds, so it's worth a short retry rather than failing the whole sweep.
fn is_sharing_violation(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32) | Some(33))
}

/// Run a fallible fs write, retrying ONLY a transient sharing/lock violation with a short backoff
/// (50/150/400 ms). Any other error returns immediately. Extracted + generic so the retry policy is
/// unit-testable without a real locked file.
fn with_sharing_retry<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    const BACKOFF_MS: [u64; 3] = [50, 150, 400];
    let mut attempt = 0;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if is_sharing_violation(&e) && attempt < BACKOFF_MS.len() => {
                std::thread::sleep(std::time::Duration::from_millis(BACKOFF_MS[attempt]));
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// `write_json_atomic` with the transient-sharing-violation retry (above) — for the per-profile
/// settings.json sweeps where a live session may briefly hold the file open.
fn write_json_atomic_retry(path: &str, content: &str) -> std::io::Result<()> {
    with_sharing_retry(|| write_json_atomic(path, content))
}

// --- Syncthing local REST (best-effort; mirrors Get-SyncthingStatus) ---

/// GUI base URL (`http://host:port`) from config.xml's `<gui>` `<address>`, so a non-default port or
/// bind address works instead of the hardcoded `127.0.0.1:8384`. `<address>` ALSO appears under
/// `<device>`, so we scope strictly to the `<gui>…</gui>` block. A wildcard bind (`0.0.0.0` / `[::]`)
/// is mapped to loopback — that's where the local REST is actually reachable. Falls back to the
/// default on any parse miss. NOTE: a tls-enabled GUI (non-default `<gui tls="true">`, self-signed
/// https) stays unsupported — a pre-existing limitation, not a regression (the old code also used http).
fn syncthing_gui_base(xml: &str) -> String {
    const DEFAULT: &str = "http://127.0.0.1:8384";
    let Some(g) = xml.find("<gui") else {
        return DEFAULT.to_string();
    };
    let rest = &xml[g..];
    let Some(e) = rest.find("</gui>") else {
        return DEFAULT.to_string();
    };
    let gui = &rest[..e];
    let Some(s) = gui.find("<address>").map(|i| i + "<address>".len()) else {
        return DEFAULT.to_string();
    };
    let Some(en) = gui[s..].find("</address>").map(|i| i + s) else {
        return DEFAULT.to_string();
    };
    let Some((host, port)) = gui[s..en].trim().rsplit_once(':') else {
        return DEFAULT.to_string();
    };
    let (host, port) = (host.trim(), port.trim());
    if port.is_empty() {
        return DEFAULT.to_string();
    }
    let host = match host {
        "0.0.0.0" | "::" | "[::]" | "" => "127.0.0.1",
        h => h,
    };
    format!("http://{host}:{port}")
}

/// Read Syncthing's config.xml ONCE → (api_key, gui_base_url). None when no API key is configured.
fn syncthing_conn() -> Option<(String, String)> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let cfg = std::fs::read_to_string(format!("{local}\\Syncthing\\config.xml")).ok()?;
    let start = cfg.find("<apikey>")? + "<apikey>".len();
    let end = cfg[start..].find("</apikey>")? + start;
    let key = cfg[start..end].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, syncthing_gui_base(&cfg)))
}

fn st_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_millis(1500)))
        .build()
        .into()
}

fn st_get(agent: &ureq::Agent, base: &str, key: &str, path: &str) -> Option<serde_json::Value> {
    let url = format!("{base}{path}");
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
fn st_claude_folder(agent: &ureq::Agent, base: &str, key: &str) -> Option<serde_json::Value> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let claude = normalize_path(&format!("{home}\\.claude"));
    let folders = st_get(agent, base, key, "/rest/config/folders")?;
    folders
        .as_array()?
        .iter()
        .find(|f| {
            f.get("path")
                .and_then(|p| p.as_str())
                .map(|p| normalize_path(p) == claude)
                .unwrap_or(false)
        })
        .cloned()
}

fn syncthing_status() -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert("available".into(), serde_json::json!(false));
    let Some((key, base)) = syncthing_conn() else {
        return serde_json::Value::Object(out); // no API key configured at all
    };
    // We have a key → let the UI tell "configured but not answering" apart from "not configured".
    out.insert("keyConfigured".into(), serde_json::json!(true));
    let agent = st_agent();
    if st_get(&agent, &base, &key, "/rest/system/ping").is_none() {
        // available stays false; keyConfigured:true signals daemon-down / wrong-key vs unconfigured.
        return serde_json::Value::Object(out);
    }
    out.insert("available".into(), serde_json::json!(true));
    if let Some(ver) = st_get(&agent, &base, &key, "/rest/system/version")
        .and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from))
    {
        out.insert("version".into(), serde_json::json!(ver));
    }
    let Some(folder) = st_claude_folder(&agent, &base, &key) else {
        out.insert("folderShared".into(), serde_json::json!(false));
        return serde_json::Value::Object(out); // ~/.claude not a Syncthing folder here
    };
    out.insert("folderShared".into(), serde_json::json!(true));
    if let Some(label) = folder.get("label").and_then(|x| x.as_str()) {
        out.insert("folderLabel".into(), serde_json::json!(label));
    }
    let fid = folder
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    out.insert("folderId".into(), serde_json::json!(fid));
    if let Some(st) = st_get(&agent, &base, &key, &format!("/rest/db/status?folder={fid}")) {
        if let Some(state) = st.get("state").and_then(|x| x.as_str()) {
            out.insert("state".into(), serde_json::json!(state));
        }
        let g = st
            .get("globalBytes")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let n = st.get("needBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.insert("globalBytes".into(), serde_json::json!(g as i64));
        out.insert("needBytes".into(), serde_json::json!(n as i64));
        // Clamp: needBytes can momentarily exceed globalBytes mid-scan, which would otherwise yield
        // a negative or >100 completion.
        let completion = (if g > 0.0 {
            (100.0 * (g - n) / g).clamp(0.0, 100.0)
        } else {
            100.0
        } * 10.0)
            .round()
            / 10.0;
        out.insert("completion".into(), serde_json::json!(completion));
    } else {
        out.insert("state".into(), serde_json::json!("unknown"));
    }
    if let Some(conns) = st_get(&agent, &base, &key, "/rest/system/connections") {
        let connected = conns
            .get("connections")
            .and_then(|c| c.as_object())
            .map(|o| {
                o.values()
                    .filter(|d| {
                        d.get("connected")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        out.insert("connectedDevices".into(), serde_json::json!(connected));
    }
    serde_json::Value::Object(out)
}

/// Ask Syncthing to rescan the ~/.claude folder so a fresh .stignore applies now (best-effort).
fn syncthing_rescan() {
    let Some((key, base)) = syncthing_conn() else {
        return;
    };
    let agent = st_agent();
    if let Some(fid) = st_claude_folder(&agent, &base, &key)
        .and_then(|f| f.get("id").and_then(|x| x.as_str()).map(String::from))
    {
        let url = format!("{base}/rest/db/scan?folder={fid}");
        let _ = agent.post(&url).header("X-API-Key", &key).send_empty();
    }
}

#[cfg(test)]
mod syncthing_tests {
    #[test]
    fn gui_base_parses_scopes_and_maps() {
        let d = "http://127.0.0.1:8384";
        assert_eq!(super::syncthing_gui_base("<gui><address>127.0.0.1:8384</address></gui>"), d);
        assert_eq!(
            super::syncthing_gui_base("<gui enabled=\"true\"><address>192.168.1.5:9090</address></gui>"),
            "http://192.168.1.5:9090"
        );
        // wildcard bind → loopback (that's where the local REST is reachable)
        assert_eq!(
            super::syncthing_gui_base("<gui><address>0.0.0.0:8080</address></gui>"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(super::syncthing_gui_base("<gui><address>[::]:8384</address></gui>"), d);
        // <address> under <device> must NOT be picked up — only the <gui> one
        let multi = "<device><address>dynamic</address></device><gui><address>10.0.0.1:7070</address></gui>";
        assert_eq!(super::syncthing_gui_base(multi), "http://10.0.0.1:7070");
        // missing / malformed → default
        assert_eq!(super::syncthing_gui_base("<gui></gui>"), d);
        assert_eq!(super::syncthing_gui_base(""), d);
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
        let items_obj: serde_json::Map<String, serde_json::Value> = items
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
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

    // Backup + atomic write of the source-of-truth config.
    let cfg = abs(SYNC_CONFIG_REL);
    let items_obj: serde_json::Map<String, serde_json::Value> = items
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
        .collect();
    let payload = serde_json::json!({
        "schemaVersion": 1,
        "_comment": "Что синхронизируется между машинами (Syncthing claude-config = ~/.claude). Менять через дашборд Castellyn.",
        "items": items_obj,
    });
    let cfg_json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    write_json_atomic(&cfg, &cfg_json).map_err(|e| format!("write sync-config.json: {e}"))?;

    // Regenerate canonical (config\.stignore, backed up) + live (~/.claude/.stignore).
    let canon = abs(SYNC_CANON_STIGNORE_REL);
    write_json_atomic(&canon, &content).map_err(|e| format!("write config\\.stignore: {e}"))?;
    let live = live_stignore_path();
    write_json_atomic(&live, &content).map_err(|e| format!("write ~/.claude/.stignore: {e}"))?;

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
        _ => Err(trv(
            "err.unknown_sync_action",
            cur_lang(),
            &[("action", &action)],
        )),
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
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
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

/// Probe a set of ports concurrently. Each probe blocks up to 250ms, so thread::scope bounds the
/// total by the slowest single probe instead of N*250ms. (port 0 → false, handled by port_listening.)
fn probe_ports(ports: &[u16]) -> Vec<bool> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = ports
            .iter()
            .map(|&p| scope.spawn(move || port_listening(p)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(false))
            .collect()
    })
}

/// Engine registry (config\engines.json) + live running status (port probe). Read-only.
#[tauri::command]
async fn read_engines() -> Vec<EngineStatus> {
    // Off the main/event-loop thread — the port probe blocks up to 250ms per dashboard refresh.
    tokio::task::spawn_blocking(read_engines_blocking)
        .await
        .unwrap_or_default()
}

fn read_engines_blocking() -> Vec<EngineStatus> {
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
                installed: if is_router {
                    Some(cmd_on_path("ccr"))
                } else {
                    None
                },
                running: false,
            }
        })
        .collect();
    let ports: Vec<u16> = engines.iter().map(|e| e.port).collect();
    for (e, r) in engines.iter_mut().zip(probe_ports(&ports)) {
        e.running = r;
    }
    engines
}

const STACK_CONFIG_REL: &str = "llm-stack\\stack.json";
const STACK_START_REL: &str = "llm-stack\\start-stack.ps1";
const STACK_STOP_REL: &str = "llm-stack\\stop-stack.ps1";

/// The `services` array from stack.json, parsed once. Empty on any failure (missing/corrupt file
/// or no services key). Callers extract the fields they need — one read+parse, not five copies.
fn stack_services() -> Vec<serde_json::Value> {
    let Ok(content) = std::fs::read_to_string(abs(STACK_CONFIG_REL)) else {
        return Vec::new();
    };
    let Ok(v) = parse_json_bom(&content) else {
        return Vec::new();
    };
    v.get("services")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default()
}

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
async fn read_stack() -> Vec<StackService> {
    // Off the main/event-loop thread — the port probe blocks up to 250ms per dashboard refresh.
    tokio::task::spawn_blocking(read_stack_blocking)
        .await
        .unwrap_or_default()
}

fn read_stack_blocking() -> Vec<StackService> {
    let s = |e: &serde_json::Value, k: &str| {
        e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let mut svcs: Vec<StackService> = stack_services()
        .iter()
        .map(|e| StackService {
            id: s(e, "id"),
            name: s(e, "name"),
            group: s(e, "group"),
            port: e.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16,
            protocol: s(e, "protocol"),
            dashboard: e
                .get("dashboard")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            dir: expand_placeholders(&s(e, "dir")),
            enabled: e.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true),
            running: false,
        })
        .collect();
    let ports: Vec<u16> = svcs.iter().map(|s| s.port).collect();
    for (svc, r) in svcs.iter_mut().zip(probe_ports(&ports)) {
        svc.running = r;
    }
    svcs
}

/// A stack service id is a manifest key, passed to PowerShell as a standalone argv element (no
/// shell), so this only needs to reject obviously malformed ids — keep it to the manifest's shape.
fn valid_stack_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
            return Err(trv("err.invalid_service_id", cur_lang(), &[("id", &id)]));
        }
    }

    // Restart = stop then start under ONE run slot, with a single run-done at the end. The stop
    // phase emits an event the UI ignores so it doesn't read as a completed run mid-way.
    if action == "restart" {
        let _slot = RunSlot::reserve(state.inner())?;
        let (stop_args, start_args) = match &only {
            Some(id) => (
                vec!["-Only".to_string(), id.clone()],
                vec!["-Only".to_string(), id.clone()],
            ),
            None => (vec!["-All".to_string()], vec!["-Router".to_string()]),
        };
        // Start even if stop failed — the goal is a running service.
        // ponytail: a cancel during the stop phase still proceeds to start; fine for a restart.
        let _ = spawn_pwsh_phase(
            &app,
            &state,
            "engine",
            abs(STACK_STOP_REL),
            stop_args,
            "run-restart-stop",
        )
        .await;
        let code = spawn_pwsh_phase(
            &app,
            &state,
            "engine",
            abs(STACK_START_REL),
            start_args,
            "run-done",
        )
        .await;
        drop(_slot); // release the run slot (also released on early return / panic above)
        return Ok(code);
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
        _ => {
            return Err(trv(
                "err.unknown_stack_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
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

/// Configured stack ports (no probe) — for callers that need only the port list, not live status.
fn stack_ports() -> Vec<u16> {
    stack_services()
        .iter()
        .filter_map(|e| e.get("port").and_then(|p| p.as_u64()))
        .map(|p| p as u16)
        .filter(|&p| p != 0)
        .collect()
}

/// PID + uptime for every currently-listening stack port (one process snapshot via pwsh). Ports
/// with no listener are omitted. Read-only; never touches the services. Empty on any failure.
#[tauri::command]
async fn read_stack_procs() -> Vec<StackProc> {
    let ports = stack_ports();
    if ports.is_empty() {
        return Vec::new();
    }
    let port_args = ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let script = abs(STACK_PROCS_SCRIPT_REL);
    let out = tokio::process::Command::new("pwsh")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script,
            "-Ports",
            &port_args,
        ])
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
    let dir = stack_services()
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some("gateway"))
        .and_then(|e| e.get("dir").and_then(|x| x.as_str()))
        .map(String::from)?;
    Some(format!("{}\\data\\freeapi.db", expand_placeholders(&dir)))
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
    let Ok(out) = out else {
        return FreellmapiAnalytics::default();
    };
    if !out.status.success() {
        return FreellmapiAnalytics::default();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: Option<AnalyticsHelperOut> = parse_json_bom(stdout.trim())
        .ok()
        .and_then(|v| serde_json::from_value(v).ok());
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
    let p = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
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
    head.starts_with("HTTP/1.")
        && head
            .split(' ')
            .nth(1)
            .map(|c| c.starts_with('2'))
            .unwrap_or(false)
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
async fn read_stack_health() -> Vec<StackHealth> {
    // Off the main/event-loop thread — probe + HTTP health checks block up to ~1.1s per refresh.
    tokio::task::spawn_blocking(read_stack_health_blocking)
        .await
        .unwrap_or_default()
}

fn read_stack_health_blocking() -> Vec<StackHealth> {
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
    let rows: Vec<Row> = stack_services()
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
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or((false, None)))
            .collect()
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
        .ok_or(tr("err.engines_no_array", cur_lang()))?;
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
        return Err(trv("err.engine_not_found", cur_lang(), &[("id", &id)]));
    }
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    // Atomic temp+rename (+ .bak): a crash mid-write must never blank the engines tab.
    write_json_atomic(&path, &json).map_err(|e| format!("write engines.json: {e}"))?;
    Ok(())
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
    cmd.spawn().map(|_| ()).map_err(|e| {
        trv(
            "err.spawn_failed",
            cur_lang(),
            &[("program", &program), ("e", &e)],
        )
    })
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
            line: tr("log.detached_launch", cur_lang()).into(),
        },
    );
    let _ = app.emit(
        "run-done",
        RunDone {
            component: "engine".into(),
            code: 0,
        },
    );
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
        return Err(trv(
            "err.unknown_engine_action",
            cur_lang(),
            &[("action", &action)],
        ));
    }
    let Some(cfg) = load_engine_cfg(&id) else {
        return Err(trv("err.engine_not_found_json", cur_lang(), &[("id", &id)]));
    };

    if action == "start" {
        // A server never exits, so launch it DETACHED in its own console and return immediately —
        // streaming-until-exit would hang the run slot forever. Status comes from the port probe.
        // Shell start command (ccr, llmstack): run via `cmd /c` in a fresh console.
        if !cfg.start.trim().is_empty() {
            let sh = expand_placeholders(&cfg.start);
            spawn_engine_detached("cmd", &["/c".to_string(), sh], None)?;
            emit_engine_started(&app);
            return Ok(0);
        }
        // File-based engine: run the file directly in its own console (.py via python), cwd = its dir.
        if !cfg.command.trim().is_empty() {
            let cmd = expand_placeholders(&cfg.command);
            let path = std::path::Path::new(&cmd);
            if !path.is_file() {
                return Err(trv("err.launch_file_missing", cur_lang(), &[("cmd", &cmd)]));
            }
            let dir = path.parent().map(|p| p.display().to_string());
            if cmd.to_lowercase().ends_with(".py") {
                spawn_engine_detached("python", std::slice::from_ref(&cmd), dir.as_deref())?;
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
        let sh = expand_placeholders(&cfg.stop);
        return spawn_streamed_prog(
            app,
            state,
            "engine".into(),
            "cmd".into(),
            vec!["/c".into(), sh],
            None,
        )
        .await;
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
    match std::process::Command::new(prog)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
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
            err(&trv("log.spawn_failed_indent", cur_lang(), &[("e", &e)]));
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
        err(tr("log.cfg_need_backend", cur_lang()));
        return 1;
    }
    if model.is_empty() {
        err(tr("log.cfg_need_model", cur_lang()));
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
    out(&trv(
        "log.provider_line",
        cur_lang(),
        &[("name", &name), ("api_base", &api_base), ("model", &model)],
    ));
    let provider = json!({ "name": name, "api_base_url": api_base, "api_key": "not-needed", "models": [model] });
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

    // Backup + atomic write (temp+rename, UTF-8 no BOM).
    let serialized = match serde_json::to_string_pretty(&cfg) {
        Ok(s) => s,
        Err(e) => {
            err(&trv("log.ser_config", cur_lang(), &[("e", &e)]));
            return 1;
        }
    };
    if let Err(e) = write_json_atomic(cfg_path, &serialized) {
        err(&trv("log.write_config_err", cur_lang(), &[("e", &e)]));
        return 1;
    }
    out(tr("log.config_written", cur_lang()));
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
            out(tr("log.ccr_already", cur_lang()));
            return 0;
        }
        let Some(npm) = exe_on_path("npm") else {
            err(tr("log.npm_missing", cur_lang()));
            return 1;
        };
        out("  npm install -g @musistudio/claude-code-router …");
        stream_output(
            &npm,
            &["install", "-g", "@musistudio/claude-code-router"],
            out,
            err,
        );
        if exe_on_path("ccr").is_some() {
            out(tr("log.ccr_installed", cur_lang()));
            0
        } else {
            out(tr("log.ccr_unconfirmed", cur_lang()));
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
        out(tr("log.ccr_done_hint", cur_lang()));
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
    out(&trv(
        "log.router_connect_header",
        cur_lang(),
        &[("name", &name), ("profile", &profile)],
    ));

    // 1. Configure ccr for this backend/model (+ ccr restart inside Setup-Router).
    let code = setup_router_native("configure", backend, model, name, out, err);
    if code != 0 {
        err(tr("log.aborted_ccr_setup", cur_lang()));
        return code;
    }

    // 2. Ensure ccr is running and verify the port came up (non-fatal warning, mirrors the script).
    if let Some(ccr) = exe_on_path("ccr") {
        out("  ccr start …");
        stream_output(&ccr, &["start"], out, err);
        std::thread::sleep(std::time::Duration::from_secs(4));
        if port_listening(3456) {
            out(tr("log.ccr_listening", cur_lang()));
        } else {
            out(tr("log.ccr_port_warn", cur_lang()));
            out(tr("log.ccr_port_hint", cur_lang()));
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
    let token_opt = if token.is_empty() {
        None
    } else {
        Some(token.as_str())
    };
    let code = manage_provider_native(
        profile,
        "set",
        ccr_base,
        false,
        token_opt,
        Some(model),
        None,
        out,
        err,
    );
    if code != 0 {
        err(tr("log.aborted_bind", cur_lang()));
        return code;
    }
    out(&trv(
        "log.router_done",
        cur_lang(),
        &[
            ("profile", &profile),
            ("ccr_base", &ccr_base),
            ("name", &name),
            ("model", &model),
        ],
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
        return Err(trv(
            "err.unknown_router_action",
            cur_lang(),
            &[("action", &action)],
        ));
    }
    let backend = backend.unwrap_or_default();
    let model = model.unwrap_or_default();
    if action == "configure" && (backend.is_empty() || model.is_empty()) {
        return Err(tr("err.configure_needs_backend_model", cur_lang()).into());
    }
    // PS default provider name is 'lmstudio'.
    let name = name
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "lmstudio".to_string());
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
        return Err(tr("err.needs_backend_model", cur_lang()).into());
    }
    if !valid_profile_name(&profile) {
        return Err(trv(
            "err.invalid_profile",
            cur_lang(),
            &[("profile", &profile)],
        ));
    }
    // PS default provider name is 'lmstudio'.
    let name = name
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "lmstudio".to_string());
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
    #[serde(rename = "isArchived")]
    is_archived: bool,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    description: String,
    language: String,
    stars: u64,
}

/// All of the authenticated user's GitHub repos (incl. private), via `gh repo list`.
/// Lets the UI surface repos that aren't locally cloned. Empty if gh is missing or
/// unauthenticated; read-only (no network writes).
#[tauri::command]
async fn list_github_repos() -> Vec<GithubRepo> {
    let fut = tokio::process::Command::new("gh")
        .args([
            "repo", "list", "--limit", "1000", "--json",
            "name,owner,nameWithOwner,isPrivate,isFork,isArchived,url,updatedAt,description,primaryLanguage,stargazerCount",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // Bound a hung `gh` (flaky network / auth prompt) — Err = timed out, Ok(Err) = spawn failed.
    let Ok(Ok(out)) = tokio::time::timeout(std::time::Duration::from_secs(30), fut).await else {
        return Vec::new();
    };
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
                is_archived: b("isArchived"),
                url: s("url"),
                updated_at: s("updatedAt"),
                description: s("description"),
                // primaryLanguage is an object {name} (or null for empty repos).
                language: r
                    .get("primaryLanguage")
                    .and_then(|p| p.get("name"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                stars: r
                    .get("stargazerCount")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
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
                    let g = |k: &str| {
                        env.get(k)
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string()
                    };
                    // Prefer the current tier env vars; fall back to the legacy single-value keys
                    // so profiles bound by an older AgentHub still display their model.
                    let g_or = |new: &str, old: &str| {
                        let v = g(new);
                        if v.is_empty() {
                            g(old)
                        } else {
                            v
                        }
                    };
                    p.base_url = g("ANTHROPIC_BASE_URL");
                    p.model = g_or("ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_MODEL");
                    p.small_model = g_or(
                        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                        "ANTHROPIC_SMALL_FAST_MODEL",
                    );
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
    [
        ".claude.json",
        "sessions",
        "shell-snapshots",
        "session-env",
        "todos",
        "projects",
    ]
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
    let Ok(home) = std::env::var("USERPROFILE") else {
        return false;
    };
    dir_recently_written(
        &std::path::Path::new(&home).join(format!(".claude-{name}")),
        120,
    )
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
        err(&trv(
            "log.profile_not_found",
            cur_lang(),
            &[("name", &name), ("known", &known.join(", "))],
        ));
        return 1;
    }
    // cc3-class guard: never rewrite a settings.json a live session is reading (see
    // profile_session_active). All provider-bind paths funnel here, so one check covers them all.
    if profile_session_active(name) {
        err(&trv(
            "log.profile_running_warn",
            cur_lang(),
            &[("name", &name)],
        ));
        return 1;
    }
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => {
            err(tr("log.no_userprofile", cur_lang()));
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
                err(&trv("log.read_settings", cur_lang(), &[("e", &e)]));
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
                tr("log.unchanged", cur_lang())
            } else if token.filter(|s| !s.is_empty()).is_some() {
                tr("log.set_value", cur_lang())
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
            out(tr("log.provider_reset", cur_lang()));
        }
        env.is_empty()
    };
    if env_empty {
        sobj.remove("env");
    }

    // Backup + atomic write (temp+rename, UTF-8 no BOM).
    let serialized = match serde_json::to_string_pretty(&settings) {
        Ok(s) => s,
        Err(e) => {
            err(&trv("log.ser_settings", cur_lang(), &[("e", &e)]));
            return 1;
        }
    };
    if let Err(e) = write_json_atomic(settings_path, &serialized) {
        err(&trv("log.write_settings", cur_lang(), &[("e", &e)]));
        return 1;
    }
    out(&trv("log.settings_updated", cur_lang(), &[("name", &name)]));
    0
}

/// Bind (set) or unbind (clear) a profile's provider (native; was Manage-Provider.ps1).
#[tauri::command]
// command handler: args come from the JS invoke boundary
#[allow(clippy::too_many_arguments)]
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
        return Err(trv(
            "err.unknown_provider_action",
            cur_lang(),
            &[("action", &action)],
        ));
    }
    if !valid_profile_name(&name) {
        return Err(trv(
            "err.invalid_profile_name",
            cur_lang(),
            &[("name", &name)],
        ));
    }
    let base_url = base_url.unwrap_or_default();
    if action == "set" && base_url.is_empty() {
        return Err(tr("err.set_needs_baseurl", cur_lang()).into());
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
        let token_arg = if keep_token {
            None
        } else {
            Some(token.as_str())
        };
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
    service
        .strip_prefix("castellyn.")
        .map(|s| format!("agenthub.{s}"))
}

fn kr_get(service: &str, user: &str) -> Option<String> {
    if let Some(v) = keyring::Entry::new(service, user)
        .ok()
        .and_then(|e| e.get_password().ok())
    {
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
    if count <= 1 {
        0
    } else {
        (active + 1) % count
    }
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

/// L3: extract the bare host from a `host[:port]` chunk. Handles an IPv6 literal (`[::1]` /
/// `[::1]:port`) without mistaking its inner colons for the port separator. Shared by
/// `valid_base_url` and `probe_url_allowed` (previously copy-pasted in both).
fn extract_host(host_port: &str) -> &str {
    if host_port.starts_with('[') {
        host_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
    }
}

/// L2: true if a literal/resolved IP is link-local, unspecified, or a known cloud-metadata address.
/// Loopback and RFC1918 are intentionally NOT blocked — local engines (LM Studio) live there.
fn is_blocked_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_link_local()                       // 169.254.0.0/16 — AWS/GCP/Azure IMDS
                || v4.is_unspecified()               // 0.0.0.0
                || v4.octets() == [100, 100, 100, 200] // Alibaba metadata
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || *v6 == std::net::Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0xec2, 0x254) // fd00:ec2::254
        }
    }
}

/// SSRF guard for a provider base URL (ports the intent of Mediafarm's validate_base_url):
/// require http/https and reject link-local + known cloud-metadata pivots. Localhost / RFC1918
/// are allowed on purpose (local engines like LM Studio). Run before storing a key and before connect.
fn valid_base_url(s: &str) -> Result<(), String> {
    let s = s.trim();
    let rest = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .ok_or(tr("err.url_scheme", cur_lang()))?;
    let host_port = rest.split('/').next().unwrap_or("");
    let host = extract_host(host_port); // L3: shared IPv6-aware host extraction
    if host.is_empty() {
        return Err(tr("err.empty_host", cur_lang()).into());
    }
    let hl = host.to_ascii_lowercase();
    let blocked = [
        "169.254.169.254",
        "100.100.100.200",
        "fd00:ec2::254",
        "metadata.google.internal",
    ];
    if blocked.contains(&hl.as_str()) || hl.starts_with("169.254.") || hl == "metadata" {
        return Err(trv("err.blocked_host", cur_lang(), &[("host", &host)]));
    }
    // L2: the string list above misses non-canonical encodings (decimal/octal/hex IPs, trailing
    // dot) and hostnames that RESOLVE to a metadata/link-local address. Check the literal IP if the
    // host is one; otherwise best-effort resolve and check what will actually be dialed. Resolution
    // failure fails OPEN — a host that can't resolve can't be connected to anyway, and we must not
    // reject a legitimate save just because DNS is momentarily flaky.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_blocked_ip(&ip) {
            return Err(trv("err.blocked_host", cur_lang(), &[("host", &host)]));
        }
    } else {
        use std::net::ToSocketAddrs;
        if let Ok(addrs) = (host, 0u16).to_socket_addrs() {
            if addrs.map(|s| s.ip()).any(|ip| is_blocked_ip(&ip)) {
                return Err(trv("err.blocked_host", cur_lang(), &[("host", &host)]));
            }
        }
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

/// Serializes the read-modify-write of myproviders.json (R2-02). save/delete/add_key/remove_key
/// each read the list, mutate it, and write it back; without this they could interleave and lose an
/// update (last writer wins). Poison-tolerant at the call sites via unwrap_or_else(|e| e.into_inner()).
static MYPROVIDERS_LOCK: Mutex<()> = Mutex::new(());

fn read_myproviders_raw() -> Vec<serde_json::Value> {
    // Lenient read for read-only/display callers: recover from .bak, else empty.
    read_json_or_recover(abs(MYPROVIDERS_CONFIG_REL), "myproviders.json")
        .ok()
        .flatten()
        .and_then(|v| v.get("providers").and_then(|p| p.as_array()).cloned())
        .unwrap_or_default()
}

/// myproviders list for MUTATING callers: Err (abort, don't overwrite) when the file is corrupt and
/// unrecoverable, instead of silently returning an empty list that the next write persists as a wipe.
fn read_myproviders_checked() -> Result<Vec<serde_json::Value>, String> {
    Ok(
        read_json_or_recover(abs(MYPROVIDERS_CONFIG_REL), "myproviders.json")?
            .and_then(|v| v.get("providers").and_then(|p| p.as_array()).cloned())
            .unwrap_or_default(),
    )
}

fn write_myproviders_raw(list: &[serde_json::Value]) -> Result<(), String> {
    let path = abs(MYPROVIDERS_CONFIG_REL);
    let v = serde_json::json!({ "schemaVersion": 1, "providers": list });
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_json_atomic(&path, &json).map_err(|e| format!("write myproviders.json: {e}"))
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
    read_myproviders_raw()
        .iter()
        .map(myprovider_from_entry)
        .collect()
}

/// Upsert a provider record. `api_key` arrives over the (local) Tauri IPC channel — not argv —
/// and is written to the Credential Manager; an empty/None key keeps any existing one.
#[tauri::command]
fn save_my_provider(p: MyProviderInput, api_key: Option<String>) -> Result<MyProvider, String> {
    let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !valid_provider_name(&p.name) {
        return Err(tr("err.invalid_provider_name", cur_lang()).into());
    }
    valid_base_url(&p.base_url)?;
    if !matches!(p.protocol.as_str(), "anthropic" | "openai") {
        return Err(tr("err.invalid_protocol", cur_lang()).into());
    }
    if !matches!(p.connect_via.as_str(), "freellmapi" | "direct") {
        return Err(tr("err.invalid_connectvia", cur_lang()).into());
    }
    let id =
        p.id.clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(gen_provider_id);
    // The id becomes a Credential Manager slot key (`provider:{id}` / `provider:{id}:{idx}`); a colon
    // or other separator in it could alias another provider's stored key. Validate like the opencode
    // path already does (alnum + _/- only) — gen_provider_id's 12-hex output passes this.
    if !valid_profile_name(&id) {
        return Err(trv("err.invalid_provider_id", cur_lang(), &[("id", &id)]));
    }
    let auth = if !p.auth_scheme.is_empty() {
        p.auth_scheme.clone()
    } else if p.protocol == "anthropic" {
        "x-api-key".to_string()
    } else {
        "bearer".to_string()
    };
    let mut list = read_myproviders_checked()?;
    let prev = find_provider(&list, &id);
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
                let idx = if active_key < key_count {
                    active_key
                } else {
                    0
                };
                kr_set(KR_PROVIDERS, &format!("provider:{id}:{idx}"), k.trim())?;
            } else {
                kr_set(KR_PROVIDERS, &format!("provider:{id}"), k.trim())?;
            }
        }
    }
    Ok(myprovider_from_entry(&entry))
}

/// Find a my-provider record by id (read) — shared predicate across the providers block.
fn find_provider<'a>(list: &'a [serde_json::Value], id: &str) -> Option<&'a serde_json::Value> {
    list.iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some(id))
}

/// Index of a my-provider record by id (for in-place mutation).
fn find_provider_idx(list: &[serde_json::Value], id: &str) -> Option<usize> {
    list.iter()
        .position(|e| e.get("id").and_then(|x| x.as_str()) == Some(id))
}

/// Delete a provider record and its Credential Manager entry.
#[tauri::command]
fn delete_my_provider(id: String) -> Result<(), String> {
    let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_myproviders_checked()?;
    let (key_count, _) = find_provider(&list, &id)
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

/// Transactional append of one key to a provider's rotation pool, parameterized over the key-store so
/// it is unit-testable with an in-memory map. On the first add it migrates the legacy single key
/// (`provider:{id}`) into slot 0, but deletes the legacy entry ONLY after `write` succeeds — so a
/// failed write rolls back cleanly (new slot + migrated slot 0 removed) and leaves the legacy key
/// intact instead of orphaning it. `write` receives the new key count, must persist it, and returns
/// Err to trigger rollback. Returns the new key count.
fn append_key_txn(
    id: &str,
    key: &str,
    count: u64,
    get: impl Fn(&str) -> Option<String>,
    set: impl Fn(&str, &str) -> Result<(), String>,
    del: impl Fn(&str),
    write: impl FnOnce(u64) -> Result<(), String>,
) -> Result<u64, String> {
    let mut count = count;
    // First add: fold the legacy single key (if any) into slot 0. Keep the legacy entry until the write
    // lands, so a write failure rolls back to it instead of losing the key.
    let migrated = if count == 0 {
        if let Some(legacy) = get(&format!("provider:{id}")) {
            set(&format!("provider:{id}:0"), &legacy)?;
            count = 1;
            true
        } else {
            false
        }
    } else {
        false
    };
    let new_slot = count;
    // If the migration set slot 0 but THIS set fails, roll that slot back before returning — else an
    // orphan `provider:{id}:0` (a duplicate of the still-intact legacy secret) would linger with a
    // JSON keyCount that never counted it.
    if let Err(e) = set(&format!("provider:{id}:{new_slot}"), key) {
        if migrated {
            del(&format!("provider:{id}:0"));
        }
        return Err(e);
    }
    let new_count = new_slot + 1;
    match write(new_count) {
        Ok(()) => {
            // Write landed — now it is safe to drop the migrated legacy entry.
            if migrated {
                del(&format!("provider:{id}"));
            }
            Ok(new_count)
        }
        Err(e) => {
            // Roll back what this call wrote; the legacy `provider:{id}` is still intact.
            del(&format!("provider:{id}:{new_slot}"));
            if migrated {
                del(&format!("provider:{id}:0"));
            }
            Err(e)
        }
    }
}

/// Append a key to a provider's rotation pool. On the first add we migrate the legacy single key
/// (`provider:<id>`) into slot 0 so the pool subsumes it. The new key is appended (it does not
/// become active — rotation is explicit via next_provider_key). `api_key` arrives over Tauri IPC.
#[tauri::command]
fn add_provider_key(id: String, api_key: String) -> Result<MyProvider, String> {
    let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key = api_key.trim();
    if key.is_empty() {
        return Err(tr("err.empty_key", cur_lang()).into());
    }
    let mut list = read_myproviders_checked()?;
    let idx = find_provider_idx(&list, &id).ok_or(tr("err.provider_not_found", cur_lang()))?;
    let (count, active) = key_pool_meta(&list[idx]);
    append_key_txn(
        &id,
        key,
        count,
        |u| kr_get(KR_PROVIDERS, u),
        |u, s| kr_set(KR_PROVIDERS, u, s),
        |u| kr_delete(KR_PROVIDERS, u),
        |new_count| {
            list[idx]["keyCount"] = serde_json::json!(new_count);
            list[idx]["activeKey"] = serde_json::json!(active.min(new_count - 1));
            write_myproviders_raw(&list)
        },
    )?;
    Ok(myprovider_from_entry(&list[idx]))
}

/// Remove one key from the pool by index and re-pack the remaining slots (keyring has no enum, so
/// we read survivors, rewrite slots 0..n-1, drop the top slot, and clamp activeKey). Returns the
/// updated provider. Removing the last key collapses the pool back to "no key".
#[tauri::command]
fn remove_provider_key(id: String, index: u64) -> Result<MyProvider, String> {
    let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_myproviders_checked()?;
    let pos = find_provider_idx(&list, &id).ok_or(tr("err.provider_not_found", cur_lang()))?;
    let (count, active) = key_pool_meta(&list[pos]);
    if count == 0 || index >= count {
        return Err(tr("err.key_not_found", cur_lang()).into());
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
    // Rewrite compactly WITHOUT deleting first: write each survivor to its new slot, and only after
    // every write succeeds delete the now-stale trailing slots. A mid-write failure then leaves every
    // survivor key still present (old or new slot) instead of destroying the pool (the old order
    // deleted all slots up front, so a failed re-write lost the survivors permanently).
    let new_count = survivors.len() as u64;
    for (i, k) in survivors.iter().enumerate() {
        kr_set(KR_PROVIDERS, &format!("provider:{id}:{i}"), k)?;
    }
    for i in new_count..count {
        kr_delete(KR_PROVIDERS, &format!("provider:{id}:{i}"));
    }
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
    for (user, val) in [
        ("email", &email),
        ("password", &password),
        ("token", &token),
    ] {
        if let Some(v) = val {
            let v = v.trim();
            if !v.is_empty() {
                kr_set(KR_FREELLMAPI, user, v)?;
                any = true;
            }
        }
    }
    if !any {
        return Err(tr("err.freellmapi_creds_needed", cur_lang()).into());
    }
    Ok(())
}

/// Which freellmapi auth is configured (for the UI). Never returns the secret values themselves.
#[tauri::command]
fn freellmapi_auth_status() -> serde_json::Value {
    serde_json::json!({
        "hasEmail": kr_get(KR_FREELLMAPI, "email").is_some(),
        "hasPassword": kr_get(KR_FREELLMAPI, "password").is_some(),
        "hasToken": kr_get(KR_FREELLMAPI, "token").is_some(),
    })
}

/// Delete a single freellmapi auth entry from Credential Manager.
/// `key` must be one of {email, password, token} — anything else is rejected to avoid accidentally
/// purging unrelated keyring entries under KR_FREELLMAPI. We don't read first because the user
/// likely wants to remove a stale credential regardless of whether kr_get still returns Some.
#[tauri::command]
fn delete_freellmapi_auth(key: String) -> Result<(), String> {
    let k = key.trim();
    if !matches!(k, "email" | "password" | "token") {
        return Err(tr("err.freellmapi_key_invalid", cur_lang()).into());
    }
    kr_delete(KR_FREELLMAPI, k);
    Ok(())
}

/// F24: canonical `~/.claude/skills` path (resolves symlinks), mirroring gateway_base_url pattern.
#[tauri::command]
fn canonical_skills_dir() -> Result<String, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| format!("USERPROFILE: {e}"))?;
    let root = std::path::Path::new(&home).join(".claude").join("skills");
    std::fs::canonicalize(&root)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("canonicalize skills dir: {e}"))
}

/// freellmapi gateway base URL from the `gateway` service port in stack.json. None if absent.
#[tauri::command]
fn gateway_base_url() -> Option<String> {
    let port = stack_services()
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some("gateway"))
        .and_then(|e| e.get("port").and_then(|x| x.as_u64()))?;
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
            err(tr("log.freellmapi_no_creds", cur_lang()));
            return 1;
        }
        out(tr("log.freellmapi_login", cur_lang()));
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
                match parsed
                    .as_ref()
                    .and_then(|v| v["token"].as_str())
                    .filter(|t| !t.is_empty())
                {
                    Some(t) => t.to_string(),
                    None => {
                        err(tr("log.freellmapi_no_token", cur_lang()));
                        return 1;
                    }
                }
            }
            Err(ureq::Error::StatusCode(401)) => {
                err(tr("log.freellmapi_401", cur_lang()));
                return 1;
            }
            Err(ureq::Error::StatusCode(429)) => {
                err(tr("log.freellmapi_429", cur_lang()));
                return 1;
            }
            Err(e) => {
                err(&trv("log.freellmapi_login_err", cur_lang(), &[("e", &e)]));
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
        json!(if display_name.is_empty() {
            base_url
        } else {
            display_name
        }),
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
    out(tr("log.freellmapi_register_header", cur_lang()));
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
            out(&trv(
                "log.provider_registered",
                cur_lang(),
                &[("key_id", &key_id), ("platform", &platform)],
            ));
            if let Some(models) = v["models"].as_array() {
                let names: Vec<String> = models
                    .iter()
                    .filter_map(|m| m.as_str().map(String::from))
                    .collect();
                if !names.is_empty() {
                    out(&trv(
                        "log.models_list",
                        cur_lang(),
                        &[("names", &names.join(", "))],
                    ));
                }
            }
            out(tr("log.freellmapi_done", cur_lang()));
            0
        }
        Err(ureq::Error::StatusCode(code @ (401 | 403))) => {
            err(&trv(
                "log.freellmapi_auth_invalid",
                cur_lang(),
                &[("code", &code)],
            ));
            1
        }
        Err(ureq::Error::StatusCode(400)) => {
            err(tr("log.freellmapi_400", cur_lang()));
            1
        }
        Err(e) => {
            err(&trv("log.freellmapi_req_err", cur_lang(), &[("e", &e)]));
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
    let e = find_provider(&list, &id).ok_or(tr("err.provider_not_found", cur_lang()))?;
    let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let (protocol, via, base_url) = (s("protocol"), s("connectVia"), s("baseUrl"));
    valid_base_url(&base_url)?;
    // active_provider_key hits the Windows Credential Manager (a blocking syscall that can stall) —
    // move it off the async runtime. read_myproviders_raw above is a fast local fs read, left inline.
    let e_owned = e.clone();
    let id_for_key = id.clone();
    let api_key = tokio::task::spawn_blocking(move || active_provider_key(&id_for_key, &e_owned))
        .await
        .map_err(|err| err.to_string())?
        .ok_or(tr("err.provider_no_apikey", cur_lang()))?;

    match (via.as_str(), protocol.as_str()) {
        // Anthropic-native → bind straight to a profile's settings.json (native Manage-Provider).
        ("direct", "anthropic") => {
            let name = s("targetProfile");
            if !valid_profile_name(&name) {
                return Err(tr("err.direct_needs_profile", cur_lang()).into());
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
                return Err(tr("err.freellmapi_login_first", cur_lang()).into());
            }
            let gateway = gateway_base_url().ok_or(tr("err.no_gateway", cur_lang()))?;
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
        _ => Err(trv("err.unknown_connectvia_protocol", cur_lang(), &[("via", &via), ("protocol", &protocol)])),
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
    // M3: serialize the read-modify-write of myproviders.json under the same lock the other four
    // mutators (save/delete/add_key/remove_key) take — without it a concurrent add/save could lose
    // this update. Scoped to the RMW only: the guard MUST be dropped before the connect .await below
    // (a std MutexGuard held across .await would make the future non-Send and could deadlock connect).
    {
        let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut list = read_myproviders_checked()?;
        let pos = find_provider_idx(&list, &id).ok_or(tr("err.provider_not_found", cur_lang()))?;
        let (count, active) = key_pool_meta(&list[pos]);
        if count < 2 {
            return Err(tr("err.single_key", cur_lang()).into());
        }
        let next = next_key_index(active, count);
        list[pos]["activeKey"] = serde_json::json!(next);
        write_myproviders_raw(&list)?;
    }
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

/// Provider auth headers for a ureq GET: anthropic → x-api-key + anthropic-version; else Bearer.
/// Empty key → no key header. Extracted so probe_provider and balance_get can't drift apart.
fn provider_auth_headers(protocol: &str, key: &str) -> Vec<(&'static str, String)> {
    let mut h: Vec<(&'static str, String)> = Vec::new();
    if protocol == "anthropic" {
        if !key.is_empty() {
            h.push(("x-api-key", key.to_string()));
        }
        h.push(("anthropic-version", "2023-06-01".to_string()));
    } else if !key.is_empty() {
        h.push(("Authorization", format!("Bearer {key}")));
    }
    h
}

/// Guard for the outbound probe: `valid_base_url` (scheme + SSRF/metadata) plus an https requirement.
/// The probe sends `Authorization: Bearer <key>`, so a plaintext http:// to a non-loopback host would
/// leak the key on the wire — http:// is allowed only for genuine loopback (localhost / 127.0.0.0/8 / ::1).
fn probe_url_allowed(base_url: &str) -> Result<(), String> {
    valid_base_url(base_url)?;
    if let Some(rest) = base_url.trim().strip_prefix("http://") {
        let host_port = rest.split('/').next().unwrap_or("");
        let host = extract_host(host_port); // L3: shared IPv6-aware host extraction
        let hl = host.to_ascii_lowercase();
        // Only exact "localhost" or a host that parses as a loopback IP counts. A prefix/substring
        // test (`starts_with("127.")`) would wrongly allow a non-IP hostname like `127.0.0.1.evil.com`
        // or userinfo like `127.0.0.1@evil.com` (real host = evil.com) — leaking the bearer key over
        // http to an attacker-influenced DNS name. Parsing rejects anything that isn't a bare IP.
        let loopback = hl == "localhost"
            || hl
                .parse::<std::net::Ipv4Addr>()
                .map(|a| a.is_loopback())
                .unwrap_or(false)
            || hl
                .parse::<std::net::Ipv6Addr>()
                .map(|a| a.is_loopback())
                .unwrap_or(false);
        if !loopback {
            return Err(tr("err.https_required", cur_lang()).into());
        }
    }
    Ok(())
}

/// Native provider liveness probe (was Check-Provider.ps1). Blocking — call via spawn_blocking.
/// GET {root}/v1/models with the optional key; returns `{ ok, detail, count? }` (same shape as before).
fn probe_provider(base_url: &str, protocol: &str, api_key: &str) -> serde_json::Value {
    // Validate the target before sending the bearer key (SSRF + https-only, loopback excepted).
    if let Err(e) = probe_url_allowed(base_url) {
        return serde_json::json!({ "ok": false, "detail": e });
    }
    // Normalize: strip a trailing /v1, then always query /v1/models (works with or without /v1).
    let root = base_url.trim_end_matches('/');
    let root = root.strip_suffix("/v1").unwrap_or(root);
    let url = format!("{root}/v1/models");

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(12)))
        .build()
        .into();
    let mut req = agent.get(&url);
    for (k, v) in provider_auth_headers(protocol, api_key) {
        req = req.header(k, &v);
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
            serde_json::json!({ "ok": true, "detail": trv("det.responded_models", cur_lang(), &[("n", &n)]), "count": n })
        }
        Err(ureq::Error::StatusCode(code)) => {
            // An HTTP status means the server is ALIVE (it answered). Only auth failure is a real
            // problem; any other status (e.g. 404 — routers/bridges like ccr have no /v1/models)
            // still means "responding".
            if code == 401 || code == 403 {
                serde_json::json!({ "ok": false, "detail": trv("det.key_rejected", cur_lang(), &[("code", &code)]) })
            } else {
                serde_json::json!({ "ok": true, "detail": trv("det.responds_http", cur_lang(), &[("code", &code)]) })
            }
        }
        Err(e) => {
            serde_json::json!({ "ok": false, "detail": trv("det.no_response", cur_lang(), &[("e", &e)]) })
        }
    }
}

/// Native model list (was Get-EngineModels.ps1). Blocking — call via spawn_blocking.
/// GET <base>/models (or /v1/models for a bare host). Returns model ids; empty on any error.
fn fetch_engine_models(base_url: &str) -> Vec<String> {
    if base_url.is_empty() {
        return Vec::new();
    }
    // M2: SSRF guard before the outbound GET — every sibling outbound fetch validates the target
    // first; this "fetch models" preview was the one that didn't. Empty on a blocked/invalid URL
    // matches this function's "empty on any error" contract.
    if valid_base_url(base_url).is_err() {
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
    let body = match agent
        .get(&url)
        .header("Authorization", "Bearer not-needed")
        .call()
    {
        Ok(mut resp) => resp.body_mut().read_to_string().unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Vec::new();
    };
    let arr = v
        .get("data")
        .and_then(|x| x.as_array())
        .or_else(|| v.as_array());
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
    let (b, p, k) = (
        base_url.to_string(),
        protocol.to_string(),
        api_key.to_string(),
    );
    tokio::task::spawn_blocking(move || probe_provider(&b, &p, &k))
        .await
        .unwrap_or_else(|e| serde_json::json!({ "ok": false, "detail": format!("{e}") }))
}

/// Liveness check for a saved custom provider: key read from the Credential Manager.
#[tauri::command]
async fn check_my_provider(id: String) -> serde_json::Value {
    let list = read_myproviders_raw();
    let entry = find_provider(&list, &id).cloned();
    let Some(e) = entry else {
        return serde_json::json!({ "ok": false, "detail": tr("err.provider_not_found", cur_lang()) });
    };
    let base_url = e
        .get("baseUrl")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let protocol = e
        .get("protocol")
        .and_then(|x| x.as_str())
        .unwrap_or("openai")
        .to_string();
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
        cur = if let Ok(i) = seg.parse::<usize>() {
            cur.get(i)?
        } else {
            cur.get(seg)?
        };
    }
    match cur {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// GET a balance endpoint with the provider's auth header; parse the JSON body.
fn balance_get(
    agent: &ureq::Agent,
    url: &str,
    protocol: &str,
    key: &str,
) -> Option<serde_json::Value> {
    let mut req = agent.get(url);
    for (k, v) in provider_auth_headers(protocol, key) {
        req = req.header(k, &v);
    }
    let body = req.call().ok()?.body_mut().read_to_string().ok()?;
    serde_json::from_str(&body).ok()
}

/// Extract (amount, currency) from a balance response across common shapes.
fn extract_balance(v: &serde_json::Value) -> Option<(f64, String)> {
    let cur = v
        .get("currency")
        .and_then(|x| x.as_str())
        .or_else(|| {
            v.pointer("/balance_infos/0/currency")
                .and_then(|x| x.as_str())
        })
        .unwrap_or("")
        .to_string();
    // Prefer keys that mean "remaining balance"; treat limit/quota fields as last-resort fallbacks
    // so we don't report a hard limit (e.g. hard_limit_usd) as if it were the available balance.
    for p in [
        "remaining",
        "balance",
        "data.balance",
        "balance_infos.0.total_balance",
        "total_balance",
        "data.quota",
        "quota",
        "hard_limit_usd",
    ] {
        if let Some(n) = json_f64(v, p) {
            return Some((n, cur));
        }
    }
    None
}

fn fetch_provider_balance(id: &str) -> serde_json::Value {
    let list = read_myproviders_raw();
    let Some(e) = find_provider(&list, id) else {
        return serde_json::json!({ "ok": false, "detail": tr("err.provider_not_found", cur_lang()) });
    };
    let base = e.get("baseUrl").and_then(|x| x.as_str()).unwrap_or("");
    let protocol = e
        .get("protocol")
        .and_then(|x| x.as_str())
        .unwrap_or("openai");
    let balance_url = e.get("balanceUrl").and_then(|x| x.as_str()).unwrap_or("");
    let key = active_provider_key(id, e).unwrap_or_default();
    let root = base
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches('/');

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();

    // 1) User-configured balance URL — most reliable. SSRF-guard it like baseUrl (R2-01): it is
    // queried WITH the provider's key, so a link-local / cloud-metadata host must be rejected.
    if !balance_url.is_empty() {
        if let Err(detail) = valid_base_url(balance_url) {
            return serde_json::json!({ "ok": false, "detail": detail });
        }
        return match balance_get(&agent, balance_url, protocol, &key) {
            Some(v) => match extract_balance(&v) {
                Some((amt, cur)) => {
                    serde_json::json!({ "ok": true, "amount": amt, "currency": cur, "detail": "" })
                }
                None => {
                    serde_json::json!({ "ok": false, "detail": tr("det.no_balance_number", cur_lang()) })
                }
            },
            None => {
                serde_json::json!({ "ok": false, "detail": tr("det.balance_no_response", cur_lang()) })
            }
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
    if let Some(v) = balance_get(
        &agent,
        &format!("{root}/dashboard/billing/subscription"),
        protocol,
        &key,
    ) {
        if let Some(amt) =
            json_f64(&v, "hard_limit_usd").or_else(|| json_f64(&v, "system_hard_limit_usd"))
        {
            return serde_json::json!({ "ok": true, "amount": amt, "currency": "USD", "detail": tr("det.limit", cur_lang()) });
        }
    }
    serde_json::json!({ "ok": false, "detail": tr("det.balance_unavailable", cur_lang()) })
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

/// After a cache entry goes stale (>60s), a FAILED re-fetch serves the last-good value instead of
/// blanking the badge (flicker fix, live-smoke 2026-07-03) — but only until it is this old, so a
/// genuinely logged-out/removed profile eventually clears instead of showing forever-stale numbers.
const USAGE_STALE_MAX_SECS: u64 = 900; // 15 min

/// Blocking: read a profile's OAuth token and query the usage endpoint. None on any failure
/// (not logged in / token expired / offline) so the UI just omits the badge.
fn fetch_profile_usage(profile: &str) -> Option<ProfileUsage> {
    let home = std::env::var("USERPROFILE").ok()?;
    let creds = format!("{home}\\.claude-{profile}\\.credentials.json");
    let content = std::fs::read_to_string(&creds).ok()?;
    let v: serde_json::Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).ok()?;
    let token = v
        .get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()?
        .to_string();
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
    let pct = |k: &str| {
        r.get(k)
            .and_then(|x| x.get("utilization"))
            .and_then(|x| x.as_f64())
    };
    let reset = |k: &str| {
        r.get(k)
            .and_then(|x| x.get("resets_at"))
            .and_then(|x| x.as_str())
            .map(String::from)
    };
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
    let fetched = tokio::task::spawn_blocking(move || fetch_profile_usage(&p))
        .await
        .ok()
        .flatten();
    let mut map = cache.0.lock().unwrap_or_else(|e| e.into_inner());
    match fetched {
        Some(u) => {
            map.insert(profile, (std::time::Instant::now(), u.clone()));
            Ok(Some(u))
        }
        // A transient re-fetch failure (rate-limit / offline / a busy account under load) must NOT
        // blank the badge — that oscillation was the flicker (live-smoke 2026-07-03). Serve the
        // last-good value until it ages past USAGE_STALE_MAX_SECS, then a truly gone profile clears.
        // ponytail: 401 is treated like any transient error here (stale ≤15 min, then clears); a
        // precise "token revoked → clear now" would need fetch to return Ok/expired/transient rather
        // than collapsing every error to None. Not worth the surface for the flicker fix.
        None => Ok(map
            .get(&profile)
            .filter(|(at, _)| at.elapsed().as_secs() < USAGE_STALE_MAX_SECS)
            .map(|(_, u)| u.clone())),
    }
}

// --- Sessions personalization sidecar (item 18 / Gap 1) --------------------------------------
// The Sessions tab's prefs (workspaces, favorites, folders, columns, monitor layout, defaults …)
// lived ONLY in webview localStorage — lost on reinstall, absent from the backup snapshot, and
// outside the Syncthing-synced ~/.claude set. This durable sidecar is their real home: a plain JSON
// file under ~/.claude/castellyn/, so it (a) survives a reinstall, (b) is standalone-readable,
// (c) rides the existing ~/.claude Syncthing sync (whitelisted in sync_item_lines), and (d) is copied
// by the backup script (Add-Source). The frontend keeps localStorage as a fast mirror; file = truth.
// NOT a secret file, so write_json_atomic keeps its .bak crash-safety. Never holds live-pane state
// (cmh-sessions-live is machine-local and deliberately excluded on the frontend).
fn sessions_prefs_path() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| format!("{h}\\.claude\\castellyn\\sessions.json"))
}

/// Read the durable Sessions-prefs sidecar (BOM-tolerant). `None` when it doesn't exist yet.
#[tauri::command]
fn read_sessions_prefs() -> Result<Option<String>, String> {
    let path = sessions_prefs_path().ok_or("no USERPROFILE")?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s.trim_start_matches('\u{feff}').to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Write the durable Sessions-prefs sidecar (atomic, UTF-8 no BOM; creates ~/.claude/castellyn/).
/// The frontend owns the JSON shape (a flat map of the mirrored cmh-* keys → their stored strings).
#[tauri::command]
fn write_sessions_prefs(json: String) -> Result<(), String> {
    let path = sessions_prefs_path().ok_or("no USERPROFILE")?;
    write_json_atomic(&path, &json).map_err(|e| e.to_string())
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
            return OpencodeStatus {
                installed: false,
                model: String::new(),
                providers: Vec::new(),
            }
        }
    };
    let v = match parse_json_bom(&content) {
        Ok(v) => v,
        Err(_) => {
            return OpencodeStatus {
                installed: true,
                model: String::new(),
                providers: Vec::new(),
            }
        }
    };
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let mut providers = Vec::new();
    if let Some(obj) = v.get("provider").and_then(|p| p.as_object()) {
        for (id, p) in obj {
            let opts = p.get("options");
            providers.push(OpencodeProvider {
                id: id.clone(),
                name: p
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or(id)
                    .to_string(),
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
    OpencodeStatus {
        installed: true,
        model,
        providers,
    }
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
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4 (called sync from the async run_opencode_provider — no guard held across .await)
    use serde_json::{json, Value};

    // Load opencode.json (BOM-tolerant) or start minimal; a parse failure aborts (as the script does).
    let mut cfg: Value = match std::fs::read_to_string(cfg_path) {
        Ok(ref c) if !c.trim().is_empty() => match parse_json_bom(c) {
            Ok(v) => v,
            Err(e) => {
                err(&trv("log.read_opencode", cur_lang(), &[("e", &e)]));
                return 1;
            }
        },
        _ => json!({}),
    };
    if !cfg.is_object() {
        cfg = json!({});
    }
    let obj = cfg.as_object_mut().unwrap();
    obj.entry("$schema")
        .or_insert_with(|| json!("https://opencode.ai/config.json"));
    if !obj.get("provider").map(|p| p.is_object()).unwrap_or(false) {
        obj.insert("provider".into(), json!({}));
    }

    out(&format!(
        "=== opencode provider: {action} {provider_id} ==="
    ));

    if action == "set" {
        let mut active_model: Option<String> = None;
        {
            let providers = obj.get_mut("provider").unwrap().as_object_mut().unwrap();
            if !providers
                .get(provider_id)
                .map(|x| x.is_object())
                .unwrap_or(false)
            {
                providers.insert(provider_id.to_string(), json!({}));
            }
            let p = providers
                .get_mut(provider_id)
                .unwrap()
                .as_object_mut()
                .unwrap();
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
            tr("log.unchanged", cur_lang()).to_string()
        } else if key.filter(|s| !s.is_empty()).is_some() {
            tr("log.literal", cur_lang()).to_string()
        } else if let Some(e) = env_key.filter(|s| !s.is_empty()) {
            format!("{{env:{e}}}")
        } else {
            tr("log.unchanged", cur_lang()).to_string()
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
        out(&trv(
            "log.provider_removed",
            cur_lang(),
            &[("provider_id", &provider_id)],
        ));
    }

    // Backup + atomic write (temp+rename, UTF-8 no BOM).
    let serialized = match serde_json::to_string_pretty(&cfg) {
        Ok(s) => s,
        Err(e) => {
            err(&trv("log.ser_opencode", cur_lang(), &[("e", &e)]));
            return 1;
        }
    };
    if let Err(e) = write_json_atomic(cfg_path, &serialized) {
        err(&trv("log.write_opencode", cur_lang(), &[("e", &e)]));
        return 1;
    }
    out(&trv(
        "log.opencode_updated",
        cur_lang(),
        &[("cfg_path", &cfg_path)],
    ));
    0
}

/// Manage-OpenCode-Provider (native): merge-patch of opencode.json. apiKey: literal `key`, else
/// `{env:env_key}`, else keep existing.
#[tauri::command]
// command handler: args come from the JS invoke boundary
#[allow(clippy::too_many_arguments)]
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
        return Err(trv(
            "err.unknown_opencode_action",
            cur_lang(),
            &[("action", &action)],
        ));
    }
    if !valid_profile_name(&provider_id) {
        return Err(trv(
            "err.invalid_provider_id",
            cur_lang(),
            &[("id", &provider_id)],
        ));
    }
    let base_url = base_url.unwrap_or_default();
    if action == "set" && base_url.is_empty() {
        return Err(tr("err.set_needs_base_url", cur_lang()).into());
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
    /// The server's full canonical definition (so the edit form can prefill the raw JSON).
    definition: serde_json::Value,
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
    let mut source_defs: Vec<(String, String, serde_json::Value)> = Vec::new(); // (name, command, def)
    if let Ok(content) = std::fs::read_to_string(abs(MCP_CONFIG_REL)) {
        if let Ok(v) = parse_json_bom(&content) {
            if let Some(obj) = v.get("mcpServers").and_then(|m| m.as_object()) {
                for (name, def) in obj {
                    let cmd = def
                        .get("command")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    source_defs.push((name.clone(), cmd, def.clone()));
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
        .map(|(name, cmd, def)| {
            let deployed_in = per_profile
                .iter()
                .filter(|(_, servers)| servers.iter().any(|s| s == name))
                .map(|(p, _)| p.clone())
                .collect();
            McpServer {
                name: name.clone(),
                command: cmd.clone(),
                definition: def.clone(),
                deployed_in,
            }
        })
        .collect();

    // Servers found in a profile but absent from the source-of-truth.
    let source_names: std::collections::HashSet<&str> =
        source_defs.iter().map(|(n, _, _)| n.as_str()).collect();
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

    Ok(McpStatus {
        source,
        extras,
        profiles: existing_profiles,
    })
}

/// Serialize lock for the canonical .mcp.json: add/edit/remove all read-modify-write it.
static MCP_LOCK: Mutex<()> = Mutex::new(());

// M4/L4: serialize every deploy-config read-modify-write — opencode.json, ~/.codex/config.toml, and
// the shared managed_mcp ledger in config.json. Each writer rewrites a whole file (or a shared ledger
// key), so two deploys fired together lose one update (and the "deployed N" toast lies). All these
// writers are synchronous fns (blocking process spawns, no .await), so a std Mutex held across the
// body is Send-safe; never nested — each is an independent top-level deploy command.
static DEPLOY_CFG_LOCK: Mutex<()> = Mutex::new(());

/// Read the canonical .mcp.json doc; defaults to {"mcpServers":{}} when absent. Errs (so a caller
/// aborts instead of overwriting) when present-but-corrupt, recovering from .bak first.
fn read_mcp_doc() -> Result<serde_json::Value, String> {
    Ok(read_json_or_recover(abs(MCP_CONFIG_REL), ".mcp.json")?
        .unwrap_or_else(|| serde_json::json!({ "mcpServers": {} })))
}

fn write_mcp_doc(doc: &serde_json::Value) -> Result<(), String> {
    let json = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    write_json_atomic(&abs(MCP_CONFIG_REL), &json).map_err(|e| trv("err.mcp_write", cur_lang(), &[("e", &e)]))
}

/// Add or replace one server in the canonical config\.mcp.json. `definition` is the server's JSON
/// object (e.g. {"command":"npx","args":[...]}); it is validated to be an object before writing.
#[tauri::command]
fn mcp_upsert_server(name: String, definition: String) -> Result<(), String> {
    let _guard = MCP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("err.mcp_name_required", cur_lang()).to_string());
    }
    let def = parse_json_bom(&definition).map_err(|e| trv("err.mcp_invalid_json", cur_lang(), &[("e", &e)]))?;
    if !def.is_object() {
        return Err(tr("err.mcp_def_not_object", cur_lang()).to_string());
    }
    let mut doc = read_mcp_doc()?;
    if !doc
        .get("mcpServers")
        .map(|m| m.is_object())
        .unwrap_or(false)
    {
        doc["mcpServers"] = serde_json::json!({});
    }
    doc["mcpServers"][name] = def;
    write_mcp_doc(&doc)
}

/// Remove one server from the canonical config\.mcp.json.
#[tauri::command]
fn mcp_remove_server(name: String) -> Result<(), String> {
    let _guard = MCP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = read_mcp_doc()?;
    if let Some(obj) = doc.get_mut("mcpServers").and_then(|m| m.as_object_mut()) {
        obj.remove(&name);
    }
    write_mcp_doc(&doc)
}

/// Remove one "extra" server (present in a profile but not in the canonical set) from that profile's
/// live .claude.json — the action behind the Mcp-tab extras list. Preserves the rest of the file.
#[tauri::command]
fn mcp_remove_extra(name: String, profile: String) -> Result<(), String> {
    if !valid_profile_name(&profile) {
        return Err(trv(
            "err.invalid_provider_id",
            cur_lang(),
            &[("id", &profile)],
        ));
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let path = format!("{home}\\.claude-{profile}\\.claude.json");
    let mut doc = read_json_or_recover(&path, ".claude.json")?
        .ok_or_else(|| format!("profile {profile} has no .claude.json"))?;
    if let Some(obj) = doc.get_mut("mcpServers").and_then(|m| m.as_object_mut()) {
        obj.remove(&name);
    }
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    write_json_atomic(&path, &json).map_err(|e| format!("write .claude.json: {e}"))
}

const SCHEDULE_SCRIPT_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\Schedule-Hub.ps1";
const SCHEDULES_JSON_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\schedules.last.json";

/// Refresh (run the helper's query) and read schedules.last.json. Not streamed.
#[tauri::command]
async fn read_schedules() -> Result<Option<serde_json::Value>, String> {
    let script = abs(SCHEDULE_SCRIPT_REL);
    let fut = tokio::process::Command::new("pwsh")
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
        .output();
    // Best-effort refresh, bounded — if the query hangs, fall through and read the last JSON anyway.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), fut).await;
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
        return Err(trv(
            "err.unknown_schedule_action",
            cur_lang(),
            &[("action", &action)],
        ));
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
        _ => {
            return Err(trv(
                "err.unknown_mcp_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
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

/// Resolve the Claude plugins dir + parsed installed_plugins.json + known_marketplaces.json.
/// None when USERPROFILE is unset or installed_plugins.json is missing/unparseable.
fn load_installed_plugins() -> Option<(String, serde_json::Value, serde_json::Value)> {
    let home = std::env::var("USERPROFILE").ok()?;
    let plugins_dir = format!("{home}\\.claude\\plugins");
    let installed = std::fs::read_to_string(format!("{plugins_dir}\\installed_plugins.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())?;
    let markets = std::fs::read_to_string(format!("{plugins_dir}\\known_marketplaces.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .unwrap_or(serde_json::Value::Null);
    Some((plugins_dir, installed, markets))
}

/// First entry's `installPath` in an installed_plugins.json plugin array ("" if absent).
fn first_install_path(arr: &serde_json::Value) -> &str {
    arr.as_array()
        .and_then(|a| a.first())
        .and_then(|e| e.get("installPath"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Map plugin id → description, read from each installed plugin's .claude-plugin/plugin.json
/// (the `claude plugin list --json` output has no description).
fn plugin_descriptions() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Some((_dir, installed, markets)) = load_installed_plugins() else {
        return map;
    };
    let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) else {
        return map;
    };
    for (id, arr) in po {
        let install_path = first_install_path(arr);
        if let Some(dir) = plugin_content_dir(id, install_path, &markets) {
            let pj = std::path::Path::new(&dir)
                .join(".claude-plugin")
                .join("plugin.json");
            if let Some(d) = std::fs::read_to_string(pj)
                .ok()
                .and_then(|c| parse_json_bom(&c).ok())
                .and_then(|m| {
                    m.get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
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
    let fut = tokio::process::Command::new("pwsh")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "claude plugin list --json",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let out = match tokio::time::timeout(std::time::Duration::from_secs(30), fut).await {
        Ok(r) => r.map_err(|e| trv("err.claude_launch", cur_lang(), &[("e", &e)]))?,
        Err(_) => {
            return Err(trv(
                "err.claude_launch",
                cur_lang(),
                &[("e", &"timed out".to_string())],
            ))
        }
    };
    // Surface a clean error when `claude` is missing/fails, not the confusing "parse plugins" below.
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(trv("err.claude_launch", cur_lang(), &[("e", &msg)]));
    }
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
    let Ok(home) = std::env::var("USERPROFILE") else {
        return set;
    };
    let p = format!("{home}\\.claude\\plugins\\known_marketplaces.json");
    if let Some(v) = std::fs::read_to_string(p)
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
    {
        let obj = v
            .get("marketplaces")
            .and_then(|m| m.as_object())
            .or_else(|| v.as_object());
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
    SkillInfo {
        name,
        description,
        version,
        dir: skill_dir.display().to_string(),
        source,
        mine,
    }
}

/// Skills bundled inside installed plugins (source = "plugin:<id>").
fn plugin_bundled_skills() -> Vec<SkillInfo> {
    let Some((_dir, installed, markets)) = load_installed_plugins() else {
        return Vec::new();
    };
    let own = own_marketplaces();
    let mut out: Vec<SkillInfo> = Vec::new();
    let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) else {
        return out;
    };
    for (id, arr) in po {
        let install_path = first_install_path(arr);
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
                if is_link {
                    "own".into()
                } else {
                    "default".into()
                },
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

// ---- Environments overview (read-only) -----------------------------------------------------
// Coverage matrix across the coding harnesses installed on this machine: how many skills each
// can see, whether plugin-bundled skills reach it, provider count, and RTK wiring. Pure reads —
// the "share skills" write is a separate command. Skill discovery follows each harness's own
// docs: Claude reads ~/.claude/skills (+ its plugin system); OpenCode reads ~/.claude/skills,
// ~/.agents/skills and ~/.config/opencode/skills; Codex reads ~/.agents/skills (user-level) and
// its own ~/.codex/skills. So ~/.agents/skills is the one folder both OpenCode and Codex honor.

/// Names of immediate sub-directories that contain a SKILL.md (case-insensitive on Windows).
/// `is_dir()`/`is_file()` follow symlinks/junctions, so linked "own" skills are counted.
fn skill_names_in(dir: &str) -> std::collections::BTreeSet<String> {
    let mut set = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && p.join("SKILL.md").is_file() {
                if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                    set.insert(n.to_string());
                }
            }
        }
    }
    set
}

/// Owned skill-name sets per harness — computed once, shared by read_environments + read_skill_matrix.
#[derive(Clone)]
struct SkillSets {
    claude_visible: std::collections::BTreeSet<String>,
    opencode_visible: std::collections::BTreeSet<String>,
    codex_visible: std::collections::BTreeSet<String>,
    source_names: std::collections::BTreeSet<String>, // names `share_skills` links (real claude + plugins)
    plugin_names: std::collections::BTreeSet<String>,
    universe: std::collections::BTreeSet<String>,
}

/// OpenCode's skills dir, derived from the (env-aware) config path so `$OPENCODE_CONFIG` /
/// `$XDG_CONFIG_HOME` are honored instead of a hardcoded `~/.config/opencode`.
fn opencode_skills_dir(home: &str) -> String {
    let cfg = opencode_config_path();
    std::path::Path::new(&cfg)
        .parent()
        .map(|p| p.join("skills").to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{home}\\.config\\opencode\\skills"))
}

/// Real (non-symlink) skill dirs in `~/.claude/skills` + every plugin-bundled skill → (name, src_dir).
/// Single source of what `share_skills` links; also drives `source_names` in read_environments.
fn shareable_skill_sources(home: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(format!("{home}\\.claude\\skills")) {
        for e in entries.flatten() {
            let p = e.path();
            let is_link = std::fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if is_link || !p.is_dir() || !p.join("SKILL.md").is_file() {
                continue;
            }
            if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                out.push((n.to_string(), p.to_string_lossy().to_string()));
            }
        }
    }
    for s in plugin_bundled_skills() {
        if !s.dir.is_empty() {
            out.push((s.name, s.dir));
        }
    }
    out
}

/// Every harness's reachable skill-name set, per each tool's documented discovery paths.
fn skill_sets(home: &str) -> SkillSets {
    use std::collections::BTreeSet;
    // Scan ~/.claude/skills once → all names (visibility) + real, non-symlink names. `source_names`
    // (= real claude skills + plugins) is exactly what `share_skills` links, so the gap closes after
    // a share; symlinked entries already live in ~/.agents/skills and don't need re-linking.
    let (mut claude_all, mut claude_real) = (BTreeSet::new(), BTreeSet::new());
    if let Ok(entries) = std::fs::read_dir(format!("{home}\\.claude\\skills")) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() || !p.join("SKILL.md").is_file() {
                continue;
            }
            if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                claude_all.insert(n.to_string());
                let is_link = std::fs::symlink_metadata(&p)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if !is_link {
                    claude_real.insert(n.to_string());
                }
            }
        }
    }
    let agents = skill_names_in(&format!("{home}\\.agents\\skills"));
    let opencode_skills = skill_names_in(&opencode_skills_dir(home));
    let codex_skills = skill_names_in(&format!("{home}\\.codex\\skills"));
    let plugin_names: BTreeSet<String> = plugin_bundled_skills()
        .into_iter()
        .map(|s| s.name)
        .collect();

    let union = |parts: &[&BTreeSet<String>]| -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for p in parts {
            out.extend(p.iter().cloned());
        }
        out
    };
    SkillSets {
        claude_visible: union(&[&claude_all, &plugin_names]),
        opencode_visible: union(&[&claude_all, &agents, &opencode_skills]),
        codex_visible: union(&[&agents, &codex_skills]),
        universe: union(&[
            &claude_all,
            &agents,
            &opencode_skills,
            &codex_skills,
            &plugin_names,
        ]),
        source_names: union(&[&claude_real, &plugin_names]),
        plugin_names,
    }
}

/// Short-TTL memo over skill_sets: opening the Environments tab fires read_environments AND
/// read_skill_matrix back-to-back, each doing the full multi-dir skill walk. A 2 s window lets the
/// second reuse the first's result. ponytail: a tiny self-expiring TTL (worst case 2 s stale, then
/// self-heals) instead of a global cache that every plugin/skill mutation would have to invalidate.
fn skill_sets_cached(home: &str) -> SkillSets {
    use std::time::{Duration, Instant};
    static CACHE: std::sync::Mutex<Option<(Instant, SkillSets)>> = std::sync::Mutex::new(None);
    const TTL: Duration = Duration::from_secs(2);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = guard.as_ref() {
        if t.elapsed() < TTL {
            return v.clone();
        }
    }
    let fresh = skill_sets(home);
    *guard = Some((Instant::now(), fresh.clone()));
    fresh
}

/// True if the managed OpenCode RTK plugin's pinned rtk path still resolves — so "RTK on" can't lie
/// when the binary moved and the plugin self-disabled at runtime. Handles the JSON-string and the
/// legacy `String.raw` forms of `const RTK = …`.
fn rtk_plugin_path_ok(content: &str) -> bool {
    for line in content.lines() {
        if let Some(rest) = line.trim().strip_prefix("const RTK = ") {
            let rest = rest.trim().trim_end_matches(';').trim();
            if let Ok(p) = serde_json::from_str::<String>(rest) {
                return std::path::Path::new(&p).is_file();
            }
            if let Some(inner) = rest
                .strip_prefix("String.raw`")
                .and_then(|s| s.strip_suffix('`'))
            {
                return std::path::Path::new(inner).is_file();
            }
        }
    }
    true
}

/// Distinct TOML table names under `prefix` (e.g. `[mcp_servers.` → the server names) — skips
/// comments and dedups dotted sub-tables. Replaces the fragile `.matches().count()` heuristic.
fn toml_table_names(text: &str, prefix: &str) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix(prefix) {
            let key: String = rest
                .chars()
                .take_while(|c| *c != '.' && *c != ']')
                .collect();
            let key = key.trim().trim_matches('"').to_string();
            if !key.is_empty() {
                set.insert(key);
            }
        }
    }
    set.into_iter().collect()
}

fn count_toml_tables(text: &str, prefix: &str) -> usize {
    toml_table_names(text, prefix).len()
}

/// Gap-2 drift: a harness's MCP set has drifted from canon when a canon server is MISSING from it,
/// OR a server WE deployed (managed) is still present but no longer in canon (a de-canonized tail).
/// A user-added server (present, not in canon, not in managed) is NOT drift.
fn mcp_drift(canon: &[String], harness: &[String], managed: &[String]) -> bool {
    let hset: std::collections::HashSet<&str> = harness.iter().map(|s| s.as_str()).collect();
    if canon.iter().any(|n| !hset.contains(n.as_str())) {
        return true;
    }
    let cset: std::collections::HashSet<&str> = canon.iter().map(|s| s.as_str()).collect();
    managed
        .iter()
        .any(|n| hset.contains(n.as_str()) && !cset.contains(n.as_str()))
}

#[cfg(test)]
mod mcp_reconcile_tests {
    #[test]
    fn stale_names_removes_only_decanonized_managed() {
        let managed = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let canon = vec!["b".to_string(), "c".to_string(), "d".to_string()];
        // 'a' was ours and dropped from canon → remove; 'd' is new canon (not ours yet) → not here.
        assert_eq!(super::mcp_stale_names(&managed, &canon), vec!["a".to_string()]);
        // a user server never entered `managed`, so nothing is returned:
        assert!(super::mcp_stale_names(&[], &canon).is_empty());
        // nothing de-canonized → empty:
        assert!(super::mcp_stale_names(&["b".to_string()], &canon).is_empty());
    }

    #[test]
    fn drift_flags_missing_and_tail_but_not_user_servers() {
        let canon = vec!["a".to_string(), "b".to_string()];
        let both = vec!["a".to_string(), "b".to_string()];
        // in sync (harness == canon, ledger == canon) → no drift
        assert!(!super::mcp_drift(&canon, &both, &both));
        // a canon server missing from the harness → drift
        assert!(super::mcp_drift(&canon, &["a".to_string()], &both));
        // a de-canonized tail we deployed is still present → drift
        assert!(super::mcp_drift(&["a".to_string()], &both, &both));
        // a USER-added server (present, not canon, not in our ledger) → NOT drift
        assert!(!super::mcp_drift(
            &["a".to_string()],
            &["a".to_string(), "user".to_string()],
            &["a".to_string()]
        ));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvInfo {
    id: String,
    name: String,
    installed: bool,
    config_path: String,
    skills_visible: usize,
    total_skills: usize,
    plugin_skills_visible: bool,
    shareable_gap: usize,
    providers: usize,
    mcp_servers: usize,
    /// Gap-2: the harness's MCP set drifted from canon (missing canon server or a de-canonized tail
    /// we deployed) → the "Среды" card shows a "stale — Deploy" hint. User-added servers don't count.
    mcp_drift: bool,
    rtk: bool,
    rtk_available: bool,
    config_ok: bool,
}

/// Read-only coverage of every supported coding harness. Composes the existing native readers;
/// no script spawn. zcode is reported as a not-installed placeholder (no Windows config dir yet).
#[tauri::command]
fn read_environments() -> Vec<EnvInfo> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };

    let sets = skill_sets_cached(&home);
    let total = sets.universe.len();
    let plugins_in = |vis: &std::collections::BTreeSet<String>| -> bool {
        !sets.plugin_names.is_empty() && sets.plugin_names.iter().all(|n| vis.contains(n))
    };
    // Skills `share_skills` would newly make visible to each harness (0 ⇒ nothing left to share).
    let oc_gap = sets.source_names.difference(&sets.opencode_visible).count();
    let cx_gap = sets.source_names.difference(&sets.codex_visible).count();

    let rtk_present = resolve_rtk_path(&home).is_some();

    // Claude — RTK detected by the real hook command token, not a loose "rtk" substring.
    let claude_rtk = std::fs::read_to_string(format!("{home}\\.claude\\settings.json"))
        .map(|s| s.contains("rtk hook"))
        .unwrap_or(false);
    let claude_mcp = read_mcp().map(|m| m.source.len()).unwrap_or(0);

    // OpenCode — read & parse the config exactly once; derive providers + MCP + parse-health from it.
    let opencode_cfg = opencode_config_path();
    let opencode_txt = std::fs::read_to_string(&opencode_cfg).ok();
    let opencode_installed = opencode_txt.is_some();
    let opencode_json = opencode_txt.as_ref().and_then(|s| parse_json_bom(s).ok());
    let opencode_config_ok = !opencode_installed || opencode_json.is_some();
    let opencode_providers = opencode_json
        .as_ref()
        .and_then(|v| v.get("provider"))
        .and_then(|p| p.as_object())
        .map(|o| o.len())
        .unwrap_or(0);
    let opencode_mcp = opencode_json
        .as_ref()
        .and_then(|v| v.get("mcp").or_else(|| v.get("mcpServers")))
        .and_then(|m| m.as_object())
        .map(|o| o.len())
        .unwrap_or(0);
    // RTK "on" only when the managed plugin's pinned path still resolves (else it self-disabled).
    let opencode_rtk =
        std::fs::read_to_string(format!("{home}\\.config\\opencode\\plugins\\rtk.ts"))
            .map(|c| {
                if c.contains("Managed by Castellyn") {
                    rtk_plugin_path_ok(&c)
                } else {
                    true
                }
            })
            .unwrap_or(false);

    // Codex — TOML config; distinct-key tallies instead of a raw substring count.
    let codex_cfg = format!("{home}\\.codex\\config.toml");
    let codex_txt = std::fs::read_to_string(&codex_cfg).ok();
    let codex_installed = codex_txt.is_some();
    let codex_providers = codex_txt
        .as_deref()
        .map(|t| count_toml_tables(t, "[model_providers."))
        .unwrap_or(0);
    let codex_mcp = codex_txt
        .as_deref()
        .map(|t| count_toml_tables(t, "[mcp_servers."))
        .unwrap_or(0);

    // Gap-2 drift per harness: canon (.mcp.json) names vs the harness's current names vs our ledger.
    let hub = read_config_file();
    // Deployable canon only (a commandless server is never fanned out → excluding it keeps drift from
    // false-flagging "missing canon server" right after a clean deploy).
    let canon_mcp: Vec<String> = read_mcp()
        .map(|m| {
            m.source
                .iter()
                .filter(|s| !s.command.is_empty())
                .map(|s| s.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let opencode_names: Vec<String> = opencode_json
        .as_ref()
        .and_then(|v| v.get("mcp").or_else(|| v.get("mcpServers")))
        .and_then(|m| m.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let opencode_drift = mcp_drift(
        &canon_mcp,
        &opencode_names,
        &hub.managed_mcp
            .as_ref()
            .and_then(|m| m.opencode.clone())
            .unwrap_or_default(),
    );
    let codex_names: Vec<String> = codex_txt
        .as_deref()
        .map(|t| toml_table_names(t, "[mcp_servers."))
        .unwrap_or_default();
    let codex_drift = mcp_drift(
        &canon_mcp,
        &codex_names,
        &hub.managed_mcp
            .as_ref()
            .and_then(|m| m.codex.clone())
            .unwrap_or_default(),
    );

    let zcode_installed = std::path::Path::new(&format!("{home}\\.zcode")).is_dir()
        || std::path::Path::new(&format!("{home}\\.config\\zcode")).is_dir();

    vec![
        EnvInfo {
            id: "claude".into(),
            name: "Claude Code".into(),
            installed: std::path::Path::new(&format!("{home}\\.claude")).is_dir(),
            config_path: format!("{home}\\.claude"),
            skills_visible: sets.claude_visible.len(),
            total_skills: total,
            plugin_skills_visible: plugins_in(&sets.claude_visible),
            shareable_gap: 0, // sharing targets ~/.agents/skills, which Claude does not read
            providers: read_providers().len(),
            mcp_servers: claude_mcp,
            mcp_drift: false, // Claude self-reconciles via read_mcp extras / mcp_remove_extra
            rtk: claude_rtk,
            rtk_available: rtk_present,
            config_ok: true,
        },
        EnvInfo {
            id: "opencode".into(),
            name: "OpenCode".into(),
            installed: opencode_installed,
            config_path: opencode_cfg,
            skills_visible: sets.opencode_visible.len(),
            total_skills: total,
            plugin_skills_visible: plugins_in(&sets.opencode_visible),
            shareable_gap: oc_gap,
            providers: opencode_providers,
            mcp_servers: opencode_mcp,
            mcp_drift: opencode_drift,
            rtk: opencode_rtk,
            rtk_available: rtk_present,
            config_ok: opencode_config_ok,
        },
        EnvInfo {
            id: "codex".into(),
            name: "Codex".into(),
            installed: codex_installed,
            config_path: codex_cfg,
            skills_visible: sets.codex_visible.len(),
            total_skills: total,
            plugin_skills_visible: plugins_in(&sets.codex_visible),
            shareable_gap: cx_gap,
            providers: codex_providers,
            mcp_servers: codex_mcp,
            mcp_drift: codex_drift,
            rtk: false,
            rtk_available: false,
            config_ok: codex_installed,
        },
        EnvInfo {
            id: "zcode".into(),
            name: "ZCode".into(),
            installed: zcode_installed,
            config_path: String::new(),
            skills_visible: 0,
            total_skills: total,
            plugin_skills_visible: false,
            shareable_gap: 0,
            providers: 0,
            mcp_servers: 0,
            mcp_drift: false,
            rtk: false,
            rtk_available: false,
            config_ok: true,
        },
    ]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillRow {
    name: String,
    claude: bool,
    opencode: bool,
    codex: bool,
    shareable: bool, // present in a shareable source but missing from OpenCode or Codex
}

/// Per-skill visibility matrix across harnesses (#20) — turns the n/total gauge into a diff.
/// Reuses the same sets as read_environments via `skill_sets`; pure reads.
#[tauri::command]
fn read_skill_matrix() -> Vec<SkillRow> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let sets = skill_sets_cached(&home);
    let mut rows: Vec<SkillRow> = sets
        .universe
        .iter()
        .map(|name| {
            let opencode = sets.opencode_visible.contains(name);
            let codex = sets.codex_visible.contains(name);
            SkillRow {
                name: name.clone(),
                claude: sets.claude_visible.contains(name),
                opencode,
                codex,
                shareable: sets.source_names.contains(name) && (!opencode || !codex),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        // Skills that still need sharing first, then alphabetical.
        b.shareable
            .cmp(&a.shareable)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    rows
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareResult {
    created: usize,
    skipped: usize,
    failed: usize,
    target: String,
    details: Vec<String>, // names that failed to link
}

/// Make every skill (regular + plugin-bundled) reachable from ~/.agents/skills — the one folder both
/// OpenCode and Codex scan at user level (per their docs). Idempotent "ensure": create missing
/// junctions, repair dangling ones (stale plugin-cache targets after an update), skip correct ones.
/// Never deletes a real directory or a still-valid link; mklink /J needs no admin. Claude is untouched.
#[tauri::command]
fn share_skills() -> Result<ShareResult, String> {
    let home = std::env::var("USERPROFILE").map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let target_dir = format!("{home}\\.agents\\skills");
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("create {target_dir}: {e}"))?;

    let mut res = ShareResult {
        created: 0,
        skipped: 0,
        failed: 0,
        target: target_dir.clone(),
        details: Vec::new(),
    };
    for (name, src) in shareable_skill_sources(&home) {
        // Reject names that could break out of the mklink argv (cmd re-parses its arguments).
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            res.failed += 1;
            res.details
                .push(format!("{name}: unsafe skill name, skipped"));
            continue;
        }
        let link = format!("{target_dir}\\{name}");
        let lp = std::path::Path::new(&link);
        // symlink_metadata (lstat) sees the junction node even when its target is gone; `exists()`
        // follows the link and would report `false` for a dangling junction, defeating the repair.
        if std::fs::symlink_metadata(lp).is_ok() {
            if std::fs::metadata(lp).is_ok() {
                res.skipped += 1; // target alive → already shared
                continue;
            }
            let _ = std::fs::remove_dir(lp); // dangling (e.g. old plugin-cache target) → drop & re-link
        }
        match std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &link, &src])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) if o.status.success() => res.created += 1,
            // A racing pass may have created it between our check and mklink — a skip, not a failure.
            Ok(_) if std::fs::symlink_metadata(lp).is_ok() => res.skipped += 1,
            Ok(o) => {
                res.failed += 1;
                res.details.push(format!(
                    "{name}: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Err(e) => {
                res.failed += 1;
                res.details.push(format!("{name}: {e}"));
            }
        }
    }
    Ok(res)
}

// Windows-safe OpenCode RTK plugin. Thin delegating shell — all rewrite logic lives in
// `rtk rewrite` (RTK's Rust registry, the single source of truth), so this file rarely changes.
// The absolute rtk path ({{RTK_JSON}} = a serde_json-escaped string literal substituted at write
// time) avoids the upstream `which rtk` probe that is broken on Windows (rtk-ai/rtk#1993) and
// silently self-disables the plugin. JSON-escaping also neutralizes `${`/backtick/quote in the path.
const OPENCODE_RTK_PLUGIN: &str = r#"import type { Plugin } from "@opencode-ai/plugin"

// Managed by Castellyn — RTK command rewriting for OpenCode, pinned to the absolute rtk path
// so binary detection works on Windows (upstream `which rtk` fails there; rtk-ai/rtk#1993).
const RTK = {{RTK_JSON}}

export const RtkOpenCodePlugin: Plugin = async ({ $ }) => {
  try {
    await $`${RTK} --version`.quiet()
  } catch {
    console.warn("[rtk] rtk binary not found — plugin disabled")
    return {}
  }

  return {
    "tool.execute.before": async (input, output) => {
      const tool = String(input?.tool ?? "").toLowerCase()
      if (tool !== "bash" && tool !== "shell") return
      const args = output?.args
      if (!args || typeof args !== "object") return

      const command = (args as Record<string, unknown>).command
      if (typeof command !== "string" || !command) return

      try {
        const result = await $`${RTK} rewrite ${command}`.quiet().nothrow()
        const rewritten = String(result.stdout).trim()
        if (rewritten && rewritten !== command) {
          ;(args as Record<string, unknown>).command = rewritten
        }
      } catch {
        // rtk rewrite failed — pass the command through unchanged
      }
    },
  }
}
"#;

/// Resolve the rtk binary as an absolute path (`where rtk`, then the cargo bin fallback). Pinning
/// the full path is what makes the OpenCode plugin work on Windows (no reliance on `which`).
fn resolve_rtk_path(home: &str) -> Option<String> {
    if let Ok(o) = std::process::Command::new("where")
        .arg("rtk")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        if o.status.success() {
            if let Some(first) = String::from_utf8_lossy(&o.stdout).lines().next() {
                let p = first.trim();
                if !p.is_empty() {
                    return Some(p.to_string());
                }
            }
        }
    }
    let cargo = format!("{home}\\.cargo\\bin\\rtk.exe");
    std::path::Path::new(&cargo).is_file().then_some(cargo)
}

/// Enable/disable RTK command-rewriting for OpenCode by writing/removing a Castellyn-managed,
/// Windows-safe plugin at ~/.config/opencode/plugins/rtk.ts. Returns the new enabled state.
/// Reversible (disable just deletes the file); never touches Claude's RTK hook.
#[tauri::command]
fn run_opencode_rtk(action: String) -> Result<bool, String> {
    let home = std::env::var("USERPROFILE").map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let plugins_dir = format!("{home}\\.config\\opencode\\plugins");
    let plugin_file = format!("{plugins_dir}\\rtk.ts");
    match action.as_str() {
        "enable" => {
            let rtk = resolve_rtk_path(&home)
                .ok_or_else(|| tr("err.rtk_not_found", cur_lang()).to_string())?;
            std::fs::create_dir_all(&plugins_dir)
                .map_err(|e| trv("err.fs_create", cur_lang(), &[("path", &plugins_dir), ("e", &e)]))?;
            // serde_json renders the path as a safe JS string literal (escapes \, ", ${, backtick).
            let rtk_json = serde_json::to_string(&rtk).map_err(|e| e.to_string())?;
            let content = OPENCODE_RTK_PLUGIN.replace("{{RTK_JSON}}", &rtk_json);
            // Atomic temp+rename + .bak of any prior rtk.ts (incl. a hand-authored one) — crash-safe.
            write_json_atomic(&plugin_file, &content)
                .map_err(|e| trv("err.fs_write", cur_lang(), &[("path", &plugin_file), ("e", &e)]))?;
            Ok(true)
        }
        "disable" => {
            // Only remove a plugin we wrote — never delete a user's own hand-authored rtk.ts.
            match std::fs::read_to_string(&plugin_file) {
                Ok(c) if c.contains("Managed by Castellyn") => {
                    std::fs::remove_file(&plugin_file)
                        .map_err(|e| trv("err.fs_remove", cur_lang(), &[("path", &plugin_file), ("e", &e)]))?
                }
                Ok(_) => return Err(tr("err.rtk_not_managed", cur_lang()).to_string()),
                Err(_) => {} // not present → already disabled
            }
            Ok(false)
        }
        _ => Err(trv("err.unknown_action", cur_lang(), &[("action", &action)])),
    }
}

/// Fan out the canonical MCP servers (.mcp.json) into OpenCode's `opencode.json` `mcp` block.
/// Translates each Claude-format server (`command` + `args` [+ `env`]) to OpenCode's local-server
/// shape (`{type:"local", command:[…], enabled:true, environment}`). Merge-patch: overwrites the
/// canonical names, preserves any user-added servers. Returns the count written. Atomic write + .bak.
/// (Codex has its own fan-out via `run_codex_mcp` — the official `codex mcp add` CLI.)
/// Gap-2 reconcile diff: names we deployed before (`managed`) that are no longer in `canon` → to
/// remove. Pure + unit-tested. A user-added server was never in `managed`, so it's never returned.
fn mcp_stale_names(managed: &[String], canon: &[String]) -> Vec<String> {
    let keep: std::collections::HashSet<&str> = canon.iter().map(|s| s.as_str()).collect();
    managed
        .iter()
        .filter(|n| !keep.contains(n.as_str()))
        .cloned()
        .collect()
}

#[tauri::command]
fn run_opencode_mcp() -> Result<usize, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4/L4
    use serde_json::{json, Value};
    let home = std::env::var("USERPROFILE").map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    // Canonical source, placeholders expanded (same as write_temp_mcp_config).
    let src = std::fs::read_to_string(abs(MCP_CONFIG_REL))
        .map_err(|e| trv("err.mcp_read", cur_lang(), &[("e", &e)]))?
        .replace("{{USERPROFILE_FWD}}", &home.replace('\\', "/"));
    let canonical = parse_json_bom(&src).map_err(|e| trv("err.mcp_parse", cur_lang(), &[("e", &e)]))?;
    let servers = canonical
        .get("mcpServers")
        .and_then(|m| m.as_object())
        .ok_or_else(|| tr("err.mcp_no_servers", cur_lang()).to_string())?;

    let cfg_path = opencode_config_path();
    let mut cfg: Value = match std::fs::read_to_string(&cfg_path) {
        Ok(ref c) if !c.trim().is_empty() => {
            parse_json_bom(c).map_err(|e| trv("err.opencode_parse", cur_lang(), &[("e", &e)]))?
        }
        _ => return Err(tr("err.opencode_missing", cur_lang()).to_string()),
    };
    let Some(obj) = cfg.as_object_mut() else {
        return Err(tr("err.opencode_not_object", cur_lang()).to_string());
    };
    obj.entry("$schema")
        .or_insert_with(|| json!("https://opencode.ai/config.json"));
    if !obj.get("mcp").map(|m| m.is_object()).unwrap_or(false) {
        obj.insert("mcp".into(), json!({}));
    }
    let mcp = obj.get_mut("mcp").unwrap().as_object_mut().unwrap();

    // Canon = DEPLOYABLE servers only (commandless entries are skipped below and never inserted), so
    // the ledger + drift match what actually lands in opencode.json.
    let canon_names: Vec<String> = servers
        .iter()
        .filter(|(_, def)| {
            def.get("command")
                .and_then(|c| c.as_str())
                .is_some_and(|c| !c.is_empty())
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut count = 0usize;
    for (name, def) in servers {
        let command = def
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or_default();
        if command.is_empty() {
            continue;
        }
        let mut cmd = vec![json!(command)];
        if let Some(args) = def.get("args").and_then(|a| a.as_array()) {
            cmd.extend(args.iter().cloned());
        }
        let mut entry = json!({ "type": "local", "command": cmd, "enabled": true });
        if let Some(env) = def.get("env").and_then(|e| e.as_object()) {
            entry
                .as_object_mut()
                .unwrap()
                .insert("environment".into(), json!(env));
        }
        mcp.insert(name.clone(), entry);
        count += 1;
    }

    // Gap-2 reconcile: drop entries we deployed on a previous run that are no longer in canon. A
    // user-added server was never in our ledger (managed_mcp.opencode), so it's never removed.
    let mut hub = read_config_file();
    let prev = hub
        .managed_mcp
        .as_ref()
        .and_then(|m| m.opencode.clone())
        .unwrap_or_default();
    for stale in mcp_stale_names(&prev, &canon_names) {
        mcp.remove(&stale);
    }

    let serialized = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    write_json_atomic(&cfg_path, &serialized).map_err(|e| format!("write opencode.json: {e}"))?;
    // Update the ledger to the current canon (best-effort: opencode.json is already written).
    hub.managed_mcp.get_or_insert_default().opencode = Some(canon_names);
    let _ = write_config_file(&hub);
    Ok(count)
}

/// Translate one myproviders.json registry entry into an (id, desired-shape) pair for
/// OpenCode's `provider` block. Returns None when the entry lacks a usable name/baseUrl.
/// Refs-only by design: `apiKey` is an `{env:<NAME>_API_KEY}` reference — the real key lives
/// only in the Credential Manager and is never copied into opencode.json.
fn opencode_provider_entry(e: &serde_json::Value) -> Option<(String, serde_json::Value)> {
    use serde_json::json;
    let display = e.get("name")?.as_str()?.trim();
    let id = display.to_lowercase();
    let base = e.get("baseUrl")?.as_str()?.trim();
    if base.is_empty() || !valid_profile_name(&id) {
        return None;
    }
    // Anthropic-protocol endpoints ride the official SDK (x-api-key auth); everything else
    // is OpenAI-compatible (bearer).
    let npm = if e.get("protocol").and_then(|x| x.as_str()) == Some("anthropic") {
        "@ai-sdk/anthropic"
    } else {
        "@ai-sdk/openai-compatible"
    };
    let env_var: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    // Registry `model`/`smallModel` are free text, possibly `;`-separated / with a stray `;`.
    let mut models = serde_json::Map::new();
    for field in ["model", "smallModel"] {
        if let Some(s) = e.get(field).and_then(|x| x.as_str()) {
            for m in s.split(';').map(str::trim).filter(|m| !m.is_empty()) {
                models.entry(m.to_string()).or_insert(json!({}));
            }
        }
    }
    let mut shape = json!({
        "npm": npm,
        "name": display,
        "options": { "baseURL": base, "apiKey": format!("{{env:{env_var}_API_KEY}}") },
    });
    if !models.is_empty() {
        shape
            .as_object_mut()
            .unwrap()
            .insert("models".into(), serde_json::Value::Object(models));
    }
    Some((id, shape))
}

/// Merge the desired provider shape into an existing opencode.json `provider.<id>` object.
/// Overwrites npm/name/baseURL (canonical wins), but PRESERVES an existing `options.apiKey`
/// (a key the user already bound manually beats our env reference) and existing model entries.
fn merge_opencode_provider(target: &mut serde_json::Value, shape: serde_json::Value) {
    if !target.is_object() {
        *target = serde_json::json!({});
    }
    let t = target.as_object_mut().unwrap();
    let s = match shape {
        serde_json::Value::Object(m) => m,
        _ => return,
    };
    for (k, v) in s {
        match k.as_str() {
            "options" => {
                if !t.get("options").map(|x| x.is_object()).unwrap_or(false) {
                    t.insert("options".into(), serde_json::json!({}));
                }
                let to = t.get_mut("options").unwrap().as_object_mut().unwrap();
                if let serde_json::Value::Object(so) = v {
                    for (ok, ov) in so {
                        if ok == "apiKey" {
                            to.entry(ok).or_insert(ov);
                        } else {
                            to.insert(ok, ov);
                        }
                    }
                }
            }
            "models" => {
                if !t.get("models").map(|x| x.is_object()).unwrap_or(false) {
                    t.insert("models".into(), serde_json::json!({}));
                }
                let tm = t.get_mut("models").unwrap().as_object_mut().unwrap();
                if let serde_json::Value::Object(sm) = v {
                    for (mk, mv) in sm {
                        tm.entry(mk).or_insert(mv);
                    }
                }
            }
            _ => {
                t.insert(k, v);
            }
        }
    }
}

/// Fan out the custom-provider registry (myproviders.json) into OpenCode's `provider` block.
/// Batch counterpart of the single-provider `run_opencode_provider` bind. Merge-patch preserves
/// user-added providers and manually bound keys; secrets never leave the Credential Manager
/// (apiKey is written as an `{env:…}` reference the user populates). Returns the count written.
#[tauri::command]
fn run_opencode_providers() -> Result<usize, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4
    use serde_json::{json, Value};
    let src = std::fs::read_to_string(abs(MYPROVIDERS_CONFIG_REL))
        .map_err(|e| trv("err.myproviders_read", cur_lang(), &[("e", &e)]))?;
    let reg = parse_json_bom(&src).map_err(|e| trv("err.myproviders_parse", cur_lang(), &[("e", &e)]))?;
    let entries: Vec<Value> = reg
        .get("providers")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        return Err(tr("err.no_providers", cur_lang()).to_string());
    }

    let cfg_path = opencode_config_path();
    let mut cfg: Value = match std::fs::read_to_string(&cfg_path) {
        Ok(ref c) if !c.trim().is_empty() => {
            parse_json_bom(c).map_err(|e| trv("err.opencode_parse", cur_lang(), &[("e", &e)]))?
        }
        _ => return Err(tr("err.opencode_missing", cur_lang()).to_string()),
    };
    let Some(obj) = cfg.as_object_mut() else {
        return Err(tr("err.opencode_not_object", cur_lang()).to_string());
    };
    obj.entry("$schema")
        .or_insert_with(|| json!("https://opencode.ai/config.json"));
    if !obj.get("provider").map(|p| p.is_object()).unwrap_or(false) {
        obj.insert("provider".into(), json!({}));
    }
    let providers = obj.get_mut("provider").unwrap().as_object_mut().unwrap();

    let mut count = 0usize;
    for e in &entries {
        if let Some((id, shape)) = opencode_provider_entry(e) {
            merge_opencode_provider(providers.entry(id).or_insert(json!({})), shape);
            count += 1;
        }
    }

    let serialized = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    write_json_atomic(&cfg_path, &serialized).map_err(|e| format!("write opencode.json: {e}"))?;
    Ok(count)
}

/// Build the `codex mcp add` argv (after `cmd /C codex`) for one canonical .mcp.json server.
/// Returns None for entries without a launchable command. `codex mcp add` is an upsert, so
/// re-deploying overwrites the canonical names and never touches user-added servers.
fn codex_mcp_add_args(name: &str, def: &serde_json::Value) -> Option<Vec<String>> {
    let command = def.get("command").and_then(|c| c.as_str()).unwrap_or_default();
    if command.is_empty() || name.is_empty() {
        return None;
    }
    let mut argv = vec!["mcp".into(), "add".into(), name.to_string()];
    if let Some(env) = def.get("env").and_then(|e| e.as_object()) {
        for (k, v) in env {
            if let Some(val) = v.as_str() {
                argv.push("--env".into());
                argv.push(format!("{k}={val}"));
            }
        }
    }
    argv.push("--".into());
    argv.push(command.to_string());
    if let Some(args) = def.get("args").and_then(|a| a.as_array()) {
        for a in args {
            if let Some(s) = a.as_str() {
                argv.push(s.to_string());
            }
        }
    }
    Some(argv)
}

/// `run_codex_mcp` runs `cmd /C codex <argv>`; cmd re-parses that line, so any of these metacharacters
/// in an argv element (command, args, env values, server name — all user-editable in .mcp.json) would
/// let a field inject a second command. Reject the whole server if any element carries one. The name
/// sits at argv[2], so checking the built argv covers it too.
fn cmd_argv_safe(argv: &[String]) -> bool {
    const UNSAFE: &[char] = &['&', '|', '<', '>', '^', '%', '"'];
    !argv
        .iter()
        .any(|a| a.chars().any(|c| UNSAFE.contains(&c)))
}

/// Fan out the canonical MCP servers (.mcp.json) into Codex via the official `codex mcp add`
/// CLI — Codex owns its config.toml format/validation, so we never hand-edit TOML. Verified
/// live 2026-07-02 (upstream #3441 is closed): servers registered this way load in a session
/// and their tools resolve via Codex's tool search. Returns the count added.
#[tauri::command]
fn run_codex_mcp() -> Result<usize, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // L4 (config.json ledger + config.toml)
    use std::os::windows::process::CommandExt;
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let src = std::fs::read_to_string(abs(MCP_CONFIG_REL))
        .map_err(|e| trv("err.mcp_read", cur_lang(), &[("e", &e)]))?
        .replace("{{USERPROFILE_FWD}}", &home.replace('\\', "/"));
    let canonical = parse_json_bom(&src).map_err(|e| trv("err.mcp_parse", cur_lang(), &[("e", &e)]))?;
    let servers = canonical
        .get("mcpServers")
        .and_then(|m| m.as_object())
        .ok_or_else(|| tr("err.mcp_no_servers", cur_lang()).to_string())?;

    let mut count = 0usize;
    let mut errs: Vec<String> = Vec::new();
    for (name, def) in servers {
        let Some(argv) = codex_mcp_add_args(name, def) else {
            continue;
        };
        // cmd re-parses `cmd /C codex <argv>`, so reject (don't silently skip) any server whose argv
        // carries cmd metacharacters — a .mcp.json field could otherwise inject a second command.
        if !cmd_argv_safe(&argv) {
            errs.push(trv("err.mcp_unsafe_chars", cur_lang(), &[("name", &name)]));
            continue;
        }
        // codex is an npm .cmd shim on Windows — must go through cmd /C. Simple canonical
        // args (npx/urls/paths) survive cmd's re-parse; cmd metacharacters would not.
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg("codex").args(&argv);
        cmd.creation_flags(CREATE_NO_WINDOW);
        match cmd.output() {
            Ok(o) if o.status.success() => count += 1,
            Ok(o) => errs.push(format!(
                "{name}: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => errs.push(format!("{name}: {e}")),
        }
    }

    // Gap-2 reconcile: remove servers we deployed on a previous run that are no longer in canon via
    // `codex mcp remove <name>` (subcommand confirmed in the Codex CLI docs). Names come from our own
    // ledger, but re-validate for cmd safety anyway. A "no such server" (already gone) is fine →
    // best-effort, never fails the deploy. User-added servers were never in the ledger, so untouched.
    // Canon = the DEPLOYABLE servers only (codex_mcp_add_args returns None for a commandless entry, so
    // it's never added) — the ledger + drift must match what actually gets deployed.
    let canon_names: Vec<String> = servers
        .iter()
        .filter(|(name, def)| codex_mcp_add_args(name, def).is_some())
        .map(|(name, _)| name.clone())
        .collect();
    let mut hub = read_config_file();
    let prev = hub
        .managed_mcp
        .as_ref()
        .and_then(|m| m.codex.clone())
        .unwrap_or_default();
    let stale = mcp_stale_names(&prev, &canon_names);
    for name in &stale {
        if !cmd_argv_safe(std::slice::from_ref(name)) {
            continue;
        }
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg("codex").arg("mcp").arg("remove").arg(name);
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.output();
    }
    // Advance the ledger ONLY on a clean deploy (else a broken/absent codex, whose `add`s all failed,
    // would record canon as "deployed" though nothing was). Re-read config.toml so a stale server that
    // FAILED to remove stays in the ledger — the next deploy retries it and drift keeps flagging it
    // (self-heal), instead of it being silently reclassified as a user server and lingering forever.
    if errs.is_empty() {
        let now = std::fs::read_to_string(format!("{home}\\.codex\\config.toml"))
            .ok()
            .map(|t| toml_table_names(&t, "[mcp_servers."))
            .unwrap_or_default();
        let mut ledger = canon_names;
        for name in stale {
            if now.contains(&name) {
                ledger.push(name); // remove didn't take → keep managing it
            }
        }
        hub.managed_mcp.get_or_insert_default().codex = Some(ledger);
        let _ = write_config_file(&hub);
    }

    if !errs.is_empty() {
        return Err(errs.join(" · "));
    }
    Ok(count)
}

/// Merge the freellmapi gateway into Codex's config.toml text: a `[model_providers.freellmapi]`
/// table (Responses API — the gateway ships a /v1/responses shim) plus a `[profiles.freellmapi]`
/// so `codex --profile freellmapi` just works. Format-preserving via toml_edit. Canonical fields
/// (name/base_url/env_key, profile's model_provider) overwrite; the profile `model` is only
/// seeded when absent so a user's model choice survives a re-deploy. The top-level
/// `model`/`model_provider` are never touched — the user's ChatGPT default stays.
/// Raw myproviders.json entries are deliberately NOT fanned out to Codex: it speaks only the
/// Responses wire API (WireApi enum has no `chat` since 2026-02), so chat-completions/anthropic
/// endpoints would register but silently fail.
fn patch_codex_gateway(toml_text: &str, base_url: &str) -> Result<String, String> {
    use toml_edit::{value, DocumentMut, Item, Table};
    let mut doc: DocumentMut = toml_text
        .parse()
        .map_err(|e| format!("parse config.toml: {e}"))?;

    fn subtable<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
        if !parent.contains_key(key) || parent.get(key).and_then(Item::as_table).is_none() {
            let mut t = Table::new();
            t.set_implicit(true);
            parent.insert(key, Item::Table(t));
        }
        parent.get_mut(key).unwrap().as_table_mut().unwrap()
    }

    let providers = subtable(doc.as_table_mut(), "model_providers");
    let p = subtable(providers, "freellmapi");
    p.insert("name", value("FreeLLMAPI"));
    p.insert("base_url", value(format!("{base_url}/v1")));
    p.insert("env_key", value("FREELLMAPI_API_KEY"));

    let profiles = subtable(doc.as_table_mut(), "profiles");
    let prof = subtable(profiles, "freellmapi");
    prof.insert("model_provider", value("freellmapi"));
    if !prof.contains_key("model") {
        prof.insert("model", value("kimi-k2-thinking"));
    }

    Ok(doc.to_string())
}

/// Connect the freellmapi gateway to Codex (the "providers" fan-out for Codex — see
/// `patch_codex_gateway` for why the raw registry is not written). After the config write
/// it best-effort mirrors the gateway's unified API key into the USER environment
/// (`setx FREELLMAPI_API_KEY`, read from the gateway's own SQLite via a read-only node
/// helper — same mechanism as analytics) so `codex --profile freellmapi` works out of the
/// box in a new terminal. Returns whether the key was set; the key itself is never logged.
#[tauri::command]
fn run_codex_providers() -> Result<bool, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4-adjacent (config.toml)
    use std::os::windows::process::CommandExt;
    let base = gateway_base_url().ok_or_else(|| tr("err.gateway_missing", cur_lang()).to_string())?;
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let cfg_path = format!("{home}\\.codex\\config.toml");
    let text = std::fs::read_to_string(&cfg_path)
        .map_err(|_| tr("err.codex_missing", cur_lang()).to_string())?;
    let patched = patch_codex_gateway(text.trim_start_matches('\u{feff}'), &base)?;
    write_json_atomic(&cfg_path, &patched).map_err(|e| format!("write config.toml: {e}"))?;

    // Key mirror is best-effort: a missing DB/node/helper leaves the config connected and
    // the toast tells the user to set the variable by hand (dashboard shows the key).
    let key_set = (|| -> Option<()> {
        let db = gateway_db_path().filter(|p| std::path::Path::new(p).exists())?;
        let helper = abs("Castellyn\\tools\\analytics\\unified-key.cjs");
        let out = std::process::Command::new("node")
            .args([&helper, &db])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = parse_json_bom(String::from_utf8_lossy(&out.stdout).trim()).ok()?;
        let key = v.get("key")?.as_str()?.trim().to_string();
        if key.is_empty() {
            return None;
        }
        let st = std::process::Command::new("setx")
            .args(["FREELLMAPI_API_KEY", &key])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        st.status.success().then_some(())
    })()
    .is_some();
    Ok(key_set)
}

/// Canonical rule files fanned into OpenCode's `instructions` array (paths, not copies —
/// OpenCode reads them in place, so edits propagate without a re-deploy).
const CANON_RULES_REL: [&str; 2] = [
    "!Настройки и MCP\\ClaudeProfiles\\config\\CLAUDE.md",
    "!Настройки и MCP\\ClaudeProfiles\\config\\RTK.md",
];

/// Attach the canonical rule files to OpenCode's `instructions` array (idempotent merge,
/// existing user entries preserved). Returns how many canonical paths are connected after
/// the merge — 0 means none of the files exist on disk.
#[tauri::command]
fn run_opencode_instructions() -> Result<usize, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4
    use serde_json::{json, Value};
    let paths: Vec<String> = CANON_RULES_REL
        .iter()
        .map(|rel| abs(rel).replace('\\', "/"))
        .filter(|p| std::path::Path::new(p).is_file())
        .collect();
    if paths.is_empty() {
        return Err(tr("err.canon_rules_missing", cur_lang()).to_string());
    }

    let cfg_path = opencode_config_path();
    let mut cfg: Value = match std::fs::read_to_string(&cfg_path) {
        Ok(ref c) if !c.trim().is_empty() => {
            parse_json_bom(c).map_err(|e| trv("err.opencode_parse", cur_lang(), &[("e", &e)]))?
        }
        _ => return Err(tr("err.opencode_missing", cur_lang()).to_string()),
    };
    let Some(obj) = cfg.as_object_mut() else {
        return Err(tr("err.opencode_not_object", cur_lang()).to_string());
    };
    obj.entry("$schema")
        .or_insert_with(|| json!("https://opencode.ai/config.json"));
    if !obj
        .get("instructions")
        .map(|i| i.is_array())
        .unwrap_or(false)
    {
        obj.insert("instructions".into(), json!([]));
    }
    let list = obj.get_mut("instructions").unwrap().as_array_mut().unwrap();
    for p in &paths {
        if !list.iter().any(|v| v.as_str() == Some(p)) {
            list.push(json!(p));
        }
    }

    let serialized = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    write_json_atomic(&cfg_path, &serialized).map_err(|e| format!("write opencode.json: {e}"))?;
    Ok(paths.len())
}

#[cfg(test)]
mod opencode_fanout_tests {
    use serde_json::json;

    #[test]
    fn provider_entry_translates_protocol_models_and_env_ref() {
        let e = json!({
            "name": "minimax", "baseUrl": "https://api.minimax.chat/v1",
            "protocol": "openai", "model": "MiniMax-M3;", "smallModel": "MiniMax-M3;"
        });
        let (id, shape) = super::opencode_provider_entry(&e).expect("valid entry");
        assert_eq!(id, "minimax");
        assert_eq!(shape["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(shape["options"]["apiKey"], "{env:MINIMAX_API_KEY}");
        // stray `;` in the registry model must not produce an empty model key
        assert_eq!(shape["models"].as_object().unwrap().len(), 1);
        assert!(shape["models"].get("MiniMax-M3").is_some());

        let a = json!({ "name": "aerolink", "baseUrl": "https://capi.aerolink.lat", "protocol": "anthropic" });
        let (_, shape) = super::opencode_provider_entry(&a).expect("valid entry");
        assert_eq!(shape["npm"], "@ai-sdk/anthropic");
        assert!(shape.get("models").is_none());
    }

    #[test]
    fn provider_entry_rejects_unusable_names() {
        assert!(super::opencode_provider_entry(&json!({ "name": "имя с пробелом", "baseUrl": "https://x" })).is_none());
        assert!(super::opencode_provider_entry(&json!({ "name": "ok", "baseUrl": "" })).is_none());
    }

    #[test]
    fn codex_gateway_patch_preserves_config_and_user_model() {
        let existing = "# my codex\nmodel = \"gpt-5.5\"\n\n[profiles.freellmapi]\nmodel = \"auto\"\n";
        let out = super::patch_codex_gateway(existing, "http://localhost:13001").unwrap();
        // comment + top-level model untouched; provider table written; user's profile model kept
        assert!(out.contains("# my codex"));
        assert!(out.contains("model = \"gpt-5.5\""));
        assert!(out.contains("[model_providers.freellmapi]"));
        assert!(out.contains("base_url = \"http://localhost:13001/v1\""));
        assert!(out.contains("env_key = \"FREELLMAPI_API_KEY\""));
        assert!(out.contains("model = \"auto\""));
        assert!(!out.contains("model = \"kimi-k2-thinking\""));
        // fresh config: profile model seeded, top-level model_provider NOT set
        let fresh = super::patch_codex_gateway("", "http://localhost:13001").unwrap();
        assert!(fresh.contains("model = \"kimi-k2-thinking\""));
        // the profile sets model_provider, but the TOP-LEVEL default must stay untouched
        let doc: toml_edit::DocumentMut = fresh.parse().unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model").is_none());
    }

    #[test]
    fn codex_add_args_shape_env_and_separator() {
        let def = json!({
            "command": "npx",
            "args": ["-y", "chrome-devtools-mcp@latest", "--browserUrl", "http://localhost:9222"],
            "env": { "FOO": "C:/Users/User/x.json" }
        });
        let argv = super::codex_mcp_add_args("chrome-devtools", &def).expect("valid");
        assert_eq!(argv[..3], ["mcp", "add", "chrome-devtools"].map(String::from));
        // env flags come BEFORE the `--` separator, server args after it
        let sep = argv.iter().position(|a| a == "--").expect("separator");
        assert!(argv[..sep].contains(&"FOO=C:/Users/User/x.json".to_string()));
        assert_eq!(argv[sep + 1], "npx");
        assert_eq!(*argv.last().unwrap(), "http://localhost:9222");
        // no command → no invocation
        assert!(super::codex_mcp_add_args("bad", &json!({ "args": ["x"] })).is_none());
    }

    #[test]
    fn merge_preserves_manual_api_key_and_user_models() {
        let mut target = json!({
            "options": { "apiKey": "{env:MY_REAL_KEY}", "timeout": 5 },
            "models": { "user-model": { "name": "kept" } }
        });
        let (_, shape) = super::opencode_provider_entry(&json!({
            "name": "minimax", "baseUrl": "https://api.minimax.chat/v1", "model": "MiniMax-M3"
        }))
        .unwrap();
        super::merge_opencode_provider(&mut target, shape);
        // manual key + unrelated option survive, canonical baseURL/npm win, models union
        assert_eq!(target["options"]["apiKey"], "{env:MY_REAL_KEY}");
        assert_eq!(target["options"]["timeout"], 5);
        assert_eq!(target["options"]["baseURL"], "https://api.minimax.chat/v1");
        assert_eq!(target["npm"], "@ai-sdk/openai-compatible");
        assert!(target["models"].get("user-model").is_some());
        assert!(target["models"].get("MiniMax-M3").is_some());
    }
}

#[cfg(test)]
mod codex_cmd_safety_tests {
    use serde_json::json;

    fn safe(command: &str, arg: &str, env_val: &str, name: &str) -> bool {
        let def = json!({ "command": command, "args": [arg], "env": { "FOO": env_val } });
        let argv = super::codex_mcp_add_args(name, &def).expect("valid def");
        super::cmd_argv_safe(&argv)
    }

    #[test]
    fn plain_canonical_argv_passes() {
        assert!(safe("npx", "chrome-devtools-mcp@latest", "C:/Users/User/x.json", "chrome-devtools"));
    }

    #[test]
    fn cmd_metachars_are_rejected_in_every_argv_position() {
        // command, args, env value, and server name each reach `cmd /C codex` — each must be guarded.
        for bad in ["a&b", "a|b", "a%b", "a\"b"] {
            assert!(!safe(bad, "ok", "ok", "srv"), "command {bad:?} should be unsafe");
            assert!(!safe("npx", bad, "ok", "srv"), "arg {bad:?} should be unsafe");
            assert!(!safe("npx", "ok", bad, "srv"), "env value {bad:?} should be unsafe");
            assert!(!safe("npx", "ok", "ok", bad), "name {bad:?} should be unsafe");
        }
    }
}

#[cfg(test)]
mod probe_url_tests {
    #[test]
    fn https_and_loopback_http_allowed() {
        assert!(super::probe_url_allowed("https://api.example.com").is_ok());
        assert!(super::probe_url_allowed("http://localhost:8080").is_ok());
        assert!(super::probe_url_allowed("http://127.0.0.1:1234").is_ok());
        assert!(super::probe_url_allowed("http://[::1]:9").is_ok());
    }

    #[test]
    fn plaintext_nonloopback_and_bad_schemes_rejected() {
        assert!(super::probe_url_allowed("http://api.example.com").is_err());
        assert!(super::probe_url_allowed("http://192.168.1.10:1234").is_err());
        assert!(super::probe_url_allowed("ftp://x").is_err());
        // SSRF/metadata host is caught by valid_base_url before the https rule.
        assert!(super::probe_url_allowed("http://169.254.169.254").is_err());
    }

    // Regression: a prefix/substring loopback test wrongly allowed these, leaking the bearer key over
    // http to an attacker-influenced host. The strict IP-parse must reject every one.
    #[test]
    fn loopback_lookalikes_are_rejected() {
        assert!(super::probe_url_allowed("http://127.0.0.1.evil.com").is_err());
        assert!(super::probe_url_allowed("http://127.foo").is_err());
        assert!(super::probe_url_allowed("http://0x7f.0.0.1").is_err());
        assert!(super::probe_url_allowed("http://localhost.evil.com").is_err());
        // Userinfo trick: real host is evil.com, not 127.0.0.1.
        assert!(super::probe_url_allowed("http://127.0.0.1@evil.com").is_err());
        // Genuine loopback across the whole 127.0.0.0/8 block still passes.
        assert!(super::probe_url_allowed("http://127.5.6.7:8080").is_ok());
    }
}

#[cfg(test)]
mod fork_slot_tests {
    use super::{ForkRepoSlot, ForkRuns, ForksGlobalSlot, FORKS_GLOBAL};
    use std::sync::atomic::Ordering;

    // One test: FORKS_GLOBAL is a shared static, so keep all its mutations sequential in a single fn.
    #[test]
    fn slots_clear_on_drop_and_reject_while_held() {
        // Global slot: set on reserve, cleared on drop.
        {
            let _g = ForksGlobalSlot::reserve();
            assert!(FORKS_GLOBAL.load(Ordering::SeqCst));
        }
        assert!(!FORKS_GLOBAL.load(Ordering::SeqCst));

        // Per-repo slot: reserve inserts, a second reserve of the same path is rejected, drop removes.
        let runs = ForkRuns::default();
        {
            let s = ForkRepoSlot::reserve(&runs, "C:/repo".to_string()).expect("first reserve");
            assert!(runs.0.lock().unwrap().contains_key("C:/repo"));
            assert!(ForkRepoSlot::reserve(&runs, "C:/repo".to_string()).is_err());
            s.set_pid(4242);
            assert_eq!(*runs.0.lock().unwrap().get("C:/repo").unwrap(), 4242);
        }
        assert!(runs.0.lock().unwrap().is_empty());
        // Drop freed the path → reserving it again works.
        let _s2 = ForkRepoSlot::reserve(&runs, "C:/repo".to_string()).expect("reserve after drop");

        // A global sweep in flight blocks a per-repo reserve until the global slot drops.
        let runs2 = ForkRuns::default();
        {
            let _g = ForksGlobalSlot::reserve();
            assert!(ForkRepoSlot::reserve(&runs2, "C:/x".to_string()).is_err());
        }
        assert!(ForkRepoSlot::reserve(&runs2, "C:/x".to_string()).is_ok());
    }
}

#[cfg(test)]
mod add_key_txn_tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    // (a) write fails on first add → legacy entry still present, no orphan slots.
    #[test]
    fn write_failure_keeps_legacy_and_leaves_no_orphans() {
        let store = RefCell::new(HashMap::<String, String>::new());
        store.borrow_mut().insert("provider:acme".into(), "LEGACY".into());
        let r = super::append_key_txn(
            "acme",
            "NEWKEY",
            0,
            |u| store.borrow().get(u).cloned(),
            |u, s| {
                store.borrow_mut().insert(u.into(), s.into());
                Ok(())
            },
            |u| {
                store.borrow_mut().remove(u);
            },
            |_new_count| Err("disk full".to_string()),
        );
        assert_eq!(r, Err("disk full".to_string()));
        let s = store.borrow();
        assert_eq!(s.get("provider:acme").map(String::as_str), Some("LEGACY"));
        assert!(s.get("provider:acme:0").is_none());
        assert!(s.get("provider:acme:1").is_none());
    }

    // (b) write succeeds → legacy gone, slot0 = legacy value, slot1 = new key.
    #[test]
    fn write_success_migrates_legacy_and_appends() {
        let store = RefCell::new(HashMap::<String, String>::new());
        store.borrow_mut().insert("provider:acme".into(), "LEGACY".into());
        let mut written = 0u64;
        let r = super::append_key_txn(
            "acme",
            "NEWKEY",
            0,
            |u| store.borrow().get(u).cloned(),
            |u, s| {
                store.borrow_mut().insert(u.into(), s.into());
                Ok(())
            },
            |u| {
                store.borrow_mut().remove(u);
            },
            |new_count| {
                written = new_count;
                Ok(())
            },
        );
        assert_eq!(r, Ok(2));
        assert_eq!(written, 2);
        let s = store.borrow();
        assert!(s.get("provider:acme").is_none());
        assert_eq!(s.get("provider:acme:0").map(String::as_str), Some("LEGACY"));
        assert_eq!(s.get("provider:acme:1").map(String::as_str), Some("NEWKEY"));
    }

    // (c) first add, migration set slot 0 SUCCEEDS but the new-key set FAILS → the migrated slot 0
    // is rolled back (no orphan) and the legacy entry stays intact.
    #[test]
    fn migration_set_ok_but_new_key_set_fails_leaves_no_orphan() {
        let store = RefCell::new(HashMap::<String, String>::new());
        store.borrow_mut().insert("provider:acme".into(), "LEGACY".into());
        let calls = std::cell::Cell::new(0);
        let r = super::append_key_txn(
            "acme",
            "NEWKEY",
            0,
            |u| store.borrow().get(u).cloned(),
            |u, s| {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    // The second set (the new key at slot 1) fails after slot 0 migration succeeded.
                    return Err("keyring busy".into());
                }
                store.borrow_mut().insert(u.into(), s.into());
                Ok(())
            },
            |u| {
                store.borrow_mut().remove(u);
            },
            |_new_count| Ok(()),
        );
        assert_eq!(r, Err("keyring busy".to_string()));
        let s = store.borrow();
        assert_eq!(s.get("provider:acme").map(String::as_str), Some("LEGACY")); // legacy intact
        assert!(s.get("provider:acme:0").is_none()); // migrated slot rolled back — no orphan
        assert!(s.get("provider:acme:1").is_none());
    }
}

#[cfg(test)]
mod opencode_rtk_plugin_tests {
    /// A path containing backtick / `${` / quote must render as a single valid JS string literal,
    /// not break out of `const RTK = …`. Guards the rtk-path injection fix (#9).
    #[test]
    fn rtk_path_renders_as_safe_string_literal() {
        let evil = r#"C:\a`b${x}"c\rtk.exe"#;
        let json = serde_json::to_string(evil).unwrap();
        let content = super::OPENCODE_RTK_PLUGIN.replace("{{RTK_JSON}}", &json);
        let line = content
            .lines()
            .find(|l| l.trim_start().starts_with("const RTK = "))
            .expect("const RTK line present");
        let val = line.trim_start().strip_prefix("const RTK = ").unwrap();
        let parsed: String =
            serde_json::from_str(val).expect("RTK value must be a valid JSON/JS string literal");
        assert_eq!(parsed, evil);
    }
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
    let Some((plugins_dir, installed, markets)) = load_installed_plugins() else {
        return Vec::new();
    };

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
        if !plugin_id_path_safe(plugin_id) {
            continue; // L7: reject a traversal-y plugin id before it reaches a filesystem path
        }
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
    if !plugin_id_path_safe(plugin_id) {
        return None; // L7: reject a traversal-y plugin id before it reaches a filesystem path
    }
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

/// M5: is this path itself a reparse point (Windows junction OR symlink)? Stat's the link, not its
/// target (symlink_metadata), same FILE_ATTRIBUTE_REPARSE_POINT test has_reparse_child uses.
fn is_reparse_point(p: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    p.symlink_metadata()
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

/// L7: a marketplace-supplied plugin id is spliced into a filesystem path (`...\plugins\{id}`);
/// reject empty / traversal (`..`, `/`, `\`) ids before that, mirroring run_plugin's charset guard.
fn plugin_id_path_safe(plugin_id: &str) -> bool {
    !plugin_id.is_empty()
        && !plugin_id.contains("..")
        && !plugin_id.contains('/')
        && !plugin_id.contains('\\')
}

/// Collect `*.md` stems under a directory recursively (used for commands/agents).
/// Nested paths are joined with `:` to mirror Claude Code's namespaced naming.
fn collect_md_names(root: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>, depth: u32) {
        if depth > 32 {
            return; // M5: belt-and-suspenders depth cap against any directory cycle
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                // M5: never descend into a junction/symlink — a self- or mutually-referencing reparse
                // point (hostile/misconfigured plugin dir) would recurse forever and crash the app.
                if is_reparse_point(&p) {
                    continue;
                }
                walk(&p, base, out, depth + 1);
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
    walk(root, root, &mut out, 0);
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
    let Some((_dir, installed, markets)) = load_installed_plugins() else {
        return Vec::new();
    };

    let mut out: Vec<PluginContents> = Vec::new();
    let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) else {
        return out;
    };
    for (id, arr) in po {
        let install_path = first_install_path(arr);
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
        out.push(PluginContents {
            id: id.clone(),
            skills,
            commands,
            agents,
        });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// Parses a plugin id (`owner/repo@marketplace`) into a GitHub owner/repo pair.
/// Returns None for npm-style scoped names (`@scope/name@...`) or non-GitHub names.
fn parse_plugin_gh_repo(id: &str) -> Option<(String, String)> {
    let at = id.rfind('@')?;
    let part = &id[..at];
    if part.is_empty() || part.starts_with('@') {
        return None;
    }
    let slash = part.find('/')?;
    let (owner, repo) = part.split_at(slash);
    let repo = &repo[1..];
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

#[derive(Serialize, Deserialize, Clone)]
struct PluginRelease {
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    body: String,
    published_at: String,
}

/// In-memory cache keyed by plugin id: (fetch_time, releases). 5 minute TTL.
type ReleasesCache = std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<PluginRelease>)>>;
static RELEASES_CACHE: std::sync::OnceLock<ReleasesCache> = std::sync::OnceLock::new();

fn get_releases_cache() -> &'static ReleasesCache {
    RELEASES_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Attempt to fetch an auth token for the GitHub API (GH_TOKEN or gh CLI).
fn gh_api_token() -> Option<String> {
    if let Ok(t) = std::env::var("GH_TOKEN") {
        if !t.is_empty() {
            return Some(t);
        }
    }
    if let Ok(t) = std::env::var("GITHUB_TOKEN") {
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

/// Blocking: fetch releases for a GitHub-hosted plugin. Cached for 5 minutes.
fn fetch_plugin_releases(id: &str) -> Vec<PluginRelease> {
    let (owner, repo) = match parse_plugin_gh_repo(id) {
        Some(r) => r,
        None => return Vec::new(),
    };
    // Check cache (5 min TTL).
    {
        if let Ok(guard) = get_releases_cache().lock() {
            if let Some((ts, cached)) = guard.get(id) {
                if ts.elapsed() < std::time::Duration::from_secs(300) {
                    return cached.clone();
                }
            }
        }
    }
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=10");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();
    let mut req = agent.get(&url).header("User-Agent", "Castellyn/1.0");
    if let Some(t) = gh_api_token() {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let body = match req.call() {
        Ok(mut resp) => resp.body_mut().read_to_string().unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    let releases: Vec<PluginRelease> = serde_json::from_str(&body).unwrap_or_default();
    // Update cache.
    if let Ok(mut guard) = get_releases_cache().lock() {
        guard.insert(
            id.to_string(),
            (std::time::Instant::now(), releases.clone()),
        );
    }
    releases
}

/// Fetch GitHub releases for a plugin (community plugins published from a GitHub repo).
/// Returns empty vec for non-GitHub plugins or on error/rate-limit.
#[tauri::command]
async fn list_plugin_releases(id: String) -> Vec<PluginRelease> {
    tokio::task::spawn_blocking(move || fetch_plugin_releases(&id))
        .await
        .unwrap_or_default()
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
    cmd.args(["plugin", action, id])
        .creation_flags(CREATE_NO_WINDOW);
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
        Err(e) => err(&trv("log.claude_spawn", cur_lang(), &[("e", &e)])),
    }
}

/// Native port of Manage-Plugin.ps1: enable / disable / update a plugin via the claude CLI.
/// `update` runs once (the plugins/ cache is shared across profiles via junction); enable/disable
/// loop every profile (enabled-state is per-profile, switched via CLAUDE_CONFIG_DIR). Uses the
/// canonical `profile_names()` (vs the script's hardcoded 6) so a 7th profile is covered too.
/// Returns the exit code; streams via out/err.
fn manage_plugin_native(action: &str, id: &str, out: &dyn Fn(&str), err: &dyn Fn(&str)) -> i32 {
    let Some(claude) = exe_on_path("claude") else {
        err(tr("log.claude_not_found", cur_lang()));
        return 1;
    };
    out(&trv(
        "log.plugin_header",
        cur_lang(),
        &[("action", &action), ("id", &id)],
    ));
    if action == "update" {
        run_claude_plugin(&claude, None, action, id, out, err);
    } else {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        for p in profile_names() {
            let dir = format!("{home}\\.claude-{p}");
            if !std::path::Path::new(&dir).exists() {
                out(&trv("log.plugin_skip", cur_lang(), &[("p", &p)]));
                continue;
            }
            out(&format!("  [{p}] claude plugin {action} {id}"));
            run_claude_plugin(&claude, Some(&dir), action, id, out, err);
        }
    }
    out(tr("log.done", cur_lang()));
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
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
        {
            return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", &id)]));
        }
        return spawn_streamed_prog(
            app,
            state,
            "plugin-mgr".to_string(),
            "cmd".to_string(),
            vec![
                "/c".into(),
                "claude".into(),
                "plugin".into(),
                "remove".into(),
                id,
            ],
            None,
        )
        .await;
    }
    if !matches!(action.as_str(), "enable" | "disable" | "update") {
        return Err(trv(
            "err.unknown_plugin_action",
            cur_lang(),
            &[("action", &action)],
        ));
    }
    // id reaches process args natively now — guard it (same rule as the remove branch).
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
    {
        return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", &id)]));
    }
    run_native_streamed(app, state, "plugin-mgr".to_string(), move |out, err| {
        manage_plugin_native(&action, &id, out, err)
    })
    .await
}

/// F17: bulk plugin op in its own domain — sequential inside (no config race), but off the global
/// RunState so other work proceeds. Streams id-tagged lines to "run-log" (component "plugin-mgr",
/// same channel the single op uses) and emits one "run-done" at the end. Cancellable between items.
#[tauri::command]
async fn run_plugins_bulk(app: AppHandle, action: String, ids: Vec<String>) -> Result<i32, String> {
    if !matches!(action.as_str(), "enable" | "disable" | "update" | "remove") {
        return Err(trv(
            "err.unknown_plugin_action",
            cur_lang(),
            &[("action", &action)],
        ));
    }
    if ids.is_empty() {
        return Ok(0);
    }
    for id in &ids {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
        {
            return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", id)]));
        }
    }
    // Reserve the bulk domain (RAII: released on drop, even if this future is dropped mid-sweep).
    let _slot = match BulkSlot::reserve() {
        Some(s) => s,
        None => return Err(tr("err.run_in_progress", cur_lang()).into()),
    };
    BULK_PLUGINS_CANCEL.store(false, Ordering::SeqCst);
    let app_job = app.clone();
    let code = tokio::task::spawn_blocking(move || {
        let out = |line: &str| {
            let _ = app_job.emit(
                "run-log",
                LogLine {
                    component: "plugin-mgr".into(),
                    stream: "out".into(),
                    line: line.to_string(),
                },
            );
        };
        let err = |line: &str| {
            let _ = app_job.emit(
                "run-log",
                LogLine {
                    component: "plugin-mgr".into(),
                    stream: "err".into(),
                    line: line.to_string(),
                },
            );
        };
        let mut worst = 0;
        for id in &ids {
            if BULK_PLUGINS_CANCEL.load(Ordering::SeqCst) {
                out(tr("log.bulk_cancelled", cur_lang()));
                worst = 130;
                break;
            }
            out(&format!("── {id} ──")); // id tag so the serialized output stays correlatable
            let c = if action == "remove" {
                // Mirror the single-remove path (`cmd /c claude`) so a claude.cmd shim still launches.
                out(&format!("  claude plugin remove {id}"));
                match std::process::Command::new("cmd")
                    .args(["/c", "claude", "plugin", "remove", id])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                {
                    Ok(o) => {
                        for line in String::from_utf8_lossy(&o.stdout).lines() {
                            out(&format!("    {line}"));
                        }
                        for line in String::from_utf8_lossy(&o.stderr).lines() {
                            err(&format!("    {line}"));
                        }
                        if o.status.success() {
                            0
                        } else {
                            o.status.code().unwrap_or(1)
                        }
                    }
                    Err(e) => {
                        err(&format!("    {e}"));
                        1
                    }
                }
            } else {
                manage_plugin_native(&action, id, &out, &err)
            };
            if c != 0 {
                worst = c;
            }
        }
        worst
    })
    .await
    .unwrap_or(-1);
    drop(_slot); // release the bulk domain (also released automatically if dropped earlier)
    let _ = app.emit(
        "run-done",
        RunDone {
            component: "plugin-mgr".into(),
            code,
        },
    );
    Ok(code)
}

/// Delete a standalone skill from ~/.claude/skills. Guard uses the entry's PARENT (not the
/// resolved target) so a symlinked "own" skill only has its LINK removed — the real collection in
/// ~/.agents stays intact. Plugin-bundled skills (parent ≠ ~/.claude/skills) are refused.
#[tauri::command]
fn delete_skill(dir: String) -> Result<(), String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let skills_root = std::path::Path::new(&home).join(".claude").join("skills");
    let target = std::path::Path::new(&dir);
    let parent = target.parent().ok_or(tr("err.bad_path", cur_lang()))?;
    let canon_root = std::fs::canonicalize(&skills_root)
        .map_err(|e| trv("err.skills_dir", cur_lang(), &[("e", &e)]))?;
    let canon_parent = std::fs::canonicalize(parent)
        .map_err(|e| trv("err.skill_not_found", cur_lang(), &[("e", &e)]))?;
    if canon_parent != canon_root {
        return Err(tr("err.skill_not_in_skills", cur_lang()).into());
    }
    let meta = std::fs::symlink_metadata(target)
        .map_err(|e| trv("err.skill_not_found", cur_lang(), &[("e", &e)]))?;
    if meta.file_type().is_symlink() {
        // Remove only the symlink, never the linked-to source collection.
        std::fs::remove_dir(target)
            .or_else(|_| std::fs::remove_file(target))
            .map_err(|e| trv("err.remove_link", cur_lang(), &[("e", &e)]))
    } else {
        std::fs::remove_dir_all(target).map_err(|e| trv("err.remove", cur_lang(), &[("e", &e)]))
    }
}

// --- Plugin sync across profiles (Plugins tab) ---
//
// Ships the user's plugin_sync.py reconcile hook as an embedded asset. Two surfaces:
//  * "Sync now" — runs the script once (py launcher) streaming into the console;
//  * SessionStart auto-sync toggle — wires/unwires the hook command into every profile's
//    settings.json (idempotent; skips symlinked settings; the reconcile itself only fills
//    MISSING enabledPlugins keys — an explicit `false` is a per-profile opt-out it never touches).

const PLUGIN_SYNC_SCRIPT: &str = include_str!("../assets/plugin_sync.py");
/// Hook command, byte-identical to the pre-Castellyn manual wiring so wired-detection
/// (filename match) and re-wiring stay idempotent with existing installs.
const PLUGIN_SYNC_HOOK_CMD: &str = "py -X utf8 ~/.claude/hooks/plugin_sync.py";

fn plugin_sync_script_path(home: &str) -> String {
    format!("{home}\\.claude\\hooks\\plugin_sync.py")
}

/// First-line "# <prefix> N" version header → N (0 when absent/unparsable).
fn script_version_header(text: &str, prefix: &str) -> u32 {
    text.lines()
        .next()
        .and_then(|l| l.strip_prefix(prefix))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn plugin_sync_version(text: &str) -> u32 {
    script_version_header(text, "# plugin-sync-version:")
}

/// Render the embedded script with the current profile-dir list substituted into the
/// `# castellyn:profiles` marker line. Pure (unit-tested); deterministic for change-compare.
fn render_plugin_sync_script(dirs: &[String]) -> String {
    let list = dirs
        .iter()
        .map(|d| format!("{d:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    PLUGIN_SYNC_SCRIPT
        .lines()
        .map(|l| {
            if l.contains("# castellyn:profiles") {
                format!("PROFILES = [{list}]  # castellyn:profiles")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Install/refresh the hook script: written when missing, older than the embedded version,
/// or same-version with a changed profile list. Never downgrades a newer local copy.
/// Returns the script path.
fn ensure_plugin_sync_script(home: &str) -> Result<String, String> {
    let path = plugin_sync_script_path(home);
    let rendered = render_plugin_sync_script(
        &plugin_sync_profiles(home)
            .into_iter()
            .map(|(dir, _)| dir)
            .collect::<Vec<_>>(),
    );
    let embedded_ver = plugin_sync_version(&rendered);
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    let disk_ver = plugin_sync_version(&on_disk);
    if disk_ver < embedded_ver || (disk_ver == embedded_ver && on_disk != rendered) {
        write_json_atomic(&path, &rendered).map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(path)
}

/// Authoritative profile-dir list: ~/.claude plus ~/.claude-<name> for every profile in
/// profiles.json — filtered to dirs whose settings.json exists and is not a symlink.
/// A home-dir scan is deliberately NOT used: sibling dirs like ~/.claude-mem (claude-mem's
/// data) or stray copies also hold a settings.json and must never be treated as profiles.
fn plugin_sync_profiles(home: &str) -> Vec<(String, String)> {
    let mut dirs = vec![".claude".to_string()];
    dirs.extend(profile_names().into_iter().map(|n| format!(".claude-{n}")));
    let mut out = Vec::new();
    for n in dirs {
        let sp = format!("{home}\\{n}\\settings.json");
        if let Ok(meta) = std::fs::symlink_metadata(&sp) {
            if meta.file_type().is_file() {
                out.push((n, sp));
            }
        }
    }
    out
}

/// A plausible Castellyn profile dir name (`~/.claude` or `~/.claude-<name>`). Deliberately
/// permissive: a foreign sibling like `~/.claude-mem` also matches here, but the marker-gated
/// unwire below never modifies it (no Castellyn marker → no-op, no write). Pure (unit-tested).
fn is_claude_profile_dirname(name: &str) -> bool {
    name == ".claude" || name.starts_with(".claude-")
}

/// Every `~/.claude*` sibling with a real (non-symlink) settings.json — the current profiles
/// PLUS orphans (dirs dropped from profiles.json but still on disk). Used ONLY by the disable
/// sweeps so a renamed/removed profile's Castellyn hook entries still get unwired. Marker-gated
/// unwire keeps foreign dirs untouched, so this may read `~/.claude-mem` but never writes it.
fn claude_settings_all(home: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(home) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !is_claude_profile_dirname(&name) {
            continue;
        }
        let sp = format!("{home}\\{name}\\settings.json");
        if let Ok(meta) = std::fs::symlink_metadata(&sp) {
            if meta.file_type().is_file() {
                out.push((name, sp));
            }
        }
    }
    out
}

/// Foreign `.claude-*` siblings that belong to OTHER tools, not abandoned Claude Code profiles.
/// Denylisted from orphan detection so we never offer to adopt/delete another tool's state dir.
/// (The `.claude.json` marker gate below already excludes most, but this is belt-and-suspenders.)
const ORPHAN_DENYLIST: &[&str] = &["mem", "code-router", "code-templates", "router-logs"];

/// Pure predicate: is `dirname` an abandoned Claude Code profile dir (adoptable/deletable)?
/// True when it is a `.claude-<suffix>` dir (not the base `.claude`), the suffix is NOT a canon
/// profile, NOT a known foreign tool, and the dir carries a Claude Code marker (`.claude.json`).
/// The marker is what separates a real orphaned CC config dir from a foreign sibling like
/// `.claude-mem` (which has no `.claude.json`). Unit-tested.
fn is_orphan_profile_dir(dirname: &str, has_claude_json: bool, canon: &[String]) -> bool {
    let Some(suffix) = dirname.strip_prefix(".claude-") else {
        return false;
    };
    if suffix.is_empty() || !has_claude_json {
        return false;
    }
    // Windows strips trailing spaces/dots from a path's final component, so `.claude-cc1 ` and
    // `.claude-cc1.` RESOLVE to the canon `.claude-cc1` through normal-path APIs. A suffix that is
    // not already in normalized form can't be safely adopted/recycled (the op would hit its
    // trimmed twin — possibly a live canon profile), so it is never treated as an orphan here.
    // Such a dir, if it truly exists distinctly, must be removed manually via a `\\?\` path.
    if suffix != suffix.trim_end_matches([' ', '.']) {
        return false;
    }
    // Compare case-insensitively: NTFS is case-insensitive, so `.claude-CC1` resolves to the canon
    // `.claude-cc1` too — treating it as an orphan would let adopt/recycle hit the canon dir.
    if ORPHAN_DENYLIST.iter().any(|d| d.eq_ignore_ascii_case(suffix)) {
        return false;
    }
    !canon.iter().any(|n| n.eq_ignore_ascii_case(suffix))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrphanInfo {
    /// Profile name = dir suffix after `.claude-` (e.g. "oldtest" for `~/.claude-oldtest`).
    name: String,
    /// Last-modified unix seconds of the dir (0 if unavailable) — lets the user judge staleness.
    modified: u64,
}

/// List `~/.claude-<name>` dirs that are NOT canon profiles yet carry a Claude Code marker
/// (`.claude.json`) — abandoned/foreign profile dirs the user can Adopt or Delete. Foreign tool
/// dirs (`.claude-mem` etc.) are excluded via the marker + denylist. Read-only.
/// ponytail: no recursive size — walking a profile dir would follow the shared-folder junctions
/// (projects/, plugins/ …) into real, possibly huge/cyclic trees. Modified date + "open folder"
/// is enough to judge; add a bounded, symlink-skipping size only if the date proves insufficient.
#[tauri::command]
fn read_orphan_profiles() -> Result<Vec<OrphanInfo>, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let canon = profile_names();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&home) else {
        return Ok(out);
    };
    for e in entries.flatten() {
        if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let dir = e.path();
        let has_claude_json = dir.join(".claude.json").is_file();
        if !is_orphan_profile_dir(&name, has_claude_json, &canon) {
            continue;
        }
        let suffix = name.strip_prefix(".claude-").unwrap_or(&name).to_string();
        let modified = std::fs::metadata(&dir)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(OrphanInfo {
            name: suffix,
            modified,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Move an orphan profile dir (`~/.claude-<name>`) to the Recycle Bin (reversible).
/// Hard guards: the name must NOT be a canon profile (canon profiles are removed via
/// `run_profile_mgmt('remove')`, never here), must carry no path separators, must currently
/// qualify as an orphan, and must not be a live session. No charset gate: an orphan dirname may
/// legitimately be off-charset (e.g. a trailing space) — we only need to locate and recycle the
/// exact dir, and the guards below already close the traversal/wrong-target risks.
#[tauri::command]
fn delete_orphan_profile(name: String) -> Result<(), String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    // Defense-in-depth. A real read_dir name is a single component, but a hand-crafted IPC call
    // could smuggle separators/`..`, control chars, or a trailing space/dot that Windows strips
    // during path normalization (so `.claude-cc1 ` would resolve to the canon `.claude-cc1`).
    // Reject all of those so the target can only ever be an exact, literal `.claude-<name>` dir.
    if name.is_empty()
        || name.contains(['/', '\\', ':'])
        || name.contains("..")
        || name.chars().any(|c| c.is_control())
        || name != name.trim_end_matches([' ', '.'])
    {
        return Err(trv("err.orphan_bad_name", cur_lang(), &[("n", &name)]));
    }
    let canon = profile_names();
    // Never let this path touch a canon profile — that is run_profile_mgmt('remove')'s job.
    // Case-insensitive: NTFS resolves `.claude-CC1` to the canon `.claude-cc1`.
    if canon.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
        return Err(trv("err.orphan_is_canon", cur_lang(), &[("n", &name)]));
    }
    let dirname = format!(".claude-{name}");
    let dir = std::path::Path::new(&home).join(&dirname);
    let has_claude_json = dir.join(".claude.json").is_file();
    if !is_orphan_profile_dir(&dirname, has_claude_json, &canon) {
        return Err(trv("err.orphan_not_found", cur_lang(), &[("n", &name)]));
    }
    if profile_session_active(&name) {
        return Err(trv("err.orphan_session_active", cur_lang(), &[("n", &name)]));
    }
    // A profile dir's shared folders (projects/, plugins/ …) are junctions into a COMMON tree used
    // by every profile. Recycling a dir with reparse-point children risks the shell op sweeping the
    // LINK TARGET into the bin — refuse and let the user detach/remove such a dir manually.
    if has_reparse_child(&dir) {
        return Err(trv("err.orphan_has_links", cur_lang(), &[("n", &name)]));
    }
    recycle_dir(&dir.to_string_lossy())
}

/// True if any immediate child of `dir` is a reparse point (junction/symlink) — OR the dir can't be
/// enumerated. Checks the raw FILE_ATTRIBUTE_REPARSE_POINT bit (via symlink_metadata, so the link
/// itself is stat'd, not its target) — this catches junctions AND symlinks, which `is_symlink()`
/// alone can miss on Windows. Fails CLOSED on a read_dir error: if we can't prove the dir is
/// junction-free, the destructive caller must refuse rather than risk sweeping a link target.
fn has_reparse_child(dir: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    entries.flatten().any(|e| {
        e.path()
            .symlink_metadata()
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false)
    })
}

/// Move a directory to the Windows Recycle Bin via the .NET VisualBasic helper (no extra crate).
/// The path is passed through an env var (not string-interpolated) so trailing spaces / special
/// chars survive verbatim and there is no quoting to escape.
fn recycle_dir(path: &str) -> Result<(), String> {
    let script = "Add-Type -AssemblyName Microsoft.VisualBasic; \
        [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($env:CASTELLYN_DEL_PATH, \
        'OnlyErrorDialogs', 'SendToRecycleBin')";
    let out = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("CASTELLYN_DEL_PATH", path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        let msg = msg.trim();
        Err(if msg.is_empty() {
            "recycle failed".to_string()
        } else {
            msg.to_string()
        })
    }
}

/// True when some `hooks.<event>[].hooks[].command` contains `marker`.
fn hook_cmd_wired(settings: &serde_json::Value, event: &str, marker: &str) -> bool {
    settings
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|s| s.as_array())
        .is_some_and(|entries| {
            entries.iter().any(|e| {
                e.get("hooks").and_then(|h| h.as_array()).is_some_and(|hs| {
                    hs.iter().any(|h| {
                        h.get("command")
                            .and_then(|c| c.as_str())
                            .is_some_and(|c| c.contains(marker))
                    })
                })
            })
        })
}

/// Append a `{type: command}` hook entry under `hooks.<event>` (idempotent via `marker`).
/// Returns true when the value changed.
fn hook_cmd_wire(settings: &mut serde_json::Value, event: &str, cmd: &str, marker: &str) -> bool {
    if hook_cmd_wired(settings, event, marker) {
        return false;
    }
    let Some(obj) = settings.as_object_mut() else {
        return false;
    };
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    // Don't clobber a malformed (non-object) hooks value — skip this profile instead.
    let Some(hobj) = hooks.as_object_mut() else {
        return false;
    };
    let ss = hobj.entry(event).or_insert_with(|| serde_json::json!([]));
    let Some(arr) = ss.as_array_mut() else {
        return false;
    };
    arr.push(serde_json::json!({
        "hooks": [{ "type": "command", "command": cmd }]
    }));
    true
}

/// Remove every `marker`-matching hook command under `hooks.<event>` (and entries left
/// empty by that). Returns true when the value changed. Other hooks are never touched.
fn hook_cmd_unwire(settings: &mut serde_json::Value, event: &str, marker: &str) -> bool {
    let Some(ss) = settings
        .get_mut("hooks")
        .and_then(|h| h.get_mut(event))
        .and_then(|s| s.as_array_mut())
    else {
        return false;
    };
    let mut changed = false;
    for e in ss.iter_mut() {
        if let Some(hs) = e.get_mut("hooks").and_then(|h| h.as_array_mut()) {
            let before = hs.len();
            hs.retain(|h| {
                !h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains(marker))
            });
            changed |= hs.len() != before;
        }
    }
    if changed {
        ss.retain(|e| {
            e.get("hooks")
                .and_then(|h| h.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(true)
        });
    }
    changed
}

fn plugin_sync_hook_wired(settings: &serde_json::Value) -> bool {
    hook_cmd_wired(settings, "SessionStart", "plugin_sync.py")
}

fn plugin_sync_wire(settings: &mut serde_json::Value) -> bool {
    hook_cmd_wire(
        settings,
        "SessionStart",
        PLUGIN_SYNC_HOOK_CMD,
        "plugin_sync.py",
    )
}

fn plugin_sync_unwire(settings: &mut serde_json::Value) -> bool {
    hook_cmd_unwire(settings, "SessionStart", "plugin_sync.py")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginSyncStatus {
    /// Profile dir names (".claude", ".claude-cc1", …) with the SessionStart hook wired.
    wired: Vec<String>,
    unwired: Vec<String>,
    script_installed: bool,
    script_version: u32,
}

#[tauri::command]
fn plugin_sync_status() -> Result<PluginSyncStatus, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let (mut wired, mut unwired) = (Vec::new(), Vec::new());
    for (name, sp) in plugin_sync_profiles(&home) {
        let is_wired = std::fs::read_to_string(&sp)
            .ok()
            .and_then(|c| parse_json_bom(&c).ok())
            .map(|v| plugin_sync_hook_wired(&v))
            .unwrap_or(false);
        if is_wired {
            wired.push(name);
        } else {
            unwired.push(name);
        }
    }
    let script = std::fs::read_to_string(plugin_sync_script_path(&home)).ok();
    Ok(PluginSyncStatus {
        wired,
        unwired,
        script_installed: script.is_some(),
        script_version: script.map(|c| plugin_sync_version(&c)).unwrap_or(0),
    })
}

/// L5: shared per-profile settings.json sweep for plugin_sync_set + agent_status_hook_set. Reads each
/// target, applies `mutate` (true = it changed the doc), and writes atomically only on change —
/// continuing past a momentarily-locked/malformed file and collecting hard write errors to surface
/// after every profile is attempted (never an early abort that strands the remaining profiles).
fn sweep_profile_settings<K>(
    targets: Vec<(K, String)>,
    mutate: impl Fn(&mut serde_json::Value) -> bool,
) -> Result<(), String> {
    let mut errs: Vec<String> = Vec::new();
    for (_, sp) in targets {
        let Ok(c) = std::fs::read_to_string(&sp) else {
            continue;
        };
        let Ok(mut v) = parse_json_bom(&c) else {
            continue;
        };
        if mutate(&mut v) {
            match serde_json::to_string_pretty(&v) {
                Ok(s) => {
                    if let Err(e) = write_json_atomic_retry(&sp, &s) {
                        errs.push(format!("{sp}: {e}"));
                    }
                }
                Err(e) => errs.push(format!("{sp}: {e}")),
            }
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join(" · "))
    }
}

/// Wire (enable) or unwire (disable) SessionStart auto-sync in every profile; enable also
/// installs/updates the hook script. Unreadable/malformed settings files are skipped.
#[tauri::command]
fn plugin_sync_set(enabled: bool) -> Result<PluginSyncStatus, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    if enabled {
        ensure_plugin_sync_script(&home)?;
    }
    // Continue-on-error across profiles: one profile whose settings.json is momentarily locked (a
    // live session, an AV scan) must NOT abort the sweep and strand the remaining profiles half-wired.
    // Retry a transient sharing violation, collect hard failures, and surface them after trying all.
    // Enable wires the CURRENT profiles only; disable sweeps orphan-inclusive so a renamed/removed
    // profile's hook entries are also unwired (marker-gated → foreign dirs untouched).
    let targets = if enabled {
        plugin_sync_profiles(&home)
    } else {
        claude_settings_all(&home)
    };
    sweep_profile_settings(targets, |v| {
        if enabled {
            plugin_sync_wire(v)
        } else {
            plugin_sync_unwire(v)
        }
    })?;
    plugin_sync_status()
}

/// Run the reconcile once now, streaming output into the console (component "pluginsync").
#[tauri::command]
async fn run_plugin_sync(app: AppHandle, state: State<'_, RunState>) -> Result<i32, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let script = ensure_plugin_sync_script(&home)?;
    run_native_streamed(app, state, "pluginsync".to_string(), move |out, err| {
        out(&format!("py -X utf8 {script} --verbose"));
        match std::process::Command::new("py")
            .args(["-X", "utf8", &script, "--verbose"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) => {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    out(line);
                }
                for line in String::from_utf8_lossy(&o.stderr).lines() {
                    err(line);
                }
                if o.status.success() {
                    0
                } else {
                    o.status.code().unwrap_or(1)
                }
            }
            Err(e) => {
                err(&format!("py: {e}"));
                1
            }
        }
    })
    .await
}

// --- Agent-status lifecycle hook (Sessions tab) ---
//
// castellyn_status.py reports Claude Code lifecycle events (working/blocked/idle) into
// %APPDATA%\castellyn\agent-status; the agent_status module turns them into pane badges.
// Wired into five events of every profile; a session without CASTELLYN_SESSION_ID in its
// env makes the hook a no-op, so regular (non-Castellyn) Claude use is unaffected.

const STATUS_HOOK_SCRIPT: &str = include_str!("../assets/castellyn_status.py");
const STATUS_HOOK_CMD: &str = "py -X utf8 ~/.claude/hooks/castellyn_status.py";
const STATUS_HOOK_MARKER: &str = "castellyn_status.py";
const STATUS_HOOK_EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// Install/refresh the status hook script (same version-gated policy as plugin_sync).
fn ensure_status_hook_script(home: &str) -> Result<(), String> {
    let path = format!("{home}\\.claude\\hooks\\castellyn_status.py");
    let ver = |t: &str| script_version_header(t, "# castellyn-status-version:");
    let disk = std::fs::read_to_string(&path).unwrap_or_default();
    if ver(&disk) < ver(STATUS_HOOK_SCRIPT) {
        write_json_atomic(&path, STATUS_HOOK_SCRIPT).map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatusHookState {
    /// Profile dir names with ALL five lifecycle events wired.
    wired: Vec<String>,
    unwired: Vec<String>,
}

fn agent_status_hook_state() -> Result<AgentStatusHookState, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let (mut wired, mut unwired) = (Vec::new(), Vec::new());
    for (name, sp) in plugin_sync_profiles(&home) {
        let all = std::fs::read_to_string(&sp)
            .ok()
            .and_then(|c| parse_json_bom(&c).ok())
            .map(|v| {
                STATUS_HOOK_EVENTS
                    .iter()
                    .all(|ev| hook_cmd_wired(&v, ev, STATUS_HOOK_MARKER))
            })
            .unwrap_or(false);
        if all {
            wired.push(name);
        } else {
            unwired.push(name);
        }
    }
    Ok(AgentStatusHookState { wired, unwired })
}

#[tauri::command]
fn agent_status_hook_status() -> Result<AgentStatusHookState, String> {
    agent_status_hook_state()
}

/// Wire (enable) or unwire (disable) the agent-status lifecycle hook in every profile;
/// enable also installs/updates the hook script. Malformed settings files are skipped.
#[tauri::command]
fn agent_status_hook_set(enabled: bool) -> Result<AgentStatusHookState, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    if enabled {
        ensure_status_hook_script(&home)?;
    }
    // Continue-on-error across profiles (see plugin_sync_set): a momentarily-locked settings.json is
    // retried, hard failures are collected and surfaced after every profile has been attempted — never
    // an early abort that leaves later profiles unwired.
    // See plugin_sync_set: enable → current profiles; disable → orphan-inclusive, marker-gated.
    let targets = if enabled {
        plugin_sync_profiles(&home)
    } else {
        claude_settings_all(&home)
    };
    sweep_profile_settings(targets, |v| {
        let mut changed = false;
        for ev in STATUS_HOOK_EVENTS {
            changed |= if enabled {
                hook_cmd_wire(v, ev, STATUS_HOOK_CMD, STATUS_HOOK_MARKER)
            } else {
                hook_cmd_unwire(v, ev, STATUS_HOOK_MARKER)
            };
        }
        changed
    })?;
    agent_status_hook_state()
}

#[cfg(test)]
mod audit_fixes_tests {
    use super::*;

    #[test]
    fn extract_host_handles_ipv6_and_ports() {
        assert_eq!(extract_host("example.com:8080"), "example.com");
        assert_eq!(extract_host("example.com"), "example.com");
        assert_eq!(extract_host("[::1]:443"), "::1");
        assert_eq!(extract_host("[fe80::1]"), "fe80::1");
    }

    #[test]
    fn is_blocked_ip_blocks_metadata_linklocal_not_loopback() {
        use std::net::IpAddr;
        for bad in ["169.254.169.254", "169.254.1.2", "100.100.100.200", "0.0.0.0", "fe80::1"] {
            assert!(is_blocked_ip(&bad.parse::<IpAddr>().unwrap()), "should block {bad}");
        }
        // Loopback + RFC1918 stay allowed — local engines (LM Studio) live there.
        for ok in ["127.0.0.1", "192.168.1.10", "10.0.0.5", "1.1.1.1"] {
            assert!(!is_blocked_ip(&ok.parse::<IpAddr>().unwrap()), "should allow {ok}");
        }
    }

    #[test]
    fn valid_base_url_blocks_metadata_allows_local() {
        // Blocked: cloud-metadata (string list), link-local literal, bad scheme.
        assert!(valid_base_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(valid_base_url("http://metadata.google.internal/").is_err());
        assert!(valid_base_url("http://[fe80::1]:8080").is_err());
        assert!(valid_base_url("ftp://example.com").is_err());
        // Allowed: loopback literal (no DNS) + localhost (resolves to loopback).
        assert!(valid_base_url("http://127.0.0.1:1234").is_ok());
        assert!(valid_base_url("http://localhost:8080").is_ok());
    }

    #[test]
    fn plugin_id_path_safe_rejects_traversal() {
        assert!(plugin_id_path_safe("my-plugin.name_1"));
        for bad in ["", "..", "../evil", "a/b", "a\\b", "..\\x"] {
            assert!(!plugin_id_path_safe(bad), "should reject {bad:?}");
        }
    }
}

#[cfg(test)]
mod plugin_sync_tests {
    use serde_json::json;

    #[test]
    fn wire_unwire_roundtrip_preserves_other_hooks() {
        // A profile with an unrelated SessionStart hook: wiring adds ours, unwiring removes
        // ONLY ours and keeps the neighbour + the rest of the settings untouched.
        let mut v = json!({
            "env": { "X": "1" },
            "hooks": { "SessionStart": [
                { "hooks": [{ "type": "command", "command": "py other_hook.py" }] }
            ]}
        });
        assert!(super::plugin_sync_wire(&mut v));
        assert!(super::plugin_sync_hook_wired(&v));
        assert!(!super::plugin_sync_wire(&mut v)); // idempotent: second wire is a no-op
        assert_eq!(v["hooks"]["SessionStart"].as_array().unwrap().len(), 2);

        assert!(super::plugin_sync_unwire(&mut v));
        assert!(!super::plugin_sync_hook_wired(&v));
        assert!(!super::plugin_sync_unwire(&mut v)); // idempotent: nothing left to remove
        let ss = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1);
        assert_eq!(ss[0]["hooks"][0]["command"], "py other_hook.py");
        assert_eq!(v["env"]["X"], "1");
    }

    #[test]
    fn wire_creates_hooks_shape_and_detects_manual_wiring() {
        // Empty settings: wire builds hooks.SessionStart from scratch with the exact command.
        let mut v = json!({});
        assert!(super::plugin_sync_wire(&mut v));
        assert_eq!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            super::PLUGIN_SYNC_HOOK_CMD
        );
        // The user's pre-Castellyn manual wiring (same filename, any wording) counts as wired.
        let manual = json!({ "hooks": { "SessionStart": [
            { "hooks": [{ "type": "command", "command": "py -X utf8 C:/x/plugin_sync.py" }] }
        ]}});
        assert!(super::plugin_sync_hook_wired(&manual));
        // Malformed hooks value: wire refuses instead of clobbering it.
        let mut bad = json!({ "hooks": "oops" });
        assert!(!super::plugin_sync_wire(&mut bad));
        assert_eq!(bad["hooks"], "oops");
    }

    #[test]
    fn claude_profile_dirname_predicate() {
        assert!(super::is_claude_profile_dirname(".claude"));
        assert!(super::is_claude_profile_dirname(".claude-cc1"));
        // A foreign sibling matches the name filter but is marker-gated at unwire, so it's read-only.
        assert!(super::is_claude_profile_dirname(".claude-mem"));
        assert!(!super::is_claude_profile_dirname("claude"));
        assert!(!super::is_claude_profile_dirname(".config"));
        assert!(!super::is_claude_profile_dirname(".claudex")); // no separator → not a profile dir
    }

    #[test]
    fn orphan_profile_predicate() {
        let canon: Vec<String> = ["cc1", "ccmy"].iter().map(|s| s.to_string()).collect();
        // Real orphan: .claude-<name>, not canon, has the .claude.json marker → adoptable.
        assert!(super::is_orphan_profile_dir(".claude-oldtest", true, &canon));
        // Canon profile → never an orphan (delete via run_profile_mgmt('remove')).
        assert!(!super::is_orphan_profile_dir(".claude-cc1", true, &canon));
        // Trailing space/dot RESOLVES to the canon dir on Windows path normalization → must NOT be
        // an orphan, else adopt/recycle would hit the live canon profile (the data-loss guard).
        assert!(!super::is_orphan_profile_dir(".claude-cc1 ", true, &canon));
        assert!(!super::is_orphan_profile_dir(".claude-cc1.", true, &canon));
        // Case-insensitive: NTFS resolves .claude-CC1 to canon .claude-cc1 → not an orphan.
        assert!(!super::is_orphan_profile_dir(".claude-CC1", true, &canon));
        // A trailing-space name that does NOT collide with canon is STILL excluded — it can't be
        // safely targeted by normalized-path APIs (would hit its trimmed twin).
        assert!(!super::is_orphan_profile_dir(".claude-oldtest ", true, &canon));
        // No .claude.json marker → not an (adoptable) CC profile dir.
        assert!(!super::is_orphan_profile_dir(".claude-oldtest", false, &canon));
        // Foreign tool dir on the denylist → excluded even with a marker present.
        assert!(!super::is_orphan_profile_dir(".claude-mem", true, &canon));
        // Base dir and non-profile names are never orphans.
        assert!(!super::is_orphan_profile_dir(".claude", true, &canon));
        assert!(!super::is_orphan_profile_dir(".config", true, &canon));
    }

    #[test]
    fn unwire_leaves_foreign_only_settings_untouched() {
        // Orphan/foreign settings.json with only a non-Castellyn hook: the disable sweep's unwire
        // is a no-op (changed=false) across every event and the foreign hook survives verbatim.
        let mut v = json!({ "hooks": { "SessionStart": [
            { "hooks": [{ "type": "command", "command": "py -X utf8 ~/.claude-mem/hook.py" }] }
        ]}});
        assert!(!super::plugin_sync_unwire(&mut v));
        for ev in super::STATUS_HOOK_EVENTS {
            assert!(!super::hook_cmd_unwire(&mut v, ev, super::STATUS_HOOK_MARKER));
        }
        assert_eq!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "py -X utf8 ~/.claude-mem/hook.py"
        );
    }

    #[test]
    fn sharing_retry_recovers_then_gives_up() {
        use std::cell::Cell;
        use std::io::{Error, ErrorKind};
        // A sharing violation (os error 32) that clears on the 3rd try → Ok after retries.
        let calls = Cell::new(0);
        let r = super::with_sharing_retry(|| {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(Error::from_raw_os_error(32))
            } else {
                Ok(7)
            }
        });
        assert_eq!(r.unwrap(), 7);
        assert_eq!(calls.get(), 3);
        // A non-sharing error is NOT retried — returns immediately on the first call.
        let calls = Cell::new(0);
        let r = super::with_sharing_retry::<()>(|| {
            calls.set(calls.get() + 1);
            Err(Error::new(ErrorKind::PermissionDenied, "nope"))
        });
        assert!(r.is_err());
        assert_eq!(calls.get(), 1);
        // A persistent sharing violation gives up after the fixed backoff budget (1 + 3 retries).
        let calls = Cell::new(0);
        let r = super::with_sharing_retry::<()>(|| {
            calls.set(calls.get() + 1);
            Err(Error::from_raw_os_error(32))
        });
        assert!(r.is_err());
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn version_header_parses() {
        assert_eq!(super::plugin_sync_version("# plugin-sync-version: 2\nrest"), 2);
        assert_eq!(super::plugin_sync_version("import json"), 0);
        // The embedded asset must carry a parsable version (guards accidental header edits).
        assert!(super::plugin_sync_version(super::PLUGIN_SYNC_SCRIPT) >= 2);
    }

    #[test]
    fn status_hook_wires_all_events_and_unwires_cleanly() {
        // Wiring all five lifecycle events must be idempotent and reversible without
        // touching an unrelated hook that shares one of the events.
        let mut v = json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": "py other.py" }] }
        ]}});
        for ev in super::STATUS_HOOK_EVENTS {
            assert!(super::hook_cmd_wire(&mut v, ev, super::STATUS_HOOK_CMD, super::STATUS_HOOK_MARKER));
            assert!(!super::hook_cmd_wire(&mut v, ev, super::STATUS_HOOK_CMD, super::STATUS_HOOK_MARKER));
        }
        assert!(super::STATUS_HOOK_EVENTS
            .iter()
            .all(|ev| super::hook_cmd_wired(&v, ev, super::STATUS_HOOK_MARKER)));
        for ev in super::STATUS_HOOK_EVENTS {
            assert!(super::hook_cmd_unwire(&mut v, ev, super::STATUS_HOOK_MARKER));
        }
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "py other.py");
        // Version header of the embedded status script parses.
        assert!(
            super::script_version_header(super::STATUS_HOOK_SCRIPT, "# castellyn-status-version:")
                >= 1
        );
    }

    #[test]
    fn render_substitutes_profile_list() {
        // The generated script must carry the exact dir list on the marker line and keep the
        // version header intact (ensure_plugin_sync_script compares version + content).
        let dirs = vec![".claude".to_string(), ".claude-cc1".to_string()];
        let s = super::render_plugin_sync_script(&dirs);
        assert!(s.contains(r#"PROFILES = [".claude", ".claude-cc1"]  # castellyn:profiles"#));
        assert_eq!(
            super::plugin_sync_version(&s),
            super::plugin_sync_version(super::PLUGIN_SYNC_SCRIPT)
        );
        // Rendering is deterministic — same input, same output (change-compare relies on it).
        assert_eq!(s, super::render_plugin_sync_script(&dirs));
    }
}

#[tauri::command]
fn read_config() -> HubConfig {
    read_config_file()
}

/// Serialize a config to disk verbatim (no field preservation). The single file-write primitive.
fn write_config_file(config: &HubConfig) -> Result<(), String> {
    let p = config_path().ok_or_else(|| tr("err.no_appdata", cur_lang()).to_string())?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    // Atomic temp+rename (+ .bak), UTF-8 no BOM — a crash must never blank config.json.
    write_json_atomic(&p, &json)
        .map_err(|e| trv("err.write_config", cur_lang(), &[("e", &e.to_string())]))?;
    // Keep the read cache consistent with what we just persisted (the single invalidation point).
    *CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner()) = Some(config.clone());
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
        // F11: backup folder path (Backups/) so Settings → About can offer "Open in Explorer".
        "backupDir": abs(BACKUP_DIR_REL),
    })
}

/// Export the current Castellyn config to a user-chosen path (#117). Serializes HubConfig so the
/// file is always valid even if config.json was never written.
#[tauri::command]
fn export_config(dest: String) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&read_config_file()).map_err(|e| e.to_string())?;
    std::fs::write(&dest, json).map_err(|e| trv("err.write", cur_lang(), &[("e", &e)]))
}

/// Read + validate a config file (#117); returns the parsed HubConfig (the frontend persists it
/// via write_config). Invalid JSON / wrong shape → Err.
#[tauri::command]
fn import_config(src: String) -> Result<HubConfig, String> {
    let text =
        std::fs::read_to_string(&src).map_err(|e| trv("err.read", cur_lang(), &[("e", &e)]))?;
    // BOM-tolerant like every other file read (PowerShell-written configs often carry one).
    serde_json::from_str::<HubConfig>(text.trim_start_matches('\u{feff}'))
        .map_err(|e| trv("err.bad_config_file", cur_lang(), &[("e", &e)]))
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
            let mode = p
                .get("mode")
                .and_then(|x| x.as_str())
                .unwrap_or("full")
                .to_string();
            let mcp = p
                .get("mcp")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
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
    // R3-03: previously a fixed `claude-hub-mcp-<name>.json` in the shared temp dir — predictable and
    // never cleaned, so two profiles launching at once could collide on it. Best-effort sweep this
    // profile's stale temp configs from prior launches, then write a uniquely-suffixed fresh one.
    let dir = std::env::temp_dir();
    let prefix = format!("claude-hub-mcp-{name}-");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with(&prefix) && n.ends_with(".json") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let tmp = dir.join(format!("{prefix}{}.json", gen_session_id()));
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
    LaunchConfigStatus {
        profiles,
        available_mcp,
    }
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
        return Err(trv(
            "err.invalid_profile_name",
            cur_lang(),
            &[("name", &name)],
        ));
    }
    if !matches!(mode.as_str(), "full" | "lean") {
        return Err(trv("err.unknown_mode", cur_lang(), &[("mode", &mode)]));
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
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_json_atomic(&path, &json).map_err(|e| format!("write profile-launch.json: {e}"))?;
    Ok(())
}

/// Measure a profile's effective system-prompt size: run `claude [lean flags] -p ok
/// --output-format json` and return usage.input_tokens. Lean is fast; full hits the model
/// with the big prompt (slow on a local engine), so this is invoked on demand only.
#[tauri::command]
async fn measure_context(name: String, lean: bool) -> Result<i64, String> {
    if !valid_profile_name(&name) {
        return Err(trv(
            "err.invalid_profile_name",
            cur_lang(),
            &[("name", &name)],
        ));
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
    cmd.creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Spawn explicitly (not cmd.output()) so a timeout can kill the orphaned tree: tokio drops the
    // Child on timeout but does NOT kill it, leaving `cmd /c claude` (+ its node child) running after
    // a real model call. kill_tree reaps the whole tree on the timeout branch.
    let child = cmd
        .spawn()
        .map_err(|e| trv("err.claude_failed", cur_lang(), &[("e", &e)]))?;
    let pid = child.id();
    let out = match tokio::time::timeout(
        std::time::Duration::from_secs(180),
        child.wait_with_output(),
    )
    .await
    {
        Ok(r) => r.map_err(|e| trv("err.claude_failed", cur_lang(), &[("e", &e)]))?,
        Err(_) => {
            if let Some(p) = pid {
                let _ = kill_tree(p);
            }
            return Err(tr("err.measure_timeout", cur_lang()).to_string());
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    // claude may print startup/log lines before the single JSON result (esp. with MCP servers),
    // so extract the outermost {...} rather than assuming the whole output is JSON.
    let raw = stdout.trim();
    let json_str = match (raw.find('{'), raw.rfind('}')) {
        (Some(a), Some(b)) if b > a => &raw[a..=b],
        _ => raw,
    };
    let v = parse_json_bom(json_str).map_err(|_| {
        trv(
            "err.parse_claude",
            cur_lang(),
            &[("e", &raw.chars().take(200).collect::<String>())],
        )
    })?;
    v.get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|t| t.as_i64())
        .ok_or_else(|| tr("err.no_usage_tokens", cur_lang()).to_string())
}

/// Launch a profile: open a console with CLAUDE_CONFIG_DIR set and `claude` running under it.
/// `mode` is accepted for API compatibility but only "terminal" is supported (the VS Code launch
/// was removed — `code` CLI can't reliably pass env to an already-running instance nor auto-open
/// a terminal). Honors the profile's saved launch config (full vs lean → lean CLI flags inline).
#[tauri::command]
fn launch_profile(name: String, mode: String) -> Result<(), String> {
    if !PROFILE_NAMES.contains(&name.as_str()) && !valid_profile_name(&name) {
        return Err(trv(
            "err.invalid_profile",
            cur_lang(),
            &[("profile", &name)],
        ));
    }
    if mode != "terminal" {
        return Err(trv(
            "err.unsupported_launch_mode",
            cur_lang(),
            &[("mode", &mode)],
        ));
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
    cmd.args([
        "/c",
        "start",
        &format!("Claude {name}"),
        "cmd",
        "/k",
        &claude_cmd,
    ])
    .env("CLAUDE_CONFIG_DIR", &dir);
    for (k, v) in profile_env_pairs(&name) {
        cmd.env(k, v);
    }
    cmd.creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| trv("err.open_terminal", cur_lang(), &[("e", &e)]))?;
    Ok(())
}

/// Open a terminal (cmd) at `path` — e.g. a repo dir, to resolve a conflict with Claude Code.
#[tauri::command]
fn open_terminal(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &path)]));
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
        .map_err(|e| trv("err.open_terminal", cur_lang(), &[("e", &e)]))?;
    Ok(())
}

/// Open a folder/file in Explorer. Guard (R3-05): only launch for a path that actually exists, so
/// a missing/odd target can't be handed to Explorer (single-user local app — existence is the bar).
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &path)]));
    }
    std::process::Command::new("explorer")
        .arg(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| trv("err.open_path", cur_lang(), &[("path", &path), ("e", &e)]))?;
    Ok(())
}

/// F10: clone a GitHub repo to a user-picked path via the git CLI. Blocks until the clone finishes;
/// `target` is the full destination dir (picked parent + repo name). Only https URLs are accepted.
#[tauri::command]
fn clone_repo(url: String, target: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(trv("err.bad_url_scheme", cur_lang(), &[("url", &url)]));
    }
    if std::path::Path::new(&target).exists() {
        return Err(trv(
            "err.clone_target_exists",
            cur_lang(),
            &[("path", &target)],
        ));
    }
    let out = std::process::Command::new("git")
        .args(["clone", "--", &url, &target])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.git_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        Err(msg.trim().to_string())
    }
}

/// Open a web URL in the default browser via the opener plugin. (open_path is for filesystem paths —
/// using it on an https URL fails with "directory not found".)
#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // Only ever hand http/https to the OS opener. A file://, UNC, or custom-scheme value (which can
    // reach here via fork remote/upstream metadata in a compare link) would otherwise launch a
    // program rather than a browser — restrict the scheme before calling the plugin.
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        return Err(trv("err.bad_url_scheme", cur_lang(), &[("url", &url)]));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
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
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let exe = exe.display().to_string();
    let added = std::process::Command::new("reg")
        .args([
            "add",
            AUTOSTART_KEY,
            "/v",
            AUTOSTART_NAME,
            "/t",
            "REG_SZ",
            "/d",
            &exe,
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if added {
        // L6: the migration promises "drop the old value ONLY once the new one is in place". If this
        // delete fails (Err, or reg returns non-zero), both AgentHub and Castellyn Run entries point at
        // the same exe → one extra duplicate-launch boot until it self-heals next start. Log it so a
        // persistent failure is diagnosable instead of being silently swallowed by `let _ = ...`.
        match std::process::Command::new("reg")
            .args(["delete", AUTOSTART_KEY, "/v", LEGACY_AUTOSTART_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) if !o.status.success() => eprintln!(
                "migrate_autostart: reg delete of legacy '{LEGACY_AUTOSTART_NAME}' returned {}",
                o.status
            ),
            Err(e) => eprintln!(
                "migrate_autostart: reg delete of legacy '{LEGACY_AUTOSTART_NAME}' failed: {e}"
            ),
            _ => {}
        }
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
/// Both branches now surface success/failure honestly — the off-path used to swallow
/// `reg delete` errors via `let _ = ...output()` so a failed unregistration left the
/// registry value orphaned while the UI believed "off" had taken effect.
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .display()
            .to_string();
        let out = std::process::Command::new("reg")
            .args([
                "add",
                AUTOSTART_KEY,
                "/v",
                AUTOSTART_NAME,
                "/t",
                "REG_SZ",
                "/d",
                &exe,
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
    } else {
        let out = std::process::Command::new("reg")
            .args(["delete", AUTOSTART_KEY, "/v", AUTOSTART_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
    }
    Ok(())
}

/// Open a profile's config dir (%USERPROFILE%\.claude-<name>) in Explorer.
#[tauri::command]
fn open_profile_dir(name: String) -> Result<(), String> {
    if !valid_profile_name(&name) {
        return Err(tr("err.invalid_profile_name_plain", cur_lang()).into());
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let path = format!("{home}\\.claude-{name}");
    std::process::Command::new("explorer")
        .arg(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| trv("err.open_path", cur_lang(), &[("path", &path), ("e", &e)]))?;
    Ok(())
}

/// Kill the currently-running child process tree (Windows: taskkill /T /F).
#[tauri::command]
fn cancel_run(state: State<'_, RunState>) -> Result<(), String> {
    // M1: also break a bulk "Repair All" loop between profiles. Harmless for other single runs —
    // repair_all_profiles resets this flag at its start, so a stale set can't affect a later run.
    PROFILES_BULK_CANCEL.store(true, Ordering::SeqCst);
    let pid = { *state.0.lock().unwrap_or_else(|e| e.into_inner()) };
    match pid {
        Some(p) if p != 0 => kill_tree(p),
        _ => Err(tr("err.no_active_run", cur_lang()).into()),
    }
}

/// F21: global panic button — kills the single-slot run, every per-repo fork run, every live PTY
/// session, and flips the bulk-plugin cancel flag. Best-effort (ignores per-target kill errors) and
/// emits 'cancel-all-done' so the UI can refresh. Bound to Ctrl+Shift+Backspace and a tray entry.
#[tauri::command]
fn cancel_all(
    app: AppHandle,
    run: State<'_, RunState>,
    forks: State<'_, ForkRuns>,
    sessions: State<'_, SessionState>,
) -> Result<(), String> {
    // 1. The single-slot run (backup / profiles / sync / engine / component / single plugin op).
    let run_pid = { *run.0.lock().unwrap_or_else(|e| e.into_inner()) };
    if let Some(p) = run_pid {
        if p != 0 {
            let _ = kill_tree(p);
        }
    }
    // 2. Every per-repo fork run (each removes itself from the map once its pid dies).
    let fork_pids: Vec<u32> = {
        let m = forks.0.lock().unwrap_or_else(|e| e.into_inner());
        m.values().copied().filter(|p| *p != 0).collect()
    };
    for p in fork_pids {
        let _ = kill_tree(p);
    }
    // 3. A bulk plugin sweep OR a bulk profile Repair-All stops at the next item boundary.
    BULK_PLUGINS_CANCEL.store(true, Ordering::SeqCst);
    PROFILES_BULK_CANCEL.store(true, Ordering::SeqCst);
    // 4. Every live PTY session (drain so their reader threads end on EOF).
    {
        let mut map = sessions.0.lock().unwrap_or_else(|e| e.into_inner());
        for (_, mut s) in map.drain() {
            let _ = s.killer.kill();
        }
    }
    update_tray_tooltip(&app);
    let _ = app.emit("cancel-all-done", ());
    Ok(())
}

/// Build the tray menu with labels in the given locale. Shared by initial build + relabel-on-switch.
/// F18: extended via the current Tauri v2 MenuBuilder (replaced the deprecated Menu::with_items).
/// Each id below has a matching arm in build_tray's on_menu_event — keep the two in sync.
fn tray_menu(app: &AppHandle, lang: Lang) -> tauri::Result<Menu<tauri::Wry>> {
    MenuBuilder::new(app)
        .text("show", tr("tray.show", lang))
        .separator()
        .text("check_all", tr("tray.check_all", lang))
        .text("refresh_forks", tr("tray.refresh_forks", lang))
        .text("refresh_providers", tr("tray.refresh_providers", lang))
        .separator()
        .text("stack_start", tr("tray.stack_start", lang))
        .text("stack_stop", tr("tray.stack_stop", lang))
        .separator()
        .text("open_backup", tr("tray.open_backup", lang))
        .text("open_settings", tr("tray.open_settings", lang))
        .separator()
        .text("cancel_all", tr("tray.cancel_all", lang))
        .text("quit", tr("tray.quit", lang))
        .build()
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
            // F18: the new entries emit an event the frontend handles (mirrors tray-check-all).
            // reveal() first for everything that talks back through in-window UI: stack_stop opens a
            // confirm dialog and stack_start reports only via the run log/toasts, so a hidden window
            // silently swallows both (R3). cancel_all stays headless on purpose — it must never steal
            // focus and its effect (things stop) needs no dialog.
            "refresh_forks" => {
                reveal(app);
                let _ = app.emit("tray-refresh-forks", ());
            }
            "refresh_providers" => {
                reveal(app);
                let _ = app.emit("tray-refresh-providers", ());
            }
            "stack_start" => {
                reveal(app);
                let _ = app.emit("tray-stack-start", ());
            }
            "stack_stop" => {
                reveal(app);
                let _ = app.emit("tray-stack-stop", ());
            }
            "open_backup" => {
                reveal(app);
                let _ = app.emit("tray-open-tab", "backup");
            }
            "open_settings" => {
                reveal(app);
                let _ = app.emit("tray-open-tab", "settings");
            }
            "cancel_all" => {
                let _ = app.emit("tray-cancel-all", ());
            }
            "quit" => {
                // F19: confirm before quitting (live sessions die) — defer to the frontend dialog.
                reveal(app);
                let _ = app.emit("tray-quit-request", ());
            }
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
        trv(
            "tray.tooltip_sessions",
            cur_lang(),
            &[("n", &n.to_string())],
        )
    };
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(&label));
    }
}

/// Register a single shortcut (no unregister_all). Errors on a bad/taken combo.
fn register_shortcut(app: &AppHandle, _accel: &str) -> Result<(), String> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let sc =
        Shortcut::from_str(_accel).map_err(|e| trv("err.bad_hotkey", cur_lang(), &[("e", &e)]))?;
    app.global_shortcut()
        .register(sc)
        .map_err(|e| format!("{e}"))
}

/// Register (replacing any previous) the OS-global show/hide accelerator. Errors on a bad/taken combo.
fn register_toggle_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    register_shortcut(app, accel)?;
    let _ = app.global_shortcut().unregister_all();
    register_shortcut(app, accel)
}

/// Apply a new toggle hotkey at runtime. Empty/None clears it. Config persistence is the frontend's job.
#[tauri::command]
fn set_toggle_hotkey(app: AppHandle, accel: Option<String>) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    match accel.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(a) => {
            // Also update the shortcuts map for consistency.
            let mut cfg = read_config_file();
            let mut m = cfg.shortcuts.clone().unwrap_or_default();
            m.insert("toggle_window".to_string(), a.to_string());
            cfg.shortcuts = Some(m);
            let _ = write_config_file(&cfg);
            register_toggle_hotkey(&app, a)
        }
        None => {
            let _ = app.global_shortcut().unregister_all();
            Ok(())
        }
    }
}

/// Return the current shortcut mapping (action → accelerator). Empty map = none configured.
#[tauri::command]
fn read_shortcuts() -> HashMap<String, String> {
    read_config_file().shortcuts.unwrap_or_default()
}

/// Replace the entire shortcut mapping and re-register all OS-level accelerators.
/// Persists to config; fails fast if any combo is invalid / taken.
#[tauri::command]
fn set_shortcuts(app: AppHandle, shortcuts: HashMap<String, String>) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let non_empty: Vec<&str> = shortcuts
        .values()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    // Teardown-FIRST: the old set must be unregistered before registering the new one. Probing the
    // new accels while the old set is still live made a KEPT combo (unchanged between old and new)
    // fail register() with "already registered", aborting the whole apply — so shortcuts could never
    // be changed while any was retained. On a genuinely bad/taken accel, roll the whole set back
    // (unregister_all) and surface the error instead of leaving a half-registered set.
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    for accel in &non_empty {
        if let Err(e) = register_shortcut(&app, accel) {
            let _ = gs.unregister_all();
            return Err(e);
        }
    }
    // All applied cleanly → persist.
    let mut cfg = read_config_file();
    cfg.shortcuts = Some(shortcuts.clone());
    cfg.toggle_hotkey = shortcuts.get("toggle_window").cloned();
    write_config_file(&cfg)?;
    Ok(())
}

// ===================== Parallel terminal sessions (real PTYs) =====================
// Each session runs a tool in a true PTY (portable-pty) so its TUI renders in an xterm.js pane.
// Output streams as raw bytes over per-window ipc Channels; input/resize flow back via commands.
// Fan-out: one session can feed SEVERAL windows at once (multi-monitor / live pop-out) — the reader
// broadcasts each chunk to every attached channel, and a bounded scrollback is replayed on attach.
// The live sessions live in Tauri-managed state.
type OutChan = tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>;
const SESSION_RING_MAX: usize = 256 * 1024; // bytes of scrollback kept for late-attaching windows
const SESSION_LIMIT: usize = 24; // global ceiling across ALL windows (main grid caps at 12 panes)

struct PtySession {
    master: Box<dyn portable_pty::MasterPty + Send>,
    // Per-session writer behind its own lock so session_write doesn't hold the whole SessionState map
    // lock across a (potentially blocking) PTY write — mirrors `chans`/`ring` below.
    writer: std::sync::Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    // Killer handle only: the Child itself moves into the reader thread so it can wait() for the
    // real exit code. session_kill signals termination through this.
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    // Every attached window's output channel as (token, chan); reader broadcasts to all, dropping
    // dead ones. The token lets a specific window detach its channel (session_detach) without a kill.
    chans: std::sync::Arc<Mutex<Vec<(u64, OutChan)>>>,
    // Recent output, replayed to a freshly attached window so it isn't blank.
    ring: std::sync::Arc<Mutex<std::collections::VecDeque<u8>>>,
    // Channel-token counter: the spawner is token 0, attaches get 1,2,… (used by session_detach).
    next_token: std::sync::atomic::AtomicU64,
}

/// Append `bytes` to a bounded byte ring, dropping the oldest bytes so it never exceeds `max`.
fn push_bounded(rb: &mut std::collections::VecDeque<u8>, bytes: &[u8], max: usize) {
    rb.extend(bytes.iter().copied());
    let over = rb.len().saturating_sub(max);
    if over > 0 {
        rb.drain(0..over);
    }
}

#[derive(Default)]
struct SessionState(Mutex<std::collections::HashMap<String, PtySession>>);

fn gen_session_id() -> String {
    // R3-04: time-only ids (nanos masked) could collide on a same-instant spawn and overwrite a
    // live session. Mix in a process-wide monotonic counter so two ids in the same tick differ.
    static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    // 12 hex of (48-bit) nanos + 3 hex of counter = 15 hex → "s" + 15 = 16-char id (shape contract).
    format!(
        "s{:012x}{:03x}",
        (n as u64) & 0x0000_ffff_ffff_ffff,
        seq & 0xfff
    )
}

/// F16/F19: live PTY session count across ALL windows (the global SESSION_LIMIT pool).
#[tauri::command]
fn global_session_count(state: State<'_, SessionState>) -> usize {
    state.0.lock().unwrap_or_else(|e| e.into_inner()).len()
}

/// F19: hard-exit the app — called from the frontend after the tray-Quit confirm.
#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Spawn a tool (claude / opencode / shell / ssh) inside a real PTY and stream its output. Returns the
/// session id. Output → the caller's `on_data` Channel (raw bytes); termination → `pty:exit:<id>` (exit i32).
#[tauri::command]
// command handler: args come from the JS invoke boundary
#[allow(clippy::too_many_arguments)]
fn session_spawn(
    app: AppHandle,
    state: State<'_, SessionState>,
    profile: String,
    tool: Option<String>,
    args: Option<String>,
    cwd: Option<String>,
    remote_dir: Option<String>,
    ssh_target: Option<String>,
    cols: u16,
    rows: u16,
    on_data: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<String, String> {
    use portable_pty::{CommandBuilder, PtySize};
    let tool = tool.unwrap_or_else(|| "claude".into());
    if !matches!(
        tool.as_str(),
        "claude" | "opencode" | "codex" | "shell" | "ssh"
    ) {
        return Err(trv("err.unknown_tool", cur_lang(), &[("tool", &tool)]));
    }
    // Global session ceiling across ALL windows — the main grid's MAX_PANES=12 is per-window, but
    // detached windows + restore can otherwise burst past it and swamp the machine.
    if state.0.lock().unwrap_or_else(|e| e.into_inner()).len() >= SESSION_LIMIT {
        return Err(trv(
            "err.session_limit",
            cur_lang(),
            &[("max", &SESSION_LIMIT)],
        ));
    }
    // The profile only matters for claude (it picks CLAUDE_CONFIG_DIR = ~/.claude-<name>).
    if tool == "claude" && !valid_profile_name(&profile) {
        return Err(trv(
            "err.invalid_profile",
            cur_lang(),
            &[("profile", &profile)],
        ));
    }
    let size = PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = portable_pty::native_pty_system()
        .openpty(size)
        .map_err(|e| format!("openpty: {e}"))?;

    // Tools are .cmd shims (claude/opencode) or a real exe (ssh) → launch inside pwsh; -NoExit keeps
    // the pane usable after the tool/connection exits. `shell` opens a bare interactive pwsh. For
    // `ssh` the target+flags (e.g. `user@host -p 22 -i key`) arrive as `extra` and run as `ssh <extra>`
    // — host-key/password prompts are handled interactively in the PTY (no flags forced; ~/.ssh used).
    // Extra args are the user's own input on their own machine, appended to the launch command verbatim.
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let extra = args.unwrap_or_default();
    let extra = extra.trim();
    // Model: environment (claude/opencode/shell) × location (local | ssh_target). `ssh` as a tool is
    // legacy (kept until the UI migrates) and means "bare remote shell with target in `args`".
    let ssh = ssh_target
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let remote = remote_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());

    // The session id is minted BEFORE the command builds so the child can carry it in
    // CASTELLYN_SESSION_ID — the castellyn_status.py lifecycle hook keys its report file
    // on it (agent_status module). Sessions outside Castellyn have no id → hook no-ops.
    let id = gen_session_id();

    let mut cmd = CommandBuilder::new("pwsh");
    cmd.arg("-NoLogo");
    cmd.env("CASTELLYN_SESSION_ID", &id);
    // Over SSH the local pwsh is only a launcher for ssh.exe — skip the user's profile banner.
    if ssh.is_some() || tool == "ssh" {
        cmd.arg("-NoProfile");
    }

    // Build the pwsh `-Command`, or leave it bare for a LOCAL interactive shell. `--%` (stop-parsing)
    // hands the ssh line to ssh.exe verbatim; a remote dir / remote tool ride an EncodedCommand
    // (base64 UTF-16) so quotes/spaces/Cyrillic survive local→ssh→remote re-parsing (Windows/PowerShell
    // remote, like minipc; mirrors grid-main.ps1). `-t` forces a PTY, `-NoExit` keeps the shell alive.
    let command: Option<String> = if let Some(target) = ssh {
        // Environment over SSH: run the tool ON the remote (shell = bare remote shell).
        let mut parts: Vec<String> = Vec::new();
        if let Some(dir) = remote {
            parts.push(format!(
                "Set-Location -LiteralPath '{}'",
                dir.replace('\'', "''")
            ));
        }
        match tool.as_str() {
            "claude" => parts.push(if extra.is_empty() {
                "claude".into()
            } else {
                format!("claude {extra}")
            }),
            "opencode" => parts.push(if extra.is_empty() {
                "opencode".into()
            } else {
                format!("opencode {extra}")
            }),
            "codex" => parts.push(if extra.is_empty() {
                "codex".into()
            } else {
                format!("codex {extra}")
            }),
            _ => {} // shell: nothing extra — just a remote shell
        }
        Some(if parts.is_empty() {
            format!("ssh --% -t {target}")
        } else {
            format!(
                "ssh --% -t {target} powershell -NoExit -EncodedCommand {}",
                ps_encoded_command(&parts.join("; "))
            )
        })
    } else if tool == "ssh" {
        // Legacy: target rides `args`; optional remote dir via EncodedCommand.
        Some(match (extra.is_empty(), remote) {
            (false, Some(dir)) => {
                let ps = format!("Set-Location -LiteralPath '{}'", dir.replace('\'', "''"));
                format!(
                    "ssh --% -t {extra} powershell -NoExit -EncodedCommand {}",
                    ps_encoded_command(&ps)
                )
            }
            (true, _) => "ssh".to_string(),
            (false, None) => format!("ssh --% {extra}"),
        })
    } else if tool == "shell" {
        None // local interactive PowerShell — no -Command
    } else {
        let base = match tool.as_str() {
            "opencode" => "opencode",
            "codex" => "codex",
            _ => "claude",
        };
        Some(if extra.is_empty() {
            base.to_string()
        } else {
            format!("{base} {extra}")
        })
    };
    if let Some(c) = command {
        cmd.arg("-NoExit");
        cmd.arg("-Command");
        cmd.arg(c);
    }
    // CLAUDE_CONFIG_DIR picks the profile for a LOCAL claude (the remote uses its own config).
    if tool == "claude" && ssh.is_none() {
        cmd.env("CLAUDE_CONFIG_DIR", format!("{home}\\.claude-{profile}"));
    }
    let dir = cwd
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| home.clone());
    if !dir.is_empty() {
        cmd.cwd(dir);
    }

    use portable_pty::{Child, ChildKiller};
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn: {e}"))?;
    drop(pair.slave); // close the slave in the parent so EOF arrives when the child exits
                      // Tie the child's process tree to the kill-on-close Job Object (#4): a crash/forced exit then
                      // can't leave orphaned node/ssh grandchildren running. Best-effort — never fail a spawn over it.
    if let Some(pid) = child.process_id() {
        assign_to_kill_job(pid);
    }
    // Keep a killer handle for session_kill; the Child moves into the reader thread so it can wait()
    // and report the tool's REAL exit code instead of a hardcoded 0. Created BEFORE the fallible
    // reader/writer clones (L8) so we can tear the already-spawned child back down if either fails —
    // otherwise it leaks untracked (portable-pty's Windows Child has no Drop-kill), reaped only when
    // the app exits and the Job Object closes.
    let mut killer: Box<dyn ChildKiller + Send + Sync> = child.clone_killer();
    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = killer.kill();
            return Err(format!("reader: {e}"));
        }
    };
    let writer: std::sync::Arc<Mutex<Box<dyn std::io::Write + Send>>> =
        std::sync::Arc::new(Mutex::new(match pair.master.take_writer() {
            Ok(w) => w,
            Err(e) => {
                let _ = killer.kill();
                return Err(format!("writer: {e}"));
            }
        }));

    let exit_event = format!("pty:exit:{id}");

    // Fan-out state: the spawning window's channel is the first subscriber; more windows can attach
    // later (session_attach) for multi-monitor / live pop-out.
    let chans: std::sync::Arc<Mutex<Vec<(u64, OutChan)>>> =
        std::sync::Arc::new(Mutex::new(vec![(0u64, on_data)])); // spawner = token 0
    let ring: std::sync::Arc<Mutex<std::collections::VecDeque<u8>>> =
        std::sync::Arc::new(Mutex::new(std::collections::VecDeque::new()));

    // Register the session under the SAME lock that enforces the ceiling, BEFORE spawning the reader
    // thread. This (a) makes the limit check + insert atomic, so two concurrent spawns can't both slip
    // past SESSION_LIMIT (the early check above is just a fast fail), and (b) guarantees the reader's
    // EOF cleanup below can never race ahead of the insert and leave a dead map entry.
    {
        let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= SESSION_LIMIT {
            let mut k = killer;
            let _ = k.kill(); // lost the race against a concurrent spawn — tear the child back down
            return Err(trv(
                "err.session_limit",
                cur_lang(),
                &[("max", &SESSION_LIMIT)],
            ));
        }
        map.insert(
            id.clone(),
            PtySession {
                master: pair.master,
                writer,
                killer,
                chans: chans.clone(),
                ring: ring.clone(),
                next_token: std::sync::atomic::AtomicU64::new(1),
            },
        );
    }
    // Track for agent status (skips shell/ssh; remote agents get PTY-activity only —
    // their hooks run on the remote host and never reach the local status dir).
    agent_status::on_spawn(&id, &tool, &profile);

    // Reader thread: stream PTY output as raw bytes to EVERY attached channel (no base64/JSON event
    // per chunk) until EOF, keeping a bounded scrollback; then wait for the child and signal exit.
    // The session stays alive even with zero attached windows (output keeps buffering into the ring).
    let app2 = app.clone();
    let chans_r = chans;
    let ring_r = ring;
    let id_r = id.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        // 32 KiB read buffer (#16, partial flow control): under a firehose (`yes`, `cat bigfile`) a
        // bigger read collapses ~4× the per-chunk syscalls AND Channel IPC messages, cutting the
        // message-flood cost. read() still returns whatever's available, so interactive output stays
        // snappy. True backpressure (a frontend→backend credit/ack so a slow xterm can pause the
        // reader) remains a follow-up — this only thins the flood, it doesn't bound it.
        let mut buf = [0u8; 32 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let bytes = &buf[..n];
                    agent_status::on_output(&id_r, n);
                    // #21b: flag a usage-limit banner in this session's output (bounded tail scan).
                    agent_status::scan_limit(&id_r, bytes);
                    push_bounded(
                        &mut ring_r.lock().unwrap_or_else(|e| e.into_inner()),
                        bytes,
                        SESSION_RING_MAX,
                    );
                    // Raw body → each JS side gets an ArrayBuffer (binary). Drop channels whose window closed.
                    let mut cs = chans_r.lock().unwrap_or_else(|e| e.into_inner());
                    cs.retain(|(_, c)| {
                        c.send(tauri::ipc::InvokeResponseBody::Raw(bytes.to_vec()))
                            .is_ok()
                    });
                }
            }
        }
        // EOF means the child has exited; surface its real exit code (-1 if wait() fails).
        let code = Child::wait(&mut *child)
            .map(|s| s.exit_code() as i32)
            .unwrap_or(-1);
        agent_status::on_exit(&id_r);
        let _ = app2.emit(exit_event.as_str(), code);
        // Reap from the map: a naturally-exited but still-open pane otherwise holds its SESSION_LIMIT
        // slot + master/ring until session_kill (which only runs when the pane is explicitly closed).
        let _ = app2
            .state::<SessionState>()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id_r);
    });

    update_tray_tooltip(&app);
    Ok(id)
}

/// Forward keystrokes (UTF-8) from an xterm pane into the PTY.
#[tauri::command]
fn session_write(state: State<'_, SessionState>, id: String, data: String) -> Result<(), String> {
    use std::io::Write;
    // Clone out the per-session writer handle under the map lock, then RELEASE the map lock before the
    // (potentially blocking) PTY write — otherwise one stalled child head-of-lines every other session
    // op (spawn's atomic insert, kill, resize, tray tooltip) that needs the same map lock.
    let writer = {
        let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(&id)
            .ok_or(tr("err.session_not_found", cur_lang()))?;
        s.writer.clone()
    };
    let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
    w.write_all(data.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    w.flush().map_err(|e| format!("flush: {e}"))
}

/// Resize the PTY when its pane changes size (xterm fit addon).
#[tauri::command]
fn session_resize(
    state: State<'_, SessionState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    use portable_pty::PtySize;
    let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
    let s = map
        .get(&id)
        .ok_or(tr("err.session_not_found", cur_lang()))?;
    s.master
        .resize(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("resize: {e}"))
}

/// Kill a session's child process and drop it (its reader thread then ends on EOF).
#[tauri::command]
fn session_kill(app: AppHandle, state: State<'_, SessionState>, id: String) -> Result<(), String> {
    if let Some(mut s) = state
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id)
    {
        let _ = s.killer.kill();
    }
    update_tray_tooltip(&app);
    Ok(())
}

/// Attach an extra output channel to a LIVE session (a second window rendering it, e.g. on another
/// monitor). Replays the bounded scrollback so the new window isn't blank, then joins the fan-out.
/// Errors if the session is gone. This is the live-move primitive: a window attaches, the old one's
/// channel drops itself on close — the PTY never restarts.
#[tauri::command]
fn session_attach(
    state: State<'_, SessionState>,
    id: String,
    on_data: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<u64, String> {
    // Grab the session's shared handles + a fresh channel token, then DROP the global map lock before
    // the (up-to-256KB) replay send — otherwise every keystroke (session_write) / resize / kill / spawn
    // stalls for its duration. The token is returned so this window can later session_detach itself.
    let (ring, chans, token) = {
        let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let s = map
            .get(&id)
            .ok_or(tr("err.session_not_found", cur_lang()))?;
        let token = s.next_token.fetch_add(1, Ordering::Relaxed);
        (s.ring.clone(), s.chans.clone(), token)
    };
    {
        let rb = ring.lock().unwrap_or_else(|e| e.into_inner());
        if !rb.is_empty() {
            let snapshot: Vec<u8> = rb.iter().copied().collect();
            let _ = on_data.send(tauri::ipc::InvokeResponseBody::Raw(snapshot));
        }
    }
    chans
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((token, on_data));
    Ok(token)
}

/// Drop one window's channel from a session's fan-out (by token) WITHOUT killing the session — used
/// when a popped-out window closes or a pane is moved back, so the reader stops sending to a gone
/// webview instead of waiting to notice on the next failed send. No-op if the session is already gone.
#[tauri::command]
fn session_detach(state: State<'_, SessionState>, id: String, token: u64) -> Result<(), String> {
    let chans = {
        let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(&id) {
            Some(s) => s.chans.clone(),
            None => return Ok(()),
        }
    };
    chans
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(t, _)| *t != token);
    Ok(())
}

/// Ids of every currently-live session (across all windows). After a webview reload (F5 / WebView2
/// crash-recovery) the frontend has lost its channels but the backend sessions keep running and
/// holding SESSION_LIMIT slots — this lets the UI re-attach its panes instead of orphaning them.
#[tauri::command]
fn session_list(state: State<'_, SessionState>) -> Vec<String> {
    state
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect()
}

/// Open a source location in the user's editor (#13), triggered by clicking a `path:line` link in a
/// terminal. The path comes from (untrusted) terminal output, so it is treated as hostile: NEVER
/// shelled out through `cmd /c` with interpolation. Instead it is (1) canonicalized — which also
/// rejects any injection payload, since a path with shell metacharacters won't resolve to a real
/// file — (2) required to be a regular, non-executable file, then (3) handed to the editor / OS via
/// argv only (no shell string is ever reconstructed).
#[tauri::command]
fn open_in_editor(app: AppHandle, path: String, line: Option<u32>) -> Result<(), String> {
    // Resolve to a real on-disk path; a non-existent or metacharacter-laden string fails here.
    let canon =
        std::fs::canonicalize(&path).map_err(|_| "open_in_editor: no such file".to_string())?;
    if !canon.is_file() {
        return Err("open_in_editor: not a regular file".into());
    }
    // Never auto-open an executable/script — opening it could run it.
    const BLOCKED_EXT: &[&str] = &[
        "exe", "bat", "cmd", "com", "ps1", "psm1", "scr", "lnk", "vbs", "vbe", "js", "jse", "wsf",
        "wsh", "msi", "reg", "hta", "cpl", "jar", "msc", "pif",
    ];
    if let Some(ext) = canon.extension().and_then(|e| e.to_str()) {
        if BLOCKED_EXT.iter().any(|b| b.eq_ignore_ascii_case(ext)) {
            return Err("open_in_editor: blocked file type".into());
        }
    }
    let canon_str = canon.to_string_lossy().to_string();
    // Prefer VS Code's --goto (jumps to the line) via argv — no shell. `code` resolves on installs
    // that expose it; where it doesn't, spawn fails and we fall back.
    let target = match line {
        Some(l) => format!("{canon_str}:{l}"),
        None => canon_str.clone(),
    };
    if std::process::Command::new("code")
        .args(["--goto", &target])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    // Fallback: open the validated path in its default app via the opener plugin (ShellExecute on the
    // path as data — not a shell command line). Loses the line jump but stays safe.
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canon_str, None::<&str>)
        .map_err(|e| format!("open_in_editor: {e}"))
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

// ===================== SSH host registry (config\sshhosts.json + ~/.ssh/config import) =====================
// Saved hosts live in a synced JSON under SCRIPTS_ROOT (same pattern as myproviders.json); NO secrets
// are stored — auth uses the system `ssh` + the user's ~/.ssh (keys/known_hosts/ControlMaster). The
// `read_ssh_hosts` command also surfaces hosts parsed read-only from the machine's ~/.ssh/config
// (source="sshconfig") so existing SSH setup is reused (DRY). An ssh session is launched via the
// normal session_spawn with tool="ssh" and the target carried in `args` (e.g. "user@host -p 22").
const SSHHOSTS_CONFIG_REL: &str = "!Настройки и MCP\\ClaudeProfiles\\config\\sshhosts.json";
static SSHHOSTS_LOCK: Mutex<()> = Mutex::new(());

fn default_ssh_source() -> String {
    "saved".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SshHost {
    #[serde(default)]
    id: String,
    name: String,
    host: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_path: Option<String>,
    // Optional remote start directory: on connect we `Set-Location` into it (Windows/PowerShell remote).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_dir: Option<String>,
    #[serde(default = "default_ssh_source")]
    source: String, // "saved" | "sshconfig"
}

fn read_ssh_hosts_saved() -> Vec<SshHost> {
    std::fs::read_to_string(abs(SSHHOSTS_CONFIG_REL))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|v| v.get("hosts").and_then(|p| p.as_array()).cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|e| serde_json::from_value::<SshHost>(e).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn write_ssh_hosts_saved(list: &[SshHost]) -> Result<(), String> {
    let path = abs(SSHHOSTS_CONFIG_REL);
    let v = serde_json::json!({ "schemaVersion": 1, "hosts": list });
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_json_atomic(&path, &json).map_err(|e| format!("write sshhosts.json: {e}"))
}

/// Strip ONE pair of surrounding double/single quotes (OpenSSH allows quoted values, e.g. an
/// IdentityFile path with spaces). Leaves unquoted strings untouched.
fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Parse `~/.ssh/config` text into hosts (read-only). Honors Host (skips wildcard/negated patterns),
/// HostName, User, Port, IdentityFile. A `Host a b` line is ONE host (named after its first concrete
/// alias). Best-effort: unknown keywords ignored; tokens split on whitespace or '=' (OpenSSH accepts both).
/// `Include` directives are spliced in by read_ssh_config_hosts before this runs (this fn is pure text).
fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut out: Vec<SshHost> = Vec::new();
    let mut cur: Option<usize> = None; // out-index of the current Host block (None for wildcard-only blocks)
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.find(|c: char| c.is_whitespace() || c == '=') {
            Some(i) => (
                line[..i].trim(),
                line[i + 1..]
                    .trim_matches(|c: char| c.is_whitespace() || c == '=')
                    .trim(),
            ),
            None => (line, ""),
        };
        if key.eq_ignore_ascii_case("host") {
            // `Host a b c` lists alternative match patterns for ONE host — use the first concrete alias
            // as its name (extra aliases aren't separate machines; one-per-alias used to dupe them).
            cur = val
                .split_whitespace()
                .find(|a| !a.contains(['*', '?', '!']))
                .map(|alias| {
                    out.push(SshHost {
                        id: format!("cfg:{alias}"),
                        name: alias.to_string(),
                        host: alias.to_string(), // replaced by HostName if the block has one
                        port: None,
                        user: None,
                        key_path: None,
                        remote_dir: None,
                        source: "sshconfig".into(),
                    });
                    out.len() - 1
                });
        } else if let Some(i) = cur {
            match key.to_ascii_lowercase().as_str() {
                "hostname" => out[i].host = unquote(val).to_string(),
                "user" => out[i].user = Some(unquote(val).to_string()),
                "port" => out[i].port = unquote(val).parse::<u16>().ok(),
                "identityfile" => out[i].key_path = Some(unquote(val).to_string()),
                _ => {}
            }
        }
    }
    out
}

fn read_ssh_config_hosts() -> Vec<SshHost> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let main = format!("{home}\\.ssh\\config");
    let mut text = String::new();
    expand_ssh_config(std::path::Path::new(&main), &home, 0, &mut text);
    parse_ssh_config(&text)
}

/// Inline `Include` directives into one config blob (OpenSSH semantics: the included file's contents
/// are spliced in at that point), so hosts defined in included files (a common `~/.ssh/config.d/*`
/// layout) are no longer silently dropped. Bounded recursion guards against include cycles.
fn expand_ssh_config(path: &std::path::Path, home: &str, depth: u8, out: &mut String) {
    if depth > 16 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let t = line.trim();
        // Match the `Include` keyword followed by whitespace/'=' (not e.g. "IncludeFoo").
        let is_include = t.len() > 7
            && t[..7].eq_ignore_ascii_case("include")
            && t[7..].starts_with(|c: char| c.is_whitespace() || c == '=');
        if is_include {
            let patterns = t[7..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
            for pat in patterns.split_whitespace() {
                for f in resolve_ssh_include(unquote(pat), home) {
                    expand_ssh_config(&f, home, depth + 1, out);
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
}

/// Resolve one Include pattern to concrete file paths. Supports `~/`, absolute and ~/.ssh-relative
/// paths, plus a single trailing `*` (the common `config.d/*` = every file directly under that dir).
/// ponytail: no general globbing (no extra dep) — `dir/prefix*` style patterns resolve to nothing.
fn resolve_ssh_include(pat: &str, home: &str) -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let expanded = if let Some(rest) = pat.strip_prefix("~/") {
        format!("{home}\\{rest}")
    } else if std::path::Path::new(pat).is_absolute() {
        pat.to_string()
    } else {
        format!("{home}\\.ssh\\{pat}")
    };
    let expanded = expanded.replace('/', "\\");
    if let Some(prefix) = expanded.strip_suffix('*') {
        let dir = prefix.trim_end_matches('\\');
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        files
    } else {
        vec![PathBuf::from(expanded)]
    }
}

/// Saved hosts (synced registry) merged with read-only hosts imported from ~/.ssh/config, each added
/// only if its host isn't already listed (saved entries win; duplicate config blocks collapse).
#[tauri::command]
fn read_ssh_hosts() -> Vec<SshHost> {
    let mut all = read_ssh_hosts_saved();
    let mut seen: std::collections::HashSet<String> =
        all.iter().map(|h| h.host.to_ascii_lowercase()).collect();
    for h in read_ssh_config_hosts() {
        if seen.insert(h.host.to_ascii_lowercase()) {
            all.push(h);
        }
    }
    all
}

/// Create or update a saved host (matched by id); returns the new saved list. No secrets stored.
#[tauri::command]
fn save_ssh_host(host: SshHost) -> Result<Vec<SshHost>, String> {
    let _g = SSHHOSTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_ssh_hosts_saved();
    let mut h = host;
    if h.id.trim().is_empty() {
        h.id = gen_session_id();
    }
    h.source = "saved".into();
    match list.iter_mut().find(|x| x.id == h.id) {
        Some(existing) => *existing = h,
        None => list.push(h),
    }
    write_ssh_hosts_saved(&list)?;
    Ok(list)
}

/// Delete a saved host by id; returns the new saved list. (sshconfig-sourced hosts can't be deleted.)
#[tauri::command]
fn delete_ssh_host(id: String) -> Result<Vec<SshHost>, String> {
    let _g = SSHHOSTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_ssh_hosts_saved();
    list.retain(|x| x.id != id);
    write_ssh_hosts_saved(&list)?;
    Ok(list)
}

/// Quick reachability probe for the host editor: TCP connect to host:port (default 22), ~2s timeout.
/// Does NOT authenticate — just tells the user the host is reachable before they launch ssh.
#[tauri::command]
fn test_ssh_host(host: String, port: Option<u16>) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let p = port.unwrap_or(22);
    match format!("{host}:{p}").to_socket_addrs() {
        // Try EVERY resolved address (short-circuits on the first success). Probing only the first
        // wrongly reported IPv6-first hosts as unreachable when only their IPv4 endpoint was up.
        Ok(addrs) => addrs
            .into_iter()
            .any(|a| TcpStream::connect_timeout(&a, std::time::Duration::from_secs(2)).is_ok()),
        Err(_) => false,
    }
}

/// Base64 (standard, padded) of a string's UTF-16LE bytes — the exact form `powershell -EncodedCommand`
/// expects. Used to ship a `Set-Location` into an SSH session so quotes/spaces/Cyrillic survive the
/// local→ssh→remote re-parsing intact. Tiny hand-rolled encoder (no extra dependency).
fn ps_encoded_command(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        bytes.push((u & 0xff) as u8);
        bytes.push((u >> 8) as u8);
    }
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        out.push(T[(b[0] >> 2) as usize] as char);
        out.push(T[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if c.len() > 1 {
            T[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[(b[2] & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod ssh_config_tests {
    use super::*;
    #[test]
    fn parses_aliases_fields_and_skips_wildcards() {
        let cfg = "# my hosts\n\
Host minipc 192.168.1.177\n\
    HostName 192.168.1.177\n\
    User dansc\n\
    Port 22\n\
    IdentityFile ~/.ssh/id_ed25519\n\
\n\
Host *\n\
    ForwardAgent yes\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(
            hosts.len(),
            1,
            "multi-alias Host line = one host, wildcard skipped"
        );
        let mini = hosts
            .iter()
            .find(|h| h.name == "minipc")
            .expect("minipc host");
        assert_eq!(mini.host, "192.168.1.177");
        assert_eq!(mini.user.as_deref(), Some("dansc"));
        assert_eq!(mini.port, Some(22));
        assert_eq!(mini.key_path.as_deref(), Some("~/.ssh/id_ed25519"));
        assert!(hosts.iter().all(|h| h.source == "sshconfig"));
    }

    #[test]
    fn strips_surrounding_quotes_from_values() {
        let cfg =
            "Host q\n  HostName \"10.0.0.5\"\n  User 'bob'\n  IdentityFile \"C:/keys/my key\"\n";
        let hosts = parse_ssh_config(cfg);
        let h = hosts.iter().find(|h| h.name == "q").expect("host q");
        assert_eq!(h.host, "10.0.0.5", "double quotes stripped");
        assert_eq!(h.user.as_deref(), Some("bob"), "single quotes stripped");
        assert_eq!(
            h.key_path.as_deref(),
            Some("C:/keys/my key"),
            "quoted path with space kept intact"
        );
    }

    #[test]
    fn ring_buffer_drops_oldest_over_cap() {
        use std::collections::VecDeque;
        let mut rb: VecDeque<u8> = VecDeque::new();
        push_bounded(&mut rb, b"hello", 4);
        assert_eq!(rb.iter().copied().collect::<Vec<u8>>(), b"ello"); // oldest 'h' dropped
        push_bounded(&mut rb, b"XY", 4);
        assert_eq!(rb.iter().copied().collect::<Vec<u8>>(), b"loXY"); // capped at 4, newest kept
    }

    #[test]
    fn ps_encoded_command_matches_powershell() {
        // [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes(x)) reference values.
        assert_eq!(ps_encoded_command("A"), "QQA=");
        assert_eq!(ps_encoded_command("Hi"), "SABpAA==");
    }
}

// ===================== Multi-monitor windows (pop a live pane onto another monitor) =====================
// A pane can be "popped out" to its own frameless window on a chosen monitor. The window renders the
// SAME live session by attaching an extra output channel (session_attach) — no respawn. Monitors are
// enumerated and windows created/positioned from Rust (PhysicalPosition → correct across mixed DPI;
// no JS window perms needed). Child window labels (mon-* / pane-*) get core:default via the capability.
// A small handoff registry passes the pane's display spec to the new window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorInfo {
    index: usize,
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    primary: bool,
}

#[tauri::command]
fn list_monitors(app: AppHandle) -> Vec<MonitorInfo> {
    let prim_pos = app.primary_monitor().ok().flatten().map(|m| {
        let p = m.position();
        (p.x, p.y)
    });
    match app.available_monitors() {
        Ok(mons) => mons
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let p = m.position();
                let s = m.size();
                MonitorInfo {
                    index: i,
                    name: m
                        .name()
                        .cloned()
                        .unwrap_or_else(|| format!("Monitor {}", i + 1)),
                    x: p.x,
                    y: p.y,
                    width: s.width,
                    height: s.height,
                    scale: m.scale_factor(),
                    primary: prim_pos == Some((p.x, p.y)),
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

// Handoff: the main window stashes a pane's display spec under the new window's label; the child reads
// (and clears) it on mount. Lazy-init map (HashMap::new isn't const).
static DETACH_REGISTRY: Mutex<Option<std::collections::HashMap<String, serde_json::Value>>> =
    Mutex::new(None);

#[tauri::command]
fn prepare_detach(label: String, spec: serde_json::Value) {
    let mut g = DETACH_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(std::collections::HashMap::new)
        .insert(label, spec);
}

#[tauri::command]
fn take_detach(label: String) -> Option<serde_json::Value> {
    let mut g = DETACH_REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    g.as_mut().and_then(|m| m.remove(&label))
}

/// Open (or focus) a frameless window filling the given monitor. Positioned/sized in PHYSICAL pixels
/// so it lands correctly across mixed-DPI monitors. The window loads the app; its label drives the
/// detached view on the frontend.
#[tauri::command]
fn open_monitor_window(app: AppHandle, label: String, monitor_index: usize) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        return Ok(());
    }
    let mons = app.available_monitors().map_err(|e| e.to_string())?;
    let m = mons
        .get(monitor_index)
        .ok_or_else(|| "monitor index out of range".to_string())?;
    let pos = *m.position();
    let size = *m.size();
    // Build OFF the main thread. The command runs on the main (event-loop) thread, and a synchronous
    // `WebviewWindowBuilder::build()` there DEADLOCKS: WebView2 creation is async and needs the event
    // loop to pump, but build() blocks that very loop. From a worker thread, build() dispatches the
    // creation to the (now free) main loop and returns once the webview is ready.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let built = tauri::WebviewWindowBuilder::new(
            &app2,
            &label,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Castellyn")
        .decorations(false)
        // Dark background so the frame never flashes white while the webview boots.
        .background_color(tauri::webview::Color(8, 12, 24, 255))
        .build();
        // Physical position/size — correct across mixed-DPI monitors (the window-state plugin is
        // denylisted for mon-* so it can't override these with a stale restored rect).
        match built {
            Ok(win) => {
                let _ = win.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                let _ = win.set_size(tauri::PhysicalSize::new(size.width, size.height));
                let _ = win.set_focus();
            }
            Err(e) => {
                // Build failed — don't fail silently. The frontend stashed the pane spec under this
                // label (prepare_detach) before calling us; clear it so it can't leak, and tell the UI
                // so it can re-home the pane / toast instead of "losing" the detached session.
                let _ = take_detach(label.clone());
                let _ = app2.emit(
                    "monitor-window-failed",
                    serde_json::json!({ "label": label, "error": e.to_string() }),
                );
            }
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Single-instance MUST be the FIRST plugin: a second launch hands its argv to this running
        // instance's callback (which just reveals the window) and then exits, instead of opening a
        // duplicate. Reveal covers the start-hidden/tray case — a re-launch is the user asking to see
        // the app, so show + unminimize + focus the existing "main" window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            reveal(app);
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Remember window position/size across launches (auto-restores on start, saves on exit).
        // Denylist the ephemeral monitor windows: they're positioned explicitly per-monitor on every
        // open, so restoring a saved rect would misplace/shrink them (pane-<id> labels are unique per
        // session, so they never collide on restore).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[
                    "mon-0", "mon-1", "mon-2", "mon-3", "mon-4", "mon-5", "mon-6", "mon-7",
                ])
                .build(),
        )
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
            reveal_backup,
            delete_backup,
            verify_backup,
            extract_backup,
            run_backup,
            read_profiles,
            run_profiles,
            repair_all_profiles,
            read_profiles_config,
            run_profile_mgmt,
            read_orphan_profiles,
            delete_orphan_profile,
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
            read_drift_diff,
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
            session_attach,
            session_detach,
            session_list,
            open_in_editor,
            list_subdirs,
            read_ssh_hosts,
            save_ssh_host,
            delete_ssh_host,
            test_ssh_host,
            list_monitors,
            prepare_detach,
            take_detach,
            open_monitor_window,
            read_freellmapi_analytics,
            read_providers,
            run_provider,
            list_my_providers,
            save_my_provider,
            delete_my_provider,
            set_freellmapi_auth,
            delete_freellmapi_auth,
            freellmapi_auth_status,
            connect_my_provider,
            check_my_provider,
            check_provider_url,
            check_provider_balance,
            read_profile_file,
            read_profile_usage,
            read_sessions_prefs,
            write_sessions_prefs,
            add_provider_key,
            remove_provider_key,
            next_provider_key,
            read_opencode,
            run_opencode_provider,
            read_mcp,
            run_mcp,
            mcp_upsert_server,
            mcp_remove_server,
            mcp_remove_extra,
            list_plugins,
            list_skills,
            read_environments,
            read_skill_matrix,
            share_skills,
            run_opencode_rtk,
            run_opencode_mcp,
            run_opencode_providers,
            run_opencode_instructions,
            run_codex_mcp,
            run_codex_providers,
            list_plugin_updates,
            list_plugin_contents,
            list_plugin_releases,
            run_plugin,
            run_plugins_bulk,
            plugin_sync_status,
            plugin_sync_set,
            run_plugin_sync,
            agent_status_hook_status,
            agent_status_hook_set,
            delete_skill,
            read_schedules,
            run_schedule,
            read_config,
            write_config,
            export_config,
            import_config,
            app_paths,
            gateway_base_url,
            canonical_skills_dir,
            global_session_count,
            quit_app,
            open_path,
            clone_repo,
            open_url,
            open_terminal,
            get_autostart,
            set_autostart,
            set_toggle_hotkey,
            read_shortcuts,
            set_shortcuts,
            set_language,
            cancel_run,
            cancel_all
        ])
        .setup(|app| {
            // Warm the elevation check off the main/UI thread: is_elevated() shells out to pwsh
            // (~100-300ms cold) and the first read_profiles would otherwise pay it on a user-facing
            // sync command. Its OnceLock makes this a one-time cost that the background thread absorbs.
            std::thread::spawn(|| {
                let _ = is_elevated();
            });
            // Seed the backend locale from config so the tray builds in the right language. The
            // frontend also re-syncs on mount (covers a fresh config with no language yet).
            if let Some(lang) = read_config_file().language {
                set_cur_lang(Lang::parse(&lang));
            }
            build_tray(app.handle())?;
            // Agent-status engine for Sessions panes (hook files + PTY activity → events).
            agent_status::start(app.handle().clone());
            // Anthropic OAuth usage-limit monitor (per profile; 85%/99% alerts). No-op for profiles
            // without OAuth creds; disableable via the `limitsMonitor` config toggle.
            limits::start(app.handle().clone());
            // One-time brand-rename migration of the autostart Run entry (AgentHub → Castellyn).
            migrate_autostart();
            let cfg = read_config_file();
            // Start minimized to tray if configured.
            if cfg.start_hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            // Register all configured shortcuts. A bad/taken combo must not block startup.
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            if let Some(shortcuts) = cfg.shortcuts.as_ref() {
                let count: usize = shortcuts
                    .values()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|accel| {
                        if let Err(e) = register_shortcut(app.handle(), accel) {
                            eprintln!("shortcut register failed ({accel}): {e}");
                            0
                        } else {
                            1
                        }
                    })
                    .sum();
                if count > 1 {
                    // If >1 registered, the probe-registers in the loop left them all active but the
                    // first probe's unregister_all + re-register would orphan the others. The probe-only
                    // loop above already registered them; we unregister_all and re-register cleanly.
                    let _ = app.global_shortcut().unregister_all();
                    for accel in shortcuts
                        .values()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                    {
                        let _ = register_shortcut(app.handle(), accel);
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button minimizes to tray instead of quitting.
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Only the MAIN window persists geometry + minimizes to tray. Detached monitor /
                // popped-out pane windows (mon-* / pane-*) are EPHEMERAL: persisting their geometry
                // made the window-state plugin restore stale/default rects on the next distribute
                // (tiny, misplaced, blank window). They also must close for real so their panes
                // unmount — otherwise the PTY session, its fan-out channel and the WebView2 leak.
                // (✕ opt-out via closeToTray=false still quits the app.)
                if window.label() == "main" {
                    use tauri_plugin_window_state::{AppHandleExt, StateFlags};
                    let _ = window.app_handle().save_window_state(StateFlags::all());
                    if read_config_file().close_to_tray.unwrap_or(true) {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            // On exit, kill every live PTY session so no headless child (claude/opencode/pwsh/ssh)
            // outlives the app. PtySession has no Drop and the tray "quit" is a hard app.exit(0), so
            // without this every parallel session keeps running invisibly after the window closes.
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<SessionState>() {
                    for (_, mut s) in state.0.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                        let _ = s.killer.kill();
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_slot_reserves_releases_and_survives_panic() {
        let state = RunState::default();
        let slot = RunSlot::reserve(&state).expect("first reserve succeeds");
        assert!(
            RunSlot::reserve(&state).is_err(),
            "a second reserve while held must fail"
        );
        drop(slot);
        assert!(RunSlot::reserve(&state).is_ok(), "the slot frees on drop");

        // A panic on the run path must NOT wedge the slot (the whole point of the RAII guard).
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _slot = RunSlot::reserve(&state).expect("reserve before panic");
            panic!("boom");
        }));
        std::panic::set_hook(prev);
        assert!(
            RunSlot::reserve(&state).is_ok(),
            "the slot frees after a panic, not wedged busy"
        );
    }

    #[test]
    fn read_config_at_bom_missing_and_corrupt() {
        use std::io::Write;
        let pid = std::process::id();
        let dir = std::env::temp_dir();
        // Missing file → None.
        let missing = dir.join(format!("castellyn_none_{pid}.json"));
        assert!(read_config_at(Some(missing.display().to_string())).is_none());
        // A UTF-8 BOM-prefixed config still parses — the migration fallback chain depends on it.
        let ok = dir.join(format!("castellyn_ok_{pid}.json"));
        std::fs::File::create(&ok)
            .unwrap()
            .write_all("\u{feff}{\"scriptsRoot\":\"X:\\\\S\"}".as_bytes())
            .unwrap();
        let cfg = read_config_at(Some(ok.display().to_string())).expect("BOM config parses");
        assert_eq!(cfg.scripts_root.as_deref(), Some("X:\\S"));
        let _ = std::fs::remove_file(&ok);
        // Corrupt JSON → None (a malformed primary falls through the chain, never panics).
        let bad = dir.join(format!("castellyn_bad_{pid}.json"));
        std::fs::write(&bad, "{ not json").unwrap();
        assert!(read_config_at(Some(bad.display().to_string())).is_none());
        let _ = std::fs::remove_file(&bad);
    }

    #[test]
    fn read_json_or_recover_falls_back_to_bak() {
        let pid = std::process::id();
        let dir = std::env::temp_dir();
        // Missing → Ok(None).
        let missing = dir.join(format!("castellyn_rec_none_{pid}.json"));
        assert!(read_json_or_recover(&missing, "t").unwrap().is_none());
        // Present + valid → Ok(Some), no .bak needed.
        let good = dir.join(format!("castellyn_rec_ok_{pid}.json"));
        std::fs::write(&good, "{\"a\":1}").unwrap();
        assert_eq!(read_json_or_recover(&good, "t").unwrap().unwrap()["a"], 1);
        // Corrupt main + valid .bak → recovers from .bak silently.
        let corrupt = dir.join(format!("castellyn_rec_bad_{pid}.json"));
        std::fs::write(&corrupt, "{ not json").unwrap();
        std::fs::write(format!("{}.bak", corrupt.display()), "{\"a\":2}").unwrap();
        assert_eq!(
            read_json_or_recover(&corrupt, "t").unwrap().unwrap()["a"],
            2
        );
        // Corrupt main + no usable .bak → Err (caller must abort, not overwrite).
        let _ = std::fs::remove_file(format!("{}.bak", corrupt.display()));
        assert!(read_json_or_recover(&corrupt, "t").is_err());
        let _ = std::fs::remove_file(&good);
        let _ = std::fs::remove_file(&corrupt);
    }

    #[test]
    fn embedded_manifest_fallback_is_valid() {
        // raw_components falls back to MANIFEST_FALLBACK on a corrupt on-disk manifest — that embedded
        // copy must itself parse and list components, else the fallback can't save a blank dashboard.
        let m: RawManifest =
            serde_json::from_str(MANIFEST_FALLBACK).expect("fallback is valid JSON");
        assert!(!m.components.is_empty(), "fallback must list components");
    }

    #[test]
    fn snapshot_name_format() {
        assert!(is_snapshot_name("2026-06-12_100002"));
        assert!(!is_snapshot_name("weekly-2026-06-11.zip"));
        assert!(!is_snapshot_name("2026-6-12_100002"));
        assert!(!is_snapshot_name(".backup-state.json"));
        assert!(!is_snapshot_name("2026-06-12_10000")); // too short
    }

    #[test]
    fn backup_args_security_gating() {
        // A plain backup: -Force, KeepSnapshots floored at 1 (never 0 → "keep none").
        let (s, a) = backup_args("backup", None, None, None, Some(0)).unwrap();
        assert_eq!(s, BACKUP_SCRIPT_REL);
        assert!(a.contains(&"-Force".to_string()));
        let kidx = a.iter().position(|x| x == "-KeepSnapshots").unwrap();
        assert_eq!(a[kidx + 1], "1");

        // A preview is always -WhatIf and NEVER carries credentials, even if asked.
        let (s, a) = backup_args(
            "restore-preview",
            Some("2026-06-12_100002".into()),
            None,
            Some(true),
            None,
        )
        .unwrap();
        assert_eq!(s, RESTORE_SCRIPT_REL);
        assert!(a.contains(&"-WhatIf".to_string()));
        assert!(!a.contains(&"-IncludeCredentials".to_string()));

        // A real restore WITHOUT the explicit flag must not include credentials.
        let (_, a) = backup_args("restore", None, None, None, None).unwrap();
        assert!(!a.contains(&"-IncludeCredentials".to_string()));
        assert!(!a.contains(&"-WhatIf".to_string()));

        // Only a real restore WITH the explicit flag carries credentials + scoping.
        let (_, a) = backup_args(
            "restore",
            Some("2026-06-12_100002".into()),
            Some(vec!["work".into()]),
            Some(true),
            None,
        )
        .unwrap();
        assert!(a.contains(&"-IncludeCredentials".to_string()));
        assert!(a.contains(&"-Timestamp".to_string()));
        assert!(a.contains(&"work".to_string()));

        // Unknown action errors — never silently picks a script.
        assert!(backup_args("rm-rf", None, None, Some(true), None).is_err());
    }

    #[test]
    fn write_json_atomic_roundtrip_and_backup() {
        // Data-integrity: the atomic writer creates the file, overwrites it on a second write,
        // and leaves a .bak of the prior good copy (the rename is the same-dir atomic swap).
        let dir = std::env::temp_dir().join(format!("castellyn_wja_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("config.json");
        let p = path.to_string_lossy().to_string();

        write_json_atomic(&p, "{\"v\":1}").unwrap(); // creates parent dirs + file
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"v\":1}");
        assert!(
            !path.with_extension("json.bak").exists()
                && !std::path::Path::new(&format!("{p}.bak")).exists()
        );

        write_json_atomic(&p, "{\"v\":2}").unwrap(); // overwrite → prior copy backed up
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"v\":2}");
        assert_eq!(
            std::fs::read_to_string(format!("{p}.bak")).unwrap(),
            "{\"v\":1}"
        );
        assert!(!std::path::Path::new(&format!("{p}.tmp")).exists()); // no stray temp left

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_json_atomic_skips_bak_for_secret_files() {
        // Security: a secret-bearing file (profile settings.json / opencode.json) must NEVER get an
        // in-place .bak that would strand a prior cleartext token; the atomic write still updates it.
        let dir = std::env::temp_dir().join(format!("castellyn_wja_secret_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["settings.json", "opencode.json"] {
            let p = dir.join(name).to_string_lossy().to_string();
            write_json_atomic(&p, "{\"token\":\"old\"}").unwrap();
            write_json_atomic(&p, "{\"token\":\"new\"}").unwrap(); // overwrite — prior secret must not survive
            assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"token\":\"new\"}");
            assert!(
                !std::path::Path::new(&format!("{p}.bak")).exists(),
                "{name} left a secret .bak"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forks_args_known_and_unknown() {
        assert_eq!(
            forks_action_args("check"),
            Some(vec!["-Unattended".to_string()])
        );
        let ff = forks_action_args("ff").unwrap();
        assert!(ff.contains(&"-FfMain".to_string()));
        assert!(ff.contains(&"-Yes".to_string())); // mutations must be unattended
        let wip = forks_action_args("sync-wip").unwrap();
        assert!(wip.contains(&"-SyncWipLocal".to_string()));
        assert!(wip.contains(&"-Yes".to_string()));
        let delwip = forks_action_args("delete-wip").unwrap();
        assert!(delwip.contains(&"-DeleteWip".to_string()));
        assert!(delwip.contains(&"-Yes".to_string()));
        let prune = forks_action_args("prune").unwrap();
        assert!(prune.contains(&"-Prune".to_string()));
        assert!(prune.contains(&"-Yes".to_string()));
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
        for a in [
            "check",
            "plan",
            "ff",
            "delete",
            "rebase",
            "sync-wip",
            "delete-wip",
            "prune",
            "normalize",
        ] {
            let args = forks_action_args(a).unwrap();
            assert!(
                args.contains(&"-Unattended".to_string()),
                "{a} must be unattended"
            );
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
            &p,
            "set",
            "tgt",
            Some("New"),
            "http://new",
            Some("m1"),
            None,
            Some("MY_KEY"),
            false,
            &noop,
            &noop,
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
            &p,
            "cc1",
            "set",
            "http://localhost:4000",
            false,
            Some("sk-x"),
            Some("glm-4.7"),
            Some(""),
            &noop,
            &noop,
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
            &p,
            "cc1",
            "set",
            "http://localhost:4000",
            false,
            Some(""),
            None,
            None,
            &noop,
            &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "agenthub-local");
        assert_eq!(v["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "glm-4.7"); // model None → untouched

        // clear → all provider keys gone; the empty env block is dropped; unrelated setting kept.
        let code = apply_provider_env(
            &p, "cc1", "clear", "", false, None, None, None, &noop, &noop,
        );
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
        let code = apply_router_config(
            &p,
            "http://localhost:1234/v1",
            "qwen",
            "lmstudio",
            &noop,
            &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["APIKEY"], "keep-me"); // unrelated top-level key preserved
        let provs = v["Providers"].as_array().unwrap();
        assert_eq!(provs.len(), 2); // upsert, not append
        let other = provs.iter().find(|x| x["name"] == "other").unwrap();
        assert_eq!(other["api_base_url"], "http://other/v1/chat/completions"); // untouched
        let lm = provs.iter().find(|x| x["name"] == "lmstudio").unwrap();
        assert_eq!(
            lm["api_base_url"],
            "http://localhost:1234/v1/chat/completions"
        ); // normalized
        assert_eq!(lm["api_key"], "not-needed");
        assert_eq!(lm["models"][0], "qwen");
        assert_eq!(v["Router"]["default"], "lmstudio,qwen");

        // A backend already ending in /chat/completions must not be doubled.
        let code = apply_router_config(
            &p,
            "http://h/v1/chat/completions",
            "m",
            "lmstudio",
            &noop,
            &noop,
        );
        assert_eq!(code, 0);
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let lm = v["Providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "lmstudio")
            .unwrap();
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
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
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
        assert!(
            out.contains("agenthub-pty-probe"),
            "pty output was: {out:?}"
        );
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
        assert_eq!(
            key_pool_meta(&serde_json::json!({ "keyCount": 3, "activeKey": 2 })),
            (3, 2)
        );
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
        mk(
            &root.join("skills\\max-dedup\\SKILL.md"),
            "---\nname: max-dedup\n---\nx",
        );
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
