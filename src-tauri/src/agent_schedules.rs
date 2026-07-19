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
        // Route through the canonical BOM-tolerant parser instead of re-rolling it here; the extra
        // Value hop is irrelevant for a file this small.
        .and_then(|t| crate::parse_json_bom(&t).ok())
        .and_then(|v| serde_json::from_value(v).ok())
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

/// Days in a civil month (leap-aware). Pure.
fn days_in_month(y: i64, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Yesterday's day key ("YYYY-MM-DD") + ISO weekday, derived from today's. None if `today_key` is
/// malformed. Pure — handles month/year/leap boundaries so the midnight recovery below is testable.
fn prev_day_key(today_weekday: u8, today_key: &str) -> Option<(u8, String)> {
    let mut it = today_key.split('-');
    let y: i64 = it.next()?.trim().parse().ok()?;
    let m: u8 = it.next()?.trim().parse().ok()?;
    let d: u8 = it.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    let (py, pm, pd) = if d > 1 {
        (y, m, d - 1)
    } else if m == 1 {
        (y - 1, 12, 31)
    } else {
        (y, m - 1, days_in_month(y, m - 1))
    };
    let yweekday = if today_weekday <= 1 { 7 } else { today_weekday - 1 };
    Some((yweekday, format!("{py:04}-{pm:02}-{pd:02}")))
}

/// Firing decision that also recovers a late-yesterday occurrence whose grace window spilled past
/// midnight (e.g. 23:50 + 30m grace, the app asleep until 00:05). `due_state` alone looks only at
/// today, so without this such an occurrence is neither fired nor recorded — silently lost. Returns
/// the Due plus the day key the outcome must be recorded under (today's, or yesterday's when a
/// recovered occurrence fires). Pure, unit-tested.
#[allow(clippy::too_many_arguments)]
fn effective_due(
    time_min: i64,
    days: &[u8],
    grace_min: i64,
    now_min: i64,
    today_weekday: u8,
    today_key: &str,
    today_fired: bool,
    yesterday: Option<(u8, String, bool)>, // (weekday, key, fired); None when the key can't be derived
) -> (Due, String) {
    let today = due_state(time_min, days, today_weekday, now_min, grace_min, today_fired);
    // Only look back when today's own occurrence hasn't begun (Wait) and the window could span
    // midnight — never override a real Fire/Missed for today.
    if today == Due::Wait && time_min + grace_min >= 1440 {
        if let Some((yw, yk, yf)) = yesterday {
            // Yesterday on a continuous timeline: "now" is now_min+1440 minutes after yesterday's
            // midnight. Recover only a still-in-window FIRE; an already-past yesterday stays ignored
            // (no launch-in-the-past), exactly as before.
            if due_state(time_min, days, yw, now_min + 1440, grace_min, yf) == Due::Fire {
                return (Due::Fire, yk);
            }
        }
    }
    (today, today_key.to_string())
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

/// Swap only the outcome word, keeping the day key from the original fire — used once the fire has
/// already been recorded and only its result changes (frontend ack, stale-pending sweep).
fn swap_outcome(s: &mut AgentSchedule, outcome: &str) {
    if let Some((day, _)) = s.last_outcome.as_deref().and_then(|o| o.split_once('|')) {
        let day = day.to_string();
        s.last_outcome = Some(format!("{day}|{outcome}"));
    }
}

fn grace_of(s: &AgentSchedule) -> i64 {
    s.grace_minutes.unwrap_or(DEFAULT_GRACE_MIN).max(1)
}

/// Is a `pending` entry too old to still be launched? The frontend drains pending entries only when
/// Sessions is mounted, so one fired while the tab was never opened sat there indefinitely: it would
/// eventually launch hours or days late (the launch-in-the-past this module refuses), and until then
/// it wedged the schedule outright — `tick` skips pending entries, so nothing else ever fired. Past
/// the grace window it becomes an honest `skipped_missed` instead. Pure.
fn pending_expired(last_fired_at: Option<i64>, now: i64, grace_min: i64) -> bool {
    // No fire stamp at all (hand-edited file, pre-upgrade record) → age is unknowable; treat it as
    // stale so a schedule can never stay wedged forever.
    let Some(fired_at) = last_fired_at else {
        return true;
    };
    now.saturating_sub(fired_at) > grace_min.saturating_mul(60_000)
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

/// A schedule that reached `Due::Fire` and still has to clear the (blocking) gates.
struct Candidate {
    id: String,
    day: String,
    label: String,
    recipe: ScheduleRecipe,
    quota_max_pct: Option<f64>,
    precheck: Option<(String, u64)>,
}

/// A decision taken under the lock (or produced by the gates) and written back under it.
struct Decision {
    id: String,
    /// Day key to record the outcome under; `None` = keep whichever day the entry already carries
    /// (the stale-pending sweep, whose day belongs to the original fire).
    day: Option<String>,
    outcome: &'static str,
    /// Set only for a fire — the payload the frontend launches.
    due: Option<ScheduleDue>,
    msg: Option<String>,
}

/// One tick: evaluate every enabled schedule, fire/skip, persist, notify. Public for tests via the
/// pure pieces; the thread wrapper below owns the cadence.
///
/// Three phases, because SCHED_LOCK is also taken by the four schedule commands: decide under the
/// lock (pure), run the gates unlocked (a precheck blocks up to a minute, the quota gate can make an
/// HTTPS call), then re-take the lock on a FRESH load to persist. Holding the lock across the gates
/// stalled every schedule command for as long as a precheck ran.
#[cfg(windows)]
fn tick(app: &AppHandle) {
    let Ok(path) = sched_path() else { return };
    let bus = crate::session_bus::bus_file_path();
    let (now_min, weekday, day_key) = local_now();
    let now = now_ms();

    // ── Phase 1: decide (locked, pure — no IO beyond the one load) ─────────────────────────────
    let (mut decisions, candidates) = {
        let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let f = load(&path);
        let mut decisions: Vec<Decision> = Vec::new();
        let mut candidates: Vec<Candidate> = Vec::new();
        for s in f.schedules.iter() {
            if s.pending {
                if pending_expired(s.last_fired_at, now, grace_of(s)) {
                    decisions.push(Decision {
                        id: s.id.clone(),
                        day: None,
                        outcome: "skipped_missed",
                        due: None,
                        msg: Some(format!(
                            "{}: пропущен (запуск не подхвачен вовремя)",
                            s.label_or_id()
                        )),
                    });
                }
                continue;
            }
            if !s.enabled {
                continue;
            }
            let Some(time_min) = parse_hhmm(&s.time) else { continue };
            let grace = grace_of(s);
            // Recover a late-yesterday occurrence whose grace window spilled past midnight; `okey` is
            // the day key the outcome is recorded under (today's, or yesterday's for a recovered fire).
            let yesterday = prev_day_key(weekday, &day_key).map(|(yw, yk)| {
                let yf = fired_today(s, &yk);
                (yw, yk, yf)
            });
            let (due, okey) = effective_due(
                time_min, &s.days, grace, now_min, weekday, &day_key, fired_today(s, &day_key), yesterday,
            );
            match due {
                Due::Wait => {}
                Due::Missed => decisions.push(Decision {
                    id: s.id.clone(),
                    day: Some(okey),
                    outcome: "skipped_missed",
                    due: None,
                    msg: Some(format!("{}: пропущен (окно {grace} мин прошло)", s.label_or_id())),
                }),
                Due::Fire => candidates.push(Candidate {
                    id: s.id.clone(),
                    day: okey,
                    label: s.label_or_id().to_string(),
                    recipe: s.recipe.clone(),
                    // The quota gate only means anything for a claude profile.
                    quota_max_pct: s.quota_gate_max_pct.filter(|_| s.recipe.env == "claude"),
                    precheck: s
                        .precheck
                        .as_deref()
                        .filter(|c| !c.trim().is_empty())
                        .map(|c| {
                            (
                                c.to_string(),
                                s.precheck_timeout_sec.unwrap_or(PRECHECK_DEFAULT_TIMEOUT_SEC),
                            )
                        }),
                }),
            }
        }
        (decisions, candidates)
    };

    // ── Phase 2: gates (UNLOCKED — blocking precheck + possible HTTPS quota lookup) ────────────
    for c in candidates {
        let skip = |outcome, msg| Decision {
            id: c.id.clone(),
            day: Some(c.day.clone()),
            outcome,
            due: None,
            msg: Some(msg),
        };
        // Gate 1: quota — a KNOWN-hot profile is skipped, unknown passes (see profile_h5).
        if let Some(maxp) = c.quota_max_pct {
            if let Some(h5) = profile_h5(&c.recipe.profile).filter(|h5| *h5 > maxp) {
                decisions.push(skip(
                    "skipped_quota",
                    format!("{}: пропущен (квота {h5:.0}% > {maxp:.0}%)", c.label),
                ));
                continue;
            }
        }
        // Gate 2: precheck (blocking, bounded — we're on the tick thread, not the UI).
        if let Some((cmd, timeout)) = &c.precheck {
            if let Err(why) = run_precheck(cmd, &c.recipe.folder, *timeout) {
                decisions.push(skip(
                    "skipped_precheck",
                    format!("{}: пропущен precheck ({why})", c.label),
                ));
                continue;
            }
        }
        decisions.push(Decision {
            id: c.id.clone(),
            day: Some(c.day.clone()),
            outcome: "fired",
            due: Some(ScheduleDue {
                id: c.id.clone(),
                label: c.label.clone(),
                recipe: c.recipe.clone(),
            }),
            msg: None,
        });
    }

    // ── Phase 3: persist (locked again, on a FRESH load) ───────────────────────────────────────
    if decisions.is_empty() {
        return;
    }
    let mut launches: Vec<ScheduleDue> = Vec::new();
    let mut posts: Vec<String> = Vec::new();
    {
        let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Re-read: a schedule command may have rewritten the file while the gates ran unlocked.
        // Only this entry's backend-owned bookkeeping is touched, so a concurrent edit survives.
        let mut f = load(&path);
        let mut changed = false;
        for d in decisions {
            let Some(s) = f.schedules.iter_mut().find(|s| s.id == d.id) else { continue };
            match &d.day {
                // Stale-pending sweep: only meaningful while it is still pending.
                None => {
                    if !s.pending {
                        continue;
                    }
                    s.pending = false;
                    swap_outcome(s, d.outcome);
                }
                // The user may have disabled or re-armed the schedule while the gates ran, or the
                // frontend may have acted on it — never overwrite a state that moved on without us.
                Some(day) => {
                    if !s.enabled || s.pending || fired_today(s, day) {
                        continue;
                    }
                    s.pending = d.due.is_some();
                    set_outcome(s, day, d.outcome);
                }
            }
            changed = true;
            if let Some(due) = d.due {
                launches.push(due);
            }
            if let Some(msg) = d.msg {
                posts.push(msg);
            }
        }
        if changed {
            let _ = store(&path, &f);
        }
    }
    // Announce only what was actually persisted — and only after the store, so the frontend can't
    // ack a fire that isn't on disk yet.
    for due in launches {
        let _ = app.emit("agent-schedule-due", due);
    }
    if let Ok(b) = &bus {
        for msg in posts {
            let _ = crate::session_bus::send(b, "scheduler", "@all", "schedule", &msg);
        }
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

// All four take SCHED_LOCK and touch the filesystem — `async` keeps that off the event-loop thread,
// so even a tick that is mid-store can't stall the UI.

#[tauri::command(async)]
pub fn read_agent_schedules() -> Result<Vec<AgentSchedule>, String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(load(&sched_path()?).schedules)
}

#[tauri::command(async)]
pub fn write_agent_schedules(schedules: Vec<AgentSchedule>) -> Result<(), String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    store(&sched_path()?, &ScheduleFile { schedules })
}

/// The frontend launched (or failed to launch) a pending schedule — record the outcome.
#[tauri::command(async)]
pub fn ack_agent_schedule(id: String, outcome: String) -> Result<(), String> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = sched_path()?;
    let mut f = load(&path);
    if let Some(s) = f.schedules.iter_mut().find(|s| s.id == id) {
        s.pending = false;
        swap_outcome(s, &outcome);
        store(&path, &f)?;
    }
    Ok(())
}

/// Pending entries (fired while Sessions wasn't mounted) — drained by the frontend on mount.
/// Entries whose grace window has already passed are withheld: Sessions mounts whenever the user
/// first opens the tab, which can be days after the fire, and launching then is exactly the
/// launch-in-the-past this module refuses. The next tick records them as `skipped_missed`.
#[tauri::command(async)]
pub fn pending_agent_schedules() -> Result<Vec<AgentSchedule>, String> {
    let now = now_ms();
    Ok(load_pending(&sched_path()?, now))
}

fn load_pending(path: &str, now: i64) -> Vec<AgentSchedule> {
    let _g = SCHED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load(path)
        .schedules
        .into_iter()
        .filter(|s| s.pending && !pending_expired(s.last_fired_at, now, grace_of(s)))
        .collect()
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
    fn prev_day_key_handles_boundaries() {
        assert_eq!(prev_day_key(3, "2026-07-19"), Some((2, "2026-07-18".into()))); // Wed→Tue
        assert_eq!(prev_day_key(1, "2026-07-01"), Some((7, "2026-06-30".into()))); // Mon→Sun, month edge
        assert_eq!(prev_day_key(4, "2026-01-01"), Some((3, "2025-12-31".into()))); // year edge
        assert_eq!(prev_day_key(1, "2024-03-01"), Some((7, "2024-02-29".into()))); // leap year
        assert_eq!(prev_day_key(1, "2026-03-01"), Some((7, "2026-02-28".into()))); // non-leap
        assert_eq!(prev_day_key(3, "garbage"), None);
        assert_eq!(prev_day_key(3, "2026-13-01"), None); // invalid month
    }

    #[test]
    fn effective_due_recovers_yesterday_spilled_grace() {
        let yest = |fired| Some((2u8, "2026-07-18".to_string(), fired));
        // 23:50 (1430) + 30m grace, app asleep until 00:05 (now_min=5): yesterday's window
        // [1430,1460] covers 1445 on the continuous timeline → recover FIRE under yesterday's key.
        assert_eq!(
            effective_due(1430, &[], 30, 5, 3, "2026-07-19", false, yest(false)),
            (Due::Fire, "2026-07-18".to_string())
        );
        // Woke at 00:25 (1465 > 1460): past yesterday's window → no launch-in-the-past, today waits.
        assert_eq!(
            effective_due(1430, &[], 30, 25, 3, "2026-07-19", false, yest(false)),
            (Due::Wait, "2026-07-19".to_string())
        );
        // Yesterday already acted → no re-fire.
        assert_eq!(
            effective_due(1430, &[], 30, 5, 3, "2026-07-19", false, yest(true)),
            (Due::Wait, "2026-07-19".to_string())
        );
        // A daytime schedule never enters the midnight branch — today's own fire stands.
        assert_eq!(
            effective_due(540, &[], 30, 545, 3, "2026-07-19", false, yest(false)),
            (Due::Fire, "2026-07-19".to_string())
        );
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
    fn pending_expires_after_its_grace_window() {
        let fired = 1_000_000_000_000i64;
        let grace = 30; // minutes
        assert!(!pending_expired(Some(fired), fired, grace));
        assert!(!pending_expired(Some(fired), fired + 30 * 60_000, grace));
        assert!(pending_expired(Some(fired), fired + 30 * 60_000 + 1, grace));
        // Hours later — the "launch-in-the-past" case the drain must refuse.
        assert!(pending_expired(Some(fired), fired + 6 * 3_600_000, grace));
        // No fire stamp at all → unknowable age, treated as stale so nothing wedges forever.
        assert!(pending_expired(None, fired, grace));
        // Clock skew (stamp in the future) must not read as stale.
        assert!(!pending_expired(Some(fired + 60_000), fired, grace));
    }

    #[test]
    fn stale_pending_is_withheld_from_the_drain() {
        // The frontend drains pending entries on Sessions mount, which may be days after the fire.
        // A pending entry past its grace window must NOT be handed over for launch.
        let dir = std::env::temp_dir().join("castellyn-sched-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("drain.json").to_string_lossy().to_string();
        let fired = 1_000_000_000_000i64;
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
            grace_minutes: Some(30),
            last_fired_at: Some(fired),
            last_outcome: Some("2026-07-19|fired".into()),
            pending: true,
        };
        store(&path, &ScheduleFile { schedules: vec![s.clone()] }).unwrap();
        // Inside the window → drained normally.
        assert_eq!(load_pending(&path, fired + 60_000).len(), 1);
        // Past it → withheld.
        assert!(load_pending(&path, fired + 6 * 3_600_000).is_empty());
        // A non-pending entry is never drained regardless of age.
        s.pending = false;
        store(&path, &ScheduleFile { schedules: vec![s] }).unwrap();
        assert!(load_pending(&path, fired + 60_000).is_empty());
    }

    #[test]
    fn swap_outcome_keeps_the_day_key() {
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
        // Nothing recorded yet → nothing to swap (no phantom day key invented).
        swap_outcome(&mut s, "launched");
        assert_eq!(s.last_outcome, None);
        set_outcome(&mut s, "2026-07-19", "fired");
        swap_outcome(&mut s, "skipped_missed");
        assert_eq!(s.last_outcome.as_deref(), Some("2026-07-19|skipped_missed"));
        // The day still counts as acted-on, so the schedule doesn't re-fire the same day.
        assert!(fired_today(&s, "2026-07-19"));
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
