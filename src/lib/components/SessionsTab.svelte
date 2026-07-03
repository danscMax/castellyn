<script lang="ts">
  import { onMount } from 'svelte';
  import TerminalPane from './TerminalPane.svelte';
  import FolderField from './FolderField.svelte';
  import Toggle from './Toggle.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import EmptyState from './EmptyState.svelte';
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
    globalSessionCount,
    agentStatusHookStatus,
    agentStatusHookSet,
    readConfig,
    writeConfig,
    type AgentStatusHookState,
    type AgentStatusEvent
  } from '$lib/ipc';
  import { agentSummary, type AgentPaneState } from '$lib/agentStatus.svelte';
  import { getMonitors, invalidateMonitors, openDetached } from '$lib/monitors';
  import Select from './Select.svelte';
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
    visible = true,
    folderReq = null,
    confirmDestructive = true,
    onFolderReqConsumed
  }: {
    profiles?: string[];
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
  };
  function renamePane(key: string, name: string) {
    panes = panes.map((p) => (p.key === key ? { ...p, name: name || undefined } : p));
  }
  // The key (not the profile) identifies a pane, so the same profile can run in several at once.
  let panes = $state<Pane[]>([]);
  let seq = 0;
  let columns = $state(2);
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
  let lastFolders = $state<Record<string, string>>({});
  // Default launch args, seeded into the phrase's args field for Claude/opencode.
  let defaultArgs = $state('');
  // Collapsible launcher settings (default args, projects root) — collapsed by default.
  let launcherOpen = $state(false);
  // A workspace is a named set of session configs you can re-launch with one click.
  type WsConfig = { tool: SessionTool; profile: string; cwd: string; args: string; remoteDir?: string; sshTarget?: string };
  let workspaces = $state<Record<string, WsConfig[]>>({});
  // Lifecycle: this tab unmounts/remounts on every tab switch, so event listeners MUST be torn down
  // (else they pile up and a returned pane gets added N times). `mounted` also gates async state
  // writes (checkReach) that may resolve after unmount.
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
      favorites = JSON.parse(localStorage.getItem(VKEY) ?? '[]');
      projectsRoot = localStorage.getItem(ROOT) ?? '';
      remoteRecent = JSON.parse(localStorage.getItem(RRKEY) ?? '[]');
      const c = Number(localStorage.getItem(CKEY));
      if (c >= 1 && c <= 3) columns = c;
      const fz = Number(localStorage.getItem('cmh-sessions-fontsize'));
      if (fz >= 8 && fz <= 28) globalFont = fz;
      launcherOpen = localStorage.getItem('cmh-sessions-launcher') === '1';
    } catch {
      /* first run / private mode */
    }
    // Re-attach sessions that survived a webview reload (#5): the backend keeps them running, so
    // mirror the still-alive ones back here as owner instead of orphaning them against SESSION_LIMIT.
    if (savedLive.length) {
      void (async () => {
        try {
          const alive = new Set(await sessionList());
          for (const s of savedLive) {
            if (alive.has(s.id)) {
              addPane({ tool: s.tool, profile: s.profile, cwd: s.cwd, args: s.args, remoteDir: s.remoteDir, sshTarget: s.sshTarget, attachId: s.id, ownsSession: true, name: s.name });
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
    // Offer to restore the last monitor arrangement (the user's "restore on launch" choice). A toast
    // with a one-click action — non-aggressive: we don't auto-spawn a grid of terminals every start.
    try {
      const saved = localStorage.getItem(MLKEY);
      savedLayoutExists = !!(saved && saved !== '{}');
      if (savedLayoutExists) {
        let detail = '';
        try {
          detail = layoutSummary(JSON.parse(saved!));
        } catch {
          /* ignore — show the prompt without the spec list */
        }
        pushToast({
          kind: 'info',
          title: t('sessions.restoreLayoutPrompt'),
          detail,
          action: { label: t('sessions.restoreLayoutAction'), onClick: restoreLayout }
        });
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
        state === 'idle' && (prev === 'working' || prev === 'blocked') && !focused
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
  // Roll the counts up for the header chips + the sidebar badge (+page reads the store).
  const statusCounts = $derived.by(() => {
    const c = { blocked: 0, working: 0, done: 0 };
    for (const id of Object.values(sessionIds)) {
      const s = agentStates[id];
      if (s === 'blocked') c.blocked++;
      else if (s === 'working') c.working++;
      else if (s === 'done') c.done++;
    }
    return c;
  });
  $effect(() => {
    agentSummary.blocked = statusCounts.blocked;
    agentSummary.working = statusCounts.working;
    agentSummary.done = statusCounts.done;
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
  // Sound / OS-toast preferences for status transitions (the backend reads them from config).
  let statusSounds = $state(true);
  let statusNotify = $state(true);
  onMount(async () => {
    try {
      const c = await readConfig();
      statusSounds = c.statusSounds ?? true;
      statusNotify = c.statusNotify ?? true;
    } catch {
      /* defaults stand */
    }
  });
  async function saveStatusPrefs() {
    try {
      // Read-patch-write: writeConfig persists the WHOLE config, so never send a partial.
      const c = await readConfig();
      await writeConfig({ ...c, statusSounds, statusNotify });
    } catch (e) {
      pushToast({ kind: 'error', title: String(e) });
    }
  }

  // Broadcast: mirror keystrokes from any pane to every running session.
  let broadcast = $state(false);
  // $state (not a plain object) so the persist effect below reacts when a pane's id arrives/clears.
  let sessionIds = $state<Record<string, string>>({});
  function onIdChange(key: string, id: string | null) {
    if (id) sessionIds = { ...sessionIds, [key]: id };
    else {
      const { [key]: _drop, ...rest } = sessionIds;
      sessionIds = rest;
    }
    refreshGlobalCount(); // F16: a spawn/exit here moved the global tally — re-read it
  }
  // ── Reload survival (#5): persist spawned-here sessions, re-attach the ones still alive on mount ──
  const LIVE_KEY = 'cmh-sessions-live';
  type LivePane = { tool: SessionTool; profile: string; cwd: string; args: string; remoteDir?: string; sshTarget?: string; id: string; claudeSid?: string; name?: string };
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
        .map((p) => ({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args, remoteDir: p.remoteDir, sshTarget: p.sshTarget, id: sessionIds[p.key], claudeSid: claudeSids[sessionIds[p.key]], name: p.name }));
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
  function broadcastInput(data: string) {
    for (const id of Object.values(sessionIds)) sessionWrite(id, data);
  }
  // One-shot: send a typed command (+Enter) to EVERY running session, without enabling
  // continuous broadcast.
  let sendAllText = $state('');
  // Send-to-all fires a command (+Enter) into EVERY live session at once — including SSH/remote panes.
  // That's the most destructive surface in the app, so gate it behind the canonical confirm dialog
  // (project rule: destructive actions confirm first) showing the exact command + how many panes.
  let confirmSend = $state<{ cmd: string; targets: string[] } | null>(null);
  function sendToAll() {
    const cmd = sendAllText.trim();
    if (!cmd) return;
    // F15: list the exact panes the command lands in (tool@profile · cwd/host) — count alone hid
    // which sessions get hit, so the user couldn't catch a stray SSH pane before sending.
    const targets = panes
      .filter((p) => sessionIds[p.key])
      .map((p) => {
        const where = p.sshTarget ? `🖥 ${p.sshTarget}` : p.cwd || '~';
        return p.tool === 'claude' ? `${p.tool}@${p.profile} · ${where}` : `${p.tool} · ${where}`;
      });
    confirmSend = { cmd, targets };
  }
  function doSendToAll() {
    if (!confirmSend) return;
    for (const id of Object.values(sessionIds)) sessionWrite(id, confirmSend.cmd + '\r');
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
    const layout: Record<number, DetachPane[]> = {};
    for (const [idx, list] of byMon) {
      const ok = await openDetached(`mon-${idx}`, idx, list.map((e) => e.dp));
      if (!ok) continue; // monitor/window unavailable — leave those panes in the main grid
      for (const e of list) {
        if (e.dp.sessionId) markMoved(e.dp.sessionId);
        removeKeys.add(e.key);
      }
      // Persist the LAUNCH config (no live session id) so this monitor can be restored next launch.
      layout[idx] = list.map((e) => ({
        title: e.dp.title,
        tool: e.dp.tool,
        profile: e.dp.profile,
        cwd: e.dp.cwd,
        args: e.dp.args,
        owns: true
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
    let saved: Record<number, DetachPane[]>;
    try {
      saved = JSON.parse(localStorage.getItem(MLKEY) ?? '{}');
    } catch {
      return;
    }
    invalidateMonitors(); // re-enumerate: the saved layout may target monitors that are now gone
    let mons;
    try {
      mons = await getMonitors();
    } catch {
      return;
    }
    const have = new Set(mons.map((m) => m.index));
    for (const [idxStr, list] of Object.entries(saved)) {
      const idx = Number(idxStr);
      if (!have.has(idx) || !list?.length) continue;
      await openDetached(`mon-${idx}`, idx, list);
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
  function addPane(v: { tool: SessionTool; profile: string; cwd: string; args: string; remoteDir?: string; sshTarget?: string; attachId?: string; ownsSession?: boolean; name?: string }) {
    // Don't block re-attaching an EXISTING session (e.g. a pane returned from a monitor) on the cap —
    // it's not a new spawn. Only new spawns count against MAX_PANES.
    if (atLimit && !v.attachId) return;
    const key = `${v.tool}:${v.profile || 'sh'}#${seq++}`;
    panes = [...panes, { key, profile: v.profile, tool: v.tool, cwd: v.cwd, args: v.args, remoteDir: v.remoteDir, sshTarget: v.sshTarget, attachId: v.attachId, ownsSession: v.ownsSession, name: v.name }];
    if (v.tool === 'claude') rememberFolder(v.profile, v.cwd);
    rememberRecent(v.cwd);
    // Auto-focus the new pane's terminal so the user can type immediately (the obvious next action
    // after launch) — one frame later, once the pane has mounted and grabbed its paneRef.
    requestAnimationFrame(() => paneRefs[key]?.focusTerminal());
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
  // Never show more columns than there are panes — 1 pane with "3 columns" selected should fill
  // the row, not sit in a third of it.
  const effCols = $derived(Math.min(columns, Math.max(1, activePanes.length)));
  const rowCount = $derived(Math.max(1, Math.ceil(activePanes.length / effCols)));
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

  // ─── Settings (⚙): projects root + default args + SSH servers — all in one place, no dialogs ───
  const ROOT = 'cmh-projects-root';
  let projectsRoot = $state('');
  function openSettings() {
    launcherOpen = true;
  }
  async function browseRoot() {
    const d = await pickFolder(projectsRoot);
    if (d) {
      projectsRoot = d;
      try {
        localStorage.setItem(ROOT, d);
      } catch {
        /* ignore */
      }
    }
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
    if (!argsTouched) lArgs = defaultArgs;
  });
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
      icon: sshReach[h.id] === 'ok' ? '🟢' : sshReach[h.id] === 'fail' ? '🔴' : '⚪',
      hint: h.source === 'sshconfig' ? '~/.ssh/config' : undefined
    })),
    { value: LOC_ADD, label: t('sessions.locAdd'), icon: '＋' }
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
    if (!h) return null;
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
    const v = paneFrom(lEnv, lProfile, lLoc, lFolder, lRemoteDir, lArgs);
    if (v) {
      if (lLoc && lRemoteDir.trim()) rememberRemote(lRemoteDir); // SSH: keep the remote dir for next time
      addPane(v);
    }
  }
  // ─── Favorites: pin the whole phrase → 1-click relaunch ───
  type Fav = { id: string; env: Env; profile: string; locId: string; folder: string; remoteDir: string; args: string; label: string };
  const VKEY = 'cmh-sessions-favorites';
  let favorites = $state<Fav[]>([]);
  function favLabel(env: Env, profile: string, locId: string, folder: string): string {
    const h = locId ? sshHostList.find((x) => x.id === locId) : null;
    const where = h
      ? `🖥 ${h.name}`
      : folder
        ? folder.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || folder
        : t('sessions.cwdShort');
    return env === 'claude' ? `${env}·${profile} · ${where}` : `${env} · ${where}`;
  }
  function pinCurrent() {
    const id = `f${Date.now()}${Math.round(Math.random() * 1e4)}`;
    const label = favLabel(lEnv, lProfile, lLoc, lFolder);
    favorites = [
      ...favorites,
      { id, env: lEnv, profile: lProfile, locId: lLoc, folder: lFolder, remoteDir: lRemoteDir, args: lEnv === 'shell' ? '' : lArgs, label }
    ];
    pushToast({ kind: 'success', title: t('sessions.pinned', { label }) }); // feedback — pinning was silent (#17)
  }
  function launchFav(f: Fav) {
    const v = paneFrom(f.env, f.profile, f.locId, f.folder, f.remoteDir, f.args);
    if (v) addPane(v);
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
    if (p) addPane({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args, remoteDir: p.remoteDir, sshTarget: p.sshTarget });
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
  // Ctrl+T is left to the focused shell), Alt+1/2/3 cols, Ctrl+]/[ cycle panes.
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
    const list = maximized ? panes.filter((p) => p.key === maximized) : panes;
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
    } else if (e.altKey && (e.key === '1' || e.key === '2' || e.key === '3')) {
      e.preventDefault();
      columns = Number(e.key);
    } else if (e.ctrlKey && (e.key === ']' || e.key === '[')) {
      // Ctrl+] / Ctrl+[ — focus next / previous pane terminal.
      e.preventDefault();
      cycleFocus(e.key === ']' ? 1 : -1);
    }
  }

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
      addPane({ tool: s.tool, profile: s.profile, cwd: s.cwd, args, remoteDir: s.remoteDir, sshTarget: s.sshTarget, name: s.name });
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
  message={t('sessions.sendAllConfirmMsg', { count: Object.keys(sessionIds).length })}
  details={confirmSend ? [confirmSend.cmd, ...confirmSend.targets] : []}
  confirmLabel={t('sessions.sendAllConfirmOk')}
  danger
  onConfirm={doSendToAll}
  onCancel={() => (confirmSend = null)}
/>

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
          {#if statusCounts.blocked}<span class="ss ss-blocked">● {t('sessions.sumBlocked', { n: statusCounts.blocked })}</span>{/if}
          {#if statusCounts.working}<span class="ss ss-working">● {t('sessions.sumWorking', { n: statusCounts.working })}</span>{/if}
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
          <Toggle bind:checked={broadcast} />
          <span class="text-sw-xs" class:broadcast-armed={broadcast} class:text-sw-text-secondary={!broadcast}
            >{broadcast ? t('sessions.broadcastArmed', { count: panes.length }) : t('sessions.broadcast')}</span>
        </label>
        <span class="text-sw-text-muted">·</span>
      {/if}
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.layout')}</span>
      {#each [1, 2, 3] as c (c)}
        <button class="sw-btn sw-btn-ghost text-sw-xs" class:active={columns === c} onclick={() => (columns = c)}
          title="{t('sessions.layoutCols', { n: c })} · Alt+{c}">{c}</button>
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
            ...(savedLayoutExists ? [{ label: `↺ ${t('sessions.forgetLayout')}`, onClick: forgetLayout }] : [])
          ]}
        />
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={closeAll} title={t('sessions.closeAllTip')}>
          {t('sessions.closeAll')}
        </button>
      {/if}
    </div>
  </header>

  <!-- Launcher: environment × location × folder × args, read as a phrase (№20 + №8) -->
  <div class="launcher">
    <div class="launchhead">
      <div class="envseg" role="tablist" aria-label={t('sessions.dlgTool')}>
        {#each ENVS as e (e.id)}
          <button type="button" class="env-btn" class:sel={lEnv === e.id} onclick={() => (lEnv = e.id)}
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
      {/if}
      {#if lLoc && lEnv !== 'shell'}
        <span class="ssh-hint" title={t('sessions.sshToolHint', { tool: lEnv })}>{t('sessions.sshToolHint', { tool: lEnv })}</span>
      {/if}
      <button type="button" class="sw-btn sw-btn-ghost star" onclick={pinCurrent} title={t('sessions.pin')} aria-label={t('sessions.pin')}>★</button>
      <button type="button" class="sw-btn sw-btn-primary text-sw-xs" onclick={launchPhrase} disabled={atLimit} title="{t('sessions.phLaunch')} · Ctrl+Shift+T">▶ {t('sessions.phLaunch')}</button>
    </div>

    <!-- Favorites (pinned phrases) + save-workspace -->
    {#if favorites.length || panes.length || savingWs}
      <div class="favs">
        {#if favorites.length}
          <span class="text-sw-xs text-sw-text-muted">★</span>
          {#each favorites as f (f.id)}
            <span class="fav-chip">
              <button type="button" class="fav-go" onclick={() => launchFav(f)} title={t('sessions.favLaunchTip')}>{f.label}</button>
              <button type="button" class="fav-x" onclick={() => askRemoveFav(f)} title={t('common.delete')} aria-label={t('common.delete')}>✕</button>
            </span>
          {/each}
          <span class="text-sw-text-muted">·</span>
        {/if}
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

    <!-- Settings (⚙): projects root + default args + SSH servers — everything configurable, no dialogs -->
    {#if launcherOpen}
      <div class="settings">
        <div class="set-row">
          <span class="set-k" title={t('sessions.projectsRootHint')}>{t('sessions.projectsRoot')}</span>
          <input class="sw-input grow font-mono text-sw-xs" bind:value={projectsRoot}
            placeholder={t('sessions.projectsRootPlaceholder')} spellcheck="false" autocomplete="off"
            onchange={() => { try { localStorage.setItem(ROOT, projectsRoot); } catch { /* ignore */ } }} />
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={browseRoot}>📁 {t('sessions.browse')}</button>
        </div>
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
            <Toggle bind:checked={statusSounds} onCheckedChange={saveStatusPrefs} />
            {t('sessions.statusSound')}
          </label>
          <label class="flex cursor-pointer items-center gap-1 text-sw-xs text-sw-text-secondary" title={t('sessions.statusToastHint')}>
            <Toggle bind:checked={statusNotify} onCheckedChange={saveStatusPrefs} />
            {t('sessions.statusToast')}
          </label>
        </div>
        <div class="set-srv">
          <span class="set-k">{t('sessions.servers')}</span>
          <div class="srv-list">
            {#each sshHostList as h (h.id)}
              <span class="srv-chip">
                <span>{sshReach[h.id] === 'ok' ? '🟢' : sshReach[h.id] === 'fail' ? '🔴' : '⚪'}</span>
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
            {#if srvTest === 'fail'}<span class="text-sw-xs" style="color:var(--sw-danger)">✕ {t('sessions.dlgSshTestFail')}</span>{/if}
            <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!srvTarget.trim()} onclick={addServer}>{t('sessions.serverAdd')}</button>
          </div>
        </div>
      </div>
    {/if}
  </div>

  {#if globalCount >= SESSION_LIMIT}
    <p class="mb-sw-2 text-sw-xs" style="color:var(--sw-warn)">{t('sessions.globalLimitNote', { n: SESSION_LIMIT })}</p>
  {:else if panes.length >= MAX_PANES}
    <p class="mb-sw-2 text-sw-xs" style="color:var(--sw-warn)">{t('sessions.limitNote', { n: MAX_PANES })}</p>
  {:else if globalCount >= SESSION_LIMIT - 4}
    <p class="mb-sw-2 text-sw-xs" style="color:var(--sw-text-muted)">{t('sessions.globalNearNote', { used: globalCount, max: SESSION_LIMIT })}</p>
  {/if}

  <!-- Restore the previous run's session set (claude panes resume their conversation) -->
  {#if restorable.length}
    <div class="restorebar">
      <span class="text-sw-xs">{t('sessions.restoreOffer', { n: restorable.length })}</span>
      <button class="sw-btn sw-btn-primary text-sw-xs" onclick={restoreLast}>{t('sessions.restoreDo')}</button>
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (restorable = [])}>{t('sessions.restoreDismiss')}</button>
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

  <!-- While one pane is maximized the others are hidden; this switcher keeps them visible and
       one-click reachable so you never lose track of running sessions. -->
  {#if maximized}
    <div class="maxbar">
      {#each activePanes as p (p.key)}
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
    <div
      class="grid"
      class:focus-dim={focusMode && !maximized}
      bind:this={gridEl}
      style="grid-template-columns: {maximized ? '1fr' : colFr.map((f) => `minmax(0, ${f}fr)`).join(' ')}; grid-template-rows: {maximized ? '1fr' : rowFr.map((f) => `minmax(80px, ${f}fr)`).join(' ')};"
    >
      <!-- Every pane stays MOUNTED (sessions must survive maximize); non-maximized ones are just
           hidden, so the maximized pane fills the single column. -->
      {#each activePanes as pane (pane.key)}
        <div class="cell" class:hidden={maximized != null && maximized !== pane.key}
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
    color: var(--sw-danger);
    font-weight: 600;
  }
  .ss-working {
    color: var(--sw-status-warn);
  }
  .ss-done {
    color: var(--sw-status-done);
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
  .fav-chip {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--sw-accent-text);
    background: var(--sw-accent-glow);
    border-radius: 9999px;
    overflow: hidden;
  }
  .fav-go {
    border: none;
    background: transparent;
    color: var(--sw-text-primary);
    cursor: pointer;
    padding: var(--sw-space-1) var(--sw-space-2);
    font-size: var(--sw-text-xs);
    white-space: nowrap;
  }
  .fav-go:hover {
    color: var(--sw-accent-text);
  }
  .fav-x {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    padding: 3px 7px;
    font-size: 10px;
    border-left: 1px solid var(--sw-border);
  }
  .fav-x:hover {
    color: var(--sw-danger);
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
