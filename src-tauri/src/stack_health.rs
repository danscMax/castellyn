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

/// Ids that have been down for TWO consecutive ticks and are not yet alarmed for this down-episode —
/// the hysteresis that stops a service whose health probe FLAPS (one slow/timed-out poll while the
/// service is actually fine) from firing a false "down" notification. A real outage persists across
/// polls and still alarms, one tick (~POLL_SECS) later. `prev_down` = last tick's down-set; `alarmed`
/// remembers which are already alarmed so a still-down service doesn't re-fire and a recovered one
/// re-arms. Replaces `newly_down` for the notification path (which fired on the FIRST down poll, so a
/// single flaky probe was enough). Pure + testable.
pub(crate) fn confirmed_down(
    prev_down: &mut HashSet<String>,
    alarmed: &mut HashSet<String>,
    curr: &[(String, bool)],
) -> Vec<String> {
    let now_down: HashSet<String> = curr
        .iter()
        .filter(|(_, down)| *down)
        .map(|(id, _)| id.clone())
        .collect();
    // Confirmed = down now AND down last tick AND not already alarmed, in stable input order.
    let fired: Vec<String> = curr
        .iter()
        .filter(|(id, down)| *down && prev_down.contains(id) && !alarmed.contains(id))
        .map(|(id, _)| id.clone())
        .collect();
    for id in &fired {
        alarmed.insert(id.clone());
    }
    // A service no longer down re-arms, so a later genuine outage alarms again.
    alarmed.retain(|id| now_down.contains(id));
    *prev_down = now_down;
    fired
}

/// Of the ids that went down this tick, only those we ever observed LISTENING this session deserve an
/// alarm. A service the user never started is not an outage — it is simply off, and firing "service
/// down" for it on the first poll (where `prev_down` is empty, so everything looks newly-down) is the
/// false alarm this filter removes. Pure + testable.
pub(crate) fn alarmable(fired: &[String], seen_up: &HashSet<String>) -> Vec<String> {
    fired
        .iter()
        .filter(|id| seen_up.contains(*id))
        .cloned()
        .collect()
}

/// Ids currently listening — the session's "we saw it alive" evidence, accumulated across ticks.
fn note_seen_up(seen_up: &mut HashSet<String>, curr: &[(String, bool)]) {
    for (id, down) in curr {
        if !*down {
            seen_up.insert(id.clone());
        }
    }
}

/// Start the stack-health poll thread. Called once from `setup()`. Respects the
/// `stackHealthMonitor` config toggle (default on). First poll runs after one interval so startup
/// isn't blocked; the health card already loads once on mount.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut prev_down: HashSet<String> = HashSet::new();
        // Services observed listening at some point THIS session. A service we never saw up was never
        // started, so its being down is not an outage — see `alarmable`.
        let mut seen_up: HashSet<String> = HashSet::new();
        // Services already alarmed for their current down-episode — the hysteresis state, so a
        // confirmed-down alarm fires once and re-arms only after the service recovers.
        let mut alarmed: HashSet<String> = HashSet::new();
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
            // No stack.json, or every service disabled → nothing to probe. Skips ~9 TCP connects and
            // as many thread spawns per tick for anyone who does not use the llm-stack.
            if !crate::any_stack_service_enabled() {
                STACK_DOWN_COUNT.store(0, Ordering::Relaxed);
                return;
            }
            let health = crate::read_stack_health_blocking();
            // Only enabled services count as "outages"; a disabled service being down is expected.
            let curr: Vec<(String, bool)> = health
                .iter()
                .filter(|h| h.enabled)
                // Every ENABLED service alarms/counts on an outage, deliberately IGNORING the `critical`
                // flag: it defaults to false in stack.json, so gating on it would silence outages for
                // every service the user didn't explicitly mark critical. `critical` is surfaced to the
                // UI (StackHealth is serialized) for display only. A disabled service being down is
                // expected and never counts.
                .map(|h| (h.id.clone(), !h.port_open || h.healthy == Some(false)))
                .collect();
            note_seen_up(&mut seen_up, &curr);
            // Hysteresis: alarm only when a service has been down for TWO consecutive polls — a single
            // flaky/slow health probe (the OmniRoute false-down that spammed) never reaches it.
            let fired = alarmable(&confirmed_down(&mut prev_down, &mut alarmed, &curr), &seen_up);
            // The tray's "needs attention" count must mean a real outage too: a stack the user never
            // started would otherwise report every one of its services as a problem.
            STACK_DOWN_COUNT.store(
                curr.iter()
                    .filter(|(id, down)| *down && seen_up.contains(id))
                    .count(),
                Ordering::Relaxed,
            );
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
    fn confirmed_down_needs_two_consecutive_ticks_and_ignores_a_flap() {
        let up = |ids: &[(&str, bool)]| ids.iter().map(|(i, d)| (i.to_string(), *d)).collect::<Vec<_>>();
        let mut prev: HashSet<String> = HashSet::new();
        let mut alarmed: HashSet<String> = HashSet::new();
        // One down poll: pending, NOT alarmed (it might be a single flaky/slow probe).
        assert_eq!(confirmed_down(&mut prev, &mut alarmed, &up(&[("gw", true)])), Vec::<String>::new());
        // Two consecutive down → confirmed → alarms once.
        assert_eq!(confirmed_down(&mut prev, &mut alarmed, &up(&[("gw", true)])), vec!["gw"]);
        // Still down → does NOT re-fire.
        assert_eq!(confirmed_down(&mut prev, &mut alarmed, &up(&[("gw", true)])), Vec::<String>::new());
        // Recovers → re-arms silently.
        assert_eq!(confirmed_down(&mut prev, &mut alarmed, &up(&[("gw", false)])), Vec::<String>::new());
        // Down once → pending; down again → alarms (re-armed).
        assert_eq!(confirmed_down(&mut prev, &mut alarmed, &up(&[("gw", true)])), Vec::<String>::new());
        assert_eq!(confirmed_down(&mut prev, &mut alarmed, &up(&[("gw", true)])), vec!["gw"]);

        // THE BUG: a probe that FLAPS down/up/down/up (OmniRoute's slow /v1/models timing out on some
        // polls) never reaches two consecutive downs → never alarms. This is the spam fix.
        let mut p2: HashSet<String> = HashSet::new();
        let mut a2: HashSet<String> = HashSet::new();
        for down in [true, false, true, false, true, false] {
            assert_eq!(
                confirmed_down(&mut p2, &mut a2, &up(&[("gw", down)])),
                Vec::<String>::new(),
                "a flapping probe must never alarm"
            );
        }
    }

    #[test]
    fn never_started_service_is_not_an_outage_but_a_crashed_one_is() {
        let tick = |ids: &[(&str, bool)]| ids.iter().map(|(i, d)| (i.to_string(), *d)).collect::<Vec<_>>();
        let mut prev: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();

        // First poll of a stack nobody started: everything looks "newly down" because the baseline is
        // empty. Nothing was ever seen listening, so nothing alarms. This is the owner-reported bug.
        let t1 = tick(&[("gateway", true), ("qwen", true)]);
        note_seen_up(&mut seen, &t1);
        assert_eq!(alarmable(&newly_down(&mut prev, &t1), &seen), Vec::<String>::new());

        // The user starts the gateway; it comes up. Still silent.
        let t2 = tick(&[("gateway", false), ("qwen", true)]);
        note_seen_up(&mut seen, &t2);
        assert_eq!(alarmable(&newly_down(&mut prev, &t2), &seen), Vec::<String>::new());

        // The gateway dies. We saw it alive, so this IS an outage and must alarm.
        let t3 = tick(&[("gateway", true), ("qwen", true)]);
        note_seen_up(&mut seen, &t3);
        assert_eq!(alarmable(&newly_down(&mut prev, &t3), &seen), vec!["gateway"]);

        // qwen was never started this session — it stays silent even now.
        assert!(!seen.contains("qwen"), "a service never observed listening is not 'seen up'");
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
