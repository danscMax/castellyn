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
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

const POLL_MS: u64 = 500;
/// Right after spawn nothing meaningful has happened yet — report `unknown`.
const STARTUP_GRACE_MS: u64 = 3_000;
/// No PTY output for this long → not actively working.
const ACTIVITY_IDLE_MS: u64 = 4_000;
/// Output resuming this long after a hook-reported `blocked` means the user answered the
/// permission prompt in the terminal — flip back to working.
const BLOCKED_RESUME_MS: u64 = 1_500;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Track {
    tool: String,
    spawned_at: u64,
    last_output: u64,
    exited: bool,
    /// Latest hook-reported state ("working" | "blocked" | "idle"; "ended" clears it).
    hook_state: Option<String>,
    hook_ts: u64,
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
pub fn on_spawn(id: &str, tool: &str) {
    if !matches!(tool, "claude" | "opencode" | "codex") {
        return;
    }
    let now = now_ms();
    TRACKS.lock().unwrap_or_else(|e| e.into_inner()).insert(
        id.to_string(),
        Track {
            tool: tool.to_string(),
            spawned_at: now,
            last_output: now,
            exited: false,
            hook_state: None,
            hook_ts: 0,
            claude_session_id: None,
            last_emitted: None,
        },
    );
}

/// PTY reader thread: bytes arrived for this session.
pub fn on_output(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(id)
    {
        t.last_output = now_ms();
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
}

fn compute(t: &Track, now: u64) -> &'static str {
    if t.exited {
        return "idle";
    }
    let silent = now.saturating_sub(t.last_output) > ACTIVITY_IDLE_MS;
    match t.hook_state.as_deref() {
        // Blocked holds until output resumes well after the prompt painted — the user
        // answered in the terminal and the agent went back to work.
        Some("blocked") => {
            if t.last_output > t.hook_ts + BLOCKED_RESUME_MS {
                "working"
            } else {
                "blocked"
            }
        }
        // A silent "working" self-heals to idle: Esc-interrupts end a turn without a
        // Stop hook firing.
        Some("working") => {
            if silent {
                "idle"
            } else {
                "working"
            }
        }
        // Hook-reported idle is authoritative: prompt-box echo/typing must not flip it —
        // the next UserPromptSubmit hook reports working.
        Some("idle") => "idle",
        // No hook authority (codex/opencode, remote claude, or claude before its first
        // event): PTY activity decides.
        _ => {
            if now.saturating_sub(t.spawned_at) < STARTUP_GRACE_MS {
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
fn absorb_hook_file(dir: &std::path::Path, id: &str, t: &mut Track) {
    if t.tool != "claude" {
        return;
    }
    let Ok(text) = std::fs::read_to_string(dir.join(format!("{id}.json"))) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
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
    if let Some(cs) = v
        .get("claudeSessionId")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        t.claude_session_id = Some(cs.to_string());
    }
}

/// Start the poll thread. Called once from `setup()`.
pub fn start(app: tauri::AppHandle) {
    // Prune week-old hook files (claude session ids in them feed session restore, so we
    // keep recent ones across app restarts).
    if let Some(dir) = status_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(entries) = std::fs::read_dir(&dir) {
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
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        let dir = status_dir();
        let mut events: Vec<StatusEvent> = Vec::new();
        {
            let mut map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_ms();
            map.retain(|id, t| {
                if let Some(d) = dir.as_deref() {
                    absorb_hook_file(d, id, t);
                }
                let state = compute(t, now);
                if t.last_emitted.as_deref() != Some(state) {
                    t.last_emitted = Some(state.to_string());
                    events.push(StatusEvent {
                        id: id.clone(),
                        state: state.to_string(),
                        claude_session_id: t.claude_session_id.clone(),
                    });
                }
                !t.exited // exited sessions emit their final idle above, then drop
            });
        }
        for ev in events {
            let _ = app.emit("agent-status", ev);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(tool: &str, now: u64) -> Track {
        Track {
            tool: tool.into(),
            spawned_at: now,
            last_output: now,
            exited: false,
            hook_state: None,
            hook_ts: 0,
            claude_session_id: None,
            last_emitted: None,
        }
    }

    #[test]
    fn activity_only_lifecycle() {
        // codex/opencode (no hooks): grace → working while output flows → idle on silence.
        let now = 1_000_000;
        let mut t = track("codex", now);
        assert_eq!(compute(&t, now + 1_000), "unknown"); // startup grace
        t.last_output = now + STARTUP_GRACE_MS + 1_000;
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_000), "working");
        assert_eq!(
            compute(&t, t.last_output + ACTIVITY_IDLE_MS + 1_000),
            "idle"
        );
        t.exited = true;
        assert_eq!(compute(&t, now), "idle");
    }

    #[test]
    fn hook_authority_and_self_heal() {
        let now = 1_000_000;
        let mut t = track("claude", now);
        // Hook says blocked → stays blocked while the prompt just sits there…
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.last_output = now + 200; // the prompt menu painting itself
        assert_eq!(compute(&t, now + 60_000), "blocked");
        // …until output resumes well after the prompt painted (user approved).
        t.last_output = now + BLOCKED_RESUME_MS + 500;
        assert_eq!(compute(&t, now + BLOCKED_RESUME_MS + 600), "working");
        // Hook-idle is authoritative even with echo activity (typing in the prompt box).
        t.hook_state = Some("idle".into());
        t.last_output = now + 10_000;
        assert_eq!(compute(&t, now + 10_100), "idle");
        // Hook-working self-heals to idle on silence (Esc interrupt fires no Stop hook).
        t.hook_state = Some("working".into());
        assert_eq!(compute(&t, t.last_output + ACTIVITY_IDLE_MS + 1), "idle");
    }
}
