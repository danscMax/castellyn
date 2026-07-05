# Verified Findings — personal verification by Lead (Phase 4)

Every item below was verified by the Lead personally: cited lines opened with Read,
evidence quotes matched against the real file at HEAD `1c02e3d`. DA verdicts merged
separately (advocate-validation.md).

## Verified: V-1 [REL-1] UTF-8 byte-slice panic in `expand_ssh_config` — HIGH (confirmed)
**File:** src-tauri/src/lib.rs:12545-12547
**Evidence (read personally):** `let is_include = t.len() > 7 && t[..7].eq_ignore_ascii_case("include") && t[7..]...` — byte-length guard only; the loop (12542-12559) has NO comment skip, every line of ~/.ssh/config and included files hits the slice.
**Call chain:** `read_ssh_hosts` (#[tauri::command], :12593) → `read_ssh_config_hosts` (:12524) → `expand_ssh_config`. Panics on any line whose byte 7 splits a multi-byte char (Cyrillic comment `# рабочий…`: byte 7 = 2nd byte of `б`). Cyrillic-heavy user environment → realistic.
**Recommendation:** `t.get(..7)` / `t.get(7..)` (returns None off-boundary) + unit test with a Cyrillic comment line.

## Verified: V-2 [SUP-1] quick-xml 0.39.4 RUSTSEC-2026-0194/0195 (2× CVSS 7.5) — HIGH as advisory, exploitability LOW in this app
**Evidence (verified personally):** re-ran `cargo tree -i quick-xml --target x86_64-pc-windows-msvc` → `quick-xml 0.39.4 ← plist 1.9.0 ← tauri-utils 2.9.3 ← tauri 2.11.5 ← castellyn` (runtime dep, not only build). Cargo.lock pins 0.39.4. Advisories dated 2026-06-29, fix >=0.41.0.
**Assessment:** parser DoS requires attacker-controlled XML/plist input; none in this trust model. Track-and-upgrade item, not fix-at-4am.
**Recommendation:** try resolver-level bump (`cargo update` chain / check plist 1.10), else `[patch]`/direct override; verify build.

## Verified: V-3 [SEC2-1] Balance fetch: key over http possible — MEDIUM (confirmed)
**File:** src-tauri/src/lib.rs:5646-5710
**Evidence (read personally):** `:5671 valid_base_url(balance_url)` (allows http://); `balance_get(&agent, balance_url, protocol, &key)` at :5674 sends the key. Fallbacks (:5690, :5697) call `balance_get` on `{root}/...` with NO url re-validation at all in this function. Comment at :5668 mistakenly claims parity with the probe guard. `check_provider_balance` itself is async+spawn_blocking (:5713-5718) — threading fine, guard wrong.
**Recommendation:** `probe_url_allowed(balance_url)` and guard `root`-derived URLs the same way.

## Verified: V-4 [PERF-1] Heavy dir-scan commands on the main thread — MEDIUM (confirmed)
**Evidence (verified personally):** plain `fn` under #[tauri::command]: read_profile_matrix (:4098), list_skills (:6825), read_environments (:7126), read_skill_matrix (:7316), list_plugin_contents (:8561). In-repo async+spawn_blocking template exists (check_provider_balance :5713, read_stack per analyst).
**Recommendation:** async fn + tokio::task::spawn_blocking for the 5 heavy ones.

## Verified: V-5 [PERF-2] Console non-keyed each + front splice — MEDIUM (confirmed)
**Evidence (read personally):** Console.svelte:151 `{#each log as line}` (no key); +page.svelte:185-191 `MAX_LOG=5000`, `log.splice(0, log.length - MAX_LOG)`. log is string[] → keying needs {id,text} wrapper or windowed render.
**Recommendation:** stable-id wrapper + keyed each, or render window of last N lines.

## Verified: V-6 [QUAL-1] updatesAttention bypasses countOf — MEDIUM (confirmed, arguably Low impact today)
**Evidence (read personally):** attention.ts:14-18 reads only `counts?.changed`; countOf (envelope.ts) carries legacy fallbacks. No countOf import in attention.ts.
**Recommendation:** use countOf in updatesAttention.

## Verified: V-7 [QUAL-2+3] Stringly-typed stream-id contract — MEDIUM (confirmed, STRONGER than reported)
**Evidence (verified personally):** 18 spawn sites with hard-coded literals (grep): forks:1058, backup:1409, profiles:1504+1791, sync:1596, engine:2678+3459, provider:3489/4460/5291/5323/6173, schedule:6437, mcp:6470, plugin-mgr:8831+8885, pluginsync:9579, onboarding:10102. Frontend run-done ladder in +page.svelte:2069-2209 dispatches on untyped string. No compile-time binding either side.
**Recommendation:** shared id constants (Rust mod + TS union), reload map keyed by the union.

## Verified: V-8 [SUP-2] serial 0.4.0 unmaintained via portable-pty 0.8.1 — reclassify LOW (no action available)
**Evidence (verified personally):** `cargo tree -i serial --target x86_64-pc-windows-msvc` → via portable-pty 0.8.1. Pin deliberate (Cargo.toml:42-45). Action impossible until 0.9 viable → tracked debt, Low.

## Verified: V-9 [SEC-1] MCP env secrets on `codex mcp add --env K=V` argv — LOW (confirmed)
**Evidence (read personally):** lib.rs:7783-7790 flattens env into argv; run_codex_mcp (:7818) spawns `cmd /C codex …`. Integration-forced; document-or-improve decision.

## Verified: V-10 [SEC-2≡SEC2-2, multi-run 2/2] freellmapi key via setx → plaintext HKCU\Environment — LOW (confirmed)
**Evidence (read personally):** lib.rs:7989-7994 `Command::new("setx").args(["FREELLMAPI_API_KEY", &key])`. Both key-on-argv (transient) and plaintext-at-rest (persistent).

## Verified: V-11 [SEC-3] cmd_argv_safe omits `(`,`)`,`\n`,`\r` — LOW (confirmed)
**Evidence (read personally):** lib.rs:7807-7812 `const UNSAFE: &[char] = &['&','|','<','>','^','%','"']`. Guard comment (:7803-7806) confirms it protects a cmd re-parse.

## Verified: V-12 [SEC2-3] ureq probes follow redirects unvalidated — LOW (confirmed, one caveat)
**Evidence (read personally):** balance agent :5663-5666 `config_builder().timeout_global(...).build()` — no redirect config. Claimed ureq3 defaults (10 redirects, SameHost auth) to be re-confirmed against ureq 3.x docs in the best-practices pass before fixing.

## Verified: V-13 [REL-2] PTY reader: reap only after Child::wait — LOW (confirmed)
**Evidence (read personally):** lib.rs:12169-12205 — loop breaks on Ok(0)/Err, then blocking `Child::wait`, and ONLY then the map reap (:12199-12204). Grandchild holding the slave = no EOF = slot held. Bounded by Job Object at app exit.

## Verified: V-14 [REL-3] No run-timeout backstop — LOW (confirmed)
**Evidence (read personally):** lib.rs:754 `child.wait().await` untimed in pump_and_wait. cancel_run exists.

## Verified: V-15 [REL-4] Unbounded next_line buffering — LOW (confirmed)
**Evidence (read personally):** lib.rs:820-831 `lines.next_line()` (no length cap; 32KiB cap is PTY-path only).

## Verified: V-16 [PERF-3] Serial limits polling — LOW (confirmed)
**Evidence (read personally):** limits.rs:250-253 serial `for … poll_profile(...)`; background thread, 300s cadence.

## Verified: V-17 [QUAL-4] ForksTab `.length` on string|string[] — LOW (confirmed)
**Evidence (read personally):** ForksTab.svelte:64-65; normalizer exists in ForkRepoCard.svelte:42-44. Only >0 compare today → works by accident.

## Verified: V-18 [QUAL-5] Auto-continue keys on h5Reset only — LOW (confirmed)
**Evidence (read personally):** SessionsTab.svelte:363-369 — only `h5Reset` consulted; d7-limited pane gets a wrong-trigger nudge. Self-correcting (one attempt per episode, :350).

## Verified: V-19/V-20/V-21 [SUP-3/4/6] anyhow unsound / unic-* unmaintained / cookie via sveltekit — LOW (tool-grounded)
**Evidence:** analyst quoted actual `cargo audit`/`npm audit`/`npm view` output; dependency paths spot-verified via cargo tree (quick-xml, serial personally). All are no-action-available/track items. SUP-5 (GTK chain) = informational, not compiled for Windows target.

---
Rejected: none by Lead. Severity adjustments by Lead: V-2 High(advisory)/Low(exploitability) — treat as High-priority upgrade chore; V-8 Medium→Low (no action available).
