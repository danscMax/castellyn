//! Agent status for Sessions panes (herdr-inspired).
//!
//! Semantic states: `working` | `blocked` | `idle` | `unknown`. "done" is a FRONTEND
//! notion (working/blocked → idle while the pane is unfocused), mirroring herdr's
//! Idle+!seen model. Authorities, strongest first:
//!  1. Claude Code lifecycle hooks — `castellyn_status.py` writes
//!     `%APPDATA%\castellyn\agent-status\<session_id>.json` on each lifecycle event.
//!     Codex writes the same file from its `notify` program, but only when a turn ENDS: a
//!     one-shot ping, not a lifecycle stream, so it expires on the next output (`turn_end_expired`).
//!     opencode writes it from a plugin (`castellyn_opencode_plugin.js`) and DOES stream a full
//!     lifecycle (busy / permission asked / idle), so its reports behave like Claude's.
//!  2. PTY output activity — a working heartbeat (full-screen agent TUIs repaint their
//!     spinner constantly, so silence is a reliable idle signal) that also self-heals a
//!     stale `blocked` after the user answers the prompt (no hook fires on approval).
//!  3. Process exit → idle.
//!
//! One poll thread (500 ms) recomputes every tracked session and emits an
//! `agent-status` event only on change. Sessions of tool `shell`/`ssh` are not tracked.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::{Emitter, Manager};

const POLL_MS: u64 = 500;
/// Right after spawn nothing meaningful has happened yet — report `unknown`.
const STARTUP_GRACE_MS: u64 = 3_000;
/// No PTY output for this long → not actively working. This governs the hookless activity branch,
/// which is the PRIMARY (only) turn signal for codex/opencode and remote claude. It was 4s, but real
/// turns go quiet well past 4s (deep thinking, a long tool/MCP/network call) — the same false-"done"
/// that WORKING_SELFHEAL_MS was raised to fix for hook-claude (live-smoke 2026-07-03). Claude's 4s was
/// safe only because a Stop hook corrects it; codex/opencode have no such correction, so 4s fired a
/// false completion on every slightly-slow turn. 15s survives normal pauses; truly-precise done/blocked
/// for these agents needs their own signals (Codex `notify` / opencode plugin) — a separate follow-up.
const ACTIVITY_IDLE_MS: u64 = 15_000;
/// A hook-reported `working` self-heals to idle only after this long of silence — the fallback for a
/// turn that ended WITHOUT a Stop hook (Esc-interrupt / crashed hook). Far longer than
/// ACTIVITY_IDLE_MS: real turns go quiet well past 4s (deep thinking, a long tool/MCP call, a network
/// wait), and treating those as "done" fired a false completion toast (live-smoke 2026-07-03). The
/// real end-of-turn signal is the Stop hook (Some("idle")); this is only the hookless backstop.
const WORKING_SELFHEAL_MS: u64 = 35_000;
/// Grace after a hook-reported `blocked` within which PTY output counts as the prompt box
/// painting itself, not a real resume — used by the time backstop below.
const BLOCKED_RESUME_MS: u64 = 1_500;
/// A resumed agent turn floods the PTY; a prompt-box repaint is small. Clear a hook-reported
/// `blocked` once this many bytes arrive since the block began (item 6, hook-less fallback).
const BLOCKED_RESUME_BYTES: u64 = 1_024;
/// Time backstop: after this long in `blocked` with real post-block output but no byte burst
/// (an Esc answer emits little), allow the flip so `blocked` can't stick forever.
/// Backlog 23 NARROWED it: it may no longer fire while an approval prompt is demonstrably the last
/// thing painted. Narrowing only — nothing shortens it. A per-chunk observation cannot prove a
/// prompt was DISMISSED (ordinary interleaved output — Claude Code's own background hooks print
/// into the terminal — clears the flag while the box is still on screen), so "the prompt stopped
/// appearing" is never treated as "the user answered".
const BLOCKED_STUCK_MS: u64 = 20_000;
/// Codex's `notify` fires once when a turn ends, and there is no matching "a turn began" event. Its
/// `idle` therefore cannot be authoritative forever, the way Claude's Stop hook can (a Claude turn
/// re-opens with UserPromptSubmit). Output arriving this long after the ping is a NEW turn, not the
/// agent painting its final answer and prompt box, so hook authority is dropped and the heartbeat
/// takes over again.
const TURN_END_ECHO_MS: u64 = 2_000;
/// After a detected usage limit, the session sits quiet until its window resets; a genuine resume
/// then floods far more than this, so that many bytes since the limit clears the `limited` state
/// (item 21b). Higher than the block threshold — a limit banner + its surrounding repaint is larger.
const LIMIT_RESUME_BYTES: u64 = 4_096;
/// A turn that ends this soon after the user's own keystroke needs no "finished" toast — they are
/// at the keyboard. Long enough to cover a quick answer, short enough that a turn they walked away
/// from still reports (item 33).
const TYPED_RECENTLY_MS: u64 = 20_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Track {
    tool: String,
    /// A lifecycle hook is expected for this session (local claude only): the authoritative
    /// working/idle signal. When true but no hook has ever reported, PTY activity is NOT used
    /// as a fallback — Claude Code and background hooks (claude-mem) print into the terminal, so
    /// activity ≠ a live turn — the session reports `unknown` instead of a false `working`
    /// (A-residual). Remote claude / codex / opencode are genuinely hookless → heartbeat is their
    /// only signal, so this is false and the activity branch applies.
    hook_expected: bool,
    /// Human label for notifications ("claude · cc1", "codex").
    label: String,
    /// The claude profile behind this pane. Read when the pane hits its usage limit, to say WHEN the
    /// window resets instead of just that it is exhausted — the only number the user actually needs
    /// at that moment.
    profile: String,
    spawned_at: u64,
    /// Unix ms of the last keystroke the USER sent into this pane. A turn that ends seconds after
    /// the user typed does not deserve a "finished" toast — they are sitting right there.
    last_user_input: AtomicU64,
    /// Unix ms of the last PTY output. Atomic so `on_output` can update it under a shared
    /// borrow (still under the TRACKS lock — see item-8 scope note).
    last_output: AtomicU64,
    /// Bytes emitted since the current `blocked` state began (reset in `apply_hook_report`).
    /// The hook-less fallback for clearing a stale `blocked` (item 6).
    bytes_since_block: AtomicU64,
    /// Backlog 23: the LAST scanned PTY chunk carried an approval/permission prompt in its
    /// unambiguous shape (`is_strong_permission_prompt`). Strictly a CORROBORATING signal, and only
    /// in ONE direction: it may hold a `blocked` (by vetoing the time ceiling) and it may turn a
    /// hookless pane's silence into `blocked`. It may never end a `blocked` or declare a turn
    /// finished — `false` here means "this chunk didn't show a prompt", which is NOT evidence that
    /// the box left the screen (any interleaved output lowers it).
    prompt_on_screen: AtomicBool,
    /// A usage limit was detected in this session's PTY output (item 21b). Shown as `limited`
    /// until a genuine resume (LIMIT_RESUME_BYTES of output past the limit) clears it.
    limited: AtomicBool,
    /// Bytes emitted since `limited` was set — the resume signal that clears it.
    bytes_since_limit: AtomicU64,
    /// A resumption-gating MENU is up: the limit menu ("What do you want to do? / 1. Stop and
    /// wait…") OR the large-session resume menu ("1. Resume from summary / 2. Resume full session…").
    /// The frontend picks option 1 and then injects "continue". Cleared once output floods past it
    /// (`bytes_since_menu`), independently of `limited` — the resume menu appears without a limit.
    /// (item 21f)
    limit_menu: AtomicBool,
    /// Bytes since `limit_menu` was set — clears the menu flag once Claude resumes (floods output),
    /// even when the pane was never `limited` (the resume-menu case). (item 21f)
    bytes_since_menu: AtomicU64,
    exited: bool,
    /// Latest hook-reported state ("working" | "blocked" | "idle"; "ended" clears it).
    hook_state: Option<String>,
    hook_ts: u64,
    /// Last-seen mtime (unix ms) of this session's hook file; skip the read+parse when it
    /// hasn't changed (item 8 mtime gate).
    hook_mtime: u64,
    claude_session_id: Option<String>,
    /// What the pane is waiting FOR, from the hook: `reason` is the kind of prompt
    /// ("permission" | "question" | "notification"), `ask` a short sanitised excerpt of it.
    /// Both live only for as long as the pane is blocked — cleared by the next non-blocked report,
    /// so yesterday's question can never ride along into the next turn.
    ask: Option<String>,
    ask_reason: Option<String>,
    last_emitted: Option<String>,
    /// Last-emitted `limit_menu`, so the menu appearing/clearing re-emits even when `state` stays
    /// "limited" (the change-gated emit keys on `state` alone otherwise). (item 21f)
    last_menu: bool,
}

/// The label a pane gets at spawn, before the frontend pushes the project-aware one.
/// Shared with `apply_hook_report`, which must recognise it to know the label is still a default.
fn spawn_label(tool: &str, profile: &str) -> String {
    if tool == "claude" && !profile.is_empty() {
        format!("{tool} · {profile}")
    } else {
        tool.to_string()
    }
}

static TRACKS: LazyLock<Mutex<HashMap<String, Track>>> = LazyLock::new(Default::default);

/// The session the user currently has open and focused, as reported by the frontend. The backend
/// otherwise only knows whether the WINDOW has focus, which silenced notifications about every
/// OTHER project while the user worked in one of them (item 35).
static FOCUSED_SESSION: LazyLock<Mutex<Option<String>>> = LazyLock::new(Default::default);

/// Frontend tells us which pane is on screen and focused; `None` when the Sessions tab is not
/// visible or nothing holds focus.
pub fn set_focused(id: Option<String>) {
    *FOCUSED_SESSION.lock().unwrap_or_else(|e| e.into_inner()) = id;
}

/// %APPDATA%\castellyn\agent-status (hook output directory).
pub fn status_dir() -> Option<std::path::PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|a| std::path::Path::new(&a).join("castellyn").join("agent-status"))
}

/// Register a freshly-spawned session. `shell`/`ssh` panes carry no agent — skipped.
/// `hook_expected` is true only for a LOCAL claude pane, whose lifecycle hook can reach the
/// local status dir; remote claude and codex/opencode are hookless (heartbeat only).
pub fn on_spawn(id: &str, tool: &str, profile: &str, hook_expected: bool) {
    if !matches!(tool, "claude" | "opencode" | "codex") {
        return;
    }
    let now = now_ms();
    TRACKS.lock().unwrap_or_else(|e| e.into_inner()).insert(
        id.to_string(),
        Track {
            tool: tool.to_string(),
            hook_expected,
            label: spawn_label(tool, profile),
            profile: profile.to_string(),
            spawned_at: now,
            last_user_input: AtomicU64::new(0),
            last_output: AtomicU64::new(now),
            bytes_since_block: AtomicU64::new(0),
            prompt_on_screen: AtomicBool::new(false),
            limited: AtomicBool::new(false),
            bytes_since_limit: AtomicU64::new(0),
            limit_menu: AtomicBool::new(false),
            bytes_since_menu: AtomicU64::new(0),
            exited: false,
            hook_state: None,
            hook_ts: 0,
            hook_mtime: 0,
            claude_session_id: None,
            ask: None,
            ask_reason: None,
            last_emitted: None,
            last_menu: false,
        },
    );
}

/// Replace the human label used in notifications. The backend only knows `tool · profile` at spawn
/// (that is all `session_spawn` hands it), but the toast should name the PROJECT and the session the
/// way the user sees them — and the pane name / project space live only on the frontend, where they
/// can also be renamed mid-session. So the frontend owns this string and pushes it here whenever it
/// changes, keeping the toast identical to the attention strip.
/// An empty label is ignored: it would blank the notification body, and the spawn-time
/// `tool · profile` is a better fallback than nothing.
/// A push always outranks the cwd-derived name `apply_hook_report` appends: that one only fills in
/// while the label is still the spawn-time default, for the panes the frontend never labels at all
/// (a detached window, a restored pane).
pub fn set_label(id: &str, label: &str) {
    if label.is_empty() {
        return;
    }
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(id)
    {
        t.label = label.to_string();
    }
}

/// PTY reader thread: `bytes` arrived for this session. Shared borrow (atomic fields) so it
/// needs no exclusive access, though it still takes the TRACKS lock to find the entry.
pub fn on_output(id: &str, bytes: usize) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        t.last_output.store(now_ms(), Ordering::Relaxed);
        t.bytes_since_block.fetch_add(bytes as u64, Ordering::Relaxed);
        // A genuine resume after a limit floods output; once enough has arrived, clear `limited`.
        if t.limited.load(Ordering::Relaxed)
            && t.bytes_since_limit.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64
                > LIMIT_RESUME_BYTES
        {
            t.limited.store(false, Ordering::Relaxed);
        }
        // The menu flag clears on its own output flood — independent of `limited`, since the resume
        // menu shows up without a rate limit (large-session case). (item 21f)
        if t.limit_menu.load(Ordering::Relaxed)
            && t.bytes_since_menu.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64
                > LIMIT_RESUME_BYTES
        {
            t.limit_menu.store(false, Ordering::Relaxed);
        }
    }
}

/// The user typed into this pane. Recorded so a turn that ends right after they pressed Enter does
/// not fire a "finished" toast at someone who is already looking at the screen (item 33).
pub fn on_user_input(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        t.last_user_input.store(now_ms(), Ordering::Relaxed);
    }
}

/// Mark a session as usage-limited (item 21b): the PTY reader detected a "limit reached" banner.
/// Only claude panes carry an agent; unknown ids are ignored. Resets the resume counter so the
/// state holds until real output resumes past LIMIT_RESUME_BYTES.
pub fn on_limit(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        if t.tool == "claude" {
            t.bytes_since_limit.store(0, Ordering::Relaxed);
            t.limited.store(true, Ordering::Relaxed);
        }
    }
}

/// The interactive limit MENU was detected ("What do you want to do? / 1. Stop and wait…"). Like
/// `on_limit` (flags limited) but also records that a menu is up, so the frontend dismisses it —
/// picks "Stop and wait" — before injecting "continue" after the window resets. (item 21f)
pub fn on_limit_menu(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        if t.tool == "claude" {
            t.bytes_since_limit.store(0, Ordering::Relaxed);
            t.limited.store(true, Ordering::Relaxed);
            t.bytes_since_menu.store(0, Ordering::Relaxed);
            t.limit_menu.store(true, Ordering::Relaxed);
        }
    }
}

/// The large-session RESUME menu was detected ("1. Resume from summary / 2. Resume full session…").
/// Sets `limit_menu` so the frontend picks option 1 and continues — but NOT `limited`: there is no
/// rate limit here, so the continue fires promptly instead of waiting for a reset. (item 21f)
pub fn on_resume_menu(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        if t.tool == "claude" {
            t.bytes_since_menu.store(0, Ordering::Relaxed);
            t.limit_menu.store(true, Ordering::Relaxed);
        }
    }
}

/// True when a line signals a Claude Code usage limit. Anchored on the qualified banner wording
/// ("usage limit reached", "N-hour limit reached") rather than a bare "limit reached", so agent
/// output merely discussing limits doesn't flip the badge; still tolerant of version drift, with the
/// endpoint monitor (limits.rs) as the confirming/secondary signal. Pure + unit-tested.
fn is_limit_line(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("usage limit reached")
        || l.contains("hour limit reached")
        || l.contains("out of extra usage")
        // Newer Claude Code banner family (documented in errors.md): "You've hit your {session |
        // weekly | Opus} limit · resets …". Anchor on "hit your" + "limit" (apostrophe-agnostic,
        // covers all three windows) so prose like "the session limit is 5h" doesn't flip the badge.
        // ponytail: "hit your … limit" could match rare prose ("you'll hit your rate limit"); the
        // state self-clears on the next output flood + the endpoint monitor cross-checks, so accept it.
        || (l.contains("hit your") && l.contains("limit"))
}

/// True when the interactive rate-limit MENU is showing ("What do you want to do? / 1. Stop and
/// wait for limit to reset / …"). Distinct from `is_limit_line`'s passive banner: the menu blocks on
/// a keypress, so auto-continue must dismiss it first. Anchored on the option label (stable text
/// emitted by the Ink widget), not the volatile "What do you want to do?" header. Pure + unit-tested.
fn is_limit_menu(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    // Require the "Stop and wait" option AND a second menu marker (the header or the Upgrade option)
    // so prose merely discussing a limit can never trigger an auto-keypress — only the real menu does.
    l.contains("stop and wait for limit")
        && (l.contains("what do you want to do") || l.contains("upgrade your plan"))
}

/// True when the large-session RESUME menu is showing ("1. Resume from summary / 2. Resume full
/// session as-is / 3. Don't ask me again"). Anchored on the distinctive "resume full session" option
/// label so ordinary prose about resuming doesn't trip it. This menu is NOT a rate limit — the
/// frontend picks option 1 and continues immediately (no reset wait). (item 21f)
fn is_resume_menu(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    // Require BOTH option labels — near-zero chance of both appearing together outside the real menu,
    // so ordinary talk of "resuming" can't trigger an auto-keypress.
    l.contains("resume from summary") && l.contains("resume full session")
}

/// Literal markers of a permission/approval prompt's affirmative options. Anchored on the OPTION
/// LABELS rather than the volatile question line ("What do you want to do?" — the LIMIT menu's own
/// header — contains "do you want to").
///
/// ORDER MATTERS: index 0 is the generic one — a bare "1. yes" also occurs in ordinary agent prose
/// (an enumerated recommendation), so `is_strong_permission_prompt` treats only `[1..]` as
/// self-sufficient. Keep any newly added generic wording at the front.
const PERMISSION_MARKERS: [&str; 5] = [
    "1. yes",
    "yes, allow",
    "yes, and",
    "no, and tell claude",
    "do you trust the files",
];

/// Offset of the LAST permission/approval marker in an already-lowercased haystack. A POSITION, not
/// a bool, because the veto is POSITIONAL: a prompt only outranks a menu when it sits BELOW it —
/// i.e. when it is the thing currently on screen. Merely being present somewhere in the window means
/// the user very likely already answered it and the menu underneath is the live thing; suppressing
/// on presence alone made a real limit/resume menu unrecognisable for as long as the confound stayed
/// in the window, which for a repainting TUI can be forever.
fn permission_menu_at(l: &str) -> Option<usize> {
    PERMISSION_MARKERS.iter().filter_map(|m| l.rfind(m)).max()
}

/// True when the text carries a permission/approval prompt at all. Used where only presence matters
/// (retracting a standing menu flag), never for the positional veto itself.
fn is_permission_menu(s: &str) -> bool {
    permission_menu_at(&s.to_ascii_lowercase()).is_some()
}

/// True only for the unmistakable SHAPE of an approval prompt: a question line together with an
/// enumerated affirmative option, or one of the prompt-specific option labels that ordinary prose
/// does not produce.
///
/// `is_permission_menu` is deliberately loose — it exists to RETRACT a menu claim, where a false
/// positive costs nothing. `prompt_on_screen` is the opposite: it makes a positive claim ("someone
/// is waiting for you") off nothing but this detector, so the generic `"1. yes"` alone is not
/// enough. A finished turn ending in "Recommendation: 1. Yes, ship it / 2. Wait" would otherwise
/// nag as `blocked` forever.
///
/// ponytail: still just substrings on a raw chunk — "1. Yes, and then deploy" in prose can pass.
/// Upgrade path is the one `menu_signal_in_text` already uses: hand it the pane's rendered rows.
fn is_strong_permission_prompt(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    PERMISSION_MARKERS[1..].iter().any(|m| l.contains(m))
        || (l.contains(PERMISSION_MARKERS[0]) && l.contains("do you want to"))
}

/// Offset of the LAST occurrence of any needle. The needles are the detectors' own anchors, so the
/// position returned is the bottom-most line of whatever the detector matched on.
fn last_of(l: &str, needles: &[&str]) -> Option<usize> {
    needles.iter().filter_map(|n| l.rfind(n)).max()
}

fn limit_menu_at(l: &str) -> Option<usize> {
    if !is_limit_menu(l) {
        return None;
    }
    last_of(l, &["stop and wait for limit"])
}

fn limit_line_at(l: &str) -> Option<usize> {
    if !is_limit_line(l) {
        return None;
    }
    // Mirrors `is_limit_line`'s alternatives; "hit your" stands in for its two-term branch.
    last_of(
        l,
        &[
            "usage limit reached",
            "hour limit reached",
            "out of extra usage",
            "hit your",
        ],
    )
}

fn resume_menu_at(l: &str) -> Option<usize> {
    if !is_resume_menu(l) {
        return None;
    }
    last_of(l, &["resume from summary", "resume full session"])
}

/// Which usage-limit signal (if any) a fresh PTY chunk's bounded tail carries. The MENU is the more
/// specific signal so it's checked first (and a menu implies a limit); the passive banner next; the
/// resume menu last. Split out from `scan_limit` as a pure fn so the TAIL-window boundary is
/// unit-testable. ponytail: bounded-tail scan, not a full-buffer regex — a banner shoved > TAIL bytes
/// back by a prompt repaint is intentionally missed here; the endpoint monitor (limits.rs) backstops.
#[derive(Debug, PartialEq, Eq)]
enum LimitSignal {
    LimitMenu,
    Limit,
    ResumeMenu,
}

/// The bounded window a PTY scan looks at — banners and prompt boxes are short, and a firehose must
/// not cost a full-buffer regex. 512 bytes for the raw stream; the click-time check (which is handed
/// a pane's rendered rows, box art and all, and runs once per click) gets a roomier window.
const PTY_TAIL: usize = 512;
const PANE_TAIL: usize = 4096;

fn tail_of(chunk: &[u8], n: usize) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(&chunk[chunk.len().saturating_sub(n)..])
}

/// The one detector. Everything that decides "is an answerable menu on screen" — the PTY scanner and
/// the click-time re-check alike — goes through here, so the wording exists in exactly one place.
fn limit_signal_in(text: &str) -> Option<LimitSignal> {
    let l = text.to_ascii_lowercase();
    let perm = permission_menu_at(&l);
    // POSITIONAL veto. A menu survives only when no approval prompt sits below it. Fail closed on a
    // tie (`p < m`, not `<=`) and whenever the menu has no locatable anchor.
    //
    // ponytail: byte order is our only screen-order proxy in a raw PTY chunk — ANSI cursor moves can
    // repaint out of order, so "below in the buffer" is a heuristic here, not a fact. That is
    // survivable because it is not the last word: `menu_signal_in_text` re-runs this same detector
    // over the pane's RENDERED rows immediately before any keystroke, and there the order IS the
    // screen order. Upgrade path, if ever needed: feed the scanner a parsed screen instead of bytes.
    let unvetoed = |at: Option<usize>| match (at, perm) {
        (Some(m), Some(p)) => p < m,
        (Some(_), None) => true,
        (None, _) => false,
    };
    // Each candidate is tested on its own merits: a vetoed menu FALLS THROUGH to the next signal
    // instead of blanking the whole chunk (a stray "1. Yes" fragment in the window used to hide a
    // freshly printed limit banner entirely, and it stayed hidden for as long as it kept repainting).
    if unvetoed(limit_menu_at(&l)) {
        Some(LimitSignal::LimitMenu)
    } else if unvetoed(limit_line_at(&l)) {
        Some(LimitSignal::Limit)
    } else if unvetoed(resume_menu_at(&l)) {
        Some(LimitSignal::ResumeMenu)
    } else {
        None
    }
}

/// Raw-chunk shorthand for the tests: they assert on the PTY path's exact window, which `scan_limit`
/// applies inline (it needs the tail twice).
#[cfg(test)]
fn limit_signal_in_tail(chunk: &[u8]) -> Option<LimitSignal> {
    limit_signal_in(&tail_of(chunk, PTY_TAIL))
}

/// Backlog 27 / TOCTOU: what is on the pane's screen RIGHT NOW, per the very same detector the PTY
/// scanner uses. The frontend hands over the live terminal rows immediately before writing a
/// keystroke, because the cached `limitMenu` flag is a scan-time verdict that can outlive its menu
/// (the veto is an allowlist, and the byte-flood clear is 4 KiB coarse) — and a confirm dialog only
/// widens that window. Returns the camelCase discriminant, or null for "nothing answerable here".
#[tauri::command]
pub fn menu_signal_in_text(text: String) -> Option<&'static str> {
    match limit_signal_in(&tail_of(text.as_bytes(), PANE_TAIL)) {
        Some(LimitSignal::LimitMenu) => Some("limitMenu"),
        Some(LimitSignal::Limit) => Some("limit"),
        Some(LimitSignal::ResumeMenu) => Some("resumeMenu"),
        None => None,
    }
}

/// Scan a fresh PTY chunk's tail for a usage-limit banner/menu and flag the session if found. The
/// reader passes the raw bytes; we inspect a bounded tail (banners are short lines) to keep it cheap
/// under a firehose.
pub fn scan_limit(id: &str, chunk: &[u8]) {
    let tail = tail_of(chunk, PTY_TAIL);
    let signal = limit_signal_in(&tail);
    // Backlog 23: is an approval prompt the thing on screen? Only the no-menu case has to ask — a
    // limit/resume menu that won `limit_signal_in`'s positional veto has already established that no
    // prompt sits below it, and this is the same lowercase+rfind pass the veto did, not a new one.
    let prompt = signal.is_none() && is_permission_menu(&tail);
    // The status flag needs the STRONGER shape (see `is_strong_permission_prompt`): it claims a pane
    // is waiting, where `prompt` below only retracts a claim. The strict pass runs on the rare hit.
    note_prompt(id, prompt && is_strong_permission_prompt(&tail));
    match signal {
        Some(LimitSignal::LimitMenu) => on_limit_menu(id),
        Some(LimitSignal::Limit) => on_limit(id),
        Some(LimitSignal::ResumeMenu) => on_resume_menu(id),
        // An approval prompt now owns the screen: RETRACT a menu flag still standing, don't merely
        // decline to set one. `on_output` only clears it after LIMIT_RESUME_BYTES of output, and a
        // prompt box is an order of magnitude smaller than that — so a menu that a permission prompt
        // replaced kept the flag up, aiming both auto-continue's "1" and the header answer buttons
        // (backlog 27) at the prompt instead. This is the one moment we can tell, so we act on it.
        // Presence (not position) is right here: nothing answerable was found anyway, so a prompt
        // anywhere in the window is reason enough to stop claiming a menu is up.
        None => {
            if prompt {
                clear_menu(id);
            }
        }
    }
}

/// Backlog 23: record whether the chunk just scanned shows an approval prompt. Mirrors the LATEST
/// chunk, so a resume's flood of ordinary output lowers it by itself, with no threshold to tune.
///
/// ponytail: a raw PTY chunk is not a rendered screen, so `true` is a proxy and `false` is barely
/// even that — ordinary interleaved output (background hooks print into the terminal, see
/// `hook_expected`) lowers the flag with the box still up. That asymmetry is why every reader
/// treats `true` as evidence and `false` as nothing at all.
fn note_prompt(id: &str, on_screen: bool) {
    if let Some(t) = TRACKS.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        t.prompt_on_screen.store(on_screen, Ordering::Relaxed);
    }
}

/// Drop the "a menu is up" flag for this session. Leaves `limited` alone: the quota is still spent
/// whatever is drawn over it — only the *answerable menu* claim is being retracted.
fn clear_menu(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
    {
        t.limit_menu.store(false, Ordering::Relaxed);
    }
}

/// PTY reader thread hit EOF (child exited). The poll loop emits the final `idle` and
/// drops the track.
pub fn on_exit(id: &str) {
    if let Some(t) = TRACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(id)
    {
        t.exited = true;
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    id: String,
    state: String,
    claude_session_id: Option<String>,
    /// Session spawn time (unix ms), static per session — the frontend derives "active for N"
    /// from `now - spawnedAt` on render (no ticking backend events).
    spawned_at: u64,
    #[serde(skip)]
    prev: Option<String>,
    #[serde(skip)]
    label: String,
    #[serde(skip)]
    exited: bool,
    /// The hook reported `idle` (a real Stop) at this emit — gates the completion toast so a
    /// hookless activity-lull can't fire a false "finished" (live-smoke 2026-07-03). Serialized
    /// (`hookIdle`) so the frontend gates its visual "done" the same way as the toast.
    hook_idle: bool,
    /// The interactive limit menu is up (`limitMenu`): the frontend picks "Stop and wait" to dismiss
    /// it before injecting "continue" after reset. Rides alongside `state` ("limited"). (item 21f)
    limit_menu: bool,
    /// What the blocked pane is asking, straight from the hook (`ask`). Authoritative, so the
    /// frontend prefers it over its own buffer heuristic; absent for hookless panes, which keep
    /// using `askPreview`. `None` on every non-blocked state.
    ask: Option<String>,
    /// Backend-only: which kind of prompt is up — picks the notification wording. Not serialized:
    /// the UI shows the question itself, and a bare "permission"/"question" label adds nothing.
    #[serde(skip)]
    ask_reason: Option<String>,
    /// Backend-only: whether the user typed into this pane moments ago (item 33).
    #[serde(skip)]
    typed_recently: bool,
    #[serde(skip)]
    profile: String,
}

/// System sound for a transition (no bundled audio: MessageBeep respects the user's
/// sound scheme and mute state). No-op on non-Windows.
fn beep(kind: crate::notify::Kind) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::{
            MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONHAND,
        };
        // Three states, three sounds — one "attention vs not" split could not tell "answer me" from
        // "I am parked on quota", and those call for very different reactions. System sounds, so the
        // user's own scheme and mute still govern.
        let _ = MessageBeep(match kind {
            crate::notify::Kind::Blocked | crate::notify::Kind::BlockedMany => MB_ICONEXCLAMATION,
            crate::notify::Kind::Limited => MB_ICONHAND,
            _ => MB_ICONASTERISK,
        });
    }
    #[cfg(not(windows))]
    let _ = kind;
}

/// Popup + sound policy (herdr-style): →blocked = attention; working/blocked→idle =
/// background completion. Suppressed while any Castellyn window is focused — the user
/// is already looking at the app.
/// When this profile's 5-hour window resets, if that is known. Reuses the shared usage cache, so a
/// warm entry costs nothing and a cold one is a single 5-second-timeout request.
fn limit_reset_for(profile: &str) -> Option<String> {
    if profile.is_empty() {
        return None;
    }
    let home = std::env::var("USERPROFILE").ok()?;
    let creds = format!(r"{home}\.claude-{profile}\.credentials.json");
    let resp = crate::limits::usage_cached(&creds)?.ok()?;
    crate::limits::util_of(&resp, "five_hour").1
}

/// Which "input needed" wording to use. Both halves must be known: the richer keys interpolate
/// `{ask}`, so a reason without a question would render a dangling dash. Any unknown reason (an
/// older or newer hook) falls back to the plain body rather than dropping the notification.
fn blocked_body_key(ask: Option<&str>, reason: Option<&str>) -> &'static str {
    match (ask, reason) {
        (Some(_), Some("permission")) => "status.blocked_perm",
        (Some(_), Some("question")) => "status.blocked_ask",
        _ => "status.blocked_body",
    }
}

fn notify_transition(app: &tauri::AppHandle, ev: &StatusEvent) {
    // A closed/exited pane also lands on idle — that's teardown, not a completion worth
    // a "finished" toast (closing a working pane must stay silent).
    if ev.exited {
        return;
    }
    let to_blocked = ev.state == "blocked" && ev.prev.as_deref() != Some("blocked");
    // A pane hitting its usage limit is attention-worthy the same way as blocked: it's stalled on
    // quota until the window resets. Same focus-gate + attention beep, its own toast text.
    let to_limited = ev.state == "limited" && ev.prev.as_deref() != Some("limited");
    // "Finished" toast only on a REAL end-of-turn signal (hook_idle): Claude's Stop hook, or codex's
    // `notify` program. An activity-lull idle just greys the dot; it must NOT claim "done" — clicking
    // into the pane (cursor repaint) or any terminal noise flips working→idle, and that once fired a
    // false "Агент закончил" though nothing ran (owner live-smoke). opencode's plugin reports the
    // same way, so all three local agents can now be honest about a finished turn.
    let completed = ev.state == "idle"
        && matches!(ev.prev.as_deref(), Some("working") | Some("blocked"))
        && ev.hook_idle;
    if !to_blocked && !completed && !to_limited {
        return;
    }
    // A finished turn seconds after the user's own keystroke is not news to them (item 33).
    if completed && ev.typed_recently {
        return;
    }
    // The window having focus is not enough to stay silent: with several projects open the user
    // looks at ONE pane, and the others were being silenced too (item 35). Suppress only when the
    // very pane this is about is the one on screen.
    let window_focused = app
        .webview_windows()
        .values()
        .any(|w| w.is_focused().unwrap_or(false));
    let looking_at_it = FOCUSED_SESSION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_deref()
        == Some(ev.id.as_str());
    if window_focused && looking_at_it {
        return;
    }
    let cfg = crate::read_config_file();
    let lang = crate::cur_lang();
    if cfg.status_sounds.unwrap_or(true) {
        beep(if to_blocked {
            crate::notify::Kind::Blocked
        } else if to_limited {
            crate::notify::Kind::Limited
        } else {
            crate::notify::Kind::Done
        });
    }
    // Everything OS-facing goes through the one channel (crate::notify): it owns the app identity,
    // replaces a previous notice about the SAME session instead of stacking, and gives the waiting
    // toast a click that jumps to that pane. The `status_notify` switch is checked in there.
    // Several agents waiting at once is ONE piece of news, not N. Beyond the first, collapse into a
    // single tagged notice that replaces itself as the count moves — otherwise a fleet of panes
    // stopping together buries the screen in toasts saying the same thing.
    let waiting = attention_counts().0;
    if to_blocked && waiting > 1 {
        crate::notify::notify(
            app,
            crate::notify::Notice {
                kind: crate::notify::Kind::BlockedMany,
                title: crate::i18n::tr("status.blocked_title", lang).to_string(),
                body: crate::i18n::trv("status.blocked_many", lang, &[("n", &waiting)]),
                tag: Some("blocked-many".to_string()),
                // Jump to the pane that just stopped — the most recent one is as good a landing
                // place as any, and the rail lists the rest.
                session: Some(ev.id.clone()),
            },
        );
        return;
    }
    // "Your input is needed" made the user open the pane to find out WHAT for. When the hook told
    // us, say it in the toast — same shape as notify.limited_body/_body_until below: a richer key
    // when the extra fact is known, the plain one otherwise.
    let asked = ev.ask.as_deref().filter(|s| !s.is_empty());
    let (kind, tk, bk) = if to_blocked {
        (
            crate::notify::Kind::Blocked,
            "status.blocked_title",
            blocked_body_key(asked, ev.ask_reason.as_deref()),
        )
    } else if to_limited {
        (crate::notify::Kind::Limited, "notify.limited_title", "notify.limited_body")
    } else {
        (crate::notify::Kind::Done, "status.done_title", "status.done_body")
    };
    // "Parked until the window resets" is only actionable with the WHEN. The number is already
    // cached per profile (limits::usage_cached, 5-minute TTL shared with the badge), so this costs
    // no extra request in the common case.
    let body = if to_limited {
        match limit_reset_for(&ev.profile) {
            Some(at) => crate::i18n::trv(
                "notify.limited_body_until",
                lang,
                &[("label", &ev.label), ("at", &at)],
            ),
            None => crate::i18n::trv(bk, lang, &[("label", &ev.label)]),
        }
    } else if to_blocked && bk != "status.blocked_body" {
        crate::i18n::trv(bk, lang, &[("label", &ev.label), ("ask", &asked.unwrap_or(""))])
    } else {
        crate::i18n::trv(bk, lang, &[("label", &ev.label)])
    };
    crate::notify::notify(
        app,
        crate::notify::Notice {
            kind,
            title: crate::i18n::tr(tk, lang).to_string(),
            body,
            // Same subject = same session, so a later notice about this pane replaces the
            // earlier one instead of stacking.
            tag: Some(ev.id.clone()),
            session: Some(ev.id.clone()),
        },
    );
}

/// Has a one-shot end-of-turn ping been overtaken by fresh output, i.e. did a new turn start?
///
/// Only codex reports this way. Claude streams a full lifecycle, so its `idle` stays authoritative
/// until the next `working` arrives — letting output clear it would let a prompt-box repaint or the
/// user's own typing fake a turn.
fn turn_end_expired(tool: &str, hook_state: Option<&str>, hook_ts: u64, last_output: u64) -> bool {
    tool == "codex" && hook_state == Some("idle") && last_output > hook_ts + TURN_END_ECHO_MS
}

fn compute(t: &Track, now: u64) -> &'static str {
    if t.exited {
        return "idle";
    }
    // A detected usage limit outranks the hook/activity states: the session is stalled on quota
    // until its window resets (cleared in on_output once real output resumes). (item 21b)
    if t.limited.load(Ordering::Relaxed) {
        return "limited";
    }
    let last_output = t.last_output.load(Ordering::Relaxed);
    let silent = now.saturating_sub(last_output) > ACTIVITY_IDLE_MS;
    match t.hook_state.as_deref() {
        // Blocked holds until the agent clearly resumed: either a byte burst since the block
        // (approval floods the PTY) or, as a backstop, real post-block output that has sat
        // past the stuck ceiling so a small (Esc-answer) response still recovers. A bare
        // prompt-box repaint (small, no burst) must NOT clear it — the old bug (item 6).
        Some("blocked") => {
            let burst = t.bytes_since_block.load(Ordering::Relaxed) > BLOCKED_RESUME_BYTES;
            let real_output = last_output > t.hook_ts + BLOCKED_RESUME_MS;
            let stuck = now.saturating_sub(t.hook_ts) > BLOCKED_STUCK_MS;
            // Backlog 23: an approval prompt was the last thing the PTY painted. ONE-WAY: it can
            // only veto the weak (time-ceiling) path, never shorten it. The reverse — "no prompt in
            // the latest chunk, so the user must have answered" — is NOT available: any interleaved
            // output lowers the flag while the box is still literally on screen, and reporting
            // `working` on that would be exactly the false "resumed" this arm exists to prevent.
            let on_screen = t.prompt_on_screen.load(Ordering::Relaxed);
            // The byte burst keeps its authority untouched — item 6 tuned it, and letting the screen
            // proxy veto it could pin a pane at `blocked` forever.
            if burst || (stuck && real_output && !on_screen) {
                // Same silence backstop as the `working` arm below: `bytes_since_block` only ever
                // resets on a NEW blocked report, so once the burst threshold is passed this arm
                // returns "working" forever — a turn that ended without a Stop hook would never
                // fall back to idle.
                if now.saturating_sub(last_output) > WORKING_SELFHEAL_MS {
                    "idle"
                } else {
                    "working"
                }
            } else {
                "blocked"
            }
        }
        // A silent "working" self-heals to idle ONLY after a long backstop: the real end-of-turn
        // signal is the Stop hook (→ Some("idle") below). This branch just recovers a turn that
        // ended without one (Esc-interrupt / crashed hook). A short output gap (<WORKING_SELFHEAL_MS)
        // is still an active turn — flipping it to idle at 4s fired a false "done" (live-smoke).
        Some("working") => {
            if now.saturating_sub(last_output) > WORKING_SELFHEAL_MS {
                "idle"
            } else {
                "working"
            }
        }
        // Hook-reported idle is authoritative: prompt-box echo/typing must not flip it —
        // the next UserPromptSubmit hook reports working.
        Some("idle") => "idle",
        // No hook authority yet. PTY activity is the fallback, but ONLY for genuinely hookless
        // sessions (codex/opencode, remote claude). A turn stays `working` through normal
        // think/tool pauses (ACTIVITY_IDLE_MS = 15s) so a hookless agent doesn't false-flip to
        // done mid-turn.
        _ => {
            if now.saturating_sub(t.spawned_at) < STARTUP_GRACE_MS {
                "unknown"
            } else if t.hook_expected {
                // Local claude that expected a hook but never got one → the Agent-statuses hook is
                // off/unwired. Claude Code and its background hooks (claude-mem) print into the PTY
                // even when the user isn't in a turn, so activity is an unreliable proxy that
                // false-flags `working` (A-residual). Report `unknown` (neutral dot, uncounted, no
                // false "done") instead of lying; enabling the hook restores authoritative status.
                "unknown"
            } else if silent {
                // Backlog 23: a hookless pane parked at an approval prompt goes quiet in exactly the
                // same way as one that finished, so 15 s of silence used to read as `idle` and the
                // user was never told anyone was waiting — remote claude over SSH is the live case,
                // and it is the one agent that gets no lifecycle hook at all. The prompt still being
                // the last thing painted turns that silence into what it actually is. This can only
                // refine `idle` into `blocked`; it never touches `working` and never declares a turn
                // over, so it cannot produce a false "finished". It CAN produce a false "waiting",
                // which is why the flag needs `is_strong_permission_prompt` and not the loose
                // detector: a finished turn whose answer ends in an enumerated list must stay idle.
                if t.prompt_on_screen.load(Ordering::Relaxed) {
                    "blocked"
                } else {
                    "idle"
                }
            } else {
                "working"
            }
        }
    }
}

/// Read this session's hook file into the track (cheap: ~1 tiny JSON per tracked pane
/// per poll; only local claude panes ever have one).
fn apply_hook_report(v: &serde_json::Value, t: &mut Track) {
    let ts = v.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);
    if ts <= t.hook_ts {
        return; // stale / unchanged
    }
    t.hook_ts = ts;
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
    // SessionEnd → the agent is gone (pane is a plain shell again): drop hook authority.
    t.hook_state = match state {
        "ended" | "" => None,
        s => Some(s.to_string()),
    };
    // A fresh block starts the byte-burst counter from zero (item 6 fallback). The initial
    // prompt-box paint usually lands before this poll reads the hook file, so it isn't
    // counted; only output after the block accrues.
    // ponytail: a large plan-approval box that repaints AFTER this reset (e.g. terminal
    // resize) could exceed BLOCKED_RESUME_BYTES and false-clear; upgrade path is a short
    // settle delay before counting. Rare enough to leave.
    if t.hook_state.as_deref() == Some("blocked") {
        t.bytes_since_block.store(0, Ordering::Relaxed);
    }
    // "Waiting" without "for what" is what this pair fixes. Both are scoped to the blocked state:
    // reading them unconditionally would leave the previous turn's question attached to the pane.
    let str_field = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let blocked = t.hook_state.as_deref() == Some("blocked");
    t.ask = blocked.then(|| str_field("ask")).flatten();
    t.ask_reason = blocked.then(|| str_field("reason")).flatten();
    if let Some(cs) = v
        .get("claudeSessionId")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        t.claude_session_id = Some(cs.to_string());
    }
    // The hook knows the pane's project directory; the frontend's richer label may never arrive
    // (a detached window, a restored pane). Name the project as soon as the first hook lands, but
    // only while the label is still the spawn-time default — a frontend push always outranks this.
    if let Some(base) = str_field("cwd")
        .as_deref()
        .map(std::path::Path::new)
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    {
        if t.label == spawn_label(&t.tool, &t.profile) {
            t.label = format!("{} · {base}", t.label);
        }
    }
}

/// Start the poll thread. Called once from `setup()`.
/// (blocked, limited) counts across live panes, from each track's last-emitted state. Cheap
/// snapshot for the tray tooltip — no recompute, just reads what the poll already published.
/// Reflect "someone is waiting" OUTSIDE the window: flash the taskbar button and put the count in
/// the native title, which is what Alt+Tab and the taskbar preview show. The in-window strip
/// already covers the case where the user is looking at Castellyn; this is for when they are not.
///
/// Called on the same transitions as the tray tooltip, so the two never disagree.
/// A bounded ring of recent state transitions, newest last.
///
/// Every false "done" and stuck "blocked" this engine ever produced was diagnosed by reconstructing
/// what the state machine saw — from memory, after the fact, from a user's description. This keeps
/// the last few hundred transitions so the next one can be read instead of reconstructed. In memory
/// only: it is a debugging aid, not a record worth writing to disk.
static TRANSITIONS: LazyLock<Mutex<std::collections::VecDeque<String>>> =
    LazyLock::new(Default::default);
const TRANSITION_LOG_CAP: usize = 300;

fn log_transition(id: &str, label: &str, prev: Option<&str>, state: &str, now: u64) {
    let mut log = TRANSITIONS.lock().unwrap_or_else(|e| e.into_inner());
    log.push_back(format!(
        "{now} {id} [{label}] {} -> {state}",
        prev.unwrap_or("-")
    ));
    while log.len() > TRANSITION_LOG_CAP {
        log.pop_front();
    }
}

/// The recent transitions, oldest first — surfaced so a misbehaving status can be inspected while
/// it is still misbehaving.
#[tauri::command]
pub fn agent_status_log() -> Vec<String> {
    TRANSITIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}

/// A small round badge, drawn in code rather than shipped as an asset — it is one flat colour and
/// generating it avoids adding (and keeping in sync) a set of PNGs for every state.
#[cfg(windows)]
fn badge_image(rgb: (u8, u8, u8)) -> tauri::image::Image<'static> {
    const S: u32 = 32;
    let r = S as f32 / 2.0;
    let mut px = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            // Distance from the centre, with a one-pixel soft edge so the dot is not jagged.
            let dx = x as f32 + 0.5 - r;
            let dy = y as f32 + 0.5 - r;
            let d = (dx * dx + dy * dy).sqrt();
            let a = ((r - d).clamp(0.0, 1.0) * 255.0) as u8;
            px.extend_from_slice(&[rgb.0, rgb.1, rgb.2, a]);
        }
    }
    tauri::image::Image::new_owned(px, S, S)
}

pub(crate) fn update_attention_surfaces(app: &tauri::AppHandle) {
    let (blocked, limited) = attention_counts();
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    // The custom chrome hides the native title bar, so this string is invisible IN the app — it
    // exists purely for the taskbar and Alt+Tab, which is exactly where it is needed.
    // The sandbox marks its window title so an iso instance is never mistaken for the real one
    // (lib.rs setup). Overwriting it here erased that marker on the first transition — keep the
    // suffix and put the count in front of it.
    let base = if crate::iso_mode() {
        "Castellyn [ISO SANDBOX]"
    } else {
        "Castellyn"
    };
    let _ = w.set_title(&if blocked > 0 {
        format!("({blocked}) {base}")
    } else {
        base.to_string()
    });
    // Flash only while unfocused: requesting attention on the window the user is already in is
    // noise. `None` clears a flash that a previous transition started.
    let focused = w.is_focused().unwrap_or(false);
    let want = (blocked > 0 || limited > 0) && !focused;
    // A badge on the taskbar button says HOW MANY without the window being visible; the flash only
    // says "something happened". Windows has no badge-count API for desktop apps — the documented
    // route is an overlay icon, so the dot is drawn and applied as one.
    #[cfg(windows)]
    {
        let overlay = if blocked > 0 {
            Some(badge_image((248, 81, 73)))
        } else if limited > 0 {
            Some(badge_image((239, 68, 68)))
        } else {
            None
        };
        let _ = w.set_overlay_icon(overlay);
    }
    let _ = w.request_user_attention(if want {
        Some(tauri::UserAttentionType::Informational)
    } else {
        None
    });
}

pub(crate) fn attention_counts() -> (usize, usize) {
    let map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
    let mut blocked = 0;
    let mut limited = 0;
    for t in map.values() {
        match t.last_emitted.as_deref() {
            Some("blocked") => blocked += 1,
            Some("limited") => limited += 1,
            _ => {}
        }
    }
    (blocked, limited)
}

/// Prune week-old hook files (claude session ids in them feed session restore, so recent ones are
/// kept across app restarts). Runs on the poll thread, not on `setup()`: it is a read_dir + a
/// metadata call per file, and the first frame should not wait on however many accumulated.
fn prune_stale_hook_files() {
    let Some(dir) = status_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|m| m.elapsed().map(|d| d.as_secs() > 7 * 86_400).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        prune_stale_hook_files();
        // Re-prune periodically (~every 5 min) so a long-uptime session self-cleans hook files that go
        // stale mid-run — the one startup prune never revisits them.
        const PRUNE_EVERY: u32 = (5 * 60 * 1000 / POLL_MS) as u32;
        let mut ticks: u32 = 0;
        loop {
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(PRUNE_EVERY) {
            prune_stale_hook_files();
        }
        crate::run_guarded("agent-status", || {
        // Nothing tracked (no session panes open) → no state can change, so skip the whole tick
        // instead of stat-ing the hook dir twice a second for the lifetime of the app.
        if TRACKS.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            return;
        }
        let dir = status_dir();
        // Read hook files OUTSIDE the tracks lock: on_output() takes that lock from every
        // PTY reader thread per chunk, so fs reads (AV scans can stall them) must not
        // serialize against it. Only local claude panes ever have a hook file.
        let claude_ids: Vec<(String, u64)> = {
            let map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
            map.iter()
                // Every local agent now writes this file: claude from its lifecycle hook, codex from
                // its `notify` program, opencode from a plugin. A remote pane simply never has one.
                .filter(|(_, t)| matches!(t.tool.as_str(), "claude" | "codex" | "opencode"))
                .map(|(id, t)| (id.clone(), t.hook_mtime))
                .collect()
        };
        // Report value plus the mtime that produced it, so the poll section can store it.
        let mut reports: HashMap<String, (u64, serde_json::Value)> = HashMap::new();
        if let Some(d) = dir.as_deref() {
            for (id, seen_mtime) in claude_ids {
                let path = d.join(format!("{id}.json"));
                // mtime gate: stat is far cheaper than read+parse. Skip when unchanged; a
                // missing file (mtime 0) is skipped too, exactly as the old read would fail.
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                if mtime == 0 || mtime == seen_mtime {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(v) = serde_json::from_str(&text) {
                        reports.insert(id, (mtime, v));
                    }
                }
            }
        }
        let mut events: Vec<StatusEvent> = Vec::new();
        {
            let mut map = TRACKS.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_ms();
            map.retain(|id, t| {
                if let Some((mtime, v)) = reports.get(id) {
                    apply_hook_report(v, t);
                    t.hook_mtime = *mtime;
                }
                if turn_end_expired(
                    &t.tool,
                    t.hook_state.as_deref(),
                    t.hook_ts,
                    t.last_output.load(Ordering::Relaxed),
                ) {
                    t.hook_state = None;
                }
                let state = compute(t, now);
                let menu = t.limit_menu.load(Ordering::Relaxed);
                // Re-emit when the menu flag flips too, not only on a `state` change — a menu appearing
                // while state stays "limited" must still reach the frontend. (item 21f)
                if t.last_emitted.as_deref() != Some(state) || t.last_menu != menu {
                    let prev = t.last_emitted.take();
                    log_transition(id, &t.label, prev.as_deref(), state, now);
                    t.last_emitted = Some(state.to_string());
                    t.last_menu = menu;
                    events.push(StatusEvent {
                        id: id.clone(),
                        state: state.to_string(),
                        claude_session_id: t.claude_session_id.clone(),
                        spawned_at: t.spawned_at,
                        prev,
                        label: t.label.clone(),
                        exited: t.exited,
                        hook_idle: t.hook_state.as_deref() == Some("idle"),
                        limit_menu: menu,
                        ask: t.ask.clone(),
                        ask_reason: t.ask_reason.clone(),
                        typed_recently: now.saturating_sub(t.last_user_input.load(Ordering::Relaxed))
                            < TYPED_RECENTLY_MS,
                        profile: t.profile.clone(),
                    });
                }
                !t.exited // exited sessions emit their final idle above, then drop
            });
        }
        let changed = !events.is_empty();
        for ev in events {
            notify_transition(&app, &ev);
            let _ = app.emit("agent-status", ev);
        }
        // Only when a state actually changed — the tooltip's attention line reads these counts.
        if changed {
            crate::update_tray_tooltip(&app);
            update_attention_surfaces(&app);
        }
        });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(tool: &str, now: u64) -> Track {
        Track {
            tool: tool.into(),
            // Default hookless (codex/opencode/remote-claude); the local-claude tests set it true.
            hook_expected: false,
            label: tool.into(),
            profile: String::new(),
            spawned_at: now,
            last_user_input: AtomicU64::new(0),
            last_output: AtomicU64::new(now),
            bytes_since_block: AtomicU64::new(0),
            prompt_on_screen: AtomicBool::new(false),
            limited: AtomicBool::new(false),
            bytes_since_limit: AtomicU64::new(0),
            limit_menu: AtomicBool::new(false),
            bytes_since_menu: AtomicU64::new(0),
            exited: false,
            hook_state: None,
            hook_ts: 0,
            hook_mtime: 0,
            claude_session_id: None,
            ask: None,
            ask_reason: None,
            last_emitted: None,
            last_menu: false,
        }
    }

    #[test]
    fn set_label_overrides_the_spawn_label_but_never_blanks_it() {
        // The toast body is built from `label`, so this is the whole point of the frontend push:
        // "claude · cc5" must become the project-aware string the attention strip shows.
        // Unique id — TRACKS is global and other tests run in parallel.
        let id = "tsetlabel1";
        on_spawn(id, "claude", "cc5", true);
        let label_of = |id: &str| {
            TRACKS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .map(|t| t.label.clone())
        };
        assert_eq!(label_of(id).as_deref(), Some("claude · cc5"));
        set_label(id, "cc5 · Docx и pdf обработка · Docx");
        assert_eq!(
            label_of(id).as_deref(),
            Some("cc5 · Docx и pdf обработка · Docx")
        );
        // An empty push must not blank the body — keep whatever was there.
        set_label(id, "");
        assert_eq!(
            label_of(id).as_deref(),
            Some("cc5 · Docx и pdf обработка · Docx")
        );
        // An unknown id is a no-op, not a panic: a detached window can push for a session that
        // already exited (the poll loop drops the track on exit).
        set_label("tsetlabelgone", "whatever");
        assert!(label_of("tsetlabelgone").is_none());
        TRACKS.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    #[test]
    fn codex_turn_end_ping_expires_on_the_next_turn_but_claude_idle_is_sticky() {
        let ts = 1_000_000;
        // The final answer and the prompt-box repaint land around the ping — still the same turn.
        assert!(!turn_end_expired("codex", Some("idle"), ts, ts - 500));
        assert!(!turn_end_expired("codex", Some("idle"), ts, ts + TURN_END_ECHO_MS));
        // Output well past the echo window: the user started a new turn.
        assert!(turn_end_expired("codex", Some("idle"), ts, ts + TURN_END_ECHO_MS + 1));
        // Codex reports nothing but end-of-turn, so no other state can expire this way.
        assert!(!turn_end_expired("codex", Some("working"), ts, ts + 60_000));
        assert!(!turn_end_expired("codex", None, ts, ts + 60_000));
        // Claude streams a full lifecycle: its idle holds until the next hook says otherwise,
        // otherwise the user's own typing at the prompt would fake a turn.
        assert!(!turn_end_expired("claude", Some("idle"), ts, ts + 60_000));
        assert!(!turn_end_expired("opencode", Some("idle"), ts, ts + 60_000));
    }

    #[test]
    fn a_codex_ping_makes_idle_authoritative_so_the_done_toast_may_fire() {
        let now = 1_000_000;
        let mut t = track("codex", now - STARTUP_GRACE_MS - 1);
        // Without a ping, silence alone is the only signal: idle, but no turn authority.
        t.last_output.store(now - ACTIVITY_IDLE_MS - 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now), "idle");
        assert_ne!(t.hook_state.as_deref(), Some("idle")); // hook_idle=false → no completion toast
        // The notify program reports the turn ended; idle is now authoritative even though the
        // agent printed its answer moments ago (no 15s of silence required).
        apply_hook_report(
            &serde_json::json!({ "state": "idle", "event": "agent-turn-complete", "ts": now }),
            &mut t,
        );
        t.last_output.store(now, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 100), "idle");
        assert_eq!(t.hook_state.as_deref(), Some("idle")); // hook_idle=true → "finished" is honest
    }

    #[test]
    fn a_blocked_report_carries_the_question_and_the_next_one_clears_it() {
        let now = 2_000_000;
        let mut t = track("claude", now);
        t.profile = "cc1".into();
        t.label = spawn_label("claude", "cc1");
        apply_hook_report(
            &serde_json::json!({
                "state": "blocked", "event": "PreToolUse", "ts": now,
                "reason": "question", "ask": "Which approach?",
                "cwd": r"E:\Scripts\Castellyn",
            }),
            &mut t,
        );
        assert_eq!(t.ask.as_deref(), Some("Which approach?"));
        assert_eq!(t.ask_reason.as_deref(), Some("question"));
        // Item 21: the project names itself from cwd, without waiting on the frontend's push.
        assert_eq!(t.label, "claude · cc1 · Castellyn");
        // A frontend label always outranks the cwd guess, and a later report must not undo it.
        set_label_on(&mut t, "cc1 · Castellyn · main");
        // The turn resumed: a stale question must not ride along into it.
        apply_hook_report(
            &serde_json::json!({ "state": "working", "event": "PostToolUse", "ts": now + 1,
                                 "cwd": r"E:\Scripts\Castellyn" }),
            &mut t,
        );
        assert_eq!(t.ask, None);
        assert_eq!(t.ask_reason, None);
        assert_eq!(t.label, "cc1 · Castellyn · main");
    }

    #[test]
    fn the_toast_wording_needs_both_the_question_and_its_reason() {
        assert_eq!(blocked_body_key(Some("rm -rf"), Some("permission")), "status.blocked_perm");
        assert_eq!(blocked_body_key(Some("Which one?"), Some("question")), "status.blocked_ask");
        // A reason with nothing to quote would render "…asking — " — use the plain body instead.
        assert_eq!(blocked_body_key(None, Some("question")), "status.blocked_body");
        // Hookless panes and the generic Notification event: no reason, no richer wording.
        assert_eq!(blocked_body_key(Some("something"), None), "status.blocked_body");
        assert_eq!(blocked_body_key(Some("x"), Some("notification")), "status.blocked_body");
        // An unknown reason from a future hook must degrade, not vanish.
        assert_eq!(blocked_body_key(Some("x"), Some("plan-approval")), "status.blocked_body");
        assert_eq!(blocked_body_key(None, None), "status.blocked_body");
    }

    /// `set_label` goes through the global TRACKS map; this test owns its Track directly.
    fn set_label_on(t: &mut Track, label: &str) {
        t.label = label.to_string();
    }

    #[test]
    fn limit_line_detection_is_tolerant() {
        assert!(is_limit_line("Claude usage limit reached. Your limit will reset at 3pm"));
        assert!(is_limit_line("5-hour limit reached"));
        assert!(is_limit_line("You are out of extra usage"));
        assert!(is_limit_line("USAGE LIMIT REACHED")); // case-insensitive
        // Newer banner family, per Claude Code errors.md — session / weekly / Opus windows.
        assert!(is_limit_line("You've hit your session limit \u{b7} resets 3:45pm"));
        assert!(is_limit_line("You've hit your weekly limit \u{b7} resets Mon 12:00am"));
        assert!(is_limit_line("You've hit your Opus limit \u{b7} resets 3:45pm"));
        assert!(!is_limit_line("the session limit is five hours per window")); // prose, no "hit your"
        assert!(!is_limit_line("running the linter, no limits here"));
        assert!(!is_limit_line("rate limited by the API")); // not our banner wording
        // Anchored: a bare "limit reached" in ordinary agent output must NOT flip the badge.
        assert!(!is_limit_line("the rate limit reached its cap in the test fixture"));
        assert!(!is_limit_line("// TODO: handle when the retry limit reached"));
    }

    #[test]
    fn limit_menu_detection() {
        // The interactive menu Claude shows on --resume into an active limit (item 21f).
        assert!(is_limit_menu("What do you want to do?\n> 1. Stop and wait for limit to reset\n  2. Upgrade your plan"));
        assert!(is_limit_menu("STOP AND WAIT FOR LIMIT to reset\nUPGRADE YOUR PLAN")); // case-insensitive
        // Two markers required: the bare option phrase alone (e.g. in prose) must NOT auto-fire a keypress.
        assert!(!is_limit_menu("you should stop and wait for limit to reset, then retry"));
        // The passive banner is NOT the menu — it needs no keypress, so it must not set limit_menu.
        assert!(!is_limit_menu("You've hit your session limit \u{b7} resets 3:45pm"));
        assert!(!is_limit_menu("running the linter, no limits here"));
    }

    #[test]
    fn resume_menu_detection() {
        // The large-session resume menu (item 21f): frontend picks option 1, continues immediately.
        assert!(is_resume_menu("> 1. Resume from summary (recommended)\n  2. Resume full session as-is\n  3. Don't ask me again"));
        assert!(is_resume_menu("RESUME FROM SUMMARY\nRESUME FULL SESSION as-is")); // case-insensitive
        // Two labels required: one alone (or prose) must NOT auto-fire a keypress.
        assert!(!is_resume_menu("I'll resume the full session where we left off"));
        assert!(!is_resume_menu("resume from summary of the meeting notes"));
        // The limit menu is not the resume menu.
        assert!(!is_resume_menu("> 1. Stop and wait for limit to reset\n  2. Upgrade your plan"));
    }

    #[test]
    fn tail_scan_classifies_signals_and_respects_the_window() {
        // The pure tail-scan classifies the three signals (menu > banner > resume precedence).
        assert_eq!(
            limit_signal_in_tail(b"What do you want to do?\n1. Stop and wait for limit to reset"),
            Some(LimitSignal::LimitMenu)
        );
        assert_eq!(
            limit_signal_in_tail(b"You've hit your session limit, resets 3pm"),
            Some(LimitSignal::Limit)
        );
        assert_eq!(
            limit_signal_in_tail(b"1. Resume from summary\n2. Resume full session as-is"),
            Some(LimitSignal::ResumeMenu)
        );
        assert_eq!(limit_signal_in_tail(b"just some normal agent output"), None);
        // A banner within the last TAIL(512) bytes is caught even when the chunk is larger…
        let mut near = vec![b'x'; 600];
        near.extend_from_slice(b"You've hit your session limit");
        assert_eq!(limit_signal_in_tail(&near), Some(LimitSignal::Limit));
        // …but one shoved > TAIL bytes before the end is intentionally missed (the #1 fragility this
        // guards): the endpoint monitor (limits.rs) is the backstop, not this scan.
        let mut far = b"You've hit your session limit".to_vec();
        far.extend(std::iter::repeat_n(b'x', 600));
        assert_eq!(limit_signal_in_tail(&far), None);
    }

    #[test]
    fn permission_menus_never_trigger_an_auto_keypress() {
        // SAFETY: Claude Code's approval prompts also render numbered options ("1. Yes / 2. No"),
        // and auto-continue presses "1". None of the three detectors may match a permission/approval
        // menu, or Castellyn would silently approve an edit/command on the user's behalf. Lock it.
        // The same contract now also covers the manual answer buttons (backlog 27): they are shown
        // only where `limit_signal_in_tail` fired, so its verdict is what has to stay honest.
        let permission_menus = [
            "Do you want to make this edit to lib.rs?\n\u{276f} 1. Yes\n  2. Yes, allow all edits this session\n  3. No, and tell Claude what to do differently (esc)",
            "Do you want to proceed?\n\u{276f} 1. Yes\n  2. Yes, and don't ask again for pwsh commands\n  3. No, and tell Claude what to do differently (esc)",
            "Do you trust the files in this folder?\n 1. Yes, proceed\n 2. No, exit",
            // Adversarial: an approval prompt that mentions "limit" must still not match — the
            // detectors anchor on "usage limit reached" / "hit your…limit" / the menu option labels.
            "Do you want to raise your usage limit?\n 1. Yes\n 2. No",
            // Adversarial: an approval prompt whose FILE PATHS carry the menu words, without the
            // full option labels — the detectors must still stay silent on their own merits.
            "Do you want to read resume-from-summary.md and stop-and-wait-for-limit.md?\n 1. Yes\n 2. No",
        ];
        // Adversarial second family: an approval prompt sharing the tail with GENUINE menu wording
        // that it is drawn OVER — a menu that had not yet scrolled away when the prompt appeared.
        // Here the menu detectors legitimately match, and only the veto stands between "press 1" and
        // an approved edit. These are the cases the answer buttons (backlog 27) made worth locking
        // down. The prompt is LAST in every sample: that is what makes it the thing on screen.
        let permission_menus_over_menu_text = [
            "1. Resume from summary\n2. Resume full session as-is\n\nDo you want to proceed?\n\u{276f} 1. Yes\n  2. Yes, and don't ask again",
            "What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan\n\nDo you want to make this edit?\n 1. Yes, allow all edits this session",
            "You've hit your 5-hour limit · resets 3pm\n\nDo you want to make this edit to lib.rs?\n\u{276f} 1. Yes\n  2. No",
        ];
        for m in permission_menus {
            assert!(!is_limit_line(m), "is_limit_line matched a permission menu: {m:?}");
            assert!(!is_limit_menu(m), "is_limit_menu matched a permission menu: {m:?}");
            assert!(!is_resume_menu(m), "is_resume_menu matched a permission menu: {m:?}");
        }
        for m in permission_menus.iter().chain(permission_menus_over_menu_text.iter()) {
            assert!(is_permission_menu(m), "is_permission_menu missed a permission menu: {m:?}");
            assert_eq!(
                limit_signal_in_tail(m.as_bytes()),
                None,
                "a menu signal escaped from a permission menu: {m:?}"
            );
        }
        // The veto must not swallow the REAL menus, or the whole feature silently dies.
        assert!(!is_permission_menu("What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan"));
        assert!(!is_permission_menu("1. Resume from summary\n2. Resume full session as-is\n3. Don't ask me again"));
    }

    #[test]
    fn an_answered_prompt_above_a_menu_does_not_hide_it() {
        // The regression a presence-based veto caused: the 512-byte window commonly still carries the
        // tail of whatever was on screen a moment ago, so an approval prompt the user ALREADY answered
        // suppressed the menu printed underneath it — and while the confound kept repainting, that
        // menu was never recognised at all (permanently unanswered, the feature's worst outcome).
        // Order is the whole signal: prompt ABOVE → the menu below it is what the user is looking at.
        let cases = [
            (
                "Do you want to make this edit to lib.rs?\n\u{276f} 1. Yes\n  2. No\n\nEdit applied.\n\nWhat do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan",
                Some(LimitSignal::LimitMenu),
            ),
            (
                // The reviewer's exact confound: a just-answered prompt above a fresh limit BANNER.
                "Do you want to proceed?\n\u{276f} 1. Yes\n  2. Yes, and don't ask again\n\nRunning…\n\nYou've hit your 5-hour limit \u{b7} resets 3pm",
                Some(LimitSignal::Limit),
            ),
            (
                "Do you trust the files in this folder?\n 1. Yes, proceed\n 2. No, exit\n\nopened.\n\n1. Resume from summary\n2. Resume full session as-is",
                Some(LimitSignal::ResumeMenu),
            ),
            // Same three, prompt LAST → still vetoed. The rule is positional, not disabled.
            (
                "What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan\n\nDo you want to make this edit?\n\u{276f} 1. Yes",
                None,
            ),
        ];
        for (text, want) in cases {
            assert_eq!(limit_signal_in_tail(text.as_bytes()), want, "positional veto: {text:?}");
        }
        // A vetoed candidate FALLS THROUGH to the next one instead of blanking the whole chunk: the
        // limit menu at the top is buried under a prompt (vetoed), but the resume menu printed BELOW
        // that prompt is the live thing and must still be reported. The old code returned None here.
        assert_eq!(
            limit_signal_in_tail(
                "What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan\n\nDo you want to proceed?\n\u{276f} 1. Yes\n\n1. Resume from summary\n2. Resume full session as-is".as_bytes()
            ),
            Some(LimitSignal::ResumeMenu)
        );
    }

    #[test]
    fn a_prompt_the_allowlist_misses_leaves_the_flag_stale() {
        // Pinned as a FACT, not a hope. `limit_menu` is retracted only by a permission prompt the
        // 5-pattern allowlist recognises, or by LIMIT_RESUME_BYTES of output — so a menu answered
        // outside Castellyn, followed by a differently-worded approval prompt under 4 KiB, leaves the
        // header button live and clickable over a prompt. The allowlist cannot be made complete, so
        // the keystroke path does NOT trust this flag: `menu_signal_in_text` re-reads the pane's live
        // rows first, and it correctly finds nothing answerable on that screen (asserted below).
        let id = "tstaleflag1";
        on_spawn(id, "claude", "cc1", true);
        let menu_up = |id: &str| {
            TRACKS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .map(|t| t.limit_menu.load(Ordering::Relaxed))
        };
        scan_limit(id, b"What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan");
        assert_eq!(menu_up(id), Some(true));
        // The user answers it in the terminal; the agent's NEXT question is a genuine approval prompt
        // worded in a way the allowlist does not carry, and it is far smaller than the byte-flood clear.
        let unknown_prompt = "Allow the agent to run `rm -rf build`? (y/n)";
        scan_limit(id, unknown_prompt.as_bytes());
        assert_eq!(menu_up(id), Some(true), "the stale flag is the known hole this test pins");
        // …and this is the guard that actually holds: nothing answerable is on that screen.
        assert_eq!(menu_signal_in_text(unknown_prompt.to_string()), None);
        // Sanity: the same call says "limitMenu" when the menu really IS on screen, so a refusal
        // means "moved on", never "the check is broken".
        assert_eq!(
            menu_signal_in_text(
                "\u{2502} What do you want to do?\n\u{2502} \u{276f} 1. Stop and wait for limit to reset\n\u{2502}   2. Upgrade your plan".into()
            ),
            Some("limitMenu")
        );
        TRACKS.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    #[test]
    fn a_permission_prompt_retracts_a_standing_menu_flag() {
        // The flag outliving its menu is the real-world version of the hazard above: the menu is
        // detected, THEN an approval prompt replaces it, and the byte-flood clear (LIMIT_RESUME_BYTES)
        // is far too coarse to notice a prompt box. Both the auto-keypress and the answer buttons key
        // off this flag, so the prompt itself has to retract it.
        let id = "tpermretract1";
        on_spawn(id, "claude", "cc1", true);
        let menu_up = |id: &str| {
            TRACKS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .map(|t| t.limit_menu.load(Ordering::Relaxed))
        };
        scan_limit(id, b"What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan");
        assert_eq!(menu_up(id), Some(true));
        // Ordinary output does NOT retract it — only a permission prompt does (the menu can still be
        // on screen behind a spinner repaint).
        scan_limit(id, b"waiting for the window to reset...");
        assert_eq!(menu_up(id), Some(true));
        scan_limit(id, "Do you want to make this edit to lib.rs?\n\u{276f} 1. Yes\n  2. No".as_bytes());
        assert_eq!(menu_up(id), Some(false));
        // An unknown id is a no-op, not a panic (the track is dropped as soon as the pane exits).
        scan_limit("tpermretractgone", b"Do you want to proceed?\n 1. Yes\n 2. No");
        TRACKS.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    #[test]
    fn limited_state_outranks_and_clears_on_resume() {
        let now = 1_000_000;
        let t = track("claude", now);
        // A limit banner flags the session; compute reports `limited` regardless of hook/activity
        // (track() leaves hook_state None, so the limited flag is what's exercised here).
        t.limited.store(true, Ordering::Relaxed);
        t.last_output.store(now + 10_000, Ordering::Relaxed); // even with recent output
        assert_eq!(compute(&t, now + 11_000), "limited");
        // A small trickle does NOT clear it (mirrors on_output's accumulate-then-compare).
        t.bytes_since_limit.store(0, Ordering::Relaxed);
        let small = 100u64;
        if t.limited.load(Ordering::Relaxed)
            && t.bytes_since_limit.fetch_add(small, Ordering::Relaxed) + small > LIMIT_RESUME_BYTES
        {
            t.limited.store(false, Ordering::Relaxed);
        }
        assert_eq!(compute(&t, now + 12_000), "limited");
        // A genuine resume (flood past the threshold) clears it → back to normal activity states.
        let big = LIMIT_RESUME_BYTES + 1;
        if t.limited.load(Ordering::Relaxed)
            && t.bytes_since_limit.fetch_add(big, Ordering::Relaxed) + big > LIMIT_RESUME_BYTES
        {
            t.limited.store(false, Ordering::Relaxed);
        }
        assert_ne!(compute(&t, now + 13_000), "limited");
    }

    #[test]
    fn activity_only_lifecycle() {
        // codex/opencode (no hooks): grace → working while output flows → idle on silence.
        let now = 1_000_000;
        let mut t = track("codex", now);
        assert_eq!(compute(&t, now + 1_000), "unknown"); // startup grace
        t.last_output
            .store(now + STARTUP_GRACE_MS + 1_000, Ordering::Relaxed);
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_000), "working");
        // A short think/tool pause (5s < ACTIVITY_IDLE_MS) must NOT false-flip to idle — the codex/
        // opencode false-"done" fix. (Was 4s → this asserted idle here; now it stays working.)
        assert_eq!(
            compute(&t, t.last_output.load(Ordering::Relaxed) + 5_000),
            "working"
        );
        assert_eq!(
            compute(
                &t,
                t.last_output.load(Ordering::Relaxed) + ACTIVITY_IDLE_MS + 1_000
            ),
            "idle"
        );
        t.exited = true;
        assert_eq!(compute(&t, now), "idle");
    }

    #[test]
    fn hook_expected_claude_stays_unknown_without_hook() {
        // A-residual: a LOCAL claude pane whose Agent-statuses hook is off/unwired must NOT infer
        // `working` from PTY activity — Claude Code + background hooks (claude-mem) print into the
        // terminal even when idle. It reports `unknown` (neutral) until a real hook event arrives.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_expected = true;
        assert_eq!(compute(&t, now + 1_000), "unknown"); // startup grace
        // Fresh PTY output past the grace (background noise) — must stay `unknown`, NOT `working`.
        t.last_output
            .store(now + STARTUP_GRACE_MS + 2_000, Ordering::Relaxed);
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_500), "unknown");
        // Once a real lifecycle hook reports, authority takes over immediately.
        t.hook_state = Some("working".into());
        assert_eq!(compute(&t, now + STARTUP_GRACE_MS + 2_500), "working");
        // Contrast: a genuinely hookless agent (remote claude / codex) DOES use the heartbeat.
        let r = track("claude", now); // hook_expected = false (remote)
        r.last_output
            .store(now + STARTUP_GRACE_MS + 2_000, Ordering::Relaxed);
        assert_eq!(compute(&r, now + STARTUP_GRACE_MS + 2_500), "working");
    }

    #[test]
    fn hook_authority_and_self_heal() {
        let now = 1_000_000;
        let mut t = track("claude", now);
        // Hook says blocked → stays blocked while the prompt just repaints (small trickle,
        // no byte burst) even long after the block.
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.last_output.store(now + 200, Ordering::Relaxed); // the prompt menu painting itself
        assert_eq!(compute(&t, now + 60_000), "blocked");
        // …until a byte burst floods in (user approved, agent resumed its turn).
        t.bytes_since_block
            .store(BLOCKED_RESUME_BYTES + 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 3_000), "working");
        // Hook-idle is authoritative even with echo activity (typing in the prompt box).
        t.hook_state = Some("idle".into());
        t.last_output.store(now + 10_000, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 10_100), "idle");
        // Hook-working self-heals to idle only after the LONG backstop (Esc interrupt fires no Stop
        // hook). A sub-backstop gap is still an active turn — flipping it fired a false "done".
        t.hook_state = Some("working".into());
        assert_eq!(
            compute(&t, t.last_output.load(Ordering::Relaxed) + ACTIVITY_IDLE_MS + 1),
            "working"
        );
        assert_eq!(
            compute(&t, t.last_output.load(Ordering::Relaxed) + WORKING_SELFHEAL_MS + 1),
            "idle"
        );
    }

    #[test]
    fn status_event_carries_spawned_at() {
        // The poll-loop push site copies the track's spawn time into the emitted event so the
        // frontend can render "active for N". Guard against it landing as 0.
        let now = now_ms();
        let t = track("claude", now);
        let ev = StatusEvent {
            id: "s1".into(),
            state: "working".into(),
            claude_session_id: None,
            spawned_at: t.spawned_at,
            prev: None,
            label: t.label.clone(),
            exited: t.exited,
            hook_idle: false,
            limit_menu: false,
            ask: None,
            ask_reason: None,
            typed_recently: false,
            profile: String::new(),
        };
        assert_ne!(ev.spawned_at, 0);
        assert_eq!(ev.spawned_at, now);
    }

    #[test]
    fn blocked_clears_on_byte_burst_not_trickle() {
        // Item 6: a small post-block trickle (prompt repaint) keeps `blocked`; a substantial
        // byte burst (the agent resumed its turn) clears it.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.last_output.store(now + 500, Ordering::Relaxed);
        t.bytes_since_block.store(64, Ordering::Relaxed); // under the threshold
        assert_eq!(compute(&t, now + 2_000), "blocked");
        t.bytes_since_block
            .store(BLOCKED_RESUME_BYTES + 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 2_100), "working");
    }

    #[test]
    fn blocked_time_backstop_recovers_on_sparse_output() {
        // Item 6 backstop: little output (an Esc answer) never reaches the byte threshold,
        // but once past the stuck ceiling with real post-block output it recovers to working.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.bytes_since_block.store(32, Ordering::Relaxed); // below BLOCKED_RESUME_BYTES
        t.last_output
            .store(now + BLOCKED_RESUME_MS + 3_000, Ordering::Relaxed); // real post-block output
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS - 1_000), "blocked"); // before ceiling
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS + 1_000), "working"); // after ceiling
    }

    #[test]
    fn the_time_ceiling_may_not_fire_while_the_prompt_is_still_on_screen() {
        // Backlog 23. The regression this protects: the user walks away from a permission prompt,
        // something small repaints past BLOCKED_RESUME_MS, and 20 s later BLOCKED_STUCK_MS declares
        // the turn resumed — the pane goes `working`, the attention badge clears, and the agent sits
        // unanswered with nothing on screen saying so. The prompt is STILL the last thing painted,
        // so the clock has no business overruling it.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.bytes_since_block.store(32, Ordering::Relaxed); // below BLOCKED_RESUME_BYTES
        t.last_output
            .store(now + BLOCKED_RESUME_MS + 3_000, Ordering::Relaxed); // real post-block output
        t.prompt_on_screen.store(true, Ordering::Relaxed);
        // Long past the ceiling that used to flip it — the screen still shows the prompt.
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS + 1_000), "blocked");
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS * 10), "blocked");
        // The byte burst is NOT vetoed: item 6 tuned it as the resume signal, and letting a screen
        // proxy override it could pin a pane at `blocked` for good.
        t.bytes_since_block
            .store(BLOCKED_RESUME_BYTES + 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now + 3_000), "working");
    }

    #[test]
    fn an_unrelated_chunk_while_the_prompt_is_up_never_reports_working() {
        // THE cardinal sin this feature must not commit. `prompt_on_screen` is per-CHUNK, and the
        // terminal is shared: Claude Code's own background hooks (claude-mem) print into it while a
        // permission box sits unanswered on screen. That chunk carries no prompt wording, so the
        // flag drops to false — which must mean "no evidence", never "the user answered". An earlier
        // draft read it as an answer and reported `working` at the first stray byte, clearing the
        // attention badge on a pane that was still waiting.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.bytes_since_block.store(32, Ordering::Relaxed); // below BLOCKED_RESUME_BYTES
        // The box paints, exactly as the scanner would see it.
        t.prompt_on_screen.store(
            is_strong_permission_prompt("Do you want to make this edit to lib.rs?\n 1. Yes\n 2. No"),
            Ordering::Relaxed,
        );
        assert!(t.prompt_on_screen.load(Ordering::Relaxed));
        // …then something unrelated prints while the box is still literally on screen.
        t.prompt_on_screen.store(
            is_strong_permission_prompt("claude-mem: indexed 12 files"),
            Ordering::Relaxed,
        );
        t.last_output
            .store(now + BLOCKED_RESUME_MS + 1, Ordering::Relaxed);
        assert_eq!(compute(&t, now + BLOCKED_RESUME_MS + 100), "blocked");
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS - 1), "blocked");
        // Accepted residual, unchanged from before the feature: with no prompt in the latest chunk
        // there is nothing to veto the ceiling, so the 20 s backstop still fires. That is the old
        // behaviour, not a new claim — and a pane pinned at `blocked` forever is worse.
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS + 1_000), "working");
    }

    #[test]
    fn a_block_with_no_visible_prompt_still_needs_the_time_ceiling() {
        // Why BLOCKED_STUCK_MS was NOT removed. PERMISSION_MARKERS is a hand-authored allowlist and
        // PTY_TAIL is 512 bytes, so a prompt can block a pane without ever reaching the scanner.
        // With no evidence either way the clock is all there is; dropping it would leave this pane
        // `blocked` for the rest of the session.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.bytes_since_block.store(32, Ordering::Relaxed);
        t.last_output
            .store(now + BLOCKED_RESUME_MS + 3_000, Ordering::Relaxed);
        assert!(!t.prompt_on_screen.load(Ordering::Relaxed));
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS - 1_000), "blocked");
        // …and the ceiling is what eventually recovers it, exactly as before.
        assert_eq!(compute(&t, now + BLOCKED_STUCK_MS + 1_000), "working");
    }

    #[test]
    fn a_finished_turn_whose_answer_resembles_a_menu_stays_idle() {
        // The other half of the trade: the hookless arm turns silence into `blocked` on the strength
        // of the detector alone, so a loose detector nags "waiting" on every turn that happens to
        // end in an enumerated recommendation. `1. Yes` is ordinary prose; only the full prompt
        // SHAPE counts.
        let now = 1_000_000;
        let t = track("claude", now - STARTUP_GRACE_MS - 1); // hookless (remote claude)
        t.last_output.store(now, Ordering::Relaxed);
        t.prompt_on_screen.store(
            is_strong_permission_prompt("Recommendation:\n 1. Yes, ship it now\n 2. Wait for CI"),
            Ordering::Relaxed,
        );
        assert_eq!(compute(&t, now + ACTIVITY_IDLE_MS + 1), "idle");
        // The real thing — question line AND enumerated affirmative — is still recognised.
        t.prompt_on_screen.store(
            is_strong_permission_prompt("Do you want to proceed?\n 1. Yes\n 2. No"),
            Ordering::Relaxed,
        );
        assert_eq!(compute(&t, now + ACTIVITY_IDLE_MS + 1), "blocked");
    }

    #[test]
    fn a_hookless_pane_sitting_at_a_prompt_reports_blocked_not_idle() {
        // Backlog 23. Remote claude over SSH gets no lifecycle hook, so the heartbeat is its only
        // signal — and an agent waiting at an approval prompt is just as quiet as one that finished.
        // ACTIVITY_IDLE_MS of silence therefore greyed the dot and nobody was told anyone was
        // waiting. The prompt still being on screen is what tells the two apart.
        let now = 1_000_000;
        let t = track("claude", now - STARTUP_GRACE_MS - 1); // hook_expected=false → remote
        t.last_output.store(now, Ordering::Relaxed);
        t.prompt_on_screen.store(true, Ordering::Relaxed);
        // Output still flowing: an on-screen prompt must not pre-empt a live turn.
        assert_eq!(compute(&t, now + ACTIVITY_IDLE_MS - 1), "working");
        // Gone quiet with the prompt up → waiting for the user, not finished.
        assert_eq!(compute(&t, now + ACTIVITY_IDLE_MS + 1), "blocked");
        // Without a prompt on screen the same silence is a plain idle, exactly as before.
        t.prompt_on_screen.store(false, Ordering::Relaxed);
        assert_eq!(compute(&t, now + ACTIVITY_IDLE_MS + 1), "idle");
    }

    #[test]
    fn the_pty_scanner_raises_and_lowers_the_on_screen_prompt_flag() {
        // The wiring behind the two tests above: `scan_limit` is the only writer, and it has to both
        // raise the flag on the prompt AND lower it again on ordinary output, or `blocked` would
        // latch. Same tail window as the answer-button veto — no second copy of the wording.
        let id = "tpromptscreen1";
        on_spawn(id, "claude", "cc1", true);
        let up = |id: &str| {
            TRACKS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .map(|t| t.prompt_on_screen.load(Ordering::Relaxed))
        };
        assert_eq!(up(id), Some(false));
        scan_limit(
            id,
            "Do you want to make this edit to lib.rs?\n\u{276f} 1. Yes\n  2. No".as_bytes(),
        );
        assert_eq!(up(id), Some(true), "the prompt is on screen");
        // Ordinary output lowers the flag on its own, with no threshold to tune.
        scan_limit(id, b"Reading lib.rs ... done. Applying the edit.");
        assert_eq!(up(id), Some(false));
        // A limit menu owns the screen instead — also "no prompt up", so a stale flag can't survive.
        scan_limit(id, "Do you want to proceed?\n 1. Yes\n 2. No".as_bytes());
        assert_eq!(up(id), Some(true));
        scan_limit(id, b"What do you want to do?\n1. Stop and wait for limit to reset\n2. Upgrade your plan");
        assert_eq!(up(id), Some(false));
        // A menu-shaped list in ordinary prose is not a prompt: the flag makes a positive claim, so
        // the generic "1. yes" alone must not raise it.
        scan_limit(id, b"Two options: 1. Yes, ship it now  2. wait for CI. I recommend the first.");
        assert_eq!(up(id), Some(false));
        TRACKS.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    #[test]
    fn resumed_block_self_heals_after_silence() {
        // The resume condition latches (bytes_since_block only resets on a NEW blocked report), so
        // without a silence backstop this pane reported "working" forever once the burst landed —
        // e.g. an approved turn the user then Esc-interrupted, firing no Stop hook.
        let now = 1_000_000;
        let mut t = track("claude", now);
        t.hook_state = Some("blocked".into());
        t.hook_ts = now;
        t.last_output.store(now + 1_000, Ordering::Relaxed);
        t.bytes_since_block
            .store(BLOCKED_RESUME_BYTES + 1, Ordering::Relaxed);
        // Fresh output → still an active turn.
        assert_eq!(compute(&t, now + 1_000 + WORKING_SELFHEAL_MS - 1), "working");
        // Silent past the same backstop the `working` arm uses → idle, not working forever.
        assert_eq!(compute(&t, now + 1_000 + WORKING_SELFHEAL_MS + 1), "idle");
    }
}
