# src/lib — Shared frontend library

## Purpose

Reusable frontend building blocks: the typed IPC boundary, runtime stores, and pure helpers
consumed by `routes/` and by components.

## Ownership

- `ipc.ts` — typed `invoke` wrappers + shared types; the single boundary to the Rust backend
- Stores (Svelte 5 runes, `*.svelte.ts`): `running.svelte.ts`, `runHistory.svelte.ts`,
  `toast.svelte.ts`, `agentStatus.svelte.ts`, `navOrder.svelte.ts`
- Run/UI mapping: `outcome.ts` (run → toast), `attention.ts` (sidebar badges),
  `glossary.ts` (per-component help), `theme.ts` (dark/light)
- Helpers: `envelope.ts`, `bytes.ts`, `clipboard.ts`, `floating.ts`, `limitSwitch.ts`,
  `monitors.ts`, `redact.ts`, `relativeTime.ts`, `sessionMove.ts`, `sessionPrefs.ts`,
  `sessionPresets.ts`, `statusColor.ts`, `updater.ts`, `url.ts`
- `shot/` — screenshot/fixtures for visual work (`fixtures.ts`)

## Local Contracts

- `envelope.ts` parses the status-envelope shape from `<id>.last.json`; keep it in sync with the
  root Status envelope contract and the Rust readers
- Secrets: redact before logging (`redact.ts`); never surface raw tokens in run logs
- Co-located tests (`*.test.ts`) are the contract for `outcome`, `attention`, `ipc`,
  `limitSwitch`, `redact` — update them with behavior changes

## Work Guidance

- Prefer a pure helper here over logic inside a component; keep components presentational
- Reuse existing helpers before adding new ones

## Verification

- `npm test` (vitest runs the co-located `*.test.ts`), `npm run check`

## Child DOX Index

- `components/AGENTS.md` — one component per tab, plus dialogs and shell components
- `i18n/AGENTS.md` — localization: locales, parity gate, translation function
