# Castellyn — CANON

The enforced conventions ("canons") of this repo, each mapped to its home in code and its
**adoption guard** — the test that fails CI when new code drifts off the canon. Guards are the
source of truth (executable, can't lie); this file is the human map, rendered from
`plans/canonize-2026-07-11/findings.json`.

**Guard idiom.** Frontend cross-cutting scans extend `src/lib/contrast.test.ts` (vitest reads
`app.css` + every `.svelte <style>` block). Backend scans are Rust `#[test]`s over
`include_str!("lib.rs")`, co-located with the canon. Everything runs in `.github/workflows/ci.yml` and in
`verify.ps1` (`npm run verify`), which is the single source of truth for the gate list:
i18n parity → PSScriptAnalyzer (all tracked `*.ps1`/`*.psm1`) → `npm run check` → `npm test` →
frontend build → `cargo clippy -D warnings` → `cargo test`.

## Visual
| Canon | Home | Adoption guard |
|---|---|---|
| Semantic colors are `--sw-*` tokens — never a raw literal of a token's value | `src/app.css` `--sw-*`, `src/lib/statusColor.ts` | `contrast.test.ts` → *"no .svelte hard-codes a declared --sw-* token value"* (ratcheted, `ALLOW=[]`) |
| White text lands only on a fill that clears WCAG AA | `src/app.css` | `contrast.test.ts` → white-text scan |
| Every `var(--sw-*)` resolves to a declared token | `src/app.css` | `contrast.test.ts` → undefined-token check (white-text surfaces) |
| `--sw-warn` / `--sw-info` express both light & dark | `app.css` `:root` + `.light` | completed 2026-07-11 (light overrides added; consumers routed through the token) |

## Logic / Contracts
| Canon | Home | Adoption guard |
|---|---|---|
| `STREAM_IDS` (TS) ≡ Rust `mod stream_id` | `src/lib/ipc.ts`, `src-tauri/src/lib.rs` | `ipc.test.ts` — cross-language source-scan |
| i18n ru/en/zh leaf-key + `{placeholder}` parity | `src/lib/i18n/` | `scripts/check-i18n-parity.ts` + `i18n/index.test.ts` |
| BOM-tolerant JSON reads | `parse_json_bom` / `read_json_opt` / `read_json_or_recover` (`lib.rs`) | unit tests (`lib.rs`); all JSON reads route through these |
| SSH target arg-injection hardening | `sshTarget` (`ipc.ts`) | `ipc.test.ts` |

## Architecture
| Canon | Home | Adoption guard |
|---|---|---|
| Every process spawn hides its console (`CREATE_NO_WINDOW`) | `CREATE_NO_WINDOW` const (`lib.rs:29`) + convention | **`spawn_window_guard` (`lib.rs`)** — every `Command::new` sets a `creation_flags` call before the next spawn |
| One streamed-run path | `pump_and_wait` (`lib.rs`) ← `spawn_streamed_prog` / `spawn_pwsh_phase` / `spawn_stack_phase` / `run_fork_repo` | by construction — every streamed run must terminate in `pump_and_wait`; the wrappers differ only in slot/registry bookkeeping |
| UTF-8-safe parsing of user files | `str::get` / `split_once` / `from_utf8_lossy` | *backlog* — the 2 known parsers are fixed + regression-tested; a general `clippy::string_slice` guard is too noisy to ratchet (findings CN-5) |
| Config-path migration fallback | `config_path → legacy_config_path` (`lib.rs`) | migration code |

## Security
| Canon | Home | Adoption guard |
|---|---|---|
| `{@html}` receives trusted/static content only | 6 static-icon sites; `Select.svelte` documents the contract | review-gated (no drift; all sources are static consts / bounded enums) |
| Secrets in Windows Credential Manager, never plaintext JSON | `keyring` crate | — |
| Version sync across manifests + release tag | `package.json` / `tauri.conf.json` / `Cargo.toml` | `release.yml` "Verify version sync" step |

## Backlog (unguarded by design — see `plans/canonize-2026-07-11/findings.json`)
- **CN-5** — UTF-8 byte-slice panic class. Both known sites are char-safe + tested; a crate-wide
  `clippy::string_slice` lint is too noisy to adopt cleanly. Revisit if a third instance appears.
- **CN-6** — inline `parse_json_bom` reimplementations were conformed 2026-07-11; no guard (below the
  enforce-cap — a wall of guards is over-enforcement).
