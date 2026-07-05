# DOX framework

- DOX is highly performant AGENTS.md hierarchy installed here
- Agent must follow DOX instructions across any edits

## Project

Castellyn — a Tauri v2 desktop control center for a local AI-coding dev environment. It is a
thin native shell around the user's PowerShell maintenance scripts under `SCRIPTS_ROOT`
(default `E:\Scripts`): a Rust backend spawns those scripts and streams output, a SvelteKit +
Svelte 5 UI renders their `*.last.json` status envelopes. No DB, no sidecar. See
`docs/ARCHITECTURE.md`, and the deeper project rail in `CLAUDE.md` (naming, migration
fallbacks, PowerShell/Cyrillic rules) — that file remains the authoritative project charter;
DOX indexes structure and local contracts, it does not replace `CLAUDE.md`.

## Core Contract

- AGENTS.md files are binding work contracts for their subtrees
- Work products, source materials, instructions, records, assets, and durable docs must stay understandable from the nearest applicable AGENTS.md plus every parent AGENTS.md above it

## Read Before Editing

1. Read the root AGENTS.md
2. Identify every file or folder you expect to touch
3. Walk from the repository root to each target path
4. Read every AGENTS.md found along each route
5. If a parent AGENTS.md lists a child AGENTS.md whose scope contains the path, read that child and continue from there
6. Use the nearest AGENTS.md as the local contract and parent docs for repo-wide rules
7. If docs conflict, the closer doc controls local work details, but no child doc may weaken DOX

Do not rely on memory. Re-read the applicable DOX chain in the current session before editing.

## Update After Editing

Every meaningful change requires a DOX pass before the task is done.

Update the closest owning AGENTS.md when a change affects:

- purpose, scope, ownership, or responsibilities
- durable structure, contracts, workflows, or operating rules
- required inputs, outputs, permissions, constraints, side effects, or artifacts
- user preferences about behavior, communication, process, organization, or quality
- AGENTS.md creation, deletion, move, rename, or index contents

Update parent docs when parent-level structure, ownership, workflow, or child index changes. Update child docs when parent changes alter local rules. Remove stale or contradictory text immediately. Small edits that do not change behavior or contracts may leave docs unchanged, but the DOX pass still must happen.

## Hierarchy

- Root AGENTS.md is the DOX rail: project-wide instructions, global preferences, durable workflow rules, and the top-level Child DOX Index
- Child AGENTS.md files own domain-specific instructions and their own Child DOX Index
- Each parent explains what its direct children cover and what stays owned by the parent
- The closer a doc is to the work, the more specific and practical it must be

## Child Doc Shape

- Create a child AGENTS.md when a folder becomes a durable boundary with its own purpose, rules, responsibilities, workflow, materials, or quality standards
- Work Guidance must reflect the current standards of the project or user instructions; if there are no specific standards or instructions yet, leave it empty
- Verification must reflect an existing check; if no verification framework exists yet, leave it empty and update it when one exists

Default section order:
- Purpose
- Ownership
- Local Contracts
- Work Guidance
- Verification
- Child DOX Index

## Style

- Keep docs concise, current, and operational
- Document stable contracts, not diary entries
- Put broad rules in parent docs and concrete details in child docs
- Prefer direct bullets with explicit names
- Do not duplicate rules across many files unless each scope needs a local version
- Delete stale notes instead of explaining history
- Trim obvious statements, repeated rules, misplaced detail, and warnings for risks that no longer exist

## Closeout

1. Re-check changed paths against the DOX chain
2. Update nearest owning docs and any affected parents or children
3. Refresh every affected Child DOX Index
4. Remove stale or contradictory text
5. Run existing verification when relevant
6. Report any docs intentionally left unchanged and why

## Root-Owned Contracts

- `manifest/maintenance-manifest.json` — canonical list of maintenance components, read from
  disk at runtime (embedded copy in `src-tauri/src/lib.rs` is fallback only). Adding/renaming a
  component touches this file, its backend readers, and i18n. No child AGENTS.md; owned here.
- `build_all.ps1`, `verify.ps1`, `run_dev.bat`, root config (`package.json`, `svelte.config.js`,
  `vite.config.js`, `tsconfig.json`) — build/dev entry points, owned here.
- **Status envelope** contract (every script writes `<id>.last.json`):
  `{ schemaVersion, component, status: ok|changes|error|held, timestamp, mode, durationSec,
  counts:{changed,failed,total}, summary }`. Producers live under `tools/`; consumers under
  `src/lib/` — both must honor this shape.

## Verification (repo-wide gates, keep green before "done")

- `npm run check` — svelte-check, must be 0/0
- `npm test` — vitest (i18n parity, outcome, attention)
- `npm run check:i18n` — ru/en/zh leaf-key parity
- `npm run build` — frontend build
- `.\build_all.ps1` — release exe + shortcut
- Rust: `cargo test` / `cargo clippy` under `src-tauri/` (cargo is not on PATH — use full path,
  trust `$LASTEXITCODE`; see `CLAUDE.md`)

## User Preferences

Durable, project-wide (mirror of the binding rules in `CLAUDE.md`; that file wins on conflict):

- No AI attribution anywhere — commits, PRs, releases, code comments, any file, any language
- Comments in English; user-facing communication in Russian
- Every user-facing string goes through `t('ns.key')`; keep ru/en/zh in parity
- All process spawns set `CREATE_NO_WINDOW` (0x08000000) to avoid console flashes
- Castellyn's own writers emit UTF-8 without BOM; strip `﻿` when reading PowerShell JSON
- Destructive actions run non-interactively (`-Yes -Unattended`) behind a confirm dialog
- DRY: search before adding; reuse `spawn_streamed` (backend) and `common.*` / `askConfirm` (frontend)
- Keep the `agenthub`→`castellyn` migration fallbacks; do not remove legacy read paths
- File paths in replies use full absolute paths

## Child DOX Index

- `src/AGENTS.md` — SvelteKit + Svelte 5 frontend (routes orchestrator, shared lib, components, i18n)
- `src-tauri/AGENTS.md` — Rust / Tauri v2 backend (commands, streaming, native readers, tray, i18n)
- `tools/AGENTS.md` — PowerShell maintenance scripts, `ScriptKit.ps1`, status-envelope producers
- `docs/AGENTS.md` — architecture / i18n / build docs and ADRs
- `plans/AGENTS.md` — historical design specs and prompts (reference, mostly frozen)
