# Changelog

All notable changes to **Castellyn** are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] — 2026-07-04

The reliability-and-polish release: two audit-driven improvement runs, a visual redesign, a live
usage-limit monitor for the Anthropic 5-hour window, cross-profile plugin sync, and full profile
hygiene including orphan-directory management.

### Added
- **Orphan profile management** — the Profiles tab now surfaces `~/.claude-<name>` directories on disk that aren't canon profiles (abandoned or foreign configs), each with **Adopt** (register as a real profile) and **Delete** (to the Recycle Bin). Guarded against canon-profile data loss (Windows trailing-space/dot path normalization and case-insensitive bypasses) and against sweeping junction targets — a dir with shared-folder junctions is refused, not recycled.
- **Usage-limit monitor** — watches the Anthropic 5-hour OAuth window and detects "limit reached" from live PTY output; a limited Sessions pane can **auto-continue after its reset** or **switch to another profile**.
- **Cross-profile plugin sync** — on-demand reconcile plus a SessionStart hook that keeps every profile's plugin set aligned.
- **Sessions: agent status** — lifecycle hooks + PTY activity drive per-pane status, with sound and OS notifications on transitions; the previous run's session set is restored after an app restart; `Shift+PgUp/PgDn` scrollback and `Alt+N` pane focus.
- **Durable personalization sidecar** — session personalization survives restarts.
- **Create one missing profile** without reinstalling the rest.
- **Updates** — auto re-check of the whole stack after Update-All, with live per-component progress.

### Fixed
- **Security & reliability hardening** — settings-write races, single-instance handling, status-engine correctness, and confirmed reliability bugs across three hardening waves.
- **Providers** — a failed new-key write now rolls back the migrated slot 0 instead of losing the key.
- **Shortcuts** — global shortcuts re-register teardown-first, so changing one while keeping another no longer silently fails ("already registered").
- **Sync** — the Syncthing GUI address is read from `config.xml` (non-standard ports/binds work) instead of a hardcoded `127.0.0.1:8384`.
- **Environments** — MCP deploy now reconciles: a server removed from the canonical `.mcp.json` is pulled from OpenCode and Codex too (with a "stale" drift badge), instead of lingering as a tail.
- **Home** — profile counts and the "Repair" target match the sidebar badges.
- **Hooks** — Castellyn's hook entries are unwired from orphaned/renamed profiles on disable.

### Changed
- **Visual redesign (waves 1.5–2D)** — a typography + density foundation, an actionable Home cockpit, a rolled-up Updates header with state sections, Providers chips + diagnostics behind Details, and a split Sessions toolbar.
- **Build** — migrated to Vite 8 (Rolldown) + vite-plugin-svelte 7, TypeScript 6, Svelte 5.5; dependency bumps.

## [0.6.1] — 2026-07-02

A same-day hotfix for v0.6.0 plus the Codex fan-outs that complete the multi-environment story.

### Fixed
- **Terminal panes were broken in v0.6.0** — the Unicode-11 width addon (new in 0.6.0) requires xterm's `allowProposedApi` flag; without it the addon threw during pane setup and aborted the whole mount, leaving every session pane empty and unresponsive. If you are on 0.6.0, update.

### Added
- **MCP fan-out to Codex** — one click writes the canonical `.mcp.json` servers into Codex via the official `codex mcp add` CLI (upsert; user-added servers untouched). Verified live: registered servers load in a session and their tools resolve through Codex tool search.
- **Connect the freellmapi gateway to Codex** — writes `[model_providers.freellmapi]` + a `[profiles.freellmapi]` into `~/.codex/config.toml` (format-preserving, your default model untouched) and mirrors the gateway's API key into the `FREELLMAPI_API_KEY` user variable, so `codex --profile freellmapi` runs your local free models with zero manual setup. The raw provider registry is deliberately not fanned out: Codex speaks only the Responses wire API, and chat-completions/anthropic endpoints would silently fail.

### Docs
- **README showcases the Environments tab** (screenshot + feature row, en/ru/zh) and all screenshots are refreshed to the post-audit UI.



The multi-environment release: a new **Environments** tab makes Castellyn aware of every AI-coding harness on the machine (Claude Code, OpenCode, Codex) and pushes the canonical skills / MCP / providers / rules into OpenCode with one click. On top of that: two full improvement waves (35 product items + a 40-item UI/UX audit), an SSH security fix, and richer MCP / backup / analytics management.

### Added
- **Environments tab** — per-harness overview of skills, providers, MCP servers and RTK state; one-click **skill sharing** via directory junctions (with dangling-junction repair); a per-skill × harness coverage matrix; an RTK command-rewriting toggle for OpenCode (Windows-safe plugin pinned to the absolute `rtk` path).
- **Fan-out to OpenCode** — three idempotent merge-patch deploys from the canonical store: MCP servers (`.mcp.json` → `mcp` block), the provider registry (`myproviders.json` → `provider` block; API keys are written only as `{env:…}` references and manually bound keys are never overwritten), and the canonical rule files attached to `instructions[]` by path, no copying. Codex fan-out is deliberately deferred (upstream `config.toml` MCP loading is unreliable).
- **MCP management from the UI** — add/edit/remove canonical servers and clear extras, no more hand-editing `.mcp.json`.
- **Backup** — delete a single snapshot from the UI; weekly maintenance operations.
- **Analytics** — script-run duration histogram by day.
- **Sessions** — Codex terminal tool and a fork-terminal picker; find/new-session moved to `Ctrl+Shift+F` / `Ctrl+Shift+T` so they no longer collide with terminal keys.
- **Forks** — direct fast-forward from the card; **Schedule** — reschedule a task in place; **Providers** — reuse a saved preset when connecting.

### Fixed
- **Security: SSH argv flag-smuggling** — the `sshTarget` validator now rejects values that could smuggle extra `ssh` flags through the session launcher.
- **Reliability** — config/provider data guarded against corruption and key loss; honest run feedback (a `held` component no longer reads as "up to date"; a finished operational run reads its real status envelope); spawn errors surface as a toast and auto-reveal the console.
- **The last active tab is restored again** — the persist effect ran before the restore and overwrote the saved tab, so the app always opened on Updates.
- **Tray actions confirm in a visible window** — Stop stack / Start stack from the tray reveal the window instead of waiting on a confirm dialog nobody can see.
- **Onboarding no longer discards an unsaved scripts folder** — "Next" saves it implicitly.

### Changed
- **UI/UX audit wave (40 items)** — hotkeys and `Ctrl+1…9` follow the visible sidebar order; grouped hotkey cheatsheet; tables fit a 1440-wide window without horizontal scroll; one SVG icon language (no emoji/unicode mix); correct Russian plurals; window chrome dims when unfocused; dialogs submit on Enter; ~50 dead i18n keys removed.
- **Performance** — the on-focus plugin check is throttled, skill scans are memoized, palette/set lookups use `Set` membership.



A hardening pass: backend stability, tighter security defaults, accessibility, faster startup, and power-user keyboard control — no behaviour changes to the workflows themselves.

### Added
- **Power-user keyboard control** — `Ctrl + 1 … 9` jumps straight to a tab, `Esc` cancels the running operation, and the command palette gained direct action verbs (Check all for updates, Refresh forks, Backup now, Open the run log). The shortcut help (`?`) lists them all.
- **Pausable toasts** — hovering or focusing the notification stack pauses its auto-dismiss timers so a toast can actually be read; error/warn toasts announce as `alert`, others as `status`.

### Changed
- **Tighter security defaults** — CSP now sets `object-src 'none'`, `base-uri 'self'`, `frame-ancestors 'none'`; detached monitor/pane windows run under a least-privilege capability (no opener/dialog access) instead of inheriting the main window's grants; `open_url` only opens `http`/`https`; provider ids are validated before use.
- **Faster, leaner startup** — `HubConfig` is parsed once and cached; the elevation check is warmed off the UI path; the xterm/Sessions terminal bundle is now lazy-loaded, so the first paint no longer pulls a ~250 KB chunk that most launches never use.

### Fixed
- **Self-healing run-lock** — the single global run-slot is now released via an RAII guard, so an early-return or panic in any spawn path can no longer leave the app permanently "busy".
- **Terminal sessions are cleaned up on exit** — open PTY sessions are killed when the app quits instead of being orphaned.
- **Honest cancel** — cancelling a run now reports a real failure if the kill fails, while treating "process already gone" (exit 128) as success.
- **Non-blocking terminal input** — `session_write` no longer holds the session map lock across the write, so one busy terminal can't stall input to the others.
- **Safer destructive confirms** — destructive dialogs focus Cancel (not the action) by default, and Enter no longer fires a dangerous or text-gated action; batch/bulk dialogs list exactly what will be affected.
- **No `.bak` for secret files** — atomic writes skip the `.bak` sidecar for `settings.json` / `opencode.json` so a credential file is never copied in plaintext next to itself.

### Accessibility
- Notification region, `aria-current` on the active nav item, `<html lang>` kept in sync with the chosen locale (zh → `zh-Hans`), focus moved into dropdown menus on open, and modal focus targeting via an `initialFocus` hook.

### Internal
- The Rust locale table is now gated for placeholder parity (every `{…}` token must match across ru/en/zh), alongside the existing leaf-key parity check.

### Docs
- **README** gained a **Download** section linking to the latest release and a clear **PowerShell 7 (`pwsh`)** prerequisite — maintenance scripts run under `pwsh`, which Windows does not install by default (en/ru/zh).

## [0.5.2] — 2026-06-26

Forks get smarter at handing work to an AI agent, plus the copy buttons are *actually* fixed and the restore flow is clearer.

### Added
- **Auto-assembled AI prompt for every fork state** — previously only the *conflict* and *dirty* cases produced a prompt; a diverged default branch (needs a manual rebase), a mid-operation/detached repo, and combinations were left with only "Open terminal". The card now inspects the real repo state and assembles one tailored prompt from every detected problem (mid-op, detached HEAD, branch conflicts, diverged/behind/ahead default, dirty/untracked tree, upstream rename/archive, redundant/behind `wip-local`). Every hand-off recommendation copies the prompt as its primary action, and a universal **Copy AI prompt** item is in the ⋯ menu for any repo.

### Fixed
- **Copy buttons — really this time** — the 0.5.1 `execCommand('copy')` fallback is *also* a silent no-op in the WebView2 shell, so copying still failed. Switched to the native clipboard plugin (OS clipboard via Rust); the web paths remain only for the browser/dev harness.
- **Single-file conflict prompt wouldn't copy** — PowerShell's `Select-Object -Unique` returns a scalar string for a single conflicting file (an array only for 2+), and the unguarded `.join` threw, so the click did nothing on repos with exactly one conflict file. The value is normalized to an array at the boundary.
- **Restore could implicitly overwrite `main` (`~/.claude`)** — it is now a first-class, selectable profile in the restore dialog instead of always being restored.
- **Restore plan shown inside the dialog** — the preview/restore output renders as a readable, localized summary (which profiles are overwritten, credential status, what is left untouched) with the raw script output under a collapsible section, instead of streaming only to the run-log behind the modal.
- **Forks status auto-check no longer blocks Backup/Restore** — opening the Forks tab no longer kicks off a status check that held the global run-lock.

### Docs
- **README screenshots refreshed** — 7 tabs at 2720×1800 including the new **Sessions** and **Forks** tabs, captured via a reusable DEV-only mock-IPC harness.

## [0.5.1] — 2026-06-25

Follow-up fixes after the 0.5.0 Forks pass, plus the release/CI pipeline.

### Fixed
- **Copy buttons did nothing** — `navigator.clipboard` is blocked in the WebView2 shell, so every copy button (fork conflict / dirty prompts, secret keys) silently no‑oped. `copyText` now falls back to the legacy `execCommand('copy')` path, which works in WebView2; the "copied ✓" flash fires again.
- **Delete wip-local / Prune failed from the tab** — the entry script never declared or forwarded `-DeleteWip` / `-Prune`, so running either action errored immediately ("a parameter cannot be found"). Both switches are now wired through.
- **Refresh-cancel was undiscoverable** — during a whole‑stack refresh the header button now reads **"Отменить"** and the entire status chip is a click target with an always‑visible ✕ (the wiring was fine; only discoverability was broken). The greyed‑out staggered "reveal wave" is replaced with a soft accent shimmer that keeps every card readable.
- **Light-theme contrast on Forks** — KPI numbers, conflict‑file lines and per‑repo run ✓/✗ used raw colours (≈1.7–2.9:1 on the near‑white card); migrated onto the `statusColor` canon so both themes clear WCAG 4.5:1.

### Internal
- **Release automation** — publishing a GitHub Release now builds the Windows binaries in CI (standalone exe + NSIS setup + MSI) and attaches them, replacing the manual `build_all.ps1` + upload.
- **CI gates + repo hygiene** — GitHub Actions runs `check` / `check:i18n` / vitest / clippy / `cargo test` on push and PR; added CONTRIBUTING and issue/PR templates.
- **Backup tests** — pinned the restore security‑gate and atomic‑write integrity (`cargo test` 28 → 30).

## [0.5.0] — 2026-06-24

A full pass over the **Forks** tab: clearer status wording, redundant-`wip-local` detection, real fork operations (compare / contribute / prune), upstream lifecycle awareness, and a richer repository table.

### Added
- **wip-local redundancy** — the tab now counts a `wip-local` branch's *unique* commits (`git cherry`). When it holds nothing new, the card says "no own commits — can be deleted" and recommends **Delete wip-local** (local, backed up, never pushed; refuses if it still has unique work) instead of "sync it forever".
- **Compare on GitHub** — one click opens the original‑vs‑your‑fork comparison.
- **Contribute back** — branches with unique work get a link straight to the upstream Pull‑Request form.
- **"main has own commits"** — a badge when your default branch is ahead of upstream (the real reason fast‑forward is blocked — usually committing straight to `main`).
- **Upstream lifecycle** — badges for an **archived original** ("dead fork") and a **default‑branch rename** (`master`→`main` drift that silently breaks sync), plus "original updated <when>".
- **Prune stale branches** — delete local branches whose fork branch was already removed (after a merged PR); local, backed up, no push.
- **Richer GitHub table** — Language · ★Stars · Updated columns, an *archived* badge, and description on hover; the wasted right‑hand space is gone. The "Open on GitHub" button is a compact link (no more 3‑line wrap).

### Changed
- **Behind vs diverged** — a branch that is merely behind (fast‑forwardable) is now visually and verbally distinct from one that has *diverged* (needs a manual rebase).
- **Plain‑language status** — reworded the cryptic lines (e.g. "remote — guessed", "uncommitted changes — commit, stash or discard", "resolve conflicts in N branch(es)") with clearer tooltips explaining what they mean and what to do.

## [0.4.0] — 2026-06-24

Parallel sessions grow up: run any tool **locally or over SSH**, spread them across **multiple monitors**, and drive everything from a redesigned launcher — plus a large reliability and DRY hardening pass.

### Added
- **SSH sessions** — run Claude / opencode / a shell on a remote host. Transport is the system `ssh` + your `~/.ssh` (keys, `known_hosts`); no secrets are stored in‑app. Host registry (saved + imported from `~/.ssh/config`) with reachability dots, an optional remote start directory, and inline server management in ⚙ Settings.
- **Multi-monitor** — pop a pane out onto another monitor (`⬈`) or "spread across monitors" as a live move (the session never dies — output fans out to every window). The arrangement persists and can be restored on launch; "forget layout" clears it.
- **Launcher redesign** — an inline *phrase* (`Run {env} [profile] on {location} in {folder} [with {args}]`) with an environment segment (Claude / opencode / shell) and **SSH as a location toggle** rather than a separate tool. Pin a whole phrase to ★ favorites for one‑click relaunch.
- Snippet menu, recent **remote-dir** suggestions (native datalist), keyboard‑shortcut hints in tooltips (Ctrl+T, Alt+1/2/3), and a toast when you pin a favorite.

### Changed
- **DRY consolidation** — a single `openDetached()` for the detach→open‑window flow; the `anchored` popover action now owns "click‑outside to close" for every popover (Select / FolderField / DropdownMenu); the snippet menu reuses the shared `DropdownMenu` (new `glyph` trigger).
- Launch args now mirror the ⚙ default‑args until you edit them, so changing the setting is reflected immediately.
- SSH reachability is probed in parallel instead of one host at a time.

### Fixed
- **Session lifecycle** — a naturally‑exited (but still‑open) pane is now reaped from the session map, freeing its slot and PTY/scrollback; the global session‑limit check and registration are atomic (no race past the cap); the `pane:add` listener is torn down on tab switch (was duplicating returned panes).
- **SSH** — reachability probes every resolved address (IPv6‑first hosts no longer false‑fail); the `~/.ssh/config` parser strips quoted values and honors `Include` (e.g. `config.d/*`).
- **Forks** — per‑repo status is written atomically (no torn read → stale card); the post‑action re‑analysis is guarded so a hiccup can’t abandon the whole run; per‑repo output files use a stable hash (no path collision); global and per‑repo runs are mutually exclusive (no concurrent `git fetch` on the same repo).
- **Popovers** — anchored menus self‑correct their position inside `backdrop-filter`/`transform`/`filter` ancestors (the `⋯` menu on fork cards landing off‑screen).
- **Windows** — a monitor window that fails to build no longer fails silently: the stashed pane is cleared and the pane is recovered into the main grid with a notification.
- Multi‑monitor windows build off the main thread (a synchronous build deadlocked WebView2 → blank window); drag‑reorder works again (native file‑drop disabled on the main window).

### Internal
- Pruned 27 dead i18n keys; ru/en/zh parity enforced (`npm run check:i18n`). Added an SSH‑config quote‑stripping unit test. All gates green: `cargo test`, `npm run check` (0/0), `npm test`, i18n parity, release build.

## [0.3.0] — 2026-06-18

First release under the **Castellyn** name (renamed from AgentHub).

### Added
- New brand: citadel icon, banner, trilingual README, in‑app logo.
- Reusable data tables (sort / search / resize / row‑expand / bulk) across Plugins & skills, MCP, Profiles and the Forks list.
- Skills/plugins **ownership** classification (items from your own local marketplace count as "mine").

_For releases 0.2.x and 0.1.0, see the [GitHub Releases](https://github.com/danscMax/castellyn/releases) page._
