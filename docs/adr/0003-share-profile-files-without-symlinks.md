# Shared profile files use Claude Code's own mechanisms, not filesystem links

A profile no longer receives seven symlinked copies of the shared configuration files.
Instead each file is shared the way Claude Code itself documents:

| File | How it becomes shared | Link needed |
| --- | --- | --- |
| `CLAUDE.md` | copied into the profile (see the correction below — an import does **not** work here) | no |
| `RTK.md` | copied too: `CLAUDE.md`'s own `@RTK.md` is *relative*, so it resolves next to the copy | no |
| `statusline.py` | `statusLine.command` points at the shared copy by absolute path | no |
| `infra-probe.ps1`, `cleanup_nul.ps1`, `subagent-monitor.ps1` | invoked by hooks / scheduled tasks, which take absolute paths | no |
| `settings.local.json` | Castellyn reconciles it — writes the desired content into each profile and reports drift | no |
| `history.jsonl` | Claude Code state with no configuration knob; shared by link when possible, per-profile otherwise | optional |

Six of the eight links dissolve into documented configuration, one becomes reconciliation,
and one — prompt history — degrades to per-profile when a link cannot be made.

## Correction, 2026-07-21: the `@` import does not reach across the tree

This document originally specified a one-line `@` import for markdown, on the strength of the
documentation quoted below. That was reasoning, not measurement, and it was **wrong for our case**.

Measured with a `CLAUDE.md` that Claude Code demonstrably loaded (a literal marker in the same file
came back), three imports in one file:

| Import | Loaded? |
| --- | --- |
| literal text in the file itself | yes — proves the file was read |
| `@local-probe.md` — inside the same tree | **yes** |
| `@~/probe-marker.md` — outside the tree | **no** |

Relative imports resolve; a cross-tree `~/` import does not. The documented cause is the one-time
approval dialog for external imports ("If you decline, the imports stay disabled") — which an
automated or headless flow can never satisfy, and which a newcomer should not be handed either.

Anthropic's advice still stands *in its own context*: `@AGENTS.md` sits **next to** the `CLAUDE.md`
importing it, so it is a relative import. Our case is cross-tree, and the advice does not carry.

So markdown is **copied**, exactly like settings. A copy can go stale; the snapshot reports these
files and content-drift detection is the reconciler's next step — divergence must be visible, not
silent. Had this shipped as designed, a provisioned profile would have carried no instructions at
all while reporting success.

## Why

The links exist to make content shared. The symlink was the mechanism, never the goal, and
on Windows it is the one mechanism that costs Administrator rights: `mklink` for a *file*
needs elevation unless Developer Mode is on. Measured on the author's machine
(`AllowDevelopmentWithoutDevLicense = 1`), which is exactly why the cost stayed invisible.

Anthropic documents this trade-off and resolves it the same way, for the same reason
([memory docs](https://code.claude.com/docs/en/memory)):

> A symlink also works if you don't need to add Claude-specific content: `ln -s AGENTS.md CLAUDE.md`.
> **On Windows, creating a symlink requires Administrator privileges or Developer Mode, so use the
> `@AGENTS.md` import instead.**

The import syntax carries the load: "Both relative and absolute paths are allowed… Imported files
can recursively import other files, with a maximum depth of four hops." Commands in settings
(`statusLine`, hooks, `apiKeyHelper`) are documented with absolute and `~/`-relative paths and
carry no restriction to the `.claude` tree.

Settings are the exception that forces reconciliation: the
[settings docs](https://code.claude.com/docs/en/settings) define a precedence chain and no
inheritance — there is no `extends`, `include` or import key, and a higher-precedence file
replaces rather than merges. So `settings.local.json` cannot be shared by configuration and must
be shared by something that writes it. That something is the reconciler, which is the direction
the product was already taking.

The profile model itself rests on documented ground: `CLAUDE_CONFIG_DIR` relocates every
`~/.claude` path, so per-profile directories are a supported arrangement, not a trick.

## Considered alternatives

- **Symlink with a one-time elevation prompt.** Honest live links, but a UAC dialog at first run —
  precisely the moment the newcomer-facing work is trying to make frictionless — and a fallback
  path is still needed for users who decline. Rejected: it pays a permanent cost to solve a
  problem the platform already solved.
- **Hard link, falling back to a copy.** Works unelevated on one volume. Rejected on measured
  behaviour: a hard link silently detaches when the file is *replaced* rather than edited in place,
  and replace-in-place is what `write_json_atomic` and Syncthing both do. A sharing mechanism that
  quietly stops sharing is worse than one that visibly refuses.
- **Copy once, then diverge.** Simplest, no rights, no magic — and no sharing after day one.
  Rejected: it deletes the feature instead of implementing it.
- **Keep symlinks and require Developer Mode.** Rejected: it makes a Windows developer setting a
  prerequisite for a desktop app, and the documentation explicitly steers away from it.

## Consequences

- **Elevation leaves the profile model entirely.** Directories use junctions (no rights), files use
  configuration (no rights). No Castellyn code path requests Administrator for linking.
- **A single-profile user has nothing to share.** Links only ever mattered with two or more
  profiles, so the newcomer path loses this whole class of failure rather than handling it.
- **Drift becomes visible instead of impossible.** A symlink made divergence unrepresentable; an
  import or a reconciled file can be edited locally. The reconciler reports that as drift with an
  explicit "align" action, which is the behaviour the declarative direction wants anyway.
- **Prompt history may stop being shared** on a machine where no link can be made. This is a real
  regression against today's setup and is accepted: history is convenience, not configuration.
- **Migration is additive.** Existing symlinked profiles keep working — an import that points at a
  symlinked file resolves the same. The link is removed only when a profile is next reconciled.
- **`integrityWatch` loses most of its job.** Four of its five files no longer exist in profiles, so
  there is nothing to drift; the check narrows to the reconciled file.
- **The status snapshot must be refreshed explicitly.** The replaced script re-ran
  `Get-ProfilesStatus.ps1` at its end so the dashboard reflected the repair. The native engine
  changes the filesystem but not `profiles.last.json`, which only that script writes — so
  `refresh_profiles_snapshot()` runs it once per operation. Without it a successful repair reads as
  a silent failure in the UI, which is the precise complaint this work set out to remove. This is
  the one PowerShell spawn the repair path still makes, and it disappears when
  `Get-ProfilesStatus.ps1` is ported.
- **The cross-process lock is kept, not dropped.** `Repair-ProfileLinks.ps1` is still reachable
  through `Manage-Profiles.ps1`, so Castellyn takes the *same* lock file
  (`%LOCALAPPDATA%\ClaudeProfiles\repair-<name>.lock`) with the same exclusive share mode. Matching
  the path exactly is what makes a Castellyn repair and a PowerShell repair interlock; the
  in-process run slot cannot see another process at all.
