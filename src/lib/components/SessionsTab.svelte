<script lang="ts">
  import { onMount } from 'svelte';
  import TerminalPane from './TerminalPane.svelte';
  import FolderField from './FolderField.svelte';
  import Toggle from './Toggle.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import EmptyState from './EmptyState.svelte';
  import ModalShell from './ModalShell.svelte';
  import { SquareTerminal } from '@lucide/svelte';
  import { t } from '$lib/i18n';
  import {
    sessionWrite,
    sessionList,
    type SessionTool,
    type DetachPane,
    type SshHost,
    readSshHosts,
    testSshHost,
    saveSshHost,
    deleteSshHost,
    sshTarget,
    parseSshTarget,
    pickFolder,
    readCodexProfiles,
    readOpencode,
    readOpencodeModels,
    readStack,
    runStack,
    openUrl,
    type StackService,
    globalSessionCount,
    agentStatusHookStatus,
    agentStatusHookSet,
    readConfig,
    saveConfig,
    type AgentStatusHookState,
    type AgentStatusEvent,
    type LimitsStatusEvent,
    type ProfileInfo
  } from '$lib/ipc';
  import { pickResumeCandidate } from '$lib/limitSwitch';
  import { parseTsMs } from '$lib/relativeTime';
  import { agentSummary, type AgentPaneState } from '$lib/agentStatus.svelte';
  import { getMonitors, invalidateMonitors, openDetached } from '$lib/monitors';
  import Select from './Select.svelte';
  import { anchored } from '$lib/floating';
  import ProfileUsageBadge from './ProfileUsageBadge.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { markMoved, peekMoved } from '$lib/sessionMove';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { ARG_PRESETS, toggleFlag } from '$lib/sessionPresets';
  import { pushToast } from '$lib/toast.svelte';

  const MAX_PANES = 12; // each pane is a pwsh+tool process — cap to keep the machine responsive
  // F16: per-window MAX_PANES isn't enough — detached monitor windows + restore share one global
  // ceiling (lib.rs SESSION_LIMIT). Mirror it here to warn/gate before the backend hard-rejects spawn.
  const SESSION_LIMIT = 24;
  let globalCount = $state(0);
  async function refreshGlobalCount() {
    try {
      globalCount = await globalSessionCount();
    } catch {
      /* ignore — backend is the hard guard; this only drives the UI hint */
    }
  }

  let {
    profiles = [],
    profileInfos = [],
    visible = true,
    folderReq = null,
    confirmDestructive = true,
    onFolderReqConsumed
  }: {
    profiles?: string[];
    /** #21e: full profile records (OAuth + link health) for after-limit profile switching. */
    profileInfos?: ProfileInfo[];
    visible?: boolean;
    /** R8: mirror the global "confirm destructive actions" toggle (settings #120). */
    confirmDestructive?: boolean;
    // Deep-link from another tab (e.g. a fork card's terminal menu): prefill the launcher with this
    // folder; when `tool` is set the launcher opens it straight away (profile applies to claude).
    folderReq?: { path: string; tool?: SessionTool; profile?: string } | null;
    onFolderReqConsumed?: () => void;
  } = $props();

  type Pane = {
    key: string;
    profile: string;
    tool: SessionTool;
    cwd: string;
    args: string;
    remoteDir?: string;
    sshTarget?: string;
    name?: string;
    attachId?: string;
    ownsSession?: boolean;
    background?: boolean;
    space?: string; // herdr W3: project space this pane belongs to (undefined = default space)
  };
  function renamePane(key: string, name: string) {
    panes = panes.map((p) => (p.key === key ? { ...p, name: name || undefined } : p));
  }
  // The key (not the profile) identifies a pane, so the same profile can run in several at once.
  let panes = $state<Pane[]>([]);
  let seq = 0;
  let columns = $state(2);
  // herdr W2: collapsible left agent rail. Persisted via localStorage (synced by sessionPrefs).
  const RAILKEY = 'cmh-sessions-rail';
  let railOpen = $state(true);
  function setRail(v: boolean) {
    railOpen = v;
    try {
      localStorage.setItem(RAILKEY, v ? '1' : '0');
    } catch {
      /* ignore */
    }
  }
  let maximized = $state<string | null>(null); // key of the pane shown full-screen, or null

  // Persisted prefs: column count + last folder used per profile (so re-launching a profile lands
  // in the same place).
  const FKEY = 'cmh-sessions-folders';
  const CKEY = 'cmh-sessions-cols';
  const WKEY = 'cmh-sessions-workspaces';
  const DAKEY = 'cmh-sessions-defargs';
  const RRKEY = 'cmh-remote-recent'; // recently-used SSH remote start dirs (datalist for #19)
  const MLKEY = 'cmh-monitor-layout'; // saved "разнести" arrangement, offered for restore on launch
  let savedLayoutExists = $state(false); // is there a saved monitor layout to restore / forget (#13)
  let savedLayoutSummary = $state(''); // one-line spec of the saved monitor layout (tool@profile · folder)
  let layoutBannerDismissed = $state(false); // hide the inline monitor-restore banner once acted on
  let lastFolders = $state<Record<string, string>>({});
  // Default launch args, seeded into the phrase's args field for Claude/opencode.
  let defaultArgs = $state('');
  // Collapsible launcher settings (default args, projects root) — collapsed by default.
  let launcherOpen = $state(false);
  // herdr W1: the launcher (env segment + phrase + settings) lives in an anchored popover behind a
  // "＋ New session" button instead of eating ~4 permanent rows above the grid. newBtnEl anchors it.
  let newOpen = $state(false);
  let newBtnEl = $state<HTMLButtonElement | undefined>(undefined);
  // A workspace is a named set of session configs you can re-launch with one click.
  type WsConfig = { tool: SessionTool; profile: string; cwd: string; args: string; remoteDir?: string; sshTarget?: string };
  let workspaces = $state<Record<string, WsConfig[]>>({});
  // Lifecycle (L12): once opened this tab stays MOUNTED — it's display-toggled, not unmounted, on tab
  // switches (see +page.svelte's `sessionsEverOpened` + `{#if}`), so onDestroy does NOT fire when you
  // switch away and these listeners don't pile up in normal use. The teardown machinery
  // (offs/track/disposed) is kept defensive for a possible future lazy-unmount; `mounted` also gates
  // async state writes (checkReach) that may resolve after such an unmount.
  let mounted = true;
  let disposed = false;
  const offs: UnlistenFn[] = [];
  const track = (p: Promise<UnlistenFn>) => {
    p.then((un) => (disposed ? un() : offs.push(un))).catch(() => {});
  };
  onMount(() => {
    try {
      lastFolders = JSON.parse(localStorage.getItem(FKEY) ?? '{}');
      workspaces = JSON.parse(localStorage.getItem(WKEY) ?? '{}');
      defaultArgs = localStorage.getItem(DAKEY) ?? '';
      const favs = JSON.parse(localStorage.getItem(VKEY) ?? '[]');
      if (Array.isArray(favs)) favorites = favs;
      const recs = JSON.parse(localStorage.getItem(RECKEY) ?? '[]');
      if (Array.isArray(recs)) recents = recs;
      const sr = JSON.parse(localStorage.getItem(SRKEY) ?? '{}');
      if (sr && typeof sr === 'object' && !Array.isArray(sr)) spaceRecipe = sr;
      remoteRecent = JSON.parse(localStorage.getItem(RRKEY) ?? '[]');
      const c = Number(localStorage.getItem(CKEY));
      if (c >= 1 && c <= 3) columns = c;
      const fz = Number(localStorage.getItem('cmh-sessions-fontsize'));
      if (fz >= 8 && fz <= 28) globalFont = fz;
      launcherOpen = localStorage.getItem('cmh-sessions-launcher') === '1';
      railOpen = localStorage.getItem(RAILKEY) !== '0';
      const savedSpaces = JSON.parse(localStorage.getItem(SPACES_KEY) ?? 'null');
      if (Array.isArray(savedSpaces) && savedSpaces.length) spaces = savedSpaces;
      const savedActive = localStorage.getItem(SPACE_ACTIVE_KEY);
      if (savedActive && spaces.some((s) => s.id === savedActive)) activeSpace = savedActive;
    } catch {
      /* first run / private mode */
    }
    // Codex config.toml profiles → first-class `--profile` picker; best-effort, empty when none defined.
    readCodexProfiles()
      .then((v) => (codexProfiles = Array.isArray(v) ? v : []))
      .catch(() => {});
    // opencode's active model → placeholder for its launcher model picker (empty field = use default).
    readOpencode()
      .then((s) => (opencodeModel = s?.model ?? ''))
      .catch(() => {});
    // Real "<provider>/<model>" catalog fetched from opencode's providers (with their keys) → the model
    // field's datalist, so the user can pick instead of guessing the name. Best-effort, background.
    readOpencodeModels()
      .then((m) => (opencodeModels = Array.isArray(m) ? m : []))
      .catch(() => {});
    void loadStack(); // FreeLLMAPI stack status for the opencode launcher block
    // Re-attach sessions that survived a webview reload (#5): the backend keeps them running, so
    // mirror the still-alive ones back here as owner instead of orphaning them against SESSION_LIMIT.
    if (savedLive.length) {
      void (async () => {
        try {
          const alive = new Set(await sessionList());
          for (const s of savedLive) {
            if (alive.has(s.id)) {
              addPane({ tool: s.tool, profile: s.profile, cwd: s.cwd, args: s.args, remoteDir: s.remoteDir, sshTarget: s.sshTarget, attachId: s.id, ownsSession: true, name: s.name, space: s.space });
            }
          }
          // Dead entries = a full app restart (the PTYs died with it). Offer to rebuild
          // the set; claude panes resume their conversation via the captured session id.
          restorable = savedLive.filter((s) => !alive.has(s.id));
        } catch {
          /* backend gone / no restore */
        }
      })();
    }
    // SSH quick-connect dropdown: load saved + imported ~/.ssh/config hosts (1-click reconnect).
    readSshHosts()
      .then((h) => {
        sshHostList = h;
        checkReach(h);
      })
      .catch(() => {});
    refreshGlobalCount(); // F16: seed the global tally (other windows may already hold sessions)
    // A pane returned from a detached monitor window (← Castellyn): re-attach it here as the owner.
    track(
      listen<{ target: string; pane: DetachPane }>('pane:add', (e) => {
        const p = e.payload;
        if (p?.target !== 'main' || !p.pane?.sessionId) return;
        addPane({
          tool: p.pane.tool,
          profile: p.pane.profile ?? '',
          cwd: p.pane.cwd ?? '',
          args: p.pane.args ?? '',
          attachId: p.pane.sessionId,
          ownsSession: true
        });
      })
    );
    // A detached monitor window failed to build (open_monitor_window worker thread). The live session
    // is still running but its window never appeared — recover the single-pane case by re-attaching it
    // here, and always tell the user instead of silently "losing" the pane.
    track(
      listen<{ label: string; error: string }>('monitor-window-failed', (e) => {
        const label = e.payload?.label ?? '';
        if (label.startsWith('pane-')) {
          const sessionId = label.slice('pane-'.length);
          addPane({ tool: 'shell', profile: '', cwd: '', args: '', attachId: sessionId, ownsSession: true });
        }
        pushToast({ kind: 'error', title: t('sessions.monitorOpenFailed') });
      })
    );
    // Note a saved per-monitor arrangement so the launcher can offer to restore it as an INLINE banner
    // next to the in-grid session restore. (Was a floating bottom-right toast: it covered the launcher
    // buttons behind it AND read as the same thing as the in-grid restore — two confusingly-similar
    // "Восстановить" prompts. Inline + distinctly labelled fixes both — owner report 2026-07-06.)
    try {
      const saved = localStorage.getItem(MLKEY);
      savedLayoutExists = !!(saved && saved !== '{}');
      if (savedLayoutExists) {
        try {
          savedLayoutSummary = layoutSummary(JSON.parse(saved!));
        } catch {
          savedLayoutSummary = '';
        }
      }
    } catch {
      /* ignore */
    }
    return () => {
      mounted = false;
      disposed = true;
      offs.forEach((un) => un());
    };
  });
  $effect(() => {
    try {
      localStorage.setItem(CKEY, String(columns));
    } catch {
      /* ignore */
    }
  });
  function rememberFolder(profile: string, folder: string) {
    if (!profile) return;
    lastFolders = { ...lastFolders, [profile]: folder };
    try {
      localStorage.setItem(FKEY, JSON.stringify(lastFolders));
    } catch {
      /* ignore */
    }
  }

  // Unread-output markers for panes that printed while hidden (off-screen behind a maximized pane).
  let unread = $state<Record<string, boolean>>({});
  function onActivity(key: string) {
    if (maximized && maximized !== key) unread = { ...unread, [key]: true };
  }

  // ── Agent status (herdr-style): backend `agent-status` events → per-session state ──
  // Keyed by SESSION id. `done` is derived here: working/blocked → idle while the pane
  // wasn't focused (herdr's Idle+!seen); focusing the pane acknowledges it back to idle.
  let agentStates = $state<Record<string, AgentPaneState>>({});
  // claude conversation ids (from the lifecycle hook) — persisted with the live-pane
  // list so a restore after an app restart can respawn with `--resume <id>`.
  let claudeSids = $state<Record<string, string>>({});
  // Session spawn times (unix ms) from the backend StatusEvent — static; "active for N" is
  // derived from `Date.now() - spawnedAt[id]` on render.
  let spawnedAt = $state<Record<string, number>>({});
  onMount(() => {
    let un: UnlistenFn | undefined;
    listen<AgentStatusEvent>('agent-status', (e) => {
      const { id, state } = e.payload;
      if (e.payload.claudeSessionId) claudeSids = { ...claudeSids, [id]: e.payload.claudeSessionId };
      if (e.payload.spawnedAt && !spawnedAt[id]) spawnedAt = { ...spawnedAt, [id]: e.payload.spawnedAt };
      const prev = agentStates[id];
      const paneKey = Object.keys(sessionIds).find((k) => sessionIds[k] === id);
      const focused = paneKey != null && activeKey === paneKey && visible;
      const next: AgentPaneState =
        state === 'idle' && (prev === 'working' || prev === 'blocked') && !focused && e.payload.hookIdle === true
          ? 'done'
          : (state as AgentPaneState);
      agentStates = { ...agentStates, [id]: next };
    }).then((u) => (un = u));
    return () => un?.();
  });
  // Looking at a pane acknowledges its "done".
  $effect(() => {
    const id = activeKey ? sessionIds[activeKey] : undefined;
    if (id && agentStates[id] === 'done') agentStates = { ...agentStates, [id]: 'idle' };
  });

  // ── #21c: auto-continue a limited pane once its 5h window resets ─────────────────────────────
  // When Claude Code hits the 5h limit it prints "limit reached" and WAITS — the session stays alive
  // (marked `limited` by the status hook), it does not exit. We watch each profile's reset time
  // (limits-status.h5Reset) and, once it passes (+ a 30–90s jitter so N panes don't fire at once),
  // send the continuation text — localised to the UI language — into the live pane, exactly once per
  // limit episode. Config-gated (autoContinueOn) for rollback. (A pane whose session actually EXITED
  // is not the limit case — Claude waits, it doesn't die — so respawn-resume is deliberately omitted.)
  let limitsByProfile = $state<Record<string, LimitsStatusEvent>>({});
  const autoContinued = new Set<string>(); // pane keys already handled this limit episode
  const switchAttempted = new Set<string>(); // #21e: switch tried once per episode (else fall to wait)
  const contJitterMs: Record<string, number> = {};
  // z5_12: last real user keystroke per pane key — auto-continue defers while the user is typing.
  const lastUserInputAt: Record<string, number> = {};
  onMount(() => {
    let un: UnlistenFn | undefined;
    listen<LimitsStatusEvent>('limits-status', (e) => {
      limitsByProfile = { ...limitsByProfile, [e.payload.profile]: e.payload };
    }).then((u) => (un = u));
    // P3: self-scheduling tick — 12s while visible, 60s when the window is hidden. Auto-continue is
    // still valuable in the background (that's the whole point), so gate the CADENCE, not the feature:
    // a limited pane still resumes within a minute of its reset when Castellyn sits in the tray.
    let tickTimer: ReturnType<typeof setTimeout> | undefined;
    const scheduleTick = () => {
      tickTimer = setTimeout(() => {
        maybeAutoContinue();
        scheduleTick();
      }, document.hidden ? 60_000 : 12_000);
    };
    scheduleTick();
    return () => {
      un?.();
      if (tickTimer) clearTimeout(tickTimer);
    };
  });
  function maybeAutoContinue() {
    if (!autoContinueOn) return; // master escape hatch for ALL unattended limit handling (21c + 21e)
    // L11: profiles switched-to during THIS pass — so a second pane that flips limited in the same
    // tick doesn't pick the same free profile (pickResumeCandidate reads a snapshot not updated mid-loop).
    const claimedThisTick = new Set<string>();
    for (const p of panes) {
      const id = sessionIds[p.key];
      if (!id || agentStates[id] !== 'limited') {
        // Recovered (or pane gone) → re-arm so a later, separate limit episode gets its own attempt.
        autoContinued.delete(p.key);
        switchAttempted.delete(p.key);
        delete contJitterMs[p.key];
        continue;
      }
      if (autoContinued.has(p.key)) continue; // one attempt per episode — never a retry loop
      // #21e: switchProfile — respawn under a free OAuth profile immediately (once per episode). No
      // candidate → fall through to the wait-on-reset path below (the spec's "fallback wait").
      if (limitMode === 'switchProfile' && !switchAttempted.has(p.key)) {
        switchAttempted.add(p.key);
        const cand = pickResumeCandidate(p.profile, profileInfos, limitsByProfile, claimedThisTick);
        if (cand && switchPaneToProfile(p, id, cand)) {
          claimedThisTick.add(cand); // L11: reserve it so a later pane this tick picks a different one
          // The old pane is gone (switchPaneToProfile removed it + cleaned its keys); nothing to guard.
          pushToast({ kind: 'info', title: t('sessions.switchedProfile', { name: p.name ?? p.profile, profile: cand }) });
          continue;
        }
      }
      // Wait for the LATER of the two windows (V-18): a pane limited on the 7-day window still has
      // a near-future 5h reset, so keying on h5Reset alone would fire a "continue" into a session
      // that's still 7-day-exhausted. The binding window is whichever resets last.
      const lim = limitsByProfile[p.profile];
      // parseTsMs tolerates a numeric-epoch resets_at (backend may stringify it) — a bare Date.parse
      // would yield NaN and silently defeat the auto-continue scheduling on that input.
      const h5 = parseTsMs(lim?.h5Reset);
      const d7 = parseTsMs(lim?.d7Reset);
      const candidates = [h5, d7].filter(Number.isFinite) as number[];
      if (!candidates.length) continue; // no known reset yet — wait for the next poll
      const reset = Math.max(...candidates);
      if (contJitterMs[p.key] == null) contJitterMs[p.key] = 30_000 + Math.floor(Math.random() * 60_000);
      if (Date.now() < reset + contJitterMs[p.key]) continue;
      // z5_12: don't inject into a pane the user is actively using — defer (don't consume the
      // episode) while it's focused or had a keystroke in the last 8s; retry on the next tick.
      const focused = activeKey === p.key && visible;
      const recentInput = Date.now() - (lastUserInputAt[p.key] ?? 0) < 8_000;
      if (focused || recentInput) continue;
      autoContinued.add(p.key);
      sessionWrite(id, t('sessions.autoContinueText') + '\r');
      pushToast({ kind: 'info', title: t('sessions.autoContinueDone', { name: p.name ?? p.profile }) });
    }
  }
  // #21e: respawn a limited pane's conversation under a free OAuth profile. Spawns the new pane FIRST
  // (so a rejected spawn at the cap never strands the conversation), queues the continuation for when
  // its session arrives, then drops the old pane (its unmount kills the old, waiting session).
  function switchPaneToProfile(p: Pane, oldId: string, candidate: string): boolean {
    const sid = claudeSids[oldId];
    // Only a local claude conversation with a captured, charset-safe id can be resumed elsewhere.
    if (p.tool !== 'claude' || p.sshTarget || !sid || !/^[\w-]{1,64}$/.test(sid)) return false;
    // Always resume the LIVE captured conversation (sid), stripping any stale --resume/--continue from
    // the original launch args — otherwise a pane launched with `--resume <oldId>` would reopen that
    // stale pointer instead of the conversation the user is actually in now.
    const args = `${p.args
      .replace(/--resume\s+[\w-]+/g, '')
      .replace(/--(resume|continue)\b/g, '')
      .replace(/\s+/g, ' ')
      .trim()} --resume ${sid}`.trim();
    const newKey = addPane({ tool: 'claude', profile: candidate, cwd: p.cwd, args, name: p.name });
    if (!newKey) return false; // cap/ceiling — keep the old pane, caller falls back to wait
    pendingContinue[newKey] = t('sessions.autoContinueText');
    // Old pane is gone → purge its keys so the episode-tracking Sets/maps don't leak across switches.
    panes = panes.filter((x) => x.key !== p.key);
    delete paneRefs[p.key];
    autoContinued.delete(p.key);
    switchAttempted.delete(p.key);
    delete contJitterMs[p.key];
    if (maximized === p.key) maximized = null;
    return true;
  }
  // Roll the counts up for the header chips + the sidebar badge (+page reads the store).
  const statusCounts = $derived.by(() => {
    const c = { blocked: 0, working: 0, done: 0, limited: 0 };
    for (const id of Object.values(sessionIds)) {
      const s = agentStates[id];
      if (s === 'blocked') c.blocked++;
      else if (s === 'working') c.working++;
      else if (s === 'done') c.done++;
      else if (s === 'limited') c.limited++;
    }
    return c;
  });
  $effect(() => {
    agentSummary.blocked = statusCounts.blocked;
    agentSummary.working = statusCounts.working;
    agentSummary.done = statusCounts.done;
    agentSummary.limited = statusCounts.limited;
  });

  // Agent-status lifecycle hook (Sessions ⚙ settings): wired into every claude profile.
  let statusHookState = $state<AgentStatusHookState | null>(null);
  const statusHookOn = $derived(
    !!statusHookState && statusHookState.wired.length > 0 && statusHookState.unwired.length === 0
  );
  onMount(async () => {
    try {
      statusHookState = await agentStatusHookStatus();
    } catch {
      /* backend unavailable — toggle stays disabled */
    }
  });
  async function toggleStatusHook(enabled: boolean) {
    try {
      statusHookState = await agentStatusHookSet(enabled);
      pushToast({ kind: 'success', title: t(enabled ? 'sessions.statusHookOnToast' : 'sessions.statusHookOffToast') });
    } catch (e) {
      pushToast({ kind: 'error', title: String(e) });
    }
  }
  // Nudge to enable the Agent-statuses hook when it's off but there are live LOCAL claude panes —
  // the only ones affected (remote claude / codex / opencode are hookless by nature and keep their
  // PTY heartbeat). Without the hook a local claude pane reports `unknown` (neutral dot), so this
  // surfaces the one-click fix. Dismissible + persisted; auto-hides once the hook is on.
  const NUDGE_KEY = 'cmh-status-nudge-dismissed';
  let statusNudgeDismissed = $state(false);
  onMount(() => {
    statusNudgeDismissed = localStorage.getItem(NUDGE_KEY) === '1';
  });
  const showStatusNudge = $derived(
    statusHookState != null &&
      !statusHookOn &&
      !statusNudgeDismissed &&
      panes.some((p) => p.tool === 'claude' && !p.sshTarget)
  );
  function dismissStatusNudge() {
    statusNudgeDismissed = true;
    localStorage.setItem(NUDGE_KEY, '1');
  }
  // Sound / OS-toast preferences for status transitions (the backend reads them from config).
  let statusSounds = $state(true);
  let statusNotify = $state(true);
  // #21c: auto-continue a limited pane after its 5h reset. Config-only escape hatch (no UI toggle).
  let autoContinueOn = $state(true);
  // #21e: after-limit behaviour — 'wait' (auto-continue on reset) | 'switchProfile' (respawn under a
  // free OAuth profile immediately). Has a UI control (saved via saveLimitMode).
  let limitMode = $state<'wait' | 'switchProfile'>('wait');
  onMount(async () => {
    try {
      const c = await readConfig();
      statusSounds = c.statusSounds ?? true;
      statusNotify = c.statusNotify ?? true;
      autoContinueOn = c.autoContinueOnReset ?? true;
      limitMode = c.limitMode === 'switchProfile' ? 'switchProfile' : 'wait';
    } catch {
      /* defaults stand */
    }
  });
  async function saveLimitMode() {
    try {
      // R7: rev-safe write so a concurrent Settings-tab save of the same fields isn't clobbered.
      await saveConfig((c) => (c.limitMode = limitMode));
    } catch (e) {
      pushToast({ kind: 'error', title: String(e) });
    }
  }
  async function saveStatusPrefs() {
    try {
      await saveConfig((c) => {
        c.statusSounds = statusSounds;
        c.statusNotify = statusNotify;
      });
    } catch (e) {
      pushToast({ kind: 'error', title: String(e) });
    }
  }

  // Broadcast: mirror keystrokes from any pane to every running session.
  let broadcast = $state(false);
  // $state (not a plain object) so the persist effect below reacts when a pane's id arrives/clears.
  let sessionIds = $state<Record<string, string>>({});
  // #21e: continuation text queued for a just-switched pane, sent once its session id arrives (below).
  const pendingContinue: Record<string, string> = {};
  function onIdChange(key: string, id: string | null) {
    if (id) sessionIds = { ...sessionIds, [key]: id };
    else {
      const oldId = sessionIds[key];
      const { [key]: _drop, ...rest } = sessionIds;
      sessionIds = rest;
      // L10: the id-keyed maps below are never pruned otherwise — they'd grow one small entry per
      // ended session for the webview's lifetime. Drop the outgoing id once no remaining pane maps to
      // it (session ids are never reused). Immutable delete matches this file's reassign-to-update style.
      if (oldId && !Object.values(rest).includes(oldId)) {
        const { [oldId]: _a, ...restA } = agentStates;
        agentStates = restA;
        const { [oldId]: _c, ...restC } = claudeSids;
        claudeSids = restC;
        const { [oldId]: _s, ...restS } = spawnedAt;
        spawnedAt = restS;
      }
    }
    refreshGlobalCount(); // F16: a spawn/exit here moved the global tally — re-read it
    // #21e: a switched-in pane just got its session — let `claude --resume` load the conversation,
    // then type the continuation. The delay is a live-tuned heuristic (the TUI must be ready to read).
    if (id && pendingContinue[key]) {
      const text = pendingContinue[key];
      delete pendingContinue[key];
      setTimeout(() => sessionWrite(id, text + '\r'), 3000);
    }
  }
  // ── Reload survival (#5): persist spawned-here sessions, re-attach the ones still alive on mount ──
  const LIVE_KEY = 'cmh-sessions-live';
  type LivePane = { tool: SessionTool; profile: string; cwd: string; args: string; remoteDir?: string; sshTarget?: string; id: string; claudeSid?: string; name?: string; space?: string };
  // Captured synchronously at init — BEFORE the persist effect first runs and overwrites it with the
  // (empty) fresh panes — so a webview reload still sees the pre-reload session list.
  const savedLive: LivePane[] = (() => {
    try {
      return JSON.parse(localStorage.getItem(LIVE_KEY) || '[]');
    } catch {
      return [];
    }
  })();
  $effect(() => {
    try {
      const live: LivePane[] = panes
        .filter((p) => !p.attachId && sessionIds[p.key])
        .map((p) => ({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args, remoteDir: p.remoteDir, sshTarget: p.sshTarget, id: sessionIds[p.key], claudeSid: claudeSids[sessionIds[p.key]], name: p.name, space: p.space }));
      // #3: keep the pending restore set (last run's dead sessions) in LIVE_KEY until the user
      // accepts or dismisses the bar — else a second webview reload before they act loses the offer.
      // Merge by id so a since-re-attached pane isn't listed twice; when restorable clears (accept /
      // dismiss) this reverts to persisting only the live panes, so the stale set never sticks.
      const persisted = restorable.length
        ? [...live, ...restorable.filter((r) => !live.some((l) => l.id === r.id))]
        : live;
      localStorage.setItem(LIVE_KEY, JSON.stringify(persisted));
    } catch {
      /* ignore */
    }
  });
  // Broadcast and send-to-all are scoped to the ACTIVE project (space): with several projects
  // open, mirrored keystrokes must never leak into another project's panes or a remote SSH
  // session in a background space.
  function broadcastInput(data: string) {
    for (const p of spacePanes) {
      const id = sessionIds[p.key];
      if (id) sessionWrite(id, data);
    }
  }
  // One-shot: send a typed command (+Enter) to every running session OF THIS PROJECT, without
  // enabling continuous broadcast.
  let sendAllText = $state('');
  // Send-to-all fires a command (+Enter) into every live pane of the active space at once —
  // including SSH/remote panes. That's the most destructive surface in the app, so gate it behind
  // the canonical confirm dialog (project rule: destructive actions confirm first) showing the
  // exact command + the pane list.
  let confirmSend = $state<{ cmd: string; targets: string[]; keys: string[] } | null>(null);
  function sendToAll() {
    const cmd = sendAllText.trim();
    if (!cmd) return;
    // F15: list the exact panes the command lands in (tool@profile · cwd/host) — count alone hid
    // which sessions get hit, so the user couldn't catch a stray SSH pane before sending.
    // The pane KEYS are captured here too: what the dialog listed is exactly what gets hit,
    // even if the user switches project tabs while the confirm is open.
    const live = spacePanes.filter((p) => sessionIds[p.key]);
    const targets = live.map((p) => {
      const where = p.sshTarget ? `🖥 ${p.sshTarget}` : p.cwd || '~';
      return p.tool === 'claude' ? `${p.tool}@${p.profile} · ${where}` : `${p.tool} · ${where}`;
    });
    confirmSend = { cmd, targets, keys: live.map((p) => p.key) };
  }
  function doSendToAll() {
    if (!confirmSend) return;
    for (const k of confirmSend.keys) {
      const id = sessionIds[k];
      if (id) sessionWrite(id, confirmSend.cmd + '\r');
    }
    sendAllText = '';
    confirmSend = null;
  }

  // F14: generic destructive confirm — one callback-driven dialog so every ✕ gates first
  // (project rule: destructive actions confirm before mutating). Each caller passes its own copy + run().
  let confirmAsk = $state<{ title: string; message: string; details?: string[]; run: () => void } | null>(null);
  function askConfirm(opts: { title: string; message: string; details?: string[]; run: () => void }) {
    // R8: honor the global confirm-destructive toggle — same gate as +page's askConfirm.
    if (!confirmDestructive) {
      opts.run();
      return;
    }
    confirmAsk = opts;
  }

  // A persisted monitor-layout pane: a launch config (no live session) plus the captured claude
  // session id, so a restore after an app restart can respawn with `--resume`. claudeSid is optional
  // for backward-compat — layouts saved before this field just start fresh.
  type SavedPane = DetachPane & { claudeSid?: string };

  // "Разнести по мониторам": open a detached window per (non-primary) monitor and spread the running
  // panes across them as a grid. Each pane mirrors its LIVE session via attach (no respawn); the main
  // window keeps its panes too. If there's only one monitor, this is a no-op (with a hint).
  async function distributeToMonitors() {
    invalidateMonitors(); // a monitor may have been (un)plugged since the cached enumeration
    let mons;
    try {
      mons = await getMonitors();
    } catch {
      return;
    }
    const targets = mons.filter((m) => !m.primary);
    const use = targets.length ? targets : mons;
    if (use.length < 1 || mons.length < 2) {
      pushToast({ kind: 'info', title: t('sessions.distributeNone') });
      return;
    }
    // Assign each running pane to a target monitor (round-robin), grouped per monitor.
    type Entry = { key: string; dp: DetachPane };
    const byMon = new Map<number, Entry[]>();
    let i = 0;
    for (const p of panes) {
      const id = sessionIds[p.key];
      if (!id) continue;
      const m = use[i % use.length];
      const dp: DetachPane = {
        sessionId: id,
        title: p.name || p.tool,
        tool: p.tool,
        profile: p.profile,
        cwd: p.cwd,
        args: p.args,
        owns: true
      };
      const arr = byMon.get(m.index) ?? [];
      arr.push({ key: p.key, dp });
      byMon.set(m.index, arr);
      i++;
    }
    if (i === 0) {
      pushToast({ kind: 'info', title: t('sessions.distributeNone') });
      return;
    }
    // Open one detached grid-window per monitor; on success live-MOVE those panes out of the grid
    // (mark so their unmount doesn't kill the session — the monitor window now owns it via attach).
    const removeKeys = new Set<string>();
    const layout: Record<number, SavedPane[]> = {};
    for (const [idx, list] of byMon) {
      const ok = await openDetached(`mon-${idx}`, idx, list.map((e) => e.dp));
      if (!ok) continue; // monitor/window unavailable — leave those panes in the main grid
      for (const e of list) {
        if (e.dp.sessionId) markMoved(e.dp.sessionId);
        removeKeys.add(e.key);
      }
      // Persist the LAUNCH config (no live session id) so this monitor can be restored next launch.
      // Capture the claude session id (keyed by the live session id) for a `--resume` on restore.
      layout[idx] = list.map((e) => ({
        title: e.dp.title,
        tool: e.dp.tool,
        profile: e.dp.profile,
        cwd: e.dp.cwd,
        args: e.dp.args,
        owns: true,
        claudeSid: e.dp.sessionId ? claudeSids[e.dp.sessionId] : undefined
      }));
    }
    if (removeKeys.size) panes = panes.filter((p) => !removeKeys.has(p.key));
    if (Object.keys(layout).length) {
      try {
        localStorage.setItem(MLKEY, JSON.stringify(layout));
        savedLayoutExists = true;
      } catch {
        /* ignore */
      }
    }
  }

  // #13 — forget the saved monitor arrangement (stops the restore prompt on future launches).
  function forgetLayout() {
    try {
      localStorage.removeItem(MLKEY);
    } catch {
      /* ignore */
    }
    savedLayoutExists = false;
    pushToast({ kind: 'info', title: t('sessions.forgetLayoutDone') });
  }

  // Restore the last "разнести" arrangement: reopen a window per saved monitor and SPAWN fresh sessions
  // (the old PTYs died with the app). Skips monitors that no longer exist. Offered via a toast on launch.
  // F22: one-line spec of what a saved monitor layout will restore (tool@profile · folder), so the
  // launch prompt shows the panes before the user clicks restore.
  function layoutSummary(saved: Record<number, DetachPane[]>): string {
    return Object.values(saved)
      .flat()
      .map((p) => {
        const folder = p.cwd ? p.cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p.cwd : '~';
        return p.tool === 'claude' ? `${p.tool}@${p.profile ?? '?'} · ${folder}` : `${p.tool} · ${folder}`;
      })
      .join('; ');
  }
  async function restoreLayout() {
    let saved: Record<number, SavedPane[]>;
    try {
      saved = JSON.parse(localStorage.getItem(MLKEY) ?? '{}');
    } catch (e) {
      pushToast({ kind: 'error', title: t('sessions.restoreLayoutFail'), detail: String(e) });
      return;
    }
    invalidateMonitors(); // re-enumerate: the saved layout may target monitors that are now gone
    let mons;
    try {
      mons = await getMonitors();
    } catch (e) {
      pushToast({ kind: 'error', title: t('sessions.restoreLayoutFail'), detail: String(e) });
      return;
    }
    const have = new Set(mons.map((m) => m.index));
    for (const [idxStr, list] of Object.entries(saved)) {
      const idx = Number(idxStr);
      if (!have.has(idx) || !list?.length) continue;
      // Resume each saved claude pane from its captured session id (same charset guard as restoreLast).
      // Old layouts lack claudeSid → the guard fails and the pane just spawns fresh (no crash).
      const spawn = list.map((p) => {
        const args = p.args ?? '';
        if (
          p.tool === 'claude' && p.claudeSid &&
          /^[\w-]{1,64}$/.test(p.claudeSid) && !/--(resume|continue)\b/.test(args)
        ) {
          return { ...p, args: `${args} --resume ${p.claudeSid}`.trim() };
        }
        return p;
      });
      await openDetached(`mon-${idx}`, idx, spawn);
    }
  }

  // F16: block a new spawn at EITHER the per-window cap OR the global ceiling (globalCount already
  // includes this window's live sessions, so the OR can't double-count).
  const atLimit = $derived(panes.length >= MAX_PANES || globalCount >= SESSION_LIMIT);
  function rememberRecent(folder: string) {
    if (!folder) return;
    try {
      const prev: string[] = JSON.parse(localStorage.getItem('cmh-recent-folders') ?? '[]');
      const next = [folder, ...prev.filter((f) => f !== folder)].slice(0, 12);
      localStorage.setItem('cmh-recent-folders', JSON.stringify(next));
    } catch {
      /* ignore */
    }
  }
  function addPane(v: { tool: SessionTool; profile: string; cwd: string; args: string; remoteDir?: string; sshTarget?: string; attachId?: string; ownsSession?: boolean; name?: string; space?: string }) {
    // Don't block re-attaching an EXISTING session (e.g. a pane returned from a monitor) on the cap —
    // it's not a new spawn. Only new spawns count against MAX_PANES. Toast, not just null: several
    // callers (space "＋", Ctrl+Shift+D clone) have no disabled affordance of their own.
    if (atLimit && !v.attachId) {
      pushToast({ kind: 'error', title: t('sessions.limitNote', { n: MAX_PANES }) });
      return null;
    }
    const key = `${v.tool}:${v.profile || 'sh'}#${seq++}`;
    panes = [...panes, { key, profile: v.profile, tool: v.tool, cwd: v.cwd, args: v.args, remoteDir: v.remoteDir, sshTarget: v.sshTarget, attachId: v.attachId, ownsSession: v.ownsSession, name: v.name, space: v.space ?? activeSpace }];
    if (v.tool === 'claude') rememberFolder(v.profile, v.cwd);
    rememberRecent(v.cwd);
    // EVERY real spawn becomes a "recent" recipe — clones, tab "＋", stack launches and deep-links
    // used to bypass the menu memory. Re-attach isn't a new launch; an SSH pane whose host is no
    // longer saved can't round-trip to a recipe (locId lost) — skip it rather than record a
    // recipe that would silently launch locally.
    if (!v.attachId) {
      const loc = locIdFor(v.sshTarget);
      if (!v.sshTarget || loc) {
        recordRecent(v.tool as Env, v.profile, loc, v.cwd, v.remoteDir ?? '', v.args);
      }
    }
    // Auto-focus the new pane's terminal so the user can type immediately (the obvious next action
    // after launch) — one frame later, once the pane has mounted and grabbed its paneRef.
    requestAnimationFrame(() => paneRefs[key]?.focusTerminal());
    return key;
  }
  $effect(() => {
    try {
      localStorage.setItem('cmh-sessions-launcher', launcherOpen ? '1' : '0');
    } catch {
      /* ignore */
    }
  });
  $effect(() => {
    try {
      localStorage.setItem(DAKEY, defaultArgs);
    } catch {
      /* ignore */
    }
  });
  function closePane(key: string) {
    const closed = panes.find((p) => p.key === key);
    const movedOut = peekMoved(sessionIds[key] ?? '');
    panes = panes.filter((p) => p.key !== key);
    delete paneRefs[key]; // drop the unmounted pane's ref so the map doesn't retain stale keys
    if (maximized === key) maximized = null;
    // Broadcast is meaningless with one pane and its toggle is hidden — reset so input doesn't
    // keep getting mirrored invisibly.
    if (panes.length <= 1) broadcast = false;
    // Offer a one-click reopen (same tool/profile/folder/args). The old PTY is gone, so this
    // relaunches a fresh session rather than restoring scrollback. Skip for a live MOVE — the
    // session isn't closed, it just relocated to a monitor window.
    if (closed && !movedOut) {
      pushToast({
        kind: 'info',
        title: t('sessions.paneClosed', { name: paneLabel(closed) }),
        action: {
          label: t('sessions.reopen'),
          onClick: () =>
            addPane({ tool: closed.tool, profile: closed.profile, cwd: closed.cwd, args: closed.args })
        }
      });
    }
  }
  function closeAll() {
    panes = [];
    maximized = null;
    broadcast = false;
    for (const k in paneRefs) delete paneRefs[k]; // clear all refs (const map — delete keys in place)
  }
  // Resizable columns/rows: per-track fraction weights + draggable dividers. Explicit equal
  // fractions (not grid-auto-rows) guarantee equal default sizes.
  let colFr = $state<number[]>([1, 1]);
  let rowFr = $state<number[]>([1]);
  let gridEl: HTMLDivElement | undefined = $state();
  const activePanes = $derived(panes.filter((p) => !p.background));
  const bgPanes = $derived(panes.filter((p) => p.background));
  // herdr W2 rail helpers: env icon, status-dot class (mirrors TerminalPane), click-to-focus.
  const envIcon = (tool: string) => ENVS.find((e) => e.id === tool)?.icon ?? '';
  function railFocus(key: string) {
    if (maximized && maximized !== key) maximized = key; // switch which pane is full-screen
    requestAnimationFrame(() => paneRefs[key]?.focusTerminal());
  }
  // Right-click on a rail row → the pane's main actions (maximize / background / close) without
  // hunting for its header. The global handler suppresses WebView2's native menu everywhere else.
  let railMenuFor = $state<string | null>(null);
  let railMenuAnchor = $state<HTMLElement | null>(null);
  function openRailMenu(e: MouseEvent, key: string) {
    e.preventDefault();
    railMenuAnchor = e.currentTarget as HTMLElement;
    railMenuFor = key;
  }
  function railMenuAct(fn: (key: string) => void) {
    const k = railMenuFor;
    railMenuFor = null;
    if (k) fn(k);
  }
  // ── herdr W3: spaces (project tabs). Each pane belongs to a space; the grid + rail show only the
  //    ACTIVE space's panes, but EVERY pane stays MOUNTED (filtered by CSS, not unmounted) so no PTY
  //    dies on a space switch. Spaces list + active id persist (synced via sessionPrefs). ──
  const DEFAULT_SPACE = 'default';
  const SPACES_KEY = 'cmh-sessions-spaces';
  const SPACE_ACTIVE_KEY = 'cmh-sessions-space-active';
  let spaces = $state<{ id: string; name: string }[]>([{ id: DEFAULT_SPACE, name: t('sessions.spaceDefault') }]);
  let activeSpace = $state(DEFAULT_SPACE);
  let spaceEditId = $state<string | null>(null); // tab being renamed inline
  let spaceEditName = $state('');
  const focusMount = (n: HTMLElement) => {
    n.focus();
    if (n instanceof HTMLInputElement) n.select();
  };
  const paneSpace = (p: Pane) => p.space ?? DEFAULT_SPACE;
  const spacePanes = $derived(activePanes.filter((p) => paneSpace(p) === activeSpace));
  function persistSpaces() {
    try {
      localStorage.setItem(SPACES_KEY, JSON.stringify(spaces));
      localStorage.setItem(SPACE_ACTIVE_KEY, activeSpace);
    } catch {
      /* ignore */
    }
  }
  function switchSpace(id: string) {
    if (id === activeSpace) return;
    activeSpace = id;
    maximized = null; // the maximized pane may belong to another space
    broadcast = false; // continuous broadcast is per-project — don't leak it into the space we switch to
    persistSpaces();
  }
  // V2: "＋ agent here" on a project tab — one more agent by the project's CURRENT recipe: the last
  // explicit launch aimed at it (spaceRecipe), falling back to cloning its primary pane. Empty
  // project with no recipe → open the launcher for its first.
  function spacePlus(id: string) {
    const r = spaceRecipe[id];
    if (r) {
      const v = paneFrom(r.env, r.profile, r.locId, r.folder, r.remoteDir, r.args);
      if (v) addPane({ ...v, space: id });
      return; // v === null → paneFrom already toasted (e.g. the recipe's SSH host is gone)
    }
    const src = activePanes.find((p) => paneSpace(p) === id);
    if (src) {
      addPane({ tool: src.tool, profile: src.profile, cwd: src.cwd, args: src.args, remoteDir: src.remoteDir, sshTarget: src.sshTarget, space: id });
    } else {
      switchSpace(id);
      newOpen = true;
    }
  }
  function addSpace() {
    const id = `s${seq++}${Math.round(Math.random() * 1e4)}`;
    spaces = [...spaces, { id, name: `${t('sessions.spaceBase')} ${spaces.length + 1}` }];
    switchSpace(id);
  }
  function beginRenameSpace(id: string) {
    spaceEditId = id;
    spaceEditName = spaces.find((s) => s.id === id)?.name ?? '';
  }
  function commitRenameSpace() {
    const id = spaceEditId;
    const name = spaceEditName.trim();
    if (id && name) spaces = spaces.map((s) => (s.id === id ? { ...s, name } : s));
    spaceEditId = null;
    persistSpaces();
  }
  function deleteSpace(id: string) {
    if (spaces.length <= 1) return; // always keep at least one space
    const rest = spaces.filter((s) => s.id !== id);
    const fallback = rest[0].id;
    // Reassign that space's panes to the first remaining space — non-destructive, no PTY killed.
    panes = panes.map((p) => (paneSpace(p) === id ? { ...p, space: fallback } : p));
    if (spaceRecipe[id]) {
      const { [id]: _gone, ...rest2 } = spaceRecipe;
      spaceRecipe = rest2;
    }
    spaces = rest;
    if (activeSpace === id) activeSpace = fallback;
    persistSpaces();
  }
  function askDeleteSpace(id: string) {
    const n = activePanes.filter((p) => paneSpace(p) === id).length;
    if (n === 0) {
      deleteSpace(id);
      return;
    }
    askConfirm({
      title: t('sessions.spaceDeleteTitle'),
      message: t('sessions.spaceDeleteMsg', { n }),
      run: () => deleteSpace(id)
    });
  }
  // Move a pane to another project tab — pure reassignment (deleteSpace's pattern), PTY untouched.
  function movePaneToSpace(key: string, spaceId: string) {
    panes = panes.map((p) => (p.key === key ? { ...p, space: spaceId } : p));
    // A maximized pane that just left the active space would leave an empty full-screen view.
    if (maximized === key) maximized = null;
  }
  // Close every pane of the active project at once (one confirm, no per-pane reopen toasts).
  function closeSpacePanes() {
    const keys = spacePanes.map((p) => p.key);
    if (!keys.length) return;
    askConfirm({
      title: t('sessions.closeSpaceTitle'),
      message: t('sessions.closeSpaceMsg', { n: keys.length }),
      run: () => {
        panes = panes.filter((p) => !keys.includes(p.key));
        for (const k of keys) delete paneRefs[k];
        maximized = null;
        if (panes.length <= 1) broadcast = false;
      }
    });
  }
  // Drag a project tab over another to reorder (same live pattern as pane reorder above).
  let dragSpaceId = $state<string | null>(null);
  function onSpaceDragEnter(targetId: string) {
    if (!dragSpaceId || dragSpaceId === targetId) return;
    const from = spaces.findIndex((s) => s.id === dragSpaceId);
    const to = spaces.findIndex((s) => s.id === targetId);
    if (from < 0 || to < 0) return;
    const next = [...spaces];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    spaces = next;
  }
  function onSpaceDragEnd() {
    dragSpaceId = null;
    persistSpaces();
  }
  const spaceCount = (id: string) => activePanes.filter((p) => paneSpace(p) === id).length;
  function spaceWorst(id: string): '' | 'working' | 'blocked' | 'done' {
    let w: '' | 'working' | 'done' = '';
    for (const p of activePanes) {
      if (paneSpace(p) !== id) continue;
      const s = agentStates[sessionIds[p.key]] ?? null;
      if (s === 'blocked') return 'blocked';
      if (s === 'working') w = 'working';
      else if (s === 'done' && w !== 'working') w = 'done';
    }
    return w;
  }
  // Never show more columns than there are panes — 1 pane with "3 columns" selected should fill
  // the row, not sit in a third of it.
  const effCols = $derived(Math.min(columns, Math.max(1, spacePanes.length)));
  const rowCount = $derived(Math.max(1, Math.ceil(spacePanes.length / effCols)));
  // Persisted per-column-count widths (so a manual resize survives restarts).
  const COLFR_KEY = 'cmh-sessions-colfr';
  function loadColFr(n: number): number[] | null {
    try {
      const all = JSON.parse(localStorage.getItem(COLFR_KEY) ?? '{}');
      const v = all[n];
      return Array.isArray(v) && v.length === n ? v : null;
    } catch {
      return null;
    }
  }
  function saveColFr() {
    try {
      const all = JSON.parse(localStorage.getItem(COLFR_KEY) ?? '{}');
      all[effCols] = colFr;
      localStorage.setItem(COLFR_KEY, JSON.stringify(all));
    } catch {
      /* ignore */
    }
  }
  $effect(() => {
    // Only acts when the track count changes — restores saved widths, else equal fractions.
    if (colFr.length !== effCols) colFr = loadColFr(effCols) ?? Array(effCols).fill(1);
  });
  $effect(() => {
    if (rowFr.length !== rowCount) rowFr = Array(rowCount).fill(1);
  });
  const colBounds = $derived.by(() => {
    const total = colFr.reduce((s, f) => s + f, 0) || 1;
    const out: number[] = [];
    let acc = 0;
    for (let i = 0; i < colFr.length - 1; i++) {
      acc += colFr[i];
      out.push((acc / total) * 100);
    }
    return out; // percent positions of each divider
  });
  const rowBounds = $derived.by(() => {
    const total = rowFr.reduce((s, f) => s + f, 0) || 1;
    const out: number[] = [];
    let acc = 0;
    for (let i = 0; i < rowFr.length - 1; i++) {
      acc += rowFr[i];
      out.push((acc / total) * 100);
    }
    return out;
  });
  // Shared divider drag: `axis` picks width/clientX (col) vs height/clientY (row).
  function startResize(e: PointerEvent, k: number, axis: 'col' | 'row') {
    e.preventDefault();
    const fr = axis === 'col' ? colFr : rowFr;
    const span = (axis === 'col' ? gridEl?.clientWidth : gridEl?.clientHeight) || 1;
    const total = fr.reduce((s, f) => s + f, 0);
    const start = axis === 'col' ? e.clientX : e.clientY;
    const a = fr[k];
    const b = fr[k + 1];
    const move = (ev: PointerEvent) => {
      const pos = axis === 'col' ? ev.clientX : ev.clientY;
      const dFr = ((pos - start) / span) * total;
      const next = [...(axis === 'col' ? colFr : rowFr)];
      next[k] = Math.max(0.25, a + dFr);
      next[k + 1] = Math.max(0.25, b - dFr);
      if (axis === 'col') colFr = next;
      else rowFr = next;
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      if (axis === 'col') saveColFr(); // remember manual column widths
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function toggleMax(key: string) {
    maximized = maximized === key ? null : key;
  }

  function toggleBackground(key: string) {
    // A backgrounded pane leaves activePanes; if it's the maximized one the grid would render
    // nothing (dead grid) — mirror closePane and drop the maximize when sending TO background.
    if (maximized === key && !panes.find((p) => p.key === key)?.background) maximized = null;
    panes = panes.map((p) => (p.key === key ? { ...p, background: !p.background } : p));
  }

  // Short label for a pane (mirrors TerminalPane's title logic): the profile for Claude, else the
  // tool + the folder it runs in. Used by the maximized-mode session switcher.
  function paneLabel(p: Pane): string {
    if (p.name) return p.name;
    if (p.tool === 'claude') return p.profile || 'claude';
    const folder = p.cwd ? p.cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || '' : '';
    return folder ? `${p.tool} · ${folder}` : p.tool;
  }
  // Humanize a duration to minute granularity: "Nm" under an hour, else "Nh Mm".
  function humanizeMs(ms: number): string {
    const m = Math.floor(ms / 60000);
    return m < 60 ? `${m}m` : `${Math.floor(m / 60)}h ${m % 60}m`;
  }
  // Pane hover title with the session's elapsed time appended (when the backend reported a spawn).
  function paneTitleElapsed(p: Pane): string {
    const id = sessionIds[p.key];
    const spawn = id ? spawnedAt[id] : undefined;
    return spawn ? `${paneLabel(p)} · ${t('sessions.activeFor', { d: humanizeMs(Date.now() - spawn) })}` : paneLabel(p);
  }

  // ─── Settings (⚙): default args + SSH servers — all in one place, no dialogs ───
  // The "browse starts in" root moved out of the launcher UI (owner: no duplicate folder inputs);
  // FolderField still reads its last value from localStorage for the Browse dialog's start dir.
  function openSettings() {
    launcherOpen = true;
  }
  // Add-server form (inline in settings) — reuses the SSH host registry.
  let srvName = $state('');
  let srvTarget = $state('');
  let srvDir = $state('');
  let srvTesting = $state(false);
  let srvTest = $state<'ok' | 'fail' | null>(null);
  async function addServer() {
    const p = parseSshTarget(srvTarget);
    if (!p.host) return;
    const name = srvName.trim() || (p.user ? `${p.user}@${p.host}` : p.host);
    await saveSshHost({ id: '', name, host: p.host, port: p.port, user: p.user, keyPath: p.keyPath, remoteDir: srvDir.trim() || null, source: 'saved' });
    srvName = '';
    srvTarget = '';
    srvDir = '';
    srvTest = null;
    const list = await readSshHosts();
    sshHostList = list;
    checkReach(list);
  }
  async function deleteServer(id: string) {
    await deleteSshHost(id);
    const list = await readSshHosts();
    sshHostList = list;
  }
  function askDeleteServer(h: SshHost) {
    askConfirm({
      title: t('sessions.srvDeleteTitle', { name: h.name }),
      message: t('sessions.srvDeleteMsg'),
      run: () => deleteServer(h.id)
    });
  }
  async function testServer() {
    const p = parseSshTarget(srvTarget);
    if (!p.host) return;
    srvTesting = true;
    srvTest = null;
    try {
      srvTest = (await testSshHost(p.host, p.port)) ? 'ok' : 'fail';
    } catch {
      srvTest = 'fail';
    } finally {
      srvTesting = false;
    }
  }
  // SSH quick-connect dropdown: saved + ~/.ssh/config hosts → 1 click launches; "+ New SSH…" → dialog.
  let sshHostList = $state<SshHost[]>([]);
  // Auto reachability check: ping each host's port (test_ssh_host, TCP) and show a status dot.
  let sshReach = $state<Record<string, 'checking' | 'ok' | 'fail'>>({});
  async function checkReach(hosts: SshHost[]) {
    if (!hosts.length) return;
    // Probe all hosts at once (was a sequential for-await: N×2s of dots stuck on "checking"). Each
    // result lands as it returns; the `mounted` guard drops writes that resolve after a tab switch.
    sshReach = { ...sshReach, ...Object.fromEntries(hosts.map((h) => [h.id, 'checking' as const])) };
    await Promise.allSettled(
      hosts.map(async (h) => {
        const ok = await testSshHost(h.host, h.port ?? null).catch(() => false);
        if (mounted) sshReach = { ...sshReach, [h.id]: ok ? 'ok' : 'fail' };
      })
    );
  }
  // #11: reachability shown as a design-system `.dot` (ok/fail/checking-pulse) instead of an emoji
  // traffic-light. Title gives a text alt so it isn't colour-only.
  const reachDotClass = (s?: 'checking' | 'ok' | 'fail') =>
    s === 'ok' ? 'dot-ok' : s === 'fail' ? 'dot-fail' : 'dot-checking';
  const reachTitle = (s?: 'checking' | 'ok' | 'fail') =>
    s === 'ok' ? t('sessions.reachOk') : s === 'fail' ? t('sessions.reachFail') : t('sessions.reachChecking');
  // ─── Launcher: environment × location × folder × args, read as a phrase (№20 + №8) ───
  type Env = 'claude' | 'opencode' | 'codex' | 'shell';
  const ENVS: { id: Env; label: string; title: string; icon: string }[] = [
    {
      id: 'claude',
      label: 'Claude',
      title: t('sessions.envClaudeTip'),
      icon: '<svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><path d="M12 2l1.9 6.4a2 2 0 0 0 1.7 1.7L22 12l-6.4 1.9a2 2 0 0 0-1.7 1.7L12 22l-1.9-6.4a2 2 0 0 0-1.7-1.7L2 12l6.4-1.9a2 2 0 0 0 1.7-1.7z"/></svg>'
    },
    {
      id: 'opencode',
      label: 'opencode',
      title: t('sessions.envOpencodeTip'),
      icon: '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8.5 7L4 12l4.5 5M15.5 7L20 12l-4.5 5"/></svg>'
    },
    {
      id: 'codex',
      label: 'codex',
      title: t('sessions.envCodexTip'),
      icon: '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M9 9l-2 3 2 3M15 9l2 3-2 3"/></svg>'
    },
    {
      id: 'shell',
      label: 'shell',
      title: t('sessions.envShellTip'),
      icon: '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 8l4 4-4 4M12 16h7"/></svg>'
    }
  ];
  let lEnv = $state<Env>('claude');
  let lProfile = $state('');
  let lLoc = $state(''); // '' = this PC; else an SSH host id
  let lFolder = $state(''); // local folder (when lLoc==='')
  let lRemoteDir = $state(''); // remote start dir (when lLoc!=='')
  let remoteRecent = $state<string[]>([]); // recent remote dirs → datalist for the remote-dir input (#19)
  let lArgs = $state('');
  // The args field mirrors the ⚙ default-args until the user edits it (then it's theirs). This is
  // what makes editing "default args" in settings actually flow into the phrase (#16 — was seeded once).
  let argsTouched = $state(false);
  $effect(() => {
    // Seed per-harness: the ⚙ default-args are Claude-oriented (--dangerously-skip-permissions etc.),
    // so only Claude inherits them. codex/opencode/shell start empty — otherwise codex is launched
    // with a Claude flag it rejects (error: unexpected argument '--dangerously-skip-permissions').
    if (!argsTouched) lArgs = lEnv === 'claude' ? defaultArgs : '';
  });
  // Per-harness launch chips (claude seeds from ⚙ default-args instead — no chips there). The codex
  // `--profile` selection is now a first-class picker (below), not a chip.
  let codexProfiles = $state<string[]>([]);
  const launchChips = $derived(lEnv === 'claude' ? [] : (ARG_PRESETS[lEnv] ?? []));
  // First-class identity selectors for the non-claude agents (parity with claude's profile dropdown):
  // codex → a `config.toml` profile (--profile), opencode → a provider/model (--model). Empty = the
  // tool's own default. The selection is composed into the launch args at spawn time (composeArgs),
  // so the whole recents/favorites machinery round-trips it via `args` with no new recipe field.
  let lCodexProfile = $state('');
  let lCodexModel = $state(''); // codex --model override (empty = the profile's / config default)
  let lOpencodeModel = $state('');
  let opencodeModel = $state(''); // opencode's active model, shown as the picker placeholder
  let opencodeModels = $state<string[]>([]); // "<provider>/<model>" catalog for the picker datalist
  // D (live-smoke): the FreeLLMAPI stack surfaced right in the opencode launcher — status, a dashboard
  // link and a one-click start — so opencode isn't launched blindly against a dead gateway.
  let stackSvcs = $state<StackService[]>([]);
  let stackChecking = $state(false);
  let stackBusy = $state(false);
  const gatewaySvc = $derived(stackSvcs.find((s) => s.id === 'gateway'));
  const stackRunning = $derived(stackSvcs.filter((s) => s.enabled && s.running).length);
  const stackTotal = $derived(stackSvcs.filter((s) => s.enabled).length);
  async function loadStack() {
    stackChecking = true;
    try {
      stackSvcs = await readStack();
    } catch {
      /* stack.json missing / not configured — the block just shows unknown */
    } finally {
      stackChecking = false;
    }
  }
  async function startStack() {
    stackBusy = true;
    try {
      await runStack('start');
      pushToast({ kind: 'success', title: t('sessions.stackStarted') });
      await loadStack();
    } catch (e) {
      pushToast({ kind: 'error', title: t('sessions.stackStartFail'), detail: String(e) });
    } finally {
      stackBusy = false;
    }
  }
  function openStackDashboard() {
    const url = gatewaySvc?.dashboard || (gatewaySvc ? `http://localhost:${gatewaySvc.port}` : '');
    if (url) openUrl(url).catch(() => {});
  }
  const PROFILE_RE = /(^|\s)(--profile|-p)(\s|=)/;
  const MODEL_RE = /(^|\s)(--model|-m)(\s|=)/;
  // Compose the identity selection (codex --profile/--model, opencode --model) into the free-text
  // args, skipping any flag the user already typed by hand (no doubling). Prepended so the flags read
  // first. This is the single place the picker turns into launch args.
  function composeArgs(env: Env, args: string): string {
    let a = args.trim();
    const add = (flag: string, re: RegExp) => {
      if (flag && !re.test(a)) a = a ? `${flag} ${a}` : flag;
    };
    if (env === 'codex') {
      add(lCodexModel.trim() && `--model ${lCodexModel.trim()}`, MODEL_RE);
      add(lCodexProfile.trim() && `--profile ${lCodexProfile.trim()}`, PROFILE_RE);
    } else if (env === 'opencode') {
      add(lOpencodeModel.trim() && `--model ${lOpencodeModel.trim()}`, MODEL_RE);
    }
    return a;
  }
  // Switching harness re-seeds the args field (unless the user already edited it) so a leftover Claude
  // flag doesn't leak into codex/opencode — and can't get pinned into a favorite mid-switch.
  function selectEnv(id: Env) {
    if (id === lEnv) return;
    lEnv = id;
    argsTouched = false;
  }
  function rememberRemote(dir: string) {
    const d = dir.trim();
    if (!d) return;
    remoteRecent = [d, ...remoteRecent.filter((x) => x !== d)].slice(0, 10);
    try {
      localStorage.setItem(RRKEY, JSON.stringify(remoteRecent));
    } catch {
      /* ignore */
    }
  }
  const LOC_ADD = '__add__';
  const locOptions = $derived([
    { value: '', label: t('sessions.locThisPc'), icon: '💻' },
    ...sshHostList.map((h) => ({
      value: h.id,
      label: h.name,
      iconHtml: `<span class="dot ${reachDotClass(sshReach[h.id])}" title="${reachTitle(sshReach[h.id])}"></span>`,
      hint: h.source === 'sshconfig' ? '~/.ssh/config' : undefined
    })),
    { value: LOC_ADD, label: t('sessions.locAdd') } // label already carries a ＋ — no icon (was double)
  ]);
  function onLocChange(v: string) {
    if (v === LOC_ADD) {
      lLoc = ''; // host management (add/test/save) lives in ⚙ settings
      openSettings();
      return;
    }
    lLoc = v;
    const h = sshHostList.find((x) => x.id === v);
    if (h) lRemoteDir = h.remoteDir ?? '';
  }
  // Seed the profile dropdown with the first profile until the user picks one.
  $effect(() => {
    if (!lProfile && profiles.length) lProfile = profiles[0];
  });
  // Build addPane input from a phrase (current or a saved favorite). null = unknown SSH host.
  function paneFrom(env: Env, profile: string, locId: string, folder: string, remoteDir: string, args: string) {
    const a = env === 'shell' ? '' : args.trim();
    const prof = env === 'claude' ? profile : '';
    if (!locId) return { tool: env as SessionTool, profile: prof, cwd: folder.trim(), args: a };
    const h = sshHostList.find((x) => x.id === locId);
    if (!h) {
      // The recipe (favorite/recent) references a since-deleted SSH host — say so instead of a
      // silent dead click (the unsafe-host branch below already toasts; this one must too).
      pushToast({ kind: 'error', title: t('sessions.hostGone') });
      return null;
    }
    let target: string;
    try {
      target = sshTarget(h); // throws on an arg-injection host/user (bad charset / whitespace / '-')
    } catch {
      pushToast({ kind: 'error', title: t('sessions.sshUnsafeHost', { host: h.host }) });
      return null;
    }
    return { tool: env as SessionTool, profile: prof, cwd: '', args: a, sshTarget: target, remoteDir: remoteDir.trim() || undefined };
  }
  // Display-safe target: sshTarget throws on an unsafe (arg-injection) host so the launch path can
  // refuse it — but the host LIST renders it too, and a throw there would crash the render. Show the
  // raw value with a ⚠ marker instead.
  function sshTargetLabel(h: SshHost): string {
    try {
      return sshTarget(h);
    } catch {
      return `${h.user ? h.user + '@' : ''}${h.host} ⚠`;
    }
  }
  function launchPhrase() {
    // Bake the identity selection (codex --profile / opencode --model) into the args once, so the
    // pane, the space-recipe and the recorded "recent" all carry it.
    const finalArgs = composeArgs(lEnv, lArgs);
    const v = paneFrom(lEnv, lProfile, lLoc, lFolder, lRemoteDir, finalArgs);
    if (v) {
      if (lLoc && lRemoteDir.trim()) rememberRemote(lRemoteDir); // SSH: keep the remote dir for next time
      setSpaceRecipe(activeSpace, lEnv, lProfile, lLoc, lFolder, lRemoteDir, finalArgs); // explicit choice
      addPane(v); // addPane records the recipe into "recents"
      newOpen = false; // close the launcher popover once a session starts
    }
  }
  // ─── Favorites: pin the whole phrase → 1-click relaunch ───
  type Fav = { id: string; env: Env; profile: string; locId: string; folder: string; remoteDir: string; args: string; label: string };
  const VKEY = 'cmh-sessions-favorites';
  let favorites = $state<Fav[]>([]);
  // `args` lets the label surface the chosen codex profile / opencode model (parsed from the composed
  // flag) so a codex/opencode favorite isn't just an anonymous "codex · folder".
  function favLabel(env: Env, profile: string, locId: string, folder: string, args = ''): string {
    const h = locId ? sshHostList.find((x) => x.id === locId) : null;
    const where = h
      ? `🖥 ${h.name}`
      : folder
        ? folder.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || folder
        : t('sessions.cwdShort');
    if (env === 'claude') return `${env}·${profile} · ${where}`;
    const id = args.match(/--(?:profile|model)\s+(\S+)/)?.[1] ?? '';
    return id ? `${env}·${id} · ${where}` : `${env} · ${where}`;
  }
  function pinCurrent() {
    const id = `f${Date.now()}${Math.round(Math.random() * 1e4)}`;
    const finalArgs = lEnv === 'shell' ? '' : composeArgs(lEnv, lArgs);
    const label = favLabel(lEnv, lProfile, lLoc, lFolder, finalArgs);
    favorites = [
      ...favorites,
      { id, env: lEnv, profile: lProfile, locId: lLoc, folder: lFolder, remoteDir: lRemoteDir, args: finalArgs, label }
    ];
    pushToast({ kind: 'success', title: t('sessions.pinned', { label }) }); // feedback — pinning was silent (#17)
  }
  function launchFav(f: Fav) {
    const v = paneFrom(f.env, f.profile, f.locId, f.folder, f.remoteDir, f.args);
    if (v) {
      setSpaceRecipe(activeSpace, f.env, f.profile, f.locId, f.folder, f.remoteDir, f.args);
      addPane(v);
    }
  }
  function removeFav(id: string) {
    favorites = favorites.filter((f) => f.id !== id);
  }
  function askRemoveFav(f: Fav) {
    askConfirm({
      title: t('sessions.favDeleteTitle'),
      message: t('sessions.favDeleteMsg', { label: f.label }),
      run: () => removeFav(f.id)
    });
  }
  $effect(() => {
    try {
      localStorage.setItem(VKEY, JSON.stringify(favorites));
    } catch {
      /* ignore */
    }
  });
  // ─── Launcher C (V9+V3): recents = last 5 unique launch recipes, shown in the ▾ menu ───
  type Recent = { env: Env; profile: string; locId: string; folder: string; remoteDir: string; args: string; label: string; when: number };
  const RECKEY = 'cmh-sessions-recents';
  let recents = $state<Recent[]>([]);
  const recipeKey = (r: { env: string; profile: string; locId: string; folder: string; remoteDir: string; args: string }) =>
    [r.env, r.profile, r.locId, r.folder.trim(), r.remoteDir.trim(), r.args.trim()].join('\u0000');
  function makeRecent(env: Env, profile: string, locId: string, folder: string, remoteDir: string, args: string): Recent {
    return {
      // Normalize like paneFrom does: profile only matters for claude, args never for shell —
      // otherwise a stale lProfile forks visually identical rows with different recipe keys.
      env, profile: env === 'claude' ? profile : '', locId, folder, remoteDir,
      args: env === 'shell' ? '' : args,
      label: favLabel(env, profile, locId, folder, env === 'shell' ? '' : args),
      when: Date.now()
    };
  }
  function recordRecent(env: Env, profile: string, locId: string, folder: string, remoteDir: string, args: string) {
    const rec = makeRecent(env, profile, locId, folder, remoteDir, args);
    recents = [rec, ...recents.filter((r) => recipeKey(r) !== recipeKey(rec))].slice(0, 5);
  }
  // Reverse-map a live pane's sshTarget back to the saved host id (recipes store locId; panes
  // store the rendered target). '' when local or when the host is gone.
  function locIdFor(target?: string): string {
    if (!target) return '';
    const h = sshHostList.find((x) => {
      try {
        return sshTarget(x) === target;
      } catch {
        return false;
      }
    });
    return h?.id ?? '';
  }
  // The project's "current recipe" = the last EXPLICIT launch aimed at it (form / menu). Clones
  // and the instant "＋" deliberately don't overwrite it — repeating a recipe isn't choosing one.
  const SRKEY = 'cmh-sessions-space-recipes';
  let spaceRecipe = $state<Record<string, Recent>>({});
  function setSpaceRecipe(space: string, env: Env, profile: string, locId: string, folder: string, remoteDir: string, args: string) {
    spaceRecipe = { ...spaceRecipe, [space]: makeRecent(env, profile, locId, folder, remoteDir, args) };
  }
  $effect(() => {
    try {
      localStorage.setItem(SRKEY, JSON.stringify(spaceRecipe));
    } catch {
      /* ignore */
    }
  });
  // A pinned recipe lives in the favorites section only — no duplicate row under "recent".
  const menuRecents = $derived(recents.filter((r) => !favorites.some((f) => recipeKey(f) === recipeKey(r))));
  function launchRecent(r: Recent) {
    const v = paneFrom(r.env, r.profile, r.locId, r.folder, r.remoteDir, r.args);
    if (v) {
      setSpaceRecipe(activeSpace, r.env, r.profile, r.locId, r.folder, r.remoteDir, r.args);
      addPane(v); // records + bumps recency
      plusMenuOpen = false;
    }
  }
  $effect(() => {
    try {
      localStorage.setItem(RECKEY, JSON.stringify(recents));
    } catch {
      /* ignore */
    }
  });
  // Split "＋": main zone = instant agent per the ACTIVE project's recipe (its primary pane —
  // exactly what the tab's own "＋" does); empty project → the full form. Chevron = memory menu.
  let plusMenuOpen = $state(false);
  let splitEl = $state<HTMLElement | undefined>(undefined);
  const mainPreset = $derived(activePanes.find((p) => paneSpace(p) === activeSpace) ?? null);
  // What the split-main button will actually launch: the space's explicit recipe wins, then the
  // primary pane's recipe. Keeps the label honest with spacePlus()'s priority order.
  const mainLabel = $derived(
    spaceRecipe[activeSpace]?.label ?? (mainPreset ? paneRecipeLabel(mainPreset) : null)
  );
  function paneRecipeLabel(p: Pane): string {
    const where = p.sshTarget
      ? `🖥 ${p.sshTarget}`
      : p.cwd
        ? p.cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p.cwd
        : t('sessions.cwdShort');
    return p.tool === 'claude' && p.profile ? `${p.profile} · ${where}` : `${p.tool} · ${where}`;
  }
  function mainPlus() {
    plusMenuOpen = false;
    spacePlus(activeSpace);
  }
  // Deep-link (e.g. from a fork card's "Terminal" menu): prefill the phrase with that repo folder
  // (local). If the menu also picked a tool, open the session straight away (profile → claude only).
  $effect(() => {
    const f = folderReq;
    if (f == null) return;
    lLoc = '';
    lFolder = f.path;
    if (f.tool) {
      const prof = f.tool === 'claude' ? f.profile || lProfile || profiles[0] || '' : '';
      if (prof) lProfile = prof;
      addPane({ tool: f.tool, profile: prof, cwd: f.path, args: '' });
    }
    onFolderReqConsumed?.();
  });

  function duplicate(key: string) {
    const p = panes.find((x) => x.key === key);
    // V1 "clone": one more agent exactly like this one — same tool/folder/args AND same space
    // (a clone belongs to the project it was cloned in, not whatever space is active).
    if (p) addPane({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args, remoteDir: p.remoteDir, sshTarget: p.sshTarget, space: p.space });
  }

  // Drag a pane's title bar over another to reorder (live, as you hover).
  let dragKey = $state<string | null>(null);
  function onDragStart(key: string) {
    dragKey = key;
  }
  function onDragEnter(targetKey: string) {
    if (!dragKey || dragKey === targetKey || maximized) return;
    const from = panes.findIndex((p) => p.key === dragKey);
    const to = panes.findIndex((p) => p.key === targetKey);
    if (from < 0 || to < 0) return;
    const next = [...panes];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    panes = next;
  }
  function onDrop() {
    dragKey = null;
  }

  // Tab-scoped shortcuts (only while the Sessions tab is shown): Ctrl+Shift+T new session (plain
  // Ctrl+T is left to the focused shell), Ctrl+Alt+1/2/3 cols, Alt+N focus pane N, Ctrl+]/[ cycle.
  // Pane component refs, so a shortcut can move focus between terminals (and the tab can drive
  // search/zoom across every pane at once).
  type PaneApi = {
    focusTerminal: () => void;
    runExternalSearch: (q: string, next?: boolean) => void;
    setFontSize: (px: number) => void;
  };
  const paneRefs: Record<string, PaneApi | undefined> = {};
  let focusIdx = 0;

  // Search every pane at once (#52). Each pane runs the query through its own SearchAddon;
  // next=false steps to the previous match.
  let searchAllText = $state('');
  function searchAll(next = true) {
    for (const k in paneRefs) paneRefs[k]?.runExternalSearch(searchAllText, next);
  }
  // Debounce the per-keystroke incremental scan (#Fsess-09): each call fans a full-buffer
  // SearchAddon scan across every pane (scrollback up to 50k lines), so fast typing with several
  // panes open fires N heavy searches per character. Enter/arrow buttons still search immediately.
  let searchAllTimer: ReturnType<typeof setTimeout> | undefined;
  function searchAllDebounced() {
    clearTimeout(searchAllTimer);
    searchAllTimer = setTimeout(() => searchAll(true), 150);
  }

  // Synced zoom: push one font size to every pane (#60). Persisted in the shared font key so new
  // panes open at the same size.
  let globalFont = $state(13);
  function zoomAll(delta: number) {
    globalFont = Math.min(28, Math.max(8, globalFont + delta));
    for (const k in paneRefs) paneRefs[k]?.setFontSize(globalFont);
  }

  // Focus mode (#61): dim every pane except the hovered one (for screencasts) — pure CSS, no
  // tracking of which terminal holds keyboard focus.
  let focusMode = $state(false);
  let activeKey = $state(''); // pane whose terminal currently holds keyboard focus (#14)
  function cycleFocus(dir: 1 | -1) {
    // Only cycle VISIBLE panes (active space, non-background) — never focus a CSS-hidden pane.
    const list = maximized ? panes.filter((p) => p.key === maximized) : spacePanes;
    if (!list.length) return;
    // Step from the pane that ACTUALLY holds focus, not a stale phantom counter.
    const cur = list.findIndex((p) => p.key === activeKey);
    focusIdx = ((cur === -1 ? focusIdx : cur) + dir + list.length) % list.length;
    paneRefs[list[focusIdx].key]?.focusTerminal();
  }
  function onKey(e: KeyboardEvent) {
    if (!visible) return;
    if (e.ctrlKey && e.shiftKey && (e.key === 't' || e.key === 'T')) {
      e.preventDefault();
      launchPhrase();
    } else if (e.ctrlKey && e.shiftKey && (e.key === 'd' || e.key === 'D')) {
      // V1: clone the focused pane (same folder/env/args, same space) — the 90% "one more like this".
      e.preventDefault();
      if (activeKey) duplicate(activeKey);
    } else if (e.ctrlKey && e.altKey && (e.key === '1' || e.key === '2' || e.key === '3')) {
      // Ctrl+Alt+N — column count (moved off plain Alt+N, which now focuses a pane, #19).
      e.preventDefault();
      columns = Number(e.key);
    } else if (e.altKey && !e.ctrlKey && e.key >= '1' && e.key <= '9') {
      // Alt+N — focus the N-th visible pane (#19).
      const list = maximized ? panes.filter((p) => p.key === maximized) : spacePanes;
      const idx = Number(e.key) - 1;
      if (idx < list.length) {
        e.preventDefault();
        paneRefs[list[idx].key]?.focusTerminal();
      }
    } else if (e.ctrlKey && (e.key === ']' || e.key === '[')) {
      // Ctrl+] / Ctrl+[ — focus next / previous pane terminal.
      e.preventDefault();
      cycleFocus(e.key === ']' ? 1 : -1);
    } else if (e.ctrlKey && (e.key === 'PageUp' || e.key === 'PageDown')) {
      // Ctrl+PageDown / Ctrl+PageUp — next / previous project tab (space), wrap-around.
      if (spaces.length > 1) {
        e.preventDefault();
        const cur = spaces.findIndex((s) => s.id === activeSpace);
        const dir = e.key === 'PageDown' ? 1 : -1;
        switchSpace(spaces[(cur + dir + spaces.length) % spaces.length].id);
      }
    }
  }
  // ⌨ cheatsheet: the shortcuts above live only in scattered tooltips — one modal lists them all.
  let hotkeysOpen = $state(false);
  const HOTKEYS: [string, string][] = [
    ['Ctrl+Shift+T', 'sessions.hkNew'],
    ['Ctrl+Shift+D', 'sessions.hkClone'],
    ['Ctrl+Alt+1/2/3', 'sessions.hkCols'],
    ['Alt+1…9', 'sessions.hkFocusN'],
    ['Ctrl+] / Ctrl+[', 'sessions.hkCycle'],
    ['Ctrl+PgUp/PgDn', 'sessions.hkSpaces']
  ];

  // ── Workspaces: save the current set of panes under a name, re-launch it later ──
  let savingWs = $state(false);
  let wsName = $state('');
  const wsNames = $derived(Object.keys(workspaces));
  function persistWs() {
    try {
      localStorage.setItem(WKEY, JSON.stringify(workspaces));
    } catch {
      /* ignore */
    }
  }
  function saveWorkspace() {
    const name = wsName.trim();
    if (!name || !panes.length) return;
    workspaces = {
      ...workspaces,
      [name]: panes.map((p) => ({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args, remoteDir: p.remoteDir, sshTarget: p.sshTarget }))
    };
    persistWs();
    savingWs = false;
    wsName = '';
  }
  function launchWorkspace(name: string) {
    const list = workspaces[name] ?? [];
    const before = panes.length;
    for (const c of list) addPane(c);
    const restored = panes.length - before;
    if (restored < list.length) pushToast({ kind: 'info', title: t('sessions.restoredPartial', { n: restored, m: list.length }) });
  }

  // ── Restore after an app restart: rebuild the last session set (Wave 3) ──
  let restorable = $state<LivePane[]>([]);
  function restoreLast() {
    const before = panes.length;
    const total = restorable.length;
    for (const s of restorable) {
      let args = s.args;
      // Resume the conversation only for a LOCAL claude with a captured id and no
      // user-supplied resume/continue flag of its own. The id lands in a pwsh -Command
      // line, so gate its charset (session ids are uuid-shaped) — never trust the file.
      if (
        s.tool === 'claude' && !s.sshTarget && s.claudeSid &&
        /^[\w-]{1,64}$/.test(s.claudeSid) && !/--(resume|continue)\b/.test(args)
      ) {
        args = `${args} --resume ${s.claudeSid}`.trim();
      }
      addPane({ tool: s.tool, profile: s.profile, cwd: s.cwd, args, remoteDir: s.remoteDir, sshTarget: s.sshTarget, name: s.name, space: s.space });
    }
    // addPane's cap guard silently drops panes past the limit — tell the user how many landed.
    const restored = panes.length - before;
    if (restored < total) pushToast({ kind: 'info', title: t('sessions.restoredPartial', { n: restored, m: total }) });
    restorable = [];
  }
  function deleteWorkspace(name: string) {
    const { [name]: _drop, ...rest } = workspaces;
    workspaces = rest;
    persistWs();
  }
  function askDeleteWorkspace(name: string) {
    askConfirm({
      title: t('sessions.wsDeleteTitle', { name }),
      message: t('sessions.wsDeleteMsg', { count: (workspaces[name] ?? []).length }),
      run: () => deleteWorkspace(name)
    });
  }
</script>

<svelte:window onkeydown={onKey} />

<ConfirmDialog
  open={!!confirmSend}
  title={t('sessions.sendAllConfirmTitle')}
  message={t('sessions.sendAllConfirmMsg', { count: confirmSend?.targets.length ?? 0 })}
  details={confirmSend ? [confirmSend.cmd, ...confirmSend.targets] : []}
  confirmLabel={t('sessions.sendAllConfirmOk')}
  danger
  onConfirm={doSendToAll}
  onCancel={() => (confirmSend = null)}
/>

<!-- ⌨ hotkey cheatsheet — one place listing the tab-scoped shortcuts (they otherwise live only in
     scattered tooltips). ModalShell handles Escape/backdrop/focus. -->
<ModalShell open={hotkeysOpen} onClose={() => (hotkeysOpen = false)} size="sm" labelledBy="hk-title">
  <h3 id="hk-title" class="mb-sw-3 text-sw-base font-semibold">⌨ {t('sessions.hotkeys')}</h3>
  <div class="hk-list">
    {#each HOTKEYS as [combo, key] (combo)}
      <span class="hk-combo">{combo}</span>
      <span class="text-sw-sm text-sw-text-secondary">{t(key)}</span>
    {/each}
  </div>
  <div class="mt-sw-4 flex justify-end">
    <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (hotkeysOpen = false)}>{t('common.close')}</button>
  </div>
</ModalShell>

<!-- F14: generic destructive confirm for ✕ actions (remove favorite / SSH host / workspace). -->
<ConfirmDialog
  open={!!confirmAsk}
  title={confirmAsk?.title ?? ''}
  message={confirmAsk?.message ?? ''}
  details={confirmAsk?.details ?? []}
  confirmLabel={t('common.delete')}
  danger
  onConfirm={() => {
    confirmAsk?.run();
    confirmAsk = null;
  }}
  onCancel={() => (confirmAsk = null)}
/>

<div class="wrap">
  <header class="mb-sw-3 flex items-center justify-between gap-sw-4">
    <div class="flex items-baseline gap-sw-3 min-w-0">
      <h1 class="text-lg font-semibold">{t('sessions.title')}</h1>
      {#if statusCounts.blocked || statusCounts.working || statusCounts.done}
        <!-- herdr-style rollup: which sessions need a decision / are running / are ready to review -->
        <span class="status-sum" role="status">
          {#if statusCounts.blocked}<span class="ss ss-blocked status-bad">● {t('sessions.sumBlocked', { n: statusCounts.blocked })}</span>{/if}
          {#if statusCounts.working}<span class="ss ss-working status-warn">● {t('sessions.sumWorking', { n: statusCounts.working })}</span>{/if}
          {#if statusCounts.done}<span class="ss ss-done">● {t('sessions.sumDone', { n: statusCounts.done })}</span>{/if}
        </span>
      {:else}
        <p class="truncate text-sw-xs text-sw-text-muted">{t('sessions.subtitle')}</p>
      {/if}
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      {#if panes.length > 1}
        <input class="sw-input text-sw-xs" style="width:120px" bind:value={searchAllText}
          placeholder={t('sessions.searchAllPlaceholder')} title={t('sessions.searchAllTip')} spellcheck="false"
          oninput={searchAllDebounced} onkeydown={(e) => e.key === 'Enter' && searchAll(!e.shiftKey)} />
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => searchAll(false)} title={t('sessions.findPrev')} aria-label={t('sessions.findPrev')}>↑</button>
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => searchAll(true)} title={t('sessions.findNext')} aria-label={t('sessions.findNext')}>↓</button>
        <input class="sw-input text-sw-xs" style="width:130px" bind:value={sendAllText}
          placeholder={t('sessions.sendAllPlaceholder')} title={t('sessions.sendAllTip')} spellcheck="false"
          onkeydown={(e) => e.key === 'Enter' && sendToAll()} />
        <span class="text-sw-text-muted">·</span>
        <label class="flex cursor-pointer items-center gap-1" title={t('sessions.broadcastTip')}>
          <Toggle bind:checked={broadcast} ariaLabel={t('sessions.broadcast')} />
          <span class="text-sw-xs" class:broadcast-armed={broadcast} class:text-sw-text-secondary={!broadcast}
            >{broadcast ? t('sessions.broadcastArmed', { count: spacePanes.length }) : t('sessions.broadcast')}</span>
        </label>
        <span class="text-sw-text-muted">·</span>
      {/if}
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.layout')}</span>
      {#each [1, 2, 3] as c (c)}
        <button class="sw-btn sw-btn-ghost text-sw-xs" class:active={columns === c} onclick={() => (columns = c)}
          title="{t('sessions.layoutCols', { n: c })} · Ctrl+Alt+{c}">{c}</button>
      {/each}
      {#if panes.length}
        <!-- Redesign 2D: rare whole-grid actions leave the header row for an overflow menu —
             the strip keeps only search / command / broadcast / layout / close-all. -->
        <DropdownMenu
          title={t('sessions.moreActions')}
          items={[
            { label: `A− ${t('sessions.zoomAllOut')}`, onClick: () => zoomAll(-1) },
            { label: `A+ ${t('sessions.zoomAllIn')}`, onClick: () => zoomAll(1) },
            { label: `◎ ${t('sessions.focusMode')}${focusMode ? ' ✓' : ''}`, onClick: () => (focusMode = !focusMode) },
            { label: `⬈ ${t('sessions.distribute')}`, onClick: distributeToMonitors },
            { label: `⌨ ${t('sessions.hotkeys')}`, onClick: () => (hotkeysOpen = true) },
            ...(savedLayoutExists ? [{ label: `↺ ${t('sessions.forgetLayout')}`, onClick: forgetLayout }] : [])
          ]}
        />
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={closeAll} title={t('sessions.closeAllTip')}>
          {t('sessions.closeAll')}
        </button>
      {/if}
    </div>
  </header>

  {#if showStatusNudge}
    <!-- Nudge: the Agent-statuses hook is off, so live LOCAL claude panes show a neutral dot
         instead of working/idle (their status can't be inferred from PTY noise). One-click enable,
         or dismiss (persisted). Auto-hides once the hook is on or no local claude pane remains. -->
    <div class="status-nudge" role="note">
      <span class="nudge-msg">💡 {t('sessions.statusNudge')}</span>
      <button class="sw-btn sw-btn-primary text-sw-xs" onclick={() => toggleStatusHook(true)}>
        {t('sessions.statusNudgeEnable')}
      </button>
      <button class="nudge-x" onclick={dismissStatusNudge} title={t('common.close')} aria-label={t('common.close')}>×</button>
    </div>
  {/if}

  <!-- Launcher C (V9+V3): split "＋" — the main zone instantly spawns one more agent per the ACTIVE
       project's recipe (label shows what will run; empty project → the full form). The chevron opens
       the memory menu: recent recipes + favorites (moved here from bar chips) + "custom launch…". -->
  <div class="newbar">
    <span class="plus-split" bind:this={splitEl}>
      <button bind:this={newBtnEl} type="button" class="split-main" disabled={atLimit}
        onclick={mainPlus} title={mainLabel ? t('sessions.plusHereTip') : t('sessions.newSession')}>
        ＋ {#if mainLabel}{t('sessions.plusHere')}<span class="split-sub">· {mainLabel}</span>{:else}{t('sessions.newSession')}{/if}
      </button>
      <button type="button" class="split-chev" class:active={plusMenuOpen}
        onclick={() => (plusMenuOpen = !plusMenuOpen)} aria-expanded={plusMenuOpen} aria-haspopup="menu"
        title={t('sessions.plusMenuTip')} aria-label={t('sessions.plusMenuTip')}>▾</button>
    </span>
  </div>

  {#if plusMenuOpen && splitEl}
    <!-- Launcher C memory menu: recent recipes (1 click = repeat) + favorites + custom launch (form) -->
    <div class="plusmenu" role="menu" aria-label={t('sessions.plusMenuTip')} tabindex="-1"
      use:anchored={{ anchor: splitEl, onOutside: () => (plusMenuOpen = false) }}
      onkeydown={(e) => e.key === 'Escape' && (plusMenuOpen = false)}>
      {#if menuRecents.length}
        <div class="pm-hdr">{t('sessions.menuRecent')}</div>
        {#each menuRecents as r (recipeKey(r))}
          <button type="button" class="pm-item" role="menuitem" disabled={atLimit} onclick={() => launchRecent(r)}>
            <span class="pm-label">{r.label}</span>
            <span class="pm-path" title={r.locId ? r.remoteDir : r.folder}>{r.locId ? r.remoteDir : r.folder}</span>
          </button>
        {/each}
      {/if}
      {#if favorites.length}
        <div class="pm-hdr">{t('sessions.menuFavs')}</div>
        {#each favorites as f (f.id)}
          <div class="pm-row">
            <button type="button" class="pm-item" role="menuitem" disabled={atLimit}
              onclick={() => { launchFav(f); plusMenuOpen = false; }} title={t('sessions.favLaunchTip')}>
              <span class="pm-star">★</span>
              <span class="pm-label">{f.label}</span>
              <span class="pm-path" title={f.locId ? f.remoteDir : f.folder}>{f.locId ? f.remoteDir : f.folder}</span>
            </button>
            <button type="button" class="pm-x" onclick={() => askRemoveFav(f)} title={t('common.delete')} aria-label={t('common.delete')}>✕</button>
          </div>
        {/each}
      {/if}
      <button type="button" class="pm-item pm-custom" role="menuitem"
        onclick={() => { plusMenuOpen = false; newOpen = true; }}>✎ {t('sessions.customLaunch')}</button>
    </div>
  {/if}

  {#if newOpen && newBtnEl}
    <!-- Launcher popover: environment × location × folder × args, read as a phrase (№20 + №8) -->
    <div class="launcher launcher-pop" role="dialog" aria-label={t('sessions.newSession')} tabindex="-1"
      use:anchored={{ anchor: newBtnEl, onOutside: () => (newOpen = false) }}
      onkeydown={(e) => e.key === 'Escape' && (newOpen = false)}>
    <div class="launchhead">
      <div class="envseg" role="tablist" aria-label={t('sessions.dlgTool')}>
        {#each ENVS as e (e.id)}
          <button type="button" class="env-btn" class:sel={lEnv === e.id} onclick={() => selectEnv(e.id)}
            title={e.title} role="tab" aria-selected={lEnv === e.id}>
            <span class="env-ic">{@html e.icon}</span>{e.label}
          </button>
        {/each}
      </div>
      <button class="sw-btn sw-btn-ghost text-sw-xs" class:active={launcherOpen}
        onclick={() => (launcherOpen = !launcherOpen)} title={t('sessions.settingsTip')} aria-pressed={launcherOpen}>⚙ {t('sessions.settings')}</button>
    </div>

    <!-- The phrase: reads as a sentence and adapts to the chosen environment / location -->
    <div class="phrase">
      <span class="pw">{t('sessions.phRun')}</span>
      {#if lEnv === 'claude'}
        <span class="pw">{t('sessions.phProfile')}</span>
        <div class="psel"><Select bind:value={lProfile} options={profiles} placeholder={t('sessions.dlgProfile')} /></div>
      {:else if lEnv === 'codex'}
        <!-- Codex identity: config.toml profile (when any exist) + a --model override, so the launch
             model is explicit instead of "whatever codex defaulted to". -->
        {#if codexProfiles.length}
          <span class="pw">{t('sessions.phProfile')}</span>
          <div class="psel"><Select bind:value={lCodexProfile} options={codexProfiles} placeholder={t('sessions.phCodexDefault')} /></div>
        {/if}
        <span class="pw">{t('sessions.phModel')}</span>
        <input class="sw-input font-mono text-sw-xs pmodel" bind:value={lCodexModel}
          placeholder={t('sessions.phCodexModelPlaceholder')} spellcheck="false" autocomplete="off" />
        {#if !codexProfiles.length}
          <span class="ph-note" title={t('sessions.codexNoProfilesHint')}>{t('sessions.codexNoProfiles')}</span>
        {/if}
      {:else if lEnv === 'opencode'}
        <!-- opencode model as provider/model — a themed Select of the real catalog (fetched from the
             providers) when we have one; a free-form input as fallback so an unlisted model can be typed. -->
        <span class="pw">{t('sessions.phModel')}</span>
        {#if opencodeModels.length}
          <div class="psel psel-wide"><Select bind:value={lOpencodeModel} options={opencodeModels} placeholder={opencodeModel || t('sessions.phModelPlaceholder')} /></div>
        {:else}
          <input class="sw-input font-mono text-sw-xs pmodel" bind:value={lOpencodeModel}
            placeholder={opencodeModel || t('sessions.phModelPlaceholder')} spellcheck="false" autocomplete="off" />
        {/if}
      {/if}
      <span class="pw">{t('sessions.phOn')}</span>
      <div class="psel"><Select value={lLoc} onChange={onLocChange} options={locOptions} placeholder={t('sessions.locThisPc')} /></div>
      <span class="pw">{t('sessions.phIn')}</span>
      {#if lLoc === ''}
        <div class="pfolder"><FolderField bind:value={lFolder} placeholder={t('sessions.cwdShort')} /></div>
      {:else}
        <input class="sw-input grow font-mono text-sw-xs pfolder" bind:value={lRemoteDir}
          list="remote-dirs" placeholder={t('sessions.dlgSshRemoteDirPlaceholder')} spellcheck="false" autocomplete="off" />
        <datalist id="remote-dirs">
          {#each remoteRecent as d (d)}<option value={d}></option>{/each}
        </datalist>
      {/if}
      {#if lEnv !== 'shell'}
        <span class="pw">{t('sessions.phWith')}</span>
        <input class="sw-input grow font-mono text-sw-xs pargs" bind:value={lArgs} oninput={() => (argsTouched = true)}
          placeholder={t('sessions.dlgArgsPlaceholder')} spellcheck="false" autocomplete="off" />
        <!-- Per-harness flag chips: claude gets its flags via the ⚙ default-args seeding, so chips
             here cover the OTHER harnesses (codex --yolo etc. + its config.toml profiles). -->
        {#each launchChips as flag (flag)}
          <button type="button" class="argchip" class:on={lArgs.includes(flag)}
            onclick={() => { lArgs = toggleFlag(lArgs, flag); argsTouched = true; }}>{flag}</button>
        {/each}
      {/if}
      {#if lLoc && lEnv !== 'shell'}
        <span class="ssh-hint" title={t('sessions.sshToolHint', { tool: lEnv })}>{t('sessions.sshToolHint', { tool: lEnv })}</span>
      {/if}
      <button type="button" class="sw-btn sw-btn-ghost star" onclick={pinCurrent} title={t('sessions.pin')} aria-label={t('sessions.pin')}>★</button>
      <button type="button" class="sw-btn sw-btn-primary text-sw-xs" onclick={launchPhrase} disabled={atLimit} title="{t('sessions.phLaunch')} · Ctrl+Shift+T">▶ {t('sessions.phLaunch')}</button>
    </div>

    {#if lEnv === 'opencode'}
      <!-- D: FreeLLMAPI stack status + one-click start + dashboard, so opencode isn't launched blindly
           against a dead gateway ("Cannot connect"). Backend already exists (readStack / runStack). -->
      <div class="stackbar">
        <span class="dot {gatewaySvc?.running ? 'dot-ok' : gatewaySvc ? 'dot-fail' : 'dot-checking'}"
          title={gatewaySvc?.running ? t('sessions.stackGwUp') : t('sessions.stackGwDown')}></span>
        <span class="stk-label">{t('sessions.stackLabel')}{#if stackTotal} <span class="text-sw-text-muted">· {stackRunning}/{stackTotal}</span>{/if}</span>
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={loadStack} disabled={stackChecking}>
          {stackChecking ? t('common.busy') : t('sessions.stackCheck')}
        </button>
        {#if gatewaySvc && !gatewaySvc.running}
          <button class="sw-btn sw-btn-primary text-sw-xs" onclick={startStack} disabled={stackBusy}
            title={t('sessions.stackStartTip')}>{stackBusy ? t('common.busy') : t('sessions.stackStart')}</button>
        {/if}
        {#if gatewaySvc}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={openStackDashboard} title={t('sessions.stackDashboardTip')}>{t('sessions.stackDashboard')}</button>
        {/if}
      </div>
    {/if}

    <!-- Save the current panes as a workspace (favorites moved to the compact bar above the popover) -->
    {#if panes.length || savingWs}
      <div class="favs">
        {#if savingWs}
          <input class="sw-input text-sw-xs" style="width:160px" bind:value={wsName} placeholder={t('sessions.wsNamePlaceholder')}
            onkeydown={(e) => e.key === 'Enter' && saveWorkspace()} />
          <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!wsName.trim() || !panes.length} onclick={saveWorkspace}>{t('common.save')}</button>
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (savingWs = false)}>{t('common.cancel')}</button>
        {:else}
          <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!panes.length} onclick={() => (savingWs = true)}
            title={t('sessions.wsSaveTip')}>{t('sessions.wsSave')}</button>
        {/if}
      </div>
    {/if}

    <!-- Settings (⚙): default args + SSH servers — everything configurable, no dialogs. The folder /
         "browse starts in" root lives ONLY in the phrase now (owner: no duplicate folder inputs). -->
    {#if launcherOpen}
      <div class="settings">
        <div class="set-row">
          <span class="set-k" title={t('sessions.defaultArgsHint')}>{t('sessions.defaultArgs')}</span>
          <input class="sw-input grow font-mono text-sw-xs" bind:value={defaultArgs}
            placeholder={t('sessions.dlgArgsPlaceholder')} spellcheck="false" autocomplete="off" />
          {#each ARG_PRESETS.claude as flag (flag)}
            <button type="button" class="argchip" class:on={defaultArgs.includes(flag)}
              onclick={() => (defaultArgs = toggleFlag(defaultArgs, flag))}>{flag}</button>
          {/each}
        </div>
        <div class="set-row">
          <span class="set-k" title={t('sessions.statusHookHint')}>{t('sessions.statusHook')}</span>
          <Toggle checked={statusHookOn} disabled={!statusHookState} onCheckedChange={toggleStatusHook}
            title={t('sessions.statusHookHint')} />
          {#if statusHookState}
            <span class="text-sw-xs text-sw-text-muted" title={statusHookState.wired.join(', ')}>
              {t('sessions.statusHookCoverage', { wired: statusHookState.wired.length, total: statusHookState.wired.length + statusHookState.unwired.length })}
            </span>
          {/if}
          <span class="text-sw-text-muted">·</span>
          <label class="flex cursor-pointer items-center gap-1 text-sw-xs text-sw-text-secondary" title={t('sessions.statusSoundHint')}>
            <Toggle bind:checked={statusSounds} onCheckedChange={saveStatusPrefs} ariaLabel={t('sessions.statusSound')} />
            {t('sessions.statusSound')}
          </label>
          <label class="flex cursor-pointer items-center gap-1 text-sw-xs text-sw-text-secondary" title={t('sessions.statusToastHint')}>
            <Toggle bind:checked={statusNotify} onCheckedChange={saveStatusPrefs} ariaLabel={t('sessions.statusToast')} />
            {t('sessions.statusToast')}
          </label>
        </div>
        <div class="set-row">
          <span class="set-k" title={t('sessions.limitModeHint')}>{t('sessions.limitMode')}</span>
          <div class="psel">
            <Select value={limitMode}
              onChange={(v) => { limitMode = v === 'switchProfile' ? 'switchProfile' : 'wait'; saveLimitMode(); }}
              options={[
                { value: 'wait', label: t('sessions.limitModeWait') },
                { value: 'switchProfile', label: t('sessions.limitModeSwitch') }
              ]} />
          </div>
        </div>
        <div class="set-srv">
          <span class="set-k">{t('sessions.servers')}</span>
          <div class="srv-list">
            {#each sshHostList as h (h.id)}
              <span class="srv-chip">
                <span class="dot {reachDotClass(sshReach[h.id])}" title={reachTitle(sshReach[h.id])}></span>
                <span class="srv-n">{h.name}</span>
                <span class="srv-t font-mono">{sshTargetLabel(h)}</span>
                {#if h.source === 'saved'}
                  <button class="srv-x" onclick={() => askDeleteServer(h)} title={t('common.delete')} aria-label={t('common.delete')}>✕</button>
                {:else}
                  <span class="srv-cfg">~/.ssh/config</span>
                {/if}
              </span>
            {/each}
            {#if !sshHostList.length}<span class="text-sw-xs text-sw-text-muted">{t('sessions.dlgSshEmpty')}</span>{/if}
          </div>
          <div class="srv-add">
            <input class="sw-input text-sw-xs" style="width:130px" bind:value={srvName} placeholder={t('sessions.dlgSshName')} spellcheck="false" autocomplete="off" />
            <input class="sw-input grow font-mono text-sw-xs" bind:value={srvTarget} placeholder={t('sessions.dlgSshTargetPlaceholder')} spellcheck="false" autocomplete="off" />
            <input class="sw-input font-mono text-sw-xs" style="width:170px" bind:value={srvDir} placeholder={t('sessions.dlgSshRemoteDir')} spellcheck="false" autocomplete="off" />
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!srvTarget.trim() || srvTesting} onclick={testServer}>{t('sessions.dlgSshTest')}</button>
            {#if srvTest === 'ok'}<span class="text-sw-xs" style="color:var(--sw-status-up)">✓ {t('sessions.dlgSshTestOk')}</span>{/if}
            {#if srvTest === 'fail'}<span class="text-sw-xs status-bad">✕ {t('sessions.dlgSshTestFail')}</span>{/if}
            <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!srvTarget.trim()} onclick={addServer}>{t('sessions.serverAdd')}</button>
          </div>
        </div>
      </div>
    {/if}
    </div>
  {/if}

  {#if globalCount >= SESSION_LIMIT}
    <p class="mb-sw-2 text-sw-xs status-warn">{t('sessions.globalLimitNote', { n: SESSION_LIMIT })}</p>
  {:else if panes.length >= MAX_PANES}
    <p class="mb-sw-2 text-sw-xs status-warn">{t('sessions.limitNote', { n: MAX_PANES })}</p>
  {:else if globalCount >= SESSION_LIMIT - 4}
    <p class="mb-sw-2 text-sw-xs" style="color:var(--sw-text-muted)">{t('sessions.globalNearNote', { used: globalCount, max: SESSION_LIMIT })}</p>
  {/if}

  <!-- Restore the previous run's session set IN-GRID (claude panes resume their conversation) -->
  {#if restorable.length}
    <div class="restorebar">
      <span class="text-sw-xs">{t('sessions.restoreOffer', { n: restorable.length })}</span>
      <button class="sw-btn sw-btn-primary text-sw-xs" onclick={restoreLast}>{t('sessions.restoreDo')}</button>
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (restorable = [])}>{t('sessions.restoreDismiss')}</button>
    </div>
  {/if}

  <!-- Restore the previous per-MONITOR arrangement (detached windows) — distinct from the in-grid
       restore above (icon + "по мониторам") so the two aren't confused, and INLINE so it never covers
       the launcher buttons the way the old floating toast did. -->
  {#if savedLayoutExists && !layoutBannerDismissed}
    <div class="restorebar">
      <span class="text-sw-xs">🖥 {t('sessions.restoreLayoutPrompt')}{#if savedLayoutSummary} · <span class="text-sw-text-muted">{savedLayoutSummary}</span>{/if}</span>
      <button class="sw-btn sw-btn-primary text-sw-xs" onclick={() => { layoutBannerDismissed = true; void restoreLayout(); }}>{t('sessions.restoreLayoutAction')}</button>
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={forgetLayout}>{t('sessions.forgetLayout')}</button>
    </div>
  {/if}

  <!-- Saved workspaces: one click re-opens the whole set of sessions -->
  {#if wsNames.length}
    <div class="workspaces">
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.wsLabel')}</span>
      {#each wsNames as name (name)}
        <span class="ws-chip">
          <button class="ws-go" onclick={() => launchWorkspace(name)} title={t('sessions.wsLaunchTip', { name })}>
            ▶ {name} ({workspaces[name].length})
          </button>
          <button class="ws-del" onclick={() => askDeleteWorkspace(name)} title={t('sessions.wsDeleteTip', { name })} aria-label="✕">✕</button>
        </span>
      {/each}
    </div>
  {/if}

  <!-- herdr W3: project spaces. Switching a space shows only its panes; every pane stays mounted
       (CSS-filtered, never unmounted) so no live session dies. Double-click a tab to rename. -->
  {#if activePanes.length > 0 || spaces.length > 1}
    <div class="spaces" role="tablist" aria-label={t('sessions.agents')}>
      {#each spaces as sp (sp.id)}
        {@const worst = spaceWorst(sp.id)}
        <span class="space-tab" class:active={activeSpace === sp.id}
          draggable={spaces.length > 1 && spaceEditId !== sp.id}
          role="presentation"
          ondragstart={() => (dragSpaceId = sp.id)} ondragenter={() => onSpaceDragEnter(sp.id)}
          ondragover={(e) => e.preventDefault()} ondragend={onSpaceDragEnd}>
          {#if spaceEditId === sp.id}
            <input class="space-edit sw-input text-sw-xs" bind:value={spaceEditName} use:focusMount
              onkeydown={(e) => { if (e.key === 'Enter') commitRenameSpace(); else if (e.key === 'Escape') (spaceEditId = null); }}
              onblur={commitRenameSpace} />
          {:else}
            <button type="button" class="space-go" role="tab" aria-selected={activeSpace === sp.id}
              onclick={() => switchSpace(sp.id)} ondblclick={() => beginRenameSpace(sp.id)}
              oncontextmenu={(e) => { e.preventDefault(); beginRenameSpace(sp.id); }} title={t('sessions.spaceRename')}>
              {#if worst}<span class="dot" class:working={worst === 'working'} class:blocked={worst === 'blocked'} class:done={worst === 'done'}></span>{/if}
              <span class="space-name">{sp.name}</span>
              {#if spaceCount(sp.id)}<span class="space-count">{spaceCount(sp.id)}</span>{/if}
            </button>
            <button type="button" class="space-plus" onclick={() => spacePlus(sp.id)}
              title={t('sessions.spaceNewAgent')} aria-label={t('sessions.spaceNewAgent')}>＋</button>
            {#if spaces.length > 1}
              <button type="button" class="space-x" onclick={() => askDeleteSpace(sp.id)}
                title={t('common.delete')} aria-label={t('common.delete')}>✕</button>
            {/if}
          {/if}
        </span>
      {/each}
      <button type="button" class="space-add" onclick={addSpace} title={t('sessions.spaceNew')} aria-label={t('sessions.spaceNew')}>＋</button>
    </div>
  {/if}

  <!-- While one pane is maximized the others are hidden; this switcher keeps them visible and
       one-click reachable so you never lose track of running sessions. -->
  {#if maximized}
    <div class="maxbar">
      {#each spacePanes as p (p.key)}
        <button class="maxchip" class:active={maximized === p.key}
          onclick={() => { maximized = p.key; unread = { ...unread, [p.key]: false }; }} title={paneTitleElapsed(p)}>
          <span class="maxchip-dot" class:unread={unread[p.key] && maximized !== p.key}></span>{paneLabel(p)}
        </button>
      {/each}
      <span class="spacer"></span>
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => { maximized = null; unread = {}; }}>⤡ {t('sessions.restore')}</button>
    </div>
  {/if}

  {#if activePanes.length}
    <div class="stage">
      <!-- herdr W2: left agent rail — one row per active pane (env icon · status dot · label · limit
           chip); click focuses that pane. Same status dots as the pane headers. Collapsible. -->
      {#if !railOpen && spacePanes.length >= 2}
        <button type="button" class="rail-reopen" onclick={() => setRail(true)}
          title={t('sessions.railShow')} aria-label={t('sessions.railShow')}>›</button>
      {/if}
      {#if railOpen && spacePanes.length >= 2}
        <aside class="rail" aria-label={t('sessions.agents')}>
          <div class="rail-head">
            <span class="rail-title">{t('sessions.agents')}</span>
            <button type="button" class="rail-toggle" onclick={() => setRail(false)}
              title={t('sessions.railHide')} aria-label={t('sessions.railHide')}>‹</button>
          </div>
          {#each spacePanes as pane (pane.key)}
            {@const st = agentStates[sessionIds[pane.key]] ?? null}
            <button type="button" class="rail-item" class:active={activeKey === pane.key}
              onclick={() => railFocus(pane.key)}
              oncontextmenu={(e) => openRailMenu(e, pane.key)}
              onkeydown={(e) => {
                // Keyboard parity with right-click: Shift+F10 / the ContextMenu key open the row menu.
                if ((e.shiftKey && e.key === 'F10') || e.key === 'ContextMenu') {
                  e.preventDefault();
                  railMenuAnchor = e.currentTarget as HTMLElement;
                  railMenuFor = pane.key;
                }
              }}
              title={st && st !== 'unknown' ? t(`sessions.state_${st}`) : paneLabel(pane)}>
              <span class="env-ic">{@html envIcon(pane.tool)}</span>
              <span class="dot" class:working={st === 'working'} class:blocked={st === 'blocked'}
                class:done={st === 'done'} class:limited={st === 'limited'}></span>
              <span class="rail-label">{paneLabel(pane)}</span>
              {#if pane.tool === 'claude' && pane.profile}<ProfileUsageBadge profile={pane.profile} compact />{/if}
            </button>
          {/each}
        </aside>
      {/if}
      {#if railMenuFor && railMenuAnchor}
        <!-- Rail row context menu — reuses the launcher menu's look (.plusmenu/.pm-item). -->
        <div class="plusmenu" role="menu" tabindex="-1"
          use:anchored={{ anchor: railMenuAnchor, onOutside: () => (railMenuFor = null) }}
          onkeydown={(e) => e.key === 'Escape' && (railMenuFor = null)}>
          <button type="button" class="pm-item" role="menuitem"
            onclick={() => railMenuAct((k) => (maximized = maximized === k ? null : k))}>
            {maximized === railMenuFor ? t('sessions.restore') : t('sessions.maximize')}</button>
          <button type="button" class="pm-item" role="menuitem"
            onclick={() => railMenuAct(toggleBackground)}>{t('sessions.backgroundPane')}</button>
          <button type="button" class="pm-item" role="menuitem"
            onclick={() => railMenuAct(closePane)}>{t('sessions.closePane')}</button>
          {#if spaces.length > 1}
            {@const curSpace = panes.find((p) => p.key === railMenuFor)?.space ?? DEFAULT_SPACE}
            <div class="pm-hdr">{t('sessions.moveToSpace')}</div>
            {#each spaces.filter((s) => s.id !== curSpace) as sp (sp.id)}
              <button type="button" class="pm-item" role="menuitem"
                onclick={() => railMenuAct((k) => movePaneToSpace(k, sp.id))}>→ {sp.name}</button>
            {/each}
          {/if}
          {#if spacePanes.length > 1}
            <button type="button" class="pm-item" role="menuitem"
              onclick={() => railMenuAct(() => closeSpacePanes())}>{t('sessions.closeSpace')}</button>
          {/if}
        </div>
      {/if}
      <div
        class="grid"
        class:focus-dim={focusMode && !maximized}
        class:collapsed={spacePanes.length === 0}
        bind:this={gridEl}
      style="grid-template-columns: {maximized ? '1fr' : colFr.map((f) => `minmax(0, ${f}fr)`).join(' ')}; grid-template-rows: {maximized ? '1fr' : rowFr.map((f) => `minmax(80px, ${f}fr)`).join(' ')};"
    >
      <!-- Every pane stays MOUNTED (sessions must survive maximize); non-maximized ones are just
           hidden, so the maximized pane fills the single column. -->
      {#each activePanes as pane (pane.key)}
        <div class="cell" class:hidden={paneSpace(pane) !== activeSpace || (maximized != null && maximized !== pane.key)}
          class:active={activeKey === pane.key && !maximized && panes.length > 1}>
          <TerminalPane
            bind:this={paneRefs[pane.key]}
            profile={pane.profile}
            tool={pane.tool}
            args={pane.args}
            cwd={pane.cwd || undefined}
            remoteDir={pane.remoteDir}
            sshTarget={pane.sshTarget}
            attachId={pane.attachId}
            ownsSession={pane.ownsSession ?? false}
            paneKey={pane.key}
            agentState={agentStates[sessionIds[pane.key]] ?? null}
            visible={visible && (maximized == null || maximized === pane.key)}
            maximized={maximized === pane.key}
            {broadcast}
            onInput={broadcastInput}
            onUserInput={(k) => (lastUserInputAt[k] = Date.now())}
            {onIdChange}
            {onActivity}
            onFocus={(k) => (activeKey = k)}
            displayName={pane.name ?? ''}
            onRename={renamePane}
            onNewSession={launchPhrase}
            onClose={() => closePane(pane.key)}
            onToggleMax={() => toggleMax(pane.key)}
            onDuplicate={() => duplicate(pane.key)}
            onBackground={() => toggleBackground(pane.key)}
            {onDragStart}
            {onDragEnter}
            {onDrop}
          />
        </div>
      {/each}
      {#if !maximized}
        {#each colBounds as pos, k (k)}
          <button type="button" class="divider col-divider" style="left:{pos}%"
            title={t('sessions.resizeCol')} aria-label={t('sessions.resizeCol')}
            onpointerdown={(e) => startResize(e, k, 'col')}></button>
        {/each}
        {#each rowBounds as pos, k (k)}
          <button type="button" class="divider row-divider" style="top:{pos}%"
            title={t('sessions.resizeRow')} aria-label={t('sessions.resizeRow')}
            onpointerdown={(e) => startResize(e, k, 'row')}></button>
        {/each}
      {/if}
      </div>
      {#if spacePanes.length === 0}
        <div class="space-empty">
          <EmptyState icon={SquareTerminal} title={t('sessions.spaceEmptyTitle')}
            description={t('sessions.spaceEmptyHint')} action={() => (newOpen = true)}
            actionLabel={t('sessions.newSession')} />
        </div>
      {/if}
    </div>
  {/if}

  <!-- Background sessions section -->
  {#if bgPanes.length}
    <div class="bg-section">
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.backgroundSection', { n: bgPanes.length })}</span>
      {#each bgPanes as pane (pane.key)}
        <span class="bg-chip">
          <span class="bg-label">{paneLabel(pane)}</span>
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => toggleBackground(pane.key)}
            title={t('sessions.restoreBg')}>{t('sessions.restoreBg')}</button>
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => closePane(pane.key)}
            title={t('sessions.closePane')} aria-label={t('sessions.closePane')}>{t('sessions.closePane')}</button>
        </span>
      {/each}
    </div>
  {/if}

  <!-- Hidden mount for backgrounded panes (keep PTY alive) -->
  <div class="bg-hidden" aria-hidden="true">
    {#each bgPanes as pane (pane.key)}
      <TerminalPane
        profile={pane.profile}
        tool={pane.tool}
        args={pane.args}
        cwd={pane.cwd || undefined}
        remoteDir={pane.remoteDir}
        sshTarget={pane.sshTarget}
        attachId={pane.attachId}
        ownsSession={pane.ownsSession ?? false}
        paneKey={pane.key}
        agentState={agentStates[sessionIds[pane.key]] ?? null}
        visible={false}
        maximized={false}
        {broadcast}
        onInput={broadcastInput}
        {onIdChange}
        {onActivity}
        onFocus={(k) => (activeKey = k)}
        displayName={pane.name ?? ''}
        onRename={renamePane}
        onNewSession={launchPhrase}
        onClose={() => closePane(pane.key)}
      />
    {/each}
  </div>

  {#if !activePanes.length && !bgPanes.length}
    <EmptyState icon={SquareTerminal} title={t('sessions.emptyTitle')} description={t('sessions.emptyHint')}
      action={launchPhrase} actionLabel={t('sessions.phLaunch')} />
  {/if}
</div>

<style>
  .wrap {
    padding: var(--sw-space-4) var(--sw-space-6) var(--sw-space-3);
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  /* Broadcast armed: warn-coloured so "every keystroke goes to all panes" isn't an invisible state. */
  .broadcast-armed {
    color: var(--sw-status-warn);
    font-weight: 600;
  }
  /* Restore-last-session offer bar. */
  .restorebar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: var(--sw-space-2);
    padding: 6px 12px;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-subtle);
  }
  /* Agent-status rollup chips (header): blocked / working / done counts. */
  .status-sum {
    display: inline-flex;
    align-items: baseline;
    gap: 10px;
    font-size: var(--sw-text-xs);
    white-space: nowrap;
  }
  .ss-blocked {
    /* V7: text color from the .status-bad canon (light-aware); the strip stays bold. */
    font-weight: 600;
  }
  .ss-done {
    color: var(--sw-status-done);
  }
  /* Nudge strip: enable the Agent-statuses hook — shown only when it's off and a live local claude
     pane exists (which otherwise shows a neutral 'unknown' dot instead of working/idle). */
  .status-nudge {
    display: flex;
    align-items: center;
    gap: var(--sw-space-3);
    margin-bottom: var(--sw-space-4);
    padding: var(--sw-space-2) var(--sw-space-3);
    border: 1px solid var(--sw-border);
    border-left: 3px solid var(--sw-status-warn);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-subtle);
    font-size: var(--sw-text-xs);
  }
  .nudge-msg {
    flex: 1 1 auto;
    color: var(--sw-text-secondary);
  }
  .nudge-x {
    flex: 0 0 auto;
    padding: 0 6px;
    border: 0;
    background: transparent;
    color: var(--sw-text-muted);
    font-size: 15px;
    line-height: 1;
    cursor: pointer;
  }
  .nudge-x:hover {
    color: var(--sw-text-secondary);
  }
  .launcher {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: var(--sw-space-3);
    margin-bottom: var(--sw-space-4);
    padding-bottom: var(--sw-space-3);
    border-bottom: 1px solid var(--sw-border);
  }
  /* herdr W1: compact bar holding the "＋ New session" button + favorite chips. */
  .newbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    margin-bottom: var(--sw-space-4);
  }
  /* The launcher rendered as an anchored popover: an elevated card (use:anchored sets position:fixed
     + top/left inline), overriding the inline .launcher border-bottom/margins. */
  .launcher-pop {
    width: min(720px, 92vw);
    margin-bottom: 0;
    padding: var(--sw-space-3);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-secondary);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
    z-index: 60;
  }
  .launchhead {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
  }
  .envseg {
    display: inline-flex;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    overflow: hidden;
    margin-right: auto;
  }
  .env-btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 8px 16px;
    border: none;
    border-right: 1px solid var(--sw-border);
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-sm);
    font-weight: 500;
    cursor: pointer;
  }
  .env-btn:last-child {
    border-right: none;
  }
  .env-btn:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  .env-btn.sel {
    background: var(--sw-accent-glow);
    color: var(--sw-accent-text);
  }
  .env-ic {
    display: inline-flex;
    opacity: 0.9;
  }
  .phrase {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
    padding: 10px 12px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
  }
  .phrase .pw {
    color: var(--sw-text-muted);
    font-size: var(--sw-text-xs);
  }
  .phrase .psel {
    min-width: 140px;
  }
  .phrase .pfolder {
    min-width: 200px;
    flex: 1;
  }
  .phrase .pargs {
    min-width: 160px;
    flex: 1;
  }
  .phrase .pmodel {
    min-width: 150px;
    max-width: 220px;
  }
  .phrase .psel-wide {
    min-width: 200px;
  }
  .phrase .ph-note {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    opacity: 0.85;
    align-self: center;
  }
  .stackbar {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-2);
    padding: var(--sw-space-2) var(--sw-space-3);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-secondary);
    font-size: var(--sw-text-xs);
  }
  .stackbar .stk-label {
    font-weight: 500;
    margin-right: auto;
  }
  .phrase .ssh-hint {
    flex-basis: 100%;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    opacity: 0.85;
  }
  .phrase .star {
    margin-left: auto;
    color: var(--sw-warn);
    font-size: 15px;
    line-height: 1;
    padding: 6px 9px;
  }
  .favs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
  }
  /* Launcher C: split "＋" (main = instant per project recipe, chevron = memory menu) */
  .plus-split {
    display: inline-flex;
    border-radius: var(--sw-radius-md);
    overflow: hidden;
    box-shadow: 0 0 0 1px var(--sw-accent);
  }
  .split-main,
  .split-chev {
    border: none;
    cursor: pointer;
    background: var(--sw-accent-solid);
    color: #fff;
    font-size: var(--sw-text-xs);
    padding: 6px 12px;
  }
  .split-main {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
  }
  .split-main:disabled,
  .split-chev:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .split-main:hover:not(:disabled),
  .split-chev:hover:not(:disabled),
  .split-chev.active {
    filter: brightness(1.12);
  }
  .split-sub {
    font-weight: 400;
    opacity: 0.85;
    max-width: 300px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .split-chev {
    border-left: 1px solid rgb(255 255 255 / 0.3);
    padding: 6px 9px;
  }
  /* Memory menu (anchored popover) */
  .plusmenu {
    width: min(440px, 92vw);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-secondary);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
    z-index: 60;
    overflow: hidden;
    padding: var(--sw-space-1) 0;
  }
  .pm-hdr {
    padding: 5px 12px 3px;
    color: var(--sw-text-muted);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .pm-row {
    display: flex;
    align-items: stretch;
  }
  .pm-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--sw-text-primary);
    cursor: pointer;
    text-align: left;
    padding: 6px 12px;
    font-size: var(--sw-text-xs);
  }
  .pm-item:hover:not(:disabled) {
    background: var(--sw-bg-hover);
  }
  .pm-item:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .pm-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pm-star {
    color: var(--sw-warn);
    flex-shrink: 0;
  }
  .pm-path {
    margin-left: auto;
    color: var(--sw-text-muted);
    font-family: var(--sw-font-mono);
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 45%;
    flex-shrink: 0;
  }
  .pm-x {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    padding: 0 10px;
    font-size: 10px;
  }
  .pm-x:hover {
    color: var(--sw-danger);
  }
  .pm-custom {
    color: var(--sw-accent-text);
    font-weight: 600;
    border-top: 1px solid var(--sw-border);
    margin-top: 3px;
    padding-top: 8px;
  }
  .workspaces {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    margin-bottom: var(--sw-space-3);
  }
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--sw-space-2);
    padding: 10px 12px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
  }
  .set-row,
  .srv-add {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
  }
  .set-srv {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid var(--sw-border);
  }
  .set-k {
    flex-shrink: 0;
    width: 130px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
  }
  .srv-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .srv-chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 3px 4px 3px 9px;
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    font-size: var(--sw-text-xs);
  }
  .srv-n {
    color: var(--sw-text-primary);
  }
  .srv-t {
    color: var(--sw-text-muted);
    font-size: 11px;
  }
  .srv-cfg {
    color: var(--sw-text-muted);
    font-size: 10px;
    padding-right: 6px;
  }
  .srv-x {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    padding: 2px 6px;
    font-size: 10px;
    border-left: 1px solid var(--sw-border);
  }
  .srv-x:hover {
    color: var(--sw-danger);
  }
  .argchip {
    padding: var(--sw-space-1) var(--sw-space-2);
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    background: transparent;
    color: var(--sw-text-muted);
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: 11px;
    cursor: pointer;
    white-space: nowrap;
  }
  .argchip:hover {
    color: var(--sw-text-secondary);
  }
  .argchip.on {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
    border-color: var(--sw-accent-text);
  }
  .ws-chip {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    overflow: hidden;
  }
  .ws-go {
    border: none;
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    padding: var(--sw-space-1) var(--sw-space-2);
    font-size: var(--sw-text-xs);
  }
  .ws-go:hover {
    color: var(--sw-text-primary);
    background: var(--sw-accent-glow);
  }
  .ws-del {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    padding: 3px 6px;
    font-size: 10px;
    border-left: 1px solid var(--sw-border);
  }
  .ws-del:hover {
    color: var(--sw-danger);
  }
  .maxbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    margin-bottom: var(--sw-space-2);
  }
  .maxbar .spacer {
    flex: 1;
  }
  .maxchip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 220px;
    padding: var(--sw-space-1) var(--sw-space-2);
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-xs);
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .maxchip:hover {
    color: var(--sw-text-primary);
    background: var(--sw-accent-glow);
  }
  .maxchip.active {
    border-color: var(--sw-accent);
    color: var(--sw-accent-text);
    background: var(--sw-accent-glow);
  }
  .maxchip-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--sw-status-up);
    flex-shrink: 0;
  }
  /* Pane printed something while it was off-screen — draw attention. */
  .maxchip-dot.unread {
    background: var(--sw-warn);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--sw-warn) 30%, transparent);
  }
  .cell {
    min-height: 0;
    min-width: 0;
    display: flex;
  }
  .cell.hidden {
    display: none;
  }
  /* The pane holding keyboard focus gets an accent ring so "you are here" is visible across panes
     (esp. for Ctrl+]/[ cycling). border-radius matches the pane's own rounding. */
  .cell.active {
    outline: 2px solid var(--sw-accent);
    outline-offset: -1px;
    border-radius: var(--sw-radius-md);
  }
  /* Focus mode: dim every pane except the one under the cursor (for screencasts). */
  .grid.focus-dim .cell {
    transition: opacity 0.15s;
  }
  .grid.focus-dim .cell:not(:hover) {
    opacity: 0.3;
  }
  .divider {
    position: absolute;
    border: none;
    background: transparent;
    z-index: 4;
    padding: 0;
  }
  .divider::after {
    content: '';
    position: absolute;
    background: var(--sw-border);
    transition: background 0.12s;
  }
  .divider:hover::after {
    background: var(--sw-accent-text);
  }
  .col-divider {
    top: 0;
    bottom: 0;
    width: 10px;
    transform: translateX(-50%);
    cursor: col-resize;
  }
  .col-divider::after {
    left: 50%;
    top: 0;
    bottom: 0;
    width: 2px;
    transform: translateX(-50%);
  }
  .row-divider {
    left: 0;
    right: 0;
    height: 10px;
    transform: translateY(-50%);
    cursor: row-resize;
  }
  .row-divider::after {
    top: 50%;
    left: 0;
    right: 0;
    height: 2px;
    transform: translateY(-50%);
  }
  /* herdr W2: horizontal stage = collapsible agent rail + terminal grid. */
  .stage {
    display: flex;
    gap: var(--sw-space-3);
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
  .rail {
    flex-shrink: 0;
    width: 190px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    overflow-y: auto;
    padding-right: var(--sw-space-1);
    border-right: 1px solid var(--sw-border);
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 4px 4px;
  }
  .rail-title {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .rail-toggle,
  .rail-reopen {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    padding: 2px 6px;
    border-radius: var(--sw-radius-sm);
  }
  .rail-toggle:hover,
  .rail-reopen:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  .rail-reopen {
    flex-shrink: 0;
    align-self: flex-start;
    border: 1px solid var(--sw-border);
  }
  .rail-item {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: var(--sw-space-1) var(--sw-space-2);
    border: 1px solid transparent;
    border-radius: var(--sw-radius-sm);
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    text-align: left;
    width: 100%;
    overflow: hidden;
  }
  .rail-item:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  .rail-item.active {
    background: var(--sw-accent-glow);
    border-color: var(--sw-accent-text);
    color: var(--sw-accent-text);
  }
  .rail-item .env-ic {
    display: inline-flex;
    opacity: 0.85;
    flex-shrink: 0;
  }
  .rail-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--sw-text-xs);
  }
  /* Agent status dots — mirror TerminalPane (shared tokens + global sw-dot-pulse keyframe). */
  .rail-item .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--sw-status-up);
    flex-shrink: 0;
  }
  .rail-item .dot.working {
    background: var(--sw-status-warn);
    animation: sw-dot-pulse 1.1s ease-in-out infinite;
  }
  .rail-item .dot.blocked {
    background: var(--sw-danger, #f85149);
    animation: sw-dot-pulse 0.7s ease-in-out infinite;
  }
  .rail-item .dot.done {
    background: var(--sw-status-done);
  }
  .rail-item .dot.limited {
    background: var(--sw-status-down, #ef4444);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--sw-status-down, #ef4444) 35%, transparent);
  }
  /* herdr W3: project space tabs. */
  .spaces {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-1);
    margin-bottom: var(--sw-space-3);
  }
  .space-tab {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    overflow: hidden;
    background: var(--sw-bg-secondary);
  }
  .space-tab.active {
    border-color: var(--sw-accent-text);
    background: var(--sw-accent-glow);
  }
  .space-go {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    padding: var(--sw-space-1) var(--sw-space-2);
    font-size: var(--sw-text-xs);
    white-space: nowrap;
  }
  .space-tab.active .space-go {
    color: var(--sw-accent-text);
    font-weight: 600;
  }
  .space-go:hover {
    color: var(--sw-text-primary);
  }
  .space-count {
    min-width: 16px;
    text-align: center;
    padding: 0 5px;
    border-radius: 9999px;
    background: var(--sw-bg-hover);
    color: var(--sw-text-muted);
    font-size: 10px;
  }
  .space-go .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--sw-status-up);
    flex-shrink: 0;
  }
  .space-go .dot.working {
    background: var(--sw-status-warn);
    animation: sw-dot-pulse 1.1s ease-in-out infinite;
  }
  .space-go .dot.blocked {
    background: var(--sw-danger, #f85149);
    animation: sw-dot-pulse 0.7s ease-in-out infinite;
  }
  .space-go .dot.done {
    background: var(--sw-status-done);
  }
  .space-x {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    padding: 3px 7px;
    font-size: 10px;
    border-left: 1px solid var(--sw-border);
  }
  .space-x:hover {
    color: var(--sw-danger);
  }
  /* V2: per-project "＋ agent here" — accented so it reads as the primary action on the tab. */
  .space-plus {
    border: none;
    border-left: 1px solid var(--sw-border);
    background: transparent;
    color: var(--sw-accent-text);
    cursor: pointer;
    padding: 4px 9px;
    font-size: 13px;
    font-weight: 700;
    line-height: 1;
  }
  .space-plus:hover {
    background: var(--sw-accent-glow);
  }
  /* ＋/✕ appear on hover / on the active tab / on keyboard focus — with 5+ projects the
     unconditional pair on every tab was pure noise. Opacity (not display) keeps tab widths stable. */
  .space-tab .space-plus,
  .space-tab .space-x {
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .space-tab:hover .space-plus,
  .space-tab:hover .space-x,
  .space-tab.active .space-plus,
  .space-tab.active .space-x,
  .space-tab:focus-within .space-plus,
  .space-tab:focus-within .space-x {
    opacity: 1;
  }
  /* ⌨ cheatsheet rows */
  .hk-list {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--sw-space-2) var(--sw-space-4);
    align-items: baseline;
  }
  .hk-combo {
    font-family: var(--sw-font-mono, monospace);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-primary);
    background: var(--sw-bg-tertiary, var(--sw-bg-hover));
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm);
    padding: 2px 6px;
    white-space: nowrap;
    justify-self: start;
  }
  .space-add {
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    padding: var(--sw-space-1) var(--sw-space-2);
    font-size: var(--sw-text-sm);
    line-height: 1;
  }
  .space-add:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  /* Inline rename: sized to sit INSIDE the tab without growing its height (was jumping the whole row). */
  .space-edit {
    width: 120px;
    margin: 0;
    padding: 2px 6px;
    font-size: var(--sw-text-xs);
    box-sizing: border-box;
  }
  .space-empty {
    flex: 1;
    display: grid;
    place-items: center;
    min-height: 0;
  }
  .grid.collapsed {
    flex: 0 0 auto;
    min-height: 0;
    height: 0;
    padding-bottom: 0;
  }
  .grid {
    position: relative;
    display: grid;
    gap: var(--sw-space-3);
    flex: 1;
    min-height: 0;
    /* Explicit equal grid-template-rows are set inline (equal default + resizable). This is just a
       fallback for any unexpected implicit row. */
    grid-auto-rows: minmax(80px, 1fr);
    overflow: hidden;
    padding-bottom: var(--sw-space-2);
  }
  .active {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
  }
  .bg-section {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-2);
    padding: var(--sw-space-2) var(--sw-space-3);
    background: var(--sw-bg-secondary);
    border: 1px dashed var(--sw-border);
    border-radius: var(--sw-radius-md);
  }
  .bg-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px 2px 10px;
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    font-size: var(--sw-text-xs);
  }
  .bg-label {
    color: var(--sw-text-muted);
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bg-hidden {
    display: none;
  }
</style>
