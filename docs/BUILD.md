# Build, release & icon

Windows-first. Requires Node + npm and the Rust toolchain (`cargo`) on PATH (see
`rust-toolchain.toml`).

## Dev

```bash
npm install            # first time
npm run tauri dev      # full app with hot reload
npm run dev            # frontend only (vite)
```

## Quality gates (keep green before "done")

```bash
npm run check          # svelte-check — types + i18n shape. Target: 0 errors / 0 warnings
npm test               # vitest (i18n parity, outcome, attention)
npm run check:i18n     # ru/en/zh leaf-key parity (needs tsx, a devDependency)
```

### Local CI (run all gates at once)

`verify.ps1` runs every gate in order and stops at the first failure. It's the single source
of truth for the gate list — read the file rather than trusting this summary:

1. `npm run check:i18n` — ru/en/zh leaf-key parity
2. **PSScriptAnalyzer** — every tracked `*.ps1`/`*.psm1` (`git ls-files`), rules in
   `PSScriptAnalyzerSettings.psd1`; installs the module once (CurrentUser) if missing
3. `npm run check` — svelte-check (types + i18n shape)
4. `npm test` — vitest
5. `npm run build` — frontend build
6. `cargo clippy --all-targets -- -D warnings` (**not** `cargo check` — warnings are errors)
7. `cargo test`

`.github/workflows/ci.yml` runs the same set; PSScriptAnalyzer there also precedes the cargo
gates and covers the whole repo, not just `tools/`.

```bash
npm run verify         # or: powershell -ExecutionPolicy Bypass -File verify.ps1
```

A committed **pre-push hook** (`.githooks/pre-push`) runs `verify.ps1` and blocks a push if
anything is red. Enable it once per clone:

```bash
git config core.hooksPath .githooks
```

Bypass a single push with `git push --no-verify`.

## Live-verify (dev only): attach to the running window over CDP

Gates are blind to xterm/WebView2 runtime, so UI-affecting changes must be *looked at* in the
live app. A **debug** build (`npm run tauri dev`) opens a Chrome DevTools Protocol endpoint on
`http://127.0.0.1:9222` — set in `run()` behind `#[cfg(debug_assertions)]`, so it is **never
present in a release exe**. Attach a browser tool to that endpoint to observe the real window
(it has the Tauri IPC bridge; a plain browser tab pointed at the Vite URL does **not** — see the
`tauri-webview2-live-verify` skill).

```js
import { chromium } from '@playwright/test';               // global: %APPDATA%\npm\node_modules
const b = await chromium.connectOverCDP('http://127.0.0.1:9222');
const page = b.contexts().flatMap(c => c.pages())[0];      // the live WebView2 window
await page.screenshot({ path: 'shot.png' });
```

- **Do NOT** `navigate` to the dev URL (`127.0.0.1:1420`) — no IPC bridge, backend values come
  up empty, and the `invoke is undefined` errors are an artifact of the method, not app bugs.
- Only one CDP client at a time; a running release exe blocks a second (debug) instance
  (single-instance plugin). Kill it first.
- Build-order gotcha: with `custom-protocol`, `cargo build` embeds `frontendDist` — run
  `npm run build` **before** `cargo build`, or the exe ships a stale frontend.

## Release build

```powershell
.\build_all.ps1                 # standalone release exe + desktop shortcut
.\build_all.ps1 -Bundle         # + NSIS/MSI installers (bundle\)
.\build_all.ps1 -SkipCheck      # skip svelte-check
.\build_all.ps1 -NoShortcut     # don't (re)create the desktop shortcut
.\build_all.ps1 -NoOpen         # don't open Explorer afterwards
```

`build_all.ps1` does: pre-flight (node/npm/cargo) → `npm install` (if needed) → `svelte-check`
→ `tauri build` (`--no-bundle` by default) → (re)create the desktop shortcut **Castellyn.lnk**.

- Output exe: `src-tauri\target\release\castellyn.exe` (the binary follows the lowercase crate
  name; the display product name is **Castellyn**).
- The exe is standalone (tens of MB — check the artifact, don't trust a number here); it reads
  `SCRIPTS_ROOT` (env → Settings → default `E:\Scripts`),
  so it runs from anywhere as long as the scripts are reachable.

Under the hood: `npm run tauri build` (`@tauri-apps/cli`). Config in `src-tauri/tauri.conf.json`
(`productName: Castellyn`, `identifier: com.danscmax.castellyn`, window `title: Castellyn`).

## App icon

Brand blue `#3b82f6 → #2563eb`. The production master is `src-tauri/icons/icon-master.png`
(1024×1024 — an AI-generated citadel/gatehouse emblem). To regenerate every format from it:

```bash
npm run tauri -- icon src-tauri/icons/icon-master.png   # regenerates src-tauri/icons/* (ico/icns/png/Square*)
```

`tools/make-icon.py` is **only an offline, dependency-free FALLBACK** — it draws the legacy
hub-and-nodes mark with Pillow (`pip install pillow`), NOT the shipped citadel icon; use it only when
`icon-master.png` is unavailable. The exe embeds `icon.ico` at build time and the tray/window use the
bundled icons, so rebuild after changing icons.

## First build / after a rename or big change

The Rust target cache can be large (multiple GB) and caches absolute paths. After moving/renaming
the project folder or changing the crate name, run a clean build:

```powershell
cd src-tauri; cargo clean; cd ..
.\build_all.ps1
```

## Troubleshooting

- **Black console window flashes** when an action runs → a `Command` is missing
  `CREATE_NO_WINDOW`. Add it (see `lib.rs`).
- **`tsx` not found** for `check:i18n` → `npm install` (it's a devDependency).
- **Cargo rebuilds everything** after a folder move → expected; `cargo clean` first.
- **Settings reset after a rename** → the WebView store is keyed by the Tauri `identifier`;
  changing it starts a fresh store. The `config.json` (scripts path / timeouts) is migrated by
  `read_config_file()`'s legacy-path fallback.
