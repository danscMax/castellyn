//! Profile link engine — the native replacement for `Repair-ProfileLinks.ps1`.
//!
//! Ports the link half of the profile scripts to Rust, with one deliberate behaviour change
//! recorded in `docs/adr/0003-share-profile-files-without-symlinks.md`: shared FOLDERS are linked
//! with **junctions**, not directory symlinks. A junction needs no Administrator rights, so the
//! elevated `Start-Process -Verb RunAs` path this replaces disappears entirely.
//!
//! Shared FILES are not linked at all any more — they are shared through Claude Code's own
//! configuration (`@` imports, absolute command paths) or reconciled. The single exception is
//! `history.jsonl`, which has no configuration knob: it is symlinked when the OS allows it and
//! silently degrades to a per-profile file when it does not. That degradation costs shared prompt
//! history and nothing else.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What is sitting at a link path right now. Distinguishing "empty real directory" from "real
/// directory holding data" is the whole safety story: the first is safe to replace, the second
/// must never be deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    /// Nothing there.
    Missing,
    /// A reparse point (junction or symlink), healthy or dangling.
    Link { points_at_target: bool },
    /// A real file/dir with no content — safe to replace.
    RealEmpty,
    /// A real file/dir holding data — replacing it would destroy user data.
    RealWithData,
}

/// What to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Healthy link already pointing where it should — leave it alone.
    Keep,
    /// Nothing (or an empty placeholder) in the way — create the link.
    Create,
    /// A link pointing at the wrong place, or dangling — remove and recreate.
    Recreate,
    /// Real data present — report it, never touch it. Merging is a reinstall's job.
    RefuseHasData,
}

/// The decision, separated from all IO so it can be tested exhaustively.
pub fn plan_link(existing: Existing) -> Plan {
    match existing {
        Existing::Link {
            points_at_target: true,
        } => Plan::Keep,
        Existing::Link {
            points_at_target: false,
        } => Plan::Recreate,
        Existing::Missing | Existing::RealEmpty => Plan::Create,
        Existing::RealWithData => Plan::RefuseHasData,
    }
}

/// Two paths naming the same location, compared the way Windows does: case-insensitively and
/// ignoring a trailing separator. Mirrors `Test-LinkHealthy` in the script this replaces.
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Inspect a link path without following it.
pub fn inspect(link: &Path, target: &Path) -> Existing {
    let Ok(meta) = fs::symlink_metadata(link) else {
        return Existing::Missing;
    };
    // is_symlink() is true for every reparse point on Windows, junctions included.
    if meta.file_type().is_symlink() {
        // A link whose target is GONE still has a perfectly readable reparse node pointing at the
        // right path, so a string comparison alone calls it healthy — and repair would then Keep a
        // link that resolves to nothing, forever. `Test-Path` in the script this replaces resolves
        // the link and reports such a link as missing; `resolves()` restores that.
        let points_at_target = resolves(link)
            && fs::read_link(link)
                .map(|t| same_path(&strip_verbatim(&t), &strip_verbatim(target)))
                // Target unreadable but resolvable — don't churn a working link over it.
                .unwrap_or(true);
        return Existing::Link { points_at_target };
    }
    if has_content(link, &meta) {
        Existing::RealWithData
    } else {
        Existing::RealEmpty
    }
}

/// Does this path resolve to something that exists, FOLLOWING any link? `symlink_metadata` sees the
/// reparse node itself and succeeds even when the target is gone; this is the `Test-Path` semantics
/// the PowerShell original used to classify a dangling link as missing.
fn resolves(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

fn has_content(path: &Path, meta: &fs::Metadata) -> bool {
    if meta.is_dir() {
        fs::read_dir(path).map(|mut d| d.next().is_some()).unwrap_or(false)
    } else {
        meta.len() > 0
    }
}

/// `\\?\C:\x` and `\??\C:\x` both name `C:\x`. Junction targets come back in one of the verbatim
/// forms, so compare on the plain path or every healthy link reads as "points elsewhere".
fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    for prefix in ["\\\\?\\", "\\??\\"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return PathBuf::from(rest);
        }
    }
    p.to_path_buf()
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_MODE, FILE_SHARE_NONE,
        GetFileInformationByHandle, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;

    /// READ|WRITE|DELETE — a status probe must not lock out anyone else looking at the same path.
    const SHARE_ALL: FILE_SHARE_MODE = FILE_SHARE_MODE(7);
    /// Not exposed by this windows-crate version, unlike its SET counterpart.
    const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
    use windows::Win32::System::IO::DeviceIoControl;

    pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    /// ReparseTag(4) + ReparseDataLength(2) + Reserved(2).
    const HEADER: usize = 8;
    /// The four USHORT name offsets/lengths that precede PathBuffer.
    const NAME_FIELDS: usize = 8;

    fn wide_nul(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Build a REPARSE_DATA_BUFFER for a mount point (junction).
    ///
    /// Layout after the 8-byte header: SubstituteNameOffset, SubstituteNameLength,
    /// PrintNameOffset, PrintNameLength, then PathBuffer holding the substitute name
    /// (`\??\C:\target`, NUL-terminated) followed by the print name (the plain path).
    fn mount_point_buffer(target: &Path) -> io::Result<Vec<u8>> {
        let plain = strip_verbatim(target);
        let plain = plain.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "junction target is not UTF-8")
        })?;
        let substitute = wide_nul(&format!("\\??\\{plain}"));
        let print = wide_nul(plain);
        // Lengths exclude the terminating NUL; offsets are byte offsets into PathBuffer.
        let sub_len = (substitute.len() - 1) * 2;
        let print_len = (print.len() - 1) * 2;
        let path_bytes = (substitute.len() + print.len()) * 2;

        let mut buf = vec![0u8; HEADER + NAME_FIELDS + path_bytes];
        buf[0..4].copy_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        // ReparseDataLength counts everything after the 8-byte header.
        buf[4..6].copy_from_slice(&((NAME_FIELDS + path_bytes) as u16).to_le_bytes());
        // buf[6..8] = Reserved, already zero.
        buf[8..10].copy_from_slice(&0u16.to_le_bytes()); // SubstituteNameOffset
        buf[10..12].copy_from_slice(&(sub_len as u16).to_le_bytes());
        buf[12..14].copy_from_slice(&((sub_len + 2) as u16).to_le_bytes()); // PrintNameOffset
        buf[14..16].copy_from_slice(&(print_len as u16).to_le_bytes());

        let mut at = HEADER + NAME_FIELDS;
        for w in substitute.iter().chain(print.iter()) {
            buf[at..at + 2].copy_from_slice(&w.to_le_bytes());
            at += 2;
        }
        Ok(buf)
    }

    /// Read a reparse point's tag so a junction can be told apart from a symlink. std exposes
    /// `is_symlink()` for both and no tag, but the dashboard reports the two by name.
    pub fn reparse_tag(path: &Path) -> Option<u32> {
        let wide = wide_nul(&path.to_string_lossy());
        let handle: HANDLE = unsafe {
            CreateFileW(
                windows::core::PCWSTR(wide.as_ptr()),
                0, // querying the tag needs no access rights, only the open itself
                SHARE_ALL,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .ok()?;
        let mut buf = [0u8; 1024];
        let mut returned: u32 = 0;
        let res = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_REPARSE_POINT,
                None,
                0,
                Some(buf.as_mut_ptr() as *mut _),
                buf.len() as u32,
                Some(&mut returned),
                None,
            )
        };
        unsafe {
            let _ = CloseHandle(handle);
        }
        res.ok()?;
        (returned >= 4).then(|| u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
    }

    /// Hard-link count for a file. std's `number_of_links()` is still unstable
    /// (rust-lang/rust#63010), and without this a hard link would be reported as a plain file —
    /// which would show as a permanently "broken" link the repair could never fix, because it
    /// refuses to replace a real file holding data. The old installer created hard links as its
    /// `mklink` fallback, so such profiles exist in the wild.
    pub fn hard_link_count(path: &Path) -> Option<u32> {
        let wide = wide_nul(&path.to_string_lossy());
        let handle: HANDLE = unsafe {
            CreateFileW(
                windows::core::PCWSTR(wide.as_ptr()),
                0,
                SHARE_ALL,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        let res = unsafe { GetFileInformationByHandle(handle, &mut info) };
        unsafe {
            let _ = CloseHandle(handle);
        }
        res.ok().map(|()| info.nNumberOfLinks)
    }

    /// Create a junction at `link` pointing at `target`. Needs no Administrator rights — that is
    /// the entire reason this function exists instead of `symlink_dir`.
    pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
        // A junction is a reparse point ON a directory, so the directory must exist and be empty.
        fs::create_dir_all(link)?;
        let buf = mount_point_buffer(target)?;
        let wide = wide_nul(&link.to_string_lossy());

        // FILE_FLAG_OPEN_REPARSE_POINT so we open the directory itself rather than following any
        // reparse point already on it; FILE_FLAG_BACKUP_SEMANTICS is required to open a directory.
        let handle: HANDLE = unsafe {
            CreateFileW(
                windows::core::PCWSTR(wide.as_ptr()),
                GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(|e| io::Error::other(format!("open {} for junction: {e}", link.display())))?;

        let mut returned: u32 = 0;
        let res = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_REPARSE_POINT,
                Some(buf.as_ptr() as *const _),
                buf.len() as u32,
                None,
                0,
                Some(&mut returned),
                None,
            )
        };
        // Close before reporting: a leaked handle would keep the directory locked for every
        // later repair on this profile.
        unsafe { let _ = CloseHandle(handle); }
        res.map_err(|e| io::Error::other(format!("set reparse point on {}: {e}", link.display())))
    }
}

#[cfg(windows)]
pub use win::create_junction;

#[cfg(not(windows))]
pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Remove whatever is at `link` — a junction must be removed as a directory entry, never
/// recursed into, or the removal would delete the SHARED content it points at.
pub fn remove_link(link: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(link) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if is_dir_entry(&meta) {
        // remove_dir on a reparse point detaches the link; it does not touch the target.
        fs::remove_dir(link)
    } else {
        fs::remove_file(link)
    }
}

/// Does this entry occupy a directory slot? NOT `Metadata::is_dir()`: Rust defines that as
/// "a directory AND not a symlink", so it is false for a junction — which would send a junction
/// to `remove_file` and fail with ACCESS_DENIED. Read the raw attribute instead.
#[cfg(windows)]
fn is_dir_entry(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
}

#[cfg(not(windows))]
fn is_dir_entry(meta: &fs::Metadata) -> bool {
    meta.is_dir()
}

/// How a path is linked, in the exact vocabulary `Get-ProfilesStatus.ps1` emitted and the
/// dashboard still reads: `None` = nothing there, `"none"` = a real item that is not a link,
/// otherwise the link type. Keeping the strings identical is what lets the native snapshot drop
/// into the same `profiles.last.json` contract.
pub fn link_kind(path: &Path) -> Option<&'static str> {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
    let meta = fs::symlink_metadata(path).ok()?;
    // A link pointing at something that no longer exists reads as MISSING, not as a healthy link —
    // matching `Get-LinkKind`, which gates on Test-Path and so resolves the link first.
    if meta.file_type().is_symlink() && !resolves(path) {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Some(match win::reparse_tag(path) {
                Some(IO_REPARSE_TAG_SYMLINK) => "SymbolicLink",
                Some(win::IO_REPARSE_TAG_MOUNT_POINT) => "Junction",
                // An unreadable or exotic tag is still a link — report it as one rather than
                // claiming the profile is broken.
                _ => "SymbolicLink",
            });
        }
        // A hard link is invisible in the attributes; only the link count betrays it.
        if !meta.is_dir() && win::hard_link_count(path).unwrap_or(1) > 1 {
            return Some("HardLink");
        }
    }
    Some("none")
}

/// The only shared item with no configuration route — see the ADR. Everything else in a profile's
/// linked set is a directory and gets a junction.
const FILE_ITEMS: &[&str] = &["history.jsonl"];

/// Advisory per-profile repair lock.
///
/// Deliberately the SAME file, in the same place, with the same exclusive share mode that
/// `Repair-ProfileLinks.ps1` takes (`%LOCALAPPDATA%\ClaudeProfiles\repair-<name>.lock`). That
/// script is still reachable through `Manage-Profiles.ps1`, so a repair started from Castellyn and
/// one started from PowerShell would otherwise interleave on the same junctions. Matching the path
/// exactly is what makes the two interlock; an in-process mutex could not.
/// `LOCALAPPDATA` is never synced, so the lock stays machine-local by construction.
pub struct RepairLock(#[allow(dead_code)] fs::File);

impl RepairLock {
    /// `None` = someone else is repairing this profile right now; the caller should stand down.
    pub fn acquire(name: &str) -> Option<RepairLock> {
        let dir = PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("ClaudeProfiles");
        fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("repair-{name}.lock"));
        let mut opts = fs::OpenOptions::new();
        opts.read(true).write(true).create(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            opts.share_mode(0); // FILE_SHARE_NONE — the exclusive hold the PowerShell lock expects
        }
        opts.open(&path).ok().map(RepairLock)
    }
}

/// What happened to one linked item. Deliberately data, not text: `lib.rs` owns the translation,
/// so this module stays testable without an i18n table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Already linked correctly — nothing done.
    Ok,
    /// Link created.
    Made,
    /// Link existed but pointed elsewhere; torn down and rebuilt.
    Rebuilt,
    /// Real data present — refused, left untouched.
    HasData,
    /// `history.jsonl` could not be linked; the profile keeps its own. Not an error.
    HistoryLocal,
    /// Shared file copied into the profile.
    Copied,
    /// Nothing to do: the file is reached by absolute path, or has no shared source.
    Skipped,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOutcome {
    pub item: String,
    pub outcome: Outcome,
}

/// How a shared FILE reaches a profile when it is not already there.
///
/// Files are not linked (see the ADR) — but they are not imported either, which an earlier design
/// assumed. MEASURED, not reasoned: with a `CLAUDE.md` that Claude Code demonstrably loaded,
/// `@local-probe.md` (inside the tree) resolved while `@~/probe-marker.md` (outside it) did not.
/// External imports sit behind a one-time approval the docs describe, so an automated flow cannot
/// rely on them — a profile wired that way would silently carry no instructions at all.
///
///   * `.md`, `.json` → a copy. For settings this was always the only option (no `extends`, no
///     include, and the highest-precedence file replaces rather than merges); for markdown it is
///     what the experiment above forces. A copy also keeps `CLAUDE.md`'s own RELATIVE imports
///     working, since those resolve next to the copy — which is why `RTK.md` must be copied too.
///   * anything else (`.py`, `.ps1`) → nothing at all. Those are invoked through `statusLine`
///     and hooks, which take an absolute path pointing at the shared copy, so a per-profile
///     duplicate would never be read and could only go stale.
///
/// A copy can go stale when the shared original changes. That is the reconciler's job — the
/// snapshot reports the file, and drift-on-content is the next step, not silent divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provision {
    Copy,
    Skip,
}

pub fn provision_of(item: &str) -> Provision {
    let lower = item.to_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".json") {
        Provision::Copy
    } else {
        Provision::Skip
    }
}

/// State of a shared FILE inside a profile, for the status snapshot.
///
/// `None` = the profile does not have it. `"link"` = shared by a link, so it cannot drift.
/// `"copy"` = a copy identical to the shared original. `"drift"` = a copy that differs.
///
/// Drift exists because the chosen mechanism is a copy (an `@` import across the tree does not
/// load — see the ADR), and a copy silently going stale is exactly the failure the reconciler must
/// make visible. Drift is REPORTED, never corrected on its own: the difference may be a deliberate
/// per-profile customisation.
pub fn shared_file_state(dest: &Path, source: &Path) -> Option<&'static str> {
    let meta = fs::symlink_metadata(dest).ok()?;
    if meta.file_type().is_symlink() {
        // Same trap `link_kind` guards: the reparse node survives its target's deletion, so a
        // dangling link would otherwise report as a healthy "link" and the profile would look
        // fully instructed while carrying nothing at all.
        return resolves(dest).then_some("link");
    }
    // No shared original to be behind — this file is the profile's own. Calling that "drift" would
    // offer a sync action that can never succeed and would blame the wrong side of the pair.
    if !source.is_file() {
        return Some("local");
    }
    Some(if files_equal(dest, source) {
        "copy"
    } else {
        "drift"
    })
}

/// Byte comparison. These files are a few KB, so reading both beats hashing; an unreadable side
/// counts as "differs" rather than silently claiming they match.
fn files_equal(a: &Path, b: &Path) -> bool {
    match (fs::read(a), fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Overwrite a profile's DRIFTED copies with the shared original. Destructive by nature — the
/// caller must have confirmed it. Files that are links, already identical, or absent are not
/// touched: this only ever collapses a difference that was reported first.
pub fn resync_shared_files(
    profile_dir: &Path,
    main_dir: &Path,
    items: &[String],
) -> Vec<ItemOutcome> {
    items
        .iter()
        .filter(|item| provision_of(item) != Provision::Skip)
        .map(|item| {
            let dest = profile_dir.join(item);
            let source = main_dir.join(item);
            let outcome = match shared_file_state(&dest, &source) {
                Some("drift") => match fs::copy(&source, &dest) {
                    Ok(_) => Outcome::Copied,
                    Err(e) => Outcome::Failed(format!("{}: {e}", dest.display())),
                },
                _ => Outcome::Skipped,
            };
            ItemOutcome {
                item: item.clone(),
                outcome,
            }
        })
        .collect()
}

/// Give a profile the shared FILES it is missing — and only those.
///
/// Anything already present is left exactly as it is, link or real file: converting a working
/// setup buys nothing and risks silently emptying a profile's instructions. New profiles (and
/// machines that cannot make symlinks at all) get the configuration-based equivalent instead.
pub fn provision_shared_files(
    profile_dir: &Path,
    main_dir: &Path,
    items: &[String],
) -> Vec<ItemOutcome> {
    items
        .iter()
        .map(|item| ItemOutcome {
            item: item.clone(),
            outcome: provision_one(profile_dir, main_dir, item),
        })
        .collect()
}

fn provision_one(profile_dir: &Path, main_dir: &Path, item: &str) -> Outcome {
    let dest = profile_dir.join(item);
    // symlink_metadata, not exists(): a dangling link is still "something the user set up", and
    // replacing it is not this function's job.
    if fs::symlink_metadata(&dest).is_ok() {
        return Outcome::Ok;
    }
    let source = main_dir.join(item);
    if !source.is_file() {
        return Outcome::Skipped;
    }
    match provision_of(item) {
        Provision::Skip => Outcome::Skipped,
        Provision::Copy => match fs::copy(&source, &dest) {
            Ok(_) => Outcome::Copied,
            Err(e) => Outcome::Failed(format!("{}: {e}", dest.display())),
        },
    }
}

/// Make one profile's shared links match `items`. Needs no elevation: directories become
/// junctions and the single shared file degrades to a local copy rather than demanding rights.
/// Idempotent — a healthy link is left alone, so repeated runs do not churn the filesystem.
pub fn repair_profile_links(
    profile_dir: &Path,
    main_dir: &Path,
    items: &[String],
) -> Vec<ItemOutcome> {
    items
        .iter()
        .map(|item| ItemOutcome {
            item: item.clone(),
            outcome: repair_one(profile_dir, main_dir, item),
        })
        .collect()
}

fn repair_one(profile_dir: &Path, main_dir: &Path, item: &str) -> Outcome {
    let link = profile_dir.join(item);
    let target = main_dir.join(item);
    let is_file = FILE_ITEMS.contains(&item);

    // The shared target must exist before anything can point at it (mirrors the installer's prep).
    let prep = if !is_file {
        fs::create_dir_all(&target)
    } else if target.exists() {
        Ok(())
    } else {
        fs::write(&target, b"")
    };
    if let Err(e) = prep {
        return Outcome::Failed(format!("prepare {}: {e}", target.display()));
    }

    let plan = plan_link(inspect(&link, &target));
    match plan {
        Plan::Keep => return Outcome::Ok,
        Plan::RefuseHasData => return Outcome::HasData,
        Plan::Create | Plan::Recreate => {}
    }
    if let Err(e) = remove_link(&link) {
        return Outcome::Failed(format!("remove {}: {e}", link.display()));
    }

    let made = if is_file {
        link_file(&link, &target)
    } else {
        create_junction(&link, &target)
    };
    match made {
        Ok(()) if plan == Plan::Recreate => Outcome::Rebuilt,
        Ok(()) => Outcome::Made,
        // A file link is best-effort by design: leave the profile its own file and say so.
        Err(_) if is_file => match fs::write(&link, b"") {
            Ok(()) => Outcome::HistoryLocal,
            Err(e) => Outcome::Failed(format!("{}: {e}", link.display())),
        },
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

#[cfg(windows)]
fn link_file(link: &Path, target: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(windows))]
fn link_file(link: &Path, target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_covers_every_state_exactly_once() {
        assert_eq!(
            plan_link(Existing::Link {
                points_at_target: true
            }),
            Plan::Keep
        );
        assert_eq!(
            plan_link(Existing::Link {
                points_at_target: false
            }),
            Plan::Recreate
        );
        assert_eq!(plan_link(Existing::Missing), Plan::Create);
        assert_eq!(plan_link(Existing::RealEmpty), Plan::Create);
        // The one that matters: real data is never silently replaced.
        assert_eq!(plan_link(Existing::RealWithData), Plan::RefuseHasData);
    }

    /// The load-bearing test: the pure ones above prove nothing about the REPARSE_DATA_BUFFER
    /// layout. This one builds a real junction on disk, unelevated, and reads a file through it.
    /// If the byte layout is wrong, DeviceIoControl fails here rather than in front of a user.
    #[cfg(windows)]
    #[test]
    fn junction_is_created_unelevated_and_reads_through() {
        let root = std::env::temp_dir().join(format!("castellyn-junction-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let target = root.join("shared");
        let link = root.join("profile-view");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("marker.txt"), "through-the-junction").unwrap();

        create_junction(&link, &target).expect("junction must be creatable WITHOUT admin rights");

        // 1. It reads through to the target's content.
        let seen = fs::read_to_string(link.join("marker.txt")).unwrap();
        assert_eq!(seen, "through-the-junction");

        // 2. It is recognised as a link, and inspect() agrees it points at the target — the
        //    property that keeps repair idempotent instead of rebuilding links every run.
        assert_eq!(inspect(&link, &target), Existing::Link { points_at_target: true });
        assert_eq!(plan_link(inspect(&link, &target)), Plan::Keep);

        // 3. A junction pointing elsewhere is detected as such, not silently accepted.
        let elsewhere = root.join("other");
        fs::create_dir_all(&elsewhere).unwrap();
        assert_eq!(inspect(&link, &elsewhere), Existing::Link { points_at_target: false });

        // 4. Removing the link must NOT take the shared content with it. This is the assertion
        //    that would have caught a recursive delete, the worst bug this module could have.
        remove_link(&link).unwrap();
        assert!(!link.exists(), "link should be gone");
        assert!(
            target.join("marker.txt").exists(),
            "removing a junction must never delete the shared target's content"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// End-to-end on a real profile shape: links get made, a second run is a no-op, and a folder
    /// holding real data is refused rather than replaced.
    #[cfg(windows)]
    #[test]
    fn repair_links_a_profile_idempotently_and_never_eats_data() {
        let root = std::env::temp_dir().join(format!("castellyn-repair-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let main = root.join(".claude");
        let profile = root.join(".claude-cc1");
        fs::create_dir_all(&profile).unwrap();
        fs::create_dir_all(main.join("skills")).unwrap();
        fs::write(main.join("skills").join("s.md"), "shared skill").unwrap();

        // A folder the user already filled by hand — the one thing repair must never destroy.
        fs::create_dir_all(profile.join("agents")).unwrap();
        fs::write(profile.join("agents").join("mine.md"), "hand written").unwrap();

        let items: Vec<String> = ["skills", "agents", "projects", "history.jsonl"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let first = repair_profile_links(&profile, &main, &items);
        let by = |n: &str| {
            first
                .iter()
                .find(|o| o.item == n)
                .unwrap()
                .outcome
                .clone()
        };
        assert_eq!(by("skills"), Outcome::Made);
        assert_eq!(by("projects"), Outcome::Made, "missing target should be created, then linked");
        assert_eq!(
            by("agents"),
            Outcome::HasData,
            "a folder with real data must be refused, never replaced"
        );
        assert!(
            profile.join("agents").join("mine.md").exists(),
            "the user's own file must survive a repair"
        );
        // history.jsonl links when the OS allows it and degrades otherwise — both are acceptable,
        // a hard failure is not.
        assert!(matches!(by("history.jsonl"), Outcome::Made | Outcome::HistoryLocal));

        // Shared content is reachable through the new junction.
        assert_eq!(
            fs::read_to_string(profile.join("skills").join("s.md")).unwrap(),
            "shared skill"
        );

        // Second run: everything healthy is left alone. Churning links on every repair would be
        // a silent bug — nothing visible would break, but the filesystem would thrash.
        let second = repair_profile_links(&profile, &main, &items);
        let skills_again = second.iter().find(|o| o.item == "skills").unwrap();
        assert_eq!(skills_again.outcome, Outcome::Ok, "repair must be idempotent");

        let _ = fs::remove_dir_all(&root);
    }

    /// The lock is only worth having if a second holder is actually refused. An advisory lock that
    /// silently grants twice would look identical in the log and protect nothing.
    #[cfg(windows)]
    #[test]
    fn a_second_repair_of_the_same_profile_is_refused_while_the_first_holds() {
        let name = format!("locktest{}", std::process::id());
        let first = RepairLock::acquire(&name).expect("first acquire must succeed");
        assert!(
            RepairLock::acquire(&name).is_none(),
            "a second holder must be refused while the first is alive"
        );
        drop(first);
        assert!(
            RepairLock::acquire(&name).is_some(),
            "the lock must be released when the guard drops"
        );
        // A DIFFERENT profile is independent — the lock is per profile, not global.
        assert!(RepairLock::acquire(&format!("{name}b")).is_some());

        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            let dir = PathBuf::from(la).join("ClaudeProfiles");
            let _ = fs::remove_file(dir.join(format!("repair-{name}.lock")));
            let _ = fs::remove_file(dir.join(format!("repair-{name}b.lock")));
        }
    }

    /// A junction whose target was deleted still has a readable reparse node naming the right path.
    /// Comparing that name alone calls it healthy — so status would hide the breakage AND repair
    /// would Keep it forever. Both must treat it as missing.
    #[cfg(windows)]
    #[test]
    fn a_link_whose_target_vanished_is_broken_not_healthy() {
        let root = std::env::temp_dir().join(format!("castellyn-dangle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let target = root.join("shared");
        let link = root.join("view");
        fs::create_dir_all(&target).unwrap();
        create_junction(&link, &target).unwrap();
        assert_eq!(inspect(&link, &target), Existing::Link { points_at_target: true });
        assert_eq!(link_kind(&link), Some("Junction"));

        // Now delete what it points at. The junction node itself survives.
        fs::remove_dir_all(&target).unwrap();
        assert!(
            fs::symlink_metadata(&link).is_ok(),
            "the reparse node must still be there — that is what made this trap possible"
        );
        assert_eq!(
            inspect(&link, &target),
            Existing::Link { points_at_target: false },
            "a dangling link must not read as pointing at its target"
        );
        assert_eq!(plan_link(inspect(&link, &target)), Plan::Recreate);
        assert_eq!(link_kind(&link), None, "status must report it as missing");

        let _ = fs::remove_dir_all(&root);
    }

    /// The rule the owner chose: leave a working setup alone, provide only what is missing.
    #[test]
    fn shared_files_fill_the_gaps_and_never_touch_what_is_already_there() {
        let root = std::env::temp_dir().join(format!("castellyn-share-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let main = root.join(".claude");
        let profile = root.join(".claude-cc1");
        fs::create_dir_all(&profile).unwrap();
        fs::create_dir_all(&main).unwrap();
        for f in ["CLAUDE.md", "RTK.md", "settings.local.json", "statusline.py"] {
            fs::write(main.join(f), format!("shared {f}")).unwrap();
        }
        // Already present in the profile, with DIFFERENT content — must survive untouched.
        fs::write(profile.join("RTK.md"), "my own RTK").unwrap();

        let items: Vec<String> = ["CLAUDE.md", "RTK.md", "settings.local.json", "statusline.py"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = provision_shared_files(&profile, &main, &items);
        let got = |n: &str| out.iter().find(|o| o.item == n).unwrap().outcome.clone();

        // Copied, NOT imported: an experiment showed an `@~/...` import outside the tree silently
        // fails to load, which would leave the profile with no instructions at all.
        assert_eq!(got("CLAUDE.md"), Outcome::Copied);
        assert_eq!(
            fs::read_to_string(profile.join("CLAUDE.md")).unwrap(),
            "shared CLAUDE.md"
        );
        // RTK.md must be copied too: CLAUDE.md's own `@RTK.md` is RELATIVE, so it resolves next to
        // the copy — a profile with CLAUDE.md but no RTK.md would have a dangling import.
        assert_eq!(got("settings.local.json"), Outcome::Copied);
        assert_eq!(
            fs::read_to_string(profile.join("settings.local.json")).unwrap(),
            "shared settings.local.json",
            "settings have no inheritance, so they must be copied, not referenced"
        );
        // A script is reached by absolute path from settings — a per-profile copy would be dead
        // weight that silently goes stale.
        assert_eq!(got("statusline.py"), Outcome::Skipped);
        assert!(!profile.join("statusline.py").exists());

        // The one that matters most: an existing file is reported as fine and left byte-identical.
        assert_eq!(got("RTK.md"), Outcome::Ok);
        assert_eq!(
            fs::read_to_string(profile.join("RTK.md")).unwrap(),
            "my own RTK",
            "a working setup must never be rewritten"
        );

        // Idempotent: a second pass reports everything as already fine and writes nothing new.
        let again = provision_shared_files(&profile, &main, &items);
        assert!(
            again
                .iter()
                .all(|o| matches!(o.outcome, Outcome::Ok | Outcome::Skipped)),
            "second pass must be a no-op"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// A copy is only acceptable if divergence is VISIBLE — that is the whole trade the ADR made
    /// when the import mechanism turned out not to load. This pins drift detection and the
    /// explicit, non-automatic correction.
    #[test]
    fn a_copy_that_diverges_is_reported_and_only_resynced_on_request() {
        let root = std::env::temp_dir().join(format!("castellyn-drift-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let main = root.join(".claude");
        let profile = root.join(".claude-cc1");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(&profile).unwrap();
        fs::write(main.join("CLAUDE.md"), "shared v1").unwrap();
        fs::write(main.join("settings.local.json"), "{}").unwrap();

        let items: Vec<String> = ["CLAUDE.md", "settings.local.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        provision_shared_files(&profile, &main, &items);
        let state = |n: &str| shared_file_state(&profile.join(n), &main.join(n));
        assert_eq!(state("CLAUDE.md"), Some("copy"), "fresh copy matches its source");

        // The shared original moves on — the profile's copy is now stale.
        fs::write(main.join("CLAUDE.md"), "shared v2").unwrap();
        assert_eq!(
            state("CLAUDE.md"),
            Some("drift"),
            "a stale copy must be visible, not silent"
        );

        // Reporting must NOT correct it: the difference can be a deliberate customisation.
        provision_shared_files(&profile, &main, &items);
        assert_eq!(
            fs::read_to_string(profile.join("CLAUDE.md")).unwrap(),
            "shared v1",
            "filling gaps must never overwrite an existing file"
        );

        // Only the explicit resync collapses it.
        let out = resync_shared_files(&profile, &main, &items);
        assert_eq!(
            out.iter().find(|o| o.item == "CLAUDE.md").unwrap().outcome,
            Outcome::Copied
        );
        assert_eq!(fs::read_to_string(profile.join("CLAUDE.md")).unwrap(), "shared v2");
        assert_eq!(state("CLAUDE.md"), Some("copy"));

        // A dangling LINK must not read as a healthy "link" — same trap link_kind already guards.
        let dangling = profile.join("dangling.md");
        let gone = main.join("gone.md");
        fs::write(&gone, "x").unwrap();
        if std::os::windows::fs::symlink_file(&gone, &dangling).is_ok() {
            assert_eq!(shared_file_state(&dangling, &gone), Some("link"));
            fs::remove_file(&gone).unwrap();
            assert_eq!(
                shared_file_state(&dangling, &gone),
                None,
                "a link whose target vanished must read as missing, not healthy"
            );
        }

        // A real file with no shared counterpart is the profile's own, not drift.
        fs::write(profile.join("own.md"), "mine").unwrap();
        assert_eq!(
            shared_file_state(&profile.join("own.md"), &main.join("own.md")),
            Some("local"),
            "a missing source must not be blamed on the profile's copy"
        );
        // An untouched file is left alone by the resync too.
        assert_eq!(
            out.iter()
                .find(|o| o.item == "settings.local.json")
                .unwrap()
                .outcome,
            Outcome::Skipped
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn verbatim_prefixes_compare_equal_to_plain_paths() {
        // A junction read back as \??\C:\x must not read as "points somewhere else" against C:\x,
        // otherwise every healthy link would be torn down and rebuilt on each repair.
        assert!(same_path(
            &strip_verbatim(Path::new("\\??\\C:\\Users\\x\\.claude\\skills")),
            &strip_verbatim(Path::new("C:\\Users\\x\\.claude\\skills")),
        ));
        assert!(same_path(
            &strip_verbatim(Path::new("\\\\?\\C:\\Users\\x\\.claude")),
            Path::new("c:\\users\\x\\.claude\\"),
        ));
        assert!(!same_path(Path::new("C:\\a"), Path::new("C:\\b")));
    }
}
