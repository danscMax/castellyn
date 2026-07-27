//! ===================== Backups tab (snapshots, weekly archives, restore) =====================
//! Snapshots and weekly zips live under `{{PROFILES}}\Backups`, written by the PowerShell
//! Backup-/Restore-ClaudeSetup scripts. Listing/verifying/extracting an archive is native here
//! (bsdtar), while creating a backup or restoring one streams the script through `spawn_streamed`.

use std::os::windows::process::CommandExt;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::i18n::{tr, trv};
use crate::{abs, cur_lang, parse_json_bom, spawn_streamed, stream_id, RunState, CREATE_NO_WINDOW};

pub(crate) const BACKUP_DIR_REL: &str = "{{PROFILES}}\\Backups";
pub(crate) const BACKUP_SCRIPT_REL: &str = "{{PROFILES}}\\Backup-ClaudeSetup.ps1";
const RESTORE_SCRIPT_REL: &str = "{{PROFILES}}\\Restore-ClaudeSetup.ps1";

#[derive(Serialize)]
pub struct BackupList {
    snapshots: Vec<String>,
    weeklies: Vec<String>,
    state: Option<serde_json::Value>,
}

/// Snapshot dir name format: yyyy-MM-dd_HHmmss.
fn is_snapshot_name(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 17 {
        return false;
    }
    let d = |i: usize| b[i].is_ascii_digit();
    d(0) && d(1)
        && d(2)
        && d(3)
        && b[4] == b'-'
        && d(5)
        && d(6)
        && b[7] == b'-'
        && d(8)
        && d(9)
        && b[10] == b'_'
        && d(11)
        && d(12)
        && d(13)
        && d(14)
        && d(15)
        && d(16)
}

/// List backup snapshots + weekly archives and parse .backup-state.json — one call so the
/// frontend never needs the absolute Backups path.
#[tauri::command]
pub fn list_backups() -> BackupList {
    let dir = abs(BACKUP_DIR_REL);
    let mut snapshots: Vec<String> = Vec::new();
    let mut weeklies: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && is_snapshot_name(&name) {
                snapshots.push(name);
            } else if !is_dir && name.starts_with("weekly-") && name.ends_with(".zip") {
                weeklies.push(name);
            }
        }
    }
    snapshots.sort();
    snapshots.reverse();
    weeklies.sort();
    weeklies.reverse();
    let state = std::fs::read_to_string(format!("{dir}\\.backup-state.json"))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok());
    BackupList {
        snapshots,
        weeklies,
        state,
    }
}

/// Validate a weekly-archive name (no path separators, `weekly-*.zip` shape) and return its absolute
/// path under Backups. Shared by reveal/verify/extract/delete so the guard can't drift between them.
fn weekly_archive_path(name: &str) -> Result<String, String> {
    if name.contains('/')
        || name.contains('\\')
        || !(name.starts_with("weekly-") && name.ends_with(".zip"))
    {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &name)]));
    }
    let path = format!("{}\\{}", abs(BACKUP_DIR_REL), name);
    if !std::path::Path::new(&path).exists() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &path)]));
    }
    Ok(path)
}

/// The SYSTEM bsdtar — a bare `tar` may resolve to Git-Bash GNU tar, which treats the `E:` in the
/// archive path as a remote host ("Cannot connect to E:"). Same reason the Backup script uses it.
fn system_tar() -> String {
    let windir = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{windir}\\System32\\tar.exe")
}

/// F9: reveal a weekly archive in Explorer (file selected).
#[tauri::command]
pub fn reveal_backup(name: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| trv("err.open_path", cur_lang(), &[("path", &path), ("e", &e)]))?;
    Ok(())
}

/// F9: delete a weekly archive (zip). The FE gates this behind a confirm.
#[tauri::command]
pub fn delete_backup(name: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    std::fs::remove_file(&path)
        .map_err(|e| trv("err.delete_failed", cur_lang(), &[("path", &path), ("e", &e)]))
}

/// `tar -tf`: entry count on success, tar's stderr when the zip is corrupt/truncated. Shared by
/// verify_backup and import_backup_zip.
fn tar_list(path: &str) -> Result<usize, String> {
    let out = std::process::Command::new(system_tar())
        .args(["-tf", path])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.tar_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).lines().count())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn tar_extract(path: &str, dest: &str) -> Result<(), String> {
    // `-k` (keep-old-files): never overwrite an existing file in the user-picked dest — these
    // extract flows are documented NON-destructive, so a zip entry that collides with existing
    // content must fail loudly, not silently clobber it. Windows bsdtar (libarchive) already strips
    // absolute paths and refuses `..` traversal by default (only `-P` would re-enable them), so the
    // remaining exposure was exactly this overwrite (Codex MEDIUM-07).
    let out = std::process::Command::new(system_tar())
        .args(["-x", "-k", "-f", path, "-C", dest])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| trv("err.tar_failed", cur_lang(), &[("e", &e)]))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// F9: verify a weekly archive by listing it (`tar -tf`). Returns the entry count on success, or the
/// tar stderr if the zip is corrupt/truncated.
/// `(async)`: a sync `#[tauri::command]` runs on the main/event-loop thread, so waiting on tar there
/// freezes the window for the whole archive. The flag moves the body to Tauri's blocking pool.
#[tauri::command(async)]
pub fn verify_backup(name: String) -> Result<usize, String> {
    tar_list(&weekly_archive_path(&name)?)
}

/// F9: extract a weekly archive to a user-picked folder. NON-destructive — never writes over the live
/// ~/.claude (the weekly archives skills/agents/commands, which are Syncthing-synced + junctioned).
/// `(async)`: blocks on tar — see verify_backup.
#[tauri::command(async)]
pub fn extract_backup(name: String, dest: String) -> Result<(), String> {
    let path = weekly_archive_path(&name)?;
    if !std::path::Path::new(&dest).is_dir() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &dest)]));
    }
    tar_extract(&path, &dest)
}

/// Import a backup zip from an ARBITRARY path (USB stick, another machine's export): verify first
/// so a corrupt zip fails before anything lands in `dest`, then extract. Non-destructive like
/// extract_backup — the user picks an explicit destination, the live ~/.claude is never a target.
/// `(async)`: blocks on tar TWICE (list, then extract) — see verify_backup.
#[tauri::command(async)]
pub fn import_backup_zip(path: String, dest: String) -> Result<usize, String> {
    let p = std::path::Path::new(&path);
    if !p.is_file() || !path.to_lowercase().ends_with(".zip") {
        return Err(tr("err.bad_path", cur_lang()).to_string());
    }
    if !std::path::Path::new(&dest).is_dir() {
        return Err(trv("err.dir_not_found", cur_lang(), &[("path", &dest)]));
    }
    let n = tar_list(&path)?;
    tar_extract(&path, &dest)?;
    Ok(n)
}

/// Build the (script, args) for a Backup-tab action. Kept pure so the security-sensitive gating is
/// unit-testable: credentials ride along ONLY on a real `restore` with the explicit flag, a preview
/// is always `-WhatIf` (non-destructive), `-KeepSnapshots` never drops below 1, and an unknown
/// action errors instead of silently running a script.
fn backup_args(
    action: &str,
    timestamp: Option<String>,
    profiles: Option<Vec<String>>,
    include_credentials: Option<bool>,
    keep_snapshots: Option<u32>,
) -> Result<(&'static str, Vec<String>), String> {
    let (script_rel, mut args): (&'static str, Vec<String>) = match action {
        "backup" => {
            let mut a = vec!["-Force".to_string()];
            if let Some(k) = keep_snapshots {
                a.push("-KeepSnapshots".into());
                a.push(k.max(1).to_string());
            }
            (BACKUP_SCRIPT_REL, a)
        }
        "restore-preview" => (RESTORE_SCRIPT_REL, vec!["-WhatIf".to_string()]),
        "restore" => (RESTORE_SCRIPT_REL, Vec::new()),
        "delete-snapshot" => match timestamp.as_deref().filter(|t| !t.is_empty()) {
            // Must carry a non-empty id: an empty -DeleteSnapshot would make the script run a normal
            // backup instead. The PS side additionally pattern-validates the id against traversal.
            Some(id) => (
                BACKUP_SCRIPT_REL,
                vec!["-DeleteSnapshot".to_string(), id.to_string()],
            ),
            None => return Err("delete-snapshot requires a snapshot id".to_string()),
        },
        _ => {
            return Err(trv(
                "err.unknown_backup_action",
                cur_lang(),
                &[("action", &action)],
            ))
        }
    };
    if action == "restore-preview" || action == "restore" {
        if let Some(t) = timestamp {
            if !t.is_empty() {
                args.push("-Timestamp".into());
                args.push(t);
            }
        }
        if let Some(ps) = profiles {
            if !ps.is_empty() {
                args.push("-Profiles".into());
                for p in ps {
                    args.push(p);
                }
            }
        }
    }
    if action == "restore" && include_credentials.unwrap_or(false) {
        args.push("-IncludeCredentials".into());
    }
    Ok((script_rel, args))
}

/// Run a Backup-tab action: create a snapshot, preview a restore (-WhatIf), or restore.
/// Restore is scoped by `timestamp`/`profiles`; credentials only with `include_credentials`.
#[tauri::command]
pub async fn run_backup(
    app: AppHandle,
    state: State<'_, RunState>,
    action: String,
    timestamp: Option<String>,
    profiles: Option<Vec<String>>,
    include_credentials: Option<bool>,
    keep_snapshots: Option<u32>,
) -> Result<i32, String> {
    let (script_rel, args) = backup_args(
        &action,
        timestamp,
        profiles,
        include_credentials,
        keep_snapshots,
    )?;
    let script = abs(script_rel);
    spawn_streamed(app, state, stream_id::BACKUP.to_string(), script, args).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_name_format() {
        assert!(is_snapshot_name("2026-06-12_100002"));
        assert!(!is_snapshot_name("weekly-2026-06-11.zip"));
        assert!(!is_snapshot_name("2026-6-12_100002"));
        assert!(!is_snapshot_name(".backup-state.json"));
        assert!(!is_snapshot_name("2026-06-12_10000")); // too short
    }

    #[test]
    fn backup_args_security_gating() {
        // A plain backup: -Force, KeepSnapshots floored at 1 (never 0 → "keep none").
        let (s, a) = backup_args("backup", None, None, None, Some(0)).unwrap();
        assert_eq!(s, BACKUP_SCRIPT_REL);
        assert!(a.contains(&"-Force".to_string()));
        let kidx = a.iter().position(|x| x == "-KeepSnapshots").unwrap();
        assert_eq!(a[kidx + 1], "1");

        // A preview is always -WhatIf and NEVER carries credentials, even if asked.
        let (s, a) = backup_args(
            "restore-preview",
            Some("2026-06-12_100002".into()),
            None,
            Some(true),
            None,
        )
        .unwrap();
        assert_eq!(s, RESTORE_SCRIPT_REL);
        assert!(a.contains(&"-WhatIf".to_string()));
        assert!(!a.contains(&"-IncludeCredentials".to_string()));

        // A real restore WITHOUT the explicit flag must not include credentials.
        let (_, a) = backup_args("restore", None, None, None, None).unwrap();
        assert!(!a.contains(&"-IncludeCredentials".to_string()));
        assert!(!a.contains(&"-WhatIf".to_string()));

        // Only a real restore WITH the explicit flag carries credentials + scoping.
        let (_, a) = backup_args(
            "restore",
            Some("2026-06-12_100002".into()),
            Some(vec!["work".into()]),
            Some(true),
            None,
        )
        .unwrap();
        assert!(a.contains(&"-IncludeCredentials".to_string()));
        assert!(a.contains(&"-Timestamp".to_string()));
        assert!(a.contains(&"work".to_string()));

        // Unknown action errors — never silently picks a script.
        assert!(backup_args("rm-rf", None, None, Some(true), None).is_err());
    }
}
