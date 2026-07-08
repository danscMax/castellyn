//! Background watcher for scheduled maintenance tasks. Every POLL_SECS it reads
//! schedules.last.json (FILE ONLY — never spawns the pwsh query) and OS-notifies once when a task
//! transitions into a failed state, so a failed nightly job is seen even with the window hidden.

use std::collections::HashSet;
use std::time::Duration;
use tauri::AppHandle;

const POLL_SECS: u64 = 300; // 5 minutes

/// A task's outcome: Some(true)=failed, Some(false)=ok, None=unknown (never ran / no result yet).
/// Prefers an explicit `ok` bool; else derives from `lastResult` (0 = ok, non-zero = failed).
fn task_failed(task: &serde_json::Value) -> Option<bool> {
    if let Some(ok) = task.get("ok").and_then(|v| v.as_bool()) {
        return Some(!ok);
    }
    match task.get("lastResult") {
        Some(v) if v.is_number() => v.as_i64().map(|c| c != 0),
        _ => None,
    }
}

/// Display name for the toast + the transition key: `label` if present, else `id`.
fn task_name(task: &serde_json::Value) -> Option<String> {
    task.get("label")
        .and_then(|v| v.as_str())
        .or_else(|| task.get("id").and_then(|v| v.as_str()))
        .map(str::to_string)
}

/// Task names newly failed this tick, mutating `prev_failed` to the current failed-set (fires once
/// per transition, re-arms on recovery). Pure + testable — reuses stack_health's newly-down logic.
pub(crate) fn newly_failed(prev_failed: &mut HashSet<String>, tasks: &[serde_json::Value]) -> Vec<String> {
    let curr: Vec<(String, bool)> = tasks
        .iter()
        .filter_map(|t| task_name(t).map(|n| (n, task_failed(t) == Some(true))))
        .collect();
    crate::stack_health::newly_down(prev_failed, &curr)
}

/// Start the schedules-watch poll thread. Called once from `setup()`. Gated on `status_notify`
/// (no new config key). Reads the last-written JSON only — the ScheduleTab already refreshes it.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut prev_failed: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(POLL_SECS));
            crate::run_guarded("schedules-watch", || {
            let tasks = crate::read_schedules_cached_inner()
                .ok()
                .flatten()
                .and_then(|doc| doc.get("tasks").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default();
            for name in newly_failed(&mut prev_failed, &tasks) {
                crate::notify_important(
                    &app,
                    crate::i18n::tr("notify.schedule_failed_title", crate::cur_lang()),
                    &crate::i18n::trv("notify.schedule_failed_body", crate::cur_lang(), &[("name", &name)]),
                );
            }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fires_once_on_transition_to_failed() {
        let mut prev: HashSet<String> = HashSet::new();
        // ok task + a never-run task → nothing fires (unknown is not a failure).
        let t = |id: &str, ok: Option<bool>| match ok {
            Some(b) => json!({ "id": id, "label": id, "ok": b }),
            None => json!({ "id": id, "label": id }),
        };
        assert_eq!(newly_failed(&mut prev, &[t("backup", Some(true)), t("sync", None)]), Vec::<String>::new());
        // backup fails → fires once.
        assert_eq!(newly_failed(&mut prev, &[t("backup", Some(false)), t("sync", None)]), vec!["backup"]);
        // still failed → no re-fire.
        assert_eq!(newly_failed(&mut prev, &[t("backup", Some(false)), t("sync", None)]), Vec::<String>::new());
        // recovers, then fails again → re-arms and fires.
        assert_eq!(newly_failed(&mut prev, &[t("backup", Some(true))]), Vec::<String>::new());
        assert_eq!(newly_failed(&mut prev, &[t("backup", Some(false))]), vec!["backup"]);
    }

    #[test]
    fn failed_derives_from_last_result_when_no_ok() {
        assert_eq!(task_failed(&json!({ "lastResult": 0 })), Some(false));
        assert_eq!(task_failed(&json!({ "lastResult": 1 })), Some(true));
        assert_eq!(task_failed(&json!({})), None);
        // Explicit ok wins over lastResult.
        assert_eq!(task_failed(&json!({ "ok": true, "lastResult": 5 })), Some(false));
    }
}
