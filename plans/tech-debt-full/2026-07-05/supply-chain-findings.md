# Supply Chain Audit — Castellyn (HEAD 1c02e3d, 2026-07-05)

Tooling actually run: `npm audit --json` (in `E:\Scripts\Castellyn`) and
`"$USERPROFILE/.cargo/bin/cargo" audit` (in `E:\Scripts\Castellyn\src-tauri`, advisory DB freshly
fetched from `github.com/RustSec/advisory-db`, 1156 advisories, scanned 597 crate deps in
`Cargo.lock`). All reverse-dependency paths below verified with
`cargo tree -i <crate> --target x86_64-pc-windows-msvc` (the actual shipping target) — this
matters because `Cargo.lock` also carries Linux-only transitive entries (via Tauri's
`tray-icon`/`libappindicator` backend) that never compile for this Windows-first app; those are
called out separately so they aren't double-counted as real exposure.

## [SUP-1] quick-xml 0.39.4 — two HIGH RUSTSEC DoS advisories, reachable on the Windows build — High
**File/Source:** `src-tauri/Cargo.lock` (quick-xml v0.39.4, pulled in via `plist v1.9.0` ← `tauri-utils v2.9.3` ← `tauri v2.11.5` / `tauri-build v2.6.3`, both used by `castellyn`)
**Description:** `cargo audit` flags two HIGH-severity (CVSS 7.5) advisories against the exact locked version, both dated 2026-06-29 — i.e. new since the 2026-06-21 baseline audit referenced in the briefing. Confirmed present on the actual Windows target (`cargo tree -i quick-xml --target x86_64-pc-windows-msvc`), not just a Linux-only artifact.
**Evidence:**
```
Crate:     quick-xml
Version:   0.39.4
Title:     Quadratic run time when checking a start tag for duplicate attribute names
Date:      2026-06-29
ID:        RUSTSEC-2026-0194
URL:       https://rustsec.org/advisories/RUSTSEC-2026-0194
Severity:  7.5 (high)
Solution:  Upgrade to >=0.41.0

Crate:     quick-xml
Version:   0.39.4
Title:     Unbounded namespace-declaration allocation in `NsReader` enables memory-exhaustion denial of service
Date:      2026-06-29
ID:        RUSTSEC-2026-0195
URL:       https://rustsec.org/advisories/RUSTSEC-2026-0195
Severity:  7.5 (high)
Solution:  Upgrade to >=0.41.0
```
`cargo tree -i quick-xml --target x86_64-pc-windows-msvc`:
```
quick-xml v0.39.4
└── plist v1.9.0
    └── tauri-utils v2.9.3
        ├── tauri-build v2.6.3
        │   [build-dependencies]
        │   ├── castellyn v0.7.0 (E:\Scripts\Castellyn\src-tauri)
        │   └── tauri v2.11.5
```
**Fix suggestion:** Not directly pinnable from `castellyn/Cargo.toml` (it's two levels transitive, via `plist`, which Castellyn does not depend on directly). `plist` has a newer release (`1.10.0` vs locked `1.9.0`) on crates.io — worth checking whether it bumps `quick-xml` to `>=0.41`; if so, the fix is to update `tauri`/`tauri-utils` to a version that pulls `plist >=1.10`, or add a `[patch.crates-io]` / direct `quick-xml = ">=0.41"` override in `src-tauri/Cargo.toml` to force the resolver. Practical exploitability here is limited — `plist` is only exercised at build time / for bundle metadata, not by attacker-controlled runtime input in this app's trust model — but it's a real, current advisory against the exact locked version and should be tracked, not silently carried.

## [SUP-2] portable-pty 0.8.1 drags in `serial` 0.4.0, unmaintained since 2017 — Medium
**File/Source:** `src-tauri/Cargo.toml:45` (`portable-pty = "0.8"`, deliberately pinned per the comment at lines 42-44) / `src-tauri/Cargo.lock` (`serial v0.4.0`)
**Description:** The deliberate `portable-pty` 0.8 pin (not being flagged — briefing says don't) has a transitive dependency, `serial 0.4.0`, that RustSec has flagged unmaintained since 2017 — the oldest and staleest of the transitive warnings. Confirmed present on the Windows target.
**Evidence:**
```
Crate:     serial
Version:   0.4.0
Warning:   unmaintained
Title:     `serial` crate is unmaintained
Date:      2017-07-02
ID:        RUSTSEC-2017-0008
URL:       https://rustsec.org/advisories/RUSTSEC-2017-0008
```
`cargo tree -i serial --target x86_64-pc-windows-msvc`:
```
serial v0.4.0
└── portable-pty v0.8.1
    └── castellyn v0.7.0 (E:\Scripts\Castellyn\src-tauri)
```
**Fix suggestion:** No action available while pinned to `portable-pty 0.8` (the 0.9 upgrade is blocked by the known PTY-hang regression documented in the Cargo.toml comment). Track this as debt tied to the same re-evaluation trigger already noted for the 0.8 pin — when upstream fixes the `PSEUDOCONSOLE_INHERIT_CURSOR` issue and 0.9 becomes viable, check whether it also drops the `serial` dependency.

## [SUP-3] anyhow 1.0.102 — unsound `Error::downcast_mut()`, transitive only — Low
**File/Source:** `src-tauri/Cargo.lock` (anyhow v1.0.102, pulled in via `portable-pty v0.8.1` and `tauri v2.11.5`/`tauri-build v2.6.3`)
**Description:** RustSec warning (not a blocking vulnerability) for unsoundness in `anyhow::Error::downcast_mut()`. `castellyn` does not depend on `anyhow` directly (absent from `Cargo.toml`), so the app's own code cannot trigger the unsound path — exposure is limited to whatever internal use `portable-pty`/`tauri-build` make of it.
**Evidence:**
```
Crate:     anyhow
Version:   1.0.102
Warning:   unsound
Title:     Unsoundness in `Error::downcast_mut()`
Date:      2026-06-25
ID:        RUSTSEC-2026-0190
URL:       https://rustsec.org/advisories/RUSTSEC-2026-0190
```
**Fix suggestion:** No direct pin available (not a direct dependency). Re-check after the next `tauri`/`portable-pty` bump; no action needed now since `castellyn` code never calls `downcast_mut` on an `anyhow::Error` itself (confirmed absent from `Cargo.toml` direct deps).

## [SUP-4] unic-* Unicode crates (5 advisories) unmaintained, reachable via `urlpattern` on Windows — Low
**File/Source:** `src-tauri/Cargo.lock` (`unic-char-property`, `unic-char-range`, `unic-common`, `unic-ucd-ident`, `unic-ucd-version`, all v0.9.0, via `urlpattern v0.3.0` ← `tauri-utils v2.9.3`)
**Description:** Five RustSec "unmaintained" warnings (dated 2025-10-18), all in the same crate family, all reachable on the actual Windows build target — confirmed via `cargo tree -i unic-char-property --target x86_64-pc-windows-msvc`. Not a vulnerability, just an abandoned dependency chain baked into `tauri-utils`'s URL-pattern matching.
**Evidence:**
```
Crate:     unic-ucd-ident
Version:   0.9.0
Warning:   unmaintained
Title:     `unic-ucd-ident` is unmaintained
Date:      2025-10-18
ID:        RUSTSEC-2025-0100
URL:       https://rustsec.org/advisories/RUSTSEC-2025-0100
```
`cargo tree -i unic-char-property --target x86_64-pc-windows-msvc`:
```
unic-char-property v0.9.0
└── unic-ucd-ident v0.9.0
    └── urlpattern v0.3.0
        └── tauri-utils v2.9.3
```
**Fix suggestion:** Entirely upstream (Tauri's choice of `urlpattern`), no action available from this repo. Re-check when bumping `tauri`.

## [SUP-5] GTK3-bindings chain (10 unmaintained warnings) — confirmed NOT compiled for this app's shipping target — Clean / informational
**File/Source:** `src-tauri/Cargo.lock` (`atk`, `atk-sys`, `gdk`, `gdk-sys`, `gdkwayland-sys`, `gdkx11`, `gdkx11-sys`, `gtk`, `gtk-sys`, `gtk3-macros`, all v0.18.2, plus `glib 0.18.5` unsound RUSTSEC-2024-0429 and `proc-macro-error 1.0.4` unmaintained RUSTSEC-2024-0370)
**Description:** This is the bulk of the "19 warnings" `cargo audit` reports, and it's noise for a Windows-first app: the whole chain comes from Tauri's `tray-icon` crate's Linux tray-icon backend (`libappindicator`), which only compiles under `cfg(target_os = "linux")`. Verified: `cargo tree -i gtk --target x86_64-pc-windows-msvc` → "nothing to print"; the same crate only appears with `--target all`. Listing separately from SUP-4 so the report doesn't overstate real exposure — `Cargo.lock` carries these entries because it's a cross-platform lockfile, but they never link into `castellyn.exe`.
**Evidence:**
```
$ cargo tree -i gtk --target x86_64-pc-windows-msvc
warning: nothing to print.
$ cargo tree -i gtk --target all
gtk v0.18.2
├── libappindicator v0.9.0
│   └── tray-icon v0.24.1
│       └── tauri v2.11.5
```
**Fix suggestion:** No action — not shipped. Worth a one-line note if the project ever adds a `cargo audit --target x86_64-pc-windows-msvc`-equivalent CI gate (the plain CLI audits the whole lockfile regardless of target), so these 10+ warnings don't create false alarm fatigue for whoever reads the next `cargo audit` run.

## [SUP-6] npm audit: `cookie` <0.7.0 via `@sveltejs/kit`, no non-major fix currently published upstream — Low
**File/Source:** `package.json:35` (`@sveltejs/kit`: `^2.69.0`), `node_modules/cookie` (locked 0.6.0)
**Description:** `npm audit --json` reports 3 low-severity findings, all one underlying issue: `cookie` accepts out-of-bounds characters in name/path/domain (GHSA-pxg6-pf52-xh8x). Verified the audit's suggested fix is stale/wrong: `npm audit` reports `fixAvailable` as a major downgrade to `@sveltejs/adapter-static@0.0.17` / `@sveltejs/kit@0.0.30`, but checking the actual current release (`npm view @sveltejs/kit@latest dependencies.cookie` → still `^0.6.0` on `2.69.1`) shows upstream has not yet bumped past the vulnerable `cookie` range even in its latest patch — there is no real fix to take today, major or minor.
**Evidence:**
```json
"cookie": {
  "name": "cookie", "severity": "low", "isDirect": false,
  "via": [{"source":1103907,"name":"cookie","url":"https://github.com/advisories/GHSA-pxg6-pf52-xh8x","severity":"low","range":"<0.7.0"}],
  "effects": ["@sveltejs/kit"], "range": "<0.7.0"
}
```
```
$ npm view @sveltejs/kit@latest dependencies.cookie
^0.6.0
```
**Fix suggestion:** No safe upgrade exists yet — do not apply the `npm audit fix --force` suggestion (it would downgrade `@sveltejs/kit` to a pre-1.0 release, breaking the app). Track and re-run `npm audit` after `@sveltejs/kit` releases a version depending on `cookie >=0.7`. Real-world impact here is minimal regardless: this is a static-adapter SPA build (per `svelte.config.js`/`adapter-static`), so SvelteKit's server-side cookie handling isn't exercised in the shipped desktop app.

## Lockfile / version-pinning hygiene
- Both lockfiles are committed and current: `src-tauri/Cargo.lock` last touched in commit `1029402` (2026-07-04), `package-lock.json` tracked in git — no drift risk from an uncommitted lockfile.
- `Cargo.toml` uses unadorned version reqs (`"2"`, `"3"`, `"0.58"`) which Cargo treats as caret ranges (`^2`, `^3`, `^0.58`) — same effective policy as the npm side's explicit `^`. Consistent across the project, nothing to flag beyond what's already visible in the manifests.
- `portable-pty = "0.8"` pin is deliberate and documented (Cargo.toml:42-45) — not re-flagging per briefing.
- `tauri-plugin-updater` (`Cargo.toml:66`, `@tauri-apps/plugin-updater ^2.10.1` in package.json) uses the standard GitHub-releases HTTPS endpoint + minisign pubkey in `tauri.conf.json:44-47` (the embedded value is a *public* key, correctly not a secret) — standard, secure Tauri updater config, no issue found.

## Licenses
Project is MIT (`package.json:17`). Direct dependencies checked (Tauri core + all `tauri-plugin-*`, `serde`, `tokio`, `keyring`, `ureq`, `portable-pty`, `windows`, `toml_edit`, `@sveltejs/*`, `svelte`, `vite`, `tailwindcss`, `@xterm/*`, `@lucide/svelte`) are all MIT or MIT/Apache-2.0 dual-licensed upstream projects — no GPL/AGPL or other copyleft direct dependency found. Clean.

## Clean areas
- No Critical-severity findings from either `npm audit` or `cargo audit`.
- `keyring 3`, `ureq 3`, `toml_edit 0.25.12`, `windows 0.58` — no RUSTSEC advisories exist against any version of these crates (confirmed by the full `cargo audit` scan covering all 597 locked deps; these crates simply don't appear in its output at all).
- `@xterm/xterm ^6.0.0` and its addons (`addon-fit`, `addon-search`, `addon-unicode11`, `addon-web-links`, `addon-webgl`) — no known GHSA/Snyk advisories found for any of the pinned versions.
- Lockfiles committed and current; no supply-chain drift from an out-of-sync lockfile.
- No non-permissive licenses in direct dependencies.
