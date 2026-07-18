// Castellyn — Tauri backend.
//   * Component manifest (embedded) → run a component's PowerShell script in -Check or -Apply
//     mode, streaming stdout/stderr to the UI.
//   * Single-run guard + cancel.
//   * System tray with minimize-to-tray.
// Paths resolve from $SCRIPTS_ROOT (fallback E:\Scripts) so the app survives a disk move.

// Edition 2024 stabilized let-chains, so clippy's `collapsible_if` now flags every `if let X { if Y }`
// nested guard (~78 across this backend). Collapsing them all into let-chains is a readability wash and
// pure churn; keep the nesting readable and allow the lint crate-wide (a common 2024-migration choice).
#![allow(clippy::collapsible_if)]

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
mod stack_health;
mod schedules_watch;
mod session_bus;
mod worktree;
mod i18n;
use i18n::{tr, trv, Lang};

/// Windows CREATE_NO_WINDOW — keep spawned console apps (pwsh/reg/taskkill) from flashing
/// a black console window in front of the GUI.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Adoption guard for the CREATE_NO_WINDOW canon above (CLAUDE.md: "All process spawns set
/// CREATE_NO_WINDOW"). A spawn that forgets to set a window-creation flag pops a black console in
/// front of the GUI, and no compile/clippy gate catches it — only convention did, until now. This
/// scans our own source: every spawn constructor must set a flags call before the next spawn (or
/// within a generous window). The one deliberate visible console (CREATE_NEW_CONSOLE) also routes
/// through the same flags call, so it passes without a special case. The two needles are built with
/// `concat!` so this guard's own body is never mistaken for a spawn site.
#[cfg(test)]
mod spawn_window_guard {
    /// First byte offset of `needle` in `hay` at or after `from` (byte search — never panics on a
    /// UTF-8 boundary, unlike slicing &str across a Cyrillic comment).
    fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        if from >= hay.len() {
            return None;
        }
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    }

    #[test]
    fn every_spawn_sets_a_window_flag() {
        let src = include_str!("lib.rs").as_bytes();
        let ctor = concat!("Command", "::", "new(").as_bytes();
        let flag = concat!("creation", "_flags(").as_bytes();
        let mut spawns = 0usize;
        let mut naked: Vec<usize> = Vec::new();
        let mut i = 0usize;
        while let Some(at) = find(src, ctor, i) {
            spawns += 1;
            let after = at + ctor.len();
            // This spawn's region ends at the next spawn, capped so a missing flag can't be
            // "rescued" by a later spawn's flag far below.
            let next = find(src, ctor, after).unwrap_or(src.len());
            let end = next.min(after + 2500).min(src.len());
            if find(&src[after..end], flag, 0).is_none() {
                naked.push(src[..at].iter().filter(|&&b| b == b'\n').count() + 1);
            }
            i = after;
        }
        // Non-vacuous: if the scan collapses to far fewer sites than exist, the pattern broke and
        // every assertion below would pass for the wrong reason.
        assert!(
            spawns >= 40,
            "scanned only {spawns} spawn sites — the source pattern must have changed"
        );
        assert!(
            naked.is_empty(),
            "spawn(s) with no window-creation flag (a console window will flash) at lib.rs line(s) {naked:?}"
        );
    }
}

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
    /// Absolute path to the settings/MCP tree (the folder holding ClaudeProfiles + ClaudeMarketplace).
    /// None = auto-detect (see `settings_tree_root`). De-hardcodes the old literal so a rename /
    /// de-Cyrillicization (owner flipped `!Настройки и MCP` ↔ `SettingsMCP` 2026-07-06) needs no code change.
    #[serde(
        rename = "settingsDir",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    settings_dir: Option<String>,
    /// Manage the LLM stack NATIVELY (Castellyn spawns/tracks/stops each service itself, hidden,
    /// PID-tracked) — now the DEFAULT (None/true). Set false to opt back into the legacy PS launcher
    /// scripts (`start-stack.ps1`/`stop-stack.ps1`), kept as a fallback + for external callers.
    #[serde(
        rename = "stackNative",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    stack_native: Option<bool>,
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
    // Background llm-stack liveness poll (Home/System Health card): None = default (true). Polls
    // every 30s and flags services that transition to down. Set false to stop the background poll.
    #[serde(
        rename = "stackHealthMonitor",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    stack_health_monitor: Option<bool>,
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
    // #8: which option to auto-pick on the large-session RESUME menu ("1. Resume from summary /
    // 2. Resume full session as-is"). "summary" (default, option 1), "full" (option 2), or "ask"
    // (never auto-press — leave the resume menu to the user). Frontend-only loop; backend persists.
    #[serde(
        rename = "resumeChoice",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    resume_choice: Option<String>,
    // #9: custom continuation text injected after a limit reset / menu dismissal. None or empty = the
    // localized default ("continue" / "продолжай"). Frontend-only; backend persists.
    #[serde(
        rename = "autoContinueText",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    auto_continue_text: Option<String>,
    // Wave C-5: show the native session-status strip (live session counts + peak Anthropic
    // utilization) in the window title bar. None = default (true). Purely a frontend read — the
    // backend only persists it (write_config preserves it like the other Sessions toggles).
    #[serde(
        rename = "showSessionStatusBar",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    show_session_status_bar: Option<bool>,
    // U3: check for a Castellyn update once at startup (badge only, never auto-installs). None =
    // default (true). Purely a frontend read — the backend only persists it.
    #[serde(
        rename = "updateCheckOnStart",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    update_check_on_start: Option<bool>,
    // Gap-2: MCP server names Castellyn has deployed to each harness — the reconcile ledger (see
    // ManagedMcp). Written by the OpenCode/Codex MCP fan-out; not user-facing.
    #[serde(
        rename = "managedMcp",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    managed_mcp: Option<ManagedMcp>,
    /// R7: optimistic-concurrency version, bumped on every write. A save carrying a stale `rev` is
    /// rejected ("config-conflict") so two concurrent read-modify-writes can't drop each other's
    /// fields. `default` = 0 for a config written before versioning existed.
    #[serde(default)]
    rev: u64,
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

/// The settings/MCP tree folder name is NOT hardcoded: it was `!Настройки и MCP` (Cyrillic) and the
/// owner de-Cyrillicized it to `SettingsMCP` (2026-07-06), keeping the old name as a reverse junction.
/// Detection order below (env → config → first existing dir → ASCII default) makes the app follow the
/// rename on any machine without a code change, and is direction-agnostic (junction either way resolves).
const SETTINGS_TREE_CANDIDATES: [&str; 2] = ["SettingsMCP", "!Настройки и MCP"];

/// Pure core of `settings_tree_root` (testable): env override → explicit config value → first
/// candidate that exists under `sr` → ASCII default (`<sr>\SettingsMCP`). Returns an ABSOLUTE path.
fn resolve_settings_tree(
    env: Option<String>,
    cfg: Option<String>,
    sr: &str,
    exists: impl Fn(&str) -> bool,
) -> String {
    if let Some(v) = env.filter(|v| !v.trim().is_empty()) {
        return v;
    }
    if let Some(v) = cfg.filter(|v| !v.trim().is_empty()) {
        return v;
    }
    for cand in SETTINGS_TREE_CANDIDATES {
        let p = format!("{sr}\\{cand}");
        if exists(&p) {
            return p;
        }
    }
    format!("{sr}\\{}", SETTINGS_TREE_CANDIDATES[0])
}

/// Absolute path to the settings/MCP tree: `$CASTELLYN_SETTINGS_DIR` → config.settingsDir →
/// detect (`<scripts_root>\SettingsMCP`, then legacy `<scripts_root>\!Настройки и MCP`) → ASCII default.
fn settings_tree_root() -> String {
    resolve_settings_tree(
        std::env::var("CASTELLYN_SETTINGS_DIR").ok(),
        read_config_file().settings_dir,
        &scripts_root(),
        // A candidate qualifies only if it actually HOLDS ClaudeProfiles — so an empty leftover
        // `SettingsMCP` can't mask a real Cyrillic tree (and vice-versa). is_dir() follows junctions,
        // so both the real folder and a reverse-junction to it resolve the same.
        |p| std::path::Path::new(&format!("{p}\\ClaudeProfiles")).is_dir(),
    )
}

/// Absolute path to the ClaudeProfiles config tree (the source of truth every maintenance script and
/// config reader lives under). De-hardcoded via `settings_tree_root` — no Cyrillic literal in code.
fn profiles_root() -> String {
    format!("{}\\ClaudeProfiles", settings_tree_root())
}

/// Expand manifest path placeholders the same way the PowerShell executors do, so paths surfaced
/// to the UI match what actually runs. `{{SCRIPTS_ROOT}}` → scripts_root(), `{{USERPROFILE}}` → home,
/// `{{SETTINGS}}` → settings_tree_root(), `{{PROFILES}}` → profiles_root() (both ABSOLUTE, so `abs`
/// uses them as-is instead of re-prefixing scripts_root).
fn expand_placeholders(s: &str) -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    s.replace("{{SCRIPTS_ROOT}}", &scripts_root())
        .replace("{{USERPROFILE}}", &home)
        .replace("{{PROFILES}}", &profiles_root())
        .replace("{{SETTINGS}}", &settings_tree_root())
}

/// Read the canonical manifest from disk; fall back to the embedded copy if the
/// file is missing or unreadable (e.g. relocated exe without the repo).
/// The on-disk maintenance manifest path under SCRIPTS_ROOT — one source for manifest_text +
/// scripts_available, so a relocation/rename is a single edit (was built identically in both).
fn manifest_path() -> String {
    format!(
        "{}\\Castellyn\\manifest\\maintenance-manifest.json",
        scripts_root()
    )
}

fn manifest_text() -> String {
    std::fs::read_to_string(manifest_path()).unwrap_or_else(|_| MANIFEST_FALLBACK.to_string())
}

/// True when the on-disk maintenance manifest exists — i.e. the owner's SCRIPTS_ROOT tooling is
/// present. A fresh OSS user without it falls back to MANIFEST_FALLBACK; the UI uses this to explain
/// that the script-backed tabs are the owner-tooling part rather than flashing empty/erroring tabs.
#[tauri::command]
fn scripts_available() -> bool {
    std::path::Path::new(&manifest_path()).exists()
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
    // Token-aware: a rel carrying a placeholder (e.g. `{{PROFILES}}\…` from the de-hardcoded
    // constants/manifest) expands to an ABSOLUTE path and is used as-is; a plain relative path is
    // still joined under scripts_root. The `{{` guard keeps the common (tokenless) case cheap.
    if rel.contains("{{") {
        let e = expand_placeholders(rel);
        if std::path::Path::new(&e).is_absolute() {
            return e;
        }
        return format!("{}\\{}", scripts_root(), e);
    }
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

/// Read and parse a *.last.json status file. Returns null if it doesn't exist yet. A present but
/// unparseable file (after the .bak recovery attempt) errors with the frozen "corrupt: " prefix so
/// the UI can show "статус повреждён" instead of the misleading "нет данных" (wargaming A2 MED-3).
#[tauri::command]
fn read_status(path: String) -> Result<Option<serde_json::Value>, String> {
    read_json_or_recover(&path, &path).map_err(|e| {
        if std::path::Path::new(&path).exists() {
            format!("corrupt: {e}")
        } else {
            e
        }
    })
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

// LLM-stack domain: start/stop/restart run in their OWN slot, independent of the global RunState — a
// stack op neither blocks nor is blocked by maintenance/config/backup runs (those serialize on the
// shared ~/.claude; the stack manages external processes, not config). Its own Mutex<Option<u32>>
// holds the live stack-child pid so a Stop can PREEMPT (kill) an in-flight start — teardown is a
// recovery action that must never be blocked (the single-slot design rejected Stop with
// err.run_in_progress for the whole ~25s×service startup window).
#[derive(Default)]
struct StackRun(Mutex<Option<u32>>);

struct StackSlot<'a>(&'a StackRun);
impl<'a> StackSlot<'a> {
    /// Claim the stack slot; Err if a stack op is already running (start/restart must not overlap).
    fn reserve(s: &'a StackRun) -> Result<Self, String> {
        let mut g = s.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return Err(tr("err.run_in_progress", cur_lang()).into());
        }
        *g = Some(0);
        Ok(StackSlot(s))
    }
    /// Claim the slot for a STOP, preempting any in-flight start (kill its tree first). Stop always
    /// proceeds; stop-stack then tears down by port, so aborting a half-done start is safe.
    fn reserve_preempt(s: &'a StackRun) -> Self {
        // Take the in-flight pid and claim the slot UNDER the lock, then drop the guard BEFORE the
        // blocking kill_tree (~600ms grace) — holding the StackRun mutex across the kill would block
        // every other stack op, and this runs from an async command.
        let victim = {
            let mut g = s.0.lock().unwrap_or_else(|e| e.into_inner());
            let victim = g.take().filter(|&pid| pid != 0);
            *g = Some(0);
            victim
        };
        if let Some(pid) = victim {
            let _ = kill_tree(pid);
        }
        StackSlot(s)
    }
    fn set_pid(&self, pid: u32) {
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
    }
}
impl Drop for StackSlot<'_> {
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

/// Canonical stream-component ids emitted as the `component` field of `run-done` for the native /
/// script streaming paths (NOT the manifest component ids, which flow through generically). These
/// are a cross-boundary contract: the frontend's `run-done` handler keys reloads on the same
/// strings. Named here so a rename is one edit per side, and kept in lock-step with the TS mirror
/// `STREAM_IDS` in `src/lib/ipc.ts` — enforced by the parity test in `src/lib/ipc.test.ts`.
/// If you add/rename one, update all three: this module, `STREAM_IDS`, and that test's expectation.
mod stream_id {
    pub const FORKS: &str = "forks";
    pub const BACKUP: &str = "backup";
    pub const PROFILES: &str = "profiles";
    pub const SYNC: &str = "sync";
    pub const ENGINE: &str = "engine";
    pub const PROVIDER: &str = "provider";
    pub const SCHEDULE: &str = "schedule";
    pub const MCP: &str = "mcp";
    pub const PLUGIN_MGR: &str = "plugin-mgr";
    pub const PLUGIN_SYNC: &str = "pluginsync";
    pub const ONBOARDING: &str = "onboarding";
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

/// Localized "failed to launch {program}" text, plus an install hint when the missing program is
/// pwsh — a fresh Windows without PowerShell 7 is the common first-run block, so point the user at
/// the installer instead of a bare OS error.
fn spawn_err_text(program: &str, e: &str) -> String {
    let lang = cur_lang();
    let mut msg = trv("err.spawn_failed", lang, &[("program", &program), ("e", &e)]);
    if program == "pwsh" {
        msg.push_str(tr("err.pwsh_missing", lang));
    }
    msg
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
            return Err(spawn_err_text(&program, &e.to_string()));
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

    let component = id.clone();
    let app_n = app.clone();
    let code = pump_and_wait(app, id, child, "run-log", "run-done").await;
    drop(slot); // release the single run slot (also happens on any early return / panic above)
    // A failed maintenance run (RunState domain — NOT the stack phases, which have their own
    // suppression) is worth a toast when the user isn't already looking at the app.
    if code != 0
        && !app_n
            .webview_windows()
            .values()
            .any(|w| w.is_focused().unwrap_or(false))
    {
        notify_important(
            &app_n,
            tr("notify.run_failed_title", cur_lang()),
            &trv("notify.run_failed_body", cur_lang(), &[("component", &component)]),
        );
    }
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
            let _ = app.emit(
                "run-log",
                LogLine {
                    component: id.to_string(),
                    stream: "err".into(),
                    line: spawn_err_text("pwsh", &e.to_string()),
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

/// Stack-domain equivalent of spawn_pwsh_phase: run one pwsh script under an ALREADY-reserved
/// StackSlot, streaming to "run-log"/`done_event`, recording the child pid on the slot so cancel_all
/// and Stop's preempt can kill whichever phase is live. Kept separate from spawn_pwsh_phase so the
/// stack's concurrency domain never touches the shared RunState hot path.
async fn spawn_stack_phase(
    app: &AppHandle,
    slot: &StackSlot<'_>,
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
            let _ = app.emit(
                "run-log",
                LogLine {
                    component: id.to_string(),
                    stream: "err".into(),
                    line: spawn_err_text("pwsh", &e.to_string()),
                },
            );
            return -1;
        }
    };
    if let Some(pid) = child.id() {
        slot.set_pid(pid);
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
            BufReader::new(stdout),
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        handles.push(tokio::spawn(pump_stream(
            app.clone(),
            id.clone(),
            log_event,
            "err",
            BufReader::new(stderr),
        )));
    }
    // Backstop (V-14): a wedged script (infinite loop, a network call with no timeout of its own,
    // an unexpected interactive prompt) would otherwise hold the single run slot forever until the
    // user hits Cancel. Cap the wait at a generous ceiling so a genuinely-stuck run frees its slot
    // on its own; legitimately long runs (big fork syncs) finish well within it. On expiry, reuse
    // the same tree-kill as cancel_run (taskkill /T /F — child.kill() would orphan the pwsh subtree)
    // then reap so no zombie/handle leaks. kill_tree is idempotent (128 → Ok), so a Cancel racing
    // the timeout can't error.
    const RUN_MAX: std::time::Duration = std::time::Duration::from_secs(30 * 60);
    let pid = child.id();
    let status = match tokio::time::timeout(RUN_MAX, child.wait()).await {
        Ok(s) => s,
        Err(_elapsed) => {
            if let Some(p) = pid {
                let _ = kill_tree(p);
            }
            // Bound the post-kill reap too: a truly unkillable tree must still free the run slot
            // instead of wedging maintenance forever. On expiry fall through to a synthetic error,
            // which becomes exit -1 below.
            const REAP_MAX: std::time::Duration = std::time::Duration::from_secs(10);
            match tokio::time::timeout(REAP_MAX, child.wait()).await {
                Ok(s) => s,
                Err(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "child did not exit after kill",
                )),
            }
        }
    };
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
/// `Split::next_segment` is cancellation-safe (its partial buffer lives in `Split`, not the future),
/// so wrapping it in `timeout_at` cannot drop a line. Splitting on the raw byte `\n` (rather than
/// `Lines`, which validates UTF-8 per line) is deliberate: a single non-UTF-8 byte — a `cmd /c` tool
/// printing CP866 on a Russian Windows — made `next_line` return Err, which the loop treated as EOF
/// and silently truncated the rest of the run's output. Now each segment is decoded leniently.
async fn pump_stream<R>(
    app: AppHandle,
    id: String,
    log_event: &'static str,
    stream: &'static str,
    reader: R,
) where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
{
    const FLUSH_MS: u64 = 30;
    const MAX_BATCH: usize = 64;
    let mut segments = reader.split(b'\n');
    let mut batch: Vec<String> = Vec::new();
    // Deadline for the current (non-empty) batch: flush FLUSH_MS after its FIRST line, bounding
    // console latency regardless of how steadily lines arrive.
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let read = match deadline {
            None => segments.next_segment().await,
            Some(dl) => match tokio::time::timeout_at(dl, segments.next_segment()).await {
                Ok(r) => r,
                Err(_) => {
                    flush_batch(&app, log_event, &id, stream, &mut batch);
                    deadline = None;
                    continue;
                }
            },
        };
        match read {
            Ok(Some(seg)) => {
                // Decode leniently (bad bytes -> U+FFFD) so one non-UTF-8 line can't truncate the
                // stream; strip the trailing CR so CRLF output doesn't leave a stray '\r' per line.
                let mut line = String::from_utf8_lossy(&seg).into_owned();
                if line.ends_with('\r') {
                    line.pop();
                }
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

/// User-editable fork discovery config, surfaced in the Forks tab. Mirrors the JSON the fork-sync
/// script reads (`roots`/`paths`/`ownPaths`/`fetchTimeoutSec`/`ghTimeoutSec`).
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ForkConfig {
    #[serde(default)]
    roots: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    own_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetch_timeout_sec: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gh_timeout_sec: Option<u32>,
}

/// Durable fork config location — kept in `%APPDATA%\castellyn` (like config.json), NOT in the
/// vendored `tools/fork-updater/repos.json`, so a tool update can't clobber the user's fork setup.
fn fork_config_path() -> Option<String> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| format!("{a}\\castellyn\\forks.json"))
}

/// The vendored `repos.json` next to the forks script — read once to seed the durable copy on migration.
fn fork_vendored_config_path() -> Option<String> {
    let comp = raw_components().into_iter().find(|c| c.id == "forks")?;
    let script = abs_with_fallback(&comp.script_rel, FORKS_SCRIPT_FALLBACK);
    let dir = std::path::Path::new(&script).parent()?.to_string_lossy().to_string();
    Some(format!("{dir}\\repos.json"))
}

/// Read the fork discovery config: durable `%APPDATA%\castellyn\forks.json` first; on first run, seed
/// it from the vendored `repos.json`; else defaults. Never errors — a bad file yields defaults.
#[tauri::command]
fn read_fork_config() -> ForkConfig {
    if let Some(p) = fork_config_path() {
        if let Ok(Some(v)) = read_json_opt(&p, "forks.json") {
            if let Ok(cfg) = serde_json::from_value(v) {
                return cfg;
            }
        }
    }
    // Migrate-seed from the vendored repos.json so an existing setup carries over.
    if let Some(vp) = fork_vendored_config_path() {
        if let Ok(Some(v)) = read_json_opt(&vp, "repos.json") {
            if let Ok(cfg) = serde_json::from_value::<ForkConfig>(v) {
                let _ = write_fork_config_inner(&cfg); // best-effort seed of the durable copy
                return cfg;
            }
        }
    }
    ForkConfig::default()
}

fn write_fork_config_inner(config: &ForkConfig) -> Result<(), String> {
    let p = fork_config_path().ok_or_else(|| "no APPDATA".to_string())?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    write_json_atomic(&p, &json).map_err(|e| e.to_string())
}

/// Persist the fork discovery config to the durable path. Subsequent fork runs read it via -ConfigPath.
#[tauri::command]
fn write_fork_config(config: ForkConfig) -> Result<(), String> {
    write_fork_config_inner(&config)
}

/// `-ConfigPath <durable forks.json>` when it exists, so the fork script reads the user's UI-edited
/// config. Absent (never edited) → empty, and the script falls back to its vendored repos.json.
fn fork_config_args() -> Vec<String> {
    match fork_config_path() {
        Some(p) if std::path::Path::new(&p).exists() => vec!["-ConfigPath".into(), p],
        _ => Vec::new(),
    }
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
    args.extend(fork_config_args());
    let script = abs_with_fallback(&comp.script_rel, FORKS_SCRIPT_FALLBACK);
    // Claim the global slot (Drop clears it even if this future is dropped mid-await), then bail if any
    // per-repo run is active (would `git fetch` the same repo concurrently). Order: the flag is set first
    // so a per-repo run starting now sees it.
    let _global = ForksGlobalSlot::reserve();
    if !runs.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        return Err(tr("err.fork_busy", cur_lang()).to_string());
    }
    spawn_streamed(app, state, stream_id::FORKS.to_string(), script, args).await
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
    full.extend(fork_config_args());
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
/// Ask the tree to close, then force it. The soft `/T` pass gives a script the moment it needs to
/// finish writing its `<id>.last.json` status envelope; a straight `/T /F` could tear the file in
/// half mid-write, and a cancelled run would then read back as "corrupt" instead of "cancelled".
/// `GRACE_MS` is the whole budget, not per-poll — a wedged tree still dies, just a beat later.
fn kill_tree(pid: u32) -> Result<(), String> {
    const GRACE_MS: u64 = 600;
    const POLL_MS: u64 = 120;

    let taskkill = |force: bool| {
        let pid_s = pid.to_string();
        let mut args = vec!["/PID", &pid_s, "/T"];
        if force {
            args.push("/F");
        }
        std::process::Command::new("taskkill")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    };

    // A soft kill can legitimately fail (console apps with no window ignore WM_CLOSE) — that is not
    // an error, it just means we fall through to the forced pass below. Only a POSITIVE "it is gone"
    // ends the grace window early: if the liveness probe itself failed we must not report a kill we
    // never confirmed, so we fall through to `/F` instead.
    if let Ok(o) = taskkill(false) {
        if o.status.success() {
            for _ in 0..(GRACE_MS / POLL_MS) {
                std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                if pid_alive(pid) == Some(false) {
                    return Ok(());
                }
            }
        }
    }

    match taskkill(true) {
        // 128 = "process not found": it exited during the grace window, which is the success we wanted.
        Ok(o) if o.status.success() || o.status.code() == Some(128) => Ok(()),
        Ok(_) if pid_alive(pid) == Some(false) => Ok(()),
        Ok(o) => {
            let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(trv("err.kill_failed", cur_lang(), &[("e", &msg)]))
        }
        Err(e) => Err(trv("err.kill_failed", cur_lang(), &[("e", &e)])),
    }
}

/// Is `pid` still running? `tasklist` filtered by PID prints the header only when nothing matches,
/// so the PID appearing in its own output is the liveness signal.
///
/// `None` means the probe could not answer (tasklist missing, spawn refused). That is NOT "the process
/// is gone": collapsing it to `false` let `kill_tree` report success before its force pass ever ran.
fn pid_alive(pid: u32) -> Option<bool> {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
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

const BACKUP_DIR_REL: &str = "{{PROFILES}}\\Backups";
const BACKUP_SCRIPT_REL: &str = "{{PROFILES}}\\Backup-ClaudeSetup.ps1";
const RESTORE_SCRIPT_REL: &str = "{{PROFILES}}\\Restore-ClaudeSetup.ps1";

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
        .map_err(|e| trv("err.delete_failed", cur_lang(), &[("path", &path), ("e", &e)]))
}

/// `tar -tf`: entry count on success, tar's stderr when the zip is corrupt/truncated. Shared by
/// verify_backup and import_backup_zip.
fn tar_list(path: &str) -> Result<usize, String> {
    let out = std::process::Command::new(system_tar())
        .args(["-tf", path])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.tar_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).lines().count())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn tar_extract(path: &str, dest: &str) -> Result<(), String> {
    let out = std::process::Command::new(system_tar())
        .args(["-x", "-f", path, "-C", dest])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.tar_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// F9: verify a weekly archive by listing it (`tar -tf`). Returns the entry count on success, or the
/// tar stderr if the zip is corrupt/truncated.
#[tauri::command]
fn verify_backup(name: String) -> Result<usize, String> {
    tar_list(&weekly_archive_path(&name)?)
}

/// F9: extract a weekly archive to a user-picked folder. NON-destructive — never writes over the live
/// ~/.claude (the weekly archives skills/agents/commands, which are Syncthing-synced + junctioned).
#[tauri::command]
fn extract_backup(name: String, dest: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    if !std::path::Path::new(&dest).is_dir() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &dest)]));
    }
    tar_extract(&path, &dest)
}

/// Import a backup zip from an ARBITRARY path (USB stick, another machine's export): verify first
/// so a corrupt zip fails before anything lands in `dest`, then extract. Non-destructive like
/// extract_backup — the user picks an explicit destination, the live ~/.claude is never a target.
#[tauri::command]
fn import_backup_zip(path: String, dest: String) -> Result<usize, String> {
    let p = std::path::Path::new(&path);
    if !p.is_file() || !path.to_lowercase().ends_with(".zip") {
        return Err(tr("err.bad_path", cur_lang()).to_string());
    }
    if !std::path::Path::new(&dest).is_dir() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &dest)]));
    }
    let n = tar_list(&path)?;
    tar_extract(&path, &dest)?;
    Ok(n)
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
    spawn_streamed(app, state, stream_id::BACKUP.to_string(), script, args).await
}

const PROFILES_SCRIPT_REL: &str = "{{PROFILES}}\\Get-ProfilesStatus.ps1";
const INSTALL_SCRIPT_REL: &str = "{{PROFILES}}\\Install-ClaudeProfiles.ps1";
const REPAIR_SCRIPT_REL: &str = "{{PROFILES}}\\Repair-ProfileLinks.ps1";
const ONBOARDING_SCRIPT_REL: &str = "{{PROFILES}}\\Repair-Onboarding.ps1";
const PROFILES_JSON_REL: &str = "{{PROFILES}}\\profiles.last.json";
// Config-drift (FUN-7): shared-config FILE link health. links.last.json is written by
// Check-Integrity.ps1; Relink self-elevates; sync-now reuses the Backup mirror.
const RELINK_SCRIPT_REL: &str = "{{PROFILES}}\\Relink-SharedConfig.ps1";
const INTEGRITY_SCRIPT_REL: &str = "{{PROFILES}}\\Check-Integrity.ps1";
const CONFIG_DRIFT_JSON_REL: &str = "{{PROFILES}}\\links.last.json";

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
/// create a single missing profile (`create`), repair the links of a single profile (`repair`), or
/// restore a single profile's onboarding flag after `/logout` (`fix-onboarding`).
/// `create`, `repair` and `fix-onboarding` require `name`.
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
        // Restore `hasCompletedOnboarding` after a /logout stranded the profile in the onboarding
        // wizard (see Repair-Onboarding.ps1). Same charset + membership gate as `repair`: the name
        // becomes a -Name argv.
        "fix-onboarding" => {
            let n = name.unwrap_or_default();
            if !valid_profile_name(&n) {
                return Err(trv("err.invalid_profile_name", cur_lang(), &[("name", &n)]));
            }
            if !profile_names().iter().any(|x| x == &n) {
                return Err(trv("err.unknown_profile", cur_lang(), &[("name", &n)]));
            }
            (ONBOARDING_SCRIPT_REL, vec!["-Name".to_string(), n])
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
    spawn_streamed(app, state, stream_id::PROFILES.to_string(), script, args).await
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
    spawn_streamed(app, state, stream_id::SYNC.to_string(), script, args).await
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
    // Cap the O(m*n) LCS: two large line-lists allocate a quadratic matrix (two 20k-line files
    // ~3.2 GB) and can OOM. Above the cap fall back to a whole-file replace (all deletes then all
    // adds). The drift diff only runs on small config files, so the cap is never hit there.
    const MAX_LCS_CELLS: usize = 4_000_000; // ~2000x2000 lines
    if m.saturating_mul(n) > MAX_LCS_CELLS {
        let mut result = Vec::with_capacity(m + n);
        result.extend(a.iter().map(|line| DiffLine {
            kind: DiffLineKind::Del,
            text: line.clone(),
        }));
        result.extend(b.iter().map(|line| DiffLine {
            kind: DiffLineKind::Add,
            text: line.clone(),
        }));
        return result;
    }
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

const CONFIG_SOURCE_REL: &str = "{{PROFILES}}\\config";

/// Read a unified diff between a drifted live file and its shared config copy.
/// `name` is the filename (e.g. "statusline.py").
/// Returns null if either file is missing.
#[tauri::command]
fn read_drift_diff(name: String) -> Result<Option<DriftDiff>, String> {
    let home = std::env::var("USERPROFILE").map_err(|_| "no USERPROFILE".to_string())?;
    let tip_path = format!("{}\\.claude\\{}", home, name);
    // abs() expands the {{PROFILES}} placeholder in CONFIG_SOURCE_REL (which is an ABSOLUTE path);
    // a raw format! left the literal token in the path AND wrongly prefixed scripts_root(), so the
    // source file was never found -> Ok(None) -> the UI rendered an empty "—" for every drifted file.
    let source_path = abs(&format!("{CONFIG_SOURCE_REL}\\{name}"));

    let tip_content = match std::fs::read_to_string(&tip_path) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let source_content = match std::fs::read_to_string(&source_path) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };

    // Strip a leading UTF-8 BOM before splitting: when one copy carries a BOM and the other doesn't,
    // an un-stripped BOM makes the first line differ and shows a phantom change (U+FEFF is not
    // trimmed by .lines()).
    let tip_lines: Vec<String> = tip_content.trim_start_matches('\u{feff}').lines().map(String::from).collect();
    let source_lines: Vec<String> = source_content.trim_start_matches('\u{feff}').lines().map(String::from).collect();

    let lines = compute_diff(&source_lines, &tip_lines);

    Ok(Some(DriftDiff {
        tip_path,
        source_path,
        source_lines: source_lines.len(),
        tip_lines: tip_lines.len(),
        lines,
    }))
}

const PROFILES_CONFIG_REL: &str = "{{PROFILES}}\\config\\profiles.json";
const PROFILE_MGMT_SCRIPT_REL: &str = "{{PROFILES}}\\Manage-Profiles.ps1";

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

/// A legitimate ssh target is `[user@]host` (v4/v6 literal or name) or a `~/.ssh/config` Host alias —
/// a single token. It must NOT begin with `-`, contain whitespace, or carry option/shell metachars:
/// `--%` stops PowerShell re-parsing the ssh line but NOT ssh.exe's own option parsing, so a "target"
/// like `-oProxyCommand=calc.exe host` (which can arrive from a persisted/synced session recipe or an
/// imported ~/.ssh/config entry, not just live typing) would run an arbitrary local program before
/// connecting. Keep the charset tight — space + a leading dash are what the attack needs.
fn valid_ssh_target(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@' | ':' | '[' | ']'))
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
    // cc3-class guard (same as provider/proxy/folders paths): removing or renaming a profile whose
    // session is live deletes/moves the very dir that session is reading.
    if matches!(action.as_str(), "remove" | "rename") && profile_session_active(&name) {
        return Err(trv("log.profile_running_warn", cur_lang(), &[("name", &name)]));
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
    spawn_streamed(app, state, stream_id::PROFILES.to_string(), script, args).await
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
const SYNC_CONFIG_REL: &str = "{{PROFILES}}\\config\\sync-config.json";
const SYNC_CANON_STIGNORE_REL: &str = "{{PROFILES}}\\config\\.stignore";

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
/// Does this file name carry a cleartext secret, so it must never get an in-place `.bak`?
///
/// A profile's `settings.json` holds the ANTHROPIC_AUTH_TOKEN, `opencode.json` a literal apiKey, and
/// `.claude.json` the auth token plus per-server MCP `env` blocks. `.mcp.json` is the SOURCE of those
/// same env blocks — it belongs here for exactly the reason `.claude.json` does. All of them live
/// outside Castellyn's own directory, in the Syncthing-synced settings tree, so a `.bak` would strand
/// the pre-rotation secret on every machine. The atomic temp+rename below already guarantees the
/// target is never blanked, so skipping the backup costs no crash safety.
fn is_secret_file(name: &str) -> bool {
    name.eq_ignore_ascii_case("settings.json")
        || name.eq_ignore_ascii_case("opencode.json")
        || name.eq_ignore_ascii_case(".claude.json")
        || name.eq_ignore_ascii_case(".mcp.json")
}

/// A hard crash (or power loss) between `write_file_no_bom` and `rename` strands `<secret>.tmp`
/// holding a full copy of the secret. Once the real file's token is rotated, that debris is the
/// *stale* secret living on in a synced tree — the same hazard `.bak` posed, which is why we sweep
/// it for the exact same file names.
///
/// Scope is the directory we are about to write to anyway, so there is no startup scan and no extra
/// I/O beyond one `read_dir` of a small folder. A temp file younger than `stale` belongs to a
/// concurrent writer (a second Castellyn instance) and is left alone; real debris is minutes old at
/// least, since the process that made it is gone.
/// Monotonic uniquifier for atomic-write temp names, so two concurrent writers of the SAME file
/// (same pid) can't share one `<file>.tmp` and clobber each other's bytes before the rename.
static ATOMIC_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Recover the real target filename from an atomic-write temp name — either "<file>.tmp" (legacy) or
/// "<file>.<pid>.<seq>.tmp" (unique per writer) — so the secret-tmp sweep still recognises both. The
/// ".<pid>.<seq>" tail is dropped only when both segments are all-digits; our secret filenames never
/// end in a numeric segment, so a real name can't be over-stripped.
fn tmp_secret_base(name: &str) -> Option<&str> {
    let rest = name.strip_suffix(".tmp")?;
    if let Some((head, seq)) = rest.rsplit_once('.') {
        if !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit()) {
            if let Some((base, pid)) = head.rsplit_once('.') {
                if !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) {
                    return Some(base);
                }
            }
        }
    }
    Some(rest)
}

fn sweep_stale_secret_tmp(dir: &std::path::Path, stale: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(base) = tmp_secret_base(name) else {
            continue;
        };
        if !is_secret_file(base) {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| std::time::SystemTime::now().duration_since(t).ok());
        // Unreadable mtime, or a clock that ran backwards → leave it. Deleting on a guess could
        // race a live writer, and the file is no worse than the cleartext target beside it.
        if age.is_some_and(|a| a >= stale) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Debris younger than this is assumed to be a concurrent writer's temp, not a crash leftover.
const SECRET_TMP_STALE: std::time::Duration = std::time::Duration::from_secs(60);

fn write_json_atomic(path: &str, content: &str) -> std::io::Result<()> {
    let p = std::path::Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
        sweep_stale_secret_tmp(dir, SECRET_TMP_STALE);
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
    // EXCEPT for secret-bearing files (see `is_secret_file`).
    let is_secret = p
        .file_name()
        .and_then(|n| n.to_str())
        .map(is_secret_file)
        .unwrap_or(false);
    if p.exists() && !is_secret {
        let _ = std::fs::copy(path, format!("{path}.bak"));
    } else if is_secret {
        // A .bak may already exist from before this file was recognised as secret-bearing (or from an
        // older build). Leaving it would strand the pre-rotation secret forever, which is exactly what
        // skipping the backup is meant to prevent. Removing OUR OWN backup of this file, nothing else.
        let _ = std::fs::remove_file(format!("{path}.bak"));
    }
    if was_hidden || was_ro {
        run_attrib(&["-h", "-r"], path);
    }
    // Temp in the SAME dir so the rename is a same-volume atomic replace. Unique per writer (pid + a
    // monotonic counter) so two concurrent writers of this file don't share one temp and clobber each
    // other's bytes before the rename. On a write failure remove the partial temp instead of leaking it.
    let seq = ATOMIC_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = format!("{path}.{}.{}.tmp", std::process::id(), seq);
    if let Err(e) = write_file_no_bom(&tmp, content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // Force the temp file's bytes to disk BEFORE the rename. The rename is atomic for metadata, but
    // without this the directory entry can land while the contents are still only in the page cache:
    // a power loss at that instant leaves a correctly-named, EMPTY config. Best-effort — a failed
    // flush still leaves the old file intact, which is the guarantee the rename already gives us.
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&tmp) {
        let _ = f.sync_all();
    }
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
const ENGINES_CONFIG_REL: &str = "{{PROFILES}}\\config\\engines.json";
/// Per-profile launch config (full vs lean mode + which tools to re-include when lean).
const LAUNCH_CONFIG_REL: &str = "{{PROFILES}}\\config\\profile-launch.json";

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
    let exts = [".exe", ".cmd", ".bat"];
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            for ext in exts {
                let cand = dir.join(format!("{name}{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    // Fallback: the npm global bin (%APPDATA%\npm) holds the claude/ccr shims but isn't always on a
    // GUI-launched app's runtime PATH (some setups add it only to an interactive shell). Without this
    // a shortcut-launched Castellyn resolved nothing → "0 plugins / claude CLI unavailable".
    if let Ok(appdata) = std::env::var("APPDATA") {
        let npm = std::path::Path::new(&appdata).join("npm");
        for ext in exts {
            let cand = npm.join(format!("{name}{ext}"));
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

// ===================== Native LLM-stack supervisor (behind config.stackNative, default off) =========
// Castellyn starts/tracks(PID)/stops each service ITSELF instead of the PS launcher's detached
// `cmd /k` windows: hidden (CREATE_NO_WINDOW) → no orphaned consoles; killed by the tracked top-of-tree
// pid → Stop always closes them; port-checked before spawn → no start race; output → a per-service log
// file. PIDs persist to disk so Stop works across an app restart. Ships dark (scripts stay default)
// until the owner flips stackNative and smokes it live.
#[derive(Default)]
struct StackProcs(Mutex<std::collections::HashMap<String, u32>>);

// Cancel flag: a Stop signals an in-flight native start to abort its sequential spawn loop (mirrors
// BULK_PLUGINS_CANCEL). Without it, Stop would kill the services started so far while the start loop
// keeps spawning more.
static STACK_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn stack_log_dir() -> Option<std::path::PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| std::path::Path::new(&a).join("castellyn").join("stack-logs"))
}
fn stack_procs_path() -> Option<std::path::PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| std::path::Path::new(&a).join("castellyn").join("stack-procs.json"))
}
fn load_stack_procs() -> std::collections::HashMap<String, u32> {
    stack_procs_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}
fn save_stack_procs(m: &std::collections::HashMap<String, u32>) {
    if let Some(p) = stack_procs_path() {
        // Route through the atomic writer (temp+rename): a crash mid-write must never leave a
        // half-written/blanked PID map — a blanked map orphans running services (Stop can't find
        // their pids). Surface a persist failure instead of swallowing it.
        match serde_json::to_string(m) {
            Ok(txt) => {
                if let Err(e) = write_json_atomic(&p.to_string_lossy(), &txt) {
                    eprintln!("[stack] failed to persist stack-procs.json: {e}");
                }
            }
            Err(e) => eprintln!("[stack] failed to serialize stack procs: {e}"),
        }
    }
}

/// Run one monitor-thread tick under a panic guard: a panicking tick logs and the polling loop
/// keeps running, instead of the whole thread dying silently (mirrors the RunSlot panic-survival
/// guarantee). A dead monitor thread means notifications quietly stop for the app's lifetime.
pub(crate) fn run_guarded<F: FnOnce()>(name: &str, tick: F) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(tick)).is_err() {
        eprintln!("[monitor:{name}] tick panicked; thread continues");
    }
}

/// Parse `netstat -ano` for LISTENING sockets → port → pid. Hidden. The fallback for stopping a
/// service whose spawn pid we no longer hold (started outside the app, or after an app restart).
fn listening_pids() -> std::collections::HashMap<u16, u32> {
    let mut out = std::collections::HashMap::new();
    let Ok(o) = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return out;
    };
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        // "TCP  0.0.0.0:13001  0.0.0.0:0  LISTENING  1234". The STATE column (f[3]) is LOCALIZED
        // (e.g. "ПРОСЛУШИВАНИЕ" on a Russian Windows), so a literal "LISTENING" match found nothing
        // and the stack's fallback stop couldn't locate the pid. Match the locale-independent
        // signature a listener always has instead — a wildcard foreign address — exactly like
        // listeners_on_port. pid is the last column.
        if f.len() < 5 || !f[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if f[2] != "0.0.0.0:0" && f[2] != "[::]:0" {
            continue;
        }
        if let (Some(port), Ok(pid)) = (
            f[1].rsplit(':').next().and_then(|p| p.parse::<u16>().ok()),
            f[f.len() - 1].parse::<u32>(),
        ) {
            out.entry(port).or_insert(pid);
        }
    }
    out
}

/// Spawn ONE service in the background, HIDDEN, stdout+stderr appended to its log file. Returns the
/// pid of the top `cmd` (kill_tree of it later takes the whole npm→node tree). env/cwd from the
/// manifest; SCRIPTS_ROOT exported for {{SCRIPTS_ROOT}} expansion. Dropping the Child does NOT kill
/// it on Windows — the service keeps running, tracked by pid.
fn spawn_service_native(svc: &serde_json::Value) -> Option<u32> {
    let s = |k: &str| svc.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let dir = expand_placeholders(&s("dir"));
    let command = expand_placeholders(&s("command"));
    let id = s("id");
    if command.trim().is_empty() || !std::path::Path::new(&dir).is_dir() {
        return None;
    }
    let mut cmd = std::process::Command::new("cmd");
    // chcp 65001 so a service's non-ASCII output lands as UTF-8 in stack-logs\<id>.log (not mojibake),
    // mirroring the launcher's per-window `chcp 65001` (start-stack.ps1).
    cmd.args(["/c", &format!("chcp 65001>nul & {command}")]);
    cmd.current_dir(&dir);
    cmd.env("SCRIPTS_ROOT", scripts_root());
    if let Some(env) = svc.get("env").and_then(|e| e.as_object()) {
        for (k, v) in env {
            if let Some(val) = v.as_str() {
                cmd.env(k, expand_placeholders(val));
            }
        }
    }
    if let Some(ld) = stack_log_dir() {
        let _ = std::fs::create_dir_all(&ld);
        if let Ok(f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(ld.join(format!("{id}.log")))
        {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f);
                cmd.stderr(f2);
            }
        }
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().ok().map(|c| c.id())
}

/// Native equivalent of the launcher's Wait-Ready: wait for the TCP port to listen, THEN — if the
/// service declares a `health` path — until any HTTP response on it (even 4xx proves it's alive).
/// One shared ~25s deadline for both, like start-stack.ps1. A bound port whose app isn't serving yet
/// must NOT read as "up" (that was the v1 port-only gap). The blocking health GET runs off the async
/// worker via spawn_blocking.
/// Outcome of waiting for a service. A listening port alone counts as "up" — a declared health path
/// is a soft upgrade, never a veto (an upstream-throttled proxy answers non-2xx on /v1/models while
/// perfectly alive; failing it would report a running service as dead and leave it running).
enum Readiness {
    Down,      // port never listened within its budget — a real failure
    PortUp,    // port listens, but a declared health path didn't answer 2xx in time (still running)
    Healthy,   // port listens + 2xx health
    Cancelled, // a Stop set STACK_CANCEL mid-wait — abort, do NOT count as a failure (CAST-5)
}

/// Per-service port-readiness budget in seconds. An OmniRoute/Qwen engine can cold-start >25s, so
/// stack.json may declare `readyTimeoutSec`; anything missing or non-positive falls back to the
/// historical 25s default (never a zero budget). Pure so the calibration knob has a real assert.
fn ready_timeout_secs(svc: &serde_json::Value) -> u64 {
    svc.get("readyTimeoutSec")
        .and_then(|x| x.as_u64())
        .filter(|&n| n > 0)
        .unwrap_or(25)
}

/// Per-service soft-health-confirmation budget in seconds (mirrors `ready_timeout_secs`). Declared
/// via `healthTimeoutSec`; anything missing or non-positive falls back to the historical 15s
/// default, so a stack.json without the field behaves exactly as before.
fn health_timeout_secs(svc: &serde_json::Value) -> u64 {
    svc.get("healthTimeoutSec")
        .and_then(|x| x.as_u64())
        .filter(|&n| n > 0)
        .unwrap_or(15)
}

async fn native_wait_ready(svc: &serde_json::Value) -> Readiness {
    let port = svc.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
    if port == 0 {
        return Readiness::Healthy;
    }
    let health = svc
        .get("health")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    // 1) wait for the port to listen — its OWN budget (stack.json readyTimeoutSec, default 25s).
    //    port_listening is a blocking TCP connect, so run it off the async worker like the health
    //    probe below. Slow-binding engines (headless-Chrome cold start) need longer or they read
    //    as a false [fail].
    let port_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(ready_timeout_secs(svc));
    let mut listening = false;
    while std::time::Instant::now() < port_deadline {
        if STACK_CANCEL.load(Ordering::SeqCst) {
            return Readiness::Cancelled;
        }
        if tokio::task::spawn_blocking(move || port_listening(port))
            .await
            .unwrap_or(false)
        {
            listening = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    if !listening {
        return Readiness::Down;
    }
    if health.is_empty() {
        return Readiness::Healthy;
    }
    // 2) soft health confirmation — a FRESH budget (stack.json healthTimeoutSec, default 15s) so a
    //    slow-binding port can't starve it. A non-2xx / no answer here downgrades to PortUp, it does
    //    NOT fail the service.
    let health_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(health_timeout_secs(svc));
    while std::time::Instant::now() < health_deadline {
        if STACK_CANCEL.load(Ordering::SeqCst) {
            return Readiness::Cancelled;
        }
        let h = health.clone();
        if tokio::task::spawn_blocking(move || http_health_ok(port, &h))
            .await
            .unwrap_or(false)
        {
            return Readiness::Healthy;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Readiness::PortUp
}

fn stack_emit(app: &AppHandle, line: String) {
    let _ = app.emit(
        "run-log",
        LogLine {
            component: stream_id::ENGINE.to_string(),
            stream: "out".into(),
            line,
        },
    );
}

/// Stable topological sort of stack.json services by `dependsOn` (Kahn's algorithm), so the start
/// loop brings backends up before the front that depends on them. A service is emitted only after
/// every id in its `dependsOn` that is actually present in `services` — a dependency on a missing
/// id is ignored, never a hang. Ties (no edge between two services) keep manifest order. A cycle
/// can't be resolved by definition, so the nodes involved in it fall back to manifest order instead
/// of panicking or looping forever. Pure (no logging/I/O) — `native_stack_start` is the only
/// consumer of the ordering.
// ponytail: O(n^2) linear scan per emission — fine for a stack.json-sized service list (single
// digits); swap for a binary-heap-backed Kahn's if this ever needs to order hundreds of services.
fn order_services(services: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let n = services.len();
    let mut index_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, svc) in services.iter().enumerate() {
        if let Some(id) = svc.get("id").and_then(|x| x.as_str()) {
            index_of.entry(id).or_insert(i);
        }
    }
    // dependents[d] = indices that depend on service d (edges only for deps present in `services`).
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree: Vec<usize> = vec![0; n];
    for (i, svc) in services.iter().enumerate() {
        let Some(dep_ids) = svc.get("dependsOn").and_then(|x| x.as_array()) else {
            continue;
        };
        for d in dep_ids {
            let Some(&di) = d.as_str().and_then(|d| index_of.get(d)) else {
                continue; // missing id — ignored, not a blocker
            };
            if di != i {
                dependents[di].push(i);
                indegree[i] += 1;
            }
        }
    }
    let mut emitted = vec![false; n];
    let mut result = Vec::with_capacity(n);
    loop {
        // Lowest-index unemitted node with no unmet deps left — keeps ties in manifest order.
        let Some(i) = (0..n).find(|&i| !emitted[i] && indegree[i] == 0) else {
            break; // remaining nodes (if any) are part of a cycle
        };
        emitted[i] = true;
        result.push(services[i].clone());
        for &dep_i in &dependents[i] {
            indegree[dep_i] = indegree[dep_i].saturating_sub(1);
        }
    }
    // Cycle fallback: whatever couldn't be resolved is appended in original manifest order.
    for (i, svc) in services.iter().enumerate() {
        if !emitted[i] {
            result.push(svc.clone());
        }
    }
    result
}

/// Whether a failed service should trigger a rollback of everything this run already started.
/// Trivial today (mirrors the `teardownOnFailure` flag verbatim) but named/typed so the call sites
/// read as intent and the gate has a real unit test independent of the async start loop.
/// Deliberately NOT keyed on `critical`: `critical` drives the health-card alarm (a display concern),
/// whereas teardown is a lifecycle concern — a service can be the health-critical front yet still
/// want its backends left running on its own failure. Only the OmniRoute front opts into teardown
/// (set in the Part B live session); no shipped service sets it, so this path is inert today.
fn should_teardown(failed_wants_teardown: bool) -> bool {
    failed_wants_teardown
}

/// Kills and unregisters every pid `native_stack_start` already spawned this run, then logs the
/// rollback. Called when a `teardownOnFailure:true` service fails to come up — leaves the stack
/// exactly as it was before this Start, instead of a half-started backend/router pair.
fn teardown_started(
    app: &AppHandle,
    procs: &StackProcs,
    failed_name: &str,
    started_pids: &[(String, u32)],
) {
    if started_pids.is_empty() {
        return;
    }
    {
        let mut m = procs.0.lock().unwrap_or_else(|e| e.into_inner());
        for (sid, pid) in started_pids {
            let _ = kill_tree(*pid);
            m.remove(sid);
        }
        save_stack_procs(&m);
    }
    stack_emit(
        app,
        format!(
            "[teardown] {failed_name} failed — rolled back {} service(s)",
            started_pids.len()
        ),
    );
}

/// Native START: spawn each target enabled service (idempotent — skip an already-listening port),
/// track its pid, wait until it's up. `only` = one service; else the core+router groups (the PS
/// -Router start-all). Aborts early if a Stop set STACK_CANCEL. A `teardownOnFailure:true` service
/// that fails to come up rolls back every service THIS run already started (kill + unregister pid) —
/// a half-started stack (e.g. a router up with no backend behind it) is worse than none.
async fn native_stack_start(app: &AppHandle, procs: &StackProcs, only: Option<&str>) -> i32 {
    STACK_CANCEL.store(false, Ordering::SeqCst);
    let services = stack_services(); // read the manifest once; reused for the id check and the loop
    // A -Only id that isn't in the manifest otherwise looks just like "nothing started".
    if let Some(o) = only {
        if !services
            .iter()
            .any(|s| s.get("id").and_then(|x| x.as_str()) == Some(o))
        {
            stack_emit(app, format!("[warn] unknown service id: {o}"));
        }
    }
    let mut started = 0;
    let mut matched = 0;
    let mut failed = 0;
    // pids this run actually spawned — rolled back in full if a teardownOnFailure service then fails.
    let mut started_pids: Vec<(String, u32)> = Vec::new();
    let mut teardown_fired = false;
    let mut cancelled = false;
    for svc in order_services(&services) {
        if STACK_CANCEL.load(Ordering::SeqCst) {
            stack_emit(app, "[cancel] stop requested — aborting start".into());
            cancelled = true;
            break;
        }
        let sid = svc.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let name = svc
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(&sid)
            .to_string();
        let enabled = svc.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
        let group = svc.get("group").and_then(|x| x.as_str()).unwrap_or("");
        let port = svc.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
        let target = match only {
            Some(o) => sid == o,
            None => enabled && matches!(group, "core" | "router"),
        };
        if !target {
            continue;
        }
        matched += 1;
        if !enabled {
            stack_emit(app, format!("[skip] {name}: disabled"));
            continue;
        }
        // dir must exist (a distinct skip from a spawn failure).
        let dir = expand_placeholders(svc.get("dir").and_then(|x| x.as_str()).unwrap_or(""));
        if dir.is_empty() || !std::path::Path::new(&dir).is_dir() {
            stack_emit(app, format!("[skip] {name}: dir not found → {dir}"));
            continue;
        }
        // requires: auth/config files that must exist before launch (deepseek-auth.json, accounts.json).
        // Missing → clean skip, not a spawn that dies and then blocks native_wait_ready for 25s.
        let missing: Vec<String> = svc
            .get("requires")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .filter(|req| !std::path::Path::new(&dir).join(req).exists())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        if !missing.is_empty() {
            stack_emit(
                app,
                format!("[skip] {name}: missing {} (authorize first)", missing.join(", ")),
            );
            continue;
        }
        if port != 0 && probe_ports(&[port]).first().copied().unwrap_or(false) {
            stack_emit(app, format!("[skip] {name}: :{port} already up"));
            continue;
        }
        match spawn_service_native(&svc) {
            Some(pid) => {
                {
                    let mut m = procs.0.lock().unwrap_or_else(|e| e.into_inner());
                    m.insert(sid.clone(), pid);
                    save_stack_procs(&m);
                }
                started_pids.push((sid.clone(), pid));
                stack_emit(app, format!("[ .. ] {name} (pid {pid}) starting…"));
                match native_wait_ready(&svc).await {
                    Readiness::Cancelled => {
                        // Stop preempted this start mid-wait — abort cleanly, not a failure (CAST-5).
                        // Tear down what THIS run spawned: the preempting Stop snapshotted the process
                        // set before our `procs.insert` above, and if the service is not listening yet
                        // its port-based fallback misses it too — so it would survive as an orphan
                        // holding a stack port. Unconditional (unlike the failure path's opt-in
                        // teardown): the user asked for a stop, not for a half-started stack.
                        stack_emit(app, format!("[cancel] {name}: start aborted"));
                        teardown_started(app, procs, &name, &started_pids);
                        cancelled = true;
                        break;
                    }
                    Readiness::Down => {
                        failed += 1;
                        stack_emit(app, format!("[fail] {name} did not come up — see stack-logs\\{sid}.log"));
                        let wants_teardown = svc.get("teardownOnFailure").and_then(|x| x.as_bool()).unwrap_or(false);
                        if should_teardown(wants_teardown) {
                            teardown_started(app, procs, &name, &started_pids);
                            teardown_fired = true;
                            break;
                        }
                    }
                    ready => {
                        started += 1;
                        if matches!(ready, Readiness::PortUp) {
                            stack_emit(app, format!("[ ok ] {name} (port up; health not confirmed)"));
                        } else {
                            stack_emit(app, format!("[ ok ] {name}"));
                        }
                        // Auto-open the dashboard for services that declare it (gateway). Reuses the
                        // opener command (scheme-guarded), not a console/Start-Process.
                        if svc.get("openDashboard").and_then(|x| x.as_bool()).unwrap_or(false) {
                            if let Some(url) = svc
                                .get("dashboard")
                                .and_then(|x| x.as_str())
                                .filter(|u| !u.is_empty())
                            {
                                let _ = open_url(app.clone(), url.to_string());
                            }
                        }
                    }
                }
            }
            None => {
                failed += 1;
                stack_emit(app, format!("[fail] {name}: could not spawn (check command)"));
                let wants_teardown = svc.get("teardownOnFailure").and_then(|x| x.as_bool()).unwrap_or(false);
                if should_teardown(wants_teardown) {
                    teardown_started(app, procs, &name, &started_pids);
                    teardown_fired = true;
                    break;
                }
            }
        }
    }
    if matched == 0 {
        stack_emit(app, "Nothing to start (check the service id / enabled flags).".into());
    } else if cancelled {
        // A Stop preempted this start — don't claim "Started N". The [cancel]/[teardown] lines above
        // already reported the specifics (mid-wait services were torn down; a Stop reaps the rest).
        stack_emit(app, "Start cancelled by a stop request.".into());
    } else if teardown_fired {
        // Don't claim "Started N" — those N were just rolled back. The [teardown] line already said what.
        stack_emit(app, "Stack rolled back — a teardown-on-failure service did not come up.".into());
    } else {
        stack_emit(app, format!("Started {started} service(s)."));
    }
    // Honest exit: a service that failed to come up must NOT surface as a green success (outcome.ts
    // maps code!=0 → error toast). Skips (disabled / requires-missing / already-up) aren't failures.
    let code = if failed > 0 { 1 } else { 0 };
    let _ = app.emit(
        "run-done",
        RunDone {
            component: stream_id::ENGINE.to_string(),
            code,
        },
    );
    code
}

/// Image name of a pid via `tasklist` (hidden), lowercased. None if not found. Guards the stop
/// port→pid fallback so we only kill our own service processes (node/python), never a foreign app
/// that merely happens to hold a manifest port. Mirrors stop-stack.ps1's `$ours` name check.
fn pid_image_name(pid: u32) -> Option<String> {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // CSV: "image.exe","1234",...  → the first quoted field on a data line.
    let line = text.lines().find(|l| l.starts_with('"'))?;
    let name = line.trim_start_matches('"').split('"').next()?;
    Some(name.to_ascii_lowercase())
}

fn is_ours_process(name: &str) -> bool {
    // node/python = the actual service processes (hold the port); cmd = our `cmd /c` wrapper, which
    // is the pid we track for kill_tree. All three are things we legitimately spawn.
    name.starts_with("node")
        || name.starts_with("python")
        || name.starts_with("py")
        || name.starts_with("cmd")
}

/// Whether a Stop should flip the global `STACK_CANCEL` (which aborts an in-flight native start).
/// Only a stop-ALL should — a targeted single-service stop must leave a concurrent full start of the
/// OTHER services running (CAST-3). Pure so the cancel scope has a real assert behind it.
fn stop_aborts_start(only: Option<&str>) -> bool {
    only.is_none()
}

/// Native STOP: kill each target service by its tracked top-of-tree pid (persisted, so it survives an
/// app restart), with a port→pid fallback (ownership-guarded) for anything we didn't spawn. kill_tree
/// = taskkill /T so the whole npm→node tree dies and no window is orphaned.
async fn native_stack_stop(
    app: &AppHandle,
    procs: &StackProcs,
    only: Option<&str>,
    emit_done: bool,
) -> i32 {
    // Only a stop-all aborts a concurrent full start; a single-service stop leaves it running (CAST-3).
    if stop_aborts_start(only) {
        STACK_CANCEL.store(true, Ordering::SeqCst);
    }
    if let Some(o) = only {
        if !stack_services()
            .iter()
            .any(|s| s.get("id").and_then(|x| x.as_str()) == Some(o))
        {
            stack_emit(app, format!("[warn] unknown service id: {o}"));
        }
    }
    let mut tracked = load_stack_procs();
    {
        let m = procs.0.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in m.iter() {
            tracked.insert(k.clone(), *v);
        }
    }
    let by_port = listening_pids();
    let mut stopped = 0;
    for svc in stack_services() {
        let sid = svc.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if let Some(o) = only {
            if sid != o {
                continue;
            }
        }
        // We are intentionally taking this service down — suppress the health poll's "down" alert
        // for the TTL window (covers stop-one, stop-all, and the restart stop phase, all via here).
        mark_expected_down(&sid);
        let name = svc
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(&sid)
            .to_string();
        let port = svc.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
        let mut killed = false;
        if let Some(pid) = tracked.get(&sid) {
            // Guard the persisted pid: after a restart `tracked` comes from disk (stack-procs.json)
            // and Windows may have reused it. Our tracked pid is always the `cmd /c` wrapper, so
            // require the live image to still be ours before force-killing its whole tree.
            // ponytail: residual = a reused pid that itself became cmd/node/python; narrow, accepted.
            if *pid != 0
                && pid_image_name(*pid).map(|n| is_ours_process(&n)).unwrap_or(false)
                && kill_tree(*pid).is_ok()
            {
                killed = true;
            }
        }
        // Port fallback ONLY when the tracked-pid kill didn't already succeed (CAST-4): a service
        // started outside the app / with a stale pid. Guarded so we never kill a foreign app that now
        // holds a manifest port after our own process already died.
        if !killed && port != 0 {
            if let Some(pid) = by_port.get(&port) {
                if pid_image_name(*pid).map(|n| is_ours_process(&n)).unwrap_or(false) {
                    let _ = kill_tree(*pid);
                    killed = true;
                }
            }
        }
        if killed {
            stopped += 1;
            stack_emit(app, format!("[stop] {name}"));
        } else {
            stack_emit(app, format!("[ -- ] {name}: not running"));
        }
        tracked.remove(&sid);
        procs
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&sid);
    }
    // Persist the remaining known pids (disk-minus-stopped), NOT just the in-memory map — a
    // single-service stop after a restart must not wipe the OTHER services' persisted pids.
    save_stack_procs(&tracked);
    stack_emit(app, format!("Stopped {stopped} service(s)."));
    // In a restart the start phase emits the final run-done; suppress the stop's to avoid a double
    // run-done (double toast) on the ENGINE stream.
    if emit_done {
        let _ = app.emit(
            "run-done",
            RunDone {
                component: stream_id::ENGINE.to_string(),
                code: 0,
            },
        );
    }
    0
}

/// Start or stop the LLM stack. With no `only`, acts on the whole stack (start `-Router` includes
/// the paid GLM router on :4000; stop `-All`). With `only=<service id>`, acts on that one service
/// via the launchers' `-Only` switch. Streamed via pwsh.
#[tauri::command]
async fn run_stack(
    app: AppHandle,
    stack: State<'_, StackRun>,
    procs: State<'_, StackProcs>,
    action: String,
    only: Option<String>,
) -> Result<i32, String> {
    let only = only.filter(|s| !s.is_empty());
    if let Some(id) = &only {
        if !valid_stack_id(id) {
            return Err(trv("err.invalid_service_id", cur_lang(), &[("id", &id)]));
        }
    }
    let id = stream_id::ENGINE;
    // Native supervisor (DEFAULT) vs the legacy PS launcher scripts (opt-out via stackNative=false).
    // Both run under the same StackRun slot so start/stop concurrency + Stop's preempt are identical.
    let native = read_config_file().stack_native.unwrap_or(true);
    let o = only.as_deref();

    match action.as_str() {
        // Stop PREEMPTS any in-flight start (kills its tree), then tears down. Recovery must never be
        // blocked by a running start — that was the "can't stop while starting" bug.
        "stop" => {
            let slot = StackSlot::reserve_preempt(stack.inner());
            let code = if native {
                native_stack_stop(&app, procs.inner(), o, true).await
            } else {
                // Script path stops too: suppress the health poll's "down" alert for these ids,
                // same as the native path does per-service (review-w1 M1 — a deliberate Stop in
                // fallback mode must not OS-notify "service down").
                mark_expected_down_scope(o);
                let args = match &only {
                    Some(id) => vec!["-Only".to_string(), id.clone()],
                    None => vec!["-All".to_string()],
                };
                spawn_stack_phase(&app, &slot, id, abs(STACK_STOP_REL), args, "run-done").await
            };
            drop(slot);
            Ok(code)
        }
        "start" => {
            let slot = StackSlot::reserve(stack.inner())?;
            let code = if native {
                native_stack_start(&app, procs.inner(), o).await
            } else {
                let args = match &only {
                    Some(id) => vec!["-Only".to_string(), id.clone()],
                    None => vec!["-Router".to_string()],
                };
                spawn_stack_phase(&app, &slot, id, abs(STACK_START_REL), args, "run-done").await
            };
            drop(slot);
            Ok(code)
        }
        // Restart = stop then start under ONE stack slot. A second restart/start while one runs is
        // rejected (only Stop preempts). The script path's stop phase emits an event the UI ignores.
        "restart" => {
            let slot = StackSlot::reserve(stack.inner())?;
            let code = if native {
                native_stack_stop(&app, procs.inner(), o, false).await;
                native_stack_start(&app, procs.inner(), o).await
            } else {
                // The script restart's stop phase takes services down on purpose — suppress the
                // health poll's alert exactly like the plain stop branch above.
                mark_expected_down_scope(o);
                let (stop_args, start_args) = match &only {
                    Some(id) => (
                        vec!["-Only".to_string(), id.clone()],
                        vec!["-Only".to_string(), id.clone()],
                    ),
                    None => (vec!["-All".to_string()], vec!["-Router".to_string()]),
                };
                let _ = spawn_stack_phase(
                    &app,
                    &slot,
                    id,
                    abs(STACK_STOP_REL),
                    stop_args,
                    "run-restart-stop",
                )
                .await;
                spawn_stack_phase(&app, &slot, id, abs(STACK_START_REL), start_args, "run-done").await
            };
            drop(slot);
            Ok(code)
        }
        _ => Err(trv(
            "err.unknown_stack_action",
            cur_lang(),
            &[("action", &action)],
        )),
    }
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
    // Read headroom: a healthy endpoint can legitimately be slow — OmniRoute's /v1/models measures
    // ~650-690ms cold (it enumerates every backend model). A 700ms read timeout straddled that, so a
    // WORKING service intermittently timed out and read as `down`, flapping the stack-health monitor
    // into notification spam. 2.5s gives real headroom; a genuinely hung service is still caught well
    // within the 30s poll. (The stack-health alarm also has a 2-tick hysteresis as a second guard.)
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(2500)));
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
pub(crate) struct StackHealth {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: String,
    pub(crate) port: u16,
    pub(crate) enabled: bool,
    /// TCP port accepts a connection.
    pub(crate) port_open: bool,
    /// HTTP health endpoint returned 2xx. None when the service has no `health` path (port-only).
    pub(crate) healthy: Option<bool>,
    /// This service is the client-facing front — its outage is the overall alarm (data-driven,
    /// replaces the old hardcoded `id == "gateway"`). From stack.json `critical` (default false).
    pub(crate) critical: bool,
}

/// U1: absolute path to a stack service's log file, if it exists. A failed service tells the user to
/// "see stack-logs\<id>.log"; the UI's "Open log" button resolves the path here and opens it.
#[tauri::command]
fn stack_log_path(id: String) -> Option<String> {
    let p = stack_log_dir()?.join(format!("{id}.log"));
    if p.exists() {
        Some(p.to_string_lossy().into_owned())
    } else {
        None
    }
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

/// Is any stack service enabled? Reads `stack.json` only — no ports touched. The health monitor asks
/// this before probing, so a user who never configured the stack pays nothing for it every 30 s.
/// Re-read each tick, so enabling a service takes effect without an app restart.
pub(crate) fn any_stack_service_enabled() -> bool {
    stack_services()
        .iter()
        .any(|e| e.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true))
}

pub(crate) fn read_stack_health_blocking() -> Vec<StackHealth> {
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
        critical: bool,
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
            critical: e.get("critical").and_then(|x| x.as_bool()).unwrap_or(false),
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
            critical: r.critical,
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
    // Fallback: kill whatever WE spawned on the engine's port. Filter the listeners through the same
    // ownership guard native_stack_stop uses (image name node/python/cmd), so a foreign process that
    // merely happens to hold the port is never force-killed; `/T` takes the npm->node child tree, not
    // just the top pid.
    let ours: Vec<u32> = listeners_on_port(cfg.port)
        .into_iter()
        .filter(|&pid| pid_image_name(pid).map(|n| is_ours_process(&n)).unwrap_or(false))
        .collect();
    if ours.is_empty() {
        return Ok(0); // nobody of ours listening — already stopped
    }
    let mut args: Vec<String> = vec!["/F".into(), "/T".into()];
    for pid in ours {
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
    run_native_streamed(app, state, stream_id::ENGINE.to_string(), move |out, err| {
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
    run_native_streamed(app, state, stream_id::PROVIDER.to_string(), move |out, err| {
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
        .kill_on_drop(true) // on timeout the future is dropped — reap the child instead of orphaning gh
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

/// Parse a profile name out of a `.claude-<name>` home-directory name: the suffix, when it is a
/// valid profile name. `.claude`, non-`.claude-` dirs, and invalid suffixes → None.
fn profile_name_from_dir(dir_name: &str) -> Option<String> {
    dir_name
        .strip_prefix(".claude-")
        .filter(|s| valid_profile_name(s))
        .map(String::from)
}

/// Profile names from config\profiles.json. Fallback (fresh OSS user with no profiles.json) scans
/// %USERPROFILE% for `.claude-<name>` dirs and returns those — NEVER the owner's hardcoded
/// PROFILE_NAMES, which would phantom-list profiles the user never created. Empty → empty Vec.
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
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| profile_name_from_dir(&e.file_name().to_string_lossy()))
        .collect();
    names.sort();
    names
}

/// One profile's provider + proxy, parsed from its settings.json `env`. The token VALUE is never
/// carried — only `has_token`. Shared by `read_providers` and `read_profile_matrix` (no copy-paste).
#[derive(Default)]
struct ProfileEnv {
    base_url: String,
    model: String,
    small_model: String,
    has_token: bool,
    proxy: String,
}

/// Read `~/.claude-<name>/settings.json` → its provider/proxy env. Unreadable/absent → all empty.
fn read_profile_env(home: &str, name: &str) -> ProfileEnv {
    let mut pe = ProfileEnv::default();
    let path = format!("{home}\\.claude-{name}\\settings.json");
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
                pe.base_url = g("ANTHROPIC_BASE_URL");
                pe.model = g_or("ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_MODEL");
                pe.small_model = g_or("ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_SMALL_FAST_MODEL");
                pe.has_token = !g("ANTHROPIC_AUTH_TOKEN").is_empty();
                pe.proxy = g("HTTPS_PROXY");
            }
        }
    }
    pe
}

/// Per-profile provider, read natively from each settings.json env.
/// The token VALUE is never returned — only `hasToken`.
#[tauri::command]
fn read_providers() -> Vec<ProfileProvider> {
    let home = match std::env::var("USERPROFILE") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    profile_names()
        .into_iter()
        .map(|name| {
            let e = read_profile_env(&home, &name);
            ProfileProvider {
                name,
                base_url: e.base_url,
                model: e.model,
                small_model: e.small_model,
                has_token: e.has_token,
            }
        })
        .collect()
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

/// A settings.json read for a surgical edit: missing or truly empty file → `{}` (fresh start), but
/// an unreadable (e.g. non-UTF-8) or unparsable file is an ERROR — treating it as empty would
/// clobber the profile's real settings on the follow-up atomic write (secret files get no .bak).
fn read_settings_for_edit(path: &str) -> Result<serde_json::Value, String> {
    match std::fs::read_to_string(path) {
        Ok(ref c) if !c.trim().is_empty() => {
            parse_json_bom(c).map_err(|e| format!("parse settings: {e}"))
        }
        Ok(_) => Ok(serde_json::json!({})),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(format!("read settings: {e}")),
    }
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
    let mut settings: Value = match read_settings_for_edit(settings_path) {
        Ok(v) => v,
        Err(e) => {
            err(&trv("log.read_settings", cur_lang(), &[("e", &e)]));
            return 1;
        }
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

// ── Ф2.5: per-profile matrix (Profiles tab) ────────────────────────────────────────────────────
// Rows = profiles; columns = provider / proxy / shared-folder links. Read here; batch apply is the
// frontend calling the setters below sequentially, then re-reading read_profile_matrix to verify.

#[derive(Serialize)]
struct MatrixProvider {
    #[serde(rename = "baseUrl")]
    base_url: String,
    model: String,
    #[serde(rename = "smallModel")]
    small_model: String,
    #[serde(rename = "hasToken")]
    has_token: bool,
}

#[derive(Serialize)]
struct MatrixFolder {
    name: String,
    /// Should this profile link the folder (its `linkedFolders` membership; absent field → all).
    desired: bool,
    /// On-disk reality at `~/.claude-<name>/<folder>`: "linked" | "real" | "missing".
    actual: String,
}

/// One plugin's state in a profile: "on" (explicit true), "off" (explicit false), "unset" (absent).
#[derive(Serialize, Clone)]
struct MatrixPlugin {
    id: String,
    state: String,
}

/// A profile's MCP picture vs the canonical config/.mcp.json server set.
#[derive(Serialize, Clone)]
struct MatrixMcp {
    /// Canonical server names (config/.mcp.json). Same for every row.
    canon: Vec<String>,
    /// Deployed servers that ARE canonical (deployed ∩ canon).
    deployed: Vec<String>,
    /// Deployed servers NOT in canon (deployed − canon).
    extras: Vec<String>,
}

#[derive(Serialize)]
struct MatrixRow {
    name: String,
    color: String,
    description: String,
    provider: MatrixProvider,
    proxy: String,
    folders: Vec<MatrixFolder>,
    plugins: Vec<MatrixPlugin>,
    mcp: MatrixMcp,
}

/// Classify one plugin's `enabledPlugins` value: explicit true → "on", explicit false → "off",
/// absent (or non-bool) → "unset". Pure so it is unit-testable.
fn plugin_state(v: Option<&serde_json::Value>) -> &'static str {
    match v.and_then(|x| x.as_bool()) {
        Some(true) => "on",
        Some(false) => "off",
        None => "unset",
    }
}

/// Sort + dedup a bag of keys into the shared plugin universe (stable column order across rows,
/// so the N/M chip denominator is identical for every profile). Pure.
fn union_sorted(mut keys: Vec<String>) -> Vec<String> {
    keys.sort();
    keys.dedup();
    keys
}

/// Split a profile's deployed MCP servers against the canonical set: (deployed ∩ canon, deployed − canon).
/// Pure.
fn mcp_split(canon: &[String], deployed_all: &[String]) -> (Vec<String>, Vec<String>) {
    let deployed = deployed_all
        .iter()
        .filter(|s| canon.iter().any(|c| c == *s))
        .cloned()
        .collect();
    let extras = deployed_all
        .iter()
        .filter(|s| !canon.iter().any(|c| c == *s))
        .cloned()
        .collect();
    (deployed, extras)
}

/// Upsert a settings.json `enabledPlugins` map in place: `enable` → true, `disable` → false. Every
/// other key (and every other top-level field) is preserved; a missing `enabledPlugins` object is
/// created. Never deletes a key — an explicit `false` is a per-profile opt-out (matches plugin_sync's
/// union semantics). Pure (Value in/out) → testable.
fn upsert_enabled_plugins(settings: &mut serde_json::Value, enable: &[String], disable: &[String]) {
    use serde_json::json;
    if !settings.is_object() {
        *settings = json!({});
    }
    let obj = settings.as_object_mut().unwrap();
    let ep = obj.entry("enabledPlugins").or_insert_with(|| json!({}));
    if !ep.is_object() {
        *ep = json!({});
    }
    let m = ep.as_object_mut().unwrap();
    for id in enable {
        m.insert(id.clone(), json!(true));
    }
    for id in disable {
        m.insert(id.clone(), json!(false));
    }
}

/// A settings.json's `enabledPlugins` object (id → bool). Unreadable/absent → empty map.
fn read_enabled_plugins_at(path: &str) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|v| {
            v.get("enabledPlugins")
                .and_then(|e| e.as_object())
                .cloned()
        })
        .unwrap_or_default()
}

/// Canonical MCP server names from config/.mcp.json (mirrors `read_mcp`'s source read). None when
/// the file is missing/unparseable or lacks an mcpServers object — callers must NOT read that as
/// "empty canon, nothing to reconcile" (that fail-open painted a broken canon green).
fn mcp_canon_servers() -> Option<Vec<String>> {
    std::fs::read_to_string(abs(MCP_CONFIG_REL))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|v| {
            v.get("mcpServers")
                .and_then(|m| m.as_object())
                .map(|o| o.keys().cloned().collect())
        })
}

/// Canon servers Deploy-Mcp.ps1 SKIPS because the plugin marketplace provides them (never in a
/// profile's user-scope mcpServers). Must stay in sync with `$skip` in Deploy-Mcp.ps1 — comparing
/// against the full canon flags these as eternally "missing" and the deploy fix never converges.
const MCP_PLUGIN_PROVIDED: [&str; 2] = ["context7", "serena"];

/// The canon servers a user-scope deploy is actually expected to place (canon minus plugin-provided).
/// None propagates "canon unreadable" from `mcp_canon_servers`.
fn mcp_deployable_canon() -> Option<Vec<String>> {
    mcp_canon_servers().map(|v| {
        v.into_iter()
            .filter(|n| !MCP_PLUGIN_PROVIDED.contains(&n.as_str()))
            .collect()
    })
}

/// `sharedFoldersDefault` (canonical column order) from a parsed profiles.json.
fn shared_folders_default(cfg: &serde_json::Value) -> Vec<String> {
    cfg.get("sharedFoldersDefault")
        .and_then(|s| s.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// Desired = folder is in the profile's linkedFolders. A MISSING linkedFolders field means "all
/// defaults" (the schema default), so `None` → true. Pure (Value slice) so it is unit-testable.
fn folder_desired(linked: Option<&Vec<serde_json::Value>>, folder: &str) -> bool {
    match linked {
        Some(arr) => arr.iter().any(|x| x.as_str() == Some(folder)),
        None => true,
    }
}

/// Classify a shared-folder path: "linked" (reparse point — junction/symlink), "real" (exists but
/// is NOT a link → holds data), "missing" (absent). Stats the link itself (symlink_metadata), so a
/// junction/symlink reads as such rather than following through. A plain file that is not a symlink
/// reports "real" (hardlinks are indistinguishable here — acceptable, per spec).
fn classify_link(path: &std::path::Path) -> &'static str {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    match path.symlink_metadata() {
        Ok(m) => {
            if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                "linked"
            } else {
                "real"
            }
        }
        Err(_) => "missing",
    }
}

/// The per-profile matrix: provider + proxy + shared-folder link state. Never panics; an unreadable
/// settings.json yields an empty provider/proxy. Empty vec when profiles.json is absent.
#[tauri::command]
async fn read_profile_matrix() -> Result<Vec<MatrixRow>, String> {
    // Off the main/UI thread: this walks every profile × shared folder (a symlink stat each) plus
    // per-profile settings/plugins/MCP reads — tens-to-hundreds of syscalls on a tab open. Mirrors
    // read_stack's spawn_blocking house style.
    tokio::task::spawn_blocking(read_profile_matrix_blocking)
        .await
        .map_err(|e| format!("read_profile_matrix task failed: {e}"))?
}

fn read_profile_matrix_blocking() -> Result<Vec<MatrixRow>, String> {
    let home = std::env::var("USERPROFILE").map_err(|_| "no USERPROFILE".to_string())?;
    let Some(cfg) = read_profiles_config()? else {
        return Ok(Vec::new());
    };
    let defaults = shared_folders_default(&cfg);
    let empty = Vec::new();
    let profiles = cfg
        .get("profiles")
        .and_then(|p| p.as_array())
        .unwrap_or(&empty);

    // Plugin universe (one column set shared by every row so the N/M chip denominator is stable):
    // installed_plugins.json keys ∪ enabledPlugins keys of base ~/.claude ∪ each profile's settings.
    // Cache each profile's enabledPlugins map to avoid a second read in the row loop.
    let mut universe_keys: Vec<String> = Vec::new();
    if let Some((_, installed, _)) = load_installed_plugins() {
        if let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) {
            universe_keys.extend(po.keys().cloned());
        }
    }
    universe_keys.extend(
        read_enabled_plugins_at(&format!("{home}\\.claude\\settings.json"))
            .keys()
            .cloned(),
    );
    let mut ep_by_profile: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>> =
        std::collections::HashMap::new();
    for p in profiles {
        if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
            if name.is_empty() {
                continue;
            }
            let m = read_enabled_plugins_at(&format!("{home}\\.claude-{name}\\settings.json"));
            universe_keys.extend(m.keys().cloned());
            ep_by_profile.insert(name.to_string(), m);
        }
    }
    let universe = union_sorted(universe_keys);
    // None (canon unreadable) → neutral MCP cells (no canon, no extras noise); the onboarding
    // checklist carries the visible "unknown" signal for a broken .mcp.json.
    let mcp_canon_opt = mcp_deployable_canon();
    let mcp_canon = mcp_canon_opt.clone().unwrap_or_default();

    let mut rows = Vec::new();
    for p in profiles {
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let e = read_profile_env(&home, name);
        let linked = p.get("linkedFolders").and_then(|l| l.as_array());
        let profile_dir = std::path::Path::new(&home).join(format!(".claude-{name}"));
        let folders = defaults
            .iter()
            .map(|folder| MatrixFolder {
                name: folder.clone(),
                desired: folder_desired(linked, folder),
                actual: classify_link(&profile_dir.join(folder)).to_string(),
            })
            .collect();
        let ep = ep_by_profile.get(name);
        let plugins = universe
            .iter()
            .map(|id| MatrixPlugin {
                id: id.clone(),
                state: plugin_state(ep.and_then(|m| m.get(id))).to_string(),
            })
            .collect();
        let deployed_all = profile_mcp_servers(name).unwrap_or_default();
        let (deployed, extras) = if mcp_canon_opt.is_some() {
            mcp_split(&mcp_canon, &deployed_all)
        } else {
            (Vec::new(), Vec::new())
        };
        rows.push(MatrixRow {
            name: name.to_string(),
            color: p.get("color").and_then(|c| c.as_str()).unwrap_or("").to_string(),
            description: p
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            provider: MatrixProvider {
                base_url: e.base_url,
                model: e.model,
                small_model: e.small_model,
                has_token: e.has_token,
            },
            proxy: e.proxy,
            folders,
            plugins,
            mcp: MatrixMcp {
                canon: mcp_canon.clone(),
                deployed,
                extras,
            },
        });
    }
    Ok(rows)
}

/// Merge HTTP_PROXY/HTTPS_PROXY into a profile settings.json `env`. Empty `url` → remove both;
/// non-empty → set both. Every other key is preserved; an emptied `env` block is dropped. Atomic,
/// no-BOM write. Explicit path (no USERPROFILE coupling) so it is unit-testable (mirrors
/// `apply_provider_env`).
fn apply_proxy_env(settings_path: &str, url: &str) -> Result<(), String> {
    use serde_json::{json, Value};
    let mut settings: Value = read_settings_for_edit(settings_path)?;
    if !settings.is_object() {
        settings = json!({});
    }
    let sobj = settings.as_object_mut().unwrap();
    if !sobj.get("env").map(|e| e.is_object()).unwrap_or(false) {
        sobj.insert("env".into(), json!({}));
    }
    let env_empty = {
        let env = sobj.get_mut("env").unwrap().as_object_mut().unwrap();
        if url.is_empty() {
            env.remove("HTTP_PROXY");
            env.remove("HTTPS_PROXY");
        } else {
            env.insert("HTTP_PROXY".into(), json!(url));
            env.insert("HTTPS_PROXY".into(), json!(url));
        }
        env.is_empty()
    };
    if env_empty {
        sobj.remove("env");
    }
    let serialized =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("serialize settings: {e}"))?;
    write_json_atomic(settings_path, &serialized).map_err(|e| format!("write settings: {e}"))
}

/// Set (or clear, when `url` empty) a profile's HTTP(S)_PROXY. Guards the live-session footgun like
/// `manage_provider_native`. Non-empty url must use an http/https/socks5 scheme.
#[tauri::command]
fn set_profile_proxy(name: String, url: String) -> Result<(), String> {
    if !valid_profile_name(&name) {
        return Err(trv("err.invalid_profile_name", cur_lang(), &[("name", &name)]));
    }
    let known = profile_names();
    if !known.iter().any(|n| n == &name) {
        return Err(trv(
            "log.profile_not_found",
            cur_lang(),
            &[("name", &name), ("known", &known.join(", "))],
        ));
    }
    if profile_session_active(&name) {
        return Err(trv("log.profile_running_warn", cur_lang(), &[("name", &name)]));
    }
    let scheme_ok = url.is_empty()
        || url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("socks5://");
    if !scheme_ok {
        // ponytail: plain-English validation error — i18n.rs (the tr/trv catalog) is outside this
        // agent's edit zone; add an err.* key later if this surfaces in the UI often.
        return Err("proxy URL must start with http://, https:// or socks5://".to_string());
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let settings_path = format!("{home}\\.claude-{name}\\settings.json");
    apply_proxy_env(&settings_path, &url)
}

/// Surgically set ONE profile's `linkedFolders` in a parsed profiles.json, preserving every other
/// profile and all top-level keys. Errs if the profile is absent. Pure (Value in/out) → testable.
fn set_linked_folders(
    cfg: &serde_json::Value,
    name: &str,
    folders: &[String],
) -> Result<serde_json::Value, String> {
    let mut out = cfg.clone();
    let arr = out
        .get_mut("profiles")
        .and_then(|p| p.as_array_mut())
        .ok_or_else(|| "profiles.json: no profiles array".to_string())?;
    let prof = arr
        .iter_mut()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
        .ok_or_else(|| format!("profile not found: {name}"))?;
    let obj = prof
        .as_object_mut()
        .ok_or_else(|| "profile entry is not an object".to_string())?;
    obj.insert("linkedFolders".into(), serde_json::json!(folders));
    Ok(out)
}

/// Outcome of trying to detach one shared-folder link.
enum LinkOutcome {
    /// The reparse point (junction/symlink) was removed — the target data is untouched.
    Removed,
    /// Path was absent — nothing to do.
    Absent,
    /// Path exists but is NOT a reparse point (real data) OR the remove failed — left in place.
    Kept,
}

/// Detach the shared-folder link at `path` — ONLY if it is a reparse point. `fs::remove_dir` on a
/// junction/dir-symlink deletes the link, not the tree it points at; a file symlink → remove_file.
/// A real (non-link) file/dir holding data is NEVER touched (returns `Kept`).
fn remove_link_if_reparse(path: &std::path::Path) -> LinkOutcome {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    let Ok(m) = path.symlink_metadata() else {
        return LinkOutcome::Absent;
    };
    let attrs = m.file_attributes();
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return LinkOutcome::Kept; // real data — refuse to delete
    }
    let res = if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    };
    match res {
        Ok(_) => LinkOutcome::Removed,
        Err(_) => LinkOutcome::Kept, // couldn't remove → report as not-detached
    }
}

/// Set which shared folders a profile links: surgically rewrite its `linkedFolders` in profiles.json,
/// then NATIVELY detach any now-unwanted links (reparse points only). Creating links is the relink
/// script's job (the frontend runs `run_profile_relink` after this). Every `folders` entry must be a
/// member of `sharedFoldersDefault`. Returns the folders that were LEFT because they hold real data
/// (or could not be detached) — empty = clean.
#[tauri::command]
fn set_profile_folders(name: String, folders: Vec<String>) -> Result<Vec<String>, String> {
    if !valid_profile_name(&name) {
        return Err(trv("err.invalid_profile_name", cur_lang(), &[("name", &name)]));
    }
    let Some(cfg) = read_profiles_config()? else {
        return Err("profiles.json not found".to_string());
    };
    let known = profile_names();
    if !known.iter().any(|n| n == &name) {
        return Err(trv(
            "log.profile_not_found",
            cur_lang(),
            &[("name", &name), ("known", &known.join(", "))],
        ));
    }
    // Same live-session footgun as provider/proxy: detaching a running profile's junctions is unsafe.
    if profile_session_active(&name) {
        return Err(trv("log.profile_running_warn", cur_lang(), &[("name", &name)]));
    }
    let defaults = shared_folders_default(&cfg);
    for f in &folders {
        if !defaults.iter().any(|d| d == f) {
            return Err(format!("unknown shared folder: {f}"));
        }
    }
    // 1. Surgically persist the new linkedFolders set.
    let updated = set_linked_folders(&cfg, &name, &folders)?;
    let serialized =
        serde_json::to_string_pretty(&updated).map_err(|e| format!("serialize profiles.json: {e}"))?;
    write_json_atomic(&abs(PROFILES_CONFIG_REL), &serialized)
        .map_err(|e| format!("write profiles.json: {e}"))?;
    // 2. Detach links no longer wanted (reparse points only; real data is kept + reported).
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let profile_dir = std::path::Path::new(&home).join(format!(".claude-{name}"));
    let mut kept = Vec::new();
    for folder in &defaults {
        if folders.iter().any(|f| f == folder) {
            continue; // still desired
        }
        if let LinkOutcome::Kept = remove_link_if_reparse(&profile_dir.join(folder)) {
            kept.push(folder.clone());
        }
    }
    Ok(kept)
}

/// Set a profile's per-profile plugin enablement: `enable` → explicit true, `disable` → explicit
/// false, in its `~/.claude-<name>/settings.json` `enabledPlugins`. Surgical — every other key and
/// the rest of the file are preserved; keys are NEVER removed (an explicit false is a per-profile
/// opt-out plugin_sync respects). Same live-session guard as provider/proxy/folders.
#[tauri::command]
fn set_profile_plugins(name: String, enable: Vec<String>, disable: Vec<String>) -> Result<(), String> {
    if !valid_profile_name(&name) {
        return Err(trv("err.invalid_profile_name", cur_lang(), &[("name", &name)]));
    }
    let known = profile_names();
    if !known.iter().any(|n| n == &name) {
        return Err(trv(
            "log.profile_not_found",
            cur_lang(),
            &[("name", &name), ("known", &known.join(", "))],
        ));
    }
    if profile_session_active(&name) {
        return Err(trv("log.profile_running_warn", cur_lang(), &[("name", &name)]));
    }
    // Charset guard on plugin ids (name@marketplace) — mirrors run_plugin's.
    let id_ok = |id: &String| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
    };
    for id in enable.iter().chain(disable.iter()) {
        if !id_ok(id) {
            return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", id)]));
        }
    }
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let settings_path = format!("{home}\\.claude-{name}\\settings.json");
    let mut settings: serde_json::Value = read_settings_for_edit(&settings_path)?;
    upsert_enabled_plugins(&mut settings, &enable, &disable);
    let serialized =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("serialize settings: {e}"))?;
    write_json_atomic(&settings_path, &serialized).map_err(|e| format!("write settings: {e}"))?;
    invalidate_plugins_cache(); // P4: the enabled set changed — next open must re-read
    Ok(())
}

/// Recreate ONE profile's shared-folder links (Repair-ProfileLinks.ps1 -Name). The matrix-apply
/// contract the Profiles UI calls after `set_profile_folders`. Delegates to the existing "repair"
/// path (same script + validation) — no second spawn wiring.
#[tauri::command]
async fn run_profile_relink(
    app: AppHandle,
    state: State<'_, RunState>,
    name: String,
) -> Result<i32, String> {
    run_profiles(app, state, "repair".to_string(), Some(name)).await
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
    run_native_streamed(app, state, stream_id::PROVIDER.to_string(), move |out, err| {
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
const MYPROVIDERS_CONFIG_REL: &str = "{{PROFILES}}\\config\\myproviders.json";
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
    let legacy = keyring::Entry::new(&old, user).ok()?;
    let v = legacy.get_password().ok()?;
    // Drop the legacy copy once the secret is safely re-homed. Keeping it meant a later rotation
    // rewrote only `castellyn.*`, leaving the PRE-rotation secret readable under `agenthub.*`
    // indefinitely. `kr_delete` already purged both names — only this read path was inconsistent.
    // If the re-home failed, leave the legacy entry alone: it is the only copy left.
    if kr_set(service, user, &v).is_ok() {
        let _ = legacy.delete_credential();
    }
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
    // Scheme is case-insensitive per RFC 3986 — case-fold only for the prefix test, keep the original
    // host slice (host casing can matter to some backends).
    let rest = {
        let lower = s.to_ascii_lowercase();
        if lower.starts_with("http://") {
            &s[7..]
        } else if lower.starts_with("https://") {
            &s[8..]
        } else {
            return Err(tr("err.url_scheme", cur_lang()).into());
        }
    };
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
    // Update in place on edit (retain+push reordered the entry to the end on every save); append only
    // when creating a new provider.
    match list
        .iter()
        .position(|e| e.get("id").and_then(|x| x.as_str()) == Some(id.as_str()))
    {
        Some(idx) => list[idx] = entry.clone(),
        None => list.push(entry.clone()),
    }
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
    let new_count = survivors.len() as u64;
    // Keep the active key pointing at a valid slot: shift down if we removed at/below it.
    let new_active = if new_count == 0 {
        0
    } else if active >= index && active > 0 {
        (active - 1).min(new_count - 1)
    } else {
        active.min(new_count - 1)
    };
    // Persist myproviders.json FIRST (mirrors append_key_txn): a JSON-write failure must not have
    // already mutated the keyring — with this order the survivors stay in their original slots and
    // nothing is lost. Only after the JSON is durable do we compact the keyring, writing each survivor
    // to its new slot BEFORE deleting stale trailing slots, so a mid-rewrite failure still keeps every
    // survivor present.
    list[pos]["keyCount"] = serde_json::json!(new_count);
    list[pos]["activeKey"] = serde_json::json!(new_active);
    write_myproviders_raw(&list)?;
    for (i, k) in survivors.iter().enumerate() {
        kr_set(KR_PROVIDERS, &format!("provider:{id}:{i}"), k)?;
    }
    for i in new_count..count {
        kr_delete(KR_PROVIDERS, &format!("provider:{id}:{i}"));
    }
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

/// OmniRoute front base URL from the `omniroute` service port in stack.json. None if absent.
/// Parallel to `gateway_base_url` (freellmapi :13001) and deliberately distinct: `gateway` feeds
/// freellmapi-backend registration + Codex-freellmapi; `omniroute` feeds the single client front.
#[tauri::command]
fn omniroute_base_url() -> Option<String> {
    let port = stack_services()
        .iter()
        .find(|e| e.get("id").and_then(|x| x.as_str()) == Some("omniroute"))
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
        // No cross-host redirects: this is a single-shot API call carrying credentials; the SSRF
        // guard validates only the initial URL, so following a 3xx to an unvalidated host (e.g.
        // link-local metadata) is exactly what we must not do (V-12).
        .max_redirects(0)
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
            run_native_streamed(app, state, stream_id::PROVIDER.to_string(), move |out, err| {
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
            run_native_streamed(app, state, stream_id::PROVIDER.to_string(), move |out, err| {
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
    let old_active;
    {
        let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut list = read_myproviders_checked()?;
        let pos = find_provider_idx(&list, &id).ok_or(tr("err.provider_not_found", cur_lang()))?;
        let (count, active) = key_pool_meta(&list[pos]);
        if count < 2 {
            return Err(tr("err.single_key", cur_lang()).into());
        }
        old_active = active;
        let next = next_key_index(active, count);
        list[pos]["activeKey"] = serde_json::json!(next);
        write_myproviders_raw(&list)?;
    }
    // Re-bind the harness to the now-active key (reuses the full connect dispatch). If the connect
    // FAILS (hard error, or a non-zero exit = the bind never took), roll the pointer back — otherwise
    // the persisted `activeKey` drifts ahead of the key the harness is really on and a second "next"
    // click silently skips a key.
    let res = connect_my_provider(app, state, id.clone()).await;
    if !matches!(&res, Ok(0)) {
        let _guard = MYPROVIDERS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(mut list) = read_myproviders_checked() {
            if let Some(pos) = find_provider_idx(&list, &id) {
                list[pos]["activeKey"] = serde_json::json!(old_active);
                let _ = write_myproviders_raw(&list);
            }
        }
    }
    res
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
        .max_redirects(0) // single-shot API GET carrying a key — no cross-host redirects (V-12)
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
    // Some servers want an Authorization header even when no real key is needed.
    fetch_models_bearer(base_url, "Bearer not-needed")
}

/// GET {baseUrl}/v1/models with a caller-supplied Authorization header value; returns the model ids
/// (`data[].id`). Empty on any error / blocked URL. Shared by the keyless engine-model preview and the
/// keyed opencode-model picker (read_opencode_models).
fn fetch_models_bearer(base_url: &str, authorization: &str) -> Vec<String> {
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
        .max_redirects(0) // single-shot API GET carrying a key — no cross-host redirects (V-12)
        .build()
        .into();
    let body = match agent
        .get(&url)
        .header("Authorization", authorization)
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
        .or_else(|| v.get("models").and_then(|x| x.as_array())) // some APIs use `models[]` (matches count_models)
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
    // Prefer keys that mean "remaining balance"; treat quota fields as last-resort fallbacks. A hard
    // limit (e.g. hard_limit_usd) is deliberately NOT here — reporting a plan ceiling as the available
    // balance is misleading; the dedicated billing path labels that separately.
    for p in [
        "remaining",
        "balance",
        "data.balance",
        "balance_infos.0.total_balance",
        "total_balance",
        "data.quota",
        "quota",
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
        .max_redirects(0) // single-shot API GET carrying a key — no cross-host redirects (V-12)
        .build()
        .into();

    // 1) User-configured balance URL — most reliable. Guard with `probe_url_allowed` (not just
    // `valid_base_url`): this request carries the provider's key, so besides the SSRF/metadata
    // block it must also require https for non-loopback hosts — otherwise a plaintext http://
    // balanceUrl would leak the key on the wire, exactly what the liveness probe guards against.
    if !balance_url.is_empty() {
        if let Err(detail) = probe_url_allowed(balance_url) {
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
    // Fallbacks 2/3 derive their URL from `root` (= baseUrl) and also send the key, so the same
    // https-or-loopback guard applies. If baseUrl is a plaintext http:// non-loopback host, skip
    // the fallbacks rather than leak the key.
    if probe_url_allowed(root).is_err() {
        return serde_json::json!({ "ok": false, "detail": tr("det.balance_unavailable", cur_lang()) });
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
    /// Highest model-scoped cap from the API's limits[] (per-model weekly). Can exceed the headline
    /// 7d — then IT gates the profile, and the badge must not show only the calmer number.
    #[serde(rename = "scopedPct")]
    scoped_pct: Option<f64>,
    #[serde(rename = "scopedLabel")]
    scoped_label: Option<String>,
}

#[derive(Default)]
struct UsageCache(Mutex<std::collections::HashMap<String, (std::time::Instant, ProfileUsage)>>);

/// After a cache entry goes stale (>60s), a FAILED re-fetch serves the last-good value instead of
/// blanking the badge (flicker fix, live-smoke 2026-07-03) — but only until it is this old, so a
/// genuinely logged-out/removed profile eventually clears instead of showing forever-stale numbers.
const USAGE_STALE_MAX_SECS: u64 = 900; // 15 min

/// Blocking: read a profile's OAuth token and query the usage endpoint. None on any failure
/// (not logged in / token expired / offline) so the UI just omits the badge.
fn fetch_profile_usage(profile: &str) -> Result<ProfileUsage, u16> {
    let home = std::env::var("USERPROFILE").map_err(|_| 0u16)?;
    let creds = format!("{home}\\.claude-{profile}\\.credentials.json");
    // Same credentials path `limits::poll_profile` uses, so both share one cached request per profile
    // instead of each hitting Anthropic on its own cadence. No token at all reads as 401: there is
    // nothing to show and nothing to wait for, so the badge should clear rather than go stale.
    let resp = crate::limits::usage_cached(&creds).ok_or(401u16)??;
    let (five_hour_pct, five_hour_resets_at) = crate::limits::util_of(&resp, "five_hour");
    let (seven_day_pct, seven_day_resets_at) = crate::limits::util_of(&resp, "seven_day");
    let (scoped_pct, scoped_label, _scoped_reset) = crate::limits::scoped_max(&resp);
    Ok(ProfileUsage {
        five_hour_pct,
        seven_day_pct,
        five_hour_resets_at,
        seven_day_resets_at,
        scoped_pct,
        scoped_label,
    })
}

/// #4: force a fresh usage poll for one profile right now (bypassing the 5-min cache), so a pane that
/// just hit its limit gets an accurate reset time within seconds instead of up to POLL_SECS later. The
/// frontend throttles per profile (once per ~90s) so this can't storm the endpoint. Fire-and-forget —
/// it re-emits `limits-status` when the fresh numbers land. `profile` is only matched against the real
/// profile list, never used to build a path, so an unknown value is a safe no-op.
#[tauri::command]
async fn poll_limits_now(app: AppHandle, profile: String) {
    let _ = tokio::task::spawn_blocking(move || crate::limits::poll_profile_now(&app, &profile)).await;
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
        .unwrap_or(Err(0));
    let mut map = cache.0.lock().unwrap_or_else(|e| e.into_inner());
    match fetched {
        Ok(u) => {
            map.insert(profile, (std::time::Instant::now(), u.clone()));
            Ok(Some(u))
        }
        // Token gone or rejected: there is no live budget to show, so drop the cached copy instead of
        // serving numbers from an account the user can no longer reach. (Previously every error
        // collapsed to None and a revoked token kept showing its last figures for 15 minutes.)
        Err(401) => {
            map.remove(&profile);
            Ok(None)
        }
        // A transient re-fetch failure (429 / offline / a busy account under load) must NOT blank the
        // badge — that oscillation was the flicker (live-smoke 2026-07-03). Serve the last-good value
        // until it ages past USAGE_STALE_MAX_SECS, then a truly gone profile clears.
        Err(_) => Ok(map
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

/// Resolve an opencode-style `{env:VAR}` apiKey reference to the env var's value; a literal key (or a
/// non-ref string) passes through unchanged. Empty when the referenced var is unset.
fn resolve_env_ref(s: &str) -> String {
    match s.strip_prefix("{env:").and_then(|x| x.strip_suffix('}')) {
        Some(var) => std::env::var(var.trim()).unwrap_or_default(),
        None => s.to_string(),
    }
}

/// Available models across opencode's configured providers, as `"<providerId>/<model>"` — so the
/// launcher can offer a real `--model` picker instead of a blank field (owner live-smoke C). Resolves
/// each provider's apiKey (`{env:X}` → the env var, or a literal) and GETs `{baseURL}/v1/models` with
/// it. Best-effort: a provider that errors / 401s just contributes nothing. Read-only; the resolved key
/// value never leaves the backend (only the model ids are returned).
#[tauri::command]
async fn read_opencode_models() -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(opencode_config_path()) else {
        return Vec::new();
    };
    let Ok(v) = parse_json_bom(&content) else {
        return Vec::new();
    };
    // (id, baseUrl, resolvedKey) per provider that has a base URL.
    let mut targets: Vec<(String, String, String)> = Vec::new();
    if let Some(obj) = v.get("provider").and_then(|p| p.as_object()) {
        for (id, p) in obj {
            let opts = p.get("options");
            let base = opts
                .and_then(|o| o.get("baseURL"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if base.is_empty() {
                continue;
            }
            let key = resolve_env_ref(
                opts.and_then(|o| o.get("apiKey"))
                    .and_then(|x| x.as_str())
                    .unwrap_or(""),
            );
            targets.push((id.clone(), base.to_string(), key));
        }
    }
    tokio::task::spawn_blocking(move || {
        let mut out: Vec<String> = Vec::new();
        for (id, base, key) in targets {
            let auth = if key.is_empty() {
                "Bearer not-needed".to_string()
            } else {
                format!("Bearer {key}")
            };
            for m in fetch_models_bearer(&base, &auth) {
                out.push(format!("{id}/{m}"));
            }
        }
        out.sort();
        out.dedup();
        out
    })
    .await
    .unwrap_or_default()
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
    run_native_streamed(app, state, stream_id::PROVIDER.to_string(), move |out, err| {
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

const MCP_CONFIG_REL: &str = "{{PROFILES}}\\config\\.mcp.json";
const MCP_DEPLOY_SCRIPT_REL: &str = "{{PROFILES}}\\Deploy-Mcp.ps1";
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

const SCHEDULE_SCRIPT_REL: &str = "{{PROFILES}}\\Schedule-Hub.ps1";
const SCHEDULES_JSON_REL: &str = "{{PROFILES}}\\schedules.last.json";

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
    read_schedules_cached_inner()
}

/// File-only read of schedules.last.json (no pwsh refresh). Shared by the cached command and the
/// schedules watcher; same shape/errors as read_schedules minus the query.
pub(crate) fn read_schedules_cached_inner() -> Result<Option<serde_json::Value>, String> {
    read_json_opt(abs(SCHEDULES_JSON_REL), "schedules")
}

/// Read schedules.last.json without the pwsh query — for a fast HomeTab seed on mount.
#[tauri::command]
async fn read_schedules_cached() -> Result<Option<serde_json::Value>, String> {
    read_schedules_cached_inner()
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
    spawn_streamed(app, state, stream_id::SCHEDULE.to_string(), script, args).await
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
    spawn_streamed(app, state, stream_id::MCP.to_string(), script, args).await
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

/// A plugin's installation scope ("user" | "managed" | "project" | "local") from its first
/// installed_plugins.json entry. None = not installed / no scope recorded (old schema).
fn plugin_scope(id: &str) -> Option<String> {
    let (_, installed, _) = load_installed_plugins()?;
    installed
        .get("plugins")?
        .get(id)?
        .as_array()?
        .first()?
        .get("scope")?
        .as_str()
        .map(String::from)
}

/// The DEPLOYED managed-settings.json `enabledPlugins` verdict for a plugin:
/// Some(false) = blocked by policy (CC refuses user-scope enable), Some(true) = policy-enabled,
/// None = policy silent (or file unreadable — then CC has no policy either).
fn managed_plugin_policy(id: &str) -> Option<bool> {
    let path = deployed_managed_path()?;
    let v = parse_json_bom(&std::fs::read_to_string(path).ok()?).ok()?;
    v.get("enabledPlugins")?.get(id)?.as_bool()
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

/// P4: cache for list_plugins — a cold `claude plugin list` (pwsh spawn) is a visible pause on every
/// tab open. TTL-cached so a re-open within a minute is instant; invalidated after any mutation.
static PLUGINS_CACHE: std::sync::LazyLock<
    Mutex<Option<(std::time::Instant, serde_json::Value)>>,
> = std::sync::LazyLock::new(|| Mutex::new(None));

fn invalidate_plugins_cache() {
    *PLUGINS_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// List installed plugins via `claude plugin list --json`, enriched with descriptions from disk.
#[tauri::command]
async fn list_plugins() -> Result<serde_json::Value, String> {
    // P4: serve a fresh (<60s) cached result instantly; otherwise fall through to the real read.
    const PLUGINS_TTL: std::time::Duration = std::time::Duration::from_secs(60);
    {
        let guard = PLUGINS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((t, v)) = guard.as_ref() {
            if t.elapsed() < PLUGINS_TTL {
                return Ok(v.clone());
            }
        }
    }
    // Resolve claude to an absolute path and spawn it DIRECTLY (like manage_plugin_native) instead of
    // `pwsh -NoProfile -Command "claude …"`: the pwsh indirection relied on claude being on the
    // spawned shell's PATH, which a GUI-launched app can lack → the whole tab showed "0 plugins".
    let Some(claude) = exe_on_path("claude") else {
        return Err(trv(
            "err.claude_launch",
            cur_lang(),
            &[("e", &tr("log.claude_not_found", cur_lang()).to_string())],
        ));
    };
    let fut = tokio::process::Command::new(&claude)
        .args(["plugin", "list", "--json"])
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
    // Scope per plugin (update must target it) + managed enabledPlugins policy (an explicit false
    // means CC refuses user-scope enable — the UI shows a lock instead of a futile toggle).
    let mut scopes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some((_, installed, _)) = load_installed_plugins() {
        if let Some(po) = installed.get("plugins").and_then(|x| x.as_object()) {
            for (pid, arr) in po {
                if let Some(s) = arr
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|e| e.get("scope"))
                    .and_then(|x| x.as_str())
                {
                    scopes.insert(pid.clone(), s.to_string());
                }
            }
        }
    }
    let policy = deployed_managed_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|m| m.get("enabledPlugins").and_then(|e| e.as_object()).cloned())
        .unwrap_or_default();
    if let Some(arr) = v.as_array_mut() {
        for item in arr.iter_mut() {
            let id = item.get("id").and_then(|x| x.as_str()).map(String::from);
            if let (Some(id), Some(obj)) = (id, item.as_object_mut()) {
                if let Some(d) = desc.get(&id) {
                    obj.insert("description".into(), serde_json::json!(d));
                }
                let mp = id.rsplit('@').next().unwrap_or("");
                obj.insert("mine".into(), serde_json::json!(own.contains(mp)));
                if let Some(s) = scopes.get(&id) {
                    obj.insert("scope".into(), serde_json::json!(s));
                }
                if let Some(b) = policy.get(&id).and_then(|x| x.as_bool()) {
                    obj.insert("managedPolicy".into(), serde_json::json!(b));
                }
            }
        }
    }
    *PLUGINS_CACHE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((std::time::Instant::now(), v.clone()));
    Ok(v)
}

/// Codex profile names from `~/.codex/*.config.toml` files. Codex 0.142+ dropped `[profiles.*]`
/// tables in the base config — each profile is now a separate `<name>.config.toml`, selected via
/// `--profile <name>`. The base `config.toml` is skipped (its suffix is `config.toml`, not
/// `<name>.config.toml`, so `strip_suffix` leaves nothing).
#[tauri::command]
fn read_codex_profiles() -> Vec<String> {
    let Ok(home) = std::env::var("USERPROFILE") else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(format!("{home}\\.codex")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(profile) = name.strip_suffix(".config.toml") {
            if !profile.is_empty() {
                out.push(profile.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Remove a plugin's explicit `enabledPlugins` entry from the SOURCE managed-settings.json, so the
/// policy stops being opinionated about it and per-profile enable works again. Only the
/// version-controlled source is edited — the UI must chain `run_managed_deploy` (one UAC prompt)
/// to publish, mirroring how every managed change flows.
#[tauri::command]
fn unblock_managed_plugin(id: String) -> Result<(), String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
    {
        return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", &id)]));
    }
    let path = source_managed_path();
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read managed source: {e}"))?;
    let mut v = parse_json_bom(&text).map_err(|e| format!("parse managed source: {e}"))?;
    let removed = v
        .get_mut("enabledPlugins")
        .and_then(|ep| ep.as_object_mut())
        .map(|m| m.remove(&id).is_some())
        .unwrap_or(false);
    if !removed {
        return Err(trv("err.plugin_not_blocked", cur_lang(), &[("id", &id)]));
    }
    let serialized = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_json_atomic(&path, &serialized).map_err(|e| format!("write managed source: {e}"))?;
    invalidate_plugins_cache(); // P4: block policy changed — next open must re-read
    Ok(())
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
    // Strip a leading UTF-8 BOM first — it isn't White_Space, so trim_start() leaves it and the
    // `---` frontmatter fence then fails to match on a BOM-prefixed file.
    let t = content.trim_start_matches('\u{feff}').trim_start();
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
async fn list_skills() -> Vec<SkillInfo> {
    // Off the main/UI thread: walks the skills tree and reads a SKILL.md front-matter per skill.
    tokio::task::spawn_blocking(list_skills_blocking)
        .await
        .unwrap_or_default()
}

fn list_skills_blocking() -> Vec<SkillInfo> {
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
async fn read_environments() -> Vec<EnvInfo> {
    // Off the main/UI thread: unions skill sets across three harness roots (read_dir + SKILL.md probe per dir).
    tokio::task::spawn_blocking(read_environments_blocking)
        .await
        .unwrap_or_default()
}

fn read_environments_blocking() -> Vec<EnvInfo> {
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
    // Read the canonical MCP set once — used both for the Claude source count here and the canon-name
    // derivation below (was read_mcp() twice).
    let mcp = read_mcp();
    let claude_mcp = mcp.as_ref().map(|m| m.source.len()).unwrap_or(0);

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
    let canon_mcp: Vec<String> = mcp
        .as_ref()
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
async fn read_skill_matrix() -> Vec<SkillRow> {
    // Off the main/UI thread: unions skill sets across harness roots with per-dir SKILL.md probes.
    tokio::task::spawn_blocking(read_skill_matrix_blocking)
        .await
        .unwrap_or_default()
}

fn read_skill_matrix_blocking() -> Vec<SkillRow> {
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
async fn share_skills() -> Result<ShareResult, String> {
    // Off the async runtime: this spawns one `cmd /c mklink /J` subprocess per skill in a loop, which
    // would stall the UI as a synchronous command.
    tokio::task::spawn_blocking(share_skills_blocking)
        .await
        .map_err(|e| format!("share_skills task panicked: {e}"))?
}

fn share_skills_blocking() -> Result<ShareResult, String> {
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

/// Marks a SKILL.md we GENERATED from a slash-command (first body line). Lets a re-share refresh our
/// own wrappers in place and a future clean find them, without ever touching a real skill or junction.
const CMD_WRAPPER_SENTINEL: &str = "<!-- castellyn:command-skill -->";

/// Body of a command/skill markdown = everything after the first `---…---` frontmatter block (the
/// whole file if there is none). Mirrors `extract_frontmatter`'s fence logic.
fn md_body(content: &str) -> &str {
    let t = content.trim_start_matches('\u{feff}').trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            return rest[end + 4..].trim_start_matches(['\r', '\n']);
        }
    }
    content
}

/// (skill_name, description, body) for every "own" slash-command: the user's own-marketplace plugin
/// commands (e.g. `/max:rootcause` → `max-rootcause`) plus personal `~/.claude/commands`. Commands
/// have no SKILL.md of their own, so `share_commands` GENERATES a wrapper (unlike share_skills, which
/// junctions existing skill dirs). Third-party plugin commands are excluded — only YOURS.
fn command_sources(home: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    fn push_dir(dir: &std::path::Path, prefix: Option<&str>, out: &mut Vec<(String, String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(content) = std::fs::read_to_string(&p) else { continue };
            let desc = fm_value(&extract_frontmatter(&content), "description")
                .unwrap_or_else(|| stem.to_string());
            let name = match prefix {
                Some(pfx) => format!("{pfx}-{stem}"),
                None => stem.to_string(),
            };
            out.push((name, desc, md_body(&content).to_string()));
        }
    }
    push_dir(&std::path::Path::new(home).join(".claude").join("commands"), None, &mut out);
    if let Some((_dir, installed, markets)) = load_installed_plugins() {
        let own = own_marketplaces();
        if let Some(po) = installed.get("plugins").and_then(|v| v.as_object()) {
            for (id, arr) in po {
                let mp = id.rsplit('@').next().unwrap_or("");
                if !own.contains(mp) {
                    continue; // only YOUR commands, not every third-party plugin's
                }
                let plugin = id.split('@').next().unwrap_or(id);
                if let Some(dir) = plugin_content_dir(id, first_install_path(arr), &markets) {
                    push_dir(&std::path::Path::new(&dir).join("commands"), Some(plugin), &mut out);
                }
            }
        }
    }
    out
}

/// Generate a `SKILL.md` wrapper in ~/.agents/skills for every "own" slash-command, so Codex/OpenCode
/// can invoke them (`$max-rootcause`) — a command is a Claude-only concept, but a skill is the shared
/// cross-harness format. Idempotent: refreshes our own wrappers in place, never clobbers a real skill
/// or a share_skills junction of the same name. Claude is untouched (it reads ~/.claude, not ~/.agents).
#[tauri::command]
async fn share_commands() -> Result<ShareResult, String> {
    tokio::task::spawn_blocking(share_commands_blocking)
        .await
        .map_err(|e| format!("share_commands task panicked: {e}"))?
}

fn share_commands_blocking() -> Result<ShareResult, String> {
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
    for (name, desc, body) in command_sources(&home) {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            res.failed += 1;
            res.details.push(format!("{name}: unsafe name, skipped"));
            continue;
        }
        let skill_dir = std::path::Path::new(&target_dir).join(&name);
        let skill_md = skill_dir.join("SKILL.md");
        // Never clobber a real skill or a share_skills junction: only (re)write OUR OWN wrappers.
        if std::fs::symlink_metadata(&skill_dir).is_ok() {
            let is_ours = std::fs::read_to_string(&skill_md)
                .map(|c| c.contains(CMD_WRAPPER_SENTINEL))
                .unwrap_or(false);
            if !is_ours {
                res.skipped += 1; // a genuine skill / junction of the same name wins
                continue;
            }
        }
        // description is a single-line double-quoted YAML scalar — escape backslash and quote.
        let desc_esc = desc.replace('\\', "\\\\").replace('"', "\\\"");
        let contents =
            format!("---\nname: {name}\ndescription: \"{desc_esc}\"\n---\n{CMD_WRAPPER_SENTINEL}\n{body}\n");
        if let Err(e) = std::fs::create_dir_all(&skill_dir) {
            res.failed += 1;
            res.details.push(format!("{name}: {e}"));
            continue;
        }
        match std::fs::write(&skill_md, contents) {
            Ok(_) => res.created += 1,
            Err(e) => {
                res.failed += 1;
                res.details.push(format!("{name}: {e}"));
            }
        }
    }
    Ok(res)
}

#[cfg(test)]
mod command_share_tests {
    use super::{extract_frontmatter, fm_value, md_body};

    #[test]
    fn md_body_strips_frontmatter() {
        let c = "---\ndescription: \"x\"\nargument-hint: \"[y]\"\n---\n\n# Body\nline";
        assert_eq!(md_body(c), "# Body\nline");
    }

    #[test]
    fn md_body_passthrough_without_frontmatter() {
        assert_eq!(md_body("no fm here"), "no fm here");
    }

    #[test]
    fn md_body_tolerates_bom_and_crlf() {
        assert_eq!(md_body("\u{feff}---\r\ndescription: \"x\"\r\n---\r\nBody"), "Body");
    }

    #[test]
    fn frontmatter_description_is_parsed_for_the_wrapper() {
        let fm = extract_frontmatter("---\ndescription: \"Diagnose the root cause\"\n---\nbody");
        assert_eq!(fm_value(&fm, "description").as_deref(), Some("Diagnose the root cause"));
    }
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

/// Load opencode.json for an in-place edit: the parsed root object with `$schema` ensured, or a
/// localized error when the file is missing / empty / not a JSON object. Shared preamble of the three
/// opencode fan-out commands (mcp servers, providers, instructions); each then ensures its own block.
fn load_opencode_cfg_for_edit() -> Result<serde_json::Value, String> {
    use serde_json::{json, Value};
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
    Ok(cfg)
}

/// The canonical MCP servers map (from MCP_CONFIG_REL, `{{USERPROFILE_FWD}}` expanded, BOM-tolerant),
/// shared by the opencode and codex deploy paths — returns the owned `mcpServers` object.
fn canonical_mcp_servers(home: &str) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let src = std::fs::read_to_string(abs(MCP_CONFIG_REL))
        .map_err(|e| trv("err.mcp_read", cur_lang(), &[("e", &e)]))?
        .replace("{{USERPROFILE_FWD}}", &home.replace('\\', "/"));
    let canonical = parse_json_bom(&src).map_err(|e| trv("err.mcp_parse", cur_lang(), &[("e", &e)]))?;
    canonical
        .get("mcpServers")
        .and_then(|m| m.as_object())
        .cloned()
        .ok_or_else(|| tr("err.mcp_no_servers", cur_lang()).to_string())
}

#[tauri::command]
fn run_opencode_mcp() -> Result<usize, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4/L4
    use serde_json::json;
    let home = std::env::var("USERPROFILE").map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    // Canonical source, placeholders expanded (same as write_temp_mcp_config).
    let servers = canonical_mcp_servers(&home)?;

    let cfg_path = opencode_config_path();
    let mut cfg = load_opencode_cfg_for_edit()?;
    let obj = cfg.as_object_mut().unwrap(); // the helper guaranteed an object with $schema
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
    for (name, def) in &servers {
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
    let prev = read_config_file()
        .managed_mcp
        .and_then(|m| m.opencode)
        .unwrap_or_default();
    for stale in mcp_stale_names(&prev, &canon_names) {
        mcp.remove(&stale);
    }

    let serialized = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    write_json_atomic(&cfg_path, &serialized).map_err(|e| format!("write opencode.json: {e}"))?;
    // R7: update the ledger through the atomic patch (bumps rev; can't lose a concurrent write).
    if let Err(e) = patch_config(|c| {
        c.managed_mcp.get_or_insert_default().opencode = Some(canon_names);
    }) {
        // Don't fail the deploy (opencode.json is already written), but don't swallow it either —
        // a persistent ledger desync would keep drift mis-flagging otherwise.
        eprintln!("[opencode-mcp] ledger update failed: {e}");
    }
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
    let mut cfg = load_opencode_cfg_for_edit()?;
    let obj = cfg.as_object_mut().unwrap(); // the helper guaranteed an object with $schema
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
                // NOTE: MCP env values may hold secrets (tokens/keys) and land on the `codex mcp add`
                // argv, briefly WMI-readable (Win32_Process) — unlike the streaming path which routes
                // secrets via STDIN (see the note at spawn_streamed_io). This is forced by the Codex
                // CLI, which only accepts `--env KEY=VALUE` on the command line; single-user local
                // machine + brief process lifetime make the exposure acceptable (accepted V-9).
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
    // Include '(' ')' and newline/CR: parentheses group commands under cmd, and a raw newline
    // terminates the current command and starts a new one — both let a field inject a second
    // command through the `cmd /C` re-parse even when the shell-op chars are blocked.
    const UNSAFE: &[char] = &['&', '|', '<', '>', '^', '%', '"', '(', ')', '\n', '\r'];
    !argv
        .iter()
        .any(|a| a.chars().any(|c| UNSAFE.contains(&c)))
}

/// Fan out the canonical MCP servers (.mcp.json) into Codex via the official `codex mcp add`
/// CLI — Codex owns its config.toml format/validation, so we never hand-edit TOML. Verified
/// live 2026-07-02 (upstream #3441 is closed): servers registered this way load in a session
/// and their tools resolve via Codex's tool search. Returns the count added.
#[tauri::command]
async fn run_codex_mcp() -> Result<serde_json::Value, String> {
    // Blocking work — a std Mutex + N `cmd /C codex mcp add/remove` subprocesses + config IO — so run
    // it off the async runtime; a synchronous command spawning subprocesses in a loop stalled the UI.
    tokio::task::spawn_blocking(run_codex_mcp_blocking)
        .await
        .map_err(|e| format!("codex mcp task panicked: {e}"))?
}

/// The blocking body of `run_codex_mcp`. Kept a sync fn so `DEPLOY_CFG_LOCK` (a std Mutex) is never
/// held across an `.await`.
fn run_codex_mcp_blocking() -> Result<serde_json::Value, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // L4 (config.json ledger + config.toml)
    use std::os::windows::process::CommandExt;
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let servers = canonical_mcp_servers(&home)?;

    let mut count = 0usize;
    let mut errs: Vec<String> = Vec::new();
    for (name, def) in &servers {
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
    let prev = read_config_file()
        .managed_mcp
        .and_then(|m| m.codex)
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
        // R7: advance the ledger through the atomic patch (bumps rev; can't lose a concurrent write).
        if let Err(e) = patch_config(|c| {
            c.managed_mcp.get_or_insert_default().codex = Some(ledger);
        }) {
            eprintln!("[codex-mcp] ledger update failed: {e}");
        }
    }

    // Partial-honest: report per-server outcome instead of an all-or-nothing Err. The hard Errs above
    // (canon/config unreadable — no candidate at all) already returned; here at least one server was
    // attempted, so surface added + failed. Ledger already advanced only when errs was empty.
    Ok(serde_json::json!({ "added": count, "failed": errs }))
}

/// Merge an OpenAI-Responses-compatible provider into Codex's config.toml text: a
/// Register a provider in `~/.codex/config.toml` as a `[model_providers.<provider_id>]` table.
/// Format-preserving via toml_edit; canonical fields (name/base_url/env_key/wire_api) overwrite.
/// `wire_api` is pinned to `"responses"` — Codex 0.142+ dropped `chat`, so it POSTs
/// `/v1/responses` (which OmniRoute serves). The top-level `model`/`model_provider` and every
/// other user table are never touched. The PROFILE itself lives in a separate file
/// `~/.codex/<provider_id>.config.toml` (see `patch_codex_profile`) — Codex 0.142+ rejects a
/// legacy `[profiles.*]` table in the base config. Raw myproviders.json entries are deliberately
/// NOT fanned out to Codex: it speaks only the Responses wire API, so chat-completions/anthropic
/// endpoints would register but silently fail.
fn patch_codex_config(
    toml_text: &str,
    provider_id: &str,
    display_name: &str,
    base_url: &str,
    env_key: &str,
) -> Result<String, String> {
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
    let p = subtable(providers, provider_id);
    p.insert("name", value(display_name));
    p.insert("base_url", value(format!("{base_url}/v1")));
    p.insert("env_key", value(env_key));
    p.insert("wire_api", value("responses"));

    Ok(doc.to_string())
}

/// Build the contents of a Codex profile file `~/.codex/<provider_id>.config.toml`. Codex 0.142+
/// selects it with `--profile <provider_id>`, overlaying it on the base config. Top-level keys
/// only (NO `[profiles.*]` wrapper). `model` is seeded only when the existing profile has none,
/// so a user's model choice survives a re-deploy. `existing` is the current file text (empty on
/// first write).
fn patch_codex_profile(
    existing: &str,
    provider_id: &str,
    seed_model: &str,
) -> Result<String, String> {
    use toml_edit::{value, DocumentMut};
    let mut doc: DocumentMut = existing
        .trim_start_matches('\u{feff}')
        .parse()
        .map_err(|e| format!("parse profile toml: {e}"))?;
    doc.as_table_mut()
        .insert("model_provider", value(provider_id));
    if !doc.as_table().contains_key("model") {
        doc.as_table_mut().insert("model", value(seed_model));
    }
    Ok(doc.to_string())
}

/// Wire a provider into Codex end-to-end: register `[model_providers.<id>]` (wire_api=responses)
/// in `~/.codex/config.toml` AND write the profile file `~/.codex/<id>.config.toml`. Both writes
/// are atomic; the profile model is seeded only when absent. Shared by the omniroute and
/// freellmapi call sites. Holds no lock — callers take `DEPLOY_CFG_LOCK` (config.toml is shared).
fn deploy_codex_provider(
    provider_id: &str,
    display_name: &str,
    base_url: &str,
    env_key: &str,
    seed_model: &str,
) -> Result<(), String> {
    let home =
        std::env::var("USERPROFILE").map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let cfg_path = format!("{home}\\.codex\\config.toml");
    let text = std::fs::read_to_string(&cfg_path)
        .map_err(|_| tr("err.codex_missing", cur_lang()).to_string())?;
    let patched = patch_codex_config(
        text.trim_start_matches('\u{feff}'),
        provider_id,
        display_name,
        base_url,
        env_key,
    )?;
    write_json_atomic(&cfg_path, &patched).map_err(|e| format!("write config.toml: {e}"))?;

    let prof_path = format!("{home}\\.codex\\{provider_id}.config.toml");
    let existing = std::fs::read_to_string(&prof_path).unwrap_or_default();
    let prof = patch_codex_profile(&existing, provider_id, seed_model)?;
    write_json_atomic(&prof_path, &prof)
        .map_err(|e| format!("write {provider_id}.config.toml: {e}"))?;
    Ok(())
}

/// Connect the freellmapi gateway to Codex (the "providers" fan-out for Codex — see
/// `patch_codex_provider` for why the raw registry is not written). After the config write
/// it best-effort mirrors the gateway's unified API key into the USER environment
/// (`setx FREELLMAPI_API_KEY`, read from the gateway's own SQLite via a read-only node
/// helper — same mechanism as analytics) so `codex --profile freellmapi` works out of the
/// box in a new terminal. Returns whether the key was set; the key itself is never logged.
#[tauri::command]
fn run_codex_providers() -> Result<bool, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4-adjacent (config.toml)
    use std::os::windows::process::CommandExt;
    let base = gateway_base_url().ok_or_else(|| tr("err.gateway_missing", cur_lang()).to_string())?;
    deploy_codex_provider(
        "freellmapi",
        "FreeLLMAPI",
        &base,
        "FREELLMAPI_API_KEY",
        "kimi-k2-thinking",
    )?;

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
        // NOTE: `setx` persists the key as plaintext in HKCU\Environment (readable by every process
        // running as the user) and puts it on argv transiently. This is a deliberate, accepted
        // trade-off (V-10): the user needs `FREELLMAPI_API_KEY` present in terminals they open by
        // HAND to run `codex`, and Windows offers no non-persistent way to inject an env var into
        // future externally-launched shells (a shell-profile hook is equally plaintext). The
        // Credential Manager remains the source of truth; this is a convenience mirror, not storage.
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

/// Connect OmniRoute to Codex (Ф6 — `patch_codex_provider` generalized off the freellmapi-only
/// `run_codex_providers`). Unlike freellmapi there is no key mirror here: OmniRoute's own key
/// management (`omniroute keys`) is the source of truth. Always returns `Ok(false)` — the boolean
/// return shape is kept only to match `run_codex_providers`'s call signature on the frontend.
#[tauri::command]
fn run_codex_omniroute() -> Result<bool, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4-adjacent (config.toml)
    let base = omniroute_base_url().ok_or_else(|| tr("err.omniroute_missing", cur_lang()).to_string())?;
    deploy_codex_provider(
        "omniroute",
        "OmniRoute",
        &base,
        "OMNIROUTE_API_KEY",
        // OmniRoute's combo-router: self-selects a live backend, so a fresh profile works even if
        // one backend is down. Verified live on :20128 (auto/coding is a real model id; the
        // freellmapi seed `kimi-k2-thinking` is NOT — OmniRoute namespaces it as
        // `openrouter/moonshotai/kimi-k2-thinking` etc., so the bare slug would 404).
        "auto/coding",
    )?;
    Ok(false)
}

/// Canonical rule files fanned into OpenCode's `instructions` array (paths, not copies —
/// OpenCode reads them in place, so edits propagate without a re-deploy).
const CANON_RULES_REL: [&str; 2] = [
    "{{PROFILES}}\\config\\CLAUDE.md",
    "{{PROFILES}}\\config\\RTK.md",
];

/// Attach the canonical rule files to OpenCode's `instructions` array (idempotent merge,
/// existing user entries preserved). Returns how many canonical paths are connected after
/// the merge — 0 means none of the files exist on disk.
#[tauri::command]
fn run_opencode_instructions() -> Result<usize, String> {
    let _cfg_guard = DEPLOY_CFG_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // M4
    use serde_json::json;
    let paths: Vec<String> = CANON_RULES_REL
        .iter()
        .map(|rel| abs(rel).replace('\\', "/"))
        .filter(|p| std::path::Path::new(p).is_file())
        .collect();
    if paths.is_empty() {
        return Err(tr("err.canon_rules_missing", cur_lang()).to_string());
    }

    let cfg_path = opencode_config_path();
    let mut cfg = load_opencode_cfg_for_edit()?;
    let obj = cfg.as_object_mut().unwrap(); // the helper guaranteed an object with $schema
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
    fn codex_config_writes_provider_with_responses_wire_api() {
        // existing config with a user default; provider registered, base config preserved
        let existing = "# my codex\nmodel = \"gpt-5.5\"\n";
        let out = super::patch_codex_config(
            existing,
            "omniroute",
            "OmniRoute",
            "http://localhost:20128",
            "OMNIROUTE_API_KEY",
        )
        .unwrap();
        assert!(out.contains("# my codex"));
        assert!(out.contains("model = \"gpt-5.5\"")); // top-level default untouched
        assert!(out.contains("[model_providers.omniroute]"));
        assert!(out.contains("name = \"OmniRoute\""));
        assert!(out.contains("base_url = \"http://localhost:20128/v1\""));
        assert!(out.contains("env_key = \"OMNIROUTE_API_KEY\""));
        assert!(out.contains("wire_api = \"responses\"")); // Codex 0.142+ requirement
        // the legacy [profiles.*] table must NOT be written into config.toml
        assert!(!out.contains("[profiles."));
        // top-level model_provider default stays untouched
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        assert!(doc.get("model_provider").is_none());
    }

    #[test]
    fn codex_profile_seeds_model_only_when_absent() {
        // fresh profile: model_provider set + model seeded, top-level keys only (no wrapper table)
        let fresh = super::patch_codex_profile("", "omniroute", "auto/coding").unwrap();
        assert!(fresh.contains("model_provider = \"omniroute\""));
        assert!(fresh.contains("model = \"auto/coding\""));
        assert!(!fresh.contains("[profiles"));
        let doc: toml_edit::DocumentMut = fresh.parse().unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("omniroute")
        );
        // existing user model is preserved on re-deploy (seed skipped)
        let existing = "model_provider = \"omniroute\"\nmodel = \"gpt-5.5-codex\"\n";
        let out = super::patch_codex_profile(existing, "omniroute", "auto/coding").unwrap();
        assert!(out.contains("model = \"gpt-5.5-codex\""));
        assert!(!out.contains("auto/coding"));
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

/// One bundled item (command / skill / agent) with the frontmatter description and the
/// on-disk file, so the master-detail expansion in PluginsTab can show more than a name.
#[derive(Serialize)]
struct PluginItem {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    path: String,
}

#[derive(Serialize)]
struct PluginContents {
    id: String,
    skills: Vec<PluginItem>,
    commands: Vec<PluginItem>,
    agents: Vec<PluginItem>,
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

/// Frontmatter `description:` of a markdown file, if the file reads and has one.
fn md_description(p: &std::path::Path) -> Option<String> {
    let c = std::fs::read_to_string(p).ok()?;
    fm_value(&extract_frontmatter(&c), "description").filter(|d| !d.is_empty())
}

/// Collect `*.md` items under a directory recursively (used for commands/agents).
/// Nested names are joined with `:` to mirror Claude Code's namespaced naming.
fn collect_md_items(root: &std::path::Path) -> Vec<PluginItem> {
    let mut out: Vec<PluginItem> = Vec::new();
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<PluginItem>, depth: u32) {
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
                        out.push(PluginItem {
                            name,
                            description: md_description(&p),
                            path: p.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
    }
    walk(root, root, &mut out, 0);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Skill items under `<dir>/skills` (one subdir per skill; name/description from SKILL.md
/// frontmatter when present, name falls back to the directory name).
fn collect_skill_items(skills_root: &std::path::Path) -> Vec<PluginItem> {
    let mut out: Vec<PluginItem> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skills_root) {
        for e in entries.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = e.file_name().to_string_lossy().to_string();
            let md = e.path().join("SKILL.md");
            let fm = std::fs::read_to_string(&md)
                .map(|c| extract_frontmatter(&c))
                .unwrap_or_default();
            out.push(PluginItem {
                name: fm_value(&fm, "name").unwrap_or(dir_name),
                description: fm_value(&fm, "description").filter(|d| !d.is_empty()),
                path: md.to_string_lossy().to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Itemize the skills / commands / agents bundled inside each installed plugin.
/// Read-only filesystem scan; no network, no claude CLI spawn.
#[tauri::command]
async fn list_plugin_contents() -> Vec<PluginContents> {
    // Off the main/UI thread: walks each plugin's dir tree (commands/agents/skills) per open.
    tokio::task::spawn_blocking(list_plugin_contents_blocking)
        .await
        .unwrap_or_default()
}

fn list_plugin_contents_blocking() -> Vec<PluginContents> {
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
        let skills = collect_skill_items(&base.join("skills"));
        let commands = collect_md_items(&base.join("commands"));
        let agents = collect_md_items(&base.join("agents"));
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

/// Run `claude plugin <action> <id>` once, optionally under a specific CLAUDE_CONFIG_DIR profile
/// and at an explicit `-s <scope>` (the CLI defaults to user scope, which fails for plugins
/// installed at managed scope). Streams stdout/stderr to the UI log (indented). Returns success —
/// callers must aggregate it instead of reporting a blanket "done".
fn run_claude_plugin(
    claude: &std::path::Path,
    cfg_dir: Option<&str>,
    action: &str,
    id: &str,
    scope: Option<&str>,
    out: &dyn Fn(&str),
    err: &dyn Fn(&str),
) -> bool {
    let mut cmd = std::process::Command::new(claude);
    cmd.args(["plugin", action, id])
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(s) = scope {
        cmd.args(["-s", s]);
    }
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
            o.status.success()
        }
        Err(e) => {
            err(&trv("log.claude_spawn", cur_lang(), &[("e", &e)]));
            false
        }
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
    // A managed-policy `false` makes CC refuse a user-scope enable in EVERY profile — fail fast
    // with the real reason instead of nine identical CLI failures and a green "done".
    if action == "enable" && managed_plugin_policy(id) == Some(false) {
        err(&trv("log.plugin_managed_blocked", cur_lang(), &[("id", &id)]));
        return 1;
    }
    out(&trv(
        "log.plugin_header",
        cur_lang(),
        &[("action", &action), ("id", &id)],
    ));
    let mut failed = 0u32;
    if action == "update" {
        // The CLI updates at user scope by default; a plugin installed at managed scope needs an
        // explicit `-s managed` or the update fails ("not installed at scope user").
        let scope = plugin_scope(id);
        if !run_claude_plugin(&claude, None, action, id, scope.as_deref(), out, err) {
            failed += 1;
        }
    } else {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        for p in profile_names() {
            let dir = format!("{home}\\.claude-{p}");
            if !std::path::Path::new(&dir).exists() {
                out(&trv("log.plugin_skip", cur_lang(), &[("p", &p)]));
                continue;
            }
            out(&format!("  [{p}] claude plugin {action} {id}"));
            if !run_claude_plugin(&claude, Some(&dir), action, id, None, out, err) {
                failed += 1;
            }
        }
    }
    if failed > 0 {
        err(&trv("log.plugin_failed_n", cur_lang(), &[("n", &failed)]));
        return 1;
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
        let rc = spawn_streamed_prog(
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
        invalidate_plugins_cache(); // the installed set changed — next list_plugins must re-read
        return rc;
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
    let rc = run_native_streamed(app, state, stream_id::PLUGIN_MGR.to_string(), move |out, err| {
        manage_plugin_native(&action, &id, out, err)
    })
    .await;
    invalidate_plugins_cache(); // the enabled/scope set changed — next list_plugins must re-read
    rc
}

/// Ф3: bump an OWN (directory-source) marketplace plugin's version via the vetted
/// Check-MarketplaceVersions.ps1 (-Bump: atomic dual-manifest write, sibling-safe, non-interactive),
/// then refresh the shared plugin cache via `claude plugin update`. Streams to the console under the
/// same "plugin-mgr" component as the other per-plugin actions.
#[tauri::command]
async fn run_marketplace_bump(
    app: AppHandle,
    state: State<'_, RunState>,
    id: String,
    level: String,
) -> Result<i32, String> {
    if !matches!(level.as_str(), "patch" | "minor" | "major") {
        return Err(format!("unknown bump level: {level}"));
    }
    // Same id guard as run_plugin: the id reaches process args and a filesystem path.
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-' | '/'))
    {
        return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", &id)]));
    }
    let Some(at) = id.rfind('@') else {
        return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", &id)]));
    };
    let plugin = id[..at].to_string();
    let market = id[at + 1..].to_string();
    if !plugin_id_path_safe(&plugin) {
        return Err(trv("err.invalid_plugin_id", cur_lang(), &[("id", &id)]));
    }
    if !own_marketplaces().contains(&market) {
        return Err(format!("{market} is not an own (directory-source) marketplace"));
    }
    let Some((_, _, markets)) = load_installed_plugins() else {
        return Err("installed_plugins.json unreadable".into());
    };
    let Some(loc) = markets
        .get(&market)
        .and_then(|m| m.get("installLocation"))
        .and_then(|v| v.as_str())
        .map(String::from)
    else {
        return Err(format!("no installLocation for {market}"));
    };
    let script = format!("{loc}\\Check-MarketplaceVersions.ps1");
    if !std::path::Path::new(&script).is_file() {
        return Err(format!("bump script not found: {script}"));
    }
    run_native_streamed(app, state, stream_id::PLUGIN_MGR.to_string(), move |out, err| {
        out(&trv(
            "log.bump_header",
            cur_lang(),
            &[("level", &level), ("id", &id)],
        ));
        let code = match std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script,
                "-Bump",
                &level,
                "-Plugin",
                &plugin,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) => {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    out(&format!("  {line}"));
                }
                for line in String::from_utf8_lossy(&o.stderr).lines() {
                    err(&format!("  {line}"));
                }
                o.status.code().unwrap_or(-1)
            }
            Err(e) => {
                err(&trv("log.claude_spawn", cur_lang(), &[("e", &e)]));
                -1
            }
        };
        if code != 0 {
            err(tr("log.bump_failed", cur_lang()));
            return code;
        }
        // Refresh the shared plugin cache so the recorded installed version follows the source.
        let Some(claude) = exe_on_path("claude") else {
            err(tr("log.claude_not_found", cur_lang()));
            return 1;
        };
        let scope = plugin_scope(&id);
        out(&format!("  claude plugin update {id}"));
        if !run_claude_plugin(&claude, None, "update", &id, scope.as_deref(), out, err) {
            err(tr("log.bump_failed", cur_lang()));
            return 1;
        }
        invalidate_plugins_cache(); // version bump + update changed the set — next list_plugins must re-read
        out(tr("log.done", cur_lang()));
        0
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
    invalidate_plugins_cache(); // P4: a bulk install/update/remove changed the list — force a re-read
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

// ---- Subagents manager (~/.claude/agents/*.md) ---------------------------------------------
// Standalone user subagents that Claude Code reads from ~/.claude/agents. Structurally identical to
// skills (frontmatter + body), so the SKILL.md parsers (extract_frontmatter/fm_value) are reused
// verbatim. The `agents` folder is junction-linked into every profile AND Syncthing-synced between
// machines (see ClaudeProfiles\config\profiles.json linkedFolders + sync_item_lines), so a write
// here fans out with no extra code — do NOT add a per-profile copy path.

#[derive(Serialize)]
struct AgentInfo {
    name: String,
    description: String,
    model: String,
    tools: String,
    path: String,
}

#[derive(Serialize)]
struct AgentDetail {
    name: String,
    description: String,
    model: String,
    tools: String,
    prompt: String,
    path: String,
}

/// ~/.claude/agents — the canonical standalone-subagent dir (mirrors list_skills' ~/.claude/skills).
fn agents_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| format!("USERPROFILE: {e}"))?;
    Ok(std::path::Path::new(&home).join(".claude").join("agents"))
}

/// Refuse any target whose PARENT isn't the real agents dir — canonicalized so a junctioned dir and
/// path-traversal both resolve honestly (same guard shape as delete_skill).
fn agent_guard(target: &std::path::Path) -> Result<(), String> {
    let canon_dir =
        std::fs::canonicalize(agents_dir()?).map_err(|e| format!("agents dir: {e}"))?;
    let parent = target
        .parent()
        .ok_or_else(|| tr("err.bad_path", cur_lang()).to_string())?;
    let canon_parent = std::fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if canon_parent != canon_dir {
        return Err(tr("err.bad_path", cur_lang()).into());
    }
    Ok(())
}

/// Body after the frontmatter's closing `---` (leading blank lines trimmed). No frontmatter → the
/// whole file is the body. Tolerant of both `\n` and `\r\n` (the `\n---` match ignores a leading \r).
fn frontmatter_body(content: &str) -> String {
    let t = content.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            // Skip the closing "---", then the rest of that line, then leading blank lines.
            let after = &rest[end + 4..];
            let after = after.split_once('\n').map(|(_, b)| b).unwrap_or("");
            return after.trim_start_matches(['\r', '\n']).to_string();
        }
    }
    content.to_string()
}

/// ASCII kebab-case slug for the .md filename. Non-ASCII/empty → "agent" (the display `name:`
/// frontmatter still carries the user's text; Claude Code identifies a subagent by `name`, not file).
fn slugify_agent(name: &str) -> String {
    let slug = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() { "agent".into() } else { slug }
}

/// Render a subagent .md: frontmatter (name/description always; model/tools only when set) + body.
/// UTF-8 without BOM (Castellyn's own writer convention). Unquoted scalars match the ecosystem
/// convention (plugin agents ship unquoted) — the description is kept single-line by the UI.
fn render_agent_md(name: &str, description: &str, model: &str, tools: &str, prompt: &str) -> String {
    // Sanitize each scalar: a raw newline (or a line equal to `---`) in a value would break out of the
    // YAML frontmatter block. Collapse interior CR/LF to spaces so every value stays on one line.
    let clean = |v: &str| v.trim().replace(['\r', '\n'], " ");
    let mut s = String::from("---\n");
    s.push_str(&format!("name: {}\n", clean(name)));
    s.push_str(&format!("description: {}\n", clean(description)));
    if !model.trim().is_empty() {
        s.push_str(&format!("model: {}\n", clean(model)));
    }
    if !tools.trim().is_empty() {
        s.push_str(&format!("tools: {}\n", clean(tools)));
    }
    s.push_str("---\n\n");
    s.push_str(prompt.trim_end());
    s.push('\n');
    s
}

#[tauri::command]
async fn list_agents() -> Vec<AgentInfo> {
    tokio::task::spawn_blocking(list_agents_blocking)
        .await
        .unwrap_or_default()
}

fn list_agents_blocking() -> Vec<AgentInfo> {
    let Ok(dir) = agents_dir() else {
        return Vec::new();
    };
    let mut out: Vec<AgentInfo> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() || p.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&p).unwrap_or_default();
            let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
            let fm = extract_frontmatter(content);
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(AgentInfo {
                name: fm_value(&fm, "name").unwrap_or(stem),
                description: fm_value(&fm, "description").unwrap_or_default(),
                model: fm_value(&fm, "model").unwrap_or_default(),
                tools: fm_value(&fm, "tools").unwrap_or_default(),
                path: p.display().to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[tauri::command]
fn read_agent(path: String) -> Result<AgentDetail, String> {
    let p = std::path::Path::new(&path);
    agent_guard(p)?;
    let content = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let fm = extract_frontmatter(content);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(AgentDetail {
        name: fm_value(&fm, "name").unwrap_or(stem),
        description: fm_value(&fm, "description").unwrap_or_default(),
        model: fm_value(&fm, "model").unwrap_or_default(),
        tools: fm_value(&fm, "tools").unwrap_or_default(),
        prompt: frontmatter_body(content),
        path: p.display().to_string(),
    })
}

/// Write a subagent. `path` present → overwrite that file (edit); absent → create a new
/// `<slug>.md`, made unique so a create never clobbers an existing agent. Returns the written path.
#[tauri::command]
fn save_agent(
    name: String,
    description: String,
    model: String,
    tools: String,
    prompt: String,
    path: Option<String>,
) -> Result<String, String> {
    let dir = agents_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = match path.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => {
            let pp = std::path::Path::new(p).to_path_buf();
            agent_guard(&pp)?;
            pp
        }
        None => {
            let base = slugify_agent(&name);
            let mut cand = dir.join(format!("{base}.md"));
            let mut n = 2;
            while cand.exists() {
                cand = dir.join(format!("{base}-{n}.md"));
                n += 1;
            }
            cand
        }
    };
    let content = render_agent_md(&name, &description, &model, &tools, &prompt);
    std::fs::write(&target, content).map_err(|e| e.to_string())?;
    Ok(target.display().to_string())
}

#[tauri::command]
fn delete_agent(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    agent_guard(p)?;
    std::fs::remove_file(p).map_err(|e| e.to_string())
}

/// One check line of a subagent smoke test.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTestLine {
    ok: bool,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTestResult {
    ok: bool,
    lines: Vec<AgentTestLine>,
}

/// Smoke-test a subagent WITHOUT invoking it in a real session: validate its frontmatter + body, and
/// for a wrapper agent (its prompt shells out to codex/opencode) probe that the target CLI actually
/// resolves and answers `--version`. Answers "is this agent well-formed and are its deps present?".
#[tauri::command]
async fn test_subagent(path: String) -> Result<AgentTestResult, String> {
    tokio::task::spawn_blocking(move || test_subagent_blocking(&path))
        .await
        .map_err(|e| format!("test_subagent panicked: {e}"))?
}

fn test_subagent_blocking(path: &str) -> Result<AgentTestResult, String> {
    let p = std::path::Path::new(path);
    agent_guard(p)?;
    let raw = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    let content = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let fm = extract_frontmatter(content);
    let body = frontmatter_body(content);
    let mut lines: Vec<AgentTestLine> = Vec::new();
    let push = |lines: &mut Vec<AgentTestLine>, ok: bool, text: String| lines.push(AgentTestLine { ok, text });

    let has_name = fm_value(&fm, "name").is_some_and(|s| !s.trim().is_empty());
    let has_desc = fm_value(&fm, "description").is_some_and(|s| !s.trim().is_empty());
    push(&mut lines, has_name, if has_name { "name задан".into() } else { "нет name во frontmatter".into() });
    push(&mut lines, has_desc, if has_desc { "description задан".into() } else { "пустое description — агент не будет авто-выбираться по задаче".into() });
    let has_body = !body.trim().is_empty();
    push(&mut lines, has_body, if has_body { "системный промпт задан".into() } else { "пустой системный промпт".into() });

    // Wrapper agents shell out to an external CLI — that CLI must resolve and run.
    let bl = body.to_lowercase();
    for (needle, cli) in [("codex", "codex"), ("opencode", "opencode")] {
        if bl.contains(needle) {
            match exe_on_path(cli) {
                Some(exe) => {
                    let out = std::process::Command::new(&exe)
                        .arg("--version")
                        .creation_flags(CREATE_NO_WINDOW)
                        .output();
                    match out {
                        Ok(o) if o.status.success() => {
                            let v = String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string();
                            push(&mut lines, true, format!("CLI `{cli}` найден и отвечает: {v}"));
                        }
                        _ => push(&mut lines, false, format!("CLI `{cli}` найден, но не отвечает на --version")),
                    }
                }
                None => push(&mut lines, false, format!("CLI `{cli}` не найден на PATH — обёртка не сработает")),
            }
        }
    }

    let ok = lines.iter().all(|l| l.ok);
    Ok(AgentTestResult { ok, lines })
}

/// Strip Syncthing's `.sync-conflict-<stamp>` infix to recover the original file's path.
/// Format: `<name>.sync-conflict-YYYYMMDD-HHMMSS-XXXXXXX<.ext>` — the infix sits right before the
/// final extension (or at the very end when the file has none). The stamp itself carries no `.`,
/// so everything from the first `.` after `.sync-conflict-` is the original extension suffix.
/// Returns None when the marker is absent.
fn conflict_base_path(path: &str) -> Option<String> {
    const MARK: &str = ".sync-conflict-";
    let (prefix, rest) = path.split_once(MARK)?;
    // Suffix = original extension (from the first dot after the stamp), or "" when extensionless.
    let suffix = rest.find('.').map(|i| &rest[i..]).unwrap_or("");
    Some(format!("{prefix}{suffix}"))
}

/// Resolve one Syncthing sync-conflict file (Sync tab). Two frozen SAFETY guards: the name must
/// carry the `.sync-conflict-` marker AND the canonicalized path must sit inside `%USERPROFILE%\.claude`
/// — either miss is refused, so a stray path can never delete/overwrite outside the profile tree.
///   keep-local  = delete the conflict file (keep the local original untouched).
///   keep-other  = adopt the conflict's version: rename base -> base.pre-conflict.bak, then conflict -> base.
#[tauri::command]
fn resolve_sync_conflict(path: String, action: String) -> Result<(), String> {
    // Guard 1: the marker. Cheap string check first; also yields the base path for keep-other.
    let base = conflict_base_path(&path)
        .ok_or_else(|| format!("not a sync-conflict file: {path}"))?;
    // Guard 2: canonicalize and confine to %USERPROFILE%\.claude (component-wise, not string prefix).
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let claude_root = std::fs::canonicalize(std::path::Path::new(&home).join(".claude"))
        .map_err(|e| format!("resolve .claude: {e}"))?;
    let canon = std::fs::canonicalize(&path).map_err(|e| format!("no such file: {e}"))?;
    if !canon.starts_with(&claude_root) {
        return Err(format!("refused: {path} is outside {}", claude_root.display()));
    }
    match action.as_str() {
        "keep-local" => {
            std::fs::remove_file(&path).map_err(|e| format!("delete conflict: {e}"))
        }
        "keep-other" => {
            // Back up the local original (best-effort: absent base just means nothing to preserve),
            // then promote the conflict copy into the original's place.
            if std::path::Path::new(&base).exists() {
                std::fs::rename(&base, format!("{base}.pre-conflict.bak"))
                    .map_err(|e| format!("backup base: {e}"))?;
            }
            std::fs::rename(&path, &base).map_err(|e| format!("promote conflict: {e}"))
        }
        _ => Err(format!("unknown action: {action}")),
    }
}

#[cfg(test)]
mod conflict_base_tests {
    use super::conflict_base_path;
    #[test]
    fn with_extension() {
        assert_eq!(
            conflict_base_path(r"C:\u\.claude\settings.sync-conflict-20260707-120000-ABCDEFG.json")
                .as_deref(),
            Some(r"C:\u\.claude\settings.json")
        );
    }
    #[test]
    fn without_extension() {
        assert_eq!(
            conflict_base_path(r"C:\u\.claude\README.sync-conflict-20260707-120000-ABCDEFG")
                .as_deref(),
            Some(r"C:\u\.claude\README")
        );
    }
    #[test]
    fn multiple_dots() {
        // Infix sits before the FINAL extension: my.config.<infix>.json -> my.config.json
        assert_eq!(
            conflict_base_path(r"C:\u\.claude\my.config.sync-conflict-20260707-120000-ABCDEFG.json")
                .as_deref(),
            Some(r"C:\u\.claude\my.config.json")
        );
    }
    #[test]
    fn no_marker_is_none() {
        assert_eq!(conflict_base_path(r"C:\u\.claude\settings.json"), None);
    }
}

#[cfg(test)]
mod agent_tests {
    use super::{
        extract_frontmatter, fm_value, frontmatter_body, render_agent_md, slugify_agent,
    };

    #[test]
    fn round_trip_render_parse() {
        let md = render_agent_md(
            "my-agent",
            "When to use it",
            "sonnet",
            "Read, Grep",
            "You are a helper.\n\nDo the thing.",
        );
        let fm = extract_frontmatter(&md);
        assert_eq!(fm_value(&fm, "name").as_deref(), Some("my-agent"));
        assert_eq!(fm_value(&fm, "description").as_deref(), Some("When to use it"));
        assert_eq!(fm_value(&fm, "model").as_deref(), Some("sonnet"));
        assert_eq!(fm_value(&fm, "tools").as_deref(), Some("Read, Grep"));
        assert_eq!(frontmatter_body(&md).trim_end(), "You are a helper.\n\nDo the thing.");
    }

    #[test]
    fn omits_empty_model_and_tools() {
        let md = render_agent_md("a", "desc", "", "  ", "body");
        assert!(!md.contains("model:"));
        assert!(!md.contains("tools:"));
    }

    #[test]
    fn slug_kebabs_and_falls_back() {
        assert_eq!(slugify_agent("My Cool Agent!"), "my-cool-agent");
        assert_eq!(slugify_agent("  a__b  "), "a-b");
        assert_eq!(slugify_agent("Агент"), "agent"); // non-ASCII → generic fallback
    }

    #[test]
    fn body_without_frontmatter_is_whole() {
        assert_eq!(frontmatter_body("just text"), "just text");
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

/// True if any descendant of `dir` (at ANY depth, not just immediate children) is a reparse point
/// (junction/symlink) — OR the dir can't be enumerated. `recycle_dir` recycles the WHOLE subtree, so
/// a junction nested several levels down would otherwise be followed and its target swept; the check
/// must therefore recurse. Checks the raw FILE_ATTRIBUTE_REPARSE_POINT bit (via symlink_metadata, so
/// the link itself is stat'd, not its target) — this catches junctions AND symlinks, which
/// `is_symlink()` alone can miss on Windows. Fails CLOSED on a read_dir error: if we can't prove the
/// dir is junction-free, the destructive caller must refuse rather than risk sweeping a link target.
fn has_reparse_child(dir: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    for e in entries.flatten() {
        let p = e.path();
        let is_reparse = p
            .symlink_metadata()
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false);
        if is_reparse {
            return true;
        }
        // p is a real (non-reparse) entry; descend into real subdirectories to catch a nested link.
        if p.is_dir() && has_reparse_child(&p) {
            return true;
        }
    }
    false
}

/// Move a path to the Windows Recycle Bin via the .NET VisualBasic helper (no extra crate).
/// `method` is a hardcoded FileSystem method name ("DeleteDirectory" | "DeleteFile"), never user
/// input. The path is passed through an env var (not string-interpolated) so trailing spaces /
/// special chars survive verbatim and there is no quoting to escape.
fn recycle_path(method: &str, path: &str) -> Result<(), String> {
    let script = format!(
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         [Microsoft.VisualBasic.FileIO.FileSystem]::{method}($env:CASTELLYN_DEL_PATH, \
         'OnlyErrorDialogs', 'SendToRecycleBin')"
    );
    let out = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
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

/// Recycle a directory (whole subtree) to the bin.
fn recycle_dir(path: &str) -> Result<(), String> {
    recycle_path("DeleteDirectory", path)
}

/// Recycle a single file to the bin.
fn recycle_file(path: &str) -> Result<(), String> {
    recycle_path("DeleteFile", path)
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
    run_native_streamed(app, state, stream_id::PLUGIN_SYNC.to_string(), move |out, err| {
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

// --- Stack drift + managed-settings redeploy (Plugins tab reconcile) ---
//
// Reports whether the local Claude Code stack matches what Castellyn owns:
//  1. plugin_sync_file    — the hook script on disk vs render_plugin_sync_script (the oracle).
//  2. plugin_sync_wiring  — every profile has the SessionStart hook AND managed-settings.json
//                           does NOT double-wire it.
//  3. managed_settings    — the deployed managed-settings.json matches the version-controlled source.
// run_managed_deploy fixes (3) via one elevated (UAC) redeploy, then re-verifies by comparison.

/// Deploy script for managed-settings.json (parent of the config source dir). Elevated by RunAs.
const MANAGED_DEPLOY_SCRIPT_REL: &str = "{{PROFILES}}\\Deploy-ManagedSettings.ps1";

/// A single stack-drift check result. Field names are the IPC contract: id, state, detail, fix.
#[derive(Serialize)]
struct StackDriftItem {
    /// Stable check id: "plugin_sync_file" | "plugin_sync_wiring" | "managed_settings".
    id: String,
    /// "ok" | "drift" | "missing" | "error".
    state: String,
    /// Human-readable specifics (missing profiles, why it drifted, or the IO error text).
    detail: String,
    /// Actionable fix key the UI can trigger: "plugin_sync" | "managed_deploy", or None.
    fix: Option<String>,
}

/// Deployed managed-settings.json under %ProgramFiles% (None when ProgramFiles is unset).
fn deployed_managed_path() -> Option<String> {
    std::env::var("ProgramFiles").ok().map(|pf| format!("{pf}\\ClaudeCode\\managed-settings.json"))
}

/// Version-controlled SOURCE managed-settings.json under scripts_root.
fn source_managed_path() -> String {
    abs(&format!("{CONFIG_SOURCE_REL}\\managed-settings.json"))
}

/// Pure: do two JSON texts parse (BOM-tolerant) to equal values? Key order / whitespace / a
/// leading BOM are all normalized away; a parse failure on either side is NOT equal. Unit-tested.
fn json_normalized_eq(a: &str, b: &str) -> bool {
    match (parse_json_bom(a), parse_json_bom(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Pure: classify the on-disk plugin_sync hook against the rendered oracle. `disk` is None when the
/// file is absent. Uses the version header to tell a newer/foreign copy from a stale-list copy, and
/// the `# castellyn:profiles` marker to tell an external (non-Castellyn) script from our own. Unit-tested.
fn classify_plugin_sync_file(disk: Option<&str>, rendered: &str) -> (String, String, Option<String>) {
    let Some(disk) = disk else {
        return (
            "missing".into(),
            "hook script not installed at ~/.claude/hooks/plugin_sync.py".into(),
            Some("plugin_sync".into()),
        );
    };
    if disk == rendered {
        return ("ok".into(), "hook script matches the current profile list".into(), None);
    }
    let disk_ver = plugin_sync_version(disk);
    let emb_ver = plugin_sync_version(rendered);
    if disk_ver > emb_ver {
        // ensure_plugin_sync_script never downgrades, so offering the plugin_sync fix here would
        // be a no-op button with the drift persisting. Report honestly, no false affordance.
        return (
            "drift".into(),
            format!("on-disk hook is a newer/foreign version ({disk_ver} > embedded {emb_ver}) — update Castellyn or remove the file manually"),
            None,
        );
    }
    let detail = if disk.contains("# castellyn:profiles") {
        "on-disk hook has a stale profile list".to_string()
    } else {
        "on-disk hook is an external version (no Castellyn marker)".to_string()
    };
    ("drift".into(), detail, Some("plugin_sync".into()))
}

/// Pure: classify wiring drift. `unwired` = profiles missing the SessionStart hook; `managed_double`
/// = managed-settings.json ALSO wires plugin_sync (double-wiring). Missing wiring is auto-fixable
/// (re-run plugin_sync setup); double-wiring alone is fixed at the managed SOURCE, so fix=None there
/// (editing + redeploying source is the remedy, not the plugin_sync setup). Unit-tested.
fn classify_wiring(unwired: &[String], managed_double: bool) -> (String, String, Option<String>) {
    let mut parts = Vec::new();
    if !unwired.is_empty() {
        parts.push(format!("profiles missing SessionStart wiring: {}", unwired.join(", ")));
    }
    if managed_double {
        parts.push("managed-settings.json also wires plugin_sync (double-wiring)".to_string());
    }
    if parts.is_empty() {
        return ("ok".into(), "every profile wired; managed settings clean".into(), None);
    }
    let fix = if unwired.is_empty() {
        None // only double-wiring → the fix is editing the managed SOURCE, not plugin_sync setup
    } else {
        Some("plugin_sync".into())
    };
    ("drift".into(), parts.join("; "), fix)
}

fn plugin_sync_file_drift_item(home: &str) -> StackDriftItem {
    let dirs = plugin_sync_profiles(home)
        .into_iter()
        .map(|(d, _)| d)
        .collect::<Vec<_>>();
    let rendered = render_plugin_sync_script(&dirs);
    let disk = std::fs::read_to_string(plugin_sync_script_path(home)).ok();
    let (state, detail, fix) = classify_plugin_sync_file(disk.as_deref(), &rendered);
    StackDriftItem { id: "plugin_sync_file".into(), state, detail, fix }
}

fn plugin_sync_wiring_drift_item(home: &str) -> StackDriftItem {
    let mut unwired = Vec::new();
    for (name, sp) in plugin_sync_profiles(home) {
        let wired = std::fs::read_to_string(&sp)
            .ok()
            .and_then(|c| parse_json_bom(&c).ok())
            .map(|v| plugin_sync_hook_wired(&v))
            .unwrap_or(false); // unreadable/malformed → treat as unwired (matches plugin_sync_status)
        if !wired {
            unwired.push(name);
        }
    }
    // Deployed managed settings must NOT mention plugin_sync (a raw substring check is enough — any
    // wiring, however shaped, contains the filename). Absence/unreadable → no double-wiring.
    let managed_double = deployed_managed_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|c| c.contains("plugin_sync"))
        .unwrap_or(false);
    let (state, detail, fix) = classify_wiring(&unwired, managed_double);
    StackDriftItem { id: "plugin_sync_wiring".into(), state, detail, fix }
}

/// Compare the version-controlled SOURCE managed-settings.json against the deployed copy. Any IO
/// failure on the source degrades to state=error; a missing/invalid/differing deployed copy is drift
/// with fix=managed_deploy. Also the re-verification step of run_managed_deploy. Never panics.
fn managed_settings_drift_item() -> StackDriftItem {
    let id = "managed_settings".to_string();
    let src = match std::fs::read_to_string(source_managed_path()) {
        Ok(s) => s,
        Err(e) => {
            return StackDriftItem { id, state: "error".into(), detail: format!("read source: {e}"), fix: None }
        }
    };
    if parse_json_bom(&src).is_err() {
        return StackDriftItem {
            id,
            state: "error".into(),
            detail: "source managed-settings.json is not valid JSON".into(),
            fix: None,
        };
    }
    let Some(dep_path) = deployed_managed_path() else {
        return StackDriftItem {
            id,
            state: "error".into(),
            detail: "ProgramFiles environment variable is unset".into(),
            fix: None,
        };
    };
    let dep = match std::fs::read_to_string(&dep_path) {
        Ok(s) => s,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            return StackDriftItem {
                id,
                state: "drift".into(),
                detail: "deployed managed-settings.json is missing".into(),
                fix: Some("managed_deploy".into()),
            }
        }
        Err(e) => {
            return StackDriftItem { id, state: "error".into(), detail: format!("read deployed: {e}"), fix: None }
        }
    };
    if json_normalized_eq(&src, &dep) {
        StackDriftItem { id, state: "ok".into(), detail: "deployed matches source".into(), fix: None }
    } else {
        // src is already valid JSON, so !eq means the deployed copy differs or is itself invalid.
        let detail = if parse_json_bom(&dep).is_err() {
            "deployed managed-settings.json is not valid JSON — needs redeploy".to_string()
        } else {
            "source is newer than deployed — needs redeploy".to_string()
        };
        StackDriftItem { id, state: "drift".into(), detail, fix: Some("managed_deploy".into()) }
    }
}

/// One own-marketplace plugin's version triple: marketplace.json entry vs its plugin.json vs the
/// installed version recorded in installed_plugins.json (None = not installed / unreadable).
struct MarketVer {
    plugin: String,
    market: String,
    mkt_ver: String,
    src_ver: Option<String>,
    installed: Option<String>,
}

/// Classify own-marketplace version alignment into a (state, detail) pair. A manifest mismatch
/// needs a dual bump (Check-MarketplaceVersions -Bump); an installed version behind an aligned
/// source needs a refresh (`claude plugin update`). "unknown" installed versions are ignored —
/// directory-source plugins load content from the source dir anyway. Unit-tested.
fn classify_marketplace_versions(rows: &[MarketVer]) -> (String, String) {
    let mut probs: Vec<String> = Vec::new();
    for r in rows {
        let who = format!("{}@{}", r.plugin, r.market);
        match &r.src_ver {
            None => probs.push(format!("{who}: plugin.json unreadable")),
            Some(sv) if sv != &r.mkt_ver => probs.push(format!(
                "{who}: marketplace.json {} \u{2260} plugin.json {sv} — bump both manifests",
                r.mkt_ver
            )),
            Some(sv) => {
                if let Some(inst) = &r.installed {
                    if inst != sv && inst != "unknown" {
                        probs.push(format!("{who}: installed {inst} behind source {sv} — update"));
                    }
                }
            }
        }
    }
    if !probs.is_empty() {
        ("drift".to_string(), probs.join("; "))
    } else if rows.is_empty() {
        ("ok".to_string(), "no own marketplaces".to_string())
    } else {
        ("ok".to_string(), "own marketplace versions aligned".to_string())
    }
}

/// Ф3: version alignment of OWN (directory-source) marketplaces — marketplace.json vs each
/// plugin.json vs installed versions. fix=None: the bump/update actions live on the Plugins tab.
fn marketplace_versions_drift_item() -> StackDriftItem {
    let id = "marketplace_versions".to_string();
    let Some((_, installed, markets)) = load_installed_plugins() else {
        return StackDriftItem {
            id,
            state: "error".into(),
            detail: "installed_plugins.json unreadable".into(),
            fix: None,
        };
    };
    let mut rows: Vec<MarketVer> = Vec::new();
    for name in own_marketplaces() {
        let Some(loc) = markets
            .get(&name)
            .and_then(|m| m.get("installLocation"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let mtxt = match std::fs::read_to_string(format!("{loc}\\.claude-plugin\\marketplace.json")) {
            Ok(s) => s,
            Err(e) => {
                return StackDriftItem {
                    id,
                    state: "error".into(),
                    detail: format!("{name}: read marketplace.json: {e}"),
                    fix: None,
                }
            }
        };
        let Ok(m) = parse_json_bom(&mtxt) else {
            return StackDriftItem {
                id,
                state: "error".into(),
                detail: format!("{name}: marketplace.json is not valid JSON"),
                fix: None,
            };
        };
        for p in m.get("plugins").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let (Some(pn), Some(mv)) = (
                p.get("name").and_then(|v| v.as_str()),
                p.get("version").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            if !plugin_id_path_safe(pn) {
                continue;
            }
            let src_ver = std::fs::read_to_string(format!(
                "{loc}\\plugins\\{pn}\\.claude-plugin\\plugin.json"
            ))
            .ok()
            .and_then(|s| parse_json_bom(&s).ok())
            .and_then(|j| j.get("version").and_then(|v| v.as_str()).map(String::from));
            let inst = installed
                .get("plugins")
                .and_then(|o| o.get(format!("{pn}@{name}")))
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|e| e.get("version"))
                .and_then(|v| v.as_str())
                .map(String::from);
            rows.push(MarketVer {
                plugin: pn.to_string(),
                market: name.clone(),
                mkt_ver: mv.to_string(),
                src_ver,
                installed: inst,
            });
        }
    }
    let (state, detail) = classify_marketplace_versions(&rows);
    StackDriftItem { id, state, detail, fix: None }
}

/// Report the four stack-drift checks (plugin_sync file, plugin_sync wiring, managed settings,
/// own-marketplace versions).
/// Never panics: every per-item IO failure degrades to that item's state=error with the error text.
#[tauri::command]
fn read_stack_drift() -> Result<Vec<StackDriftItem>, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    Ok(vec![
        plugin_sync_file_drift_item(&home),
        plugin_sync_wiring_drift_item(&home),
        managed_settings_drift_item(),
        marketplace_versions_drift_item(),
    ])
}

/// Redeploy managed-settings.json with a single elevation prompt, then re-verify by comparison.
/// The OUTER powershell is hidden (CREATE_NO_WINDOW); the INNER Start-Process -Verb RunAs shows the
/// UAC dialog (intended). The exit code is deliberately NOT trusted — the returned StackDriftItem is
/// a fresh source↔deployed comparison, so a silent no-op deploy surfaces as lingering drift.
#[tauri::command]
async fn run_managed_deploy() -> Result<StackDriftItem, String> {
    // Double any apostrophe so the path can't break out of the single-quoted PowerShell array element
    // below (a username with a `'` would otherwise inject into the -ArgumentList string).
    let deploy = abs(MANAGED_DEPLOY_SCRIPT_REL).replace('\'', "''");
    tokio::task::spawn_blocking(move || {
        let inner = format!(
            "Start-Process powershell -Verb RunAs -Wait -ArgumentList \
             '-ExecutionPolicy','Bypass','-File','{deploy}'"
        );
        let _ = std::process::Command::new("powershell.exe")
            .args(["-ExecutionPolicy", "Bypass", "-Command", &inner])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        managed_settings_drift_item()
    })
    .await
    .map_err(|e| e.to_string())
}

// --- Onboarding: new-machine deployment checklist (first-run view + Settings entry) ---
//
// read_onboarding scans the machine into idempotent checklist steps; every fix routes to an
// EXISTING command (run_profiles 'reinstall', run_mcp 'deploy', run_managed_deploy, Backup tab).
// run_onboarding_step covers the two scripts not wrapped elsewhere (Configure-Syncthing.ps1,
// Assert-Installation.ps1); create_settings_junction is the one native action. Re-running any
// step is safe — the wizard is a reconciler, not a one-shot.

const SYNCTHING_SCRIPT_REL: &str = "{{PROFILES}}\\Configure-Syncthing.ps1";
const ASSERT_SCRIPT_REL: &str = "{{PROFILES}}\\Assert-Installation.ps1";

/// One onboarding step. Field names ARE the IPC contract.
#[derive(Serialize, Clone)]
struct OnbStep {
    /// "prereq_git"|"prereq_node"|"prereq_claude"|"prereq_syncthing"|"tree"|"junction"
    /// |"profiles"|"creds"|"mcp"|"managed"|"syncthing"|"verify"
    id: String,
    /// "ok" | "todo" | "blocked" (dependency not met) | "unknown" (not natively detectable —
    /// the action is idempotent, run it to be sure)
    state: String,
    detail: String,
    /// Fix key the UI wires to a command: "install_profiles" | "mcp_deploy" | "managed_deploy"
    /// | "junction" | "syncthing" | "verify" | "backup_tab"; None = informational.
    fix: Option<String>,
}

fn onb(id: &str, state: &str, detail: String, fix: Option<&str>) -> OnbStep {
    OnbStep { id: id.into(), state: state.into(), detail, fix: fix.map(String::from) }
}

/// Does a link's read_link target match the expected dir? Windows may report a `\\?\`-prefixed
/// target; compare after stripping it, ASCII-case-insensitively (drive letters), trailing `\`
/// ignored. Pure, unit-tested. A false negative only re-flags the step "todo" — safe direction.
fn link_target_matches(target: &std::path::Path, expected: &str) -> bool {
    let t = target.to_string_lossy();
    let t = t.strip_prefix(r"\\?\").unwrap_or(&t);
    t.trim_end_matches('\\')
        .eq_ignore_ascii_case(expected.trim_end_matches('\\'))
}

/// Which managed folders carry the versioning Configure-Syncthing.ps1 sets: staggered on
/// E:\Scripts and ~\.memory, trashcan on ~\.claude — matched BY PATH like the script (folder ids
/// are per-machine). Returns (managed folders found, folders already hardened); None when the
/// REST API is unreachable / has no key.
fn syncthing_hardening_state() -> Option<(usize, usize)> {
    let (key, base) = syncthing_conn()?;
    let agent = st_agent();
    let folders_v = st_get(&agent, &base, &key, "/rest/config/folders")?;
    let folders = folders_v.as_array()?;
    let home = std::env::var("USERPROFILE").ok()?;
    let want = [
        (normalize_path(&scripts_root()), "staggered"),
        (normalize_path(&format!("{home}\\.memory")), "staggered"),
        (normalize_path(&format!("{home}\\.claude")), "trashcan"),
    ];
    let (mut found, mut hardened) = (0usize, 0usize);
    for (path, vtype) in &want {
        let Some(f) = folders.iter().find(|f| {
            f.get("path")
                .and_then(|p| p.as_str())
                .map(|p| normalize_path(p) == *path)
                .unwrap_or(false)
        }) else {
            continue;
        };
        found += 1;
        if f.get("versioning").and_then(|v| v.get("type")).and_then(|t| t.as_str()) == Some(vtype) {
            hardened += 1;
        }
    }
    Some((found, hardened))
}

/// Machine scan for the onboarding checklist. Pure detection — no writes, no elevation, no
/// process spawns (the one network touch is the local Syncthing REST read, 1.5s-capped).
fn onboarding_scan() -> Vec<OnbStep> {
    let lang = cur_lang();
    let mut out: Vec<OnbStep> = Vec::new();
    // Prerequisites: PATH-resolvable CLIs (exe_on_path — no process spawns).
    // prereq_pwsh FIRST: pwsh is what every maintenance script is spawned with — it gates the rest.
    for (id, name) in [("prereq_pwsh", "pwsh"), ("prereq_git", "git"), ("prereq_node", "node"), ("prereq_claude", "claude")] {
        out.push(match exe_on_path(name) {
            Some(p) => onb(id, "ok", p.to_string_lossy().into_owned(), None),
            None => onb(id, "todo", trv("onb.not_on_path", lang, &[("name", &name)]), None),
        });
    }
    // Syncthing is optional; presence = its config.xml exists.
    let st_cfg = std::env::var("LOCALAPPDATA")
        .map(|l| format!("{l}\\Syncthing\\config.xml"))
        .ok()
        .filter(|p| std::path::Path::new(p).is_file());
    out.push(match &st_cfg {
        Some(p) => onb("prereq_syncthing", "ok", p.clone(), None),
        None => onb("prereq_syncthing", "todo", tr("onb.syncthing_cfg_missing", lang).into(), None),
    });
    // Settings tree = the ClaudeProfiles source of truth (arrives via Syncthing / copy / backup).
    // De-hardcoded: profiles_root() detects the real tree (ASCII `SettingsMCP` or legacy Cyrillic).
    let tree = profiles_root();
    let tree_ok = std::path::Path::new(&tree).is_dir();
    out.push(if tree_ok {
        onb("tree", "ok", tree.clone(), None)
    } else {
        onb("tree", "todo", trv("onb.tree_missing", lang, &[("path", &tree)]), Some("backup_tab"))
    });
    // ASCII-safe path to the tree at <scripts_root>\SettingsMCP. Direction-agnostic: OK when
    // SettingsMCP either IS the detected tree (owner de-Cyrillicized the real folder) or is a reparse
    // point aimed at it (legacy: real Cyrillic folder + ASCII junction). A plain dir that isn't the
    // tree, or a junction to an old tree, is drift.
    let junction = format!("{}\\SettingsMCP", scripts_root());
    let jpath = std::path::Path::new(&junction);
    let tree_root = settings_tree_root();
    let jstate: Option<Result<(), String>> = if link_target_matches(jpath, &tree_root) {
        Some(Ok(())) // SettingsMCP is itself the tree — no separate junction needed
    } else if is_reparse_point(jpath) {
        Some(match std::fs::read_link(jpath) {
            Ok(t) if link_target_matches(&t, &tree_root) => Ok(()),
            Ok(t) => Err(trv("onb.junction_wrong_target", lang, &[("target", &t.display())])),
            Err(_) => Err(tr("onb.junction_unreadable", lang).into()),
        })
    } else if jpath.is_dir() {
        Some(Err(tr("onb.junction_plain_dir", lang).into()))
    } else {
        None // absent
    };
    out.push(match (jstate, tree_ok) {
        (Some(Ok(())), _) => onb("junction", "ok", junction, None),
        (Some(Err(d)), _) => onb("junction", "todo", d, Some("junction")),
        (None, true) => onb("junction", "todo", trv("onb.missing_path", lang, &[("path", &junction)]), Some("junction")),
        (None, false) => onb("junction", "blocked", String::new(), None),
    });
    // Profiles: expected list from profiles.json vs `~\.claude-<name>` dirs on disk.
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let names = profile_names();
    let present = names
        .iter()
        .filter(|n| std::path::Path::new(&format!("{home}\\.claude-{n}")).is_dir())
        .count();
    out.push(if !tree_ok {
        onb("profiles", "blocked", String::new(), None)
    } else if present == names.len() {
        onb("profiles", "ok", format!("{present}/{}", names.len()), None)
    } else {
        onb("profiles", "todo", format!("{present}/{}", names.len()), Some("install_profiles"))
    });
    // Credentials: presence only — the file is never read.
    let creds = format!("{home}\\.claude\\.credentials.json");
    out.push(if std::path::Path::new(&creds).is_file() {
        onb("creds", "ok", creds, None)
    } else {
        onb("creds", "todo", trv("onb.creds_missing", lang, &[("path", &creds)]), Some("backup_tab"))
    });
    // MCP canon deployed: every profile carries every canon server (matrix V2 reconcile reuse).
    // Unreadable canon → "unknown" WITHOUT a deploy fix (deploying against a broken .mcp.json is
    // not the cure) — never "ok" (the old fail-open painted a broken canon green).
    out.push(if !tree_ok {
        onb("mcp", "blocked", String::new(), None)
    } else {
        match mcp_deployable_canon() {
            None => onb("mcp", "unknown", tr("onb.mcp_canon_unreadable", lang).into(), None),
            Some(canon) => {
                let missing = names
                    .iter()
                    .filter(|n| {
                        let deployed = profile_mcp_servers(n).unwrap_or_default();
                        mcp_split(&canon, &deployed).0.len() < canon.len()
                    })
                    .count();
                if missing == 0 {
                    onb("mcp", "ok", trv("onb.mcp_ok", lang, &[("n", &canon.len())]), None)
                } else {
                    onb("mcp", "todo", trv("onb.mcp_missing", lang, &[("n", &missing)]), Some("mcp_deploy"))
                }
            }
        }
    });
    // Managed settings: source↔deployed comparison (stack-drift reuse).
    out.push(if !tree_ok {
        onb("managed", "blocked", String::new(), None)
    } else {
        let m = managed_settings_drift_item();
        match m.state.as_str() {
            "ok" => onb("managed", "ok", m.detail, None),
            _ => onb("managed", "todo", m.detail, Some("managed_deploy")),
        }
    });
    // Syncthing hardening IS natively detectable: the script's effect is versioning on the managed
    // folders (staggered on Scripts/.memory, trashcan on ~/.claude), readable over the same REST
    // infra the limits watcher uses. REST down / no API key / no managed folders → "unknown"
    // (the script is idempotent and self-skipping — offer the run).
    out.push(match (&st_cfg, tree_ok) {
        (None, _) | (_, false) => onb("syncthing", "blocked", String::new(), None),
        _ => match syncthing_hardening_state() {
            Some((total, hardened)) if total > 0 && hardened == total => {
                onb("syncthing", "ok", trv("onb.st_hardened", lang, &[("n", &total)]), None)
            }
            Some((total, hardened)) if total > 0 => onb(
                "syncthing",
                "todo",
                trv("onb.st_partial", lang, &[("done", &hardened), ("total", &total)]),
                Some("syncthing"),
            ),
            _ => onb("syncthing", "unknown", String::new(), Some("syncthing")),
        },
    });
    // Final gate: Assert-Installation.ps1 (non-zero exit on any failure) — streamed to console.
    out.push(if tree_ok {
        onb("verify", "unknown", String::new(), Some("verify"))
    } else {
        onb("verify", "blocked", String::new(), None)
    });
    out
}

#[tauri::command]
async fn read_onboarding() -> Result<Vec<OnbStep>, String> {
    tokio::task::spawn_blocking(onboarding_scan)
        .await
        .map_err(|e| e.to_string())
}

/// Streamed onboarding actions not wrapped by other commands.
#[tauri::command]
async fn run_onboarding_step(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
) -> Result<i32, String> {
    let rel = match action.as_str() {
        "syncthing" => SYNCTHING_SCRIPT_REL,
        "verify" => ASSERT_SCRIPT_REL,
        _ => {
            return Err(trv("err.unknown_onb_action", cur_lang(), &[("action", &action)]));
        }
    };
    spawn_streamed(app, state, stream_id::ONBOARDING.to_string(), abs(rel), Vec::new()).await
}

/// Create the ASCII junction <scripts_root>\SettingsMCP → the Cyrillic settings dir.
/// Junctions need no elevation (unlike symlinks); verified by a fresh probe after mklink.
#[tauri::command]
async fn create_settings_junction() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let target = settings_tree_root();
        let link = format!("{}\\SettingsMCP", scripts_root());
        let lpath = std::path::Path::new(&link);
        // De-Cyrillicized topology: SettingsMCP already IS the tree → nothing to create.
        if link_target_matches(lpath, &target) {
            return Ok(());
        }
        if lpath.exists() || is_reparse_point(lpath) {
            // Idempotent only when it's already a junction AIMED AT the tree; a plain dir or a
            // junction to an old tree is a conflict the user must resolve (we never delete here).
            if is_reparse_point(lpath)
                && std::fs::read_link(lpath)
                    .map(|t| link_target_matches(&t, &target))
                    .unwrap_or(false)
            {
                return Ok(());
            }
            return Err(trv(
                "err.junction_conflict",
                cur_lang(),
                &[("link", &link), ("target", &target)],
            ));
        }
        if !std::path::Path::new(&target).is_dir() {
            return Err(format!("target missing: {target}"));
        }
        let st = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &link, &target])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| e.to_string())?;
        if std::path::Path::new(&link).is_dir() {
            Ok(())
        } else {
            Err(format!("mklink /J failed (exit {:?})", st.code()))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// --- Ф2-GC: stack garbage collector (Home card) ---
//
// Plugin updates leave junk behind in the physical plugin store
// (~\.claude\plugins\cache\<org>\<plugin>\<ver>): superseded versions, `temp_git_*` clone
// leftovers, `.bak` copies, and foreign-OS (darwin/linux) binaries. read_gc_scan itemizes them;
// run_gc_delete recycles the deletable ones (never active versions, wrong_os is report-only).
// Profile `plugins` dirs are junctions into the one physical store, so we scan physical stores only.

/// One garbage item. Field names ARE the IPC contract (no serde rename).
#[derive(Serialize, Clone)]
struct GcItem {
    /// Stable id: "stale:<org>/<plugin>/<ver>" | "tempgit:<dir>" | "bak:<name>" | "wrongos:<store>".
    id: String,
    /// "stale_version" | "temp_git" | "bak" | "wrong_os" (derived from the id prefix).
    category: String,
    label: String,
    path: String,
    size_bytes: u64,
    /// wrong_os is report-only (false); the rest are recyclable.
    deletable: bool,
}

#[derive(Serialize)]
struct GcDeleteReport {
    /// ids actually recycled.
    deleted: Vec<String>,
    /// (id, short english reason) for each id we refused / failed to delete.
    skipped: Vec<(String, String)>,
    freed_bytes: u64,
}

/// Map an id prefix to its category. Single source of truth used at item construction. Unit-tested.
fn gc_id_category(id: &str) -> Option<&'static str> {
    if id.starts_with("stale:") {
        Some("stale_version")
    } else if id.starts_with("tempgit:") {
        Some("temp_git")
    } else if id.starts_with("bak:") {
        Some("bak")
    } else if id.starts_with("wrongos:") {
        Some("wrong_os")
    } else {
        None
    }
}

fn gc_item(id: String, label: String, path: String, size_bytes: u64, deletable: bool) -> GcItem {
    let category = gc_id_category(&id).unwrap_or("").to_string();
    GcItem { category, id, label, path, size_bytes, deletable }
}

/// DirEntry list for a dir, or empty on any IO error (a per-store failure must not abort the scan).
fn gc_read_dir(p: &std::path::Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(p).into_iter().flatten().flatten().collect()
}

/// Physical plugin stores: `~\.claude\plugins` + `~\.claude-<name>\plugins` per profile, minus any
/// that are missing or reparse points (profile stores junction into the canonical one — skip so we
/// count each physical tree exactly once).
fn gc_stores(home: &str) -> Vec<(String, std::path::PathBuf)> {
    let mut names = vec![".claude".to_string()];
    names.extend(profile_names().into_iter().map(|n| format!(".claude-{n}")));
    let mut out = Vec::new();
    for n in &names {
        let p = std::path::Path::new(home).join(n).join("plugins");
        if p.is_dir() && !is_reparse_point(&p) {
            out.push((n.clone(), p));
        }
    }
    out
}

/// Recursive byte size, depth-capped, reparse points NOT dereferenced (counted as 0), read errors
/// skipped. A file path returns its own length.
fn gc_dir_size(path: &std::path::Path) -> u64 {
    fn walk(p: &std::path::Path, depth: u32) -> u64 {
        if depth > 32 {
            return 0;
        }
        let Ok(md) = p.symlink_metadata() else {
            return 0;
        };
        if md.file_type().is_file() {
            return md.len();
        }
        if is_reparse_point(p) || !md.is_dir() {
            return 0;
        }
        gc_read_dir(p).iter().map(|e| walk(&e.path(), depth + 1)).sum()
    }
    walk(path, 0)
}

/// Extract `(org, plugin, ver)` from an installPath's tail after `...\plugins\cache\`. Marker match
/// is case-insensitive and slash-agnostic; components keep their original case. None if the marker
/// is absent or fewer than three components follow. Unit-tested.
fn install_path_tuple(install_path: &str) -> Option<(String, String, String)> {
    let norm = install_path.replace('/', "\\");
    let lower = norm.to_ascii_lowercase();
    let marker = "\\plugins\\cache\\";
    let pos = lower.find(marker)?;
    let tail = &norm[pos + marker.len()..];
    let mut comps = tail.split('\\').filter(|s| !s.is_empty());
    let org = comps.next()?.to_string();
    let plugin = comps.next()?.to_string();
    let ver = comps.next()?.to_string();
    Some((org, plugin, ver))
}

type GcActiveSets = (
    std::collections::HashSet<(String, String)>,
    std::collections::HashSet<(String, String, String)>,
    // Dirnames of stores whose installed_plugins.json was read+parsed OK. Stale detection is
    // limited to these: an unreadable manifest means this store's active versions were never
    // blessed, and a blessing from ANOTHER store would then flag them as stale (fail-closed).
    std::collections::HashSet<String>,
);

/// Global active-version sets (lowercased) over every store's installed_plugins.json:
/// (org, plugin) pairs and (org, plugin, ver) triples. installPaths point through any profile alias,
/// so the version is blessed across all physical stores.
fn gc_active_sets(stores: &[(String, std::path::PathBuf)]) -> GcActiveSets {
    let mut pairs = std::collections::HashSet::new();
    let mut triples = std::collections::HashSet::new();
    let mut manifest_ok = std::collections::HashSet::new();
    for (dirname, store) in stores {
        let Ok(txt) = std::fs::read_to_string(store.join("installed_plugins.json")) else {
            continue;
        };
        let Ok(v) = parse_json_bom(&txt) else {
            continue;
        };
        manifest_ok.insert(dirname.clone());
        let Some(plugins) = v.get("plugins").and_then(|p| p.as_object()) else {
            continue;
        };
        for arr in plugins.values() {
            let Some(entries) = arr.as_array() else {
                continue;
            };
            for e in entries {
                if let Some(ip) = e.get("installPath").and_then(|x| x.as_str()) {
                    if let Some((org, plugin, ver)) = install_path_tuple(ip) {
                        let o = org.to_ascii_lowercase();
                        let p = plugin.to_ascii_lowercase();
                        let ver = ver.to_ascii_lowercase();
                        pairs.insert((o.clone(), p.clone()));
                        triples.insert((o, p, ver));
                    }
                }
            }
        }
    }
    (pairs, triples, manifest_ok)
}

/// A version dir is stale iff its (org, plugin) has SOME active version but THIS version isn't it.
/// A pair with no active version at all is left alone (conservative). Case-insensitive. Unit-tested.
fn is_stale_ver(
    org: &str,
    plugin: &str,
    ver: &str,
    pairs: &std::collections::HashSet<(String, String)>,
    triples: &std::collections::HashSet<(String, String, String)>,
) -> bool {
    let o = org.to_ascii_lowercase();
    let p = plugin.to_ascii_lowercase();
    let key3 = (o.clone(), p.clone(), ver.to_ascii_lowercase());
    pairs.contains(&(o, p)) && !triples.contains(&key3)
}

/// True if `token` appears in `name_lower` as a whole token — bounded by start/end of string or one
/// of `-_.` on each side. `token` and `name_lower` must be ASCII-lowercased. Unit-tested.
fn token_bounded(name_lower: &str, token: &str) -> bool {
    let bytes = name_lower.as_bytes();
    let mut from = 0;
    while let Some(rel) = name_lower[from..].find(token) {
        let start = from + rel;
        let end = start + token.len();
        let before_ok = start == 0 || matches!(bytes[start - 1], b'-' | b'_' | b'.');
        let after_ok = end == bytes.len() || matches!(bytes[end], b'-' | b'_' | b'.');
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Foreign-OS binary name? (`darwin`/`linux` as a bounded token, case-insensitive).
fn has_os_token(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    token_bounded(&lower, "darwin") || token_bounded(&lower, "linux")
}

/// Recursive walk of a cache dir summing foreign-OS entries; does not descend into a matched
/// subtree. Returns (total_bytes, entry_count).
fn gc_scan_wrong_os(cache: &std::path::Path) -> (u64, u64) {
    fn walk(dir: &std::path::Path, depth: u32, bytes: &mut u64, count: &mut u64) {
        if depth > 32 {
            return;
        }
        for e in gc_read_dir(dir) {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if has_os_token(&name) {
                *bytes += gc_dir_size(&p);
                *count += 1;
                continue; // matched: don't descend into its subtree
            }
            if p.is_dir() && !is_reparse_point(&p) {
                walk(&p, depth + 1, bytes, count);
            }
        }
    }
    let mut bytes = 0;
    let mut count = 0;
    walk(cache, 0, &mut bytes, &mut count);
    (bytes, count)
}

/// Scan all physical stores for garbage. Never errors: a per-store IO failure yields fewer items.
fn gc_scan(home: &str) -> Vec<GcItem> {
    let stores = gc_stores(home);
    let (pairs, triples, manifest_ok) = gc_active_sets(&stores);
    let mut items = Vec::new();
    for (dirname, store) in &stores {
        let cache = store.join("cache");

        // 1. stale versions: cache\<org>\<plugin>\<ver> (exactly 3 levels). Only in stores whose
        // own manifest was readable — otherwise this store's active versions were never blessed
        // and another store's blessing of the same pair would flag them (fail-closed).
        for org_e in gc_read_dir(&cache) {
            if !manifest_ok.contains(dirname) {
                break;
            }
            let org_p = org_e.path();
            if !org_p.is_dir() || is_reparse_point(&org_p) {
                continue;
            }
            let org = org_e.file_name().to_string_lossy().to_string();
            for pl_e in gc_read_dir(&org_p) {
                let pl_p = pl_e.path();
                if !pl_p.is_dir() || is_reparse_point(&pl_p) {
                    continue;
                }
                let plugin = pl_e.file_name().to_string_lossy().to_string();
                for ver_e in gc_read_dir(&pl_p) {
                    let ver_p = ver_e.path();
                    if !ver_p.is_dir() || is_reparse_point(&ver_p) {
                        continue;
                    }
                    let ver = ver_e.file_name().to_string_lossy().to_string();
                    if is_stale_ver(&org, &plugin, &ver, &pairs, &triples) {
                        items.push(gc_item(
                            // Qualify with the store dirname: without it two profiles' caches can
                            // produce the same id and run_gc_delete's first-match .find() resolves the
                            // wrong item (or misses one).
                            format!("stale:{dirname}:{org}/{plugin}/{ver}"),
                            format!("{plugin} {ver} ({org})"),
                            ver_p.to_string_lossy().into_owned(),
                            gc_dir_size(&ver_p),
                            true,
                        ));
                    }
                }
            }
        }

        // 2. temp_git_* leftover clone dirs at cache top-level.
        for e in gc_read_dir(&cache) {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            if name.starts_with("temp_git_") && p.is_dir() {
                items.push(gc_item(
                    format!("tempgit:{dirname}:{name}"),
                    name.clone(),
                    p.to_string_lossy().into_owned(),
                    gc_dir_size(&p),
                    true,
                ));
            }
        }

        // 3. .bak files/dirs at store and cache top-levels (case-insensitive). Suffix match only:
        // a bare `contains(".bak")` would also flag `.backup`, `x.bak2`, `x.bak.zip` for deletion.
        for (base_tag, base) in [("store", store.as_path()), ("cache", cache.as_path())] {
            for e in gc_read_dir(base) {
                let name = e.file_name().to_string_lossy().to_string();
                if name.to_ascii_lowercase().ends_with(".bak") {
                    let p = e.path();
                    let size = if p.is_dir() {
                        gc_dir_size(&p)
                    } else {
                        p.symlink_metadata().map(|m| m.len()).unwrap_or(0)
                    };
                    items.push(gc_item(
                        // Qualify with store dirname + which base (store vs cache) so same-named .bak
                        // entries across profiles/bases don't collide on run_gc_delete's first-match find.
                        format!("bak:{dirname}:{base_tag}:{name}"),
                        name.clone(),
                        p.to_string_lossy().into_owned(),
                        size,
                        true,
                    ));
                }
            }
        }

        // 4. wrong_os aggregate (report-only): one item per store.
        let (wrong_bytes, wrong_count) = gc_scan_wrong_os(&cache);
        if wrong_count > 0 {
            items.push(gc_item(
                format!("wrongos:{dirname}"),
                format!("{wrong_count} darwin/linux entries"),
                store.to_string_lossy().into_owned(),
                wrong_bytes,
                false,
            ));
        }
    }
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    items
}

/// Itemize stack garbage across physical plugin stores. Err only when USERPROFILE is unset.
#[tauri::command]
async fn read_gc_scan() -> Result<Vec<GcItem>, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || gc_scan(&home))
        .await
        .map_err(|e| e.to_string())
}

/// Recycle the requested gc ids. Defense-in-depth (the UI already confirmed): a fresh rescan must
/// still list the id, it must be deletable, live under a physical `plugins` store, not be a reparse
/// point (nor, for dirs, contain reparse children), and actually vanish after the recycle call.
#[tauri::command]
async fn run_gc_delete(ids: Vec<String>) -> Result<GcDeleteReport, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let stores = gc_stores(&home);
        let items = gc_scan(&home);
        let mut report = GcDeleteReport {
            deleted: Vec::new(),
            skipped: Vec::new(),
            freed_bytes: 0,
        };
        for id in ids {
            let Some(item) = items.iter().find(|i| i.id == id) else {
                report.skipped.push((id, "not found on rescan".into()));
                continue;
            };
            if !item.deletable {
                report.skipped.push((id, "not deletable".into()));
                continue;
            }
            let path = std::path::Path::new(&item.path);
            // Must be strictly inside a known physical plugins store (component-wise prefix).
            let under_store = stores
                .iter()
                .any(|(_, sp)| path.starts_with(sp) && path != sp.as_path());
            if !under_store {
                report.skipped.push((id, "path outside plugins store".into()));
                continue;
            }
            if is_reparse_point(path) {
                report.skipped.push((id, "target is a reparse point".into()));
                continue;
            }
            let Ok(md) = path.symlink_metadata() else {
                report.skipped.push((id, "target vanished".into()));
                continue;
            };
            let is_dir = md.file_type().is_dir();
            if is_dir && has_reparse_child(path) {
                report.skipped.push((id, "contains reparse children".into()));
                continue;
            }
            let recycled = if is_dir {
                recycle_dir(&item.path)
            } else {
                recycle_file(&item.path)
            };
            if let Err(e) = recycled {
                report.skipped.push((id, e));
                continue;
            }
            if path.exists() {
                report.skipped.push((id, "still present after recycle".into()));
                continue;
            }
            report.freed_bytes += item.size_bytes;
            report.deleted.push(id);
        }
        report
    })
    .await
    .map_err(|e| e.to_string())
}

// --- Agent-status lifecycle hook (Sessions tab) ---
//
// castellyn_status.py reports Claude Code lifecycle events (working/blocked/idle) into
// %APPDATA%\castellyn\agent-status; the agent_status module turns them into pane badges.
// Wired into the lifecycle events of every profile; a session without CASTELLYN_SESSION_ID
// in its env makes the hook a no-op, so regular (non-Castellyn) Claude use is unaffected.
// v2 added the tool-use heartbeat events (working stays fresh through a quiet tool call)
// and the explicit waiting-on-human ones; an event name a given Claude build doesn't know
// is simply never fired — wiring it is harmless.

const STATUS_HOOK_SCRIPT: &str = include_str!("../assets/castellyn_status.py");
const STATUS_HOOK_CMD: &str = "py -X utf8 ~/.claude/hooks/castellyn_status.py";
const STATUS_HOOK_MARKER: &str = "castellyn_status.py";
const STATUS_HOOK_EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Notification",
    "PermissionRequest",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

/// The single on-disk location of the status hook script — one shared file (STATUS_HOOK_CMD wires the
/// literal `~/.claude/hooks/…`, the same for every profile), so its presence is one check, not N.
fn status_hook_script_path(home: &str) -> String {
    format!("{home}\\.claude\\hooks\\castellyn_status.py")
}

/// Whether the wired hook command actually has a script to run. A profile can reference the hook in
/// settings while the file was never installed or later deleted — then the hook silently no-ops and
/// panes go stale-quiet. Surfacing this is the point of the diagnostic (acceptance #1: script state).
fn status_hook_script_present(home: &str) -> bool {
    std::path::Path::new(&status_hook_script_path(home)).exists()
}

/// Write `content` to `path` only when the on-disk copy is an older (or missing) version, keyed by a
/// `…-version:` header line — the shared version-gated install policy for the hook/notifier/plugin.
fn ensure_versioned_file(path: &str, content: &str, marker: &str) -> Result<(), String> {
    let ver = |t: &str| script_version_header(t, marker);
    let disk = std::fs::read_to_string(path).unwrap_or_default();
    if ver(&disk) < ver(content) {
        write_json_atomic(path, content).map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(())
}

/// Install/refresh the status hook script (same version-gated policy as plugin_sync).
fn ensure_status_hook_script(home: &str) -> Result<(), String> {
    ensure_versioned_file(
        &status_hook_script_path(home),
        STATUS_HOOK_SCRIPT,
        "# castellyn-status-version:",
    )
}

// --- Codex end-of-turn notifier (Sessions tab) ---
//
// Codex 0.142 grew a hooks framework, but it is configured in the user's `~/.codex/config.toml` —
// state we refuse to own. Its older `notify` program, by contrast, is settable per invocation with
// `-c`, so the pane carries its own notifier and nothing outside Castellyn's directory is touched.
// It fires once per finished turn, which is the authoritative "done" the PTY heartbeat can only
// guess at (see agent_status: no completion toast without a real turn signal).
//
// pwsh, not `py`: the pane is already launched through pwsh, so this adds no new dependency.

const NOTIFY_SCRIPT: &str = include_str!("../assets/castellyn_notify.ps1");

fn notify_script_path() -> Result<String, String> {
    let base = std::env::var("APPDATA").map_err(|e| e.to_string())?;
    Ok(format!("{base}\\castellyn\\hooks\\castellyn_notify.ps1"))
}

/// Install/refresh the notifier (same version-gated policy as the status hook).
fn ensure_notify_script() -> Result<String, String> {
    let path = notify_script_path()?;
    ensure_versioned_file(&path, NOTIFY_SCRIPT, "# castellyn-notify-version:")?;
    Ok(path)
}

/// The `-c` argument that points codex at our notifier, quoted for the pwsh `-Command` line it
/// rides in.
///
/// The path uses FORWARD slashes on purpose. PowerShell collapses a run of backslashes that precedes
/// a quote while it builds the child's command line, so a TOML-escaped `"C:\\Users\\…"` arrives at
/// codex as `"C:\Users\…"` — where `\U` is not a valid TOML escape. Codex then silently keeps the
/// whole array as a *string* and refuses to start: `invalid type: string …, expected a sequence`
/// (reproduced live, 2026-07-10). Windows accepts `/` in every path API, so the escape never happens.
///
/// The remaining escape is PowerShell's: a single-quoted argument doubles every apostrophe, which a
/// user directory may legally contain.
fn codex_notify_arg(script: &str) -> String {
    let path = script.replace('\\', "/");
    let value = format!(r#"notify=["pwsh","-NoLogo","-NoProfile","-File","{path}"]"#);
    format!("-c '{}'", value.replace('\'', "''"))
}

// --- opencode status plugin (Sessions tab) ---
//
// opencode has no notifier, but `OPENCODE_CONFIG_CONTENT` is documented (in its own binary) as
// "inject inline JSON as a final local-scope merge", and plugin specs from it are APPENDED to the
// user's list rather than replacing it. So the pane carries its own reporter through an env var and
// the user's `opencode.json` is never touched. The plugin registers no hooks at all when
// CASTELLYN_SESSION_ID is absent, so an opencode launched by hand is unaffected.

const OPENCODE_PLUGIN: &str = include_str!("../assets/castellyn_opencode_plugin.js");

fn opencode_plugin_path() -> Result<String, String> {
    let base = std::env::var("APPDATA").map_err(|e| e.to_string())?;
    Ok(format!(
        "{base}\\castellyn\\hooks\\castellyn_opencode_plugin.js"
    ))
}

fn ensure_opencode_plugin() -> Result<String, String> {
    let path = opencode_plugin_path()?;
    ensure_versioned_file(&path, OPENCODE_PLUGIN, "// castellyn-plugin-version:")?;
    Ok(path)
}

/// The inline config that appends our reporter to opencode's plugin list. Rides in an environment
/// variable, so no shell quoting is involved — only JSON, and a `file:///` URL with forward slashes.
fn opencode_plugin_config(plugin: &str) -> String {
    let url = format!("file:///{}", plugin.replace('\\', "/"));
    serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "plugin": [url],
    })
    .to_string()
}

/// Lifecycle events (from STATUS_HOOK_EVENTS) whose command is NOT wired in `settings`, returned
/// in canonical STATUS_HOOK_EVENTS order. Empty = fully wired. Read-only; a malformed/absent hooks
/// value simply yields every event (fully unwired) rather than panicking.
fn status_hook_missing_events(settings: &serde_json::Value) -> Vec<&'static str> {
    STATUS_HOOK_EVENTS
        .iter()
        .copied()
        .filter(|ev| !hook_cmd_wired(settings, ev, STATUS_HOOK_MARKER))
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileHookGaps {
    profile: String,
    /// Lifecycle events still missing for this profile, in STATUS_HOOK_EVENTS order.
    missing: Vec<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatusHookState {
    /// Profile dir names with ALL lifecycle events wired.
    wired: Vec<String>,
    unwired: Vec<String>,
    /// Profiles wired for SOME but not all events, with the exact events still missing. Separates
    /// drift/partial wiring from a profile that was simply never enabled (all events missing — that
    /// one stays only in `unwired`). Read-only diagnostic.
    partial: Vec<ProfileHookGaps>,
    /// Whether the hook script exists on disk. `false` while any profile is `wired` means the command
    /// is referenced but has nothing to run — a silent-failure state the UI must flag.
    script_present: bool,
}

fn agent_status_hook_state() -> Result<AgentStatusHookState, String> {
    let home = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let (mut wired, mut unwired, mut partial) = (Vec::new(), Vec::new(), Vec::new());
    for (name, sp) in plugin_sync_profiles(&home) {
        // Unreadable/malformed settings.json → treated as fully unwired (every event missing).
        let missing = std::fs::read_to_string(&sp)
            .ok()
            .and_then(|c| parse_json_bom(&c).ok())
            .map(|v| status_hook_missing_events(&v))
            .unwrap_or_else(|| STATUS_HOOK_EVENTS.to_vec());
        if missing.is_empty() {
            wired.push(name);
        } else {
            if missing.len() < STATUS_HOOK_EVENTS.len() {
                partial.push(ProfileHookGaps {
                    profile: name.clone(),
                    missing,
                });
            }
            unwired.push(name);
        }
    }
    Ok(AgentStatusHookState {
        wired,
        unwired,
        partial,
        script_present: status_hook_script_present(&home),
    })
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
    fn tmp_secret_base_recovers_target_from_legacy_and_unique_temp_names() {
        // Legacy "<file>.tmp" and unique "<file>.<pid>.<seq>.tmp" both resolve to the target file, so
        // the secret-tmp sweep still recognises them (L91/L92).
        assert_eq!(tmp_secret_base("settings.json.tmp"), Some("settings.json"));
        assert_eq!(tmp_secret_base("settings.json.1234.5.tmp"), Some("settings.json"));
        assert_eq!(
            tmp_secret_base(".credentials.json.999.0.tmp"),
            Some(".credentials.json")
        );
        assert_eq!(tmp_secret_base("not-a-temp.json"), None);
    }

    #[test]
    fn is_stale_ver_classifies_only_old_versions_of_installed_plugins() {
        // gc deletability classification (L138): an OLD version of an INSTALLED plugin is stale; the
        // active version and any uninstalled plugin's dirs are kept.
        let pairs: std::collections::HashSet<(String, String)> =
            [("acme".to_string(), "widget".to_string())].into_iter().collect();
        let triples: std::collections::HashSet<(String, String, String)> =
            [("acme".to_string(), "widget".to_string(), "2.0.0".to_string())]
                .into_iter()
                .collect();
        assert!(is_stale_ver("acme", "widget", "1.0.0", &pairs, &triples)); // old → stale
        assert!(!is_stale_ver("acme", "widget", "2.0.0", &pairs, &triples)); // active → keep
        assert!(!is_stale_ver("other", "thing", "1.0.0", &pairs, &triples)); // uninstalled → keep
        assert!(is_stale_ver("ACME", "Widget", "1.0.0", &pairs, &triples)); // case-insensitive
    }

    #[test]
    fn valid_base_url_scheme_is_case_insensitive() {
        // L8: RFC 3986 schemes are case-insensitive — HTTP:// / HTTPS:// must be accepted.
        assert!(valid_base_url("HTTP://127.0.0.1:1234").is_ok());
        assert!(valid_base_url("HttpS://127.0.0.1:8080").is_ok());
    }

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
    fn valid_ssh_target_blocks_option_injection() {
        // Rejected: leading-dash option injection, embedded spaces (a second `-o…` token), empty.
        for bad in [
            "-oProxyCommand=calc.exe host",
            "-oProxyCommand=calc.exe",
            "host -oProxyCommand=calc.exe",
            "user@host; calc",
            "",
            "   ",
        ] {
            assert!(!valid_ssh_target(bad), "should reject {bad:?}");
        }
        // Allowed: ordinary targets, aliases, IPv4/IPv6 literals, user@host.
        for ok in ["host", "user@host.example.com", "my-server_1", "192.168.1.10", "user@[::1]", "10.0.0.5"] {
            assert!(valid_ssh_target(ok), "should allow {ok:?}");
        }
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
        // Wiring every lifecycle event must be idempotent and reversible without
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
    fn status_hook_missing_events_names_partial_gaps() {
        // No wiring at all → every lifecycle event is reported missing, in canonical order.
        let empty = json!({});
        assert_eq!(
            super::status_hook_missing_events(&empty),
            super::STATUS_HOOK_EVENTS.to_vec()
        );
        // A malformed hooks value is treated as fully unwired, not a panic or partial.
        let bad = json!({ "hooks": "oops" });
        assert_eq!(
            super::status_hook_missing_events(&bad),
            super::STATUS_HOOK_EVENTS.to_vec()
        );
        // Wire three of the events → the diagnostic names exactly the ones still missing,
        // preserving STATUS_HOOK_EVENTS order (not the order they were wired in).
        let mut v = json!({});
        for ev in ["Stop", "SessionStart", "UserPromptSubmit"] {
            super::hook_cmd_wire(&mut v, ev, super::STATUS_HOOK_CMD, super::STATUS_HOOK_MARKER);
        }
        assert_eq!(
            super::status_hook_missing_events(&v),
            vec![
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "Notification",
                "PermissionRequest",
                "StopFailure",
                "SessionEnd"
            ]
        );
        // Fully wired → nothing missing.
        for ev in [
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "Notification",
            "PermissionRequest",
            "StopFailure",
            "SessionEnd",
        ] {
            super::hook_cmd_wire(&mut v, ev, super::STATUS_HOOK_CMD, super::STATUS_HOOK_MARKER);
        }
        assert!(super::status_hook_missing_events(&v).is_empty());
    }

    #[test]
    fn status_hook_script_presence_tracks_the_file() {
        // Hermetic fake HOME under the temp dir — never touches the real ~/.claude.
        let home = std::env::temp_dir().join(format!("castellyn_shp_{}", std::process::id()));
        let home_s = home.to_string_lossy().to_string();
        let _ = std::fs::remove_dir_all(&home);
        // Absent: settings may reference the hook, but the script file is not there.
        assert!(!super::status_hook_script_present(&home_s));
        // The resolved path is the single canonical shared location (not per-profile).
        let path = super::status_hook_script_path(&home_s);
        assert!(path.ends_with("\\.claude\\hooks\\castellyn_status.py"));
        // Present once the file exists at exactly that path.
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        std::fs::write(&path, "# stub").unwrap();
        assert!(super::status_hook_script_present(&home_s));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn status_hook_diagnostic_ignores_and_preserves_unrelated_hooks() {
        // An unrelated hook already lives on a lifecycle event we also wire. It must never be
        // counted as ours by the diagnostic, and must survive our wire + unwire untouched.
        let mut v = json!({ "hooks": { "SessionStart": [
            { "hooks": [{ "type": "command", "command": "py other_hook.py" }] }
        ]}});
        // The neighbour's presence does NOT make SessionStart count as wired for us.
        assert_eq!(
            super::status_hook_missing_events(&v),
            super::STATUS_HOOK_EVENTS.to_vec()
        );
        for ev in super::STATUS_HOOK_EVENTS {
            super::hook_cmd_wire(&mut v, ev, super::STATUS_HOOK_CMD, super::STATUS_HOOK_MARKER);
        }
        assert!(super::status_hook_missing_events(&v).is_empty());
        // The foreign neighbour still sits alongside ours on the shared event.
        let ss = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 2);
        assert_eq!(ss[0]["hooks"][0]["command"], "py other_hook.py");
        // Unwiring ours reverts the diagnostic to fully-missing and leaves the neighbour verbatim.
        for ev in super::STATUS_HOOK_EVENTS {
            super::hook_cmd_unwire(&mut v, ev, super::STATUS_HOOK_MARKER);
        }
        assert_eq!(
            super::status_hook_missing_events(&v),
            super::STATUS_HOOK_EVENTS.to_vec()
        );
        let ss = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1);
        assert_eq!(ss[0]["hooks"][0]["command"], "py other_hook.py");
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

    #[test]
    fn json_normalized_eq_tolerates_bom_and_key_order() {
        // A leading BOM on one side and a different key order both normalize to equal.
        assert!(super::json_normalized_eq(
            "\u{feff}{\"a\":1,\"b\":2}",
            "{\"b\":2,\"a\":1}"
        ));
        // Genuinely different content is not equal.
        assert!(!super::json_normalized_eq("{\"a\":1}", "{\"a\":2}"));
        // A parse failure on either side is not equal (never a false "match").
        assert!(!super::json_normalized_eq("{\"a\":1}", "not json"));
    }

    #[test]
    fn plugin_sync_file_drift_classification() {
        // render_plugin_sync_script is the oracle — an identical disk copy is ok.
        let dirs = vec![".claude".to_string(), ".claude-cc1".to_string()];
        let rendered = super::render_plugin_sync_script(&dirs);

        let (state, _d, fix) = super::classify_plugin_sync_file(Some(&rendered), &rendered);
        assert_eq!(state, "ok");
        assert!(fix.is_none());

        // Absent → missing, auto-fixable.
        let (state, _d, fix) = super::classify_plugin_sync_file(None, &rendered);
        assert_eq!(state, "missing");
        assert_eq!(fix.as_deref(), Some("plugin_sync"));

        // External copy: no Castellyn marker, version ≤ embedded → drift, external detail.
        let external = "# plugin-sync-version: 1\nimport os\n";
        let (state, detail, fix) = super::classify_plugin_sync_file(Some(external), &rendered);
        assert_eq!(state, "drift");
        assert!(detail.contains("external"));
        assert_eq!(fix.as_deref(), Some("plugin_sync"));

        // Same version, marker present, different profile list → stale-list drift.
        let stale = super::render_plugin_sync_script(&[".claude".to_string()]);
        let (state, detail, fix) = super::classify_plugin_sync_file(Some(&stale), &rendered);
        assert_eq!(state, "drift");
        assert!(detail.contains("stale"));
        assert_eq!(fix.as_deref(), Some("plugin_sync"));

        // Newer/foreign version wins over the marker check — and offers NO fix: ensure_
        // plugin_sync_script never downgrades, so the fix button would be a no-op.
        let newer = "# plugin-sync-version: 99\n# castellyn:profiles\n";
        let (state, detail, fix) = super::classify_plugin_sync_file(Some(newer), &rendered);
        assert_eq!(state, "drift");
        assert!(detail.contains("newer"));
        assert_eq!(fix, None);
    }

    #[test]
    fn wiring_drift_classification() {
        // All wired, managed clean → ok.
        let (state, _d, fix) = super::classify_wiring(&[], false);
        assert_eq!(state, "ok");
        assert!(fix.is_none());

        // A profile missing wiring → drift, auto-fixable via plugin_sync.
        let (state, detail, fix) = super::classify_wiring(&["cc1".to_string()], false);
        assert_eq!(state, "drift");
        assert!(detail.contains("cc1"));
        assert_eq!(fix.as_deref(), Some("plugin_sync"));

        // Only double-wiring in managed → drift with NO auto-fix (fix is editing the source).
        let (state, detail, fix) = super::classify_wiring(&[], true);
        assert_eq!(state, "drift");
        assert!(detail.contains("double-wiring"));
        assert!(fix.is_none());

        // Both problems → the missing-wiring fix takes precedence.
        let (state, _d, fix) = super::classify_wiring(&["cc1".to_string()], true);
        assert_eq!(state, "drift");
        assert_eq!(fix.as_deref(), Some("plugin_sync"));
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

/// R7: serializes every config write so a read-modify-write is atomic (separate from CONFIG_CACHE's
/// RwLock, which only guards the read cache). Both patch_config and write_config hold it.
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// R7: the backend's safe config mutator — read-modify-write under the write lock, bumping `rev`.
/// Every backend writer (ledger updates, hotkeys, language) goes through this so a concurrent
/// frontend save sees a fresh rev and can't silently lose these fields (or have them lost).
fn patch_config(f: impl FnOnce(&mut HubConfig)) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = read_config_file();
    f(&mut cfg);
    cfg.rev = cfg.rev.wrapping_add(1);
    write_config_file(&cfg)
}

#[tauri::command]
fn write_config(mut config: HubConfig, expected_rev: Option<u64>) -> Result<u64, String> {
    // R7: optimistic concurrency — reject a save whose base rev is stale (someone wrote in between),
    // so the frontend can re-read + retry instead of clobbering the other writer's fields.
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let current = read_config_file();
    if let Some(exp) = expected_rev {
        if exp != current.rev {
            return Err("config-conflict".to_string()); // stable sentinel the frontend matches on
        }
    }
    // language is owned by set_language — a generic settings save must never clobber it.
    config.language = current.language;
    config.rev = current.rev.wrapping_add(1);
    write_config_file(&config)?;
    Ok(config.rev)
}

/// Mirror the UI locale into the backend: update the in-process Lang (so errors/log localize),
/// persist it in config (so the tray is correct at next startup too), and relabel the tray now.
#[tauri::command]
fn set_language(app: AppHandle, lang: String) -> Result<(), String> {
    let l = Lang::parse(&lang);
    set_cur_lang(l);
    patch_config(|c| c.language = Some(lang))?;
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
    // R8: bundle the durable forks.json alongside config so a settings export carries the fork
    // registry too (schemaVersion 2). import_config accepts both this and the legacy flat HubConfig.
    let forks = fork_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| parse_json_bom(&c).ok());
    let bundle = serde_json::json!({
        "schemaVersion": 2,
        "config": read_config_file(),
        "forks": forks,
    });
    let json = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    std::fs::write(&dest, json).map_err(|e| trv("err.write", cur_lang(), &[("e", &e)]))
}

/// Read + validate a config file (#117); returns the parsed HubConfig (the frontend persists it
/// via write_config). R8: accepts both the new bundle {schemaVersion, config, forks} — writing the
/// forks section to the durable path — and the legacy flat HubConfig. Invalid JSON / shape → Err.
#[tauri::command]
fn import_config(src: String) -> Result<HubConfig, String> {
    let text =
        std::fs::read_to_string(&src).map_err(|e| trv("err.read", cur_lang(), &[("e", &e)]))?;
    // BOM-tolerant like every other file read (PowerShell-written configs often carry one).
    let v: serde_json::Value = parse_json_bom(&text)
        .map_err(|e| trv("err.bad_config_file", cur_lang(), &[("e", &e.to_string())]))?;
    if v.get("schemaVersion").is_some() && v.get("config").is_some() {
        // New bundle: restore the fork registry to its durable path, return the config for the UI.
        if let Some(forks) = v.get("forks").filter(|f| !f.is_null()) {
            if let (Some(fp), Ok(fj)) = (fork_config_path(), serde_json::to_string_pretty(forks)) {
                let _ = write_json_atomic(&fp, &fj);
            }
        }
        serde_json::from_value::<HubConfig>(v.get("config").cloned().unwrap_or_default())
            .map_err(|e| trv("err.bad_config_file", cur_lang(), &[("e", &e.to_string())]))
    } else {
        // Legacy flat HubConfig (pre-R8 exports).
        serde_json::from_value::<HubConfig>(v)
            .map_err(|e| trv("err.bad_config_file", cur_lang(), &[("e", &e.to_string())]))
    }
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
        // `claude_cmd` is handed to `cmd /k` as one string and re-tokenized there, so quote any
        // path-bearing flag value that contains a space — an unquoted --mcp-config / --add-dir path
        // under e.g. `C:\Users\John Doe\...` would otherwise split into two args. Windows paths can't
        // contain a literal `"`, so simple double-quoting is safe.
        let flags = lean_flags(&name)
            .into_iter()
            .map(|f| if f.contains(' ') { format!("\"{f}\"") } else { f })
            .collect::<Vec<_>>()
            .join(" ");
        format!("claude {flags}")
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
    stack: State<'_, StackRun>,
    forks: State<'_, ForkRuns>,
    sessions: State<'_, SessionState>,
) -> Result<(), String> {
    // 1. The single-slot run (backup / profiles / sync / component / single plugin op).
    let run_pid = { *run.0.lock().unwrap_or_else(|e| e.into_inner()) };
    if let Some(p) = run_pid {
        if p != 0 {
            let _ = kill_tree(p);
        }
    }
    // 1b. The LLM-stack run (its own domain now) — kill whichever start/stop/restart phase is live.
    let stack_pid = { *stack.0.lock().unwrap_or_else(|e| e.into_inner()) };
    if let Some(p) = stack_pid {
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

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("Castellyn")
        .menu(&menu)
        .show_menu_on_left_click(false);
    // Degrade to an icon-less tray rather than panic if the window icon is somehow absent.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
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

/// Service ids we intentionally stopped, with the instant of the stop. The stack-health poll
/// consults this so a service WE just took down doesn't fire a spurious "service down" alert.
/// Entries older than STACK_EXPECTED_TTL are stale (an unexpected death) and no longer suppress.
pub(crate) static STACK_EXPECTED_DOWN: std::sync::OnceLock<Mutex<HashMap<String, std::time::Instant>>> =
    std::sync::OnceLock::new();

/// Mark a service id as expected-down as of now. Called from every native stop path.
pub(crate) fn mark_expected_down(id: &str) {
    STACK_EXPECTED_DOWN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), std::time::Instant::now());
}

/// Mark one service (or, with None, every configured service) expected-down — the script stop
/// path can't enumerate per-process like the native one, so it suppresses by scope.
pub(crate) fn mark_expected_down_scope(only: Option<&str>) {
    match only {
        Some(id) => mark_expected_down(id),
        None => {
            for svc in stack_services() {
                if let Some(sid) = svc.get("id").and_then(|x| x.as_str()) {
                    mark_expected_down(sid);
                }
            }
        }
    }
}

/// When (if ever) `id` was last marked expected-down. The stack-health poll pairs this with the TTL.
pub(crate) fn expected_down_at(id: &str) -> Option<std::time::Instant> {
    STACK_EXPECTED_DOWN
        .get()?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .copied()
}

/// OS-notify (Windows toast) for a genuinely important background event, for NEW callers (stack
/// down, run failed, schedule failed). No-op unless status_notify is on (default true). A failed
/// `.show()` is logged, never propagated — a monitor thread must not die on a toast error.
pub(crate) fn notify_important(app: &AppHandle, title: &str, body: &str) {
    if !read_config_file().status_notify.unwrap_or(true) {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notify_important: {e}");
    }
}

/// Reflect open session panes + any aggregate attention (agents waiting / limited, stack down) in
/// the tray tooltip. Called on session changes and from the health/agent poll threads so a
/// minimized user sees trouble on hover.
// ponytail: tooltip count, not a drawn overlay badge — add image-gen only if a visual badge is requested.
pub(crate) fn update_tray_tooltip(app: &AppHandle) {
    let n = app
        .state::<SessionState>()
        .0
        .lock()
        .map(|m| m.len())
        .unwrap_or(0);
    let lang = cur_lang();
    let mut label = if n == 0 {
        "Castellyn".to_string()
    } else {
        trv("tray.tooltip_sessions", lang, &[("n", &n.to_string())])
    };
    let (blocked, limited) = agent_status::attention_counts();
    let down = stack_health::down_count();
    if blocked + limited + down > 0 {
        label.push('\n');
        label.push_str(&trv(
            "tray.tooltip_attention",
            lang,
            &[
                ("blocked", &blocked.to_string()),
                ("limited", &limited.to_string()),
                ("down", &down.to_string()),
            ],
        ));
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(&label));
    }
}

/// Register a single shortcut (no unregister_all). Errors on a bad/taken combo.
fn register_shortcut(app: &AppHandle, accel: &str) -> Result<(), String> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let sc =
        Shortcut::from_str(accel).map_err(|e| trv("err.bad_hotkey", cur_lang(), &[("e", &e)]))?;
    app.global_shortcut()
        .register(sc)
        .map_err(|e| format!("{e}"))
}

/// Register (replacing any previous) the OS-global show/hide accelerator. Errors on a bad/taken combo.
fn register_toggle_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    // Teardown-FIRST (mirrors set_shortcuts): clear the old registration BEFORE registering, so
    // re-applying the SAME accel doesn't fail register() with "already registered" and leave the
    // toggle unbound. Replaces the old register→unregister_all→register dance (which registered twice).
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
            let _ = patch_config(|c| {
                let mut m = c.shortcuts.clone().unwrap_or_default();
                m.insert("toggle_window".to_string(), a.to_string());
                c.shortcuts = Some(m);
            });
            register_toggle_hotkey(&app, a)
        }
        None => {
            // Unregister ONLY the toggle accelerator, not every OS-global shortcut (unregister_all
            // would also drop any other registered accelerator). Look up the currently-configured
            // toggle and drop just it.
            use std::str::FromStr;
            use tauri_plugin_global_shortcut::Shortcut;
            if let Some(cur) = read_config_file()
                .shortcuts
                .and_then(|m| m.get("toggle_window").cloned())
                .filter(|s| !s.trim().is_empty())
            {
                if let Ok(sc) = Shortcut::from_str(&cur) {
                    let _ = app.global_shortcut().unregister(sc);
                }
            }
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
    // All applied cleanly → persist (R7: atomic patch so it bumps rev).
    patch_config(|c| {
        c.shortcuts = Some(shortcuts.clone());
        c.toggle_hotkey = shortcuts.get("toggle_window").cloned();
    })?;
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
    // The structured ssh target is dropped verbatim into the `ssh --% -t {target}` line. `--%` blocks
    // PowerShell, not ssh.exe's option parsing — a target sourced from a persisted/restored recipe or
    // an imported ~/.ssh/config could carry `-oProxyCommand=…` and run a local program. Gate it.
    if let Some(t) = ssh {
        if !valid_ssh_target(t) {
            return Err(trv("err.invalid_ssh_target", cur_lang(), &[("target", &t)]));
        }
    }
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
        // A LOCAL codex pane carries its own end-of-turn notifier (see `codex_notify_arg`). Remote
        // codex is skipped: the notifier would run on the far side and could not reach this
        // machine's status dir — the same reason remote claude stays hookless.
        let base: String = match tool.as_str() {
            "opencode" => "opencode".to_string(),
            "codex" => match ensure_notify_script() {
                Ok(script) => format!("codex {}", codex_notify_arg(&script)),
                // A launch must never fail because the notifier could not be written; the pane
                // falls back to the PTY heartbeat, exactly as before this existed.
                Err(_) => "codex".to_string(),
            },
            _ => "claude".to_string(),
        };
        Some(if extra.is_empty() {
            base
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
    // A LOCAL opencode pane reports its turns through a plugin merged in via the environment. Remote
    // opencode is skipped: the env does not cross ssh, and the plugin could not reach this machine's
    // status dir anyway. A failure to write the plugin must not block the launch.
    if tool == "opencode" && ssh.is_none() {
        if let Ok(plugin) = ensure_opencode_plugin() {
            cmd.env("OPENCODE_CONFIG_CONTENT", opencode_plugin_config(&plugin));
        }
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
    // their hooks run on the remote host and never reach the local status dir). Only a LOCAL
    // claude pane expects the lifecycle hook; without it, activity is not used as a fallback
    // (it would false-flag `working` on background hook output — A-residual).
    let hook_expected = tool == "claude" && ssh.is_none();
    agent_status::on_spawn(&id, &tool, &profile, hook_expected);

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
        // Transient read errors (a PTY hiccup, an interrupted syscall) previously counted as EOF and
        // reaped a still-live session. Distinguish: Interrupted → retry at once; any other error →
        // short backoff (50/100/200ms) up to 3× while checking the child is alive — a live child means
        // keep reading, a dead/exhausted one means the session really ended. Reset on any good read.
        let mut transient_retries = 0u8;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: the child closed the PTY (it has exited)
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    if transient_retries >= 3 {
                        break; // persistent error → give up (never a busy-loop / zombie reader)
                    }
                    let backoff = [50u64, 100, 200][transient_retries as usize];
                    transient_retries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(backoff));
                    match child.try_wait() {
                        Ok(Some(_)) => break, // child exited → real end of session
                        // Alive → a transient PTY hiccup. Reset the budget so a long-lived session
                        // isn't reaped after 3 TOTAL hiccups over its lifetime; the cap only trips on
                        // consecutive errors where we can't confirm the child is alive.
                        Ok(None) => {
                            transient_retries = 0;
                            continue;
                        }
                        Err(_) => continue, // can't tell → retry within the cap
                    }
                }
                Ok(n) => {
                    transient_retries = 0;
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
    // `code` on Windows is usually a `code.cmd` shim, which spawning the bare name `code` won't
    // resolve (only a `code.exe` is found on PATH). Resolve the real launcher PATHEXT-aware so
    // installs that expose only code.cmd still get the --goto line jump instead of falling back to
    // the default app. The target is a canonicalized path (metacharacters already rejected above),
    // so passing it to the .cmd shim is safe.
    let editor = exe_on_path("code").unwrap_or_else(|| "code".into());
    if std::process::Command::new(&editor)
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
const SSHHOSTS_CONFIG_REL: &str = "{{PROFILES}}\\config\\sshhosts.json";
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
        // `split_once` cuts on char boundaries. The old `find(..)` + `line[i + 1..]` assumed the
        // separator was one byte, but `char::is_whitespace()` is true for NBSP (U+00A0), U+2000-200A
        // and U+3000 — a `~/.ssh/config` pasted from a web page panics the byte slice mid-char. Same
        // class as the `expand_ssh_config` panic fixed via `str::get`; this sibling parser was missed.
        let (key, val) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, rest)) => (
                k.trim(),
                rest.trim_matches(|c: char| c.is_whitespace() || c == '=').trim(),
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
        // Slice via `str::get` (not `t[..7]`): a non-ASCII line (e.g. a Cyrillic comment
        // `# рабочий сервер`) whose byte 7 lands inside a multi-byte char would panic on a raw
        // byte-index slice — `get` returns None instead, so the line is just passed through.
        let is_include = t
            .get(..7)
            .is_some_and(|h| h.eq_ignore_ascii_case("include"))
            && t
                .get(7..)
                .is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '='));
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

    #[test]
    fn expand_ssh_config_survives_cyrillic_line_and_still_detects_include() {
        // Regression: a Cyrillic comment/value whose 7th byte splits a multi-byte char must not
        // panic the byte-index Include check (str::get, not t[..7]). And a real `Include` on a
        // line that also carries Cyrillic elsewhere must still be recognized.
        let dir = std::env::temp_dir().join(format!("castellyn_ssh_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let inc = dir.join("extra.conf");
        std::fs::write(&inc, "Host included\n  HostName 10.9.9.9\n").unwrap();
        let main = dir.join("config");
        // `# рабочий сервер`: byte 7 lands inside a Cyrillic char — the panic repro.
        std::fs::write(
            &main,
            format!(
                "# рабочий сервер\nHost родной\n  HostName 10.0.0.1\nInclude {}\n",
                inc.display()
            ),
        )
        .unwrap();
        let mut out = String::new();
        expand_ssh_config(&main, &dir.to_string_lossy(), 0, &mut out); // must not panic
        assert!(
            out.contains("Host included"),
            "Include directive was resolved and its file spliced in"
        );
        assert!(out.contains("# рабочий сервер"), "Cyrillic line passed through");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_ssh_config_survives_multibyte_whitespace_separator() {
        // Regression (sibling of the expand_ssh_config panic): `char::is_whitespace()` matches NBSP
        // and other multi-byte spaces, so the old `line[i + 1..]` sliced mid-char. A config pasted
        // from a web page carries U+00A0; parsing it must not panic and must still read the value.
        let hosts = parse_ssh_config("Host box\n  HostName\u{00a0}10.0.0.7\n  Port\u{3000}2222\n");
        assert_eq!(hosts.len(), 1, "the Host block is still recognized");
        assert_eq!(hosts[0].host, "10.0.0.7", "HostName parsed across an NBSP separator");
        assert_eq!(hosts[0].port, Some(2222), "Port parsed across an ideographic space");
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
                // If a window with this label already exists, ANOTHER open_monitor_window won the race
                // (a duplicate-label build error). Don't clear the registry or report a failure — the
                // winner owns the stashed spec and the live window.
                if app2.get_webview_window(&label).is_some() {
                    return;
                }
                // Genuine build failure — don't fail silently. The frontend stashed the pane spec under
                // this label (prepare_detach) before calling us; clear it so it can't leak, and tell the
                // UI so it can re-home the pane / toast instead of "losing" the detached session.
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
    // Dev-only: expose a loopback CDP endpoint so live-verify tooling can attach to the running
    // WebView2 window and observe real backend-driven UI state (pane `limited` badge, etc.). NEVER
    // in release — a remote-debugging port exposes the webview + the IPC bridge to the Rust backend.
    // WebView2 binds the port to 127.0.0.1 only. Respects a pre-set value so the port is overridable
    // (or disable-able). Must run BEFORE the first WebView2 is created, i.e. here at the top of run().
    #[cfg(debug_assertions)]
    if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_none() {
        // SAFETY: first statement in run(), before any Tauri plugin or worker thread starts — there
        // is no concurrent env access, so this single-threaded set_var is sound under edition 2024.
        unsafe {
            std::env::set_var(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                "--remote-debugging-port=9222",
            );
        }
        eprintln!("[castellyn] dev CDP endpoint on http://127.0.0.1:9222");
    }
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
                // Skip the ephemeral detached windows by label PREFIX (mon-* monitor fills, pane-*
                // popped-out panes) — robust to any monitor count, unlike an enumerated mon-0..mon-7
                // list that let a 9th+ monitor window persist/restore a stale rect. `true` = save.
                .with_filter(|label| !(label.starts_with("mon-") || label.starts_with("pane-")))
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
        .manage(StackRun::default())
        .manage(StackProcs::default())
        .manage(ForkRuns::default())
        .manage(SessionState::default())
        .manage(UsageCache::default())
        .invoke_handler(tauri::generate_handler![
            list_components,
            scripts_available,
            read_status,
            run_component,
            run_forks,
            run_fork_repo,
            read_fork_config,
            write_fork_config,
            cancel_fork_repo,
            read_fork_repo_status,
            list_backups,
            reveal_backup,
            delete_backup,
            verify_backup,
            extract_backup,
            import_backup_zip,
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
            stack_log_path,
            read_stack_procs,
            session_spawn,
            session_write,
            session_resize,
            session_kill,
            session_attach,
            session_detach,
            session_list,
            worktree::worktree_create,
            worktree::worktree_remove,
            worktree::worktree_is_clean,
            worktree::is_git_repo,
            session_bus::bus_send,
            session_bus::bus_poll,
            session_bus::bus_mark_read,
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
            read_profile_matrix,
            set_profile_proxy,
            set_profile_folders,
            set_profile_plugins,
            unblock_managed_plugin,
            read_codex_profiles,
            read_onboarding,
            run_onboarding_step,
            create_settings_junction,
            run_profile_relink,
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
            poll_limits_now,
            read_sessions_prefs,
            write_sessions_prefs,
            add_provider_key,
            remove_provider_key,
            next_provider_key,
            read_opencode,
read_opencode_models,
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
            share_commands,
            run_opencode_rtk,
            run_opencode_mcp,
            run_opencode_providers,
            run_opencode_instructions,
            run_codex_mcp,
            run_codex_providers,
            run_codex_omniroute,
            list_plugin_updates,
            list_plugin_contents,
            list_plugin_releases,
            run_plugin,
            run_plugins_bulk,
            run_marketplace_bump,
            plugin_sync_status,
            plugin_sync_set,
            run_plugin_sync,
            read_stack_drift,
            run_managed_deploy,
            read_gc_scan,
            run_gc_delete,
            agent_status_hook_status,
            agent_status_hook_set,
            delete_skill,
            list_agents,
            read_agent,
            save_agent,
            delete_agent,
            test_subagent,
            resolve_sync_conflict,
            read_schedules,
            read_schedules_cached,
            run_schedule,
            read_config,
            write_config,
            export_config,
            import_config,
            app_paths,
            gateway_base_url,
            omniroute_base_url,
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
            // Read config once (was read twice in setup): the locale seed, start-hidden and the
            // shortcut registration below all read from it.
            let cfg = read_config_file();
            // Seed the backend locale from config so the tray builds in the right language. The
            // frontend also re-syncs on mount (covers a fresh config with no language yet).
            if let Some(lang) = cfg.language.as_deref() {
                set_cur_lang(Lang::parse(lang));
            }
            build_tray(app.handle())?;
            // Agent-status engine for Sessions panes (hook files + PTY activity → events).
            agent_status::start(app.handle().clone());
            // Anthropic OAuth usage-limit monitor (per profile; 85%/99% alerts). No-op for profiles
            // without OAuth creds; disableable via the `limitsMonitor` config toggle.
            limits::start(app.handle().clone());
            // Background llm-stack liveness poll (every 30s): pushes stack-health + flags
            // transition-to-down so post-startup death is seen without a manual refresh.
            stack_health::start(app.handle().clone());
            // Background schedules watcher (every 5 min, file-only): OS-notify when a scheduled
            // maintenance task transitions to failed, so a failed nightly job isn't missed.
            schedules_watch::start(app.handle().clone());
            // One-time brand-rename migration of the autostart Run entry (AgentHub → Castellyn).
            migrate_autostart();
            // Start minimized to tray if configured.
            if cfg.start_hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            // Register all configured shortcuts. A bad/taken combo must not block startup.
            if let Some(shortcuts) = cfg.shortcuts.as_ref() {
                // register_shortcut does NOT unregister_all, so one pass registers each accel cleanly.
                // The old `if count > 1 { unregister_all + re-register }` block was dead churn (stale
                // copy-paste from an earlier probe-register design).
                for accel in shortcuts.values().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Err(e) = register_shortcut(app.handle(), accel) {
                        eprintln!("shortcut register failed ({accel}): {e}");
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
    fn omniroute_base_url_resolves_from_stack_json() {
        // The `omniroute` entry (Ф4) must be present and expose port 20128.
        let url = super::omniroute_base_url().expect("omniroute entry present in stack.json");
        assert_eq!(url, "http://localhost:20128");
    }

    #[test]
    fn stop_aborts_only_on_stop_all() {
        // A stop-all (only=None) must flip STACK_CANCEL to abort a concurrent full start.
        assert!(stop_aborts_start(None));
        // A targeted single-service stop must NOT — it would otherwise cancel the full start
        // of every OTHER service (CAST-3).
        assert!(!stop_aborts_start(Some("gateway")));
    }

    #[test]
    fn ready_timeout_reads_override_else_default() {
        // No field → historical 25s default.
        assert_eq!(ready_timeout_secs(&serde_json::json!({})), 25);
        // A positive override wins (Qwen cold start).
        assert_eq!(
            ready_timeout_secs(&serde_json::json!({ "readyTimeoutSec": 60 })),
            60
        );
        // Nonsense (0 / non-number) falls back to the default, never a zero budget.
        assert_eq!(
            ready_timeout_secs(&serde_json::json!({ "readyTimeoutSec": 0 })),
            25
        );
        assert_eq!(
            ready_timeout_secs(&serde_json::json!({ "readyTimeoutSec": "x" })),
            25
        );
    }

    #[test]
    fn health_timeout_reads_override_else_default() {
        // No field → historical 15s default.
        assert_eq!(health_timeout_secs(&serde_json::json!({})), 15);
        // A positive override wins (a slow health endpoint).
        assert_eq!(
            health_timeout_secs(&serde_json::json!({ "healthTimeoutSec": 40 })),
            40
        );
        // Zero falls back to the default, never a zero budget.
        assert_eq!(
            health_timeout_secs(&serde_json::json!({ "healthTimeoutSec": 0 })),
            15
        );
    }

    #[test]
    fn should_teardown_mirrors_flag() {
        assert!(should_teardown(true));
        assert!(!should_teardown(false));
    }

    #[test]
    fn order_services_deps_precede_dependents() {
        fn ids(services: &[serde_json::Value]) -> Vec<&str> {
            services
                .iter()
                .map(|s| s.get("id").and_then(|x| x.as_str()).unwrap())
                .collect()
        }
        let services = vec![
            serde_json::json!({ "id": "gateway", "dependsOn": ["qwen"] }),
            serde_json::json!({ "id": "qwen" }),
        ];
        let ordered = order_services(&services);
        assert_eq!(ids(&ordered), vec!["qwen", "gateway"]);
    }

    #[test]
    fn order_services_missing_dep_no_hang() {
        // dependsOn an id that isn't in the list — must still be emitted, manifest order preserved.
        let services = vec![
            serde_json::json!({ "id": "a", "dependsOn": ["nope"] }),
            serde_json::json!({ "id": "b" }),
        ];
        let ordered = order_services(&services);
        let ids: Vec<&str> = ordered
            .iter()
            .map(|s| s.get("id").and_then(|x| x.as_str()).unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn order_services_cycle_falls_back() {
        // a depends on b, b depends on a — unresolvable, must fall back to manifest order, not hang.
        let services = vec![
            serde_json::json!({ "id": "a", "dependsOn": ["b"] }),
            serde_json::json!({ "id": "b", "dependsOn": ["a"] }),
        ];
        let ordered = order_services(&services);
        let ids: Vec<&str> = ordered
            .iter()
            .map(|s| s.get("id").and_then(|x| x.as_str()).unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn order_services_no_deps_is_identity() {
        // Regression guard: absent `dependsOn` anywhere must be a no-op reorder.
        let services = vec![
            serde_json::json!({ "id": "freellmapi" }),
            serde_json::json!({ "id": "gateway" }),
            serde_json::json!({ "id": "deepseek" }),
        ];
        let ordered = order_services(&services);
        let ids: Vec<&str> = ordered
            .iter()
            .map(|s| s.get("id").and_then(|x| x.as_str()).unwrap())
            .collect();
        assert_eq!(ids, vec!["freellmapi", "gateway", "deepseek"]);
    }

    #[test]
    fn gc_install_path_tuple_parses_alias_and_slashes() {
        // A different profile's alias in the path is stripped down to (org, plugin, ver).
        assert_eq!(
            install_path_tuple(r"C:\Users\X\.claude-cc1\plugins\cache\org\pl\1.0.0"),
            Some(("org".into(), "pl".into(), "1.0.0".into()))
        );
        // Forward slashes tolerated.
        assert_eq!(
            install_path_tuple("C:/Users/X/.claude/plugins/cache/max-marketplace/max/1.9.0"),
            Some(("max-marketplace".into(), "max".into(), "1.9.0".into()))
        );
        // "unknown" is a valid version dir.
        assert_eq!(
            install_path_tuple(r"C:\u\.claude\plugins\cache\o\p\unknown"),
            Some(("o".into(), "p".into(), "unknown".into()))
        );
        // Marker absent / too few components → None.
        assert_eq!(install_path_tuple(r"C:\somewhere\else"), None);
        assert_eq!(install_path_tuple(r"C:\u\.claude\plugins\cache\o\p"), None);
    }

    #[test]
    fn gc_stale_detect() {
        use std::collections::HashSet;
        let mut pairs = HashSet::new();
        let mut triples = HashSet::new();
        pairs.insert(("max-marketplace".to_string(), "max".to_string()));
        triples.insert((
            "max-marketplace".to_string(),
            "max".to_string(),
            "1.9.0".to_string(),
        ));
        // active pair, different ver dir → stale.
        assert!(is_stale_ver(
            "max-marketplace",
            "max",
            "1.7.0",
            &pairs,
            &triples
        ));
        // the active ver itself → not stale (case-insensitive).
        assert!(!is_stale_ver(
            "Max-Marketplace",
            "Max",
            "1.9.0",
            &pairs,
            &triples
        ));
        // pair not in active-set at all → left alone (conservative).
        assert!(!is_stale_ver("other", "plugin", "2.0.0", &pairs, &triples));
    }

    #[test]
    fn gc_os_token_matcher() {
        assert!(has_os_token("node-linux-x64"));
        assert!(has_os_token("linux"));
        assert!(has_os_token("foo.darwin.node"));
        assert!(has_os_token("linux-notes.md"));
        assert!(!has_os_token("mylinuxish"));
        assert!(!has_os_token("darwinism")); // no trailing boundary
        assert!(!has_os_token("windows-x64"));
    }

    #[test]
    fn gc_id_category_roundtrip() {
        assert_eq!(gc_id_category("stale:org/pl/1.0.0"), Some("stale_version"));
        assert_eq!(gc_id_category("tempgit:temp_git_abc"), Some("temp_git"));
        assert_eq!(gc_id_category("bak:foo.bak"), Some("bak"));
        assert_eq!(gc_id_category("wrongos:.claude"), Some("wrong_os"));
        assert_eq!(gc_id_category("mystery:x"), None);
        // Constructor derives the category field from the id prefix.
        let it = gc_item("bak:x.bak".into(), "x.bak".into(), "p".into(), 3, true);
        assert_eq!(it.category, "bak");
    }

    #[test]
    fn gc_dir_size_sums_tree() {
        use std::io::Write;
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("castellyn_gc_{pid}"));
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::File::create(root.join("a.txt"))
            .unwrap()
            .write_all(&[0u8; 100])
            .unwrap();
        std::fs::File::create(sub.join("b.txt"))
            .unwrap()
            .write_all(&[0u8; 50])
            .unwrap();
        assert_eq!(gc_dir_size(&root), 150);
        // A single file path returns its own length.
        assert_eq!(gc_dir_size(&root.join("a.txt")), 100);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn marketplace_versions_classify() {
        fn row(plugin: &str, mkt: &str, src: Option<&str>, inst: Option<&str>) -> MarketVer {
            MarketVer {
                plugin: plugin.into(),
                market: "max-marketplace".into(),
                mkt_ver: mkt.into(),
                src_ver: src.map(String::from),
                installed: inst.map(String::from),
            }
        }
        // Everything aligned → ok.
        let (s, d) = classify_marketplace_versions(&[row("max", "1.14.1", Some("1.14.1"), Some("1.14.1"))]);
        assert_eq!(s, "ok");
        assert!(d.contains("aligned"));
        // marketplace.json ≠ plugin.json → drift asking for a bump.
        let (s, d) = classify_marketplace_versions(&[row("max", "1.14.1", Some("1.14.0"), None)]);
        assert_eq!(s, "drift");
        assert!(d.contains("bump"));
        // Manifests aligned but installed lags → drift asking for an update.
        let (s, d) = classify_marketplace_versions(&[row("max", "1.14.1", Some("1.14.1"), Some("1.9.0"))]);
        assert_eq!(s, "drift");
        assert!(d.contains("update") && d.contains("max@max-marketplace"));
        // installed "unknown" is ignored; not installed at all is fine.
        let (s, _) = classify_marketplace_versions(&[
            row("max", "1.14.1", Some("1.14.1"), Some("unknown")),
            row("speckit", "1.0.2", Some("1.0.2"), None),
        ]);
        assert_eq!(s, "ok");
        // Unreadable plugin.json → drift (surfaced, not silently ok).
        let (s, d) = classify_marketplace_versions(&[row("max", "1.14.1", None, None)]);
        assert_eq!(s, "drift");
        assert!(d.contains("unreadable"));
        // No own marketplaces → ok with the explicit empty note.
        let (s, d) = classify_marketplace_versions(&[]);
        assert_eq!(s, "ok");
        assert!(d.contains("no own marketplaces"));
    }

    /// Manual live smoke of the marketplace-versions drift check over the REAL disk (read-only).
    /// Not part of the gates: `cargo test marketplace_drift_live_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn marketplace_drift_live_smoke() {
        let item = marketplace_versions_drift_item();
        println!("id={} state={} detail={}", item.id, item.state, item.detail);
        assert_eq!(item.fix, None);
    }

    /// Manual live smoke of the onboarding scan on the REAL machine (read-only). Not a gate:
    /// `cargo test onboarding_scan_live_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn onboarding_scan_live_smoke() {
        let steps = onboarding_scan();
        for s in &steps {
            println!("{:<18} {:<8} fix={:<16} {}", s.id, s.state, s.fix.as_deref().unwrap_or("-"), s.detail);
        }
        // The scan is a fixed-shape checklist: every id present exactly once, states in-vocabulary.
        let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        for want in ["prereq_git", "prereq_node", "prereq_claude", "prereq_syncthing", "tree",
                     "junction", "profiles", "creds", "mcp", "managed", "syncthing", "verify"] {
            assert_eq!(ids.iter().filter(|i| **i == want).count(), 1, "{want}");
        }
        assert!(steps.iter().all(|s| ["ok", "todo", "blocked", "unknown"].contains(&s.state.as_str())));
    }

    #[test]
    fn profile_name_from_dir_parses_suffix() {
        assert_eq!(profile_name_from_dir(".claude-cc2").as_deref(), Some("cc2"));
        assert_eq!(profile_name_from_dir(".claude-ccmy").as_deref(), Some("ccmy"));
        // No suffix / wrong prefix / invalid suffix → None.
        assert_eq!(profile_name_from_dir(".claude"), None);
        assert_eq!(profile_name_from_dir(".claude-"), None); // empty suffix
        assert_eq!(profile_name_from_dir(".config"), None);
        assert_eq!(profile_name_from_dir("claude-cc2"), None); // no leading dot
        assert_eq!(profile_name_from_dir(".claude-bad/name"), None); // path separator rejected
    }

    #[test]
    fn settings_tree_precedence() {
        // env override wins outright (even over a config value).
        assert_eq!(
            resolve_settings_tree(Some(r"D:\envtree".into()), Some(r"D:\cfg".into()), r"E:\Scripts", |_| true),
            r"D:\envtree"
        );
        // else the explicit config value.
        assert_eq!(
            resolve_settings_tree(None, Some(r"D:\cfg".into()), r"E:\Scripts", |_| false),
            r"D:\cfg"
        );
        // blank env/config are ignored → fall through to detection.
        assert_eq!(
            resolve_settings_tree(Some("  ".into()), Some(String::new()), r"E:\Scripts",
                |p| p == r"E:\Scripts\!Настройки и MCP"),
            r"E:\Scripts\!Настройки и MCP"
        );
        // detection prefers the ASCII candidate when it holds the tree (owner de-Cyrillicized).
        assert_eq!(
            resolve_settings_tree(None, None, r"E:\Scripts", |_| true),
            r"E:\Scripts\SettingsMCP"
        );
        // nothing exists → ASCII default (created later, no Cyrillic literal in code).
        assert_eq!(
            resolve_settings_tree(None, None, r"E:\Scripts", |_| false),
            r"E:\Scripts\SettingsMCP"
        );
    }

    #[test]
    fn link_target_match_rules() {
        use std::path::Path;
        let exp = r"E:\Scripts\!Настройки и MCP";
        // Exact, \\?\-prefixed, trailing-backslash and ASCII-case variants all match.
        assert!(link_target_matches(Path::new(r"E:\Scripts\!Настройки и MCP"), exp));
        assert!(link_target_matches(Path::new(r"\\?\E:\Scripts\!Настройки и MCP"), exp));
        assert!(link_target_matches(Path::new(r"E:\Scripts\!Настройки и MCP\"), exp));
        assert!(link_target_matches(Path::new(r"e:\scripts\!Настройки и MCP"), exp));
        // A different tree (old location, sibling dir) must NOT match.
        assert!(!link_target_matches(Path::new(r"E:\Old\!Настройки и MCP"), exp));
        assert!(!link_target_matches(Path::new(r"E:\Scripts\SettingsMCP"), exp));
    }

    /// Manual live smoke over the REAL plugin stores (read-only). Not part of the gates:
    /// `cargo test gc_scan_live_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn gc_scan_live_smoke() {
        let Ok(home) = std::env::var("USERPROFILE") else {
            return;
        };
        let items = gc_scan(&home);
        for i in &items {
            println!(
                "{:<14} {:>12} bytes  deletable={}  {}  [{}]",
                i.category, i.size_bytes, i.deletable, i.label, i.path
            );
        }
        // Every id must map to a known category and wrong_os must be report-only.
        assert!(items.iter().all(|i| gc_id_category(&i.id) == Some(i.category.as_str())));
        assert!(items.iter().filter(|i| i.category == "wrong_os").all(|i| !i.deletable));
    }

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
    fn run_guarded_survives_panic() {
        // A panicking monitor tick must not stop later ticks — the whole point of R2's guard.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        run_guarded("test", || panic!("boom in a monitor tick"));
        let mut ran_after = false;
        run_guarded("test", || ran_after = true);
        std::panic::set_hook(prev);
        assert!(ran_after, "a panicking tick must not stop later ticks");
    }

    #[test]
    fn read_settings_for_edit_never_clobbers_unreadable() {
        let pid = std::process::id();
        let dir = std::env::temp_dir();
        // Missing → fresh {} (a new profile settings.json is legitimate).
        let missing = dir.join(format!("castellyn_sfe_none_{pid}.json"));
        assert_eq!(
            read_settings_for_edit(&missing.display().to_string()).unwrap(),
            serde_json::json!({})
        );
        // Empty/whitespace → fresh {}.
        let empty = dir.join(format!("castellyn_sfe_empty_{pid}.json"));
        std::fs::write(&empty, "  \n").unwrap();
        assert_eq!(
            read_settings_for_edit(&empty.display().to_string()).unwrap(),
            serde_json::json!({})
        );
        let _ = std::fs::remove_file(&empty);
        // Valid (with BOM) → parsed as-is.
        let ok = dir.join(format!("castellyn_sfe_ok_{pid}.json"));
        std::fs::write(&ok, "\u{feff}{\"env\":{\"K\":\"v\"}}").unwrap();
        assert_eq!(
            read_settings_for_edit(&ok.display().to_string()).unwrap()["env"]["K"],
            "v"
        );
        let _ = std::fs::remove_file(&ok);
        // Corrupt JSON → Err (NOT {}: an edit would atomically clobber the real file).
        let bad = dir.join(format!("castellyn_sfe_bad_{pid}.json"));
        std::fs::write(&bad, "{ not json").unwrap();
        assert!(read_settings_for_edit(&bad.display().to_string()).is_err());
        let _ = std::fs::remove_file(&bad);
        // Non-UTF-8 (e.g. a PowerShell UTF-16 rewrite) → Err, same reason.
        let utf16 = dir.join(format!("castellyn_sfe_utf16_{pid}.json"));
        std::fs::write(&utf16, [0xFF, 0xFE, 0x7B, 0x00, 0x7D, 0x00]).unwrap(); // UTF-16LE "{}"
        assert!(read_settings_for_edit(&utf16.display().to_string()).is_err());
        let _ = std::fs::remove_file(&utf16);
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
    fn secret_files_never_get_a_bak() {
        // `.mcp.json` was the gap: it is the source of the per-server MCP `env` blocks that
        // `.claude.json` is protected for, and it lives in the Syncthing-synced settings tree.
        for name in [".mcp.json", "settings.json", "opencode.json", ".claude.json"] {
            assert!(is_secret_file(name), "{name} must be excluded from .bak");
        }
        assert!(is_secret_file(".MCP.JSON"), "the check is case-insensitive");
        // Castellyn's own non-secret state keeps its crash-safety backup.
        for name in ["config.json", "forks.json", "schedules.json", "sessions.json"] {
            assert!(!is_secret_file(name), "{name} should keep its .bak");
        }
    }

    #[test]
    fn codex_notify_arg_survives_both_quoting_layers() {
        let a = codex_notify_arg("C:\\Users\\U\\AppData\\Roaming\\castellyn\\hooks\\n.ps1");
        // PowerShell sees one single-quoted argument, so codex receives the TOML verbatim.
        assert!(a.starts_with("-c '") && a.ends_with('\''));
        assert!(a.contains(r#"notify=["pwsh","-NoLogo","-NoProfile","-File","#), "{a}");
        // Not one backslash may survive: PowerShell eats the run before the closing quote, and the
        // half-eaten `\U` then makes the whole array parse as a string (live-reproduced).
        assert!(!a.contains('\\'), "{a}");
        assert!(
            a.contains("\"C:/Users/U/AppData/Roaming/castellyn/hooks/n.ps1\""),
            "{a}"
        );
        // A path with an apostrophe (a legal Windows path) must not close the PowerShell quote.
        let q = codex_notify_arg("C:\\it's\\n.ps1");
        assert!(q.contains("C:/it''s/n.ps1"), "{q}");
        assert_eq!(q.matches('\'').count(), 4); // opening + doubled pair + closing
    }

    #[test]
    fn opencode_plugin_config_appends_a_file_url() {
        let c = opencode_plugin_config("C:\\Users\\U\\AppData\\Roaming\\castellyn\\hooks\\p.js");
        let v: serde_json::Value = serde_json::from_str(&c).unwrap();
        assert_eq!(
            v["plugin"][0],
            "file:///C:/Users/U/AppData/Roaming/castellyn/hooks/p.js"
        );
        // The schema key keeps opencode from warning about an unknown inline config.
        assert_eq!(v["$schema"], "https://opencode.ai/config.json");
        // Only `plugin` may be set: this JSON is merged over the user's config, and any other key
        // would silently override what they chose.
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[test]
    fn sweep_removes_crash_debris_but_spares_a_live_writer() {
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("castellyn_sweep_{pid}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let secret = dir.join("settings.json.tmp");
        let mcp = dir.join(".mcp.json.tmp");
        let plain = dir.join("config.json.tmp"); // non-secret: its .bak already covers it
        let unrelated = dir.join("settings.json"); // the real file must survive
        for f in [&secret, &mcp, &plain, &unrelated] {
            std::fs::write(f, "{}").unwrap();
        }

        // Every file was just created, so a 60s threshold sees them all as a live writer's.
        sweep_stale_secret_tmp(&dir, std::time::Duration::from_secs(60));
        assert!(secret.exists(), "a fresh temp belongs to a concurrent writer");

        // Zero threshold = "everything is debris": only secret-bearing temps go.
        sweep_stale_secret_tmp(&dir, std::time::Duration::ZERO);
        assert!(!secret.exists(), "settings.json.tmp is a stranded secret");
        assert!(!mcp.exists(), ".mcp.json.tmp is a stranded secret");
        assert!(plain.exists(), "non-secret temps are not ours to delete");
        assert!(unrelated.exists(), "the real file must never be touched");

        let _ = std::fs::remove_dir_all(&dir);
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
    fn matrix_folder_desired_and_actual() {
        use serde_json::json;
        // desired: linkedFolders membership; a MISSING field → all default folders desired.
        let linked = vec![json!("agents"), json!("projects")];
        assert!(folder_desired(Some(&linked), "agents"));
        assert!(!folder_desired(Some(&linked), "plugins"));
        assert!(folder_desired(None, "plugins")); // no field → all desired

        // actual classification on a real (non-link) dir + a missing path.
        let base =
            std::env::temp_dir().join(format!("castellyn_matrix_{}", std::process::id()));
        let real_dir = base.join("real");
        std::fs::create_dir_all(&real_dir).unwrap();
        assert_eq!(classify_link(&real_dir), "real"); // exists, not a reparse point
        assert_eq!(classify_link(&base.join("nope")), "missing");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn matrix_plugin_state_classifies() {
        use serde_json::json;
        assert_eq!(plugin_state(Some(&json!(true))), "on");
        assert_eq!(plugin_state(Some(&json!(false))), "off");
        assert_eq!(plugin_state(None), "unset"); // key absent
        assert_eq!(plugin_state(Some(&json!("yes"))), "unset"); // non-bool → unset
    }

    #[test]
    fn matrix_plugin_universe_union_sorted() {
        // Union of keys from several sources, deduped and sorted (stable column order).
        let keys = vec![
            "b@m".to_string(),
            "a@m".to_string(),
            "b@m".to_string(), // dup
            "c@m".to_string(),
        ];
        assert_eq!(
            union_sorted(keys),
            vec!["a@m".to_string(), "b@m".to_string(), "c@m".to_string()]
        );
        assert!(union_sorted(Vec::new()).is_empty());
    }

    #[test]
    fn matrix_upsert_enabled_plugins_surgical() {
        use serde_json::json;
        // Existing file: unrelated top key + an enabledPlugins with a foreign entry.
        let mut settings = json!({
            "theme": "dark",
            "enabledPlugins": { "keep@m": true }
        });
        upsert_enabled_plugins(
            &mut settings,
            &["on@m".to_string()],
            &["off@m".to_string()],
        );
        assert_eq!(settings["theme"], "dark"); // other field preserved
        let ep = &settings["enabledPlugins"];
        assert_eq!(ep["keep@m"], json!(true)); // foreign key untouched
        assert_eq!(ep["on@m"], json!(true)); // enable → true
        assert_eq!(ep["off@m"], json!(false)); // disable → explicit false (never deleted)

        // Missing enabledPlugins object is created.
        let mut bare = json!({ "x": 1 });
        upsert_enabled_plugins(&mut bare, &["p@m".to_string()], &[]);
        assert_eq!(bare["x"], 1);
        assert_eq!(bare["enabledPlugins"]["p@m"], json!(true));
    }

    #[test]
    fn matrix_mcp_split_intersect_and_extras() {
        let canon = vec!["context7".to_string(), "playwright".to_string()];
        let deployed_all = vec![
            "context7".to_string(), // ∩ canon
            "local-extra".to_string(), // extra
        ];
        let (deployed, extras) = mcp_split(&canon, &deployed_all);
        assert_eq!(deployed, vec!["context7".to_string()]);
        assert_eq!(extras, vec!["local-extra".to_string()]);
    }

    #[test]
    fn set_profile_folders_rejects_unknown() {
        // The folder-membership guard lives inline in set_profile_folders; mirror it here against the
        // canonical default set (unknown → reject; known subset → accept).
        let defaults = ["agents", "commands", "plugins", "history.jsonl"];
        let ok = |fs: &[&str]| fs.iter().all(|f| defaults.contains(f));
        assert!(ok(&["agents", "plugins"]));
        assert!(!ok(&["agents", "bogus"]));
    }

    #[test]
    fn proxy_env_merge_set_clear_preserves_others() {
        use serde_json::json;
        let path = std::env::temp_dir().join(format!("castellyn_proxy_{}.json", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);
        // Seed: an unrelated top-level key + an unrelated env var that must survive both ops.
        std::fs::write(
            &p,
            serde_json::to_string(&json!({ "theme": "dark", "env": { "FOO": "bar" } })).unwrap(),
        )
        .unwrap();

        // set → both proxy vars written; FOO + theme preserved.
        apply_proxy_env(&p, "http://127.0.0.1:8080").unwrap();
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["env"]["FOO"], "bar");
        assert_eq!(v["env"]["HTTP_PROXY"], "http://127.0.0.1:8080");
        assert_eq!(v["env"]["HTTPS_PROXY"], "http://127.0.0.1:8080");

        // clear → both proxy vars gone, FOO kept (env stays because FOO remains).
        apply_proxy_env(&p, "").unwrap();
        let v = parse_json_bom(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v["env"].get("HTTP_PROXY").is_none());
        assert!(v["env"].get("HTTPS_PROXY").is_none());
        assert_eq!(v["env"]["FOO"], "bar");

        // scheme validation (the guard set_profile_proxy applies before calling apply_proxy_env).
        let scheme_ok = |u: &str| {
            u.is_empty()
                || u.starts_with("http://")
                || u.starts_with("https://")
                || u.starts_with("socks5://")
        };
        assert!(scheme_ok("https://p:3128"));
        assert!(scheme_ok("socks5://p:1080"));
        assert!(scheme_ok("")); // empty = clear
        assert!(!scheme_ok("ftp://nope"));
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{p}.bak"));
    }

    #[test]
    fn linked_folders_surgical_edit_preserves_siblings() {
        use serde_json::json;
        // Fixture with an unknown top-level key + two profiles; edit only cc1's linkedFolders.
        let cfg = json!({
            "schemaVersion": 1,
            "customTopKey": "keep-me",
            "sharedFoldersDefault": ["agents", "plugins", "projects"],
            "profiles": [
                { "name": "cc1", "color": "Green", "linkedFolders": ["agents", "plugins", "projects"] },
                { "name": "cc2", "color": "Cyan", "linkedFolders": ["agents"] }
            ]
        });
        let out = set_linked_folders(&cfg, "cc1", &["agents".to_string()]).unwrap();
        // Top-level key preserved.
        assert_eq!(out["customTopKey"], "keep-me");
        let profs = out["profiles"].as_array().unwrap();
        // cc1 rewritten to the new set; sibling color untouched.
        let cc1 = profs.iter().find(|p| p["name"] == "cc1").unwrap();
        assert_eq!(cc1["linkedFolders"], json!(["agents"]));
        assert_eq!(cc1["color"], "Green");
        // cc2 completely untouched.
        let cc2 = profs.iter().find(|p| p["name"] == "cc2").unwrap();
        assert_eq!(cc2["linkedFolders"], json!(["agents"]));
        assert_eq!(cc2["color"], "Cyan");
        // Absent profile → Err.
        assert!(set_linked_folders(&cfg, "ghost", &[]).is_err());
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
            "---\nname: max-dedup\ndescription: Find duplicate implementations\n---\nx",
        );
        mk(&root.join("skills\\plain\\SKILL.md"), "no frontmatter here");
        mk(
            &root.join("commands\\check.md"),
            "---\ndescription: Quick check\n---\nc",
        );
        mk(&root.join("commands\\sub\\nested.md"), "c");
        mk(&root.join("agents\\dev-researcher.md"), "a");

        let skills = collect_skill_items(&root.join("skills"));
        let dedup = skills.iter().find(|s| s.name == "max-dedup").unwrap(); // from frontmatter
        assert_eq!(
            dedup.description.as_deref(),
            Some("Find duplicate implementations")
        );
        assert!(dedup.path.ends_with("SKILL.md"));
        let plain = skills.iter().find(|s| s.name == "plain").unwrap(); // fallback to dir name
        assert_eq!(plain.description, None);

        let commands = collect_md_items(&root.join("commands"));
        let check = commands.iter().find(|c| c.name == "check").unwrap();
        assert_eq!(check.description.as_deref(), Some("Quick check"));
        let nested = commands.iter().find(|c| c.name == "sub:nested").unwrap(); // nested -> ':'
        assert_eq!(nested.description, None);

        let agents = collect_md_items(&root.join("agents"));
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "dev-researcher");

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
