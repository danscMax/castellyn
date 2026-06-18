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
