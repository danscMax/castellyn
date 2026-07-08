//! Background liveness monitor for the llm-stack. Polls read_stack_health_blocking() on a timer,
//! pushes the full list to the UI, and flags services that transition to down (once per transition).

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const POLL_SECS: u64 = 30;
/// How long after WE stop a service its down-transition stays suppressed (matches the contract).
const EXPECTED_TTL: Duration = Duration::from_secs(180);

/// Count of enabled services down as of the last poll — read by the tray tooltip.
static STACK_DOWN_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Down-service count from the most recent poll (0 before the first tick).
pub(crate) fn down_count() -> usize {
    STACK_DOWN_COUNT.load(Ordering::Relaxed)
}

/// Whether a newly-down service should be suppressed: it was marked expected-down within the TTL.
/// Pure so the suppression window is unit-testable without threads or a live poll.
fn suppressed(marked: Option<Instant>, now: Instant, ttl: Duration) -> bool {
    marked.map(|t| now.duration_since(t) < ttl).unwrap_or(false)
}

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
        // P8: last-emitted snapshot + ticks since the last emit, so we push only on a real change
        // (plus a keep-alive) instead of a full payload every 30s.
        let mut prev_emit: Option<String> = None;
        let mut ticks_since_emit: u32 = 0;
        // ~5 min keep-alive so a card that treats "no events" as "monitor dead" stays reassured.
        const KEEPALIVE_TICKS: u32 = 10;
        loop {
            std::thread::sleep(Duration::from_secs(POLL_SECS));
            crate::run_guarded("stack-health", || {
            if !crate::read_config_file().stack_health_monitor.unwrap_or(true) {
                return;
            }
            let health = crate::read_stack_health_blocking();
            // Only enabled services count as "outages"; a disabled service being down is expected.
            let curr: Vec<(String, bool)> = health
                .iter()
                .filter(|h| h.enabled)
                .map(|h| (h.id.clone(), !h.port_open || h.healthy == Some(false)))
                .collect();
            let fired = newly_down(&mut prev_down, &curr);
            STACK_DOWN_COUNT.store(curr.iter().filter(|(_, down)| *down).count(), Ordering::Relaxed);
            // P8: push the list only when it changed since the last emit, plus a periodic keep-alive.
            let snap = serde_json::to_string(&health).unwrap_or_default();
            ticks_since_emit += 1;
            if prev_emit.as_deref() != Some(snap.as_str()) || ticks_since_emit >= KEEPALIVE_TICKS {
                let _ = app.emit("stack-health", &health);
                prev_emit = Some(snap);
                ticks_since_emit = 0;
            }
            let now = Instant::now();
            for id in fired {
                // A service WE just stopped (within the TTL) is expected down — stay silent (it's
                // still in the stack-health list emit above). An unexpected death still alerts.
                if suppressed(crate::expected_down_at(&id), now, EXPECTED_TTL) {
                    continue;
                }
                if let Some(h) = health.iter().find(|h| h.id == id) {
                    let _ = app.emit("stack-service-down", ServiceDown { id: h.id.clone(), name: h.name.clone() });
                    crate::notify_important(
                        &app,
                        crate::i18n::tr("notify.stack_down_title", crate::cur_lang()),
                        &crate::i18n::trv("notify.stack_down_body", crate::cur_lang(), &[("name", &h.name)]),
                    );
                }
            }
            crate::update_tray_tooltip(&app);
            });
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

    #[test]
    fn suppressed_only_within_ttl() {
        let now = Instant::now();
        let ttl = Duration::from_secs(180);
        // Never marked → always alerts.
        assert!(!suppressed(None, now, ttl));
        // Marked just now → suppressed.
        assert!(suppressed(Some(now), now, ttl));
        // Marked 100s ago → still inside the 180s window → suppressed.
        assert!(suppressed(now.checked_sub(Duration::from_secs(100)), now, ttl));
        // Marked 200s ago → stale (unexpected death) → alerts.
        assert!(!suppressed(now.checked_sub(Duration::from_secs(200)), now, ttl));
    }
}
