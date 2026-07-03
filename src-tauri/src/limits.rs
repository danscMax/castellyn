//! Anthropic OAuth usage-limit monitor for Sessions (per profile).
//!
//! Every `POLL_SECS` this polls `https://api.anthropic.com/api/oauth/usage` for each profile that
//! has a `.credentials.json` carrying `claudeAiOauth.accessToken`, and surfaces the 5-hour / 7-day
//! utilization. It mirrors the request the user's own statusline.py already makes, so it introduces
//! no new trust surface: the access token is read, sent to Anthropic's OWN API over TLS, and NEVER
//! logged or emitted. A `limits-status` event carries the raw percentages (no token) to the UI, and
//! a `limits-alert` fires once per profile per window when utilization crosses 85% / 99% (the 99%
//! alert also rings + shows an OS toast). A 401 marks the token expired; we do NOT refresh it.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const POLL_SECS: u64 = 300; // 5 minutes
const HTTP_TIMEOUT_SECS: u64 = 8;
const WARN_PCT: f64 = 85.0;
const CRIT_PCT: f64 = 99.0;
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

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
    /// The OAuth token was rejected (401) — the user must re-auth this profile.
    expired: bool,
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
    fired.retain(|k| !(k.starts_with(&prefix) && !k.ends_with(&suffix)));
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
fn util_of(resp: &serde_json::Value, field: &str) -> (Option<f64>, Option<String>) {
    let b = resp.get(field);
    let util = b.and_then(|x| x.get("utilization")).and_then(|x| x.as_f64());
    let reset = b.and_then(|x| x.get("resets_at")).and_then(json_scalar_str);
    (util, reset)
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
    if level < 99 {
        return;
    }
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
    let Some(token) = read_access_token(cred_path) else {
        return; // no OAuth on this profile — N/A
    };
    match fetch_usage(&token) {
        Ok(resp) => {
            let (h5, h5_reset) = util_of(&resp, "five_hour");
            let (d7, d7_reset) = util_of(&resp, "seven_day");
            let _ = app.emit(
                "limits-status",
                LimitsStatus {
                    profile: profile.to_string(),
                    h5,
                    d7,
                    h5_reset: h5_reset.clone(),
                    d7_reset: d7_reset.clone(),
                    expired: false,
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
                    expired: true,
                },
            );
        }
        Err(_) => { /* transient (network / 5xx) — skip this round, retry next poll */ }
    }
}

/// Start the usage-limit poll thread. Called once from `setup()`. Respects the `limitsMonitor`
/// config toggle (default on); a first poll runs after one interval so startup isn't blocked.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(POLL_SECS));
        if !crate::read_config_file().limits_monitor.unwrap_or(true) {
            continue;
        }
        let Ok(home) = std::env::var("USERPROFILE") else {
            continue;
        };
        for (name, _settings) in crate::plugin_sync_profiles(&home) {
            let cred = format!("{home}\\{name}\\.credentials.json");
            poll_profile(&app, &name, &cred);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
