//! ===================== Ф2-GC: stack garbage collector (Home card) =====================

use serde::Serialize;

use crate::i18n::tr;
use crate::{
    cur_lang, has_reparse_child, is_reparse_point, parse_json_bom, profile_names, recycle_path,
};

// --- Ф2-GC: stack garbage collector (Home card) ---
//
// Plugin updates leave junk behind in the physical plugin store
// (~\.claude\plugins\cache\<org>\<plugin>\<ver>): superseded versions, `temp_git_*` clone
// leftovers, `.bak` copies, and foreign-OS (darwin/linux) binaries. read_gc_scan itemizes them;
// run_gc_delete recycles the deletable ones (never active versions, wrong_os is report-only).
// Profile `plugins` dirs are junctions into the one physical store, so we scan physical stores only.

/// One garbage item. Field names ARE the IPC contract (no serde rename).
#[derive(Serialize, Clone)]
pub struct GcItem {
    /// Stable id: "stale:<org>/<plugin>/<ver>" | "tempgit:<dir>" | "bak:<name>" | "wrongos:<store>".
    id: String,
    /// "stale_version" | "temp_git" | "bak" | "wrong_os" (derived from the id prefix).
    category: String,
    label: String,
    path: String,
    size_bytes: u64,
    /// wrong_os is report-only (false); the rest are recyclable.
    deletable: bool,
}

#[derive(Serialize)]
pub struct GcDeleteReport {
    /// ids actually recycled.
    deleted: Vec<String>,
    /// (id, short english reason) for each id we refused / failed to delete.
    skipped: Vec<(String, String)>,
    freed_bytes: u64,
}

/// Map an id prefix to its category. Single source of truth used at item construction. Unit-tested.
fn gc_id_category(id: &str) -> Option<&'static str> {
    if id.starts_with("stale:") {
        Some("stale_version")
    } else if id.starts_with("tempgit:") {
        Some("temp_git")
    } else if id.starts_with("bak:") {
        Some("bak")
    } else if id.starts_with("wrongos:") {
        Some("wrong_os")
    } else {
        None
    }
}

fn gc_item(id: String, label: String, path: String, size_bytes: u64, deletable: bool) -> GcItem {
    let category = gc_id_category(&id).unwrap_or("").to_string();
    GcItem { category, id, label, path, size_bytes, deletable }
}

/// DirEntry list for a dir, or empty on any IO error (a per-store failure must not abort the scan).
fn gc_read_dir(p: &std::path::Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(p).into_iter().flatten().flatten().collect()
}

/// Physical plugin stores: `~\.claude\plugins` + `~\.claude-<name>\plugins` per profile, minus any
/// that are missing or reparse points (profile stores junction into the canonical one — skip so we
/// count each physical tree exactly once).
fn gc_stores(home: &str) -> Vec<(String, std::path::PathBuf)> {
    let mut names = vec![".claude".to_string()];
    names.extend(profile_names().into_iter().map(|n| format!(".claude-{n}")));
    let mut out = Vec::new();
    for n in &names {
        let p = std::path::Path::new(home).join(n).join("plugins");
        if p.is_dir() && !is_reparse_point(&p) {
            out.push((n.clone(), p));
        }
    }
    out
}

/// Recursive byte size, depth-capped, reparse points NOT dereferenced (counted as 0), read errors
/// skipped. A file path returns its own length.
fn gc_dir_size(path: &std::path::Path) -> u64 {
    fn walk(p: &std::path::Path, depth: u32) -> u64 {
        if depth > 32 {
            return 0;
        }
        let Ok(md) = p.symlink_metadata() else {
            return 0;
        };
        if md.file_type().is_file() {
            return md.len();
        }
        if is_reparse_point(p) || !md.is_dir() {
            return 0;
        }
        gc_read_dir(p).iter().map(|e| walk(&e.path(), depth + 1)).sum()
    }
    walk(path, 0)
}

/// Extract `(org, plugin, ver)` from an installPath's tail after `...\plugins\cache\`. Marker match
/// is case-insensitive and slash-agnostic; components keep their original case. None if the marker
/// is absent or fewer than three components follow. Unit-tested.
fn install_path_tuple(install_path: &str) -> Option<(String, String, String)> {
    let norm = install_path.replace('/', "\\");
    let lower = norm.to_ascii_lowercase();
    let marker = "\\plugins\\cache\\";
    let pos = lower.find(marker)?;
    let tail = &norm[pos + marker.len()..];
    let mut comps = tail.split('\\').filter(|s| !s.is_empty());
    let org = comps.next()?.to_string();
    let plugin = comps.next()?.to_string();
    let ver = comps.next()?.to_string();
    Some((org, plugin, ver))
}

type GcActiveSets = (
    std::collections::HashSet<(String, String)>,
    std::collections::HashSet<(String, String, String)>,
    // Dirnames of stores whose installed_plugins.json was read+parsed OK. Stale detection is
    // limited to these: an unreadable manifest means this store's active versions were never
    // blessed, and a blessing from ANOTHER store would then flag them as stale (fail-closed).
    std::collections::HashSet<String>,
);

/// Global active-version sets (lowercased) over every store's installed_plugins.json:
/// (org, plugin) pairs and (org, plugin, ver) triples. installPaths point through any profile alias,
/// so the version is blessed across all physical stores.
fn gc_active_sets(stores: &[(String, std::path::PathBuf)]) -> GcActiveSets {
    let mut pairs = std::collections::HashSet::new();
    let mut triples = std::collections::HashSet::new();
    let mut manifest_ok = std::collections::HashSet::new();
    for (dirname, store) in stores {
        let Ok(txt) = std::fs::read_to_string(store.join("installed_plugins.json")) else {
            continue;
        };
        let Ok(v) = parse_json_bom(&txt) else {
            continue;
        };
        manifest_ok.insert(dirname.clone());
        let Some(plugins) = v.get("plugins").and_then(|p| p.as_object()) else {
            continue;
        };
        for arr in plugins.values() {
            let Some(entries) = arr.as_array() else {
                continue;
            };
            for e in entries {
                if let Some(ip) = e.get("installPath").and_then(|x| x.as_str()) {
                    if let Some((org, plugin, ver)) = install_path_tuple(ip) {
                        let o = org.to_ascii_lowercase();
                        let p = plugin.to_ascii_lowercase();
                        let ver = ver.to_ascii_lowercase();
                        pairs.insert((o.clone(), p.clone()));
                        triples.insert((o, p, ver));
                    }
                }
            }
        }
    }
    (pairs, triples, manifest_ok)
}

/// A version dir is stale iff its (org, plugin) has SOME active version but THIS version isn't it.
/// A pair with no active version at all is left alone (conservative). Case-insensitive. Unit-tested.
fn is_stale_ver(
    org: &str,
    plugin: &str,
    ver: &str,
    pairs: &std::collections::HashSet<(String, String)>,
    triples: &std::collections::HashSet<(String, String, String)>,
) -> bool {
    let o = org.to_ascii_lowercase();
    let p = plugin.to_ascii_lowercase();
    let key3 = (o.clone(), p.clone(), ver.to_ascii_lowercase());
    pairs.contains(&(o, p)) && !triples.contains(&key3)
}

/// True if `token` appears in `name_lower` as a whole token — bounded by start/end of string or one
/// of `-_.` on each side. `token` and `name_lower` must be ASCII-lowercased. Unit-tested.
fn token_bounded(name_lower: &str, token: &str) -> bool {
    let bytes = name_lower.as_bytes();
    let mut from = 0;
    while let Some(rel) = name_lower[from..].find(token) {
        let start = from + rel;
        let end = start + token.len();
        let before_ok = start == 0 || matches!(bytes[start - 1], b'-' | b'_' | b'.');
        let after_ok = end == bytes.len() || matches!(bytes[end], b'-' | b'_' | b'.');
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Foreign-OS binary name? (`darwin`/`linux` as a bounded token, case-insensitive).
fn has_os_token(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    token_bounded(&lower, "darwin") || token_bounded(&lower, "linux")
}

/// Recursive walk of a cache dir summing foreign-OS entries; does not descend into a matched
/// subtree. Returns (total_bytes, entry_count).
fn gc_scan_wrong_os(cache: &std::path::Path) -> (u64, u64) {
    fn walk(dir: &std::path::Path, depth: u32, bytes: &mut u64, count: &mut u64) {
        if depth > 32 {
            return;
        }
        for e in gc_read_dir(dir) {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if has_os_token(&name) {
                *bytes += gc_dir_size(&p);
                *count += 1;
                continue; // matched: don't descend into its subtree
            }
            if p.is_dir() && !is_reparse_point(&p) {
                walk(&p, depth + 1, bytes, count);
            }
        }
    }
    let mut bytes = 0;
    let mut count = 0;
    walk(cache, 0, &mut bytes, &mut count);
    (bytes, count)
}

/// Scan all physical stores for garbage. Never errors: a per-store IO failure yields fewer items.
fn gc_scan(home: &str) -> Vec<GcItem> {
    let stores = gc_stores(home);
    let (pairs, triples, manifest_ok) = gc_active_sets(&stores);
    let mut items = Vec::new();
    for (dirname, store) in &stores {
        let cache = store.join("cache");

        // 1. stale versions: cache\<org>\<plugin>\<ver> (exactly 3 levels). Only in stores whose
        // own manifest was readable — otherwise this store's active versions were never blessed
        // and another store's blessing of the same pair would flag them (fail-closed).
        for org_e in gc_read_dir(&cache) {
            if !manifest_ok.contains(dirname) {
                break;
            }
            let org_p = org_e.path();
            if !org_p.is_dir() || is_reparse_point(&org_p) {
                continue;
            }
            let org = org_e.file_name().to_string_lossy().to_string();
            for pl_e in gc_read_dir(&org_p) {
                let pl_p = pl_e.path();
                if !pl_p.is_dir() || is_reparse_point(&pl_p) {
                    continue;
                }
                let plugin = pl_e.file_name().to_string_lossy().to_string();
                for ver_e in gc_read_dir(&pl_p) {
                    let ver_p = ver_e.path();
                    if !ver_p.is_dir() || is_reparse_point(&ver_p) {
                        continue;
                    }
                    let ver = ver_e.file_name().to_string_lossy().to_string();
                    if is_stale_ver(&org, &plugin, &ver, &pairs, &triples) {
                        items.push(gc_item(
                            // Qualify with the store dirname: without it two profiles' caches can
                            // produce the same id and run_gc_delete's first-match .find() resolves the
                            // wrong item (or misses one).
                            format!("stale:{dirname}:{org}/{plugin}/{ver}"),
                            format!("{plugin} {ver} ({org})"),
                            ver_p.to_string_lossy().into_owned(),
                            gc_dir_size(&ver_p),
                            true,
                        ));
                    }
                }
            }
        }

        // 2. temp_git_* leftover clone dirs at cache top-level.
        for e in gc_read_dir(&cache) {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            if name.starts_with("temp_git_") && p.is_dir() {
                items.push(gc_item(
                    format!("tempgit:{dirname}:{name}"),
                    name.clone(),
                    p.to_string_lossy().into_owned(),
                    gc_dir_size(&p),
                    true,
                ));
            }
        }

        // 3. .bak files/dirs at store and cache top-levels (case-insensitive). Suffix match only:
        // a bare `contains(".bak")` would also flag `.backup`, `x.bak2`, `x.bak.zip` for deletion.
        for (base_tag, base) in [("store", store.as_path()), ("cache", cache.as_path())] {
            for e in gc_read_dir(base) {
                let name = e.file_name().to_string_lossy().to_string();
                if name.to_ascii_lowercase().ends_with(".bak") {
                    let p = e.path();
                    let size = if p.is_dir() {
                        gc_dir_size(&p)
                    } else {
                        p.symlink_metadata().map(|m| m.len()).unwrap_or(0)
                    };
                    items.push(gc_item(
                        // Qualify with store dirname + which base (store vs cache) so same-named .bak
                        // entries across profiles/bases don't collide on run_gc_delete's first-match find.
                        format!("bak:{dirname}:{base_tag}:{name}"),
                        name.clone(),
                        p.to_string_lossy().into_owned(),
                        size,
                        true,
                    ));
                }
            }
        }

        // 4. wrong_os aggregate (report-only): one item per store.
        let (wrong_bytes, wrong_count) = gc_scan_wrong_os(&cache);
        if wrong_count > 0 {
            items.push(gc_item(
                format!("wrongos:{dirname}"),
                format!("{wrong_count} darwin/linux entries"),
                store.to_string_lossy().into_owned(),
                wrong_bytes,
                false,
            ));
        }
    }
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    items
}

/// Itemize stack garbage across physical plugin stores. Err only when USERPROFILE is unset.
#[tauri::command]
pub async fn read_gc_scan() -> Result<Vec<GcItem>, String> {
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    tokio::task::spawn_blocking(move || gc_scan(&home))
        .await
        .map_err(|e| e.to_string())
}

/// Recycle the requested gc ids. Defense-in-depth (the UI already confirmed): a fresh rescan must
/// still list the id, it must be deletable, live under a physical `plugins` store, not be a reparse
/// point (nor, for dirs, contain reparse children), and actually vanish after the recycle call.
#[tauri::command]
pub async fn run_gc_delete(ids: Vec<String>) -> Result<GcDeleteReport, String> {
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    tokio::task::spawn_blocking(move || {
        let stores = gc_stores(&home);
        let items = gc_scan(&home);
        let mut report = GcDeleteReport {
            deleted: Vec::new(),
            skipped: Vec::new(),
            freed_bytes: 0,
        };
        for id in ids {
            let Some(item) = items.iter().find(|i| i.id == id) else {
                report.skipped.push((id, "not found on rescan".into()));
                continue;
            };
            if !item.deletable {
                report.skipped.push((id, "not deletable".into()));
                continue;
            }
            let path = std::path::Path::new(&item.path);
            // Must be strictly inside a known physical plugins store (component-wise prefix).
            let under_store = stores
                .iter()
                .any(|(_, sp)| path.starts_with(sp) && path != sp.as_path());
            if !under_store {
                report.skipped.push((id, "path outside plugins store".into()));
                continue;
            }
            if is_reparse_point(path) {
                report.skipped.push((id, "target is a reparse point".into()));
                continue;
            }
            let Ok(md) = path.symlink_metadata() else {
                report.skipped.push((id, "target vanished".into()));
                continue;
            };
            let is_dir = md.file_type().is_dir();
            if is_dir && has_reparse_child(path) {
                report.skipped.push((id, "contains reparse children".into()));
                continue;
            }
            // One call for both: the shell operation recycles a file and a whole subtree
            // identically, so `is_dir` no longer selects between two code paths.
            if let Err(e) = recycle_path(&item.path) {
                report.skipped.push((id, e));
                continue;
            }
            if path.exists() {
                report.skipped.push((id, "still present after recycle".into()));
                continue;
            }
            report.freed_bytes += item.size_bytes;
            report.deleted.push(id);
        }
        report
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stale_ver_classifies_only_old_versions_of_installed_plugins() {
        // gc deletability classification (L138): an OLD version of an INSTALLED plugin is stale; the
        // active version and any uninstalled plugin's dirs are kept.
        let pairs: std::collections::HashSet<(String, String)> =
            [("acme".to_string(), "widget".to_string())].into_iter().collect();
        let triples: std::collections::HashSet<(String, String, String)> =
            [("acme".to_string(), "widget".to_string(), "2.0.0".to_string())]
                .into_iter()
                .collect();
        assert!(is_stale_ver("acme", "widget", "1.0.0", &pairs, &triples)); // old → stale
        assert!(!is_stale_ver("acme", "widget", "2.0.0", &pairs, &triples)); // active → keep
        assert!(!is_stale_ver("other", "thing", "1.0.0", &pairs, &triples)); // uninstalled → keep
        assert!(is_stale_ver("ACME", "Widget", "1.0.0", &pairs, &triples)); // case-insensitive
    }

    #[test]
    fn gc_install_path_tuple_parses_alias_and_slashes() {
        // A different profile's alias in the path is stripped down to (org, plugin, ver).
        assert_eq!(
            install_path_tuple(r"C:\Users\X\.claude-cc1\plugins\cache\org\pl\1.0.0"),
            Some(("org".into(), "pl".into(), "1.0.0".into()))
        );
        // Forward slashes tolerated.
        assert_eq!(
            install_path_tuple("C:/Users/X/.claude/plugins/cache/max-marketplace/max/1.9.0"),
            Some(("max-marketplace".into(), "max".into(), "1.9.0".into()))
        );
        // "unknown" is a valid version dir.
        assert_eq!(
            install_path_tuple(r"C:\u\.claude\plugins\cache\o\p\unknown"),
            Some(("o".into(), "p".into(), "unknown".into()))
        );
        // Marker absent / too few components → None.
        assert_eq!(install_path_tuple(r"C:\somewhere\else"), None);
        assert_eq!(install_path_tuple(r"C:\u\.claude\plugins\cache\o\p"), None);
    }

    #[test]
    fn gc_stale_detect() {
        use std::collections::HashSet;
        let mut pairs = HashSet::new();
        let mut triples = HashSet::new();
        pairs.insert(("max-marketplace".to_string(), "max".to_string()));
        triples.insert((
            "max-marketplace".to_string(),
            "max".to_string(),
            "1.9.0".to_string(),
        ));
        // active pair, different ver dir → stale.
        assert!(is_stale_ver(
            "max-marketplace",
            "max",
            "1.7.0",
            &pairs,
            &triples
        ));
        // the active ver itself → not stale (case-insensitive).
        assert!(!is_stale_ver(
            "Max-Marketplace",
            "Max",
            "1.9.0",
            &pairs,
            &triples
        ));
        // pair not in active-set at all → left alone (conservative).
        assert!(!is_stale_ver("other", "plugin", "2.0.0", &pairs, &triples));
    }

    #[test]
    fn gc_os_token_matcher() {
        assert!(has_os_token("node-linux-x64"));
        assert!(has_os_token("linux"));
        assert!(has_os_token("foo.darwin.node"));
        assert!(has_os_token("linux-notes.md"));
        assert!(!has_os_token("mylinuxish"));
        assert!(!has_os_token("darwinism")); // no trailing boundary
        assert!(!has_os_token("windows-x64"));
    }

    #[test]
    fn gc_id_category_roundtrip() {
        assert_eq!(gc_id_category("stale:org/pl/1.0.0"), Some("stale_version"));
        assert_eq!(gc_id_category("tempgit:temp_git_abc"), Some("temp_git"));
        assert_eq!(gc_id_category("bak:foo.bak"), Some("bak"));
        assert_eq!(gc_id_category("wrongos:.claude"), Some("wrong_os"));
        assert_eq!(gc_id_category("mystery:x"), None);
        // Constructor derives the category field from the id prefix.
        let it = gc_item("bak:x.bak".into(), "x.bak".into(), "p".into(), 3, true);
        assert_eq!(it.category, "bak");
    }

    #[test]
    fn gc_dir_size_sums_tree() {
        use std::io::Write;
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("castellyn_gc_{pid}"));
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::File::create(root.join("a.txt"))
            .unwrap()
            .write_all(&[0u8; 100])
            .unwrap();
        std::fs::File::create(sub.join("b.txt"))
            .unwrap()
            .write_all(&[0u8; 50])
            .unwrap();
        assert_eq!(gc_dir_size(&root), 150);
        // A single file path returns its own length.
        assert_eq!(gc_dir_size(&root.join("a.txt")), 100);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Manual live smoke over the REAL plugin stores (read-only). Not part of the gates:
    /// `cargo test gc_scan_live_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn gc_scan_live_smoke() {
        let Ok(home) = std::env::var("USERPROFILE") else {
            return;
        };
        let items = gc_scan(&home);
        for i in &items {
            println!(
                "{:<14} {:>12} bytes  deletable={}  {}  [{}]",
                i.category, i.size_bytes, i.deletable, i.label, i.path
            );
        }
        // Every id must map to a known category and wrong_os must be report-only.
        assert!(items.iter().all(|i| gc_id_category(&i.id) == Some(i.category.as_str())));
        assert!(items.iter().filter(|i| i.category == "wrong_os").all(|i| !i.deletable));
    }
}
