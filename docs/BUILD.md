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

- Output exe: `src-tauri\target\release\agenthub.exe` (the binary follows the lowercase crate
  name; the display product name is **Castellyn**).
- The exe is standalone (~9 MB); it reads `SCRIPTS_ROOT` (env → Settings → default `E:\Scripts`),
  so it runs from anywhere as long as the scripts are reachable.

Under the hood: `npm run tauri build` (`@tauri-apps/cli`). Config in `src-tauri/tauri.conf.json`
(`productName: Castellyn`, `identifier: com.danscmax.agenthub`, window `title: Castellyn`).

## App icon

Brand blue `#3b82f6 → #2563eb`. The master is `src-tauri/icons/icon.png` (1024×1024). To change
or regenerate every format:

```bash
python tools/make-icon.py                 # writes the 1024 master to a temp path (prints it)
npm run tauri -- icon "<printed path>"     # regenerates src-tauri/icons/* (ico/icns/png/Square*)
```

`tools/make-icon.py` draws the hub-and-nodes mark with Pillow (`pip install pillow`); no SVG
toolchain needed. The exe embeds `icon.ico` at build time and the tray/window use the bundled
icons, so rebuild after changing icons.

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
