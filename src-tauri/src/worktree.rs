//! Worktree-per-session (W3): run a session in an isolated `git worktree` so parallel sessions no
//! longer share one working copy (the "File modified since read" contention). Ported from Orca's
//! worktree lifecycle — the hard-won safety lessons are the point (see plans/orca-analysis):
//!   - remove only OUR worktrees, proven by a persisted `castellynSource` provenance mark (a path
//!     shape is not authority — the user may have made a plain worktree inside the same folder);
//!   - `-d` never `-D` when deleting the branch, so unmerged agent commits are preserved;
//!   - dangerous-path + nested-worktree guards before any removal (a force-remove would silently
//!     wipe a nested worktree's files);
//!   - classify results by FACT (re-read `git worktree list`), not by localized git error text.
//!
//! Git is on PATH (already used by clone_repo). Every spawn sets CREATE_NO_WINDOW (crate canon).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use std::os::windows::process::CommandExt;

use crate::CREATE_NO_WINDOW;

/// `git worktree add` checkout bound — a OneDrive cloud-placeholder can otherwise hang the checkout
/// for minutes (Orca ticket STA-1292). Generous enough not to kill a legitimate large checkout.
const ADD_TIMEOUT: Duration = Duration::from_secs(180);
/// Windows filesystem-removal retry backoff (ms): AV / indexers / just-released handles briefly hold
/// a freshly-emptied dir (EBUSY/ENOTEMPTY/EPERM) — a short retry clears it.
const REMOVE_RETRY_MS: [u64; 4] = [250, 500, 1000, 2000];
/// Max suffix attempts when a name collides (`feat` → `feat-2` … `feat-100`).
const MAX_SUFFIX: u32 = 100;

#[derive(serde::Serialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveResult {
    /// The branch was kept (unmerged/unpublished commits) instead of deleted — surfaced so the UI can
    /// tell the user their work survives. `None` = branch cleanly deleted (no work to preserve).
    pub preserved_branch: Option<String>,
}

/// Run git in `repo` (blocking), hidden console. Git is on PATH.
fn git(repo: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

/// Collapse every run of dots to a single dot. `..` in a name would make git reject the ref
/// (`check-ref-format`), and `../../x` would otherwise slugify to `..-..-x` and still be rejected.
fn collapse_dots(s: &str) -> String {
    let mut out = String::new();
    let mut in_dots = false;
    for c in s.chars() {
        if c == '.' {
            in_dots = true;
        } else {
            if in_dots {
                out.push('.');
                in_dots = false;
            }
            out.push(c);
        }
    }
    if in_dots {
        out.push('.');
    }
    out
}

/// Session/recipe name → a safe git-branch + folder name. Keeps Unicode letters/digits (CJK,
/// Cyrillic, diacritics); replaces only what git/the filesystem actually reject. Empty / `.` / `..`
/// after cleaning → Err (there is no valid branch to make).
fn sanitize_name(input: &str) -> Result<String, String> {
    let collapsed = collapse_dots(input);
    let mut s: String = collapsed
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let trimmed = s.trim_matches(|c| c == '-' || c == '.').to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return Err(format!("invalid worktree name: {input:?}"));
    }
    Ok(trimmed)
}

fn candidate(base: &str, n: u32) -> String {
    if n == 1 {
        base.to_string()
    } else {
        format!("{base}-{n}")
    }
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")],
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// `git worktree add --no-track -b <branch> <path>` with a 180s bound. `--no-track` so the new
/// branch has no upstream and `git status` doesn't read it as "behind N" before the first push.
fn git_add_timeout(repo: &Path, branch: &str, path: &str) -> Result<(), String> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .current_dir(repo)
        .args(["worktree", "add", "--no-track", "-b", branch, path])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git worktree add: {e}"))?;
    // Drain stderr on its own thread: reading it only after exit deadlocks if a checkout hook/helper
    // emits more than the OS pipe buffer (~64KB) — git blocks writing, never exits, and dies only at
    // the timeout. The thread reads to EOF; the pipe closes when git (or its tree-kill) ends.
    let mut err_pipe = child.stderr.take();
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + ADD_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let s = err_t.join().unwrap_or_default();
                if status.success() {
                    return Ok(());
                }
                return Err(if s.trim().is_empty() {
                    "git worktree add failed".to_string()
                } else {
                    s.trim().to_string()
                });
            }
            Ok(None) => {
                if Instant::now() > deadline {
                    // Tree-kill (checkout may have spawned children), then reap so the child + reader
                    // thread don't leak — child.kill() alone left descendants and an unreaped root.
                    let _ = Command::new("taskkill")
                        .args(["/T", "/F", "/PID", &child.id().to_string()])
                        .creation_flags(CREATE_NO_WINDOW)
                        .output();
                    let _ = child.wait();
                    let _ = err_t.join();
                    return Err("git worktree add timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("git worktree add: {e}")),
        }
    }
}

/// Create an isolated worktree off the repo's current HEAD. Returns the checkout path + branch name.
/// Collision-safe: a taken name/branch/path advances the suffix (`feat` → `feat-2` …).
#[tauri::command]
pub fn worktree_create(repo: String, name: String) -> Result<WorktreeInfo, String> {
    let repo_path = Path::new(&repo);
    if !repo_path.join(".git").exists() {
        return Err(format!("not a git repository: {repo}"));
    }
    let base = sanitize_name(&name)?;
    let parent = repo_path
        .parent()
        .ok_or_else(|| "repo has no parent directory".to_string())?;
    let repo_name = repo_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    // Sibling of the repo dir: <repo>\..\.castellyn-worktrees\<repoName>\<name>
    let root = parent.join(".castellyn-worktrees").join(&repo_name);

    for n in 1..=MAX_SUFFIX {
        let cand = candidate(&base, n);
        let branch = format!("castellyn/{cand}");
        let wt_path = root.join(&cand);
        if wt_path.exists() || branch_exists(repo_path, &branch) {
            continue;
        }
        std::fs::create_dir_all(&root).map_err(|e| format!("mkdir worktree root: {e}"))?;
        let wt_str = wt_path.to_string_lossy().to_string();
        git_add_timeout(repo_path, &branch, &wt_str)?;
        // Provenance: mark this branch as ours so removal is authorized by a persisted fact, never by
        // the path shape. Best-effort — the mark's absence just makes removal refuse (fail-safe).
        let ms = now_ms();
        let _ = git(
            repo_path,
            &[
                "config",
                &format!("branch.{branch}.castellynSource"),
                "castellyn",
            ],
        );
        let _ = git(
            repo_path,
            &[
                "config",
                &format!("branch.{branch}.castellynCreatedAt"),
                &ms.to_string(),
            ],
        );
        return Ok(WorktreeInfo {
            path: wt_str,
            branch,
        });
    }
    Err(format!("no free worktree name after {MAX_SUFFIX} attempts"))
}

/// `git status --porcelain` empty = clean. Any git error → false (conservative: a caller that can't
/// prove cleanliness must NOT remove the worktree).
#[tauri::command]
pub fn worktree_is_clean(path: String) -> bool {
    git(Path::new(&path), &["status", "--porcelain"])
        .map(|o| o.status.success() && o.stdout.iter().all(|b| b.is_ascii_whitespace()))
        .unwrap_or(false)
}

/// True if `path` looks like a git repo root (has a `.git` dir or, for a linked worktree, a `.git`
/// file). Cheap filesystem check — gates the launch-form "isolate in worktree" checkbox.
#[tauri::command]
pub fn is_git_repo(path: String) -> bool {
    !path.trim().is_empty() && Path::new(&path).join(".git").exists()
}

struct WtEntry {
    path: String,
    branch: Option<String>,
}

/// Parse `git worktree list --porcelain`. First entry is the main worktree. `branch` is stripped of
/// its `refs/heads/` prefix; a detached/bare entry has `None`.
fn list_worktrees(repo: &Path) -> Result<Vec<WtEntry>, String> {
    let out = git(repo, &["worktree", "list", "--porcelain"])
        .map_err(|e| format!("git worktree list: {e}"))?;
    if !out.status.success() {
        return Err("git worktree list failed".to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries: Vec<WtEntry> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(p) = line.strip_prefix("worktree ") {
            entries.push(WtEntry {
                path: p.to_string(),
                branch: None,
            });
        } else if let Some(b) = line.strip_prefix("branch ") {
            if let Some(last) = entries.last_mut() {
                last.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            }
        }
    }
    Ok(entries)
}

/// Normalize a path for comparison: lowercase, forward slashes, no trailing slash (Windows is
/// case-insensitive and git may echo either separator).
fn norm(p: &str) -> String {
    p.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// Is `inner` at or under `ancestor`? The `ancestor/` boundary avoids sibling-prefix false positives
/// (`parent-copy` is NOT under `parent`).
fn is_under(inner: &str, ancestor: &str) -> bool {
    inner == ancestor || inner.starts_with(&format!("{ancestor}/"))
}

/// A drive root ("e:" / "e:/") normalizes to a 2-char "x:" — never removable.
fn is_drive_root(np: &str) -> bool {
    np.len() <= 2 && np.chars().nth(1) == Some(':')
}

/// Refuse removal if the path is empty, is the repo itself, is a drive root, CONTAINS the repo, or is
/// (under) the user's home dir. A force-remove of any of these is a catastrophe.
fn is_dangerous(path: &str, repo: &str) -> bool {
    let np = norm(path);
    if np.is_empty() || is_drive_root(&np) {
        return true;
    }
    let nrepo = norm(repo);
    if np == nrepo || is_under(&nrepo, &np) {
        return true; // path == repo, or worktree contains the repo
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        let nh = norm(&home);
        if !nh.is_empty() && (np == nh || is_under(&nh, &np)) {
            return true;
        }
    }
    false
}

/// Recursive delete with Windows transient-error retries + long-path (`\\?\`) fallback. Only reached
/// in the partial-delete recovery path (git accepted the removal but left files behind).
fn remove_dir_all_retry(path: &Path) {
    let long = to_verbatim(path);
    for (i, delay) in std::iter::once(&0u64).chain(REMOVE_RETRY_MS.iter()).enumerate() {
        if *delay > 0 {
            std::thread::sleep(Duration::from_millis(*delay));
        }
        match std::fs::remove_dir_all(&long) {
            Ok(_) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if i < REMOVE_RETRY_MS.len() => continue,
            Err(_) => return, // best-effort — a leftover dir is not worth failing the whole remove
        }
    }
}

/// `\\?\`-prefixed path so Git-for-Windows-style long recursive deletes don't fail. Assumes an
/// absolute backslash path (which our worktree paths always are).
fn to_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().replace('/', "\\");
    if s.starts_with("\\\\") {
        PathBuf::from(s)
    } else {
        PathBuf::from(format!("\\\\?\\{s}"))
    }
}

/// Remove one of OUR worktrees and clean up its branch. All guards run BEFORE any side effect. The
/// branch is deleted with `-d` (never `-D`): if it holds unmerged commits git refuses and the branch
/// is preserved (returned as `preservedBranch`). Never forces a dirty removal.
#[tauri::command]
pub fn worktree_remove(repo: String, path: String) -> Result<RemoveResult, String> {
    let repo_path = Path::new(&repo);
    let entries = list_worktrees(repo_path)?;
    let target = norm(&path);

    let idx = entries
        .iter()
        .position(|e| norm(&e.path) == target)
        .ok_or_else(|| format!("refusing to remove unregistered worktree: {path}"))?;
    if idx == 0 {
        return Err("refusing to remove the main worktree".to_string());
    }
    if is_dangerous(&path, &repo) {
        return Err(format!("refusing to remove protected path: {path}"));
    }
    // A force-remove treats a nested registered worktree as untracked and wipes its files, orphaning
    // git's admin record — refuse if this worktree contains another registered one.
    for (i, e) in entries.iter().enumerate() {
        if i == idx {
            continue;
        }
        let ne = norm(&e.path);
        if ne != target && is_under(&ne, &target) {
            return Err(format!("refusing: worktree contains a nested worktree: {path}"));
        }
    }
    // Provenance is the authority, not the path shape: the user may have made a plain worktree here.
    let branch = entries[idx]
        .branch
        .clone()
        .ok_or_else(|| format!("refusing: worktree has no branch (not a castellyn worktree): {path}"))?;
    let src = git(
        repo_path,
        &["config", "--get", &format!("branch.{branch}.castellynSource")],
    )
    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    .unwrap_or_default();
    if src != "castellyn" {
        return Err(format!("refusing: not a castellyn-created worktree: {path}"));
    }

    // Remove (no --force). Classify by FACT, not by localized error text.
    let rm = git(repo_path, &["worktree", "remove", &path])
        .map_err(|e| format!("git worktree remove: {e}"))?;
    if !rm.status.success() {
        let still = list_worktrees(repo_path)?
            .iter()
            .any(|e| norm(&e.path) == target);
        if still {
            // A genuine refusal (dirty / locked) — never force.
            return Err(format!(
                "worktree has uncommitted changes or is locked, left in place: {path}"
            ));
        }
        // git accepted the removal but Windows left files behind — clean up and prune the admin record.
        remove_dir_all_retry(Path::new(&path));
        let _ = git(repo_path, &["worktree", "prune"]);
    }

    // Delete the branch with -d (preserves unmerged work). A refusal → keep it, report as preserved.
    let preserved = match git(repo_path, &["branch", "-d", &branch]) {
        Ok(o) if o.status.success() => None,
        _ => Some(branch),
    };
    Ok(RemoveResult {
        preserved_branch: preserved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn run(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    fn temp_repo() -> PathBuf {
        let mut p = std::env::temp_dir();
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        p.push(format!("cast-wt-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&p).unwrap();
        run(&p, &["init", "-q"]);
        std::fs::write(p.join("a.txt"), "hi").unwrap();
        run(&p, &["add", "."]);
        run(&p, &["commit", "-q", "-m", "init"]);
        p
    }

    fn cleanup(repo: &Path) {
        let _ = std::fs::remove_dir_all(repo);
        if let (Some(parent), Some(name)) = (repo.parent(), repo.file_name()) {
            let _ = std::fs::remove_dir_all(parent.join(".castellyn-worktrees").join(name));
        }
    }

    #[test]
    fn sanitize_traversal_empty_unicode() {
        // ../../x must not error and must not carry `..` (git would reject the ref).
        let s = sanitize_name("../../x").unwrap();
        assert!(!s.is_empty() && !s.contains(".."), "got {s:?}");
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("..").is_err());
        assert!(sanitize_name(".").is_err());
        // Unicode survives verbatim.
        assert_eq!(sanitize_name("проект").unwrap(), "проект");
    }

    #[test]
    fn suffix_collision_bumps_name() {
        let repo = temp_repo();
        let rs = repo.display().to_string();
        let a = worktree_create(rs.clone(), "feat".into()).unwrap();
        let b = worktree_create(rs.clone(), "feat".into()).unwrap();
        assert_eq!(a.branch, "castellyn/feat");
        assert_eq!(b.branch, "castellyn/feat-2");
        assert!(Path::new(&a.path).exists());
        assert!(Path::new(&b.path).exists());
        cleanup(&repo);
    }

    #[test]
    fn create_remove_round_trip() {
        let repo = temp_repo();
        let rs = repo.display().to_string();
        let c = worktree_create(rs.clone(), "round".into()).unwrap();
        assert!(Path::new(&c.path).exists());
        assert!(worktree_is_clean(c.path.clone()));
        let r = worktree_remove(rs.clone(), c.path.clone()).unwrap();
        assert!(!Path::new(&c.path).exists());
        // No commits beyond base → branch is merged → -d succeeds → nothing preserved.
        assert_eq!(r.preserved_branch, None);
        cleanup(&repo);
    }

    #[test]
    fn remove_foreign_worktree_without_provenance_errs() {
        let repo = temp_repo();
        let rs = repo.display().to_string();
        // A worktree the user made by hand (no castellynSource mark).
        let foreign = repo.parent().unwrap().join(format!(
            "cast-wt-foreign-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let fs_str = foreign.to_string_lossy().to_string();
        run(&repo, &["worktree", "add", "--no-track", "-b", "manualbr", &fs_str]);
        assert!(worktree_remove(rs.clone(), fs_str.clone()).is_err());
        // It must still be there (we refused).
        assert!(foreign.exists());
        let _ = std::fs::remove_dir_all(&foreign);
        cleanup(&repo);
    }

    #[test]
    fn unmerged_commit_preserves_branch() {
        let repo = temp_repo();
        let rs = repo.display().to_string();
        let c = worktree_create(rs.clone(), "work".into()).unwrap();
        let wt = PathBuf::from(&c.path);
        std::fs::write(wt.join("b.txt"), "new").unwrap();
        run(&wt, &["add", "."]);
        run(&wt, &["commit", "-q", "-m", "agent work"]);
        // Committed → working tree clean, but the branch now carries an unmerged commit.
        assert!(worktree_is_clean(c.path.clone()));
        let r = worktree_remove(rs.clone(), c.path.clone()).unwrap();
        assert_eq!(r.preserved_branch.as_deref(), Some("castellyn/work"));
        assert!(branch_exists(&repo, "castellyn/work"));
        cleanup(&repo);
    }

    #[test]
    fn dangerous_paths_refused() {
        let repo = temp_repo();
        let rs = repo.display().to_string();
        // path == repo (the main worktree)
        assert!(worktree_remove(rs.clone(), rs.clone()).is_err());
        // parent of the repo — unregistered AND dangerous (contains the repo)
        let parent = repo.parent().unwrap().to_string_lossy().to_string();
        assert!(worktree_remove(rs.clone(), parent).is_err());
        cleanup(&repo);
    }
}
