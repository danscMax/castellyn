//! Opt-in liveness probe for a configured MCP server.
//!
//! `read_mcp` answers "is this server CONFIGURED and deployed" — it never contacts one, so a server
//! whose command is missing, whose path rotted, or which crashes at startup looks exactly like a
//! healthy one. This module answers the other question by doing the real thing: spawn the server,
//! complete the MCP `initialize` handshake, list its tools, then kill it.
//!
//! **Never automatic.** A probe starts a real process (often `npx` → a package download), so it is
//! only ever triggered by an explicit user action — opening the MCP tab probes nothing.

use std::process::Stdio;
use std::time::{Duration, Instant};

use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde::Serialize;
use tokio::io::AsyncReadExt;
// `creation_flags` here is tokio's own inherent Windows method on `tokio::process::Command`,
// not the `std::os::windows::process::CommandExt` trait the rest of the backend imports.
use tokio::process::Command;

use crate::i18n::trv;
use crate::{canonical_mcp_servers, cur_lang, tr, CREATE_NO_WINDOW};

/// Whole budget for one probe: spawn + initialize + tools/list. Deliberately generous — a cold
/// `npx -y <pkg>` resolves and unpacks the package on its first run, and a false "broken" on every
/// fresh machine is a worse failure than waiting. A hung server costs exactly this much and no more.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Hard cap on how much stderr we buffer from a probed server, and how many of its trailing
/// characters end up in the reported reason (the tail is where the fatal error is).
const STDERR_CAP: u64 = 64 * 1024;
const STDERR_TAIL: usize = 300;

#[derive(Serialize)]
pub struct McpProbeResult {
    pub name: String,
    pub ok: bool,
    /// `name version` as the server reported it in its initialize response; empty when unreachable.
    pub server: String,
    pub tools: usize,
    /// Localized reason for `ok: false`; empty when ok.
    pub error: String,
    pub ms: u64,
}

/// Launch one canonical MCP server, handshake with it, and report reachable / broken with a reason.
///
/// Takes only the server NAME: the command line is read from `config\.mcp.json` here, so the
/// frontend can never hand the backend an arbitrary process to spawn.
#[tauri::command]
pub async fn probe_mcp(name: String) -> Result<McpProbeResult, String> {
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    let servers = canonical_mcp_servers(&home)?;
    let def = servers
        .get(&name)
        .ok_or_else(|| trv("err.mcp_no_such_server", cur_lang(), &[("name", &name)]))?;
    let program = def
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    if program.is_empty() {
        return Err(trv("err.mcp_no_command", cur_lang(), &[("name", &name)]));
    }

    // Resolve through PATH ourselves. npm installs its CLIs as `<name>` + `<name>.cmd` + `<name>.ps1`
    // shims, and CreateProcess only ever appends `.exe` — a bare `cclsp`/`codex` would come back
    // "program not found" for a server that works perfectly under Claude Code (whose Node spawn does
    // the PATHEXT walk). `exe_on_path` picks the .exe/.cmd/.bat that Rust can actually launch.
    let launch = (!program.contains(['\\', '/']))
        .then(|| crate::exe_on_path(program))
        .flatten()
        .unwrap_or_else(|| std::path::PathBuf::from(program));
    let mut cmd = Command::new(&launch);
    for a in def.get("args").and_then(|a| a.as_array()).into_iter().flatten() {
        if let Some(s) = a.as_str() {
            cmd.arg(s);
        }
    }
    for (k, v) in def.get("env").and_then(|e| e.as_object()).into_iter().flatten() {
        if let Some(s) = v.as_str() {
            cmd.env(k, s);
        }
    }
    // Canon (CLAUDE.md): every spawn sets a window-creation flag, or a black console flashes in
    // front of the GUI. rmcp spawns the `tokio::process::Command` we hand it verbatim, so setting
    // the flag here is what makes the probe silent.
    cmd.creation_flags(CREATE_NO_WINDOW);

    let start = Instant::now();
    // stderr piped, not rmcp's default `inherit`: a server that dies at startup prints the only
    // useful diagnostic there ("Cannot find module …"), and inheriting would dump it into the GUI
    // process instead. Piped means it MUST be drained — see the reader task below.
    let spawned = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn();
    let (child, stderr) = match spawned {
        Ok(pair) => pair,
        // A missing/unresolvable command is the single most common breakage and is a RESULT, not a
        // command failure — "check all" must render it in the row like any other broken server.
        Err(e) => {
            return Ok(McpProbeResult {
                name,
                ok: false,
                server: String::new(),
                tools: 0,
                error: trv("err.mcp_probe_spawn", cur_lang(), &[("e", &e)]),
                ms: start.elapsed().as_millis() as u64,
            })
        }
    };
    let pid = child.id();
    // Same kill-on-close Job Object every other spawn joins: if the app dies mid-probe, the server
    // dies with it instead of orphaning.
    if let Some(p) = pid {
        crate::assign_to_kill_job(p);
    }
    // Drain stderr from the start — an undrained pipe wedges a chatty server at the pipe buffer,
    // which would look like a hang. Capped, and collected only after the kill guarantees EOF.
    let stderr_reader = stderr.map(|e| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = AsyncReadExt::read_to_end(&mut AsyncReadExt::take(e, STDERR_CAP), &mut buf).await;
            buf
        })
    });

    let outcome = tokio::time::timeout(PROBE_TIMEOUT, async {
        let svc = ().serve(child).await.map_err(|e| e.to_string())?;
        let server = svc
            .peer_info()
            .map(|i| format!("{} {}", i.server_info.name, i.server_info.version))
            .unwrap_or_default();
        let tools = svc.list_all_tools().await.map_err(|e| e.to_string())?.len();
        let _ = svc.cancel().await;
        Ok::<(String, usize), String>((server, tools))
    })
    .await;

    // Kill the TREE, on every path including success. rmcp's own cleanup kills the direct child
    // only — for `cmd /c npx …` that is the shim, and the real node server would survive it.
    if let Some(p) = pid {
        let _ = tokio::task::spawn_blocking(move || crate::kill_tree(p)).await;
    }

    let ms = start.elapsed().as_millis() as u64;
    let (ok, server, tools, error) = match outcome {
        Ok(Ok((server, tools))) => (true, server, tools, String::new()),
        // "connection closed" alone is not a reason a user can act on; the server's own last words
        // usually are. The kill above closed the pipe, so the reader has already hit EOF.
        Ok(Err(e)) => {
            let mut why = e;
            if let Some(h) = stderr_reader {
                if let Ok(buf) = h.await {
                    let text = String::from_utf8_lossy(&buf);
                    let tail = text.trim();
                    if !tail.is_empty() {
                        // Last STDERR_TAIL *characters* — a byte offset could split a UTF-8 sequence.
                        let from = tail
                            .char_indices()
                            .rev()
                            .nth(STDERR_TAIL)
                            .map_or(0, |(i, _)| i);
                        why = format!("{why} — {}", tail[from..].trim());
                    }
                }
            }
            (
                false,
                String::new(),
                0,
                trv("err.mcp_probe_failed", cur_lang(), &[("e", &why)]),
            )
        }
        Err(_) => (
            false,
            String::new(),
            0,
            trv(
                "err.mcp_probe_timeout",
                cur_lang(),
                &[("s", &PROBE_TIMEOUT.as_secs())],
            ),
        ),
    };
    Ok(McpProbeResult {
        name,
        ok,
        server,
        tools,
        error,
        ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three outcomes must be distinguishable, and none of them may leave a process behind.
    /// `cmd /c` shims are what the real config uses, so the fixtures use the same shape.
    #[tokio::test]
    async fn probe_reports_missing_command_without_hanging() {
        // A command that cannot be spawned resolves fast and reports, rather than erroring out.
        let mut cmd = Command::new("castellyn-no-such-mcp-server.exe");
        cmd.creation_flags(CREATE_NO_WINDOW);
        let spawned = TokioChildProcess::builder(cmd).stderr(Stdio::null()).spawn();
        assert!(spawned.is_err(), "a nonexistent binary must fail to spawn");
    }

    /// A server that starts but never answers the handshake must be cut off by the timeout, not
    /// wait forever. Uses a 3s budget so the test is cheap; the production ceiling is PROBE_TIMEOUT.
    #[tokio::test]
    async fn silent_server_is_cut_off_by_the_timeout() {
        let mut cmd = Command::new("powershell.exe");
        // Alive, mute, and long-lived: the exact shape of an MCP server that starts and then wedges.
        // (`cmd /c pause` does NOT work as a fixture — with stdin redirected it returns immediately.)
        cmd.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
        cmd.creation_flags(CREATE_NO_WINDOW);
        let (child, _) = TokioChildProcess::builder(cmd)
            .stderr(Stdio::null())
            .spawn()
            .expect("powershell.exe must spawn");
        let pid = child.id().expect("a spawned child has a pid");
        let started = Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(3), async {
            ().serve(child).await.map_err(|e| e.to_string())
        })
        .await;
        assert!(outcome.is_err(), "a mute server must hit the timeout");
        assert!(started.elapsed() < Duration::from_secs(15), "timeout did not fire promptly");
        let _ = tokio::task::spawn_blocking(move || crate::kill_tree(pid)).await;
        assert_eq!(
            crate::pid_alive(pid),
            Some(false),
            "the probed server survived its own timeout"
        );
    }

    /// Live smoke: probe every server in the REAL `config\.mcp.json` and print the verdicts.
    /// `#[ignore]` because it launches actual MCP servers (and downloads packages on a cold `npx`) —
    /// that belongs in a human-run check, not in the per-commit gate.
    /// `cargo test mcp_probe_live_smoke -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn mcp_probe_live_smoke() {
        let home = std::env::var("USERPROFILE").expect("USERPROFILE");
        let servers = canonical_mcp_servers(&home).expect("config/.mcp.json");
        for name in servers.keys() {
            match probe_mcp(name.clone()).await {
                Ok(r) => println!(
                    "{:<12} ok={:<5} tools={:<3} {:>6}ms  {}{}",
                    r.name, r.ok, r.tools, r.ms, r.server, r.error
                ),
                Err(e) => println!("{name:<12} ERR {e}"),
            }
        }
    }
}
