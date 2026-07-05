# src — Frontend (SvelteKit + Svelte 5)

## Purpose

The Castellyn UI: a SvelteKit app (static adapter, SPA) using Svelte 5 runes. Renders
maintenance components, streams run logs, and drives all `run_*` / `read_*` backend commands.

## Ownership

- `routes/+page.svelte` — the orchestrator: tab state, all `run_*`/`read_*` invokes, the confirm
  dialog (`askConfirm`/`doConfirm`), run-log + toasts. Most cross-tab wiring lives here.
- `routes/+layout.svelte`, `routes/+layout.ts` — app shell + SPA layout load
- `app.html`, `app.css` — HTML shell and global styles
- Shared logic and UI live under `lib/` (see child index)

## Local Contracts

- Svelte 5 runes only (`$state`, `$derived`, `$effect`); no legacy stores syntax for new code
- Every user-facing string goes through `t('ns.key')` from `lib/i18n`
- Never name an `{#each … as t}` loop var or a param `t` — it shadows the translation function
- Talk to the backend only through typed wrappers in `lib/ipc.ts`, never raw `invoke`
- Custom window chrome: `decorations:false` + `WindowTitleBar.svelte`; repaint with the theme

## Work Guidance

- Reuse existing components and `common.*` i18n keys before adding new ones
- Route a finished run to a toast via `lib/outcome.ts`; sidebar badges via `lib/attention.ts`

## Verification

- `npm run check` (0/0), `npm test`, `npm run check:i18n`, `npm run build`

## Child DOX Index

- `lib/AGENTS.md` — shared library: IPC wrappers, stores, utilities, then components + i18n
