# Bind agents to the stack via per-profile settings.json + a dummy auth token

We make a harness reach the stack by writing `ANTHROPIC_BASE_URL`, a (possibly
dummy) `ANTHROPIC_AUTH_TOKEN`, and the tier-model env keys into the harness's own
native config — for Claude Code, each profile's `~/.claude-<name>/settings.json`
`env` block — rather than relying on injecting those vars into the process
environment at launch time.

## Why

An empirical probe on Claude Code 2.1.177 (isolated `CLAUDE_CONFIG_DIR`, no env
injection) proved that a non-empty `ANTHROPIC_AUTH_TOKEN` in `settings.json` `env`
is enough to skip the "Not logged in / Select login method" screen on a bare
`claude` launch; an empty `settings.json` fails fast with "Not logged in". This
contradicts the prior in-code assumption (that only the *process* env is read for
the auth check) — that assumption is now outdated.

## Considered alternatives

- **OS-level user env vars** (`setx ANTHROPIC_BASE_URL ...`): would also work for a
  bare launch but is **global**, so it cannot give each profile a different provider
  — it breaks Castellyn's multi-profile model. Rejected.
- **Keep the launch-time process-env injection** as the auth mechanism: works only
  when the harness is launched *through Castellyn*, not from a plain terminal — the
  exact fragility we set out to remove. Kept only as harmless redundancy, not relied on.

## Consequences

- The single writer of `settings.json` stays `Manage-Provider.ps1` (extended with a
  dummy-token rule and tier params); no second writer is introduced.
- Binding to a keyless gateway must write a dummy `ANTHROPIC_AUTH_TOKEN`, never remove
  it. The previous "empty token removes the key" path (`Connect-Router.ps1:66` →
  `Manage-Provider.ps1:87`) is a *latent* bug: it makes a bare launch show "Not logged
  in" only for a profile that is ccr/keyless-bound AND has never completed an Anthropic
  login. As of this decision no such profile exists (all profiles carry stored
  credentials), so this is preventive correctness, not repair of a live breakage.
