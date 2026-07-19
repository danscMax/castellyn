//! Scheduled agent-session launches (W7). Owner decisions 2026-07-18: launches are NORMAL Sessions
//! panes (recipe = env/profile/folder/args/worktree) driven by an INTERNAL minute tick — not Windows
//! Task Scheduler — with Orca's automation semantics: a run missed while the app was closed becomes
//! an honest `skipped_missed` inside a grace window, never a launch-in-the-past; v1 gates are a
//! quota gate (skip when the profile's 5h window is already above a threshold — don't burn a limit
//! overnight for nothing) and an optional shell precheck (timeout + tree-kill, exit!=0 = skip).
//!
//! Split of authority: the BACKEND decides *that* a schedule fires (tick, grace, gates) and marks it
//! `pending`; the FRONTEND owns the actual launch (panes/worktrees/recents live there) — it drains
//! pending entries on the `agent-schedule-due` event or on Sessions mount, then acks the outcome.
//! Every fired/skipped decision is also posted to the session bus (`@all`, kind `schedule`) — the
//! scheduler is the bus's first real producer, so outcomes surface as badge+toast even later.

use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter};

const TICK_SECS: u64 = 60;
const DEFAULT_GRACE_MIN: i64 = 30;
const PRECHECK_DEFAULT_TIMEOUT_SEC: u64 = 60;
/// Tail cap for stored precheck output — enough to see why it failed, never a log dump.
const PRECHECK_OUTPUT_CAP: usize = 2000;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRecipe {
    pub env: String, // claude | codex | opencode | shell
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub worktree: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AgentSchedule {
    pub id: String,
    pub enabled: bool,
    #[serde(default)]
    pub label: String,
    pub recipe: ScheduleRecipe,
    /// "HH:MM" local time.
    pub time: String,
    /// ISO weekday numbers 1(Mon)..7(Sun); empty = every day.
    #[serde(default)]
    pub days: Vec<u8>,
    /// Skip when the profile's 5h utilization is ABOVE this percent (None = no quota gate).
    #[serde(default)]
    pub quota_gate_max_pct: Option<f64>,
    /// Optional shell gate: non-zero exit (or timeout) skips the run.
    #[serde(default)]
    pub precheck: Option<String>,
    #[serde(default)]
    pub precheck_timeout_sec: Option<u64>,
    #[serde(default)]
    pub grace_minutes: Option<i64>,
    /// Bookkeeping (backend-owned).
    #[serde(default)]
    pub last_fired_at: Option<i64>,
    #[serde(default)]
    pub last_outcome: Option<String>,
    /// A fired schedule waiting for the frontend to launch and ack.
    #[serde(default)]
    pub pending: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct ScheduleFile {
    schedules: Vec<AgentSchedule>,
}

static SCHED_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn sched_path() -> Result<String, String> {
    let base = std::env::var("APPDATA").map_err(|e| e.to_string())?;
    Ok(format!("{base}\\castellyn\\agent-schedules.json"))
}

fn load(path: &str) -> ScheduleFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

fn store(path: &str, f: &ScheduleFile) -> Result<(), String> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Minutes since local midnight for an "HH:MM" string; None on a malformed value.
fn parse_hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.split_once(':')?;
    let h: i64 = h.trim().parse().ok()?;
    let m: i64 = m.trim().parse().ok()?;
    if (0..24).contains(&h) && (0..60).contains(&m) {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Pure firing decision, unit-tested: given "now" (epoch minutes of local day + weekday + an
/// absolute ms clock) and the schedule's state, should it fire, be skipped as missed, or wait?
/// `last_fired_at` de-duplicates: a schedule fires at most once per calendar day.
#[derive(PartialEq, Debug)]
pub enum Due {
    Fire,
    Missed,
    Wait,
}

pub fn due_state(
    time_min: i64,          // schedule's HH:MM as minutes since midnight
    days: &[u8],            // allowed ISO weekdays, empty = all
    weekday: u8,            // today's ISO weekday
    now_min: i64,           // minutes since local midnight, now
    grace_min: i64,         // fire window length
    fired_today: bool,      // already fired/skipped this calendar day
) -> Due {
    if fired_today || (!days.is_empty() && !days.contains(&weekday)) {
        return Due::Wait;
    }
    if now_min < time_min {
        return Due::Wait;
    }
    if now_min <= time_min + grace_min {
        Due::Fire
    } else {
        // The moment passed while the app was closed/asleep — honest skip, no launch-in-the-past.
        Due::Missed
    }
}

/// Local wall-clock pieces via chrono-free arithmetic is error-prone; lean on `time` from std:
/// we only need minutes-since-midnight + ISO weekday + a day key, so shell out to PowerShell is
/// overkill — use libc-free localtime via the `chrono`-less approach: std has no local tz, so we
/// take the pragmatic route: civil time from an env-independent source — `std::process` is too
/// heavy per minute. We accept the `chrono` dependency already in the tree? It is NOT — so:
/// derive local time from GetLocalTime on Windows (the only target).
#[cfg(windows)]
fn local_now() -> (i64 /* min since midnight */, u8 /* iso weekday */, String /* day key */) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let st = unsafe { GetLocalTime() };
    let mins = st.wHour as i64 * 60 + st.wMinute as i64;
    // SYSTEMTIME wDayOfWeek: 0=Sunday..6=Saturday → ISO 1=Mon..7=Sun.
    let iso = if st.wDayOfWeek == 0 { 7 } else { st.wDayOfWeek as u8 };
    let key = format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay);
    (mins, iso, key)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Did this schedule already act (fire or skip) on the given local day? Encoded by pairing
/// `last_fired_at` with a stored day key inside `last_outcome` ("<day>|<outcome>").
fn fired_today(s: &AgentSchedule, day_key: &str) -> bool {
    s.last_outcome
        .as_deref()
        .and_then(|o| o.split_once('|'))
        .is_some_and(|(d, _)| d == day_key)
}

fn set_outcome(s: &mut AgentSchedule, day_key: &str, outcome: &str) {
    s.last_fired_at = Some(now_ms());
    s.last_outcome = Some(format!("{day_key}|{outcome}"));
}

/// Keep the last `cap` BYTES of `s`, snapped forward to a char boundary. `s[s.len()-cap..]` alone
/// panics when the byte offset lands inside a multi-byte UTF-8 char (Cyrillic/emoji precheck output).
fn tail_capped(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut start = s.len() - cap;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// Run the precheck command through pwsh with a timeout; kill the tree on expiry. Returns
/// Ok(tail) on exit 0, Err(reason+tail) otherwise. Blocking — called from the tick thread only.
fn run_precheck(cmd: &str, cwd: &str, timeout_sec: u64) -> Result<String, String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let mut child = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", cmd])
        .current_dir(if cwd.is_empty() { "." } else { cwd })
        .creation_flags(crate::CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("precheck spawn: {e}"))?;
    // Drain both pipes on their OWN threads: reading them only after the child exits deadlocks a
    // chatty precheck (>~64 KB fills the OS pipe buffer, the child blocks writing, never exits, and
    // dies only at the timeout — a false failure). Threads read to EOF; the child closes the pipes
    // on exit (or when the tree-kill lands), so both joins return.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_sec);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut tail = out_t.join().unwrap_or_default();
                tail.push_str(&err_t.join().unwrap_or_default());
                let tail = tail_capped(&tail, PRECHECK_OUTPUT_CAP);
                return if status.success() {
                    Ok(tail)
                } else {
                    Err(format!("exit {:?}: {}", status.code(), tail.trim()))
                };
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    // Tree-kill: the precheck may have spawned children (git, node…).
                    let _ = std::process::Command::new("taskkill")
                        .args(["/T", "/F", "/PID", &child.id().to_string()])
                        .creation_flags(crate::CREATE_NO_WINDOW)
                        .output();
                    // The kill closes the pipes → reader threads reach EOF; join so they don't leak.
                    let _ = out_t.join();
                    let _ = err_t.join();
                    return Err("precheck timed out".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => return Err(format!("precheck wait: {e}")),
        }
    }
}

/// The profile's CURRENT 5h utilization via the shared limits cache (no extra endpoint traffic).
/// None = unknown (no OAuth / transport error) — an unknown quota does NOT block the run: the gate
/// protects against a KNOWN-exhausted profile, not against missing data.
fn profile_h5(profile: &str) -> Option<f64> {
    let home = std::env::var("USERPROFILE").ok()?;
    let dir = crate::plugin_sync_profiles(&home)
        .into_iter()
        .map(|(name, _)| name)
        .find(|name| name.strip_prefix(".claude-").unwrap_or(name) == profile)?;
    let cred = format!("{home}\\{dir}\\.credentials.json");
    match crate::limits::usage_cached(&cred)? {
        Ok(resp) => crate::limits::util_of(&resp, "five_hour").0,
        Err(_) => None,
    }
}

/// Payload of `agent-schedule-due` — the frontend launches this recipe and acks.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScheduleDue {
    id: String,
    label: String,
    recipe: ScheduleRecipe,
}

/// One tick: evaluate every enabled schedule, fire/skip, persist, notify. Public for tests via the
/// pure pieces; the thread wrapper below owns the cadence.
#[cfg(windows)]
fn tick(app: &AppHandle) {
    let Ok(path) = sched_path() else { return };
    let bus = crate::session_bus::bus_file_path();
    let (now_min, weekday, day_key) = local_now();
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = load(&path);
    let mut changed = false;
    for s in f.schedules.iter_mut() {
        if !s.enabled || s.pending {
            continue;
        }
        let Some(time_min) = parse_hhmm(&s.time) else { continue };
        let grace = s.grace_minutes.unwrap_or(DEFAULT_GRACE_MIN).max(1);
        match due_state(time_min, &s.days, weekday, now_min, grace, fired_today(s, &day_key)) {
            Due::Wait => {}
            Due::Missed => {
                set_outcome(s, &day_key, "skipped_missed");
                changed = true;
                if let Ok(b) = &bus {
                    let _ = crate::session_bus::send(
                        b,
                        "scheduler",
                        "@all",
                        "schedule",
                        &format!("{}: пропущен (окно {} мин прошло)", s.label_or_id(), grace),
                    );
                }
            }
            Due::Fire => {
                // Gate 1: quota — a KNOWN-hot profile is skipped, unknown passes (see profile_h5).
                if let (Some(maxp), "claude") = (s.quota_gate_max_pct, s.recipe.env.as_str()) {
                    if let Some(h5) = profile_h5(&s.recipe.profile) {
                        if h5 > maxp {
                            set_outcome(s, &day_key, "skipped_quota");
                            changed = true;
                            if let Ok(b) = &bus {
                                let _ = crate::session_bus::send(
                                    b,
                                    "scheduler",
                                    "@all",
                                    "schedule",
                                    &format!("{}: пропущен (квота {h5:.0}% > {maxp:.0}%)", s.label_or_id()),
                                );
                            }
                            continue;
                        }
                    }
                }
                // Gate 2: precheck (blocking, bounded — we're on the tick thread, not the UI).
                if let Some(cmd) = s.precheck.as_deref().filter(|c| !c.trim().is_empty()) {
                    let t = s.precheck_timeout_sec.unwrap_or(PRECHECK_DEFAULT_TIMEOUT_SEC);
                    if let Err(why) = run_precheck(cmd, &s.recipe.folder, t) {
                        set_outcome(s, &day_key, "skipped_precheck");
                        changed = true;
                        if let Ok(b) = &bus {
                            let _ = crate::session_bus::send(
                                b,
                                "scheduler",
                                "@all",
                                "schedule",
                                &format!("{}: пропущен precheck ({why})", s.label_or_id()),
                            );
                        }
                        continue;
                    }
                }
                // Fire: mark pending, hand the launch to the frontend.
                s.pending = true;
                set_outcome(s, &day_key, "fired");
                changed = true;
                let _ = app.emit(
                    "agent-schedule-due",
                    ScheduleDue {
                        id: s.id.clone(),
                        label: s.label_or_id().to_string(),
                        recipe: s.recipe.clone(),
                    },
                );
            }
        }
    }
    if changed {
        let _ = store(&path, &f);
    }
}

impl AgentSchedule {
    fn label_or_id(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// Start the tick thread (called once from setup()).
#[cfg(windows)]
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(TICK_SECS));
        crate::run_guarded("agent-schedules", || tick(&app));
    });
}

// ── Commands ───────────────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn read_agent_schedules() -> Result<Vec<AgentSchedule>, String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(load(&sched_path()?).schedules)
}

#[tauri::command]
pub fn write_agent_schedules(schedules: Vec<AgentSchedule>) -> Result<(), String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    store(&sched_path()?, &ScheduleFile { schedules })
}

/// The frontend launched (or failed to launch) a pending schedule — record the outcome.
#[tauri::command]
pub fn ack_agent_schedule(id: String, outcome: String) -> Result<(), String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = sched_path()?;
    let mut f = load(&path);
    if let Some(s) = f.schedules.iter_mut().find(|s| s.id == id) {
        s.pending = false;
        // Keep the day-key from the fire, swap only the outcome word.
        if let Some((day, _)) = s.last_outcome.as_deref().and_then(|o| o.split_once('|')) {
            let day = day.to_string();
            s.last_outcome = Some(format!("{day}|{outcome}"));
        }
        store(&path, &f)?;
    }
    Ok(())
}

/// Pending entries (fired while Sessions wasn't mounted) — drained by the frontend on mount.
#[tauri::command]
pub fn pending_agent_schedules() -> Result<Vec<AgentSchedule>, String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(load(&sched_path()?)
        .schedules
        .into_iter()
        .filter(|s| s.pending)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hhmm_parses_and_rejects() {
        assert_eq!(parse_hhmm("03:30"), Some(210));
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(23 * 60 + 59));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("nope"), None);
        assert_eq!(parse_hhmm("12"), None);
    }

    #[test]
    fn due_semantics_fire_missed_wait() {
        // Before the slot → wait; inside the grace window → fire; after it → honest missed.
        assert_eq!(due_state(210, &[], 1, 200, 30, false), Due::Wait);
        assert_eq!(due_state(210, &[], 1, 210, 30, false), Due::Fire);
        assert_eq!(due_state(210, &[], 1, 239, 30, false), Due::Fire);
        assert_eq!(due_state(210, &[], 1, 241, 30, false), Due::Missed);
        // Already acted today → wait regardless of the window.
        assert_eq!(due_state(210, &[], 1, 215, 30, true), Due::Wait);
        // Day filter: Monday-only schedule on Sunday → wait even inside the window.
        assert_eq!(due_state(210, &[1], 7, 215, 30, false), Due::Wait);
        assert_eq!(due_state(210, &[1, 7], 7, 215, 30, false), Due::Fire);
    }

    #[test]
    fn fired_today_keys_by_day() {
        let mut s = AgentSchedule {
            id: "x".into(),
            enabled: true,
            label: String::new(),
            recipe: ScheduleRecipe::default(),
            time: "03:00".into(),
            days: vec![],
            quota_gate_max_pct: None,
            precheck: None,
            precheck_timeout_sec: None,
            grace_minutes: None,
            last_fired_at: None,
            last_outcome: None,
            pending: false,
        };
        assert!(!fired_today(&s, "2026-07-18"));
        set_outcome(&mut s, "2026-07-18", "fired");
        assert!(fired_today(&s, "2026-07-18"));
        assert!(!fired_today(&s, "2026-07-19")); // next day re-arms
    }

    #[test]
    fn precheck_pass_fail_shapes() {
        assert!(run_precheck("exit 0", "", 30).is_ok());
        let err = run_precheck("Write-Output boom; exit 3", "", 30).unwrap_err();
        assert!(err.contains("boom"), "tail should carry output: {err}");
    }

    #[test]
    fn tail_capped_never_panics_on_multibyte_boundary() {
        // "щ" is 2 bytes; a cap that lands mid-char must snap forward, not panic (the old
        // byte-slice `s[s.len()-cap..]` crashed the scheduler tick on Cyrillic/emoji output).
        let s = "щ".repeat(3000); // 6000 bytes, all multi-byte
        let out = tail_capped(&s, PRECHECK_OUTPUT_CAP);
        assert!(out.len() <= PRECHECK_OUTPUT_CAP);
        assert!(out.chars().all(|c| c == 'щ')); // never a split/replacement char
        // Short input is returned whole.
        assert_eq!(tail_capped("ok", PRECHECK_OUTPUT_CAP), "ok");
    }
}
