//! Native `profiles.last.json` — the port of `Get-ProfilesStatus.ps1`.
//!
//! Read-only health snapshot of every profile: existence, credentials, onboarding, and the link
//! health of the shared folders, plus a backup-freshness canary and a Syncthing conflict count.
//!
//! Two reasons this is native rather than a script call:
//!   1. It is the last PowerShell spawn left in the repair path (~3.5s per refresh).
//!   2. A machine with no maintenance scripts at all — every user who is not the author — could
//!      never populate this file, so the whole Profiles surface was permanently empty for them.
//!
//! The emitted shape is byte-for-byte the contract the dashboard already reads (`ProfilesStatus`
//! in `src/lib/ipc.ts`); the link-kind vocabulary lives in `links::link_kind`.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Identity of one profile, from `config/profiles.json`.
pub struct ProfileMeta {
    pub name: String,
    pub description: String,
    pub color: String,
}

/// Everything the snapshot needs, passed in rather than discovered, so the builder is testable
/// against a temporary directory instead of the real machine.
pub struct Inputs {
    pub home: PathBuf,
    pub profiles: Vec<ProfileMeta>,
    pub shared_items: Vec<String>,
    /// The shared FILES a profile should carry (config/shared-items.json `sharedFiles`).
    pub shared_files: Vec<String>,
    pub conflict_roots: Vec<PathBuf>,
    pub backups_dir: PathBuf,
    pub is_admin: bool,
    pub machine_name: String,
    pub generated_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileReport {
    pub name: String,
    pub description: String,
    pub color: String,
    pub exists: bool,
    pub credentials_present: bool,
    pub credentials_valid: bool,
    pub settings_present: bool,
    pub onboarding_complete: bool,
    pub needs_onboarding: bool,
    pub logout_residue: bool,
    /// item -> link type, or null when the item is absent entirely.
    pub shared_links: serde_json::Map<String, serde_json::Value>,
    /// Shared FILES: item -> "link" | "real", or null when the profile does not have it.
    ///
    /// Reported but never acted on for an existing profile: a profile deliberately running without
    /// the global instructions must not acquire them silently. Only files that a profile is
    /// *supposed* to carry appear here — scripts reached by absolute path are excluded, or every
    /// healthy profile would look permanently incomplete.
    pub shared_files: serde_json::Map<String, serde_json::Value>,
    /// Every shared item is a link (structure).
    pub links_intact: bool,
    /// Every link resolves to the right target (health).
    pub links_valid: bool,
    pub link_problems: Vec<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BackupCanary {
    pub last_run: Option<String>,
    pub last_snapshot: Option<String>,
    pub age_hours: Option<f64>,
    pub stale: bool,
    pub snapshot_present: bool,
}

#[derive(Serialize)]
pub struct SyncConflicts {
    pub count: usize,
    pub files: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub generated_at: String,
    pub machine_name: String,
    pub is_admin: bool,
    pub profiles: Vec<ProfileReport>,
    pub backup: BackupCanary,
    pub sync_conflicts: SyncConflicts,
}

/// Onboarding facts read out of a profile's `.claude.json`.
///
/// Claude Code gates its whole startup on `hasCompletedOnboarding`: while it is false the CLI
/// replays the wizard AND demands a fresh login, ignoring a perfectly valid token
/// (anthropics/claude-code#4714). `/logout` clears the flag and leaves the
/// `hasAvailableSubscription`/`subscriptionNoticeCount` pair behind — that pair is the fingerprint
/// of a profile stranded that way.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Onboarding {
    pub oauth_account_present: bool,
    pub complete: bool,
    pub logout_residue: bool,
}

/// Pure: derive the onboarding state from an already-parsed `.claude.json`.
pub fn onboarding_of(doc: &serde_json::Value) -> Onboarding {
    let complete = doc
        .get("hasCompletedOnboarding")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Onboarding {
        oauth_account_present: !matches!(
            doc.get("oauthAccount"),
            None | Some(serde_json::Value::Null)
        ),
        complete,
        logout_residue: !complete
            && (doc.get("hasAvailableSubscription").is_some()
                || doc.get("subscriptionNoticeCount").is_some()),
    }
}

/// Pure: is the stored OAuth token still usable?
///
/// The file existing proves nothing — a wiped profile keeps `.credentials.json` with empty tokens.
/// Claude Code silently refreshes an expired ACCESS token, so a live refresh token is enough.
/// Credential shapes we do not positively understand (setup-token, gateway) fail OPEN: only a blob
/// we can read and can see is dead reports as unusable.
pub fn credentials_usable(doc: &serde_json::Value, now_ms: i64) -> bool {
    let Some(o) = doc.get("claudeAiOauth") else {
        return true; // unknown shape — do not claim it is broken
    };
    if o.is_null() {
        return false;
    }
    let num = |k: &str| o.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let nonempty = |k: &str| o.get(k).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());

    let access_alive = nonempty("accessToken") && num("expiresAt") > now_ms;
    // A refresh token with no expiry stamp is treated as alive — older Claude Code versions omit it.
    let refresh_alive = nonempty("refreshToken")
        && (o.get("refreshTokenExpiresAt").is_none() || num("refreshTokenExpiresAt") > now_ms);
    access_alive || refresh_alive
}

/// Read a JSON file, tolerating the UTF-8 BOM PowerShell writers leave behind.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()
}

/// Assemble one profile's report.
fn report(inputs: &Inputs, meta: &ProfileMeta, now_ms: i64) -> ProfileReport {
    let dir = inputs.home.join(format!(".claude-{}", meta.name));
    let main = inputs.home.join(".claude");
    let exists = dir.is_dir();

    let mut shared_links = serde_json::Map::new();
    let mut problems = Vec::new();
    let mut intact = exists;
    let mut valid = exists;
    if !exists {
        problems.push("profile dir missing".to_string());
    } else {
        for item in &inputs.shared_items {
            let link_path = dir.join(item);
            let kind = crate::links::link_kind(&link_path);
            shared_links.insert(
                item.clone(),
                match kind {
                    Some(k) => serde_json::Value::String(k.to_string()),
                    None => serde_json::Value::Null,
                },
            );
            match kind {
                None => {
                    intact = false;
                    valid = false;
                    problems.push(format!("{item}: missing"));
                }
                Some("none") => {
                    intact = false;
                    valid = false;
                    problems.push(format!("{item}: real item, not a link"));
                }
                Some(_) => {
                    let healthy = matches!(
                        crate::links::inspect(&link_path, &main.join(item)),
                        crate::links::Existing::Link {
                            points_at_target: true
                        }
                    );
                    if !healthy {
                        valid = false;
                        problems.push(format!("{item}: link points at wrong/missing target"));
                    }
                }
            }
        }
    }

    let mut shared_files = serde_json::Map::new();
    if exists {
        for item in &inputs.shared_files {
            if crate::links::provision_of(item) == crate::links::Provision::Skip {
                continue;
            }
            shared_files.insert(
                item.clone(),
                match crate::links::shared_file_state(&dir.join(item), &main.join(item)) {
                    Some(state) => serde_json::Value::from(state),
                    None => serde_json::Value::Null,
                },
            );
        }
    }

    let creds_file = dir.join(".credentials.json");
    let credentials_present = exists && creds_file.is_file();
    let credentials_valid = credentials_present
        && match read_json(&creds_file) {
            Some(doc) => credentials_usable(&doc, now_ms),
            // Present but unparseable — fail OPEN, same as the script did.
            None => true,
        };
    let onb = exists
        .then(|| read_json(&dir.join(".claude.json")))
        .flatten()
        .map(|d| onboarding_of(&d))
        .unwrap_or_default();

    ProfileReport {
        name: meta.name.clone(),
        description: meta.description.clone(),
        color: meta.color.clone(),
        exists,
        credentials_present,
        credentials_valid,
        settings_present: exists && dir.join("settings.json").is_file(),
        onboarding_complete: onb.complete,
        // A profile that never logged in has no oauthAccount — an unset flag is normal there, not
        // damage. Damage = signed in (token AND account on file) yet the flag says otherwise.
        needs_onboarding: credentials_present && onb.oauth_account_present && !onb.complete,
        logout_residue: onb.logout_residue,
        shared_links,
        shared_files,
        links_intact: intact,
        links_valid: valid,
        link_problems: problems,
    }
}

/// Recursively collect Syncthing conflict artifacts under `roots`.
///
/// Iterative rather than recursive so a pathological tree cannot blow the stack, and symlinked
/// directories are not followed — `~/.claude` is full of junctions into the shared tree, and
/// following them would count the same conflict once per profile.
pub fn find_conflicts(roots: &[PathBuf]) -> Vec<String> {
    const MARK: &str = ".sync-conflict-";
    let mut found = Vec::new();
    let mut stack: Vec<PathBuf> = roots.iter().filter(|r| r.is_dir()).cloned().collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(e.path());
            } else if e.file_name().to_string_lossy().contains(MARK) {
                found.push(e.path().display().to_string());
            }
        }
    }
    found.sort();
    found
}

/// Backup freshness canary: surfaces a silently stopped backup instead of discovering it at
/// restore time. A recent timestamp naming a snapshot that is gone (or has no manifest) is not
/// fresh — `snapshot_present` is what distinguishes the two.
fn backup_canary(backups_dir: &Path, now: std::time::SystemTime) -> BackupCanary {
    let mut c = BackupCanary {
        stale: true,
        ..Default::default()
    };
    let Some(state) = read_json(&backups_dir.join(".backup-state.json")) else {
        return c;
    };
    let Some(last_run) = state.get("lastRun").and_then(|v| v.as_str()) else {
        return c;
    };
    c.last_run = Some(last_run.to_string());
    c.last_snapshot = state
        .get("lastSnapshot")
        .and_then(|v| v.as_str())
        .map(String::from);
    if let Some(age) = age_hours(last_run, now) {
        c.age_hours = Some((age * 10.0).round() / 10.0);
        c.stale = age >= 48.0;
    }
    if let Some(snap) = &c.last_snapshot {
        c.snapshot_present = backups_dir.join(snap).join("manifest.json").is_file();
    }
    c
}

/// Hours between an ISO-8601 timestamp and `now`. `None` when the stamp cannot be read, which
/// keeps `stale` at its pessimistic default rather than inventing a freshness.
fn age_hours(iso: &str, now: std::time::SystemTime) -> Option<f64> {
    let epoch_secs = iso_to_epoch_secs(iso)?;
    let now_secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((now_secs - epoch_secs) as f64 / 3600.0)
}

/// Minimal ISO-8601 → epoch seconds for the shape PowerShell's `-Format 'o'` emits
/// (`yyyy-MM-ddTHH:mm:ss.fffffff±HH:mm`).
///
/// The fractional part is ignored — it cannot matter to a 48-hour threshold. The **offset is not**:
/// ignoring it compares a local stamp against a UTC clock and shifts every age by the timezone,
/// which a live comparison against the script caught as a 2.9-hour discrepancy. A stamp with no
/// offset is read as UTC.
fn iso_to_epoch_secs(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    // Offset sits after the date+time (and any fraction): `+HH:mm`, `-HH:mm`, or `Z`.
    // `.get()`, not `[19..]`: a byte index that lands mid-character would PANIC on a hand-edited or
    // corrupted stamp, breaking this function's "None when unreadable" contract.
    let tail = iso.get(19..)?;
    let offset_secs = match tail
        .char_indices()
        .find(|(_, c)| matches!(c, '+' | '-' | 'Z' | 'z'))
    {
        None | Some((_, 'Z')) | Some((_, 'z')) => 0,
        Some((i, sign)) => {
            let rest = tail.get(i + 1..)?;
            let h: i64 = rest.get(0..2)?.parse().ok()?;
            let m: i64 = rest.get(3..5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let magnitude = h * 3600 + m * 60;
            if sign == '-' { -magnitude } else { magnitude }
        }
    };
    let n = |a: usize, z: usize| iso.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, s) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // Days since 1970-01-01 via the civil-from-days algorithm (Howard Hinnant), which is exact for
    // every proleptic Gregorian date and needs no date library.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    // Subtract the offset: 12:00+03:00 is 09:00 UTC.
    Some(days * 86_400 + h * 3600 + mi * 60 + s - offset_secs)
}

/// Local time as the ISO-8601 shape PowerShell's `-Format 'o'` produced, so the dashboard's
/// existing rendering of `generatedAt` keeps working unchanged.
#[cfg(windows)]
pub fn now_iso() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}0000",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(not(windows))]
pub fn now_iso() -> String {
    String::new()
}

/// Build the whole snapshot.
pub fn build(inputs: &Inputs) -> Snapshot {
    let now = std::time::SystemTime::now();
    let now_ms = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let conflicts = find_conflicts(&inputs.conflict_roots);
    Snapshot {
        generated_at: inputs.generated_at.clone(),
        machine_name: inputs.machine_name.clone(),
        is_admin: inputs.is_admin,
        profiles: inputs
            .profiles
            .iter()
            .map(|m| report(inputs, m, now_ms))
            .collect(),
        backup: backup_canary(&inputs.backups_dir, now),
        sync_conflicts: SyncConflicts {
            count: conflicts.len(),
            files: conflicts.into_iter().take(20).collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn credentials_fail_open_on_shapes_we_do_not_understand() {
        let now = 1_000_000;
        // A gateway/setup-token blob we cannot read must NOT be called dead.
        assert!(credentials_usable(&json!({"setupToken": "x"}), now));
        // Explicit null OAuth = positively dead.
        assert!(!credentials_usable(&json!({ "claudeAiOauth": null }), now));
        // Empty tokens = dead (the wiped-profile case the file's presence hides).
        assert!(!credentials_usable(
            &json!({"claudeAiOauth": {"accessToken": "", "refreshToken": ""}}),
            now
        ));
        // Live access token.
        assert!(credentials_usable(
            &json!({"claudeAiOauth": {"accessToken": "a", "expiresAt": now + 1}}),
            now
        ));
        // Expired access token but a live refresh token — Claude Code refreshes silently.
        assert!(credentials_usable(
            &json!({"claudeAiOauth": {"accessToken": "a", "expiresAt": now - 1, "refreshToken": "r"}}),
            now
        ));
        // Both expired = dead.
        assert!(!credentials_usable(
            &json!({"claudeAiOauth": {"accessToken":"a","expiresAt":now-1,"refreshToken":"r","refreshTokenExpiresAt":now-1}}),
            now
        ));
    }

    #[test]
    fn onboarding_spots_the_logout_fingerprint() {
        // Signed in and finished — healthy.
        let ok = onboarding_of(&json!({"oauthAccount": {"x": 1}, "hasCompletedOnboarding": true}));
        assert!(ok.complete && ok.oauth_account_present && !ok.logout_residue);

        // The /logout fingerprint: flag cleared, subscription keys left behind.
        let stranded = onboarding_of(
            &json!({"oauthAccount": {"x": 1}, "hasCompletedOnboarding": false, "subscriptionNoticeCount": 3}),
        );
        assert!(stranded.logout_residue, "must recognise the /logout residue");

        // A never-used profile: no account, no flag — normal, NOT residue.
        let fresh = onboarding_of(&json!({}));
        assert!(!fresh.oauth_account_present && !fresh.complete && !fresh.logout_residue);
    }

    #[test]
    fn iso_timestamps_convert_against_known_epochs() {
        assert_eq!(iso_to_epoch_secs("1970-01-01T00:00:00"), Some(0));
        assert_eq!(iso_to_epoch_secs("2000-01-01T00:00:00"), Some(946_684_800));
        // PowerShell's -Format 'o' shape. The fraction is ignored; the OFFSET is not — 12:00+03:00
        // is 09:00 UTC. Getting this wrong shifted every backup age by the timezone, which only the
        // live comparison against the script exposed.
        assert_eq!(
            iso_to_epoch_secs("2026-07-20T12:00:00.1234567+03:00"),
            Some(1_784_538_000)
        );
        // Same instant, expressed three ways — all must agree.
        assert_eq!(
            iso_to_epoch_secs("2026-07-20T09:00:00Z"),
            iso_to_epoch_secs("2026-07-20T12:00:00.1234567+03:00")
        );
        assert_eq!(
            iso_to_epoch_secs("2026-07-20T04:00:00-05:00"),
            iso_to_epoch_secs("2026-07-20T09:00:00Z")
        );
        // A negative offset must ADD, not subtract — the sign bug this pins down.
        assert_eq!(
            iso_to_epoch_secs("2026-07-20T09:00:00-05:00").unwrap()
                - iso_to_epoch_secs("2026-07-20T09:00:00Z").unwrap(),
            5 * 3600
        );
        assert_eq!(iso_to_epoch_secs("garbage"), None);
        // Must RETURN None, not panic: byte 19 lands inside a multi-byte character here, and a
        // raw `iso[19..]` slice would abort the whole status refresh on a corrupted state file.
        assert_eq!(iso_to_epoch_secs("2026-07-20T12:00:0ё+03:00"), None);
        // Trailing junk AFTER a complete, valid timestamp is tolerated, not rejected — byte 19 is
        // a clean boundary here, so this must parse rather than blow up or return None.
        assert_eq!(
            iso_to_epoch_secs("2026-07-20T12:00:00ё"),
            iso_to_epoch_secs("2026-07-20T12:00:00")
        );
        // Dates before the epoch stay negative rather than wrapping.
        assert!(iso_to_epoch_secs("1969-12-31T00:00:00").unwrap() < 0);
        // Leap day — the civil-from-days algorithm must not slip a day.
        assert_eq!(
            iso_to_epoch_secs("2024-02-29T00:00:00").unwrap()
                - iso_to_epoch_secs("2024-02-28T00:00:00").unwrap(),
            86_400
        );
    }

    #[test]
    fn a_snapshot_of_a_real_directory_matches_the_dashboard_contract() {
        let root =
            std::env::temp_dir().join(format!("castellyn-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let dir = home.join(".claude-cc1");
        std::fs::create_dir_all(dir.join("skills")).unwrap(); // a REAL folder, not a link
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"a":1},"hasCompletedOnboarding":false,"subscriptionNoticeCount":1}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r"}}"#,
        )
        .unwrap();
        // A conflict artifact the scan must find, and a nested one to prove recursion.
        std::fs::create_dir_all(home.join(".claude").join("deep")).unwrap();
        std::fs::write(
            home.join(".claude").join("deep").join("x.sync-conflict-20260101-000000-ABCDEFG.json"),
            "{}",
        )
        .unwrap();

        let inputs = Inputs {
            home: home.clone(),
            profiles: vec![ProfileMeta {
                name: "cc1".into(),
                description: "d".into(),
                color: "#fff".into(),
            }],
            shared_items: vec!["skills".into(), "agents".into()],
            shared_files: vec!["CLAUDE.md".into(), "statusline.py".into()],
            conflict_roots: vec![home.join(".claude")],
            backups_dir: root.join("Backups"),
            is_admin: false,
            machine_name: "TEST".into(),
            generated_at: "2026-07-20T00:00:00".into(),
        };
        let snap = build(&inputs);
        let v = serde_json::to_value(&snap).unwrap();

        // Key names are a cross-boundary contract with src/lib/ipc.ts — camelCase, not snake.
        let p = &v["profiles"][0];
        assert_eq!(p["name"], "cc1");
        assert_eq!(p["settingsPresent"], true);
        assert_eq!(p["credentialsPresent"], true);
        assert_eq!(p["credentialsValid"], true, "no expiry stamp = treat as alive");
        assert_eq!(p["needsOnboarding"], true, "token + account but flag false");
        assert_eq!(p["logoutResidue"], true);
        // skills exists as a REAL dir -> "none"; agents is absent entirely -> null.
        assert_eq!(p["sharedLinks"]["skills"], "none");
        assert_eq!(p["sharedLinks"]["agents"], serde_json::Value::Null);
        assert_eq!(p["linksIntact"], false);
        assert_eq!(p["linkProblems"].as_array().unwrap().len(), 2);
        // Shared FILES are reported, never acted on: CLAUDE.md is absent here -> null. A script
        // reached by absolute path must NOT appear at all, or every healthy profile would look
        // permanently incomplete.
        assert_eq!(p["sharedFiles"]["CLAUDE.md"], serde_json::Value::Null);
        assert!(
            p["sharedFiles"].get("statusline.py").is_none(),
            "scripts are reached by absolute path and must be excluded from the gap report"
        );
        assert_eq!(v["syncConflicts"]["count"], 1, "must recurse into subdirs");
        // No backup state at all -> stale, which is the safe default.
        assert_eq!(v["backup"]["stale"], true);
        assert_eq!(v["backup"]["snapshotPresent"], false);

        let _ = std::fs::remove_dir_all(&root);
    }
}
