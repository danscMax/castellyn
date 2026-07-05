# Diagnosis — Analytics + Sessions statusline (Lead's own cluster)

## A. Analytics tab shows only the FreeLLMAPI gateway

**Symptom:** the Analytics tab ("Сводка ВСЕХ запросов через локальный шлюз freellmapi") shows only 2 requests, 12k tokens, all from Gemini/Gemma via the gateway. Owner wants app-wide analytics: session counts, time spent, tokens per profile, etc.

**Root cause (100%, verified):** `read_freellmapi_analytics(range_hours)` (`src-tauri/src/lib.rs:2878`) reads exactly one source — the gateway's SQLite `freeapi.db` via a node helper (`gateway_db_path` → `{dir}\data\freeapi.db`, `lib.rs:2871`; `Castellyn\tools\analytics\query.cjs`, `:2884`). There is no other analytics command. The tab is, by construction, "gateway-only". The header even says so. So it is not a bug — it's a scope limitation: the app never aggregates the *other* data it already has.

**Data sources the app ALREADY has but doesn't surface in Analytics:**
- Claude Code OAuth usage per profile: `fetch_usage`/`limits.rs` (5h/7d utilization, resets) — already polled and shown as chips on Profiles, never aggregated.
- Run history: `history.jsonl` (synced) + `runHistory.svelte.ts` (maintenance run durations/outcomes).
- Sessions: `agent_status.rs` tracks per-session working/blocked/idle + durations; PTY session lifecycle.
- Schedule run history: last-run success/time per scheduled task.
- Per-profile token spend: the gateway DB is per-model, not per-Claude-profile; Claude-subscription usage isn't token-metered by us (OAuth gives % utilization, not raw tokens).

**Fix (root cause, no crutch):** turn Analytics into a multi-source dashboard with a source switch, not a single-DB view. Tabs/sections: (1) Gateway (current freeapi.db view), (2) Claude usage (per-profile 5h/7d utilization history + reset timelines from limits), (3) Maintenance (run counts/durations/success from runHistory + schedule history), (4) Sessions (count, active time, blocked time, per-profile/per-tool from agent_status). Each source is independent; render whichever have data, empty-state the rest.

**Trade-offs:**
- Full multi-source aggregation is real work (new read commands for history/sessions aggregation, a source selector, ~4 new panels). High value, medium-high effort.
- Cheapest increment: rename/rescope the current tab honestly ("Аналитика шлюза") and add ONE new "Claude usage" panel from data already polled (limits) — low effort, partial win.
- A unified per-request token ledger across ALL providers (not just gateway) would need Claude Code's own usage export, which Anthropic OAuth doesn't provide as raw tokens — so "tokens per Claude profile" is only approximable via utilization %, not exact. Be honest about that ceiling.

### 20 metrics worth showing (owner asked to brainstorm)
Gateway: 1) requests, 2) success %, 3) in/out tokens, 4) avg latency, 5) $ saved vs paid, 6) top model by requests, 7) top model by cost, 8) cost-by-model bar, 9) requests-over-time trend, 10) per-model table. Claude: 11) per-profile 5h/7d utilization now, 12) utilization history sparkline, 13) time-to-reset per profile, 14) which profile is closest to limit. Maintenance: 15) runs per component (7/30d), 16) run duration histogram (already have "История запусков скриптов" — currently empty because runHistory is session-local; persist it), 17) success/fail ratio per component, 18) last-run recency per scheduled task. Sessions: 19) active sessions count + total active time today, 20) per-tool (claude/codex/opencode) session minutes + blocked-waiting time.

**Note:** the "История запусков скриптов" histogram on the Analytics tab is EMPTY ("Истории запусков пока нет") because run history (`runHistory.svelte.ts`) is an in-memory bounded store reset each app launch — it's not persisted. Root-cause fix: persist run history to a small JSON/sqlite so the histogram survives restarts (it's promised in the UI but never has data).

## B. Sessions statusline duplicated; move to the window title bar (owner feature request)

**Symptom:** each session pane renders the Claude statusline at the bottom (model, limits, RAM/CPU, sync, git). The app already has a custom window title bar (`WindowTitleBar.svelte`, `decorations:false`). Owner wants the statusline info surfaced in the top title bar (nicely), and the in-pane statusline hidden or shortened — configurable.

**Assessment (design, not a bug):** the in-pane statusline is rendered by Claude Code itself inside the PTY (it's the user's `statusline.py` output in the terminal), NOT by Castellyn — so Castellyn can't "move" it out of the terminal; it can only (a) surface a PARALLEL, native status strip in its own chrome (title bar or a header band for the focused pane) built from data Castellyn already has (agent_status: model/limits per session; the limits poll; git/branch), and (b) optionally the user can shorten their own statusline.py. The title bar is per-window; with multiple panes, "which session's status" must follow the FOCUSED pane.

**Fix direction:** add an optional native status strip for the focused session (in WindowTitleBar or just under it) fed by agent_status + limits + the session's profile/tool/cwd, with a settings toggle ("show session status in title bar" + "compact in-pane statusline hint"). Configurable per the owner's ask. The actual in-terminal statusline stays owned by Claude Code; Castellyn can't strip it, only duplicate-nicely and advise the user to trim statusline.py.

**Trade-offs:** native title-bar status is clean and always visible but only reflects data Castellyn tracks (not the exact statusline.py content); truly mirroring statusline.py would mean parsing terminal output (fragile). Recommend the native-data strip, not terminal scraping.
