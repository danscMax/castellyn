# Castellyn — Agent Binding Context

How Castellyn wires coding agents to the local LLM stack. This glossary fixes the
vocabulary for the "connect an agent to the stack" surface (Providers tab, profile
provider env, router setup). It is a glossary, not a spec.

## Language

**Harness**:
A coding-agent CLI that Castellyn can launch and wire to a backend (Claude Code,
later Codex / Gemini / OpenCode). In Russian conversation the user says "агент";
in code/docs prefer "harness" to distinguish the CLI from the LLM behind it.
_Avoid_: assistant, bot.

**Binding** (привязка):
Writing the endpoint + auth token + model tiers into a harness's own native config
file so the harness reaches the stack — persistently, surviving restarts and bare
launches. For Claude Code this is the per-profile `settings.json` `env` block.
_Avoid_: connecting, configuring (too vague), "setting the provider".

**Stack endpoint**:
The single network door a harness actually talks to. For Claude Code today this is
**ccr (:3456)**, never freellmapi directly — Claude Code speaks Anthropic and the
gateway speaks OpenAI. "Single endpoint = the gateway" is an aspiration, not the
literal wire for an Anthropic harness.
_Avoid_: "the gateway" when describing what Claude Code points at.

**Gateway**:
`freellmapi` on :13001 — the unified OpenAI-compatible entry to the free backends
(Qwen/DeepSeek/GLM/Kimi). The intended *final* hop of every binding's chain.
_Avoid_: router, proxy, stack (the stack is the whole set of services).

**Router** (ccr):
`claude-code-router` on :3456 — the Anthropic↔OpenAI translator that lets Claude
Code reach an OpenAI backend. Configurable per backend; "connect to the stack"
means pointing ccr at the gateway.
_Avoid_: gateway, bridge.

**Profile**:
One Claude Code configuration directory, `~/.claude-<name>`, selected at launch via
`CLAUDE_CONFIG_DIR`. Each profile can be bound to a different provider independently.
_Avoid_: account, instance, workspace.

**Tier mapping**:
The map from Claude's model tiers (Sonnet / Opus / Haiku) to concrete backend model
names, expressed as `ANTHROPIC_DEFAULT_SONNET_MODEL` / `_OPUS_` / `_HAIKU_` env keys.
_Avoid_: model override (that's the legacy `ANTHROPIC_MODEL` single value).

**Dummy token**:
A non-empty placeholder `ANTHROPIC_AUTH_TOKEN` written when the gateway needs no real
key. Its only job is to get the harness past the "Not logged in" screen; a keyless
local gateway ignores its value. Must never be left empty for a custom base URL.
_Avoid_: fake key, API key (the gateway path uses AUTH_TOKEN, not API_KEY).

## Storage

**Config home**:
`%APPDATA%\castellyn` — where Castellyn keeps what it owns outright. **Today that is
`config.json` and `forks.json` only.** The rest of the configuration (profiles, MCP, providers,
engines, sync rules) still lives under the **scripts root** as `{{PROFILES}}\config\*.json`
(`PROFILES_CONFIG_REL`, `ENGINES_CONFIG_REL`, `MYPROVIDERS_CONFIG_REL`, `SYNC_CONFIG_REL`),
because the PowerShell scripts are co-owners of those files.

Consolidating everything here is a DECIDED but NOT IMPLEMENTED direction — ADR 0002, which is
deliberately `ON HOLD` until the profile scripts are ported to Rust and PowerShell stops reading
`profiles.json`. Do not write code or docs that assume the move has happened.
_Avoid_: "the config folder" when you mean the maintenance scripts' own directory —
that is the **scripts root**, a different thing with a different owner.

**Scripts root**:
`SCRIPTS_ROOT` (default `E:\Scripts`) — where the PowerShell maintenance scripts live, and
today also the home of the configuration they co-own. A machine without it is a normal
machine, not a broken one: the features that need it simply do not apply there.
_Avoid_: calling its absence an error.

## Cross-harness coverage (the «Среды» tab)

**Harness coverage**:
What each harness can *see* — skills, MCP servers, providers, RTK — surfaced read-only per harness
(`read_environments`), plus one-way actions to close gaps. Distinct from **Binding**, which only writes
a provider into one harness's config; coverage is the wider "is this artifact reachable here?" view.
_Avoid_: "sync" — that's Syncthing between *machines*, a different axis.

**Shared skills folder**:
`~/.agents/skills` — the one directory both OpenCode and Codex scan at the user level (Claude's profiles
already share via `~/.claude/skills`). "Share skills" (`share_skills`) junctions every skill here so all
harnesses see one set; re-running self-heals junctions left dangling by a plugin update.
_Avoid_: copying skills per-harness — the junction is the single source of truth.

**Shareable gap**:
Per harness, the count of skills that *sharing would still add* (`shareableGap`). It gates the "share"
button (hidden at 0) and the amber/green skills badge — so the permanent residual (skills that live only
in OpenCode/Codex and can't be pushed into Claude) never reads as an unclosed gap.
_Avoid_: "missing skills" (the n/total residual is not all closable).
