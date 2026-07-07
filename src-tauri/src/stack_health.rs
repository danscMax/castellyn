//! Background liveness monitor for the llm-stack. Polls read_stack_health_blocking() on a timer,
//! pushes the full list to the UI, and flags services that transition to down (once per transition).

use std::collections::HashSet;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const POLL_SECS: u64 = 30;

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ServiceDown {
    id: String,
    name: String,
}

/// Ids that went down THIS tick (were not down last tick). Mutates `prev_down` to the current
/// down-set so a still-down service does not re-fire and a recovered one re-arms. Pure + testable.
pub(crate) fn newly_down(prev_down: &mut HashSet<String>, curr: &[(String, bool)]) -> Vec<String> {
    let now_down: HashSet<String> = curr
        .iter()
        .filter(|(_, down)| *down)
        .map(|(id, _)| id.clone())
        .collect();
    // Newly down = down now but not down last tick, in stable input order.
    let fired: Vec<String> = curr
        .iter()
        .filter(|(id, down)| *down && !prev_down.contains(id))
        .map(|(id, _)| id.clone())
        .collect();
    *prev_down = now_down;
    fired
}

/// Start the stack-health poll thread. Called once from `setup()`. Respects the
/// `stackHealthMonitor` config toggle (default on). First poll runs after one interval so startup
/// isn't blocked; the health card already loads once on mount.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut prev_down: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(POLL_SECS));
            if !crate::read_config_file().stack_health_monitor.unwrap_or(true) {
                continue;
            }
            let health = crate::read_stack_health_blocking();
            // Only enabled services count as "outages"; a disabled service being down is expected.
            let curr: Vec<(String, bool)> = health
                .iter()
                .filter(|h| h.enabled)
                .map(|h| (h.id.clone(), !h.port_open || h.healthy == Some(false)))
                .collect();
            let fired = newly_down(&mut prev_down, &curr);
            // Push the full list every tick so the UI updates live without a manual refresh.
            let _ = app.emit("stack-health", &health);
            for id in fired {
                if let Some(h) = health.iter().find(|h| h.id == id) {
                    let _ = app.emit("stack-service-down", ServiceDown { id: h.id.clone(), name: h.name.clone() });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_per_transition_and_rearms() {
        let mut prev: HashSet<String> = HashSet::new();
        let up = |ids: &[(&str, bool)]| ids.iter().map(|(i, d)| (i.to_string(), *d)).collect::<Vec<_>>();

        // First tick: gateway down, qwen up → gateway newly-down.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", false)])), vec!["gateway"]);
        // Still down → does NOT re-fire.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", false)])), Vec::<String>::new());
        // Gateway recovers → nothing fires, but it re-arms.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", false), ("qwen", false)])), Vec::<String>::new());
        // Gateway drops again → fires again (re-armed).
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", false)])), vec!["gateway"]);
        // A second service drops in the same tick → only the newly-down one.
        assert_eq!(newly_down(&mut prev, &up(&[("gateway", true), ("qwen", true)])), vec!["qwen"]);
    }
}
