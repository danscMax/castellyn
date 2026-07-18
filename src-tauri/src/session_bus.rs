//! Minimal inter-session message bus (Orca-orchestration lesson, stripped to the useful core).
//!
//! One JSON file (`%APPDATA%\castellyn\messages.json`) holding a bounded list of messages between
//! Sessions panes: `bus_send` appends, `bus_poll` returns a pane's unread mail (direct, `@all`, or
//! `@idle` when the pane says it is idle), `bus_mark_read` consumes. Deliberately NOT a task-DAG,
//! no gates, no coordinator (Orca itself does no AI decomposition — that layer solves fleet
//! problems Castellyn doesn't have). Two separate stamps per message — `delivered_at` (shown in
//! some UI) vs `read_at` (consumed) — the split that fixed re-nagging notifications before
//! ([[limits-notify-persist]] class): a restart must not replay mail that was already shown.
//!
//! ponytail: file-backed with a 500-message cap; move to SQLite only if a real workload outgrows it.

use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};

/// Bounded history: enough for "what did my other sessions tell me", not a log store.
const MAX_MESSAGES: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BusMessage {
    pub id: u64,
    pub from: String,
    /// A session id, `@all`, or `@idle` (delivered only to panes that report themselves idle).
    pub to: String,
    /// Free-form kind (`status` / `ask` / `note` …) — the UI renders it, the bus doesn't interpret.
    pub kind: String,
    pub body: String,
    pub created_at: u64,
    /// Stamped when a poll RETURNED this message to its addressee (it reached a UI).
    pub delivered_at: Option<u64>,
    /// Stamped when the addressee consumed it (badge cleared). Read mail is never re-surfaced.
    pub read_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Default)]
struct BusFile {
    next_id: u64,
    messages: Vec<BusMessage>,
}

/// Serializes all file access in-process (poll storms from several panes must not lose appends).
static BUS_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load(path: &str) -> BusFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

fn store(path: &str, file: &BusFile) -> Result<(), String> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string(file).map_err(|e| e.to_string())?;
    // Atomic tmp+rename so a crash mid-write can't corrupt the whole mailbox.
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Append a message. Returns its id.
pub fn send(path: &str, from: &str, to: &str, kind: &str, body: &str) -> Result<u64, String> {
    let _g = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = load(path);
    f.next_id += 1;
    let id = f.next_id;
    f.messages.push(BusMessage {
        id,
        from: from.to_string(),
        to: to.to_string(),
        kind: kind.to_string(),
        body: body.to_string(),
        created_at: now_ms(),
        delivered_at: None,
        read_at: None,
    });
    // Cap: drop the oldest READ messages first, then plain oldest — unread mail survives longest.
    if f.messages.len() > MAX_MESSAGES {
        let overflow = f.messages.len() - MAX_MESSAGES;
        let mut dropped = 0;
        f.messages.retain(|m| {
            if dropped < overflow && m.read_at.is_some() {
                dropped += 1;
                false
            } else {
                true
            }
        });
        if f.messages.len() > MAX_MESSAGES {
            let excess = f.messages.len() - MAX_MESSAGES;
            f.messages.drain(..excess);
        }
    }
    store(path, &f)?;
    Ok(id)
}

/// A pane's unread mail: direct (`to == session`), broadcast (`@all`), and — when the pane reports
/// itself idle — `@idle`. Marks returned messages `delivered_at` (they reached a UI); they stay
/// unread until `mark_read`, but a restart won't re-notify already-delivered mail (the UI keys
/// "new" off `delivered_at == None`).
pub fn poll(path: &str, session: &str, is_idle: bool) -> Result<Vec<BusMessage>, String> {
    let _g = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = load(path);
    let now = now_ms();
    let mut out = Vec::new();
    for m in f.messages.iter_mut() {
        if m.read_at.is_some() || m.from == session {
            continue;
        }
        let addressed = m.to == session || m.to == "@all" || (m.to == "@idle" && is_idle);
        if addressed {
            if m.delivered_at.is_none() {
                m.delivered_at = Some(now);
            }
            out.push(m.clone());
        }
    }
    if !out.is_empty() {
        store(path, &f)?;
    }
    Ok(out)
}

/// Consume messages (badge cleared / acted upon). Unknown ids are ignored.
pub fn mark_read(path: &str, ids: &[u64]) -> Result<(), String> {
    let _g = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = load(path);
    let now = now_ms();
    let mut changed = false;
    for m in f.messages.iter_mut() {
        if ids.contains(&m.id) && m.read_at.is_none() {
            m.read_at = Some(now);
            changed = true;
        }
    }
    if changed {
        store(path, &f)?;
    }
    Ok(())
}

/// `%APPDATA%\castellyn\messages.json` — sibling of config.json.
fn bus_path() -> Result<String, String> {
    let base = std::env::var("APPDATA").map_err(|e| e.to_string())?;
    Ok(format!("{base}\\castellyn\\messages.json"))
}

#[tauri::command]
pub fn bus_send(from: String, to: String, kind: String, body: String) -> Result<u64, String> {
    send(&bus_path()?, &from, &to, &kind, &body)
}

#[tauri::command]
pub fn bus_poll(session: String, is_idle: bool) -> Result<Vec<BusMessage>, String> {
    poll(&bus_path()?, &session, is_idle)
}

#[tauri::command]
pub fn bus_mark_read(ids: Vec<u64>) -> Result<(), String> {
    mark_read(&bus_path()?, &ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let dir = std::env::temp_dir().join("castellyn-bus-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        let _ = std::fs::remove_file(&p);
        p.to_string_lossy().to_string()
    }

    #[test]
    fn send_poll_mark_read_roundtrip() {
        let p = tmp("roundtrip.json");
        let id = send(&p, "s1", "s2", "note", "hello").unwrap();
        // Addressee sees it; sender and third parties don't.
        let got = poll(&p, "s2", false).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].body, "hello");
        assert!(got[0].delivered_at.is_some());
        assert!(poll(&p, "s1", false).unwrap().is_empty());
        assert!(poll(&p, "s3", false).unwrap().is_empty());
        // Still unread until mark_read; after it — gone from poll.
        assert_eq!(poll(&p, "s2", false).unwrap().len(), 1);
        mark_read(&p, &[id]).unwrap();
        assert!(poll(&p, "s2", false).unwrap().is_empty());
    }

    #[test]
    fn broadcast_and_idle_addressing() {
        let p = tmp("groups.json");
        send(&p, "s1", "@all", "status", "fyi").unwrap();
        send(&p, "s1", "@idle", "task", "pick me up").unwrap();
        // Busy pane: only @all. Idle pane: both. Sender: neither.
        assert_eq!(poll(&p, "s2", false).unwrap().len(), 1);
        assert_eq!(poll(&p, "s3", true).unwrap().len(), 2);
        assert!(poll(&p, "s1", true).unwrap().is_empty());
    }

    #[test]
    fn delivered_stamp_survives_and_cap_prefers_dropping_read() {
        let p = tmp("cap.json");
        let first = send(&p, "a", "b", "note", "oldest-unread").unwrap();
        let read_id = send(&p, "a", "b", "note", "read-one").unwrap();
        mark_read(&p, &[read_id]).unwrap();
        // 2 + 499 = 501 total at the final push: the cap evicts the READ one and every unread
        // message — including the oldest — survives. (With MORE unread than the cap the oldest
        // unread would go too; that's the cap doing its job, not a bug.)
        for i in 0..(MAX_MESSAGES - 1) {
            send(&p, "a", "b", "note", &format!("m{i}")).unwrap();
        }
        let all = poll(&p, "b", false).unwrap();
        // The read message was evicted first; the oldest UNREAD one survived the cap.
        assert!(all.iter().any(|m| m.id == first));
        assert!(all.iter().all(|m| m.body != "read-one"));
        assert!(all.len() <= MAX_MESSAGES);
        // delivered_at persisted: a second poll returns the same stamp, not a fresh one.
        let again = poll(&p, "b", false).unwrap();
        let d1 = all.iter().find(|m| m.id == first).unwrap().delivered_at;
        let d2 = again.iter().find(|m| m.id == first).unwrap().delivered_at;
        assert_eq!(d1, d2);
    }

    #[test]
    fn corrupt_file_resets_instead_of_wedging() {
        let p = tmp("corrupt.json");
        std::fs::write(&p, "{not json").unwrap();
        assert!(poll(&p, "x", false).unwrap().is_empty());
        send(&p, "a", "@all", "note", "alive").unwrap();
        assert_eq!(poll(&p, "b", false).unwrap().len(), 1);
    }
}
