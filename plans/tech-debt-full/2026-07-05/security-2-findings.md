# Security Audit — Run 2 of 2 (independent pass)

Project: Castellyn (Tauri v2 desktop, Windows). HEAD `1c02e3d`. Threat model per briefing:
malicious/compromised script output, malformed JSON, hostile provider endpoint, secret leakage —
NOT a remote or local-admin attacker.

Scope-honest summary up front: this codebase has clearly been through a prior security pass. The
command-injection, SSRF-metadata, profile-name-into-elevated-PowerShell, secrets-via-stdin-not-argv,
CSP, capabilities, and open_url/open_in_editor surfaces are all already hardened. I found **one
Medium** (a real inconsistency in the key-leak guard), and **two Low** observations. No Critical/High.

---

## [SEC2-1] Provider API key can be sent over cleartext HTTP via `balanceUrl` (guard inconsistency) — Medium

**File:** `src-tauri/src/lib.rs:5646-5710` (balance fetch), contrast with `5411-5438` (`probe_url_allowed`)

**Description:**
The provider liveness probe deliberately enforces "https, except genuine loopback" via
`probe_url_allowed()` *because* it sends the bearer key — its own doc comment says a plaintext
`http://` to a non-loopback host "would leak the key on the wire". The **balance** fetch also sends
the provider's key (`balance_get` attaches `provider_auth_headers`), but validates the target with
only `valid_base_url()`, which permits `http://` to any non-metadata host. So a provider whose
`balanceUrl` (or `baseUrl`, used to derive the DeepSeek/OpenAI-billing fallbacks) is `http://…` will
transmit the API key (`Authorization: Bearer …` / `x-api-key`) in cleartext to a non-loopback host.
The code comment at line 5668 even claims it is "SSRF-guard[ed] … like baseUrl (R2-01) … queried WITH
the provider's key" — but it wired the *wrong* guard: the sibling key-bearing path uses
`probe_url_allowed`, this one uses `valid_base_url`, and the https requirement is exactly what differs.

**Evidence:**
```rust
// fetch_provider_balance — case 1, the user-configured balance URL:
if !balance_url.is_empty() {
    if let Err(detail) = valid_base_url(balance_url) {          // <-- allows http://
        return serde_json::json!({ "ok": false, "detail": detail });
    }
    return match balance_get(&agent, balance_url, protocol, &key) { // <-- sends the key
```
```rust
// balance_get — the key goes on the wire:
let mut req = agent.get(url);
for (k, v) in provider_auth_headers(protocol, key) {   // x-api-key / Bearer <key>
    req = req.header(k, &v);
}
```
```rust
// probe_url_allowed — the CORRECT guard the balance path should reuse:
/// The probe sends `Authorization: Bearer <key>`, so a plaintext http:// to a non-loopback host would
/// leak the key on the wire — http:// is allowed only for genuine loopback (localhost / 127.0.0.0/8 / ::1).
fn probe_url_allowed(base_url: &str) -> Result<(), String> {
    valid_base_url(base_url)?;
    if let Some(rest) = base_url.trim().strip_prefix("http://") { ... return Err(err.https_required) ... }
```

**Fix suggestion:** In `fetch_provider_balance`, replace the `valid_base_url(balance_url)` check with
`probe_url_allowed(balance_url)` (and gate the derived `root`-based fallback URLs the same way, since
they inherit `baseUrl`'s scheme). One-line swap; makes the key-bearing balance path match the probe
path's own stated invariant.

---

## [SEC2-2] freellmapi key copied from Credential Manager into a persistent plaintext user env var (`setx`) — Low

**File:** `src-tauri/src/lib.rs:7955-7997` (`run_codex_providers`)

**Description:**
Cargo.toml states secrets "live in the Windows Credential Manager, never in plaintext JSON." This
command reads the gateway key and mirrors it with `setx FREELLMAPI_API_KEY <key>`, which (a) persists
the secret in `HKCU\Environment` in **plaintext**, readable by every future process running as the
user and surviving reboots, and (b) passes the key as a **command-line argument** to `setx`, briefly
visible to any process that can enumerate command lines (WMI `Win32_Process`, etc.). The doc comment
asserts "the key itself is never logged," which is true of logs but the `setx` mirror is a broader,
more persistent exposure than the keyring the rest of the code is careful to use. Outside the stated
threat model (no local attacker), so Low — but it is a genuine secret-management inconsistency and
worth a conscious decision rather than an incidental one.

**Evidence:**
```rust
let st = std::process::Command::new("setx")
    .args(["FREELLMAPI_API_KEY", &key])   // key on argv + persisted plaintext in HKCU\Environment
    .creation_flags(CREATE_NO_WINDOW)
    .output()
    .ok()?;
```

**Fix suggestion:** If the persistent env var is genuinely required for `codex --profile freellmapi`
to work in a fresh terminal, keep it but document the plaintext-persistence tradeoff at the call site
(so it isn't read as "secrets never leave the keyring"). If not strictly required, prefer setting the
var only in the specific child's environment (`.env(...)` on the spawned process) instead of
machine-persisting it via `setx`.

## [SEC2-3] `ureq` probes follow redirects to hosts the SSRF guard never re-validates (blind) — Low

**File:** `src-tauri/src/lib.rs:5442-5485` (`probe_provider`), `5490-5537` (`fetch_engine_models`), `5646-5710` (balance)

**Description:**
Every outbound probe validates only the *initial* URL (`probe_url_allowed` / `valid_base_url`) and
builds a `ureq::Agent` with no redirect policy set. ureq 3.3.0 follows up to 10 redirects by default,
and none of the probes re-check the redirect *target* against the metadata/link-local blocklist. A
hostile provider endpoint can therefore 3xx-redirect a probe to an internal address
(`http://169.254.169.254/…`, a LAN service, etc.), bypassing the SSRF blocklist. Impact is limited:
ureq 3.x defaults `redirect_auth_headers` to `SameHost`, so the API key is **not** forwarded across a
host change, and the response body is never returned to the provider (only parsed locally for model
count / balance numbers), making this a *blind* SSRF. Hence Low, not Medium.

**Evidence:**
```rust
let agent: ureq::Agent = ureq::Agent::config_builder()
    .timeout_global(Some(std::time::Duration::from_secs(12)))
    .build()                       // <-- no .max_redirects(0); default follows up to 10
    .into();
let mut req = agent.get(&url);     // url validated; redirect target is not
```

**Fix suggestion:** Set `.max_redirects(0)` on the probe agents (these are single-shot API GETs that
have no legitimate need to follow cross-host redirects), or add a per-hop validation. `.max_redirects(0)`
is the one-line, lowest-risk option and closes the redirect-SSRF entirely.

---

## Clean areas (verified, not just assumed)

- **Command / arg injection into PowerShell:** `spawn_streamed_prog` passes every arg as a separate
  argv (`cmd.arg(a)`), never a reconstructed shell string. The one place a name is interpolated into
  an *elevated* `-Command` string (`repair_profile_elevated`, lib.rs:1798-1852) is charset-validated
  by `valid_profile_name` first, cross-checked against `profile_names()`, and single-quote-escapes the
  localized strings — the developer comment documents the exact injection risk and mitigates it.
- **Secrets never on argv:** the streaming path threads secrets through **stdin**
  (`spawn_streamed_io` / `stdin_payload`, lib.rs:603-670), never command-line args. Provider keys and
  the freellmapi token/email/password are stored in the keyring (`kr_set` at 4849/5044) and used via
  native `ureq` HTTP bodies/headers (`connect_custom_native`, 5104-5258), not shelled out. The
  `myproviders` JSON stores only metadata (`4821-4835`), no key.
- **Loopback/metadata SSRF guard (initial URL):** `valid_base_url` (4604-4643) blocks
  169.254.169.254, GCP/Alibaba/AWS-IPv6 metadata, `169.254.*`, and resolves hostnames to catch
  non-canonical IP encodings and DNS-rebind-to-metadata; `probe_url_allowed` (5411-5438) correctly
  rejects `127.0.0.1@evil.com` / `127.0.0.1.evil.com` by *parsing* the host rather than prefix-matching.
- **open_url / open_in_editor:** `open_url` (11428-11443) restricts to http/https before the OS opener;
  `open_in_editor` (12343-12382) canonicalizes the (untrusted, terminal-sourced) path, requires a
  regular file, blocks executable/script extensions, and only ever passes argv (never a shell string);
  `clone_repo` (11400-11423) is https-only and uses `git clone --`.
- **xterm link handling:** `WebLinksAddon`/`registerLinkProvider` (TerminalPane.svelte:466-487) route
  through the guarded `open_url` / `open_in_editor` commands — a malicious link in PTY output can't
  open a `file://`/custom-scheme target or execute.
- **CSP / capabilities / updater:** `tauri.conf.json` CSP is tight (`default-src 'self'`,
  `script-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, `base-uri 'self'`, no remote
  `connect-src`); capabilities are least-privilege (detached pane windows get no dialog/opener); the
  updater endpoint is https-GitHub with a minisign `pubkey` set (signature verification on).
- **`{@html}` sinks** (Select/Sidebar/SessionsTab) render internal icon constants and a
  known-enum `tool`, not untrusted script output — no injection sink reached.
- **agent_status report files** (agent_status.rs:388-420) are keyed on internally-generated session
  ids (`gen_session_id`), not on any untrusted PTY content — no path traversal, malformed JSON skipped.

---

**Counts:** Critical 0, High 0, Medium 1, Low 2.
Output: `E:\Scripts\Castellyn\plans\tech-debt-full\2026-07-05\security-2-findings.md`
