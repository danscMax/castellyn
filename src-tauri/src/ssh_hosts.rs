//! ===================== SSH host registry (config\sshhosts.json + ~/.ssh/config import) =====================
//! Saved hosts live in a synced JSON under SCRIPTS_ROOT (same pattern as myproviders.json); NO secrets
//! are stored — auth uses the system `ssh` + the user's ~/.ssh (keys/known_hosts/ControlMaster). The
//! `read_ssh_hosts` command also surfaces hosts parsed read-only from the machine's ~/.ssh/config
//! (source="sshconfig") so existing SSH setup is reused (DRY). An ssh session is launched via the
//! normal session_spawn with tool="ssh" and the target carried in `args` (e.g. "user@host -p 22").

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::i18n::trv;
use crate::{abs, cur_lang, gen_session_id, parse_json_bom, write_json_atomic};

const SSHHOSTS_CONFIG_REL: &str = "{{PROFILES}}\\config\\sshhosts.json";
static SSHHOSTS_LOCK: Mutex<()> = Mutex::new(());

fn default_ssh_source() -> String {
    "saved".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHost {
    #[serde(default)]
    id: String,
    name: String,
    host: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_path: Option<String>,
    // Optional remote start directory: on connect we `Set-Location` into it (Windows/PowerShell remote).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_dir: Option<String>,
    #[serde(default = "default_ssh_source")]
    source: String, // "saved" | "sshconfig"
}

fn read_ssh_hosts_saved() -> Vec<SshHost> {
    std::fs::read_to_string(abs(SSHHOSTS_CONFIG_REL))
        .ok()
        .and_then(|c| parse_json_bom(&c).ok())
        .and_then(|v| v.get("hosts").and_then(|p| p.as_array()).cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|e| serde_json::from_value::<SshHost>(e).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn write_ssh_hosts_saved(list: &[SshHost]) -> Result<(), String> {
    let path = abs(SSHHOSTS_CONFIG_REL);
    let v = serde_json::json!({ "schemaVersion": 1, "hosts": list });
    let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_json_atomic(&path, &json)
        .map_err(|e| trv("err.fs_write", cur_lang(), &[("path", &"sshhosts.json"), ("e", &e)]))
}

/// Strip ONE pair of surrounding double/single quotes (OpenSSH allows quoted values, e.g. an
/// IdentityFile path with spaces). Leaves unquoted strings untouched.
fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Parse `~/.ssh/config` text into hosts (read-only). Honors Host (skips wildcard/negated patterns),
/// HostName, User, Port, IdentityFile. A `Host a b` line is ONE host (named after its first concrete
/// alias). Best-effort: unknown keywords ignored; tokens split on whitespace or '=' (OpenSSH accepts both).
/// `Include` directives are spliced in by read_ssh_config_hosts before this runs (this fn is pure text).
fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut out: Vec<SshHost> = Vec::new();
    let mut cur: Option<usize> = None; // out-index of the current Host block (None for wildcard-only blocks)
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `split_once` cuts on char boundaries. The old `find(..)` + `line[i + 1..]` assumed the
        // separator was one byte, but `char::is_whitespace()` is true for NBSP (U+00A0), U+2000-200A
        // and U+3000 — a `~/.ssh/config` pasted from a web page panics the byte slice mid-char. Same
        // class as the `expand_ssh_config` panic fixed via `str::get`; this sibling parser was missed.
        let (key, val) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, rest)) => (
                k.trim(),
                rest.trim_matches(|c: char| c.is_whitespace() || c == '=').trim(),
            ),
            None => (line, ""),
        };
        if key.eq_ignore_ascii_case("host") {
            // `Host a b c` lists alternative match patterns for ONE host — use the first concrete alias
            // as its name (extra aliases aren't separate machines; one-per-alias used to dupe them).
            cur = val
                .split_whitespace()
                .find(|a| !a.contains(['*', '?', '!']))
                .map(|alias| {
                    out.push(SshHost {
                        id: format!("cfg:{alias}"),
                        name: alias.to_string(),
                        host: alias.to_string(), // replaced by HostName if the block has one
                        port: None,
                        user: None,
                        key_path: None,
                        remote_dir: None,
                        source: "sshconfig".into(),
                    });
                    out.len() - 1
                });
        } else if let Some(i) = cur {
            match key.to_ascii_lowercase().as_str() {
                "hostname" => out[i].host = unquote(val).to_string(),
                "user" => out[i].user = Some(unquote(val).to_string()),
                "port" => out[i].port = unquote(val).parse::<u16>().ok(),
                "identityfile" => out[i].key_path = Some(unquote(val).to_string()),
                _ => {}
            }
        }
    }
    out
}

fn read_ssh_config_hosts() -> Vec<SshHost> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let main = format!("{home}\\.ssh\\config");
    let mut text = String::new();
    expand_ssh_config(std::path::Path::new(&main), &home, 0, &mut text);
    parse_ssh_config(&text)
}

/// Inline `Include` directives into one config blob (OpenSSH semantics: the included file's contents
/// are spliced in at that point), so hosts defined in included files (a common `~/.ssh/config.d/*`
/// layout) are no longer silently dropped. Bounded recursion guards against include cycles.
fn expand_ssh_config(path: &std::path::Path, home: &str, depth: u8, out: &mut String) {
    if depth > 16 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let t = line.trim();
        // Match the `Include` keyword followed by whitespace/'=' (not e.g. "IncludeFoo").
        // Slice via `str::get` (not `t[..7]`): a non-ASCII line (e.g. a Cyrillic comment
        // `# рабочий сервер`) whose byte 7 lands inside a multi-byte char would panic on a raw
        // byte-index slice — `get` returns None instead, so the line is just passed through.
        let is_include = t
            .get(..7)
            .is_some_and(|h| h.eq_ignore_ascii_case("include"))
            && t
                .get(7..)
                .is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '='));
        if is_include {
            let patterns = t[7..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
            for pat in patterns.split_whitespace() {
                for f in resolve_ssh_include(unquote(pat), home) {
                    expand_ssh_config(&f, home, depth + 1, out);
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
}

/// Resolve one Include pattern to concrete file paths. Supports `~/`, absolute and ~/.ssh-relative
/// paths, plus a single trailing `*` (the common `config.d/*` = every file directly under that dir).
/// ponytail: no general globbing (no extra dep) — `dir/prefix*` style patterns resolve to nothing.
fn resolve_ssh_include(pat: &str, home: &str) -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let expanded = if let Some(rest) = pat.strip_prefix("~/") {
        format!("{home}\\{rest}")
    } else if std::path::Path::new(pat).is_absolute() {
        pat.to_string()
    } else {
        format!("{home}\\.ssh\\{pat}")
    };
    let expanded = expanded.replace('/', "\\");
    if let Some(prefix) = expanded.strip_suffix('*') {
        let dir = prefix.trim_end_matches('\\');
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        files
    } else {
        vec![PathBuf::from(expanded)]
    }
}

/// Saved hosts (synced registry) merged with read-only hosts imported from ~/.ssh/config, each added
/// only if its host isn't already listed (saved entries win; duplicate config blocks collapse).
#[tauri::command]
pub fn read_ssh_hosts() -> Vec<SshHost> {
    let mut all = read_ssh_hosts_saved();
    let mut seen: std::collections::HashSet<String> =
        all.iter().map(|h| h.host.to_ascii_lowercase()).collect();
    for h in read_ssh_config_hosts() {
        if seen.insert(h.host.to_ascii_lowercase()) {
            all.push(h);
        }
    }
    all
}

/// Create or update a saved host (matched by id); returns the new saved list. No secrets stored.
#[tauri::command]
pub fn save_ssh_host(host: SshHost) -> Result<Vec<SshHost>, String> {
    let _g = SSHHOSTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_ssh_hosts_saved();
    let mut h = host;
    if h.id.trim().is_empty() {
        h.id = gen_session_id();
    }
    h.source = "saved".into();
    match list.iter_mut().find(|x| x.id == h.id) {
        Some(existing) => *existing = h,
        None => list.push(h),
    }
    write_ssh_hosts_saved(&list)?;
    Ok(list)
}

/// Delete a saved host by id; returns the new saved list. (sshconfig-sourced hosts can't be deleted.)
#[tauri::command]
pub fn delete_ssh_host(id: String) -> Result<Vec<SshHost>, String> {
    let _g = SSHHOSTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_ssh_hosts_saved();
    list.retain(|x| x.id != id);
    write_ssh_hosts_saved(&list)?;
    Ok(list)
}

/// Quick reachability probe for the host editor: TCP connect to host:port (default 22), ~2s timeout.
/// Does NOT authenticate — just tells the user the host is reachable before they launch ssh.
/// `(async)`: blocking DNS + TCP connects. As a sync command these ran on the main/event-loop
/// thread, so the frontend's `Promise.allSettled` fan-out over every host serialized AND froze the
/// UI for N×2s; on the blocking pool the probes actually overlap.
#[tauri::command(async)]
pub fn test_ssh_host(host: String, port: Option<u16>) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let p = port.unwrap_or(22);
    match format!("{host}:{p}").to_socket_addrs() {
        // Try EVERY resolved address (short-circuits on the first success). Probing only the first
        // wrongly reported IPv6-first hosts as unreachable when only their IPv4 endpoint was up.
        Ok(addrs) => addrs
            .into_iter()
            .any(|a| TcpStream::connect_timeout(&a, std::time::Duration::from_secs(2)).is_ok()),
        Err(_) => false,
    }
}
#[cfg(test)]
mod ssh_config_tests {
    use super::*;
    #[test]
    fn parses_aliases_fields_and_skips_wildcards() {
        let cfg = "# my hosts\n\
Host minipc 192.168.1.177\n\
    HostName 192.168.1.177\n\
    User dansc\n\
    Port 22\n\
    IdentityFile ~/.ssh/id_ed25519\n\
\n\
Host *\n\
    ForwardAgent yes\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(
            hosts.len(),
            1,
            "multi-alias Host line = one host, wildcard skipped"
        );
        let mini = hosts
            .iter()
            .find(|h| h.name == "minipc")
            .expect("minipc host");
        assert_eq!(mini.host, "192.168.1.177");
        assert_eq!(mini.user.as_deref(), Some("dansc"));
        assert_eq!(mini.port, Some(22));
        assert_eq!(mini.key_path.as_deref(), Some("~/.ssh/id_ed25519"));
        assert!(hosts.iter().all(|h| h.source == "sshconfig"));
    }

    #[test]
    fn strips_surrounding_quotes_from_values() {
        let cfg =
            "Host q\n  HostName \"10.0.0.5\"\n  User 'bob'\n  IdentityFile \"C:/keys/my key\"\n";
        let hosts = parse_ssh_config(cfg);
        let h = hosts.iter().find(|h| h.name == "q").expect("host q");
        assert_eq!(h.host, "10.0.0.5", "double quotes stripped");
        assert_eq!(h.user.as_deref(), Some("bob"), "single quotes stripped");
        assert_eq!(
            h.key_path.as_deref(),
            Some("C:/keys/my key"),
            "quoted path with space kept intact"
        );
    }

    #[test]
    fn expand_ssh_config_survives_cyrillic_line_and_still_detects_include() {
        // Regression: a Cyrillic comment/value whose 7th byte splits a multi-byte char must not
        // panic the byte-index Include check (str::get, not t[..7]). And a real `Include` on a
        // line that also carries Cyrillic elsewhere must still be recognized.
        let dir = std::env::temp_dir().join(format!("castellyn_ssh_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let inc = dir.join("extra.conf");
        std::fs::write(&inc, "Host included\n  HostName 10.9.9.9\n").unwrap();
        let main = dir.join("config");
        // `# рабочий сервер`: byte 7 lands inside a Cyrillic char — the panic repro.
        std::fs::write(
            &main,
            format!(
                "# рабочий сервер\nHost родной\n  HostName 10.0.0.1\nInclude {}\n",
                inc.display()
            ),
        )
        .unwrap();
        let mut out = String::new();
        expand_ssh_config(&main, &dir.to_string_lossy(), 0, &mut out); // must not panic
        assert!(
            out.contains("Host included"),
            "Include directive was resolved and its file spliced in"
        );
        assert!(out.contains("# рабочий сервер"), "Cyrillic line passed through");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_ssh_config_survives_multibyte_whitespace_separator() {
        // Regression (sibling of the expand_ssh_config panic): `char::is_whitespace()` matches NBSP
        // and other multi-byte spaces, so the old `line[i + 1..]` sliced mid-char. A config pasted
        // from a web page carries U+00A0; parsing it must not panic and must still read the value.
        let hosts = parse_ssh_config("Host box\n  HostName\u{00a0}10.0.0.7\n  Port\u{3000}2222\n");
        assert_eq!(hosts.len(), 1, "the Host block is still recognized");
        assert_eq!(hosts[0].host, "10.0.0.7", "HostName parsed across an NBSP separator");
        assert_eq!(hosts[0].port, Some(2222), "Port parsed across an ideographic space");
    }
}
