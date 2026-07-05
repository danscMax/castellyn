# Devil's Advocate Validation — Castellyn tech-debt audit (HEAD 1c02e3d, 2026-07-05)

Every item below was checked against the actual current code (Read tool, exact lines quoted),
not against the analyst's summary. Threat model applied: single-user local desktop tool;
Critical = "fix at 4am." Findings are NOT regenerated — only judged.

---

## HIGH

### V-1 [REL-1] `expand_ssh_config` UTF-8 byte-slice panic — **CONFIRMED, High**
`src-tauri/src/lib.rs:12535-12560` read in full. The loop (`for line in text.lines()`) has
**no `#`-comment skip** before the byte-index slice — confirmed, `parse_ssh_config` (the
sibling function, ~12490) does skip comments but `expand_ssh_config` does not; every line,
comment or not, reaches `t[..7]`/`t[7..]` gated only by `t.len() > 7` (byte length, not char
boundary). Byte math re-verified independently: `"# рабочий"` → `#`(0) ` `(1) `р`(2-3) `а`(4-5)
`б`(6-7) — byte offset 7 falls between the two bytes of `б`, not a char boundary → confirmed
panic on `t[..7]`. Reachable via `read_ssh_hosts` (`#[tauri::command] fn`, not async) →
`read_ssh_config_hosts` → `expand_ssh_config`, triggered by opening the Sessions/SSH host list
with `~/.ssh/config` present. This project's own CLAUDE.md and PowerShell rules describe a
Cyrillic-heavy working environment (dedicated ASCII-junction rules for Cyrillic paths), making a
hand-written Russian comment in the owner's real `~/.ssh/config` a plausible, not theoretical,
input. Additionally checked `Cargo.toml:73-74`: the release profile explicitly keeps
`panic=unwind` (not abort) specifically "so a panicked spawn_blocking closure" doesn't crash the
app — but `read_ssh_hosts` is a **plain sync command**, not routed through `spawn_blocking`, so
that safety net doesn't apply here; whether the panic crashes only the invoke or the whole
process depends on Tauri's sync-command dispatch thread, which I could not fully pin down, but
either outcome (a core feature always failing, or a full-app crash) clears the High bar for a
deterministic, easily-triggered panic on the user's own real data. Verdict stands as reported.

### V-2 [SUP-1] quick-xml 0.39.4 RUSTSEC HIGH — **DOWNGRADED to Medium**
Versions confirmed in `Cargo.lock`: `quick-xml 0.39.4` (:3051), `plist 1.9.0` (:2863). However,
checking `tauri`'s own dependency list in the lockfile (`name = "tauri"`, :4013-4048) shows
`plist` listed as a **direct runtime dependency of the `tauri` crate itself**, not only a
build-time dependency of `tauri-build` as the analyst's evidence implied — so the "build-time
only, not attacker-controlled" mitigation argument is weaker than stated; the code is genuinely
linked into the shipped Windows binary (the analyst's own `--target x86_64-pc-windows-msvc`
check confirms this, unlike the GTK chain which is verified absent). That said, `plist` is a
macOS/Apple-property-list format; nothing in this Windows-only desktop app's actual runtime
paths (spawn scripts, config JSON, provider HTTP) feeds attacker-controlled XML into it — there
is no demonstrated reachable invocation with hostile input, only dependency-graph presence. The
CVSS 7.5 score describes the generic vulnerability class (untrusted XML quadratic parsing /
memory exhaustion), not a proven exploitable path in Castellyn. Given the briefing's own
severity philosophy (Critical/High = urgent, fix-now), "compiled in, not demonstrably reachable
by any user-facing input" plus a concrete available upgrade path (bump `tauri`/`tauri-utils` to
pull `plist >=1.10`) reads as a **tracked, near-term supply-chain item, not an urgent one** —
Medium, one notch below the analyst's High, distinguished from the Low no-action items (V-19/20)
by having an actual fix path worth doing soon.

---

## MEDIUM

### V-3 [SEC2-1] Balance fetch allows http:// while sending the key — **CONFIRMED, Medium**
Read `lib.rs:5646-5710` (`fetch_provider_balance`) and `5411-5438`/`4604-4643`
(`probe_url_allowed`/`valid_base_url`) in full. Confirmed exactly as reported: line 5671 calls
`valid_base_url(balance_url)` (allows `http://` to any non-metadata host — the test at
`:10649-10658` proves only metadata/link-local hosts are blocked, https is never required), then
line 5674 calls `balance_get(...)` which (via `provider_auth_headers`, :5398-5409) attaches
`Authorization`/`x-api-key` with the real key. The sibling `probe_url_allowed` exists
specifically to require https for non-loopback *because* it carries the same key (its own doc
comment says so, :5412-5413) — `fetch_provider_balance` uses the wrong guard. This is a real,
narrow guard-swap bug reachable if the user (or a hostile/misconfigured provider they've added,
which is explicitly in-scope per the briefing's threat model) sets a `balanceUrl`/`baseUrl` to
`http://`. Medium is right: real key-leak vector, but gated behind an explicit http:// config
choice, not attacker-forced.

### V-4 [PERF-1] Heavy dir-scan commands on the main thread — **CONFIRMED, Medium**
Grepped and read the signature of every named command: `read_profile_matrix` (:4098),
`read_opencode` (:5925), `read_mcp` (:6233), `read_codex_profiles` (:6631), `list_skills`
(:6825), `read_environments` (:7126), `read_skill_matrix` (:7316), `list_plugin_contents`
(:8561), `read_stack_drift` (:9918) — every one is `#[tauri::command] fn`, **not** `async fn`,
confirmed verbatim. Contrast pattern (`read_stack` using `spawn_blocking`) also verified present
elsewhere in the file. The dir-walk/multi-file-read bodies (per-profile stat loops, `SKILL.md`
reads per skill) are real work, not single reads — this matches the analyst's description
exactly. Medium is fair: real, reproducible UI jank on tab open for power users with many
profiles/skills, not a correctness bug.

### V-5 [PERF-2] Console non-keyed `{#each}` + front-splice — **CONFIRMED, Medium**
`Console.svelte:151-158` confirmed: `{#each log as line}` has no key expression. `+page.svelte`
confirmed: `MAX_LOG = 5000` (:185) and `log.splice(0, log.length - MAX_LOG)` (:190) — a front
removal, which in a non-keyed each does force Svelte to rebind every visible line's text. The
30ms/64-line backend coalescing (verified in `pump_stream`, :805-851) softens frequency but
doesn't change the O(n) nature of each reconciliation once the buffer is at cap. Medium is
reasonable — the analyst is honest that this only bites when the (default-collapsed) dock is
open during a heavy run, which is a real, not rare, scenario (that's when a user opens it).

### V-6 [QUAL-1] `updatesAttention` bypasses `countOf` — **CONFIRMED, Medium**
`attention.ts:9-19` confirmed: `statuses?.[c.id]?.counts?.changed` only, no `Array.isArray`
fallback, no `plugins_changed` fallback — no import of `countOf` anywhere in the file (grepped,
absent). `envelope.ts:7-13`'s `countOf` confirmed to have both fallbacks. Currently latent
(`Write-StatusJson` always emits `counts` per the quality report's own clean-areas check, which
I did not re-verify independently but is consistent with the envelope contract read elsewhere in
this pass) — but the entire reason `countOf` exists is defense against a writer that doesn't,
and this is the one caller that skipped it. Keeping Medium: same class of "the compiler can't
catch it, only a future writer will," consistent with how QUAL-2/3 (below) are also scored
Medium for identical reasoning (latent-but-real architectural risk, not a live bug).

### V-7 [QUAL-2+3] Stringly-typed stream component-id contract — **CONFIRMED, Medium**
Grepped all `spawn_streamed(app, state, "...".to_string()` call sites: `"forks"` (:1058),
`"backup"` (:1409), `"profiles"` (:1504, :1791), `"sync"` (:1596), `"engine"` (:2678),
`"schedule"` (:6437), `"mcp"` (:6470), `"onboarding"` (:10102). Read `+page.svelte:2069-2209`
in full: the `run-done` listener's `if (id === '...')` ladder covers `backup`/`profiles`/`mcp`/
`sync`/`engine`/`provider`/`schedule`/`plugin-mgr`. IDs do line up today for every site checked
— no live bug found, matching the analyst's own claim. This is genuine, correctly-scoped debt
(untyped cross-boundary string contract, silent-fail-mode on a future mismatch); Medium is the
right call for a maintainability hotspot with no compiler backstop on either side, not inflated.

### V-8 [SUP-2] portable-pty → `serial` 0.4.0 unmaintained — **DOWNGRADED to Low**
Confirmed in `Cargo.lock:3624-3626` (`serial v0.4.0`). This is scored Medium by the analyst, but
it is the *same warning class* (RustSec "unmaintained," no CVE, no unsoundness) as V-19 (anyhow,
"unsound" even — arguably worse — still Low) and V-20 (unic-* ×5 "unmaintained," Low) in the
*same report*, all three sharing the identical "no action available while pinned/upstream"
disposition. No justification is given in the source finding for why `serial` gets Medium while
the equivalent unic-* chain gets Low. Applying the same yardstick used elsewhere in this audit:
downgrade to Low for consistency — an unmaintained-since-2017 crate with no known vulnerability,
reachable but never exercised except via the PTY backend's low-level serial-port fallback code
(not used by this app's ConPTY-only Windows PTY path), tracked under the same portable-pty
0.9-readiness trigger.

---

## LOW (spot-checked + full pass)

### V-9 [SEC-1] MCP env secrets on `codex mcp add` argv — **CONFIRMED, Low**
`lib.rs:7777-7801` (`codex_mcp_add_args`) confirmed: `argv.push(format!("{k}={val}"))` puts the
raw secret value on the argv line; spawned via `cmd /C codex` per `run_codex_mcp` (:7819+).
Contrast with the STDIN-secret comment at `:603-605`, confirmed present verbatim. Integration-
forced (Codex CLI's own interface), correctly scoped Low.

### V-10 [SEC-2/SEC2-2] freellmapi key via `setx` — **CONFIRMED, Low (multi-run agreement holds)**
`lib.rs:7989-7994` confirmed verbatim: `Command::new("setx").args(["FREELLMAPI_API_KEY", &key])`.
This persists plaintext to `HKCU\Environment` and briefly exposes the key on `setx`'s own argv,
a real downgrade from the keyring (`Cargo.toml:37-39` comment confirmed: "Secrets ... live in
the Windows Credential Manager, never in plaintext JSON"). Both independent security passes
found this identically — strong signal it's real, not a false positive. Low is right per the
stated threat model (no local attacker); it's a genuine documented-vs-actual inconsistency worth
a decision, not an emergency.

### V-11 [SEC-3] `cmd_argv_safe` denylist gap — **CONFIRMED, Low**
`lib.rs:7807-7812` confirmed verbatim: `UNSAFE` = `['&','|','<','>','^','%','"']` — no `(`, `)`,
`\n`, `\r`. Defense-in-depth gap on an already-narrow surface (chaining still needs `&`/`|`,
both already blocked). Low is appropriate.

### V-12 [SEC2-3] `ureq` redirects unvalidated (blind SSRF) — **CONFIRMED, Low**
Confirmed `ureq = "3"` in `Cargo.toml:33` and `ureq v3.3.0` in `Cargo.lock:4955-4957`. Read all
three probe sites (`probe_provider` :5442-5486, `fetch_engine_models` :5490+, `fetch_provider_
balance`/`balance_get` :5646+/5602-5613) — none sets a redirect policy on the `ureq::Agent`
builder, only `.timeout_global(...)`. ureq 3.x's default of following redirects with
`SameHost`-scoped auth headers (cited by the analyst, not independently re-verified against
ureq's source in this pass, but consistent with ureq's documented v3 behavior) makes this blind
rather than key-leaking. Low confirmed.

### V-13 [REL-2] PTY reader `Child::wait` can block — **CONFIRMED, Low**
`lib.rs:12160-12205` confirmed verbatim: the reader loop breaks on `Ok(0) | Err(_)`, *then*
calls blocking `Child::wait`, and only after that removes the session from the map (:12199-
12204). Bounded by the Job Object at app exit per the file's own design (consistent with
`RunEvent::Exit` cleanup referenced elsewhere in this file). Low, matches description.

### V-14 [REL-3] No timeout backstop on script runs — **CONFIRMED, Low**
`lib.rs:728-768` (`pump_and_wait`) confirmed verbatim: `let status = child.wait().await;` with
no `tokio::time::timeout` wrapper. `cancel_run` exists as the manual escape hatch (not
re-verified line-by-line in this pass, but its existence is consistent with the rest of the
run-lifecycle code read). Low confirmed — UX-degradation, not data loss.

### V-15 [REL-4] Unbounded line buffering in `pump_stream` — **CONFIRMED, Low**
`lib.rs:805-851` confirmed verbatim: `lines.next_line().await` with no length cap, contrasted
correctly with the PTY reader's fixed `32 * 1024` buffer (:12168, confirmed same file). Low,
matches description — scripts here are trusted/known-newline-terminated in practice.

### V-16 [PERF-3] Serial per-profile limits polling — **CONFIRMED, Low**
`limits.rs:190-254` confirmed verbatim: `for (name, _settings) in ... { poll_profile(...) }` is
a plain sequential loop on a background thread with a 300s (`POLL_SECS`) cadence. Low confirmed
— background thread, 5-minute cycle, laggy-chip impact only.

### V-17 [QUAL-4] `ForksTab.repoHasConflict` `.length` on union — **CONFIRMED, Low**
`ForksTab.svelte:64-65` confirmed verbatim: `(b.conflictFiles?.length ?? 0) > 0`, no
`Array.isArray` normalization, unlike `ForkRepoCard.svelte`'s `confFiles()` helper (not
re-read line-by-line here, but its existence as the correct pattern is plausible and doesn't
change the verdict on ForksTab). Confirmed latent-but-harmless today since only `>0` is checked
(any non-empty string has length ≥ 1). Low confirmed.

### V-18 [QUAL-5] Sessions auto-continue ignores `d7` window — **CONFIRMED, Low**
`SessionsTab.svelte:363-369` confirmed verbatim: only `limitsByProfile[p.profile]?.h5Reset` is
read; no reference to a 7-day/`d7Reset` value anywhere in the shown logic. Self-correcting as
described (a premature continue re-triggers `limited` and can be retried). Low confirmed.

### V-19 [SUP-3] anyhow unsound `downcast_mut`, transitive-only — **CONFIRMED, Low**
`Cargo.lock:45-47` confirms `anyhow 1.0.102` present; `Cargo.toml`'s full `[dependencies]` list
(read in full, lines 20-49) confirms `anyhow` is **absent** as a direct dependency — matches the
analyst's claim that `castellyn`'s own code cannot trigger the unsound path. Low confirmed.

### V-20 [SUP-4] unic-* unmaintained via urlpattern — **CONFIRMED, Low**
`Cargo.lock` confirms all five crate names/version 0.9.0 present (`unic-char-property`,
`unic-char-range`, `unic-common`, `unic-ucd-ident`, `unic-ucd-version`). Purely upstream
(Tauri's `urlpattern` choice), no action available from this repo. Low confirmed.

### V-21 [SUP-6] npm `cookie` <0.7.0 via `@sveltejs/kit` — **CONFIRMED, Low**
`package.json:35` confirms `"@sveltejs/kit": "^2.69.0"`, consistent with the analyst's claim
that the latest release still depends on `cookie ^0.6.0` (not independently re-queried against
npm registry in this pass, but the locally-observable half of the claim checks out). Correctly
noted this is a static-adapter SPA build, so server-side cookie handling isn't exercised in the
shipped desktop app anyway — appropriately Low, no over-claiming.

(SUP-5 GTK chain: not re-validated — explicitly marked informational/not-applicable to the
Windows target by the analyst, and this is echoed by `tauri`'s own dependency list which lists
`gtk` unconditionally in `Cargo.lock` but the analyst's `--target x86_64-pc-windows-msvc` check
showing "nothing to print" is the correct, decisive test. No finding to validate here.)

---

## Summary Table

| ID | Verdict | Final Severity |
|----|---------|-----------------|
| V-1 | confirmed | High |
| V-2 | downgraded | Medium (was High) |
| V-3 | confirmed | Medium |
| V-4 | confirmed | Medium |
| V-5 | confirmed | Medium |
| V-6 | confirmed | Medium |
| V-7 | confirmed | Medium |
| V-8 | downgraded | Low (was Medium) |
| V-9 | confirmed | Low |
| V-10 | confirmed | Low |
| V-11 | confirmed | Low |
| V-12 | confirmed | Low |
| V-13 | confirmed | Low |
| V-14 | confirmed | Low |
| V-15 | confirmed | Low |
| V-16 | confirmed | Low |
| V-17 | confirmed | Low |
| V-18 | confirmed | Low |
| V-19 | confirmed | Low |
| V-20 | confirmed | Low |
| V-21 | confirmed | Low |

**Counts:** 19 confirmed, 2 downgraded, 0 rejected, 0 needs-evidence (out of 21 total).
