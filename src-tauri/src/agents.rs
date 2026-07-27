//! ===================== Subagents manager (~/.claude/agents/*.md) =====================

use std::os::windows::process::CommandExt;

use serde::Serialize;

use crate::i18n::{tr, trv};
use crate::{cur_lang, exe_on_path, extract_frontmatter, fm_value, CREATE_NO_WINDOW};

// ---- Subagents manager (~/.claude/agents/*.md) ---------------------------------------------
// Standalone user subagents that Claude Code reads from ~/.claude/agents. Structurally identical to
// skills (frontmatter + body), so the SKILL.md parsers (extract_frontmatter/fm_value) are reused
// verbatim. The `agents` folder is junction-linked into every profile AND Syncthing-synced between
// machines (see ClaudeProfiles\config\profiles.json linkedFolders + sync_item_lines), so a write
// here fans out with no extra code — do NOT add a per-profile copy path.

#[derive(Serialize)]
pub struct AgentInfo {
    name: String,
    description: String,
    model: String,
    tools: String,
    path: String,
}

#[derive(Serialize)]
pub struct AgentDetail {
    name: String,
    description: String,
    model: String,
    tools: String,
    prompt: String,
    path: String,
}

/// ~/.claude/agents — the canonical standalone-subagent dir (mirrors list_skills' ~/.claude/skills).
fn agents_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("USERPROFILE")
        .map_err(|_| tr("err.no_userprofile", cur_lang()).to_string())?;
    Ok(std::path::Path::new(&home).join(".claude").join("agents"))
}

/// Refuse any target whose PARENT isn't the real agents dir — canonicalized so a junctioned dir and
/// path-traversal both resolve honestly (same guard shape as delete_skill).
fn agent_guard(target: &std::path::Path) -> Result<(), String> {
    let dir = agents_dir()?;
    let canon_dir = std::fs::canonicalize(&dir)
        .map_err(|_| trv("err.dir_not_found", cur_lang(), &[("path", &dir.display())]))?;
    let parent = target
        .parent()
        .ok_or_else(|| tr("err.bad_path", cur_lang()).to_string())?;
    let canon_parent = std::fs::canonicalize(parent)
        .map_err(|_| tr("err.bad_path", cur_lang()).to_string())?;
    if canon_parent != canon_dir {
        return Err(tr("err.bad_path", cur_lang()).into());
    }
    Ok(())
}

/// Body after the frontmatter's closing `---` (leading blank lines trimmed). No frontmatter → the
/// whole file is the body. Tolerant of both `\n` and `\r\n` (the `\n---` match ignores a leading \r).
fn frontmatter_body(content: &str) -> String {
    let t = content.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            // Skip the closing "---", then the rest of that line, then leading blank lines.
            let after = &rest[end + 4..];
            let after = after.split_once('\n').map(|(_, b)| b).unwrap_or("");
            return after.trim_start_matches(['\r', '\n']).to_string();
        }
    }
    content.to_string()
}

/// ASCII kebab-case slug for the .md filename. Non-ASCII/empty → "agent" (the display `name:`
/// frontmatter still carries the user's text; Claude Code identifies a subagent by `name`, not file).
fn slugify_agent(name: &str) -> String {
    let slug = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() { "agent".into() } else { slug }
}

/// A single-line YAML scalar, double-quoted only when the plain form would be invalid or would mean
/// something else. `description: Use when: X` is the common real case — a plain scalar containing
/// `: ` makes the whole frontmatter unparseable, and Claude Code then cannot load the subagent, while
/// Castellyn's own naive `fm_value` reader (which strips surrounding quotes) reports it healthy either
/// way. Left unquoted otherwise, matching the ecosystem convention for these files.
fn yaml_scalar(v: &str) -> String {
    let hostile = v.contains(": ")
        || v.ends_with(':')
        || v.contains(" #")
        || v.starts_with(['#', '[', '{', '&', '*', '!', '|', '>', '%', '@', '`', '"', '\'', '-', '?', ','])
        || v.trim() != v;
    if v.is_empty() || !hostile {
        return v.to_string();
    }
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Render a subagent .md: frontmatter (name/description always; model/tools only when set) + body.
/// UTF-8 without BOM (Castellyn's own writer convention). Unquoted scalars match the ecosystem
/// convention (plugin agents ship unquoted) — the description is kept single-line by the UI.
fn render_agent_md(name: &str, description: &str, model: &str, tools: &str, prompt: &str) -> String {
    // Sanitize each scalar: a raw newline (or a line equal to `---`) in a value would break out of the
    // YAML frontmatter block. Collapse interior CR/LF to spaces so every value stays on one line.
    let clean = |v: &str| yaml_scalar(&v.trim().replace(['\r', '\n'], " "));
    let mut s = String::from("---\n");
    s.push_str(&format!("name: {}\n", clean(name)));
    s.push_str(&format!("description: {}\n", clean(description)));
    if !model.trim().is_empty() {
        s.push_str(&format!("model: {}\n", clean(model)));
    }
    if !tools.trim().is_empty() {
        s.push_str(&format!("tools: {}\n", clean(tools)));
    }
    s.push_str("---\n\n");
    s.push_str(prompt.trim_end());
    s.push('\n');
    s
}

#[tauri::command]
pub async fn list_agents() -> Vec<AgentInfo> {
    tokio::task::spawn_blocking(list_agents_blocking)
        .await
        .unwrap_or_default()
}

fn list_agents_blocking() -> Vec<AgentInfo> {
    let Ok(dir) = agents_dir() else {
        return Vec::new();
    };
    let mut out: Vec<AgentInfo> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() || p.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&p).unwrap_or_default();
            let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
            let fm = extract_frontmatter(content);
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(AgentInfo {
                name: fm_value(&fm, "name").unwrap_or(stem),
                description: fm_value(&fm, "description").unwrap_or_default(),
                model: fm_value(&fm, "model").unwrap_or_default(),
                tools: fm_value(&fm, "tools").unwrap_or_default(),
                path: p.display().to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[tauri::command]
pub fn read_agent(path: String) -> Result<AgentDetail, String> {
    let p = std::path::Path::new(&path);
    agent_guard(p)?;
    let content = std::fs::read_to_string(p)
        .map_err(|e| trv("err.fs_read", cur_lang(), &[("path", &path), ("e", &e)]))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let fm = extract_frontmatter(content);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(AgentDetail {
        name: fm_value(&fm, "name").unwrap_or(stem),
        description: fm_value(&fm, "description").unwrap_or_default(),
        model: fm_value(&fm, "model").unwrap_or_default(),
        tools: fm_value(&fm, "tools").unwrap_or_default(),
        prompt: frontmatter_body(content),
        path: p.display().to_string(),
    })
}

/// Write a subagent. `path` present → overwrite that file (edit); absent → create a new
/// `<slug>.md`, made unique so a create never clobbers an existing agent. Returns the written path.
#[tauri::command]
pub fn save_agent(
    name: String,
    description: String,
    model: String,
    tools: String,
    prompt: String,
    path: Option<String>,
) -> Result<String, String> {
    let dir = agents_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| trv("err.fs_create", cur_lang(), &[("path", &dir.display()), ("e", &e)]))?;
    let target = match path.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => {
            let pp = std::path::Path::new(p).to_path_buf();
            agent_guard(&pp)?;
            pp
        }
        None => {
            let base = slugify_agent(&name);
            let mut cand = dir.join(format!("{base}.md"));
            let mut n = 2;
            while cand.exists() {
                cand = dir.join(format!("{base}-{n}.md"));
                n += 1;
            }
            cand
        }
    };
    let content = render_agent_md(&name, &description, &model, &tools, &prompt);
    std::fs::write(&target, content)
        .map_err(|e| trv("err.fs_write", cur_lang(), &[("path", &target.display()), ("e", &e)]))?;
    Ok(target.display().to_string())
}

#[tauri::command]
pub fn delete_agent(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    agent_guard(p)?;
    std::fs::remove_file(p).map_err(|e| trv("err.fs_remove", cur_lang(), &[("path", &path), ("e", &e)]))
}

/// One check line of a subagent smoke test.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTestLine {
    ok: bool,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTestResult {
    ok: bool,
    lines: Vec<AgentTestLine>,
}

/// Smoke-test a subagent WITHOUT invoking it in a real session: validate its frontmatter + body, and
/// for a wrapper agent (its prompt shells out to codex/opencode) probe that the target CLI actually
/// resolves and answers `--version`. Answers "is this agent well-formed and are its deps present?".
#[tauri::command]
pub async fn test_subagent(path: String) -> Result<AgentTestResult, String> {
    tokio::task::spawn_blocking(move || test_subagent_blocking(&path))
        .await
        .map_err(|e| format!("test_subagent panicked: {e}"))?
}

/// True when `cli` appears in a system prompt as a COMMAND, not merely as prose. The probe used to
/// fire on a bare lowercase substring, so an agent that only MENTIONS codex/opencode (e.g. a
/// `codex-review-notes` reference) failed its whole smoke test when that CLI was absent from PATH —
/// for a wrapper it never was. A command sits either inside a fenced block / after a shell prompt
/// marker, or right after a pipeline-chaining operator, and is followed by an argument.
fn cli_invoked(body: &str, cli: &str) -> bool {
    let mut in_fence = false;
    for line in body.lines() {
        let l = line.to_lowercase();
        if l.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        let mut from = 0;
        while let Some(rel) = l[from..].find(cli) {
            let start = from + rel;
            let end = start + cli.len();
            // Followed by an argument (or the end of the command), never glued into a longer word.
            let after_ok = end == l.len() || l[end..].starts_with(' ');
            let before = l[..start].trim_end();
            let before_ok = (before.is_empty() && in_fence)
                || before.ends_with(['|', '&', ';', '(', '`', '$', '>']);
            if after_ok && before_ok {
                return true;
            }
            from = start + 1;
        }
    }
    false
}

fn test_subagent_blocking(path: &str) -> Result<AgentTestResult, String> {
    let p = std::path::Path::new(path);
    agent_guard(p)?;
    let raw = std::fs::read_to_string(p)
        .map_err(|e| trv("err.fs_read", cur_lang(), &[("path", &path), ("e", &e)]))?;
    let content = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let fm = extract_frontmatter(content);
    let body = frontmatter_body(content);
    let mut lines: Vec<AgentTestLine> = Vec::new();
    let push = |lines: &mut Vec<AgentTestLine>, ok: bool, text: String| lines.push(AgentTestLine { ok, text });

    let has_name = fm_value(&fm, "name").is_some_and(|s| !s.trim().is_empty());
    let has_desc = fm_value(&fm, "description").is_some_and(|s| !s.trim().is_empty());
    push(&mut lines, has_name, if has_name { "name задан".into() } else { "нет name во frontmatter".into() });
    push(&mut lines, has_desc, if has_desc { "description задан".into() } else { "пустое description — агент не будет авто-выбираться по задаче".into() });
    let has_body = !body.trim().is_empty();
    push(&mut lines, has_body, if has_body { "системный промпт задан".into() } else { "пустой системный промпт".into() });

    // Wrapper agents shell out to an external CLI — that CLI must resolve and run. Only a real
    // invocation counts; a passing mention of the name must not fail the agent's smoke test.
    for cli in ["codex", "opencode"] {
        if cli_invoked(&body, cli) {
            match exe_on_path(cli) {
                Some(exe) => {
                    let out = std::process::Command::new(&exe)
                        .arg("--version")
                        .creation_flags(CREATE_NO_WINDOW)
                        .output();
                    match out {
                        Ok(o) if o.status.success() => {
                            let v = String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string();
                            push(&mut lines, true, format!("CLI `{cli}` найден и отвечает: {v}"));
                        }
                        _ => push(&mut lines, false, format!("CLI `{cli}` найден, но не отвечает на --version")),
                    }
                }
                None => push(&mut lines, false, format!("CLI `{cli}` не найден на PATH — обёртка не сработает")),
            }
        }
    }

    let ok = lines.iter().all(|l| l.ok);
    Ok(AgentTestResult { ok, lines })
}

#[cfg(test)]
mod agent_tests {
    use super::{frontmatter_body, render_agent_md, slugify_agent};
    use crate::{extract_frontmatter, fm_value};

    #[test]
    fn round_trip_render_parse() {
        let md = render_agent_md(
            "my-agent",
            "When to use it",
            "sonnet",
            "Read, Grep",
            "You are a helper.\n\nDo the thing.",
        );
        let fm = extract_frontmatter(&md);
        assert_eq!(fm_value(&fm, "name").as_deref(), Some("my-agent"));
        assert_eq!(fm_value(&fm, "description").as_deref(), Some("When to use it"));
        assert_eq!(fm_value(&fm, "model").as_deref(), Some("sonnet"));
        assert_eq!(fm_value(&fm, "tools").as_deref(), Some("Read, Grep"));
        assert_eq!(frontmatter_body(&md).trim_end(), "You are a helper.\n\nDo the thing.");
    }

    #[test]
    fn omits_empty_model_and_tools() {
        let md = render_agent_md("a", "desc", "", "  ", "body");
        assert!(!md.contains("model:"));
        assert!(!md.contains("tools:"));
    }

    #[test]
    fn slug_kebabs_and_falls_back() {
        assert_eq!(slugify_agent("My Cool Agent!"), "my-cool-agent");
        assert_eq!(slugify_agent("  a__b  "), "a-b");
        assert_eq!(slugify_agent("Агент"), "agent"); // non-ASCII → generic fallback
    }

    #[test]
    fn body_without_frontmatter_is_whole() {
        assert_eq!(frontmatter_body("just text"), "just text");
    }

    #[test]
    fn yaml_scalar_quotes_only_what_would_break_the_frontmatter() {
        // Plain values stay plain — the ecosystem convention for these files.
        for plain in ["my-agent", "Use for X", "sonnet", "Read, Grep, Glob"] {
            assert_eq!(super::yaml_scalar(plain), plain);
        }
        // `: ` makes the whole frontmatter unparseable for any real YAML reader.
        assert_eq!(
            super::yaml_scalar("Use when: the user asks"),
            "\"Use when: the user asks\""
        );
        assert_eq!(super::yaml_scalar("trailing:"), "\"trailing:\"");
        assert_eq!(super::yaml_scalar("#comment"), "\"#comment\"");
        // Quotes and backslashes inside a quoted scalar must be escaped, not emitted raw.
        assert_eq!(
            super::yaml_scalar("say: \"hi\\there\""),
            "\"say: \\\"hi\\\\there\\\"\""
        );
    }

    #[test]
    fn render_agent_md_survives_a_colon_in_the_description() {
        let md = super::render_agent_md("a", "Use when: X", "", "", "body");
        assert!(md.contains("description: \"Use when: X\""), "{md}");
        // Castellyn's own reader strips the surrounding quotes, so the fix is backward-compatible.
        let fm = crate::extract_frontmatter(&md);
        assert_eq!(
            crate::fm_value(&fm, "description").as_deref(),
            Some("Use when: X")
        );
    }

    #[test]
    fn cli_invoked_separates_a_command_from_a_mention() {
        // Mentions must NOT trigger the CLI probe (they used to fail the whole agent test).
        for prose in [
            "See codex-review-notes for background.",
            "This agent replaces opencode entirely.",
            "codex is a CLI by OpenAI",
        ] {
            assert!(!super::cli_invoked(prose, "codex"), "{prose}");
        }
        // Real invocations still do.
        assert!(super::cli_invoked("Run `codex exec \"fix it\"` now.", "codex"));
        assert!(super::cli_invoked("```sh\ncodex exec --json\n```", "codex"));
        assert!(super::cli_invoked("cd repo && opencode run", "opencode"));
        assert!(super::cli_invoked("$ codex", "codex"));
    }
}
