# Cluster B — Fan-out of MCP / Providers / Instructions / Plugins

Read-only investigation. All facts carry `lib.rs:<line>` (or file:line) evidence, verified 2026-07-03.
Backend = `E:\Scripts\Castellyn\src-tauri\src\lib.rs`. Frontend orchestrator =
`E:\Scripts\Castellyn\src\routes\+page.svelte`. Tab = `E:\Scripts\Castellyn\src\lib\components\EnvironmentsTab.svelte`.

## TL;DR

- There are **two distinct fan-out families**:
  1. **Claude-profile fan-out** (canonical config → the 6 CC profiles `~/.claude-<name>\.claude.json`): MCP via `Deploy-Mcp.ps1`; plugins via `plugin_sync.py`.
  2. **Cross-harness fan-out** (canonical config → OpenCode `opencode.json` / Codex `config.toml`): native Rust writers + the `codex` CLI, all fired from the **Среды/Environments** tab.
- **Only ONE thing is automatic: plugin sync** (via a per-profile `SessionStart` hook — and even that hook is only installed when the user flips the toggle). **Everything else is a button the user presses.** There is no scheduler, no watcher, no on-save trigger for MCP/providers/instructions.
- Sources of truth all live under `!Настройки и MCP\ClaudeProfiles\config\`: `.mcp.json`, `myproviders.json`, `CLAUDE.md`, `RTK.md`.

---

## 1. Plugins sync — `plugin_sync.py`

**WHAT** — Reconciles `enabledPlugins` and `extraKnownMarketplaces` across CC profiles. Canonical rule: any plugin enabled `True` in **any** profile is added to every profile **missing that key entirely**; an explicit `False` is a deliberate per-profile opt-out and is never touched. Marketplaces propagate the same way via `setdefault`. `assets/plugin_sync.py:44-77`; header doc `plugin_sync.py:1-16`.

**FROM → TO** — No single "canon" file: the union across all profiles' own `settings.json` is the source, written back into each profile's `settings.json` that is missing keys. Profiles = `~/.claude` + `~/.claude-<name>` for each of the 6 (`ccmy, cc1..cc5`) — but the list is **injected by Castellyn**, not scanned. `assets/plugin_sync.py:23` (`PROFILES = [".claude"]  # castellyn:profiles`), rendered by `render_plugin_sync_script` `lib.rs:7836-7854`; authoritative dir list `plugin_sync_profiles` `lib.rs:7880-7893`. A home-dir scan is deliberately avoided so `~/.claude-mem` etc. are never treated as profiles (`lib.rs:7876-7879`, `plugin_sync.py:12-16`).

**HOW / BY WHAT** — Embedded Python asset shipped to `~/.claude\hooks\plugin_sync.py` (`PLUGIN_SYNC_SCRIPT = include_str!("../assets/plugin_sync.py")` `lib.rs:7812`; path `lib.rs:7817-7819`). Two surfaces (`lib.rs:7806-7810`):
- **`plugin_sync_set(enabled)`** `lib.rs:8025-8062` — wires/unwires the `SessionStart` hook command `py -X utf8 ~/.claude/hooks/plugin_sync.py` (`PLUGIN_SYNC_HOOK_CMD` `lib.rs:7815`) into every profile's `settings.json` via `plugin_sync_wire`/`plugin_sync_unwire` (`lib.rs:7975-7986`). Enable also calls `ensure_plugin_sync_script` (`lib.rs:8028-8030`). Continue-on-error across profiles with retry on transient lock (`lib.rs:8031-8057`, `write_json_atomic_retry`).
- **`run_plugin_sync`** `lib.rs:8065-8091` — the "Sync now" button: runs the script once with `--verbose`, streaming into the console (component `"pluginsync"`).
- **`plugin_sync_status`** `lib.rs:7999-8021` — reports wired/unwired profiles + script version.
- Hook install is **version-gated**: `ensure_plugin_sync_script` rewrites only when disk version < embedded, or same-version with a changed profile list (`lib.rs:7859-7873`); version header `# plugin-sync-version: 2` (`plugin_sync.py:1`, parser `lib.rs:7830-7832`).

**AUTO or user-triggered** — **Hybrid, and the only automatic path in the cluster.** The reconcile fires **automatically at every CC session start** — *but only after the user turns on the toggle*, which installs the `SessionStart` hook. "Sync now" (`run_plugin_sync`) is manual. If the toggle is off, no auto-sync happens. Registered `lib.rs:8386-8388` (`plugin_sync_status, plugin_sync_set, run_plugin_sync`).

**GAP** —
- Only fills **missing** keys; it can never *disable* a plugin across profiles (by design — `False` is sacred). So a plugin removed in one profile stays enabled everywhere else.
- **Fail-open / silent**: SessionStart run swallows all exceptions and `exit(0)` (`plugin_sync.py:99-104`); a broken reconcile leaves profiles silently unsynced. Only `--verbose` (Sync now) reports (`plugin_sync.py:80-96`).
- Skips symlinked `settings.json` (`plugin_sync.py:40`, `lib.rs:7886-7889`) — a profile whose settings is a symlink is silently excluded.
- Propagation is **eventual, not immediate**: added keys "apply at the next start of those profiles" (`plugin_sync.py:93-94`).

---

## 2. MCP fan-out — canonical `.mcp.json`

**WHAT / SOURCE OF TRUTH** — `MCP_CONFIG_REL = "!Настройки и MCP\ClaudeProfiles\config\.mcp.json"` (`lib.rs:5404`). Claude-format servers (`{command, args?, env?}`). Read-only inspection via **`read_mcp`** `lib.rs:5446-5513` (source-of-truth servers vs each profile's live `.claude.json` top-level `mcpServers`; also reports "extras" present in a profile but not canonical). Edited canonically by `mcp_upsert_server`/`mcp_remove_server` (`lib.rs:5532-5564`), serialized behind `MCP_LOCK` (`lib.rs:5516`).

Three fan-out targets from that one source:

### 2a. → Claude profiles (`.claude-<name>\.claude.json`)
- **HOW**: PowerShell `Deploy-Mcp.ps1` (`MCP_DEPLOY_SCRIPT_REL` `lib.rs:5405`), run by **`run_mcp`** `lib.rs:5652-5677` (`action == "deploy"`), optional `-Only a,b` to limit profiles (`lib.rs:5670-5675`). Streamed via `spawn_streamed`, component `"mcp"`.
- **Kept on PS deliberately** (`lib.rs:5646-5651`): the deploy runs `claude mcp add-json <name> <json>`; Rust's `.cmd` escaping mangles the quoted JSON, PowerShell 7 forwards it intact. A native port would have to hand-edit each profile's live `.claude.json` — judged too invasive.
- **AUTO?** User-triggered (MCP tab "deploy" action).

### 2b. → OpenCode (`opencode.json` `mcp` block)
- **HOW**: **`run_opencode_mcp`** `lib.rs:6531-6589`. Reads `.mcp.json`, expands `{{USERPROFILE_FWD}}` (`lib.rs:6538`), translates each Claude server to OpenCode's local shape `{type:"local", command:[cmd, ...args], enabled:true, environment?}` (`lib.rs:6571-6582`). **Merge-patch** into `opencode.json` — overwrites canonical names, preserves user-added servers (`lib.rs:6526-6529`). Atomic write + `.bak` (`write_json_atomic` `lib.rs:6586-6587`). Returns count. Native writer, no script.
- **AUTO?** User-triggered — "Deploy MCP" button on the OpenCode card (`EnvironmentsTab.svelte:211-214`, wired `+page.svelte:799-801`).

### 2c. → Codex (`config.toml` via CLI)
- **HOW**: **`run_codex_mcp`** `lib.rs:6784-6827`. For each canonical server builds `codex mcp add <name> [--env k=v]... -- <command> <args...>` via **`codex_mcp_add_args`** `lib.rs:6742-6766`, then runs `cmd /C codex <argv>` with `CREATE_NO_WINDOW` (`lib.rs:6811-6813`). Codex owns its TOML — never hand-edited; `codex mcp add` is an upsert (`lib.rs:6779-6782, 6740-6741`).
- **Security gate**: `cmd_argv_safe` `lib.rs:6772-6777` rejects (does not silently skip — `lib.rs:6805-6808`) any server whose argv carries cmd metacharacters `& | < > ^ % "` — a `.mcp.json` field is user-editable and could otherwise inject a second command through `cmd`'s re-parse.
- **AUTO?** User-triggered — "Deploy MCP" button on the Codex card (`EnvironmentsTab.svelte:221-223`, wired `+page.svelte:802-803`).

**GAP (MCP)** —
- **Three separate one-shot pushes, no auto-reconcile.** Editing `.mcp.json` does not re-deploy anywhere; the user must remember to click deploy on each of the (up to) three targets. `read_mcp` shows *Claude-profile* drift but there is **no equivalent drift view for OpenCode/Codex** (the Среды cards show only a raw count `mcpServers`, `lib.rs:6209-6214, 6235-6238`).
- Fan-out to OpenCode/Codex only *adds/overwrites* canonical names; a server **removed** from `.mcp.json` is **never removed** from `opencode.json`/Codex (leftovers persist). Same one-directional limitation as plugin sync.
- Codex path aborts the whole run (returns joined errors `lib.rs:6823-6825`) if any single server fails — partial success is reported as failure.
- `codex_mcp_add_args` only forwards **string** env values (`lib.rs:6750`) and string args (`lib.rs:6760`); non-string args are dropped silently.

---

## 3. Providers fan-out — `myproviders.json`

**SOURCE OF TRUTH** — `MYPROVIDERS_CONFIG_REL = "!Настройки и MCP\ClaudeProfiles\config\myproviders.json"` (`lib.rs:3784`), `{ providers: [...] }`.

### 3a. → OpenCode (`opencode.json` `provider` block)
- **HOW**: **`run_opencode_providers`** `lib.rs:6695-6737`. Each registry entry → `(id, shape)` via **`opencode_provider_entry`** `lib.rs:6595-6641`: `id = name.toLowerCase()` (must pass `valid_profile_name`), npm pkg picked by protocol — `@ai-sdk/anthropic` for `protocol == "anthropic"`, else `@ai-sdk/openai-compatible` (`lib.rs:6605-6609`); `model`/`smallModel` split on `;` (`lib.rs:6620-6628`). **Merge** via `merge_opencode_provider` `lib.rs:6646-6688`: canonical npm/name/baseURL win, but an existing `options.apiKey` and existing model entries are **preserved** (`lib.rs:6664-6665, 6672-6681`).
- **Secrets never leave Credential Manager** (`lib.rs:6591-6594, 6690-6693`): `apiKey` is written only as an `{env:<ID>_API_KEY}` reference (`lib.rs:6632`); the real key is never copied into `opencode.json`.
- **AUTO?** User-triggered — "Deploy providers" on the OpenCode card (`EnvironmentsTab.svelte:215-216`, wired `+page.svelte:805-807`).

### 3b. → Codex (freellmapi gateway only — NOT the raw registry)
- **HOW**: **`run_codex_providers`** `lib.rs:6876-6914` → **`patch_codex_gateway`** `lib.rs:6838-6867` (format-preserving `toml_edit`). Writes a `[model_providers.freellmapi]` table (`name=FreeLLMAPI`, `base_url={base}/v1`, `env_key=FREELLMAPI_API_KEY`) + a `[profiles.freellmapi]` (`model_provider=freellmapi`; seeds `model=kimi-k2-thinking` only when absent so a user's choice survives — `lib.rs:6862-6864`). Base URL from `gateway_base_url()` (`lib.rs:6878`). Top-level `model`/`model_provider` never touched (user's ChatGPT default stays — `lib.rs:6833-6834`).
- **Best-effort key mirror**: reads the gateway's unified key from its SQLite via `tools\analytics\unified-key.cjs` (node) and `setx FREELLMAPI_API_KEY` (`lib.rs:6889-6912`) so `codex --profile freellmapi` works in a fresh terminal. Returns *whether the key was set*; key never logged.
- **CAVEAT — the Responses-API restriction**: raw `myproviders.json` entries are **deliberately NOT** fanned out to Codex (`lib.rs:6835-6837`). Codex speaks only the Responses wire API (`WireApi` has no `chat` since 2026-02), so chat-completions / anthropic endpoints would register but silently fail. Only the freellmapi gateway (which ships a `/v1/responses` shim) is connected.
- **AUTO?** User-triggered — labelled "Connect gateway" on the Codex card (`EnvironmentsTab.svelte:224-225`; special-cased in `+page.svelte:808-821` because the result is "was the key mirrored", which picks the toast — success vs warn).

**Which providers go where** — OpenCode: *all* valid registry entries (any protocol). Codex: *only* the freellmapi gateway, and *only* as a Responses-API provider+profile.

**GAP (providers)** —
- **Claude** is not a providers fan-out target here at all (its own providers come from `read_providers()` `lib.rs:6253`; the Среды card just counts them).
- Codex gets exactly **one** provider (the gateway); every other custom provider in the registry is unreachable from Codex by design — no UI cue explains *why* a user's minimax/etc. provider never appears in Codex.
- OpenCode keys are `{env:…}` refs the user must populate manually; nothing verifies the env var is actually set (contrast Codex's `setx` mirror). Card shows only a raw provider count (`lib.rs:6203-6208`), no "key bound?" health.
- Codex key mirror is best-effort and silent-degrading: missing DB/node/helper leaves config connected but key unset, surfaced only as a warn toast (`lib.rs:6887-6888`, `+page.svelte:813-816`).

---

## 4. Instructions fan-out — `run_opencode_instructions`

**WHAT / FROM → TO** — Canonical rule files → OpenCode's `instructions` array. Source = `CANON_RULES_REL` `lib.rs:6918-6921`: `config\CLAUDE.md` + `config\RTK.md`. **`run_opencode_instructions`** `lib.rs:6927-6967`.

**HOW** — Attaches the **paths** (forward-slashed, `lib.rs:6931`), not copies — OpenCode reads them in place so edits propagate with no re-deploy (`lib.rs:6916-6917`). Idempotent merge into `instructions` (dedup by path `lib.rs:6958-6961`), preserves existing user entries. Only files that exist on disk are included (`lib.rs:6932`); returns the connected count (0 ⇒ none exist → actually errors first, `lib.rs:6934-6936`). Atomic write. Native writer.

**AUTO?** User-triggered — "Deploy instructions" on the **OpenCode card only** (`EnvironmentsTab.svelte:217-218`, wired `+page.svelte:823-826`).

**GAP (instructions)** — OpenCode-only. **No Codex or Claude instructions fan-out** exists (`onDeployInstructions` ignores any id != `opencode`, `+page.svelte:824`; the Codex card has no instructions button, `EnvironmentsTab.svelte:220-227`). Since paths (not copies) are attached, moving/renaming the canonical files silently breaks the reference.

---

## 5. The «Среды»/Environments tab — orchestration

**WHAT** — `EnvironmentsTab.svelte` + backend **`read_environments`** `lib.rs:6174-6305`. A **read-only** cross-harness overview: for claude/opencode/codex/zcode it composes existing native readers (no script spawn — `lib.rs:6171-6172`, lazy-loaded on first open `+page.svelte:744-751`) and reports per harness: installed, skills visible/total, plugin-skills-visible, shareable gap, **providers count**, **mcp count**, rtk, config-ok.
- Provider/MCP counts are read straight from each harness's own config: OpenCode from `opencode.json` `provider`/`mcp` (`lib.rs:6203-6214`), Codex from `config.toml` `[model_providers.` / `[mcp_servers.` table tallies (`lib.rs:6231-6238`), Claude MCP from `read_mcp().source.len()` (`lib.rs:6195`).

**HOW it orchestrates fan-out** — It **does not**. The tab renders per-harness **buttons**; each button calls one backend fan-out command through `+page.svelte`:
- OpenCode card → Deploy MCP / Deploy providers / Deploy instructions (`EnvironmentsTab.svelte:211-219`).
- Codex card → Deploy MCP / Connect gateway (`EnvironmentsTab.svelte:220-227`).
- All routed via props `onDeployMcp/onDeployProviders/onDeployInstructions` (`EnvironmentsTab.svelte:33-37`) → `+page.svelte:799-826`, each wrapped by `deployToHarness` (run → toast the count → `reloadEnvs`, `+page.svelte:789-798`). No single "sync everything" action; each is one explicit click per harness.
- Separately, "Share skills" (`onShare`) junctions skills into `~/.agents/skills` (`+page.svelte:828-834`) — a related but different (filesystem-junction) mechanism, not part of the config fan-out.

**AUTO or button?** — **Entirely button-driven.** Fan-out to OpenCode/Codex happens only when the user clicks a deploy button on the tab. The tab's own load is read-only. (The *plugins* SessionStart hook is the sole automatic sync, and it lives in the Plugins tab / `plugin_sync_set`, not here.)

**GAP (Среды)** —
- **No drift/staleness indicator**: the cards show current counts but nothing tells the user the canonical `.mcp.json`/`myproviders.json` changed since the last deploy, so fan-out is easy to forget. `read_mcp`'s Claude-profile `deployedIn` diff has no cross-harness analog.
- Fan-out is **per-harness, per-artifact** (up to 5 separate clicks: OC-mcp, OC-providers, OC-instructions, CX-mcp, CX-gateway) — no "deploy all".
- zcode is a hard-coded not-installed placeholder (`lib.rs:6240-6241, 6289-6303`) — no fan-out path.
- Buttons don't reflect success granularity: `run_codex_mcp` returning a partial failure is toasted as a single error (`+page.svelte:795-797`), the succeeded servers are invisible.

---

## Source-of-truth quick map

| Artifact | Canon path (`abs()` of) | Claude profiles | OpenCode | Codex |
|---|---|---|---|---|
| MCP servers | `config\.mcp.json` (`lib.rs:5404`) | `Deploy-Mcp.ps1` via `run_mcp` (`lib.rs:5652`) — **manual** | `run_opencode_mcp` (`lib.rs:6531`) — **manual btn** | `run_codex_mcp`→`codex mcp add` (`lib.rs:6784`) — **manual btn** |
| Providers | `config\myproviders.json` (`lib.rs:3784`) | — (own `read_providers`) | `run_opencode_providers` (`lib.rs:6695`) — **manual btn** | `run_codex_providers` gateway-only (`lib.rs:6876`) — **manual btn** |
| Instructions | `config\CLAUDE.md`,`RTK.md` (`lib.rs:6918`) | — (is Claude's own) | `run_opencode_instructions` (`lib.rs:6927`) — **manual btn** | — (none) |
| Plugins/markets | union of profiles' `settings.json` | `plugin_sync.py` SessionStart hook (`lib.rs:8025`) — **AUTO when toggle on** | — | — |

Commands registered `lib.rs:10365-10388` (`read_mcp, run_opencode_mcp, run_opencode_providers, run_opencode_instructions, run_codex_mcp, run_codex_providers, plugin_sync_status, plugin_sync_set, run_plugin_sync`).
