# Contributing to Castellyn

Thanks for your interest. Castellyn is a Tauri v2 desktop app — a SvelteKit
(static SPA) frontend over a single-file Rust backend (`src-tauri/src/lib.rs`).

## Getting started

```bash
npm install        # first time
npm run tauri dev  # full app, hot reload   (npm run dev = frontend only)
```

You need a recent **Node** (24.x); the Rust toolchain is **pinned to 1.94.0** via
`rust-toolchain.toml` (rustup selects it automatically). On Windows
nothing else is required (WebView2 ships with the OS); on Linux/macOS install the
[Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/).

## Before you open a PR — the green gates

CI runs exactly these; run them locally first so the PR goes green on the first try:

```bash
npm run check        # svelte-check — must be 0 errors / 0 warnings
npm run check:i18n   # ru/en/zh translation-key parity
npm test             # vitest
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml
```

(`cargo` steps need the frontend built once — run `npm run build` first, since the
Rust side embeds `build/` via `generate_context!`.)

## Conventions

- **DRY** — search before adding. Backend: reuse `spawn_streamed`; never add a second
  streaming path. Frontend: reuse existing components, `common.*` i18n keys, `askConfirm`.
- **i18n** — every user-facing string goes through `t('ns.key')`, kept in **ru/en/zh** parity.
  Never shadow the `t` function with a loop var or param named `t`.
- **Destructive actions** gate behind a confirm dialog; scripts run non-interactively.
- Keep diffs focused; one topic per PR. Conventional-commit subjects (`feat(...)`, `fix(...)`)
  are appreciated but not required.

See [`CLAUDE.md`](../CLAUDE.md) and [`docs/`](../docs) for the deeper architecture notes.
