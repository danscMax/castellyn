//! Anthropic OAuth usage-limit monitor for Sessions (per profile).
//!
//! Every `POLL_SECS` this polls `https://api.anthropic.com/api/oauth/usage` for each profile that
//! has a `.credentials.json` carrying `claudeAiOauth.accessToken`, and surfaces the 5-hour / 7-day
//! utilization. It mirrors the request the user's own statusline.py already makes, so it introduces
//! no new trust surface: the access token is read, sent to Anthropic's OWN API over TLS, and NEVER
//! logged or emitted. A `limits-status` event carries the raw percentages (no token) to the UI, and
//! a `limits-alert` fires once per profile per window when utilization crosses 85% / 99% (the 99%
//! alert also rings + shows an OS toast). A 401 marks the token expired; we do NOT refresh it.
//!
//! `usage_cached` is the ONE network path to the usage endpoint for the whole app: this poller and
//! the per-profile badge (`fetch_profile_usage` in lib.rs) both go through it, keyed by the
//! credentials path they share. Before this, each had its own copy of the request and hit Anthropic
//! on its own cadence — together often enough to earn a real 429. Rate-limiting is now surfaced
//! (`rateLimited`) rather than collapsed into "some transient error", so the UI can say the numbers
//! are stale instead of quietly showing the last ones forever.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const POLL_SECS: u64 = 300; // 5 minutes
const HTTP_TIMEOUT_SECS: u64 = 5; // P5: was 8; a stalled profile shouldn't hold up the round for long
const WARN_PCT: f64 = 85.0;
const CRIT_PCT: f64 = 99.0;
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Matches `POLL_SECS` so the badge's tick and this poller's round, whichever lands first, serve the
/// other from cache instead of making a second identical request for the same profile.
const CACHE_TTL_SECS: u64 = 300;

/// Deduplicates usage requests across every caller, keyed by the profile's `.credentials.json` path
/// (both callers derive the same path). 401/429 and Ok are cached (a 429 must not trigger a retry
/// storm); a transient transport failure is NOT (network recovery should take effect next poll). The
/// stored token invalidates the entry on re-auth. Value = (fetched_at, token_used, result).
#[allow(clippy::type_complexity)]
static USAGE_CACHE: LazyLock<Mutex<HashMap<String, (Instant, String, Result<serde_json::Value, u16>)>>> =
    LazyLock::new(Default::default);

/// The single entry point to the usage endpoint. `None` = this profile has no OAuth token at all
/// (never logged in) — distinct from `Some(Err(401))`, which means Anthropic rejected the token we
/// do have. `Some(Err(429))` = rate-limited; `Some(Err(0))` = transport failure.
pub(crate) fn usage_cached(cred_path: &str) -> Option<Result<serde_json::Value, u16>> {
    let token = read_access_token(cred_path)?;
    {
        let cache = USAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, cached_token, res)) = cache.get(cred_path) {
            // Invalidate on re-auth: a fresh token for the same profile must not keep reading the old
            // (e.g. 401) verdict from the cache until the TTL elapses.
            if *cached_token == token && at.elapsed().as_secs() < CACHE_TTL_SECS {
                return Some(res.clone());
            }
        }
    }
    let res = fetch_usage(&token);
    // Don't cache a transient transport failure (Err(0)) — a network recovery should take effect on
    // the next poll, not 5 minutes later. Ok / 401 / 429 ARE cached.
    if !matches!(res, Err(0)) {
        let mut cache = USAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(cred_path.to_string(), (Instant::now(), token, res.clone()));
    }
    Some(res)
}

/// Raw per-profile utilization pushed to the UI every poll (never includes the token).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LimitsStatus {
    profile: String,
    /// 5-hour / 7-day utilization percentages (None when the field is absent).
    h5: Option<f64>,
    d7: Option<f64>,
    h5_reset: Option<String>,
    d7_reset: Option<String>,
    /// Highest model/surface-SCOPED limit from the response's `limits[]` array (e.g. a per-model
    /// weekly cap). It can exceed `d7` — and then IT is the binding constraint, not the headline
    /// seven_day number, so auto-switch and the title-bar peak must see it.
    scoped: Option<f64>,
    scoped_label: Option<String>,
    scoped_reset: Option<String>,
    /// `extra_usage`: pay-as-you-go credits that keep the profile working past the plan limits.
    extra_enabled: bool,
    extra_pct: Option<f64>,
    /// The OAuth token was rejected (401) — the user must re-auth this profile.
    expired: bool,
    /// Anthropic answered 429: the percentages above are unknown for this round, NOT zero. Kept
    /// separate from `expired` and from a plain network error so the UI can say "rate-limited" and
    /// `pickResumeCandidate` skips the profile rather than switching to it on stale numbers.
    rate_limited: bool,
}

/// Fired only when a window newly crosses a threshold (UI toast + at 99% sound/OS).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LimitsAlert {
    profile: String,
    window: String, // "5h" | "7d"
    level: u8,      // 85 | 99
    utilization: f64,
    resets_at: Option<String>,
}

/// Which (profile, window, level, window-id) alerts have already fired. The window-id is the
/// `resets_at` string, so a NEW window re-arms both thresholds.
static FIRED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Threshold decision + antispam, pure so it is unit-testable. Returns the level to fire (99 or 85)
/// or None to stay quiet. Fires each threshold at most once per window; a changed `reset` (new
/// window) drops the prior window's fired levels for this (profile, window) so it re-arms.
fn take_alert(
    fired: &mut HashSet<String>,
    profile: &str,
    window: &str,
    util: f64,
    reset: Option<&str>,
) -> Option<u8> {
    let level: u8 = if util >= CRIT_PCT {
        99
    } else if util >= WARN_PCT {
        85
    } else {
        return None;
    };
    let win = reset.unwrap_or("-");
    let prefix = format!("{profile}\x1f{window}\x1f");
    let key = format!("{prefix}{level}\x1f{win}");
    if fired.contains(&key) {
        return None;
    }
    // A new window for this (profile, window): drop the prior window's fired levels so it re-arms.
    let suffix = format!("\x1f{win}");
    // Keep keys that aren't a prior window of this (profile, window): either a different group, or
    // already the current window. (De Morgan of `!(starts_with && !ends_with)`.)
    fired.retain(|k| !k.starts_with(&prefix) || k.ends_with(&suffix));
    fired.insert(key);
    Some(level)
}

/// Pull `claudeAiOauth.accessToken` from a profile's `.credentials.json`. None = no OAuth (skip the
/// profile). The token is returned by value and never logged.
fn read_access_token(cred_path: &str) -> Option<String> {
    let text = std::fs::read_to_string(cred_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// GET the usage endpoint with the OAuth token. Ok(json) on 200; Err(status) on an HTTP status
/// (401 = expired token); Err(0) on a transport error. The token is only ever a header value.
fn fetch_usage(token: &str) -> Result<serde_json::Value, u16> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(HTTP_TIMEOUT_SECS)))
        // V-12: this request carries a bearer token — never follow a redirect to another host.
        .max_redirects(0)
        .build()
        .into();
    match agent
        .get(USAGE_URL)
        .header("Accept", "application/json")
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .call()
    {
        Ok(mut resp) => resp
            .body_mut()
            .read_to_string()
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .ok_or(0),
        Err(ureq::Error::StatusCode(code)) => Err(code),
        Err(_) => Err(0),
    }
}

/// `response.<field>.utilization` / `.resets_at` — tolerant of a missing branch. `resets_at` is
/// coerced to a string whether the API sends it as an ISO string or a numeric timestamp, so the
/// antispam window-id (which keys off it) is stable and re-arms per window either way.
pub(crate) fn util_of(resp: &serde_json::Value, field: &str) -> (Option<f64>, Option<String>) {
    let b = resp.get(field);
    let util = b.and_then(|x| x.get("utilization")).and_then(|x| x.as_f64());
    let reset = b.and_then(|x| x.get("resets_at")).and_then(json_scalar_str);
    (util, reset)
}

/// The tightest SCOPED limit from the response's `limits[]` array. Entries with a non-null `scope`
/// are per-model/per-surface caps (live example: weekly_scoped "Fable" at 18% while the headline
/// seven_day said 12%) that `five_hour`/`seven_day` do NOT include. Returns (percent, label,
/// resets_at) of the max-percent scoped entry; label falls back to the entry's `kind`.
pub(crate) fn scoped_max(resp: &serde_json::Value) -> (Option<f64>, Option<String>, Option<String>) {
    let mut best: (Option<f64>, Option<String>, Option<String>) = (None, None, None);
    for l in resp.get("limits").and_then(|x| x.as_array()).into_iter().flatten() {
        let Some(scope) = l.get("scope").filter(|s| !s.is_null()) else {
            continue;
        };
        let Some(pct) = l.get("percent").and_then(|x| x.as_f64()) else {
            continue;
        };
        if best.0.is_some_and(|b| b >= pct) {
            continue;
        }
        let label = scope
            .get("model")
            .and_then(|m| m.get("display_name"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| l.get("kind").and_then(|x| x.as_str()))
            .map(str::to_string);
        best = (Some(pct), label, l.get("resets_at").and_then(json_scalar_str));
    }
    best
}

/// `extra_usage` — pay-as-you-go credits past the plan limits: (is_enabled, utilization %).
pub(crate) fn extra_of(resp: &serde_json::Value) -> (bool, Option<f64>) {
    let e = resp.get("extra_usage");
    (
        e.and_then(|x| x.get("is_enabled")).and_then(|x| x.as_bool()).unwrap_or(false),
        e.and_then(|x| x.get("utilization")).and_then(|x| x.as_f64()),
    )
}

/// A JSON scalar (string OR number) as an owned string; None for null/object/array.
fn json_scalar_str(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// System sound for the 99% alert (MessageBeep respects the user's sound scheme + mute). No-op off
/// Windows. Mirrors agent_status's beep; kept local so the two monitors stay independent.
fn beep_crit() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::MB_ICONEXCLAMATION;
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    }
}

/// Softer warning chime for the 85% threshold (the info/asterisk sound, quieter than the crit
/// exclamation). No OS toast at 85 — sound only, so the user isn't nagged with a popup mid-work.
fn beep_warn() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
        let _ = MessageBeep(MB_ICONASTERISK);
    }
}

/// Emit the alert (UI toast for either level) and, at 99%, ring + OS-notify — each gated by the same
/// config toggles as agent-status. The percentage is rounded for display only.
fn fire_alert(app: &AppHandle, profile: &str, window: &str, level: u8, util: f64, reset: Option<&str>) {
    let _ = app.emit(
        "limits-alert",
        LimitsAlert {
            profile: profile.to_string(),
            window: window.to_string(),
            level,
            utilization: util,
            resets_at: reset.map(str::to_string),
        },
    );
    if (85..99).contains(&level) {
        // 85%: a quiet heads-up chime only, gated on sounds — no toast (see beep_warn).
        if crate::read_config_file().status_sounds.unwrap_or(true) {
            beep_warn();
        }
        return;
    }
    // Unreachable: the 85..99 branch above returned, and take_alert only ever emits 85 or 99, so the
    // only level that reaches here is 99. (Was a dead `if level < 99 { return }`.)
    debug_assert_eq!(level, 99, "fire_alert only receives 85 (handled above) or 99");
    let cfg = crate::read_config_file();
    if cfg.status_sounds.unwrap_or(true) {
        beep_crit();
    }
    if cfg.status_notify.unwrap_or(true) {
        use tauri_plugin_notification::NotificationExt;
        let lang = crate::cur_lang();
        let pct = format!("{}", util.round() as i64);
        let _ = app
            .notification()
            .builder()
            .title(crate::i18n::tr("limits.crit_title", lang))
            .body(crate::i18n::trv(
                "limits.crit_body",
                lang,
                &[("profile", &profile), ("window", &window), ("pct", &pct)],
            ))
            .show();
    }
}

/// Poll one profile once: read its token, fetch usage, emit status, and fire any newly-crossed
/// threshold alerts. Profiles without OAuth creds are skipped (return without emitting).
fn poll_profile(app: &AppHandle, profile: &str, cred_path: &str) {
    let Some(result) = usage_cached(cred_path) else {
        return; // no OAuth on this profile — N/A
    };
    match result {
        Ok(resp) => {
            let (h5, h5_reset) = util_of(&resp, "five_hour");
            let (d7, d7_reset) = util_of(&resp, "seven_day");
            let (scoped, scoped_label, scoped_reset) = scoped_max(&resp);
            let (extra_enabled, extra_pct) = extra_of(&resp);
            let _ = app.emit(
                "limits-status",
                LimitsStatus {
                    profile: profile.to_string(),
                    h5,
                    d7,
                    h5_reset: h5_reset.clone(),
                    d7_reset: d7_reset.clone(),
                    scoped,
                    scoped_label: scoped_label.clone(),
                    scoped_reset: scoped_reset.clone(),
                    extra_enabled,
                    extra_pct,
                    expired: false,
                    rate_limited: false,
                },
            );
            let mut fired = FIRED.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(u) = h5 {
                if let Some(level) = take_alert(&mut fired, profile, "5h", u, h5_reset.as_deref()) {
                    fire_alert(app, profile, "5h", level, u, h5_reset.as_deref());
                }
            }
            if let Some(u) = d7 {
                if let Some(level) = take_alert(&mut fired, profile, "7d", u, d7_reset.as_deref()) {
                    fire_alert(app, profile, "7d", level, u, d7_reset.as_deref());
                }
            }
            // A scoped (per-model) cap alerts too — it gates real work even when the headline 5h/7d
            // are calm. The window id is the model label, so each model re-arms independently.
            if let Some(u) = scoped {
                let win = scoped_label.as_deref().unwrap_or("model");
                if let Some(level) = take_alert(&mut fired, profile, win, u, scoped_reset.as_deref()) {
                    fire_alert(app, profile, win, level, u, scoped_reset.as_deref());
                }
            }
        }
        Err(401) => {
            // Expired token — surface it, do NOT attempt a refresh.
            let _ = app.emit(
                "limits-status",
                LimitsStatus {
                    profile: profile.to_string(),
                    h5: None,
                    d7: None,
                    h5_reset: None,
                    d7_reset: None,
                    scoped: None,
                    scoped_label: None,
                    scoped_reset: None,
                    extra_enabled: false,
                    extra_pct: None,
                    expired: true,
                    rate_limited: false,
                },
            );
        }
        Err(429) => {
            // Rate-limited. Emitting None percentages (rather than staying silent) is deliberate:
            // silence left the UI showing the previous round's numbers as if they were current, and
            // left `pickResumeCandidate` free to switch onto a profile using utilization we can no
            // longer vouch for. Unknown must read as unknown.
            let _ = app.emit(
                "limits-status",
                LimitsStatus {
                    profile: profile.to_string(),
                    h5: None,
                    d7: None,
                    h5_reset: None,
                    d7_reset: None,
                    scoped: None,
                    scoped_label: None,
                    scoped_reset: None,
                    extra_enabled: false,
                    extra_pct: None,
                    expired: false,
                    rate_limited: true,
                },
            );
        }
        Err(_) => { /* transient (network / 5xx) — skip this round, retry next poll */ }
    }
}

/// The bare profile key the whole frontend indexes usage by (`cc1`), derived from a profile
/// DIRECTORY name (`.claude-cc1`). The default profile dir `.claude` has no suffix and is
/// returned unchanged. `limits-status` MUST carry this key — panes, profileInfos, the launch
/// advisor and the resume/auto-switch all key usage by the bare name, never the directory. (H-1)
fn profile_key(dir: &str) -> &str {
    dir.strip_prefix(".claude-").unwrap_or(dir)
}

/// Start the usage-limit poll thread. Called once from `setup()`. Respects the `limitsMonitor`
/// config toggle (default on); a first poll runs after one interval so startup isn't blocked.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(POLL_SECS));
        crate::run_guarded("limits", || {
            if !crate::read_config_file().limits_monitor.unwrap_or(true) {
                return;
            }
            let Ok(home) = std::env::var("USERPROFILE") else {
                return;
            };
            // P5: poll profiles concurrently (bounded to 4 at a time) instead of serially — the round's
            // wall-clock was up to N × HTTP_TIMEOUT_SECS; now it's ~max(per-profile), not the sum.
            let profiles = crate::plugin_sync_profiles(&home);
            for chunk in profiles.chunks(4) {
                std::thread::scope(|s| {
                    for (name, _settings) in chunk {
                        let app_ref = &app;
                        let home_ref = &home;
                        s.spawn(move || {
                            let cred = format!("{home_ref}\\{name}\\.credentials.json");
                            // H-1: emit the BARE profile key the frontend uses (`cc1`), not the
                            // directory name (`.claude-cc1`) — otherwise the launch advisor and the
                            // resume/auto-switch see usage-unknown forever and silently do nothing.
                            poll_profile(app_ref, profile_key(name), &cred);
                        });
                    }
                });
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_key_strips_the_claude_dir_prefix() {
        // The frontend indexes usage by the BARE name; the poller must emit it, not the dir.
        assert_eq!(profile_key(".claude-cc1"), "cc1");
        assert_eq!(profile_key(".claude-work"), "work");
        // Default profile dir has no suffix — returned unchanged.
        assert_eq!(profile_key(".claude"), ".claude");
    }

    #[test]
    fn thresholds_fire_once_per_window_and_rearm() {
        let mut fired = HashSet::new();
        // Below warn → nothing.
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 80.0, Some("R1")), None);
        // Cross 85 → fire once, re-fire suppressed.
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 86.0, Some("R1")), Some(85));
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 90.0, Some("R1")), None);
        // Cross 99 in the SAME window → fires (distinct threshold), then suppressed.
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 99.5, Some("R1")), Some(99));
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 100.0, Some("R1")), None);
        // A different window (new resets_at) re-arms — jumping straight to 99 fires 99.
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 99.9, Some("R2")), Some(99));
        assert_eq!(take_alert(&mut fired, "cc1", "5h", 99.9, Some("R2")), None);
        // Independent (profile, window) tracks separately.
        assert_eq!(take_alert(&mut fired, "cc1", "7d", 88.0, Some("W1")), Some(85));
        assert_eq!(take_alert(&mut fired, "cc2", "5h", 88.0, Some("R2")), Some(85));
    }

    #[test]
    fn util_parsing_is_tolerant() {
        let resp = serde_json::json!({
            "five_hour": { "utilization": 42.5, "resets_at": "2026-07-03T18:00:00Z" },
            "seven_day": { "utilization": 10, "resets_at": 1_751_565_600i64 }, // int util + numeric ts
            "empty": {}
        });
        assert_eq!(util_of(&resp, "five_hour"), (Some(42.5), Some("2026-07-03T18:00:00Z".to_string())));
        // Numeric resets_at is coerced to a string so the antispam window-id stays stable.
        assert_eq!(util_of(&resp, "seven_day"), (Some(10.0), Some("1751565600".to_string())));
        assert_eq!(util_of(&resp, "empty"), (None, None));
        assert_eq!(util_of(&resp, "missing"), (None, None));
    }

    #[test]
    fn scoped_and_extra_parsing() {
        // Shape captured from a live /api/oauth/usage response (2026-07-10): the scoped weekly cap
        // (18%) exceeds the headline seven_day (12%) — exactly the case these parsers exist for.
        let resp = serde_json::json!({
            "seven_day": { "utilization": 12.0, "resets_at": "2026-07-11T05:00:00Z" },
            "limits": [
                { "kind": "session", "group": "session", "percent": 8, "scope": null, "is_active": false },
                { "kind": "weekly_all", "group": "weekly", "percent": 12, "scope": null, "is_active": false },
                { "kind": "weekly_scoped", "group": "weekly", "percent": 18, "is_active": true,
                  "resets_at": "2026-07-11T05:00:00Z",
                  "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } }
            ],
            "extra_usage": { "is_enabled": false, "utilization": null }
        });
        assert_eq!(
            scoped_max(&resp),
            (Some(18.0), Some("Fable".to_string()), Some("2026-07-11T05:00:00Z".to_string()))
        );
        assert_eq!(extra_of(&resp), (false, None));

        // Enabled extra credits; scoped label falls back to `kind` when the model name is absent.
        let resp2 = serde_json::json!({
            "limits": [
                { "kind": "weekly_scoped", "percent": 91.5, "scope": { "model": null } }
            ],
            "extra_usage": { "is_enabled": true, "utilization": 37.5 }
        });
        assert_eq!(scoped_max(&resp2), (Some(91.5), Some("weekly_scoped".to_string()), None));
        assert_eq!(extra_of(&resp2), (true, Some(37.5)));

        // No limits[] / no extra_usage at all — everything reads as absent, nothing panics.
        let empty = serde_json::json!({});
        assert_eq!(scoped_max(&empty), (None, None, None));
        assert_eq!(extra_of(&empty), (false, None));
    }

    #[test]
    fn read_access_token_shape() {
        // A missing/empty token yields None; a present one is returned verbatim (no logging path).
        let dir = std::env::temp_dir().join("castellyn-limits-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("creds.json");
        std::fs::write(&p, r#"{"claudeAiOauth":{"accessToken":"tok-abc"}}"#).unwrap();
        assert_eq!(read_access_token(p.to_str().unwrap()).as_deref(), Some("tok-abc"));
        std::fs::write(&p, r#"{"claudeAiOauth":{"accessToken":""}}"#).unwrap();
        assert_eq!(read_access_token(p.to_str().unwrap()), None);
        std::fs::write(&p, r#"{"other":1}"#).unwrap();
        assert_eq!(read_access_token(p.to_str().unwrap()), None);
        let _ = std::fs::remove_file(&p);
    }
}
