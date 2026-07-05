# src/lib/i18n — Localization

## Purpose

Runtime localization for the UI. Three locales in strict leaf-key parity: `en`, `ru`, `zh`.

## Ownership

- `index.ts`, `index.svelte.ts` — the `t()` translation function and reactive locale state
- `locales/<lang>/*.ts` — one namespace file per feature (e.g. `sessions`, `profiles`, `mcp`,
  `providers`, `forks`, `sync`, `schedule`, `backup`, `analytics`, `settings`, `updates`,
  `plugins`, `environments`, `onboarding`, `health`, `nav`, `page`, `console`, `titlebar`,
  `glossary`, `common`, `myProviders`), aggregated by `locales/<lang>/index.ts`
- `index.test.ts` — parity + shape gate

## Local Contracts

- ru/en/zh must have identical leaf keys — enforced by `npm run check:i18n` and `index.test.ts`
- Add a key to all three locales in the same namespace file, then use `t('ns.key')`
- Never shadow `t`: no `{#each … as t}`, no param named `t`
- Shared/generic strings live in `common.*`; do not duplicate them per namespace

## Work Guidance

- New namespace → create `locales/<lang>/<ns>.ts` for all three langs and register in each
  `locales/<lang>/index.ts`
- Backend/tray strings are localized separately in `src-tauri/src/i18n.rs` — keep concepts aligned

## Verification

- `npm run check:i18n` (leaf-key parity), `npm test` (runs `index.test.ts`)

## Child DOX Index

None.
