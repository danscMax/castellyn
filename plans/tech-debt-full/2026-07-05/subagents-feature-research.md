# Cross-engine / cross-provider subagents for Claude Code — research

**Date:** 2026-07-05
**For:** Castellyn (Tauri desktop control center for a local Claude Code stack)
**Question:** While running ONE Claude Code session (some profile), spawn SUBAGENTS that run on a
DIFFERENT engine (OpenAI Codex CLI) or a DIFFERENT provider (the freellmapi gateway / another
OpenAI-compatible endpoint) — mix engines/providers inside a single session.

> Every factual claim below is tagged with the source it came from. Items I could not fully verify
> are marked **[UNCERTAIN]**.

---

## TL;DR

The user is right — this exists, in three architecturally different shapes:

1. **Model-as-tool via MCP** — the other engine/model is exposed to Claude Code as an MCP *tool*,
   not a real Claude subagent. This is the most robust and Windows-friendly shape today.
   Two families: **Codex-as-MCP** (`codex mcp-server`, "a subagent that is literally Codex") and
   **multi-model orchestrators** like **zen-mcp-server / PAL MCP** (consult GPT-5/Gemini/Grok/any
   OpenAI-compatible endpoint as tools).
2. **Proxy/gateway routing** — **claude-code-router (CCR)** intercepts Claude Code's HTTP calls and
   rewrites each *per request type* to a different provider. It has an explicit subagent hook
   (`<CCR-SUBAGENT-MODEL>provider,model</CCR-SUBAGENT-MODEL>`) and a `background`/`subagent` route.
   Castellyn **already manages CCR** (the `ccrrouter` component).
3. **Native Claude Code subagents with a provider field** — does **not exist yet** (open feature
   request), and even the *model*-only routing that does exist is currently **reported broken**
   (open bug, Windows). Least viable path.

**Recommendation:** integrate **(1b) Codex-as-MCP** and **(2) CCR provider routing** first — both
ride plumbing Castellyn already owns (per-profile MCP deploy fan-out; codex `config.toml` via
`toml_edit`; ccrrouter config; Credential-Manager keys; freellmapi as an OpenAI-compatible
provider). Both bypass Claude Code's own (buggy) subagent-model resolution, so they actually work.

---

## Landscape (per project)

### A. Codex CLI as an MCP server ("a subagent that is Codex")

**What it does.** Codex CLI can run itself as an MCP server so any MCP client (Claude Code,
Claude Desktop, Cursor) can call Codex as a tool. Native command: adding
`{"mcpServers":{"codex":{"command":"codex","args":["mcp-server"]}}}` to the client config exposes
Codex as a callable tool. Codex can also *consume* MCP servers the other direction.
Source: OpenAI Codex docs — MCP server config is added in `~/.codex/config.toml` or via the
`codex mcp` CLI, "Codex launches them automatically when a session starts and exposes their tools
next to the built-ins." (https://developers.openai.com/codex/config-reference,
https://community.openai.com/t/sync-codex-and-claude-code-configs-skills-agents-mcp-permissions/1380517)

**Third-party wrappers** (thin MCP servers that shell out to `codex exec`), all similar in spirit:
- `tuannvm/codex-mcp-server` — "MCP server wrapper for OpenAI Codex CLI that enables Claude Code to
  leverage Codex's AI capabilities directly." (https://github.com/tuannvm/codex-mcp-server)
- `kky42/codex-as-mcp` — "small MCP server that lets MCP clients (Claude Code, Cursor, etc.)
  delegate work to the Codex CLI." (https://github.com/kky42/codex-as-mcp)
- `LanceVCS/codex-mcp` — "Stateful MCP server for Codex CLI." (https://github.com/LanceVCS/codex-mcp)
- `cexll/codex-mcp-server`, `mr-tomahawk/codex-cli-mcp-tool` (LobeHub listing).

**Windows.** Native. "On Windows, Codex uses the native Windows sandbox when you run in PowerShell
and the Linux sandbox implementation when you run in WSL2." Config the native sandbox in
`config.toml` under `[windows] sandbox = "unelevated"|"elevated"`.
(https://developers.openai.com/codex/concepts/sandboxing)

**Headless gotcha [IMPORTANT].** Non-interactive `codex exec` that itself calls MCP tools currently
"hits a wall" — the only path is `--dangerously-bypass-approvals-and-sandbox`, OR set in
`config.toml`: `sandbox_mode = "danger-full-access"` + `approval_policy = "never"`. Open issue.
(https://github.com/openai/codex/issues/24135,
https://developers.openai.com/codex/agent-approvals-security). For Castellyn's *outer* use (Claude
Code calling Codex-as-a-tool to write code) this means the deployed `codex mcp-server` needs an
approval policy configured up front or every tool call blocks. This is exactly the kind of
config-file surgery Castellyn already does via `toml_edit`.

**Maturity.** OpenAI-official CLI; MCP-server mode is documented and stable. Wrappers are small,
young community repos (hundreds of stars range) — prefer the **native `codex mcp-server`** over a
wrapper unless a wrapper's stateful-session feature is needed.

### B. zen-mcp-server → now "PAL MCP" (BeehiveInnovations) — multi-model orchestrator as tools

**What it does.** MCP server that lets Claude Code "consult with multiple AI models within a single
prompt," keeping conversation continuity across models — multi-model code review, consensus,
debugging. Models appear as **callable tools** (`chat`, `consensus`, `clink`, …), *not* as
independent Claude subagents. "Claude stays in full control."
(https://github.com/BeehiveInnovations/zen-mcp-server,
https://github.com/BeehiveInnovations/pal-mcp-server)

**Providers.** Gemini, OpenAI (GPT-5, O3), Azure OpenAI, X.AI (Grok), OpenRouter, DIAL, Ollama, and
**any OpenAI-compatible API via a custom endpoint** — the explicit hook for the user's freellmapi
gateway. (README + https://claudelog.com/claude-code-mcps/zen-mcp-server/)

**`clink` = "CLI subagents" [relevant].** PAL's `clink` tool can "launch isolated CLI instances from
within your current CLI" — i.e. spawn Codex/Gemini/Claude CLI as an external subagent that returns
only final results. This is the closest thing to "spawn a real other-engine subagent" in the
MCP-tool world. (pal-mcp-server README)

**Windows.** Via **WSL** — repo ships a `docs/wsl-setup.md`. No documented native-Windows path.
(README). This is a **minus** for Castellyn (Windows-native app; WSL dependency is friction).

**Maturity.** ~10.9k stars, Apache-2.0, updated Jan 2026 — the most mature multi-model MCP
orchestrator. (https://github.com/beehiveinnovations/zen-mcp-server)

**Install shape.** MCP entry in client config, e.g. `command: "bash", args: ["-c", "uvx --from
git+https://…pal-mcp-server pal-mcp-server"]`, env holds `GEMINI_API_KEY` / `OPENAI_API_KEY` /
custom endpoint + `DISABLED_TOOLS`. (README)

### C. claude-code-router (CCR) — proxy that routes per-request to any provider

**What it does.** Open-source proxy that "intercepts requests, rewrites them for whatever provider
you point it at, and picks the model per request type." Claude Code talks to CCR (via
`ANTHROPIC_BASE_URL`), CCR fans out to real providers.
(https://github.com/musistudio/claude-code-router, https://www.datacamp.com/tutorial/claude-code-router)

**Per-request Router keys.** `default` (general coding), `background` (cheap fast tasks like diff
summaries/compaction), `think` (planning), `longContext` (past `longContextThreshold`, default
60000 tokens), `webSearch`, `image`. Each key maps to a `provider,model`.
(https://www.datacamp.com/tutorial/claude-code-router,
https://devtools.shingoirie.com/blog/en/claude-code-router-model-switching-guide/)

**Subagent routing [KEY FEATURE].** Prefix a subagent prompt with
`<CCR-SUBAGENT-MODEL>provider,model</CCR-SUBAGENT-MODEL>` to force *that* subagent onto a specific
provider/model — "run exploration subagents on a cheap model and reserve the expensive one for the
subagent doing the actual edits." Provider e.g. `openrouter|deepseek|ollama|gemini`, model e.g.
`anthropic/claude-sonnet-4|deepseek-chat`.
(https://www.morphllm.com/claude-code-router — confirmed by the DataCamp guide)

**Providers.** "OpenAI-compatible APIs, Anthropic Messages, Gemini, OpenRouter, DeepSeek,
SiliconFlow, Moonshot, Kimi, Mistral, Z.AI, Bailian, and custom providers." Config per provider:
provider name, base URL, protocol, API key, model list. → **freellmapi drops straight in as an
OpenAI-compatible custom provider.** (musistudio README)

**Windows.** Yes — ships a `Claude Code Router_<version>.exe`. (README)

**Maturity.** ~35.6k stars, 2.9k forks, 28 contributors, very active — the dominant router.
(README). Castellyn **already integrates it** as the `ccrrouter` maintenance component.

### D. Native Claude Code subagents (`.claude/agents/*.md`, `model:` frontmatter)

**What exists.** Subagents are markdown files with YAML frontmatter in `.claude/agents/`
(project) or `~/.claude/agents/` (personal); frontmatter has a `model:` field (`sonnet|opus|haiku`
or a full id). (https://code.claude.com/docs/en/sub-agents)

**Provider is session-wide, not per-agent.** "Currently, `ANTHROPIC_BASE_URL` and model provider
configuration is **session-wide**. There's no way to route individual subagents to different
providers within the same session." Open feature request #38698 asks for a per-agent
`provider`/`base_url` field; **unimplemented, no maintainer response.** Workaround listed there:
"Run separate terminal sessions with different `ANTHROPIC_BASE_URL` values" — which loses the
orchestrator→subagent model entirely. (https://github.com/anthropics/claude-code/issues/38698)

**And the model-only routing is currently broken [IMPORTANT CAVEAT].** Open bug #43869: all five
documented mechanisms (`Agent(model:…)`, frontmatter `model:`, `CLAUDE_CODE_SUBAGENT_MODEL` env in
two forms, `settings.json` env) "silently ignore routing directives and always execute using the
parent session's model." Reporter ran 15 subagents; the target model stayed at 2% usage. **Open as
of 2026-04-05, Windows 11 / Opus, no maintainer fix.**
(https://github.com/anthropics/claude-code/issues/43869)

> Consequence for design: any approach that leans on Claude Code's *own* subagent model resolution
> is unreliable right now. The **proxy** (CCR intercepts at the HTTP layer, before CC's resolution)
> and the **MCP-tool** (Codex/PAL are tools, not subagents) approaches sidestep this bug entirely.

### E. CLI-Agent-Orchestrator (CAO, AWS Labs) — real per-engine CLI subagents in tmux

**What it does.** "One supervisor agent launches, messages, and coordinates multiple worker agents —
each one a real CLI tool (Claude Code, Kiro, Codex, etc.) running in its own tmux terminal." Pin a
worker's engine via `provider` frontmatter: `kiro_cli|claude_code|codex|antigravity_cli|hermes|
kimi_cli|copilot_cli|opencode_cli|cursor_cli`. Headless/`--yolo`/`--async` for CI.
(https://github.com/awslabs/cli-agent-orchestrator)

**Windows.** ✗ — requires **tmux 3.3+**, macOS/Linux only; no documented Windows or WSL path. This
is the cleanest "real multi-engine subagents" model conceptually but the **worst Windows fit** for
Castellyn. ~790 stars, Apache-2.0, v2.2.0 (June 2026), active. Keep on the radar; not first.

### F. Others (noted, not deep-dived)

- **Every Code (`just-every/code`)** — a Codex-CLI fork with "multi-agents … multi-provider
  orchestration (OpenAI, Claude, Gemini)." A whole alternative engine rather than a Claude-Code
  subagent mechanism. (search: bradAGI/awesome-cli-coding-agents,
  https://www.tembo.io/blog/coding-cli-tools-comparison)
- **`mkXultra/ai-cli-mcp`** — "MCP server to run Claude, Codex, and Gemini CLI agents in the
  background from any MCP client." Same family as Codex-as-MCP but multi-engine.
  (https://github.com/mkXultra/claude-code-mcp)
- **Warp / Conductor / Shipyard / vibe-kanban–style** — GUI orchestrators that run several agent
  CLIs side by side; parallel *sessions*, not in-session cross-provider subagents. Out of scope for
  "mix inside one session."

---

## Approaches ranked (integration effort vs capability, for Castellyn on Windows)

| # | Approach | What the user gets | Windows | Reliability vs CC bugs | Castellyn effort | Fit |
|---|----------|--------------------|---------|------------------------|------------------|-----|
| 1 | **CCR provider routing** (`background`/`subagent` route + `<CCR-SUBAGENT-MODEL>` tag) → freellmapi & others | Subagents/background traffic on a different provider, in-session | ✅ native | ✅ HTTP-layer, bypasses #43869 | **Low** — already the `ccrrouter` component; add route config + a freellmapi provider entry | ★★★★★ |
| 2 | **Codex-as-MCP** (`codex mcp-server`) deployed per profile | "A subagent that is literally Codex" as a callable tool | ✅ native sandbox | ✅ tool call, not a CC subagent | **Low–Med** — reuse per-profile MCP fan-out + `toml_edit` for approval policy | ★★★★★ |
| 3 | **PAL / zen-mcp** multi-model tools (incl. `clink` CLI subagents, any OpenAI-compatible endpoint) | Consult GPT-5/Gemini/Grok/freellmapi as tools; spawn CLI subagents | ⚠️ WSL only | ✅ tool call | **Med** — MCP deploy + WSL/uvx dependency + more keys | ★★★☆☆ |
| 4 | **CAO** real per-engine CLI subagents | True multi-engine worker agents | ✗ tmux/Linux | ✅ separate processes | **High** — needs tmux; wrong OS | ★★☆☆☆ |
| 5 | **Native per-agent provider** (`.claude/agents` + provider field) | Cleanest UX if it existed | n/a | ✗ unimplemented (#38698) + model routing broken (#43869) | blocked upstream | ★☆☆☆☆ |

---

## Recommendation

**Integrate #1 (CCR) and #2 (Codex-as-MCP) first — they are complementary and both land on plumbing
Castellyn already owns.** Together they deliver the user's exact wish: inside one Claude Code
session, some subagents run on a *different provider* (CCR → freellmapi / OpenRouter / etc.) and one
can be *a different engine* (Codex via MCP).

**Why these two, concretely:**

- Both **sidestep the two upstream blockers** (native per-agent provider is unimplemented #38698;
  native subagent *model* routing is currently broken #43869). CCR rewrites at the HTTP layer before
  Claude Code resolves a model; Codex-as-MCP is a tool call, not a subagent — neither depends on
  CC's internal subagent resolution.
- Both are **Windows-native** (CCR ships an `.exe`; Codex uses the native Windows sandbox).
- Both reuse existing Castellyn capabilities.

**What the integration would touch:**

1. **CCR route config (ccrrouter component).**
   - Add the user's **freellmapi gateway as an OpenAI-compatible provider** in CCR's provider list
     (name, base URL, key from Windows Credential Manager, model list) — Castellyn already writes
     provider config and stores keys in the keyring.
   - Add a **`background`** (and/or `subagent`) route → freellmapi (or a cheaper provider). Highest
     leverage, invisible-token win, zero prompt changes.
   - Surface the **`<CCR-SUBAGENT-MODEL>provider,model`** convention in the UI as a documented
     "route this subagent to X" affordance (glossary/help text), since it's a prompt-prefix, not a
     config knob.
   - Verify with a live session that `background`/subagent calls actually hit freellmapi (CCR logs /
     freellmapi logs) — do **not** trust green build; per project rules, verify the real effect.

2. **Codex-as-MCP deploy (per-profile MCP fan-out + `toml_edit`).**
   - Add a **`codex` MCP entry** (`command: codex`, `args: ["mcp-server"]`) to the per-profile MCP
     deploy set (the same fan-out that deploys context7/serena, etc.). Note: this is deploying an
     MCP server *into Claude Code profiles*, distinct from Castellyn's existing Codex-side
     `config.toml` management.
   - Pre-configure Codex's **approval policy / sandbox** in `~/.codex/config.toml` via `toml_edit`
     (`approval_policy`, `[windows] sandbox`) so headless tool calls don't block on the #24135
     approval wall. Choose the *least* permissive mode that still lets tool calls through; gate any
     `danger-full-access` behind a confirm dialog (destructive-action rule).
   - Keys (OpenAI) from Credential Manager, consistent with existing provider handling.

3. **Sessions tab (optional, later).** A launch recipe that starts a Claude session with CCR active
   (`ANTHROPIC_BASE_URL` → CCR) so the mixing is on by default for that session — Castellyn already
   builds PTY launch recipes for `claude`/`codex`/ssh.

**Defer** PAL/zen-mcp (#3) until there's demand for multi-model *consultation* (GPT-5+Gemini+Grok
consensus) beyond Codex — its WSL dependency clashes with the Windows-native design, though its
OpenAI-compatible custom-endpoint support would also expose freellmapi as tools if wanted later.
**Skip** CAO (#4) on Windows (tmux). **Do not** build on native per-agent provider (#5) until #38698
ships and #43869 is fixed.

---

## Open / uncertain items (flagged, not asserted)

- **[UNCERTAIN]** Whether `<CCR-SUBAGENT-MODEL>` and the `background` route are unaffected by CC bug
  #43869 in *current* CC versions — architecturally they should be (HTTP-layer), but I did not find
  a source explicitly testing CCR subagent routing against a post-#43869 build. Verify live.
- **[UNCERTAIN]** freellmapi's exact OpenAI-compatibility surface (does it accept the model ids /
  streaming CCR sends?) — needs a live probe against the gateway, not documented externally.
- Codex-as-MCP headless approval (#24135) is an **open OpenAI issue**; the `approval_policy="never"`
  + sandbox config is the current workaround, may change.
- CC feature #38698 (per-agent provider) and bug #43869 (subagent model routing) are both **open**
  as of the dates cited; re-check before relying on native paths.
