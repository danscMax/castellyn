# Brief: Wave 1 — rust zone (items 1a, 1b, 1e, 1f)

Repo: `E:\Scripts\Castellyn`. All edits in `src-tauri/src/lib.rs` (+ new i18n key in `src-tauri/src/i18n.rs`).
Comments in English. Match surrounding code style. Do NOT commit. Do NOT touch `.serena/project.yml`, `plans/**`, frontend files, or any EXISTING test (adding new tests is required; modifying/deleting existing tests is forbidden).

## Read first
- `src-tauri/src/lib.rs:6473-6544` — `codex_mcp_add_args` + `run_codex_mcp` (item 1a)
- `src-tauri/src/lib.rs:7144-7152` — the plugin-id charset guard (pattern to mirror)
- `src-tauri/src/lib.rs:3702-3740` — `valid_base_url` (item 1b)
- `src-tauri/src/lib.rs:4447-4489` — `probe_provider` (item 1b)
- `src-tauri/src/lib.rs:385-445` — `RunSlot` / `BulkSlot` RAII patterns (item 1e)
- `src-tauri/src/lib.rs:820-934` — `run_forks` global flag + `run_fork_repo` per-repo map (item 1e)
- `src-tauri/src/lib.rs:3988-4020` — `add_provider_key` (item 1f); also read `kr_get`/`kr_set`/`kr_delete` at 3632-3660
- `src-tauri/src/i18n.rs:82-87` — err.mcp_* key block (where the new key goes)
- Existing tests module: search `mod tests` in lib.rs; see `codex_mcp_add_args` tests at ~6744.

## Contract (FROZEN — flag if it doesn't fit reality, do not silently change)
- Signatures frozen: `probe_provider`, `add_provider_key`, `codex_mcp_add_args` (see contracts-wave1.json).
- cmd metachar reject set for 1a: `& | < > ^ % "` .
- 1b loopback allowlist for http://: host `localhost`, `127.*`, `::1`.
- New i18n key: `err.mcp_unsafe_chars` with `{name}` placeholder, 3 locales (ru/en/zh), added in the err.mcp_* block.

## Tasks

### 1a — reject cmd metachars in `codex mcp add` argv (security)
`run_codex_mcp` spawns `cmd /C codex <argv>`; argv values come from user-editable `.mcp.json` (command, args, env values, server name). cmd re-parses the line, so `&`, `|`, `<`, `>`, `^`, `%`, `"` in any element allow command injection.
- Add a small helper, e.g. `fn cmd_argv_safe(argv: &[String]) -> bool` (or per-string `cmd_safe`), rejecting any element containing a char from the frozen set. Put it next to `codex_mcp_add_args`.
- In `run_codex_mcp`'s loop: after building `argv`, if unsafe → `errs.push(trv("err.mcp_unsafe_chars", cur_lang(), &[("name", &name)]))` (match how other errs there are formatted) and `continue` — do NOT silently skip (must surface in the joined error), do NOT run cmd.
- Also guard `name` itself (it is an argv element — include it in the check; note argv from `codex_mcp_add_args` already contains name at index 2, so checking the built argv covers it).
- Add unit tests next to the existing `codex_mcp_add_args` tests: safe argv passes; each metachar (at least `&`, `|`, `%`, `"`) in command/args/env value/name is rejected.
- Add the i18n key `err.mcp_unsafe_chars` to `src-tauri/src/i18n.rs` (ru: «{name}: значения содержат небезопасные для cmd символы», en: "{name}: values contain characters unsafe for cmd", zh: "{name}: 值包含对 cmd 不安全的字符" — adjust to match neighboring key phrasing style).

### 1b — probe_provider: validate base_url + https-only with loopback exception (security)
`probe_provider` (lib.rs:4449) sends `Authorization: Bearer <key>` to an arbitrary URL with no validation.
- At the top of `probe_provider`: run the existing `valid_base_url(base_url)`; on Err(e) return `serde_json::json!({ "ok": false, "detail": e })`.
- Additionally require https: if the url starts with `http://` and the host is NOT loopback (`localhost`, `127.*`, `[::1]`/`::1`) → return `{ok:false, detail: <i18n>}`. Reuse the host-extraction approach from `valid_base_url` — extract a small testable helper, e.g. `fn probe_url_allowed(base_url: &str) -> Result<(), String>` that does both checks (calls `valid_base_url` + the https/loopback rule), so `probe_provider` just does `if let Err(e) = probe_url_allowed(base_url) { return json!({"ok": false, "detail": e}); }`.
- New i18n key for the https rejection, e.g. `err.https_required` (ru: «нужен https (http разрешён только для localhost)», en: "https required (http allowed only for localhost)", zh: "需要 https（http 仅允许用于 localhost）") — add to i18n.rs near the other err.* url keys.
- Unit tests for `probe_url_allowed`: `https://api.example.com` ok; `http://localhost:8080` ok; `http://127.0.0.1:1234` ok; `http://[::1]:9` ok; `http://api.example.com` rejected; `http://192.168.1.10:1234` rejected; `ftp://x` rejected; metadata host `http://169.254.169.254` rejected.
- Do NOT touch `fetch_engine_models` or other callers — scope is probe_provider only.

### 1e — fork busy-state → RAII (reliability)
Two leak sites when a command future is dropped (webview reload / F5):
1. `run_forks` (lib.rs:833-842): `FORKS_GLOBAL.store(true)` … `store(false)` after `.await` — never runs if the future is dropped mid-await → global forks permanently busy.
2. `run_fork_repo` (lib.rs:870-933): `runs` map insert … remove after `pump_and_wait().await` — dropped future strands the path → that repo permanently busy.
Fix following the `RunSlot`/`BulkSlot` precedent (Drop ALWAYS clears):
- `struct ForksGlobalSlot;` — `reserve()` sets `FORKS_GLOBAL` true (keep the current ordering semantics: set flag FIRST, then check `runs` emptiness in `run_forks`; on the reject path just return Err and let Drop clear). `Drop` stores false.
- A per-repo guard, e.g. `struct ForkRepoSlot<'a>(&'a ForkRuns, String)` — `reserve(runs, path)` does the current locked check (FORKS_GLOBAL busy → Err; contains_key → Err; insert path→0) and returns the guard; a `set_pid(&self, pid)` method updates the entry; `Drop` removes the path from the map. Replace all manual inserts/removes in `run_fork_repo` (including the spawn-error path) with the guard.
- Keep error messages identical (`err.fork_busy`).
- Add a unit test if practical (e.g. reserve → drop → reserve again succeeds for both slots); the async command paths themselves are covered by `cargo test` compiling + existing tests.

### 1f — add_provider_key: don't lose the migrated legacy key on rollback (reliability)
Current bug (lib.rs:4001-4018): on first add, the legacy single key `provider:{id}` is copied to slot 0 and DELETED immediately; if `write_myproviders_raw` then fails, the rollback only deletes the new key's slot — but `keyCount` in JSON is still 0 and the legacy entry is gone → slot 0 is orphaned and the provider silently loses its key.
Decision (frozen): migrate-after-successful-write. Reorder so the legacy `provider:{id}` entry is deleted ONLY after the JSON write succeeds:
- Read legacy via `kr_get`; if present, `kr_set` slot 0 (do NOT `kr_delete` legacy yet), count=1.
- `kr_set` the new key at slot `count`, count+=1; update json fields; `write_myproviders_raw`.
- On write Err: rollback — delete the new key's slot AND (if migration happened this call) delete slot 0; the legacy `provider:{id}` entry is still intact. Return Err.
- On success: if migration happened, `kr_delete` the legacy `provider:{id}` now. Return Ok.
- For the required unit test, extract the transactional core into a testable helper parameterized over the key-store ops (closures or a tiny trait), e.g. `fn append_key_txn(get/set/del closures, write: impl FnOnce()->Result<(),String>, ...)` — keep it minimal; `add_provider_key` calls it with the real `kr_*`/`write_myproviders_raw`. Unit test with an in-memory HashMap store: (a) write fails on first add → legacy entry still present, no orphan slots; (b) write succeeds → legacy gone, slot0=legacy value, slot1=new key.
- Behavior for non-first adds must stay byte-identical.

## Out of scope
Everything else in lib.rs; frontend; `Cargo.toml` (do not add dependencies); existing tests; any git command that mutates state.

## Verify (must all pass before you report done)
```
cd src-tauri && C:\Users\User\.cargo\bin\cargo.exe test
```
(cargo is NOT on PATH — use the full path; trust the printed result, PowerShell may report a false exit code.)

## Definition of done
- All 4 items implemented per contract; new unit tests exist for 1a, 1b, 1f (and 1e if practical) and pass.
- `cargo test` green (49 existing + your new ones), zero warnings introduced (`cargo test` output clean of new warnings).
- No existing test modified. No new dependency.

## Report back
Per item: what changed (file:line ranges), new tests added (names), cargo test summary line, anything that didn't fit the contract (flag explicitly — do not improvise around it).
