//! Agent status for Sessions panes (herdr-inspired).
//!
//! Semantic states: `working` | `blocked` | `idle` | `unknown`. "done" is a FRONTEND
//! notion (working/blocked → idle while the pane is unfocused), mirroring herdr's
//! Idle+!seen model. Authorities, strongest first:
//!  1. Claude Code lifecycle hooks — `castellyn_status.py` writes
//!     `%APPDATA%\castellyn\agent-status\<session_id>.json` on each lifecycle event.
//!  2. PTY output activity — a working heartbeat (full-screen agent TUIs repaint their
//!     spinner constantly, so silence is a reliable idle signal) that also self-heals a
//!     stale `blocked` after the user answers the prompt (no hook fires on approval).
//!  3. Process exit → idle.
//!
//! One poll thread (500 ms) recomputes every tracked session and emits an
//! `agent-status` event only on change. Sessions of tool `shell`/`ssh` are not tracked.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::{Emitter, Manager};

const POLL_MS: u64 = 500;
/// Right after spawn nothing meaningful has happened yet — report `unknown`.
const STARTUP_GRACE_MS: u64 = 3_000;
/// No PTY output for this long → not actively working. This governs the hookless activity branch,
/// which is the PRIMARY (only) turn signal for codex/opencode and remote claude. It was 4s, but real
/// turns go quiet well past 4s (deep thinking, a long tool/MCP/network call) — the same false-"done"
/// that WORKING_SELFHEAL_MS was raised to fix for hook-claude (live-smoke 2026-07-03). Claude's 4s was
/// safe only because a Stop hook corrects it; codex/opencode have no such correction, so 4s fired a
/// false completion on every slightly-slow turn. 15s survives normal pauses; truly-precise done/blocked
/// for these agents needs their own signals (Codex `notify` / opencode plugin) — a separate follow-up.
const ACTIVITY_IDLE_MS: u64 = 15_000;
/// A hook-reported `working` self-heals to idle only after this long of silence — the fallback for a
/// turn that ended WITHOUT a Stop hook (Esc-interrupt / crashed hook). Far longer than
/// ACTIVITY_IDLE_MS: real turns go quiet well past 4s (deep thinking, a long tool/MCP call, a network
/// wait), and treating those as "done" fired a false completion toast (live-smoke 2026-07-03). The
/// real end-of-turn signal is the Stop hook (Some("idle")); this is only the hookless backstop.
const WORKING_SELFHEAL_MS: u64 = 35_000;
/// Grace after a hook-reported `blocked` within which PTY output counts as the prompt box
/// painting itself, not a real resume — used by the time backstop below.
const BLOCKED_RESUME_MS: u64 = 1_500;
/// A resumed agent turn floods the PTY; a prompt-box repaint is small. Clear a hook-reported
/// `blocked` once this many bytes arrive since the block began (item 6, hook-less fallback).
const BLOCKED_RESUME_BYTES: u64 = 1_024;
/// Time backstop: after this long in `blocked` with real post-block output but no byte burst
/// (an Esc answer emits little), allow the flip so `blocked` can't stick forever.
const BLOCKED_STUCK_MS: u64 = 20_000;
/// After a detected usage limit, the session sits quiet until its window resets; a genuine resume
/// then floods far more than this, so that many bytes since the limit clears the `limited` state
/// (item 21b). Higher than the block threshold — a limit banner + its surrounding repaint is larger.
const LIMIT_RESUME_BYTES: u64 = 4_096;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Track {
    tool: String,
    /// A lifecycle hook is expected for this session (local claude only): the authoritative
    /// working/idle signal. When true but no hook has ever reported, PTY activity is NOT used
    /// as a fallback — Claude Code and background hooks (claude-mem) print into the terminal, so
    /// activity ≠ a live turn — the session reports `unknown` instead of a false `working`
    /// (A-residual). Remote claude / codex / opencode are genuinely hookless → heartbeat is their
    /// only signal, so this is false and the activity branch applies.
    hook_expected: bool,
    /// Human label for notifications ("claude · cc1", "codex").
    label: String,
    spawned_at: u64,
    /// Unix ms of the last PTY output. Atomic so `on_output` can update it under a shared
    /// borrow (still under the TRACKS lock — see item-8 scope note).
    last_output: AtomicU64,
    /// Bytes emitted since the current `blocked` state began (reset in `apply_hook_report`).
    /// The hook-less fallback for clearing a stale `blocked` (item 6).
    bytes_since_block: AtomicU64,
    /// A usage limit was detected in this session's PTY output (item 21b). Shown as `limited`
    /// until a genuine resume (LIMIT_RESUME_BYTES of output past the limit) clears it.
    limited: AtomicBool,
    /// Bytes emitted since `limited` was set — the resume signal that clears it.
    bytes_since_limit: AtomicU64,
    exited: bool,
    /// Latest hook-reported state ("working" | "blocked" | "idle"; "ended" clears it).
    hook_state: Option<String>,
    hook_ts: u64,
    /// Last-seen mtime (unix ms) of this session's hook file; skip the read+parse when it
    /// hasn't changed (item 8 mtime gate).
    hook_mtime: u64,
    claude_session_id: Option<String>,
    last_emitted: Option<String>,
}

static TRACKS: LazyLock<Mutex<HashMap<String, Track>>> = LazyLock::new(Default::default);

/// %APPDATA%\castellyn\agent-status (hook output directory).
pub fn status_dir() -> Option<std::path::PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| std::path::Path::new(&a).join("castellyn").join("agent-status"))
}

/// Register a freshly-spawned session. `shell`/`ssh` panes carry no agent — skipped.
/// `hook_expected` is true only for a LOCAL claude pane, whose lifecycle hook can reach the
/// local status dir; remote claude and codex/opencode are hookless (heartbeat only).
pub fn on_spawn(id: &str, tool: &str, profile: &str, hook_expected: bool) {
    if !matches!(tool, "claude" | "opencode" | "codex") {
        return;
    }
    let now = now_ms();
    TRACKS.lock().unwrap_or_else(|e| e.into_inner()).insert(
        id.to_string(),
        Track {
            tool: tool.to_string(),
            hook_expected,
            label: if tool == "claude" && !profile.is_empty() {
                format!("{tool} · {profile}")
            } else {
                tool.to_string()
            },
            spawned_at: now,
            last_output: AtomicU64::new(now),
            bytes_since_block: AtomicU64::new(0),
            limited: AtomicBool::new(false),
            bytes_since_limit: AtomicU64::new(0),
            exited: false,
            hook_state: None,
            hook_ts: 0,
            hook_mtime: 0,
            claude_session_id: None,
            last_emitted: None,
        },
    );
}

/// PTY reader thread: `bytes` arrived for this session. Shared borrow (atomic fields) so it
/// needs no exclusive access, though it still takes the TRACKS lock to find the entry.
pub fn on_output(id: &str, bytes: usize) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        t.last_output.store(now_ms(), Ordering::Relaxed);
        t.bytes_since_block.fetch_add(bytes as u64, Ordering::Relaxed);
        // A genuine resume after a limit floods output; once enough has arrived, clear `limited`.
        if t.limited.load(Ordering::Relaxed)
            && t.bytes_since_limit.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64
                > LIMIT_RESUME_BYTES
        {
            t.limited.store(false, Ordering::Relaxed);
        }
    }
}

/// Mark a session as usage-limited (item 21b): the PTY reader detected a "limit reached" banner.
/// Only claude panes carry an agent; unknown ids are ignored. Resets the resume counter so the
/// state holds until real output resumes past LIMIT_RESUME_BYTES.
pub fn on_limit(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        if t.tool == "claude" {
            t.bytes_since_limit.store(0, Ordering::Relaxed);
            t.limited.store(true, Ordering::Relaxed);
        }
    }
}

/// True when a line signals a Claude Code usage limit. Anchored on the qualified banner wording
/// ("usage limit reached", "N-hour limit reached") rather than a bare "limit reached", so agent
/// output merely discussing limits doesn't flip the badge; still tolerant of version drift, with the
/// endpoint monitor (limits.rs) as the confirming/secondary signal. Pure + unit-tested.
fn is_limit_line(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("usage limit reached")
        || l.contains("hour limit reached")
        || l.contains("out of extra usage")
}

/// Scan a fresh PTY chunk's tail for a usage-limit banner and flag the session if found. The reader
/// passes the raw bytes; we inspect a bounded tail (banners are short lines) to keep it cheap under
/// a firehose. ponytail: bounded-tail scan, not a full-buffer regex — a banner split across two
/// chunk boundaries beyond the tail window would be missed; the endpoint monitor still catches it.
pub fn scan_limit(id: &str, chunk: &[u8]) {
    const TAIL: usize = 512;
    let start = chunk.len().saturating_sub(TAIL);
    let tail = String::from_utf8_lossy(&chunk[start..]);
    if is_limit_line(&tail) {
        on_limit(id);
    }
}

/// PTY reader thread hit EOF (child exited). The poll loop emits the final `idle` and
/// drops the track.
pub fn on_exit(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(id)
    {
        t.exited = true;
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    id: String,
    state: String,
    claude_session_id: Option<String>,
    /// Session spawn time (unix ms), static per session — the frontend derives "active for N"
    /// from `now - spawnedAt` on render (no ticking backend events).
    spawned_at: u64,
    #[serde(skip)]
    prev: Option<String>,
    #[serde(skip)]
    label: String,
    #[serde(skip)]
    exited: bool,
    /// The hook reported `idle` (a real Stop) at this emit — gates the completion toast so a
    /// hookless activity-lull can't fire a false "finished" (live-smoke 2026-07-03). Serialized
    /// (`hookIdle`) so the frontend gates its visual "done" the same way as the toast.
    hook_idle: bool,
}

/// System sound for a transition (no bundled audio: MessageBeep respects the user's
/// sound scheme and mute state). No-op on non-Windows.
fn beep(attention: bool) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONASTERISK, MB_ICONEXCLAMATION};
        let _ = MessageBeep(if attention {
            MB_ICONEXCLAMATION
        } else {
            MB_ICONASTERISK
        });
    }
    #[cfg(not(windows))]
    let _ = attention;
}

/// Popup + sound policy (herdr-style): →blocked = attention; working/blocked→idle =
/// background completion. Suppressed while any Castellyn window is focused — the user
/// is already looking at the app.
fn notify_transition(app: &tauri::AppHandle, ev: &StatusEvent) {
    // A closed/exited pane also lands on idle — that's teardown, not a completion worth
    // a "finished" toast (closing a working pane must stay silent).
    if ev.exited {
        return;
    }
    let to_blocked = ev.state == "blocked" && ev.prev.as_deref() != Some("blocked");
    // A pane hitting its usage limit is attention-worthy the same way as blocked: it's stalled on
    // quota until the window resets. Same focus-gate + attention beep, its own toast text.
    let to_limited = ev.state == "limited" && ev.prev.as_deref() != Some("limited");
    // "Finished" toast only on a REAL end-of-turn = the Stop hook fired (hook_idle). An activity-lull
    // idle just greys the dot; it must NOT claim "done". This applies to codex/opencode too: they have
    // no hook, so their idle is pure PTY-silence — clicking into the pane (cursor repaint) or any
    // terminal noise would flip working→idle and fire a false "Агент закончил" though nothing ran
    // (owner live-smoke). So NO completion toast for hookless agents until they gain a real turn
    // signal (Codex `notify` / opencode plugin — Phase 2b). The status dot still tracks working/idle.
    let completed = ev.state == "idle"
        && matches!(ev.prev.as_deref(), Some("working") | Some("blocked"))
        && ev.hook_idle;
    if !to_blocked && !completed && !to_limited {
        return;
    }
    if app
        .webview_windows()
        .values()
        .any(|w| w.is_focused().unwrap_or(false))
    {
        return;
    }
    let cfg = crate::read_config_file();
    let lang = crate::cur_lang();
    if cfg.status_sounds.unwrap_or(true) {
        beep(to_blocked || to_limited);
    }
    if cfg.status_notify.unwrap_or(true) {
        use tauri_plugin_notification::NotificationExt;
        let (tk, bk) = if to_blocked {
            ("status.blocked_title", "status.blocked_body")
        } else if to_limited {
            ("notify.limited_title", "notify.limited_body")
        } else {
            ("status.done_title", "status.done_body")
        };
        let _ = app
            .notification()
            .builder()
            .title(crate::i18n::tr(tk, lang))
            .body(crate::i18n::trv(bk, lang, &[("label", &ev.label)]))
            .show();
    }
}

fn compute(t: &Track, now: u64) -> &'static str {
    if t.exited {
        return "idle";
    }
    // A detected usage limit outranks the hook/activity states: the session is stalled on quota
    // until its window resets (cleared in on_output once real output resumes). (item 21b)
    if t.limited.load(Ordering::Relaxed) {
        return "limited";
    }
    let last_output = t.last_output.load(Ordering::Relaxed);
    let silent = now.saturating_sub(last_output) > ACTIVITY_IDLE_MS;
    match t.hook_state.as_deref() {
        // Blocked holds until the agent clearly resumed: either a byte burst since the block
        // (approval floods the PTY) or, as a backstop, real post-block output that has sat
        // past the stuck ceiling so a small (Esc-answer) response still recovers. A bare
        // prompt-box repaint (small, no burst) must NOT clear it — the old bug (item 6).
        Some("blocked") => {
            let burst = t.bytes_since_block.load(Ordering::Relaxed) > BLOCKED_RESUME_BYTES;
            let real_output = last_output > t.hook_ts + BLOCKED_RESUME_MS;
            let stuck = now.saturating_sub(t.hook_ts) > BLOCKED_STUCK_MS;
            if burst || (stuck && real_output) {
                "working"
            } else {
                "blocked"
            }
        }
        // A silent "working" self-heals to idle ONLY after a long backstop: the real end-of-turn
        // signal is the Stop hook (→ Some("idle") below). This branch just recovers a turn that
        // ended without one (Esc-interrupt / crashed hook). A short output gap (<WORKING_SELFHEAL_MS)
        // is still an active turn — flipping it to idle at 4s fired a false "done" (live-smoke).
        Some("working") => {
            if now.saturating_sub(last_output) > WORKING_SELFHEAL_MS {
                "idle"
            } else {
                "working"
            }
        }
        // Hook-reported idle is authoritative: prompt-box echo/typing must not flip it —
        // the next UserPromptSubmit hook reports working.
        Some("idle") => "idle",
        // No hook authority yet. PTY activity is the fallback, but ONLY for genuinely hookless
        // sessions (codex/opencode, remote claude). A turn stays `working` through normal
        // think/tool pauses (ACTIVITY_IDLE_MS = 15s) so a hookless agent doesn't false-flip to
        // done mid-turn.
        _ => {
            if now.saturating_sub(t.spawned_at) < STARTUP_GRACE_MS {
                "unknown"
            } else if t.hook_expected {
                // Local claude that expected a hook but never got one → the Agent-statuses hook is
                // off/unwired. Claude Code and its background hooks (claude-mem) print into the PTY
                // even when the user isn't in a turn, so activity is an unreliable proxy that
                // false-flags `working` (A-residual). Report `unknown` (neutral dot, uncounted, no
                // false "done") instead of lying; enabling the hook restores authoritative status.
                "unknown"
            } else if silent {
                "idle"
            } else {
                "working"
            }
        }
    }
}

/// Read this session's hook file into the track (cheap: ~1 tiny JSON per tracked pane
/// per poll; only local claude panes ever have one).
fn apply_hook_report(v: &serde_json::Value, t: &mut Track) {
    let ts = v.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);
    if ts <= t.hook_ts {
        return; // stale / unchanged
    }
    t.hook_ts = ts;
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
    // SessionEnd → the agent is gone (pane is a plain shell again): drop hook authority.
    t.hook_state = match state {
        "ended" | "" => None,
        s => Some(s.to_string()),
    };
    // A fresh block starts the byte-burst counter from zero (item 6 fallback). The initial
    // prompt-box paint usually lands before this poll reads the hook file, so it isn't
    // counted; only output after the block accrues.
    // ponytail: a large plan-approval box that repaints AFTER this reset (e.g. terminal
    // resize) could exceed BLOCKED_RESUME_BYTES and false-clear; upgrade path is a short
    // settle delay before counting. Rare enough to leave.
    if t.hook_state.as_deref() == Some("blocked") {
        t.bytes_since_block.store(0, Ordering::Relaxed);
    }
    if let Some(cs) = v
        .get("claudeSessionId")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        t.claude_session_id = Some(cs.to_string());
    }
}

/// Start the poll thread. Called once from `setup()`.
/// (blocked, limited) counts across live panes, from each track's last-emitted state. Cheap
/// snapshot for the tray tooltip — no recompute, just reads what the poll already published.
pub(crate) fn attention_counts() -> (usize, usize) {
    let map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
    let mut blocked = 0;
    let mut limited = 0;
    for t in map.values() {
        match t.last_emitted.as_deref() {
            Some("blocked") => blocked += 1,
            Some("limited") => limited += 1,
            _ => {}
        }
    }
    (blocked, limited)
}

/// Prune week-old hook files (claude session ids in them feed session restore, so recent ones are
/// kept across app restarts). Runs on the poll thread, not on `setup()`: it is a read_dir + a
/// metadata call per file, and the first frame should not wait on however many accumulated.
fn prune_stale_hook_files() {
    let Some(dir) = status_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|m| m.elapsed().map(|d| d.as_secs() > 7 * 86_400).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        prune_stale_hook_files();
        loop {
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        crate::run_guarded("agent-status", || {
        // Nothing tracked (no session panes open) → no state can change, so skip the whole tick
        // instead of stat-ing the hook dir twice a second for the lifetime of the app.
        if TRACKS.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            return;
        }
        let dir = status_dir();
        // Read hook files OUTSIDE the tracks lock: on_output() takes that lock from every
        // PTY reader thread per chunk, so fs reads (AV scans can stall them) must not
        // serialize against it. Only local claude panes ever have a hook file.
        let claude_ids: Vec<(String, u64)> = {
            let map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
            map.iter()
                .filter(|(_, t)| t.tool == "claude")
                .map(|(id, t)| (id.clone(), t.hook_mtime))
                .collect()
        };
        // Report value plus the mtime that produced it, so the poll section can store it.
        let mut reports: HashMap<String, (u64, serde_json::Value)> = HashMap::new();
        if let Some(d) = dir.as_deref() {
            for (id, seen_mtime) in claude_ids {
                let path = d.join(format!("{id}.json"));
                // mtime gate: stat is far cheaper than read+parse. Skip when unchanged; a
                // missing file (mtime 0) is skipped too, exactly as the old read would fail.
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                if mtime == 0 || mtime == seen_mtime {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(v) = serde_json::from_str(&text) {
                        reports.insert(id, (mtime, v));
                    }
                }
            }
        }
        let mut events: Vec<StatusEvent> = Vec::new();
        {
            let mut map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_ms();
            map.retain(|id, t| {
                if let Some((mtime, v)) = reports.get(id) {
                    apply_hook_report(v, t);
                    t.hook_mtime = *mtime;
                }
                let state = compute(t, now);
                if t.last_emitted.as_deref() != Some(state) {
                    let prev = t.last_emitted.take();
                    t.last_emitted = Some(state.to_string());
                    events.push(StatusEvent {
                        id: id.clone(),
                        state: state.to_string(),
                        claude_session_id: t.claude_session_id.clone(),
                        spawned_at: t.spawned_at,
                        prev,
                        label: t.label.clone(),
                        exited: t.exited,
                        hook_idle: t.hook_state.as_deref() == Some("idle"),
                    });
                }
                !t.exited // exited sessions emit their final idle above, then drop
            });
        }
        let changed = !events.is_empty();
        for ev in events {
            notify_transition(&app, &ev);
            let _ = app.emit("agent-status", ev);
        }
        // Only when a state actually changed — the tooltip's attention line reads these counts.
        if changed {
            crate::update_tray_tooltip(&app);
        }
        });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(tool: &str, now: u64) -> Track {
        Track {
            tool: tool.into(),
            // Default hookless (codex/opencode/remote-claude); the local-claude tests set it true.
            hook_expected: false,
            label: tool.into(),
            spawned_at: now,
            last_output: AtomicU64::new(now),
            bytes_since_block: AtomicU64::new(0),
            limited: AtomicBool::new(false),
            bytes_since_limit: AtomicU64::new(0),
            exited: false,
            hook_state: None,
            hook_ts: 0,
            hook_mtime: 0,
            claude_session_id: None,
            last_emitted: None,
        }
    }

    #[test]
    fn limit_line_detection_is_tolerant() {
        assert!(is_limit_line("Claude usage limit reached. Your limit will reset at 3pm"));
        assert!(is_limit_line("5-hour limit reached"));
        assert!(is_limit_line("You are out of extra usage"));
        assert!(is_limit_line("USAGE LIMIT REACHED")); // case-insensitive
        assert!(!is_limit_line("running the linter, no limits here"));
        assert!(!is_limit_line("rate limited by the API")); // not our banner wording
        // Anchored: a bare "limit reached" in ordinary agent output must NOT flip the badge.
        assert!(!is_limit_line("the rate limit reached its cap in the test fixture"));
        assert!(!is_limit_line("// TODO: handle when the retry limit reached"));
    }

    #[test]
    fn limited_state_outranks_and_clears_on_resume() {
        let now = 1_000_000;
        let t = track("claude", now);
        // A limit banner flags the session; compute reports `limited` regardless of hook/activity
        // (track() leaves hook_state None, so the limited flag is what's exercised here).
        t.limited.store(true, Ordering::Relaxed);
        t.last_output.store(now + 10_000, Ordering::Relaxed); // even with recent output
        assert_eq!(compute(&t, now + 11_000), "limited");
        // A small trickle does NOT clear it (mirrors on_output's accumulate-then-compare).
        t.bytes_since_limit.store(0, Ordering::Relaxed);
        let small = 100u64;
        if t.limited.load(Ordering::Relaxed)
            && t.bytes_since_limit.fetch_add(small, Ordering::Relaxed) + small > LIMIT_RESUME_BYTES
        {
            t.limited.store(false, Ordering::Relaxed);
        }
        assert_eq!(compute(&t, now + 12_000), "limited");
        // A genuine resume (flood past the threshold) clears it → back to normal activity states.
        let big = LIMIT_RESUME_BYTES + 1;
        if t.limited.load(Ordering::Relaxed)
            && t.bytes_since_limit.fetch_add(big, Ordering::Relaxed) + big > LIMIT_RESUME_BYTES
        {
            t.limited.store(false, Ordering::Relaxed);
        }
        assert_ne!(compute(&t, now + 13_000), "limited");
    }

    #[test]
    fn activity_only_lifecycle() {
        // codex/opencode (no hooks): grace → working while output flows → idle on silence.
        let now = 1_000_000;
        let mut t = track("codex", now);
        assert_eq!(compute(&t, now + 1_000), "unknown"); // startup grace
        t.last_output
            .store(now + STARTUP_GRACE_MS + 1_000, Ordering::Relaxed);
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_000), "working");
        // A short think/tool pause (5s < ACTIVITY_IDLE_MS) must NOT false-flip to idle — the codex/
        // opencode false-"done" fix. (Was 4s → this asserted idle here; now it stays working.)
        assert_eq!(
            compute(&t, t.last_output.load(Ordering::Relaxed) + 5_000),
            "working"
        );
        assert_eq!(
            compute(
                &t,
                t.last_output.load(Ordering::Relaxed) + ACTIVITY_IDLE_MS + 1_000
            ),
            "idle"
        );
        t.exited = true;
        assert_eq!(compute(&t, now), "idle");
    }

    #[test]
    fn hook_expected_claude_stays_unknown_without_hook() {
        // A-residual: a LOCAL claude pane whose Agent-statuses hook is off/unwired must NOT infer
        // `working` from PTY activity — Claude Code + background hooks (claude-mem) print into the
        // terminal even when idle. It reports `unknown` (neutral) until a real hook event arrives.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_expected = true;
        assert_eq!(compute(&t, now + 1_000), "unknown"); // startup grace
        // Fresh PTY output past the grace (background noise) — must stay `unknown`, NOT `working`.
        t.last_output
            .store(now + STARTUP_GRACE_MS + 2_000, Ordering::Relaxed);
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_500), "unknown");
        // Once a real lifecycle hook reports, authority takes over immediately.
        t.hook_state = Some("working".into());
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_500), "working");
        // Contrast: a genuinely hookless agent (remote claude / codex) DOES use the heartbeat.
        let r = track("claude", now); // hook_expected = false (remote)
        r.last_output
            .store(now + STARTUP_GRACE_MS + 2_000, Ordering::Relaxed);
        assert_eq!(compute(&r, now + STARTUP_GRACE_MS + 2_500), "working");
    }

    #[test]
    fn hook_authority_and_self_heal() {
        let now = 1_000_000;
        let mut t = track("claude", now);
        // Hook says blocked → stays blocked while the prompt just repaints (small trickle,
        // no byte burst) even long after the block.
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.last_output.store(now + 200, Ordering::Relaxed); // the prompt menu painting itself
        assert_eq!(compute(&t, now + 60_000), "blocked");
        // …until a byte burst floods in (user approved, agent resumed its turn).
        t.bytes_since_block
            .store(BLOCKED_RESUME_BYTES + 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 3_000), "working");
        // Hook-idle is authoritative even with echo activity (typing in the prompt box).
        t.hook_state = Some("idle".into());
        t.last_output.store(now + 10_000, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 10_100), "idle");
        // Hook-working self-heals to idle only after the LONG backstop (Esc interrupt fires no Stop
        // hook). A sub-backstop gap is still an active turn — flipping it fired a false "done".
        t.hook_state = Some("working".into());
        assert_eq!(
            compute(&t, t.last_output.load(Ordering::Relaxed) + ACTIVITY_IDLE_MS + 1),
            "working"
        );
        assert_eq!(
            compute(&t, t.last_output.load(Ordering::Relaxed) + WORKING_SELFHEAL_MS + 1),
            "idle"
        );
    }

    #[test]
    fn status_event_carries_spawned_at() {
        // The poll-loop push site copies the track's spawn time into the emitted event so the
        // frontend can render "active for N". Guard against it landing as 0.
        let now = now_ms();
        let t = track("claude", now);
        let ev = StatusEvent {
            id: "s1".into(),
            state: "working".into(),
            claude_session_id: None,
            spawned_at: t.spawned_at,
            prev: None,
            label: t.label.clone(),
            exited: t.exited,
            hook_idle: false,
        };
        assert_ne!(ev.spawned_at, 0);
        assert_eq!(ev.spawned_at, now);
    }

    #[test]
    fn blocked_clears_on_byte_burst_not_trickle() {
        // Item 6: a small post-block trickle (prompt repaint) keeps `blocked`; a substantial
        // byte burst (the agent resumed its turn) clears it.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.last_output.store(now + 500, Ordering::Relaxed);
        t.bytes_since_block.store(64, Ordering::Relaxed); // under the threshold
        assert_eq!(compute(&t, now + 2_000), "blocked");
        t.bytes_since_block
            .store(BLOCKED_RESUME_BYTES + 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 2_100), "working");
    }

    #[test]
    fn blocked_time_backstop_recovers_on_sparse_output() {
        // Item 6 backstop: little output (an Esc answer) never reaches the byte threshold,
        // but once past the stuck ceiling with real post-block output it recovers to working.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.bytes_since_block.store(32, Ordering::Relaxed); // below BLOCKED_RESUME_BYTES
        t.last_output
            .store(now + BLOCKED_RESUME_MS + 3_000, Ordering::Relaxed); // real post-block output
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS - 1_000), "blocked"); // before ceiling
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS + 1_000), "working"); // after ceiling
    }
}
