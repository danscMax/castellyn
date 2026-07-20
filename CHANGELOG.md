# Changelog

All notable changes to **Castellyn** are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.1] — 2026-07-20

An audit-and-repair release. A full global audit produced 113 findings; 112 are fixed here, along
with three problems the audit itself missed and two of its own conclusions that turned out to be
wrong. No new features — this release is about the build being trustworthy and the app not lying to
you.

**If you are on 0.7.0, this is the release to take.** The 0.7.0 artifacts were built by a workflow
whose gate steps did not exist yet, so they shipped without a single check having run, and they also
predate the 15 fixes from the previous audit.

### Fixed — the build could report success without doing anything

- **CI had been failing on every run since 07-14 and nobody could see it.** The gates were only ever run locally, where they passed. Three tests failed on a clean machine; two of them were real defects (below).
- **`verify.ps1` printed "All gates green." with both Rust gates silently skipped** when `cargo` was not on PATH. Under `Set-StrictMode`, the old one-liner left the variable unassigned rather than null, so the fallback threw, and the previous gate's exit code of 0 carried the run to a green finish. It now fails loudly, and each step resets the exit code it is judged by.
- **The PSScriptAnalyzer gate had never executed once.** It sat after `cargo test` in CI, which was red. Moved ahead of the Rust gates, widened from `tools/` to every tracked script, added to the release workflow — whose comment already claimed it ran "the SAME gates as CI" — and given a shared settings file so local and CI cannot drift apart. Its severity filter also excluded PSScriptAnalyzer *errors*; it now covers them.
- **The pre-push hook could not run `verify.ps1` at all** under Windows PowerShell 5.1: a BOM-less file is decoded as ANSI, a mis-decoded em-dash becomes a typographic quote, PowerShell honours it as a string delimiter, and `function Step` was swallowed whole. The 14 scripts carrying non-ASCII now have BOMs and the hook calls `pwsh`, matching `verify.ps1`'s own shebang.

### Fixed — defects that only appeared on machines that were not the developer's

- **`worktree_remove` refused to remove the worktrees it had created**, for anyone whose Windows profile path does not fit 8.3 (any username longer than eight characters). Paths were compared as raw strings, so a short-form path could never match git's canonical long form. The same helper backs the dangerous-path guards, which were therefore silently missing too.
- **A gate test asserted against the developer's own `stack.json`**, a file that only exists on that machine. Split into a pure mapping the gate can check anywhere, with the environment assertion kept as a manual smoke.

### Fixed — the app telling you things that were not true

- **The restore dialog could report "Restore complete" for a restore that never ran** — the most destructive operation in the app. Previewing armed the danger button whether or not a run started, and the completion watcher, once armed, was settled by any later run.
- **The Home cockpit called a fully-down stack "All good"**, while treating a partial outage as a warning.
- **The fork-sync outcome reported the previous run's numbers** when the current run produced a stale status envelope.
- **`CLAUDE.md` documented a procedure that destroys the shipped app icon**, naming the wrong file as the master and pointing at a legacy fallback generator.

### Fixed — responsiveness

- **Thirteen commands blocked the UI thread while waiting on a subprocess** — backup verify/extract/import, profile reads, repository clone, SSH connection test, the worktree commands, the schedule commands and the deploy pair. The window stopped repainting for the duration of each.
- **The schedule tick held its lock across a 60-second precheck and a network call**, blocking all four schedule commands. It now decides under the lock, runs the gates without it, then re-locks and re-validates against fresh state — so a schedule you edit during a precheck survives instead of being overwritten.

### Fixed — interface

- **Compact density was very nearly a no-op.** Tailwind's `@theme` resolved the spacing scale on `:root`, where a custom property collapses to a literal, so redeclaring it further down the tree changed nothing. Measured after the fix: card padding 16px → 10px.
- **Long tables were unusable.** Sticky headers had never worked (there was no vertical scrollport), and worse, roughly 2800px of a long table sat below the window with no way to scroll to it. Both fixed; the last row is now reachable and the header holds its place.
- Focus trap now includes the backdrop button; the titlebar double-click guard is actually wired up; detached windows hand their session back when closed with Alt+F4; terminal `file:line` links no longer drift right of their text on CJK and emoji; hover-pause and focus-pause no longer cancel each other in the toast stack.

### Changed

- Rust toolchain moved from the 1.93.0 pin to 1.94.0. The `time` regression that forced the pin is gone, verified by a clean check and a full release build.
- Clipboard read permission granted (write was, read was not, while the app pastes); two unused plugin capability sets removed.
- Terminal column mapping extracted to `src/lib/termColumns.ts` and the confirm gate to `src/lib/confirmGate.ts`, both under test.
- `parse_json_bom` shared across modules instead of being hand-copied four times.

### Removed

- `containUntrustedContext` — dead code whose comment promised prompt-injection protection it never provided, with broken truncation arithmetic on top.

## [0.7.0] — 2026-07-19

The multi-agent-and-gateway release. Castellyn grows from a Claude Code maintenance shell into a
control center for a whole local AI-coding stack: a single OmniRoute front gateway for every client,
first-class Codex/OpenCode agents, a native stack supervisor, a custom-subagent manager, a live
usage-limit monitor for both Anthropic and Codex, parallel git-worktree sessions with a cross-session
message bus, an agent-session scheduler, and a large audit-driven reliability + UI-consolidation pass.

### Added
- **OmniRoute unified gateway** — one front (`:20128`) routes every client: Claude Code (Anthropic `/v1/messages`), Codex (OpenAI `/v1/responses`), and OpenCode (OpenAI `/v1`). A **Connect OmniRoute** button on the Codex environment card wires `~/.codex` to the gateway (Codex 0.142+ format: `wire_api="responses"` + per-profile config file). Verified end-to-end live across all three client protocols.
- **Native stack supervisor** — Castellyn starts/stops the LLM-stack services itself (no console windows, reliable Stop), with dependency-ordered startup (topological sort over `dependsOn`), per-service readiness waits, and an opt-in `teardownOnFailure`. Falls back to the legacy PowerShell scripts when disabled.
- **Stack-health monitor** — a background poller checks LLM-stack service liveness (real HTTP health where configured, not just an open port) every 30 s and raises an error toast, tray count, and live card update when an unexpected outage happens; a service Castellyn stopped on purpose stays silent.
- **Subagents tab** — manage your own Claude Code subagents (`~/.claude/agents/*.md`) with create/edit/delete, model + tool-scope badges, and fan-out to every profile and machine; includes a Codex-delegate template.
- **Usage-limit monitor** — watches the Anthropic 5-hour OAuth window and detects "limit reached" from live PTY output; a limited Sessions pane can **auto-continue after its reset** or **switch to another profile**. A launch advisor recommends a profile + reasoning effort from live limits.
- **Orphan profile management** — the Profiles tab surfaces `~/.claude-<name>` directories that aren't canon profiles, each with **Adopt** and **Delete** (to the Recycle Bin), guarded against canon-profile data loss and against sweeping junction targets.
- **Cross-profile plugin sync** — on-demand reconcile plus a SessionStart hook that keeps every profile's plugin set aligned.
- **Sessions: agent status** — lifecycle hooks + PTY activity drive per-pane status with sound and OS notifications; the previous run's session set is restored after a restart; `Shift+PgUp/PgDn` scrollback and `Alt+N` pane focus.
- **Sessions: git-worktree isolation** — launch a session in its own throwaway git worktree so parallel agents on one repo never collide, with suffix-collision-safe naming, provenance metadata on the branch, and staggered auto-cleanup on close.
- **Sessions: scrollback persistence** — each pane's terminal scrollback is serialized and restored across restarts (capped, TTL-pruned), so a resumed session keeps its history.
- **Sessions: cross-session message bus + fan-out** — send a prompt to `@all` / `@idle` sessions (or a fan-out set) from one place; a file-backed mailbox delivers and tracks read state, with a per-tab unread badge.
- **Agent-session scheduler** — schedule a favorite session recipe to launch on a minute-tick timer, gated by a live quota check and an optional shell precheck; outcomes post back to the session bus.
- **Codex usage-limit monitor** — the usage watcher now also reads the Codex (ChatGPT) rate-limit window alongside the Anthropic one, so a Codex pane's limits surface the same way.
- **Updates** — auto re-check of the whole stack after Update-All, with live per-component progress.

### Changed
- **Providers — one "Services" section** — the former separate "LLM stack" (lifecycle) and "Engines" (wiring) lists are merged into a single card per service (joined by port), with lifecycle + wiring actions together and secondary actions in a per-card menu; the duplicated running badge / Dashboard / Check are gone.
- **Profiles — one table** — the health/status table and the separate provider/proxy/folders/plugins/MCP "Matrix" are merged: each profile row expands to its own config editor, with a single accumulate → preview → apply bar. No more two tables keyed on the same profiles.
- **Unified UI controls** — one segmented control (Settings theme/density/language, Analytics source + range, Environments Cards/Table, the config viewer) instead of four hand-rolled variants; the "My providers" cards drop a 7-button row for Connect + Check + a kebab; wide tables (MCP, Plugins, Forks) no longer clip their actions column.
- **Visual redesign (waves 1.5–2D)** — a typography + density foundation, an actionable Home cockpit, a rolled-up Updates header, Providers chips + diagnostics behind Details, and a split Sessions toolbar.
- **Build** — migrated the Rust crate to **edition 2024**; Vite 8 (Rolldown) + vite-plugin-svelte 7, TypeScript 6, Svelte 5.5; dependency bumps (windows 0.61, @sveltejs/kit, tsx, vitest).

### Fixed
- **Global audit hardening (2026-07-12)** — ~155 verified correctness / reliability / quality findings fixed across the backend, frontend, PowerShell, and CI, each validated for reachability. Notably: the launch advisor + auto-switch were dead in production (a profile-key prefix mismatch) and now work end-to-end.
- **Bug-hunt hardening (2026-07-15)** — 46 more verified fixes, each traced to source: SSH option-injection surface, a disposed-xterm write on pane teardown, DataTable rows that lingered after their data changed, an auto-update version-prefix mismatch that broke self-update, and a window-close path that killed live paid sessions instead of returning them to the main window.
- **Notifications** — a persistently re-firing background error (e.g. an unreadable plugins file) no longer fills the notification history with identical rows; consecutive duplicates collapse into a single ×N entry, matching the visible-toast dedup, and error toasts auto-expire instead of sticking forever.
- **Security & reliability hardening** — settings-write races, single-instance handling, status-engine correctness, and confirmed reliability bugs across multiple hardening waves; UTF-8-safe parsing of user files; BOM-tolerant JSON reads.
- **Providers** — a failed new-key write rolls back the migrated slot 0 instead of losing the key.
- **Shortcuts** — global shortcuts re-register teardown-first, so changing one while keeping another no longer silently fails.
- **Sync** — the Syncthing GUI address is read from `config.xml` (non-standard ports/binds work) instead of a hardcoded `127.0.0.1:8384`.
- **Environments** — MCP deploy reconciles: a server removed from the canonical `.mcp.json` is pulled from OpenCode and Codex too (with a "stale" drift badge).
- **Home** — profile counts and the "Repair" target match the sidebar badges; the stack start/stop action is no longer duplicated between the quick-action bar and the LLM-stack tile.
- **Hooks** — Castellyn's hook entries are unwired from orphaned/renamed profiles on disable.

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
