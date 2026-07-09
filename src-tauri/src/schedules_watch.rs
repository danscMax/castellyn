//! Background watcher for scheduled maintenance tasks. Every POLL_SECS it reads
//! schedules.last.json (FILE ONLY — never spawns the pwsh query) and OS-notifies once when a task
//! transitions into a failed state, so a failed nightly job is seen even with the window hidden.

use std::collections::HashSet;
use std::time::Duration;
use tauri::AppHandle;

const POLL_SECS: u64 = 300; // 5 minutes

/// `SCHED_S_TASK_RUNNING` — the task is executing right now. Non-zero, but not a failure.
const TASK_RUNNING: i64 = 0x0004_1301;
/// `SCHED_S_TASK_HAS_NOT_RUN` — the task has never been triggered. Non-zero, but not a failure.
const TASK_HAS_NOT_RUN: i64 = 0x0004_1303;

/// A task's outcome: Some(true)=failed, Some(false)=ok, None=unknown (never ran / no result yet).
/// Prefers an explicit `ok` bool; else derives from `lastResult`.
fn task_failed(task: &serde_json::Value) -> Option<bool> {
    if let Some(ok) = task.get("ok").and_then(|v| v.as_bool()) {
        return Some(!ok);
    }
    match task.get("lastResult") {
        Some(v) if v.is_number() => match v.as_i64() {
            Some(0) => Some(false),
            // Windows reports "running" and "never run" as non-zero HRESULTs. Reading every non-zero
            // value as a failure toasted the user about tasks that were merely in progress or idle.
            Some(TASK_RUNNING) | Some(TASK_HAS_NOT_RUN) => None,
            Some(_) => Some(true),
            None => None,
        },
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
        // The first tick only records what is already failing. Without this baseline, a job that failed
        // days ago re-toasted on every single app launch until it next succeeded — `prev_failed` starts
        // empty, so everything currently failed looks like a fresh transition.
        let mut baseline_taken = false;
        loop {
            std::thread::sleep(Duration::from_secs(POLL_SECS));
            crate::run_guarded("schedules-watch", || {
            let tasks = crate::read_schedules_cached_inner()
                .ok()
                .flatten()
                .and_then(|doc| doc.get("tasks").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default();
            let fired = newly_failed(&mut prev_failed, &tasks);
            if !baseline_taken {
                baseline_taken = true;
                return;
            }
            for name in fired {
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

    #[test]
    fn running_and_never_run_hresults_are_not_failures() {
        // Windows returns these as non-zero `lastResult`; treating them as failures toasted the user
        // about a task that was simply executing, or one that had never been triggered.
        assert_eq!(task_failed(&json!({ "lastResult": 0x0004_1301 })), None, "running");
        assert_eq!(task_failed(&json!({ "lastResult": 0x0004_1303 })), None, "never run");
        // A real failure HRESULT still reads as failed (0x80070002 exceeds i32 — annotate it).
        assert_eq!(task_failed(&json!({ "lastResult": 0x8007_0002i64 })), Some(true));
    }
}
