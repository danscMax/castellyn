# Security Findings — Castellyn (pass 1)

Branch `main` @ `1c02e3d`, 2026-07-05. Threat model: malicious/compromised script output,
malformed JSON, hostile provider endpoint, secret leakage — single-user local desktop tool,
NOT a remote/physical attacker.

**Bottom line:** the classic attack surfaces (PowerShell/cmd argument injection, path traversal,
SSRF, bearer-key exfiltration) are unusually well hardened — clearly the target of a prior audit,
with in-code guards (`valid_profile_name`, `probe_url_allowed`, `weekly_archive_path`,
`cmd_argv_safe`, `delete_orphan_profile` normalization guards) and comments citing earlier item IDs.
No Critical/High issues found. Three genuine Low findings below, all in the "secret hygiene /
defense-in-depth" category and all consistent with (i.e. narrow exceptions to) the codebase's own
stated policies.

---

## [SEC-1] MCP `env` secret values are placed on the `codex mcp add` command line — Low
**File:** `src-tauri/src/lib.rs:7783-7788` (built in `codex_mcp_add_args`), spawned at `:7847-7850`
**Description:** When fanning canonical `.mcp.json` servers into Codex, each server's `env` map is
flattened into `--env KEY=VALUE` argv elements and run as `cmd /C codex mcp add … --env KEY=VALUE`.
MCP-server env values are exactly where secrets live in practice (GitHub tokens, provider API keys,
etc.). Command-line arguments are readable by any same-user process via WMI
(`Get-CimInstance Win32_Process`) — the very leak the codebase deliberately avoids elsewhere by
routing secrets through STDIN (see the explicit note at `:603-605`). `cmd_argv_safe` only blocks
shell metacharacters; it does not (and cannot) stop a clean `KEY=secret` value from appearing on the
line. This is a narrow, integration-forced exception (the Codex CLI only accepts `--env` on argv),
but it is a real inconsistency with the project's own secret-on-argv policy.
**Evidence:**
```rust
if let Some(env) = def.get("env").and_then(|e| e.as_object()) {
    for (k, v) in env {
        if let Some(val) = v.as_str() {
            argv.push("--env".into());
            argv.push(format!("{k}={val}"));
        }
    }
}
```
```rust
let mut cmd = std::process::Command::new("cmd");
cmd.arg("/C").arg("codex").args(&argv);
```
**Fix suggestion:** If the Codex CLI supports reading server env from a config file or stdin, prefer
that over `--env`. Otherwise, document the transient exposure (single-user machine, brief lifetime)
next to `codex_mcp_add_args` so the deviation from the STDIN-secret rule is a recorded decision, not
an oversight — mirroring the `:603-605` comment.

## [SEC-2] freellmapi API key persisted to HKCU\Environment via `setx` (plaintext, globally inherited) — Low
**File:** `src-tauri/src/lib.rs:7989-7994`
**Description:** After wiring the Codex gateway, the resolved freellmapi API key is written with
`setx FREELLMAPI_API_KEY <key>`. `setx` persists to `HKCU\Environment` as **plaintext**, readable by
any process running as the user and inherited by every future child process — a strictly weaker
posture than the Credential Manager (DPAPI, per-secret) the project uses for all other secrets
(`kr_set`, Cargo.toml:37-39). The key also appears transiently on `setx`'s own argv (WMI-readable).
The env var is required for the gateway integration to function, so this is a necessary trade-off,
but it is an undocumented downgrade of one secret to plaintext-at-rest.
**Evidence:**
```rust
let st = std::process::Command::new("setx")
    .args(["FREELLMAPI_API_KEY", &key])
    .creation_flags(CREATE_NO_WINDOW)
    .output()
    .ok()?;
```
**Fix suggestion:** Keep the canonical copy in the keyring and set the process/child env from it at
launch where feasible, rather than persisting plaintext to the registry. If `setx` persistence is
truly required by the gateway, add a comment recording that the plaintext-env exposure is an accepted
constraint of this integration (as SEC-1), so it isn't mistaken for a leak.

## [SEC-3] `cmd_argv_safe` denylist omits `(`, `)` and newline — Low (defense-in-depth)
**File:** `src-tauri/src/lib.rs:7807-7812`
**Description:** The guard that vets user-editable `.mcp.json` fields before they enter a re-parsed
`cmd /C codex …` line blocks `& | < > ^ % "` but not `(` `)` or an embedded newline (`\n`/`\r`).
Newline injection into a `cmd /C` line is a known technique for terminating the current command and
starting a new one. Practical exploitability here is low because `&`, `|`, and `^` are already
blocked (chaining a second command is hard without them) and the input is the user's own local
config — but as a denylist guarding a command re-parse, completeness matters and the gap is silent.
**Evidence:**
```rust
fn cmd_argv_safe(argv: &[String]) -> bool {
    const UNSAFE: &[char] = &['&', '|', '<', '>', '^', '%', '"'];
    !argv
        .iter()
        .any(|a| a.chars().any(|c| UNSAFE.contains(&c)))
}
```
**Fix suggestion:** Add `'('`, `')'`, `'\n'`, `'\r'` to `UNSAFE` (and consider an allowlist for the
name field, which is already charset-constrained elsewhere). Cheap, closes the residual re-parse gap.

---

## Clean areas (examined, no new finding)

- **PowerShell/cmd argument injection (primary surface).** Every path that interpolates
  user-controllable strings into a `-Command`/`cmd /c` line is guarded:
  `run_profiles`/`repair_profile_elevated` charset-validate the profile name via `valid_profile_name`
  (`:1470`, `:1808`) *before* it reaches an elevated `Start-Process` string; `recycle_path` passes the
  target via the `CASTELLYN_DEL_PATH` env var, not string interpolation (`:9349-9357`);
  `launch_profile`/`measure` set provider secrets and config via real env vars and `current_dir`, not
  inlined `set K=V &&` (`:11342-11354`, `:11258`); `open_terminal` uses `current_dir` to dodge cmd
  metacharacters (`:11374-11376`). Secrets are routed through STDIN, never argv (`spawn_streamed_io`
  `:603-616`).
- **Path traversal.** `weekly_archive_path` rejects separators and enforces a `weekly-…zip`
  prefix/suffix (`:1250-1256`); `delete_skill` canonicalizes and requires parent == `~/.claude/skills`
  (`:9058-9064`); `delete_orphan_profile` rejects separators, `..`, control chars, and trailing
  space/dot (the Windows path-normalization trick) and refuses canon dirs and reparse-point children
  (`:9294-9322`). `tar -x` extraction uses system bsdtar without `-P` (`:1316-1318`), so `..`/absolute
  entries are stripped by libarchive defaults.
- **SSRF / bearer-key exfiltration.** `probe_url_allowed` requires https for any non-loopback host
  before the `Authorization: Bearer <key>` probe, with a parse-based (not substring) loopback check
  that defeats `127.0.0.1.evil.com` / `127.0.0.1@evil.com` (`:5411-5438`); `fetch_engine_models` runs
  `valid_base_url` before the outbound GET (`:5494-5499`).
- **Secret handling in logs/IPC.** Provider tokens are written to `settings.json` natively and logged
  as `Token=(set)` / `(dummy…)`, never the value (`:3868-3879`); `read_providers`/`read_profile_matrix`
  return `has_token` booleans, never the token (`:3612-3674`); the freellmapi login logs only the URL,
  baseUrl and model — not email/password/token/apiKey (`:5199-5203`). Keyring get/set/delete are clean
  (`:4496-4527`).
- **Tauri config / capabilities / updater.** `tauri.conf.json` CSP is restrictive
  (`script-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, no wildcard connect-src);
  `dragDropEnabled: false`; the updater endpoint is https-only with a minisign `pubkey`; the default
  capability set is minimal (no `fs`/`shell` scope exposed to the WebView).
- **Frontend `{@html}` / opener.** All `{@html}` sinks render trusted constants — `ENVS[].icon` and
  `envIcon()` map from a hardcoded array (`SessionsTab.svelte:791,1106`), Sidebar/Select icons are
  static, and the SSH reach dot `iconHtml` is built solely from a fixed enum
  (`SessionsTab.svelte:1100-1179`). `open_url` restricts to http/https before handing to the OS opener
  (`:11428-11439`), so the xterm `WebLinksAddon → openUrl` path (untrusted terminal output,
  `TerminalPane.svelte:466`) cannot launch `file://`/custom-scheme programs; `clone_repo` forces https
  and uses `git clone --` (`:11400-11414`).

---

**Summary:** A deep pass over the argument-injection, path-traversal, SSRF, and secret-leakage
surfaces found no Critical/High issues — those surfaces are thoroughly hardened. The three Low
findings are all secret-hygiene / defense-in-depth gaps: MCP `env` secrets on the `codex` argv
(SEC-1), the freellmapi key persisted to plaintext HKCU\Environment via `setx` (SEC-2), and an
incomplete `cmd_argv_safe` denylist (SEC-3). Counts — Critical: 0, High: 0, Medium: 0, Low: 3.
Output file: `E:\Scripts\Castellyn\plans\tech-debt-full\2026-07-05\security-findings.md`
