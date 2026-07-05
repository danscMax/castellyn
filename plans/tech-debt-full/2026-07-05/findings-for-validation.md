# Findings for DA Validation — Castellyn tech-debt audit (HEAD 1c02e3d, 2026-07-05)

Deduplicated synthesis of 6 analyst reports (security ×2 independent runs, reliability,
performance, quality, supply-chain). Source files in this directory.

Validate ALL findings below, priority order: High → Medium → Low. For each, verify the
evidence against the actual code (Read the cited lines), look for defenses the analyst
missed, and issue a verdict: confirmed / downgraded / needs-evidence / rejected, with
reasoning. Full finding bodies (evidence quotes, fix suggestions) are in the source files
named per item.

## HIGH

### V-1 [REL-1] UTF-8 byte-slice panic in `expand_ssh_config` — High
src-tauri/src/lib.rs:12545-12547. `t[..7]`/`t[7..]` slices by byte index guarded only by
`t.len() > 7`; a line whose byte 7 falls inside a multi-byte char (e.g. Cyrillic comment
`# рабочий сервер` in ~/.ssh/config) panics. No `#`-comment skip in this loop (unlike
sibling `parse_ssh_config`). Takes down `read_ssh_hosts`. Source: reliability-findings.md.
Check: is there any upstream filter that prevents non-ASCII lines reaching this? Is the
panic actually reachable from a command?

### V-2 [SUP-1] quick-xml 0.39.4 — two RUSTSEC HIGH (CVSS 7.5) advisories — High
Cargo.lock, via plist←tauri-utils←tauri/tauri-build. RUSTSEC-2026-0194/0195, dated
2026-06-29. Confirmed reachable on x86_64-pc-windows-msvc via cargo tree. Analyst notes
practical exploitability is limited (plist = build-time/bundle metadata, not
attacker-controlled runtime input). Source: supply-chain-findings.md.
Check: severity — is High justified given the trust model, or is this a tracked-upgrade
Medium/Low? Is a fix actually available through resolver override?

## MEDIUM

### V-3 [SEC2-1] Balance fetch sends API key but allows http:// (wrong guard) — Medium
src-tauri/src/lib.rs:5646-5710. `fetch_provider_balance` validates `balanceUrl` with
`valid_base_url` (permits http:// to non-loopback) yet `balance_get` attaches
provider_auth_headers — key can go cleartext. Sibling probe path uses `probe_url_allowed`
(https-or-loopback) precisely to prevent this. Source: security-2-findings.md.
Check: verify balance_get really attaches the key for the user-configured balanceUrl path
AND the derived fallback URLs; confirm no https enforcement elsewhere on that path.

### V-4 [PERF-1] Heavy dir-scan commands run synchronously on the Tauri main thread — Medium
lib.rs: read_profile_matrix (4097), list_skills (6825), read_environments (7126),
read_skill_matrix (7316), list_plugin_contents (8561), and lighter read_mcp/read_opencode/
read_codex_profiles. Plain `fn` commands = main thread in Tauri v2 (doc-cited);
spawn_blocking template already in-repo (read_stack etc.). Source: performance-findings.md.
Check: confirm these are plain fn (not #[tauri::command(async)]); confirm the listed ones
really do multi-file/dir-walk work.

### V-5 [PERF-2] Console log: non-keyed {#each} + front-splice → O(n) DOM churn at cap — Medium
src/lib/components/Console.svelte:151-158 + src/routes/+page.svelte:183-190 (MAX_LOG 5000,
splice(0,…)). Non-keyed each re-diffs all rendered lines per append once at cap; visible
jank when dock open during verbose runs. Source: performance-findings.md.
Check: confirm no key on the each; confirm log entries are plain strings (so keying needs
an id wrapper); sanity-check the O(n) claim for Svelte 5 non-keyed each with shifted values.

### V-6 [QUAL-1] `updatesAttention` bypasses `countOf` envelope helper — Medium
src/lib/attention.ts:9-19 vs src/lib/envelope.ts:7-13. Sidebar badge reads only
counts.changed, no legacy fallbacks; toast/cards use countOf with fallbacks. Latent
divergence. Source: quality-findings.md.
Check: confirm countOf import is absent in attention.ts; assess whether "latent" merits
Medium or Low given Write-StatusJson always emits counts today.

### V-7 [QUAL-2+QUAL-3 merged] Stringly-typed stream component-id contract across Rust↔TS — Medium
Backend: lib.rs spawn sites hard-code "backup"/"profiles"/"sync"/"engine"/"provider"…
(lines 1409, 1504, 1596, 2678, 3489); frontend: +page.svelte:2069-2209 run-done listener
dispatches on untyped string with ~7 module flags. No compiler binding either side; a
mismatched literal fails silently. Not a bug today (analyst verified ids line up) —
maintainability hotspot. Source: quality-findings.md (two findings, one root cause).
Check: verify the ids do line up today (no live bug); assess Medium vs Low as pure debt.

### V-8 [SUP-2] portable-pty 0.8.1 → `serial` 0.4.0 unmaintained since 2017 — Medium
Cargo.lock, RUSTSEC-2017-0008. Tied to the deliberate portable-pty 0.8 pin (briefing:
pin itself not flaggable; no action until 0.9 viable). Source: supply-chain-findings.md.
Check: is Medium right for a no-action-available tracked item, or downgrade to Low?

## LOW (validate if budget allows; sample at minimum)

### V-9 [SEC-1] MCP env secret values on `codex mcp add --env KEY=VALUE` argv — Low
lib.rs:7783-7788, spawned :7847-7850. WMI-readable argv; codebase routes secrets via
STDIN elsewhere (:603-605). Integration-forced. Source: security-findings.md. (multi-run 1/2)

### V-10 [SEC-2 ≡ SEC2-2] freellmapi key persisted plaintext to HKCU\Environment via setx — Low
lib.rs:7989-7994. Found independently by BOTH security runs (multi-run 2/2). Plaintext
at rest + argv exposure; weaker than the keyring used everywhere else. Both analysts note
it may be integration-required → document-or-scope-down decision.

### V-11 [SEC-3] `cmd_argv_safe` denylist omits `(`, `)`, `\n`, `\r` — Low
lib.rs:7807-7812. Defense-in-depth gap in cmd /C re-parse guard. (multi-run 1/2)

### V-12 [SEC2-3] ureq probes follow redirects without re-validating target — blind SSRF — Low
lib.rs:5442-5485, 5490-5537, 5646-5710. No .max_redirects(0); ureq3 default follows 10.
Auth header not forwarded cross-host (SameHost default) → blind only. (multi-run 1/2)

### V-13 [REL-2] PTY reader Child::wait can block forever if grandchild holds pty slave — Low
lib.rs:12169-12205. Slot leak while app runs; bounded by Job Object at exit.

### V-14 [REL-3] No timeout backstop on script runs — hung script holds the single run slot — Low
lib.rs:728-768. cancel_run exists; unattended runs (tray check_all) have no backstop.

### V-15 [REL-4] Unbounded line buffering in pump_stream for newline-less output — Low
lib.rs:805-851. next_line() has no length cap (PTY path's 32KiB cap doesn't apply here).

### V-16 [PERF-3] limits.rs serial per-profile polling, 8s timeout each — Low
limits.rs:190-254. Background thread, 5-min cadence — laggy chip only.

### V-17 [QUAL-4] ForksTab `conflictFiles?.length` on string|string[] union — Low
ForksTab.svelte:64-65 vs ForkRepoCard.svelte:42-44 normalizer. Works today (only >0
compare), latent misuse.

### V-18 [QUAL-5] Sessions auto-continue keys on h5Reset only, ignores d7 window — Low
SessionsTab.svelte:363-369. Wrong-trigger nudge into still-limited session; self-correcting.

### V-19 [SUP-3] anyhow 1.0.102 unsound downcast_mut (transitive only) — Low
### V-20 [SUP-4] unic-* ×5 unmaintained via urlpattern←tauri-utils — Low
### V-21 [SUP-6] cookie <0.7.0 via @sveltejs/kit — no real upstream fix exists yet — Low
(SUP-5 GTK chain = informational/clean, not compiled for Windows target — no validation needed.)

## Output
Write your validation to:
E:\Scripts\Castellyn\plans\tech-debt-full\2026-07-05\advocate-validation.md
Per item: verdict (confirmed / downgraded-to-X / needs-evidence / rejected) + 1-5 sentence
justification grounded in code you actually read. You do NOT generate new findings.
