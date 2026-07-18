<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal, type ILink } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import { Unicode11Addon } from '@xterm/addon-unicode11';
  import { SerializeAddon } from '@xterm/addon-serialize';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { Channel } from '@tauri-apps/api/core';
  import '@xterm/xterm/css/xterm.css';
  import {
    sessionSpawn,
    sessionAttach,
    sessionWrite,
    sessionResize,
    sessionKill,
    sessionDetach,
    openUrl,
    openPath,
    openInEditor,
    type SessionTool,
    type MonitorInfo
  } from '$lib/ipc';
  import { getMonitors, openDetached } from '$lib/monitors';
  import { downloadBlob } from '$lib/download';
  import DropdownMenu from './DropdownMenu.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  // V9: one icon language (lucide) across the pane toolbar — replaces the emoji/glyph zoo.
  import {
    FolderOpen, RotateCw, Search,
    Maximize2, Minimize2, X, ChevronUp, ChevronDown
  } from '@lucide/svelte';
  import { markMoved, consumeMoved } from '$lib/sessionMove';
  import { MSG_SNIPPETS } from '$lib/sessionPresets';
  import { t } from '$lib/i18n';
  import ProfileUsageBadge from './ProfileUsageBadge.svelte';
  import { copyText, pasteText } from '$lib/clipboard';
  import { pushToast } from '$lib/toast.svelte';
  import { saveScrollback, takeScrollback } from '$lib/scrollbackStore';

  let {
    profile,
    tool = 'claude',
    args = '',
    cwd = undefined,
    remoteDir = undefined,
    sshTarget = undefined,
    attachId = undefined,
    ownsSession = false,
    paneKey = '',
    agentState = null,
    visible = true,
    maximized = false,
    broadcast = false,
    onClose,
    onReturnToMain,
    onToggleMax,
    onBackground,
    onDuplicate,
    onDragStart,
    onDragEnter,
    onDrop,
    onInput,
    onUserInput,
    onIdChange,
    onNewSession,
    onActivity,
    onFocus,
    autoResumeLabel = null,
    displayName = '',
    onRename,
    showUsage = true
  }: {
    profile: string;
    tool?: SessionTool;
    args?: string;
    cwd?: string;
    remoteDir?: string;
    sshTarget?: string;
    attachId?: string;
    ownsSession?: boolean;
    paneKey?: string;
    /** Semantic agent state from SessionsTab (working|blocked|done|idle|unknown) or null. */
    agentState?: string | null;
    /** #21f: "Castellyn will auto-resume this pane" text (e.g. "auto-resume at 11:00"), or null. */
    autoResumeLabel?: string | null;
    displayName?: string;
    onRename?: (key: string, name: string) => void;
    /** Redesign 2026-07: hide the header usage badge when the rail already shows it (dedup). */
    showUsage?: boolean;
    visible?: boolean;
    maximized?: boolean;
    broadcast?: boolean;
    onClose: () => void;
    onReturnToMain?: () => void;
    onToggleMax?: () => void;
    onBackground?: () => void;
    onDuplicate?: () => void;
    onDragStart?: (key: string) => void;
    onDragEnter?: (key: string) => void;
    onDrop?: () => void;
    onInput?: (data: string) => void;
    /** Fires on real user keystrokes/paste in this pane — lets the tab defer auto-continue. */
    onUserInput?: (key: string) => void;
    onIdChange?: (key: string, id: string | null) => void;
    onNewSession?: () => void;
    onActivity?: (key: string) => void;
    onFocus?: (key: string) => void;
  } = $props();

  // Inline pane rename (double-click the title). Empty name → falls back to the derived label.
  let renaming = $state(false);
  let editName = $state('');
  let renameInput: HTMLInputElement | undefined = $state();
  function startRename() {
    if (!onRename) return;
    editName = displayName;
    renaming = true;
    queueMicrotask(() => renameInput?.focus());
  }
  function commitRename() {
    if (!renaming) return;
    renaming = false;
    onRename?.(paneKey, editName.trim());
  }

  // Pane title reflects environment × location: "env · 🖥 host" over SSH, else profile (claude),
  // the legacy ssh target, or the folder (opencode/shell).
  const folderName = $derived(cwd ? cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd : '');
  const locHost = $derived(sshTarget ? sshTarget.trim().split(/\s+/)[0] : '');
  const isSsh = $derived(!!sshTarget || tool === 'ssh'); // runs over SSH (new model or legacy tool)
  const label = $derived(
    locHost
      ? `${tool} · 🖥 ${locHost}`
      : tool === 'ssh'
        ? `ssh · ${args.trim().split(/\s+/)[0] || 'ssh'}`
        : tool === 'claude'
          ? `claude · ${profile}`
          : folderName
            ? `${tool} · ${folderName}`
            : tool
  );
  // Full hover detail: tool/profile + folder + launch args.
  const fullTitle = $derived(
    [label, cwd, args].filter(Boolean).join(' · ') || t('sessions.paneTitle', { profile: label })
  );

  // Stable-across-restart identity for the persisted scrollback (W2). paneKey (`tool:profile#seq`)
  // is NOT stable — seq resets every app start — but the LAUNCH RECIPE is: layout-restore recreates
  // a pane from exactly this tuple (tool/profile/cwd/target/args). The one volatile bit is the
  // `--resume <sid>` restore appends to a claude pane's args, so strip it to match the original save.
  const scrollbackId = $derived(
    [
      tool,
      profile,
      sshTarget ?? '',
      cwd ?? '',
      args.replace(/--resume\s+[\w-]+/g, '').replace(/\s+/g, ' ').trim()
    ].join('|')
  );

  // Multi-monitor: a non-attached pane can be opened (mirrored) on another monitor. The session keeps
  // running in this window; the monitor window attaches to it (fan-out) and replays scrollback.
  let monitors = $state<MonitorInfo[]>([]);
  $effect(() => {
    if (!attachId && monitors.length === 0) {
      getMonitors().then((m) => (monitors = m)); // shared cache → one enumeration across all panes
    }
  });
  async function sendToMonitor(idx: number) {
    if (!id) return;
    const ok = await openDetached(`pane-${id}`, idx, [
      { sessionId: id, title: displayName || label, tool, profile, cwd, args, owns: true }
    ]);
    if (ok) {
      markMoved(id); // this pane leaves the main grid; its unmount must not kill the live session
      onClose?.();
    }
  }
  const monItems = $derived(
    monitors.map((m) => {
      // Prefer a friendly monitor name; Windows often returns device paths (\\.\DISPLAY1) — fall back.
      const friendly = m.name && !m.name.startsWith('\\\\') ? m.name : `${t('sessions.toMonitor')} ${m.index + 1}`;
      return {
        label: `${friendly}${m.primary ? ' ★' : ''} · ${m.width}×${m.height}`,
        onClick: () => sendToMonitor(m.index)
      };
    })
  );

  let host: HTMLDivElement;
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let search: SearchAddon | undefined;
  // W2: serialize the buffer to ANSI for cold-restore. Saved on a trailing debounce after output and
  // on dispose; replayed as inert scrollback before the fresh PTY attaches (see onMount / onDestroy).
  let serialize: SerializeAddon | undefined;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function saveScrollbackNow() {
    // Attached mirrors don't own persisted scrollback — the backend replays it on attach, and the
    // owning window is the one that persists. Only a self-spawned pane writes here.
    if (attachId || !term || !serialize) return;
    try {
      // excludeAltBuffer: a dead full-screen TUI's alt-screen is gone on restart — restore the normal
      // scrollback that led up to it. scrollback omitted → full history (capped in the store).
      saveScrollback(scrollbackId, serialize.serialize({ excludeAltBuffer: true }));
    } catch {
      /* serialize/storage failure → no persisted buffer this round; dispose save retries */
    }
  }
  function scheduleScrollbackSave() {
    if (attachId) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveScrollbackNow, 5000);
  }
  // P2: keep a handle so a hidden pane can release its GPU context (WebView2 caps ~16 live contexts —
  // dozens of panes otherwise starve the visible ones back into the slow DOM renderer).
  let webgl: WebglAddon | undefined;
  let webglDisposeTimer: ReturnType<typeof setTimeout> | undefined;
  // P1: buffer output for an off-screen pane and flush it on show, so a hidden pane doesn't burn CPU
  // parsing a busy TUI's redraws. FIFO; capped so a runaway producer can't grow it without bound.
  let pendingBuf: Uint8Array[] = [];
  let pendingBytes = 0;
  const PENDING_CAP = 2 * 1024 * 1024; // 2 MB
  function drainPending() {
    if (!term || !pendingBuf.length) return;
    for (const b of pendingBuf) term.write(b);
    pendingBuf = [];
    pendingBytes = 0;
  }
  function loadWebgl() {
    if (!term || webgl) return;
    try {
      const w = new WebglAddon();
      // On a runtime GPU context loss (driver reset, suspend/resume, browser reclaiming contexts)
      // dispose the dead addon so xterm reverts to its DOM renderer and keeps repainting.
      w.onContextLoss(() => {
        w.dispose();
        if (webgl === w) webgl = undefined;
        term?.refresh(0, term.rows - 1);
        refit();
      });
      term.loadAddon(w);
      webgl = w;
    } catch {
      /* WebGL unavailable → xterm uses its default renderer */
    }
  }
  let id = $state<string | null>(null);
  let myToken = 0; // this pane's fan-out channel token (0 = spawner; attach returns its own) — for detach
  let gotData = $state(false); // first PTY byte seen → drives the ssh connecting→connected dot (#17)
  let exited = $state(false);
  let error = $state('');
  let unlisteners: UnlistenFn[] = [];
  // L13: set true in onDestroy so a start()/relaunch() listen() still awaiting can bail instead of
  // pushing a listener that would fire post-destroy into a disposed terminal.
  let destroyed = false;
  let ro: ResizeObserver | undefined;
  let themeObs: MutationObserver | undefined; // re-themes the terminal when the app flips dark/light
  let resizeTimer: ReturnType<typeof setTimeout> | undefined; // trailing-debounce the PTY resize

  // Full 16-colour ANSI palette tied to the app theme (#15). Two real fixes over the old 2-colour
  // literal: a light-mode terminal instead of a permanent near-black one, and a tuned bright-black so
  // the dim `\x1b[90m` we print (e.g. "[session ended]") clears the 4.5:1 contrast bar.
  const isLight = () => document.documentElement.classList.contains('light');
  function xtermTheme(light: boolean) {
    return light
      ? {
          background: '#eff1f5', foreground: '#4c4f69', cursor: '#dc8a78',
          selectionBackground: 'rgba(30,102,245,0.25)',
          black: '#5c5f77', red: '#d20f39', green: '#40a02b', yellow: '#df8e1d',
          blue: '#1e66f5', magenta: '#ea76cb', cyan: '#179299', white: '#acb0be',
          brightBlack: '#6c6f85', brightRed: '#d20f39', brightGreen: '#40a02b', brightYellow: '#df8e1d',
          brightBlue: '#1e66f5', brightMagenta: '#ea76cb', brightCyan: '#179299', brightWhite: '#bcc0cc'
        }
      : {
          background: '#0b0e14', foreground: '#cdd6f4', cursor: '#f5e0dc',
          selectionBackground: 'rgba(137,180,250,0.30)',
          black: '#45475a', red: '#f38ba8', green: '#a6e3a1', yellow: '#f9e2af',
          blue: '#89b4fa', magenta: '#f5c2e7', cyan: '#94e2d5', white: '#bac2de',
          brightBlack: '#7f849c', brightRed: '#f38ba8', brightGreen: '#a6e3a1', brightYellow: '#f9e2af',
          brightBlue: '#89b4fa', brightMagenta: '#f5c2e7', brightCyan: '#94e2d5', brightWhite: '#a6adc8'
        };
  }

  // In-terminal find (Ctrl+Shift+F).
  let searchOpen = $state(false);
  let searchInput: HTMLInputElement | undefined = $state();
  let query = $state('');

  // Exposed to the parent (SessionsTab) so a keyboard shortcut can cycle focus between panes.
  export function focusTerminal() {
    term?.focus();
  }
  // Search this pane for an externally-supplied query (the tab's "search all panes" box, #52).
  export function runExternalSearch(q: string, next = true) {
    query = q;
    if (!q) return;
    searchOpen = true;
    if (next) search?.findNext(q);
    else search?.findPrevious(q);
  }
  // Set an absolute font size pushed from the tab's synced-zoom control (#60).
  export function setFontSize(px: number) {
    fontSize = Math.min(28, Math.max(8, px));
    if (term) term.options.fontSize = fontSize;
    try {
      localStorage.setItem(FONT_KEY, String(fontSize));
    } catch {
      /* ignore */
    }
    refit();
  }

  async function copySelection() {
    const sel = term?.getSelection();
    if (!sel) return;
    const ok = await copyText(sel);
    if (ok) pushToast({ kind: 'success', title: t('sessions.copied') }, 1500);
  }
  async function paste() {
    if (!id || exited) return;
    const text = await pasteText(); // native OS clipboard (WebView2 web clipboard is unreliable)
    if (text == null || text === '') {
      pushToast({ kind: 'error', title: t('sessions.pasteEmpty') }, 2500);
      return;
    }
    // Route through xterm.paste — it honours bracketed-paste mode (wraps in \e[200~…201~ when the
    // PTY app requested DECSET 2004, so multiline text doesn't auto-execute) and sanitizes control
    // bytes. onData then forwards to the PTY (and to broadcast, if armed).
    term?.paste(text);
  }
  function openSearch() {
    drainPending(); // P1: search must see buffered output even if this pane is currently off-screen
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
  }
  // Dump the full scrollback to a .log file (client-side download, no backend).
  function exportLog() {
    if (!term) return;
    drainPending(); // P1: export the buffered output too, not just what was on screen
    const buf = term.buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buf.length; i++) {
      const line = buf.getLine(i);
      if (line) lines.push(line.translateToString(true));
    }
    const text = lines.join('\n').replace(/\s+$/, '') + '\n';
    downloadBlob(`${label.replace(/[^\w.-]+/g, '_') || 'session'}.log`, text, 'text/plain;charset=utf-8');
  }
  function runSearch(next: boolean) {
    if (!query) return;
    if (next) search?.findNext(query);
    else search?.findPrevious(query);
  }

  const FONT_KEY = 'cmh-sessions-fontsize';
  let fontSize = $state(13);

  // Coalesce fits into one per frame. Fitting synchronously right after a font-size change (zoom)
  // measures stale glyph metrics and oscillates — especially under a full-screen TUI like opencode.
  // Deferring to the next frame lets metrics settle and collapses ResizeObserver bursts into one fit.
  const nextFrame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  let refitPending = false;
  function refit() {
    if (refitPending) return;
    refitPending = true;
    requestAnimationFrame(() => {
      refitPending = false;
      if (!term || !fit) return;
      try {
        fit.fit();
      } catch {
        return; /* layout not settled yet — the next observation retries */
      }
      // Visual fit runs every frame, but the PTY resize is trailing-debounced: a window/layout drag
      // fires the observer on every pane each frame, and SIGWINCH-storming 12 children + their TUIs
      // is wasteful. Send a single resize once the gesture settles.
      if (id) {
        const c = term.cols;
        const r = term.rows;
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          // Fire-and-forget: the child may exit in the debounce gap (map entry gone → session_not_found).
          if (id) sessionResize(id, c, r).catch(() => {});
        }, 120);
      }
    });
  }
  function zoom(delta: number) {
    setFontSize(fontSize + delta);
  }
  // Snippets: insert a templated first message into THIS pane (#57). No auto-Enter — user reviews
  // then sends. Default templates live in sessionPresets; rendered via the shared DropdownMenu (so it
  // escapes the toolbar overflow, gets roving focus + Esc, instead of a hand-rolled clipped popover).
  function insertSnippet(text: string) {
    if (id && !exited) term?.paste(text); // bracketed-paste aware; routes via onData (broadcast too)
    term?.focus();
  }
  const snipItems = $derived(MSG_SNIPPETS.map((s) => ({ label: s, onClick: () => insertSnippet(s) })));
  // Council A: one ⋯ menu aggregates every rare bar action (snippets, clear, export, zoom,
  // duplicate, background, send-to-monitor) — the bar keeps only search / maximize / close.
  const paneMenuItems = $derived([
    ...snipItems.map((s) => ({ label: `✏ ${s.label}`, onClick: s.onClick })),
    { label: `⌫ ${t('sessions.clearOutput')}`, onClick: () => term?.clear() },
    { label: `⭳ ${t('sessions.exportLog')}`, onClick: exportLog },
    { label: `A− ${t('sessions.zoomOut')}`, onClick: () => zoom(-1) },
    { label: `A+ ${t('sessions.zoomIn')}`, onClick: () => zoom(1) },
    ...(onDuplicate ? [{ label: `⧉ ${t('sessions.duplicate')} · Ctrl+Shift+D`, onClick: onDuplicate }] : []),
    ...(onBackground ? [{ label: `▁ ${t('sessions.backgroundPane')}`, onClick: onBackground }] : []),
    ...(!attachId && id && !exited && monitors.length > 1
      ? monItems.map((m) => ({ label: `🖥 ${m.label}`, onClick: m.onClick }))
      : [])
  ]);
  function onWheel(e: WheelEvent) {
    if (!e.ctrlKey) return; // plain wheel → xterm scrollback
    e.preventDefault();
    zoom(e.deltaY < 0 ? 1 : -1);
  }

  // Spawn the session and wire its streams. Re-runnable so a finished pane can relaunch in place.
  async function start() {
    if (!term || !fit) return;
    exited = false;
    error = '';
    // Spawn the PTY at the FINAL fitted size. The PTY — and the full-screen TUI it runs (e.g.
    // Claude Code on the alternate screen) — inherit cols/rows at launch. If we spawned at
    // xterm's default 80×24 and only fitted a frame later, the TUI would paint at 80 cols and
    // then a resize would arrive mid-paint, landing the redraw scrambled. Wait one frame so the
    // freshly-added grid cell has laid out, then fit synchronously before reading cols/rows.
    // A single frame can be too early if the grid is still settling, so wait (bounded) for the
    // cell to report a real size.
    for (let i = 0; i < 5 && (!host?.clientWidth || !host?.clientHeight); i++) await nextFrame();
    try {
      fit.fit();
    } catch {
      /* cell not laid out yet — the ResizeObserver will fit once it is */
    }
    // Binary output channel: raw PTY bytes arrive as ArrayBuffers (no base64/JSON per chunk).
    const chan = new Channel<ArrayBuffer>();
    chan.onmessage = (buf) => {
      // Late chunks queued in the webview loop can fire AFTER onDestroy disposed the terminal —
      // writing to a disposed xterm throws. Dropping post-close bytes is correct.
      if (destroyed) return;
      if (!gotData) gotData = true;
      const bytes = new Uint8Array(buf);
      if (visible) {
        // Flush any buffered backlog FIRST: `visible` flips true a tick before the drain effect runs,
        // so a live write here would otherwise jump ahead of older buffered bytes → scrambled TUI redraw.
        if (pendingBuf.length) drainPending();
        term?.write(bytes);
      } else {
        // P1: off-screen — buffer instead of parsing now. Flushed on show (visible effect below).
        pendingBuf.push(bytes);
        pendingBytes += bytes.length;
        if (pendingBytes > PENDING_CAP) drainPending(); // cap memory: flush once, keep buffering
        onActivity?.(paneKey); // mark unread
      }
      scheduleScrollbackSave(); // W2: persist ~5s after output settles (also on dispose)
    };
    try {
      if (attachId) {
        // Detached window: mirror an existing LIVE session (no respawn). Scrollback replays on attach.
        id = attachId;
        myToken = await sessionAttach(attachId, chan);
      } else {
        id = await sessionSpawn(profile, tool, args, cwd, term.cols, term.rows, chan, remoteDir, sshTarget);
      }
    } catch (e) {
      error = String(e);
      term.writeln(`\r\n\x1b[31m${t('sessions.spawnError', { e: String(e) })}\x1b[0m`);
      return;
    }
    // L13: the pane may have been closed (onDestroy) while the spawn/attach was in flight. onDestroy
    // ran when `id` was still null, so it couldn't tear this session down — do it here, or the live
    // PTY leaks as an orphan. Mirror onDestroy: detach a mirror, kill a session we spawned.
    if (destroyed) {
      if (attachId) sessionDetach(id, myToken);
      else void sessionKill(id);
      id = null;
      return;
    }
    onIdChange?.(paneKey, id);
    // L13: the pane can be closed (onDestroy) during the await below; onDestroy already drained
    // unlisteners, so a naive push would leak this listener — it'd later fire into a disposed term
    // and forward a stale paneKey. Register it only if still alive, else tear it down immediately.
    const exitUn = await listen<number>(`pty:exit:${id}`, () => {
      exited = true;
      term?.writeln(`\r\n\x1b[90m${t('sessions.ended')}\x1b[0m`);
      // Drop the dead session from SessionsTab's target set so broadcast / send-to-all / status
      // counts stop hitting it. The child is already gone, so relaunch()'s pre-respawn kill guard
      // (`if (id && !attachId)`) just skips a redundant kill; start() then reassigns a fresh id.
      onIdChange?.(paneKey, null);
      id = null;
    });
    if (destroyed) exitUn();
    else unlisteners.push(exitUn);
  }

  // F14: relaunch resets the terminal — confirm first so a stray click doesn't wipe the finished
  // session's scrollback (the only record left once the PTY is gone).
  let confirmRelaunch = $state(false);
  async function relaunch() {
    unlisteners.forEach((u) => u());
    unlisteners = [];
    if (id && !attachId) {
      // Await the kill before reusing the pane: the old PTY's reader thread drains asynchronously,
      // so spawning a fresh session without waiting can briefly intermix trailing stale bytes into
      // the reset terminal. The invoke resolves once the backend has killed the child (no deadlock).
      // (Attached mirrors never kill — they'd terminate the session for the owning window too.)
      try {
        await sessionKill(id);
      } catch {
        /* already gone / backend error — proceed to relaunch regardless */
      }
      id = null;
    }
    // Drop any output the PREVIOUS session buffered while hidden — otherwise a later show would drain
    // stale pre-reset bytes into the fresh terminal.
    pendingBuf = [];
    pendingBytes = 0;
    term?.reset();
    await start();
  }

  onMount(async () => {
    try {
      const f = Number(localStorage.getItem(FONT_KEY));
      if (f >= 8 && f <= 28) fontSize = f;
    } catch {
      /* ignore */
    }
    // Scrollback cap is user-configurable (Settings → View); read it per-pane so a changed
    // setting applies to every newly-opened pane without a restart (#132).
    let sb = 5000;
    try {
      const v = Number(localStorage.getItem('cmh-sessions-scrollback'));
      if (v >= 1000 && v <= 50000) sb = v;
    } catch {
      /* ignore */
    }
    // Load the bundled Nerd Font BEFORE the terminal measures glyphs: xterm's WebGL renderer bakes a
    // glyph atlas at open time, so if the font isn't ready it caches missing-glyph tofu for the
    // statusline icons (Nerd Font PUA / powerline) and keeps showing it until a resize (live-smoke).
    try {
      await document.fonts.load(`${fontSize}px 'Cascadia Code NF'`);
    } catch {
      /* font unavailable → xterm falls back to Cascadia Code / Consolas */
    }
    term = new Terminal({
      fontFamily: "'Cascadia Code NF', 'Cascadia Code', 'Consolas', monospace",
      fontSize,
      cursorBlink: true,
      scrollback: sb,
      theme: xtermTheme(isLight()),
      // Required by the Unicode11 addon below (`term.unicode` is a proposed API): without it
      // loadAddon THROWS and aborts the whole pane mount — no spawn, no input, empty terminal.
      allowProposedApi: true
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    // GPU renderer for smooth output across many panes; released for hidden panes (P2) and reloaded
    // on show. Falls back to the DOM renderer if WebGL is unavailable or the context drops.
    if (visible) loadWebgl();
    search = new SearchAddon();
    term.loadAddon(search);
    serialize = new SerializeAddon(); // W2: buffer → ANSI for cold-restore
    term.loadAddon(serialize);
    // Unicode 11 widths (#19): xterm defaults to v6, mismeasuring modern emoji / CJK and drifting the
    // cursor inside hosted TUIs (Claude's UI, ru/zh output).
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = '11';
    // Clickable URLs in output (#12) — git/build/Claude logs are full of them. Open via the OS opener,
    // not the webview, so a click never navigates the app away.
    term.loadAddon(new WebLinksAddon((_e, uri) => openUrl(uri)));
    // Clickable source locations (#13): absolute Windows paths with a :line suffix → open in editor.
    // Conservative (absolute paths only) to avoid false positives and broken relative-path opens.
    term.registerLinkProvider({
      provideLinks(y, callback) {
        const text = term?.buffer.active.getLine(y - 1)?.translateToString(true) ?? '';
        const re = /([A-Za-z]:\\[^\s:*?"<>|%&^()`]+):(\d+)(?::\d+)?/g;
        const links: ILink[] = [];
        let m: RegExpExecArray | null;
        while ((m = re.exec(text))) {
          const path = m[1];
          const line = Number(m[2]);
          const x = m.index + 1;
          links.push({
            range: { start: { x, y }, end: { x: x + m[0].length, y } },
            text: m[0],
            activate: () => openInEditor(path, line)
          });
        }
        callback(links.length ? links : undefined);
      }
    });
    // Keystrokes read `id`/`exited` live, so this single handler survives a relaunch. With broadcast
    // on, route input up to the tab so it's mirrored to every pane.
    term.onData((d) => {
      onUserInput?.(paneKey); // mark this pane as actively typed-in (defers auto-continue)
      if (broadcast && onInput) {
        onInput(d);
        return;
      }
      if (id && !exited) sessionWrite(id, d).catch(() => {}); // child may exit in the write gap
    });
    // A shell BEL (\a) is an explicit attention signal — mark the pane unread even when it's
    // on screen (unlike off-screen output). The marker self-clears when the pane is focused.
    term.onBell(() => onActivity?.(paneKey));
    // Windows-Terminal-style copy/paste so plain Ctrl+C/V behave as users expect (and so apps that
    // inject text via a simulated Ctrl+V, e.g. Sweet Whisper, land in the PTY): Ctrl+C copies when
    // there's a selection else falls through as SIGINT; Ctrl+V always pastes. Ctrl+Shift+C/V kept
    // for muscle memory. find = Ctrl+Shift+F, new session = Ctrl+Shift+T (Windows-Terminal style) so
    // plain Ctrl+F/Ctrl+T reach the shell (readline forward-char / transpose-char). return false →
    // xterm/PTY don't also receive the chord.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;
      // Shift+Enter → insert a newline instead of submitting. xterm collapses both Enter and
      // Shift+Enter to a bare CR (it branches only on Alt), so Claude Code sees \r and submits.
      // Emit ESC+CR (\x1b\r) — the sequence Claude reads as "newline" (the same bytes Alt+Enter
      // sends); route it like onData so broadcast panes mirror it. return false → xterm won't also
      // send CR. Harmless in a plain shell (reads as Alt+Enter, which has no standard action).
      if (e.key === 'Enter' && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
        const seq = '\x1b\r';
        onUserInput?.(paneKey);
        if (broadcast && onInput) onInput(seq);
        else if (id && !exited) sessionWrite(id, seq).catch(() => {});
        return false;
      }
      if (e.ctrlKey && !e.shiftKey && (e.key === 'c' || e.key === 'C')) {
        if (term?.hasSelection()) {
          copySelection();
          term.clearSelection(); // so a 2nd Ctrl+C interrupts instead of re-copying a stale selection
          return false;
        }
        return true; // no selection → let Ctrl+C through as SIGINT (interrupt)
      }
      if (e.ctrlKey && !e.shiftKey && (e.key === 'v' || e.key === 'V')) {
        // Cancel the browser's native paste, else its ClipboardEvent also triggers xterm's built-in
        // paste listener and the text lands twice. Our paste() is still needed (WebView2 clipboard /
        // synthetic Ctrl+V from Sweet Whisper / right-click / snippets all route through it).
        e.preventDefault();
        paste();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'c')) {
        copySelection();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'V' || e.key === 'v')) {
        e.preventDefault(); // see Ctrl+V above — prevent the duplicate native paste
        paste();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'f' || e.key === 'F')) {
        openSearch();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 't' || e.key === 'T') && onNewSession) {
        onNewSession();
        return false;
      }
      // #19: Shift+PgUp/PgDn scroll the scrollback (Shift+Home/End jump to top/bottom). Skipped on the
      // alternate screen so full-screen TUIs (Claude Code, less, vim) still receive the raw keys.
      if (e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey && term && term.buffer.active.type !== 'alternate') {
        if (e.key === 'PageUp') { term.scrollPages(-1); return false; }
        if (e.key === 'PageDown') { term.scrollPages(1); return false; }
        if (e.key === 'Home') { term.scrollToTop(); return false; }
        if (e.key === 'End') { term.scrollToBottom(); return false; }
      }
      return true;
    });
    ro = new ResizeObserver(() => refit());
    ro.observe(host);
    // Live-retheme when the app toggles dark/light (theme.ts flips the `light` class on <html>).
    themeObs = new MutationObserver(() => {
      if (term) term.options.theme = xtermTheme(isLight());
    });
    themeObs.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
    // Report keyboard focus up so the tab can mark the active pane (#14). xterm's hidden textarea
    // is inside host, so focusin bubbles here.
    host.addEventListener('focusin', () => onFocus?.(paneKey));
    // W2: cold-restore — replay the previous buffer as INERT scrollback BEFORE start() spawns/attaches
    // the PTY. The channel doesn't exist yet, so xterm's query auto-replies (DA/DSR) physically cannot
    // reach a shell. One scrollToBottom once the replay is parsed; no marker line (a clean buffer).
    // Attached mirrors skip this — the backend replays their scrollback on attach.
    if (!attachId) {
      const saved = takeScrollback(scrollbackId);
      if (saved) term.write(saved, () => term?.scrollToBottom());
    }
    await start();
  });

  onDestroy(() => {
    destroyed = true; // L13
    ro?.disconnect();
    themeObs?.disconnect();
    clearTimeout(resizeTimer);
    unlisteners.forEach((u) => u());
    if (id) {
      if (consumeMoved(id)) {
        // Moved to another window — keep the session alive there; just drop OUR channel so the reader
        // stops sending to this gone webview (instead of noticing lazily on the next failed send).
        sessionDetach(id, myToken);
      } else if (!attachId || ownsSession) {
        // We own it (spawner, or a detached pane that took ownership) → terminate the session.
        sessionKill(id);
      }
      // else: an attached non-owner that wasn't a move (shouldn't happen in current model) → leave it.
    }
    onIdChange?.(paneKey, null);
    if (webglDisposeTimer) clearTimeout(webglDisposeTimer);
    clearTimeout(saveTimer);
    saveScrollbackNow(); // W2: final persist while term + serialize addon are still alive
    term?.dispose();
  });

  // A hidden pane (other tab active, or another pane maximized) has zero size; re-fit when shown.
  // Also drives P1 (flush the output buffer on show) and P2 (release/restore the GPU context).
  $effect(() => {
    const vis = visible;
    maximized;
    if (vis) {
      if (webglDisposeTimer) {
        clearTimeout(webglDisposeTimer);
        webglDisposeTimer = undefined;
      }
      if (term && !webgl) loadWebgl(); // restore GPU renderer if it was released while hidden
      drainPending(); // P1: flush buffered output now that we're on screen
      if (term && fit) requestAnimationFrame(() => refit());
    } else if (webgl && !webglDisposeTimer) {
      // P2: release the GPU context, throttled — a quick tab flip shouldn't churn WebGL.
      webglDisposeTimer = setTimeout(() => {
        webglDisposeTimer = undefined;
        if (!visible) {
          webgl?.dispose();
          webgl = undefined;
        }
      }, 5000);
    }
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pane"
  ondragover={(e) => {
    if (!onDragEnter) return;
    e.preventDefault(); // REQUIRED for the drop to be valid (else the cursor shows "not allowed")
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  }}
  ondragenter={() => onDragEnter?.(paneKey)}
  ondrop={(e) => {
    if (!onDrop) return;
    e.preventDefault();
    onDrop();
  }}
>
  <!-- The bar doubles as the drag handle (xterm keeps the terminal area for selection). -->
  <div
    class="bar"
    class:ssh={isSsh}
    draggable={!!onDragStart}
    ondragstart={(e) => {
      if (!onDragStart) return;
      // Chromium/WebView2 only starts a real drag once dataTransfer carries something.
      e.dataTransfer?.setData('text/plain', paneKey);
      if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
      onDragStart(paneKey);
    }}
    title={onDragStart ? t('sessions.dragHint') : undefined}
  >
    <span
      class="dot"
      class:dead={exited}
      class:err={!!error}
      class:connecting={isSsh && !gotData && !exited && !error}
      class:working={!exited && !error && agentState === 'working'}
      class:blocked={!exited && !error && agentState === 'blocked'}
      class:done={!exited && !error && agentState === 'done'}
      class:limited={!exited && !error && agentState === 'limited'}
      title={!exited && !error && agentState && agentState !== 'unknown'
        ? t(`sessions.state_${agentState}`)
        : isSsh && !exited
          ? (gotData ? t('sessions.sshConnected') : t('sessions.sshConnecting'))
          : undefined}
    ></span>
    {#if renaming}
      <input class="rename-input" bind:this={renameInput} bind:value={editName}
        onkeydown={(e) => { if (e.key === 'Enter') commitRename(); else if (e.key === 'Escape') renaming = false; }}
        onblur={commitRename} placeholder={label} spellcheck="false" />
    {:else}
      <span class="name" title={onRename ? t('sessions.renameHint') : fullTitle} ondblclick={startRename}>{displayName || label}</span>
      {#if onRename}
        <!-- Council M: dblclick-rename was tooltip-only — an ✎ that appears on bar hover makes it discoverable. -->
        <button class="x rename-ic" onclick={startRename} title={t('sessions.renameHint')} aria-label={t('sessions.renameHint')}>✎</button>
      {/if}
    {/if}
    {#if cwd && !sshTarget && folderName}<span class="folder" title={cwd}>{folderName}</span>{/if}
    <!-- F12: open the working folder in Explorer. Local panes only — an SSH pane's cwd is a remote path. -->
    {#if cwd && !sshTarget}
      <button class="x" onclick={() => openPath(cwd).catch((e) => pushToast({ kind: 'error', title: String(e) }))}
        title={t('sessions.openCwd')} aria-label={t('sessions.openCwd')}><FolderOpen size={14} /></button>
    {/if}
    {#if args}<span class="argbadge" title={args}>⚑</span>{/if}
    {#if tool === 'claude' && profile && showUsage}<ProfileUsageBadge {profile} compact />{/if}
    <!-- #21f: Castellyn will auto-resume this pane once the limit resets — surfaced so the user knows -->
    {#if autoResumeLabel}<span class="autoresume" title={t('sessions.autoResumeTip')}><RotateCw size={12} /> {autoResumeLabel}</span>{/if}
    <span class="spacer"></span>
    {#if exited || error}
      <button class="x relaunch" onclick={() => (confirmRelaunch = true)} title={t('sessions.relaunch')}><RotateCw size={14} /> {t('sessions.relaunch')}</button>
    {/if}
    <!-- Council A (2026-07): the bar held ~10 unlabeled icon buttons in a tight row and «close»
         sat one slip away from «maximize». Frequent actions stay (search / maximize / close);
         everything rare lives in ONE ⋯ menu; close is set apart and turns red on hover. -->
    <button class="x" onclick={openSearch} title={t('sessions.find')} aria-label={t('sessions.find')}><Search size={14} /></button>
    <DropdownMenu title={t('sessions.moreActions')} items={paneMenuItems} />
    {#if onToggleMax}
      <button class="x" onclick={onToggleMax}
        title={maximized ? t('sessions.restore') : t('sessions.maximize')}
        aria-label={maximized ? t('sessions.restore') : t('sessions.maximize')}>{#if maximized}<Minimize2 size={14} />{:else}<Maximize2 size={14} />{/if}</button>
    {/if}
    {#if onReturnToMain}
      <button class="x" onclick={onReturnToMain} title={t('sessions.returnToMain')} aria-label={t('sessions.returnToMain')}>←</button>
    {/if}
    <button class="x close" onclick={onClose} title={t('sessions.closePane')} aria-label={t('sessions.closePane')}><X size={14} /></button>
  </div>
  {#if searchOpen}
    <div class="find">
      <input
        bind:this={searchInput}
        bind:value={query}
        class="sw-input text-sw-xs"
        placeholder={t('sessions.findPlaceholder')}
        spellcheck="false"
        oninput={() => runSearch(true)}
        onkeydown={(e) => {
          if (e.key === 'Enter') runSearch(!e.shiftKey);
          else if (e.key === 'Escape') searchOpen = false;
        }}
      />
      <button class="x" onclick={() => runSearch(false)} title={t('sessions.findPrev')} aria-label={t('sessions.findPrev')}><ChevronUp size={14} /></button>
      <button class="x" onclick={() => runSearch(true)} title={t('sessions.findNext')} aria-label={t('sessions.findNext')}><ChevronDown size={14} /></button>
      <button class="x" onclick={() => (searchOpen = false)} aria-label={t('sessions.closeFind')}><X size={14} /></button>
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="term" bind:this={host} onwheel={onWheel} oncontextmenu={(e) => { e.preventDefault(); paste(); }}></div>
</div>

<ConfirmDialog
  open={confirmRelaunch}
  title={t('sessions.relaunchConfirmTitle')}
  message={t('sessions.relaunchConfirmMsg')}
  confirmLabel={t('sessions.relaunch')}
  danger
  onConfirm={() => {
    confirmRelaunch = false;
    relaunch();
  }}
  onCancel={() => (confirmRelaunch = false)}
/>

<style>
  .pane {
    position: relative;
    display: flex;
    flex-direction: column;
    /* Fill the flex cell: without flex-grow a flex child sizes to its content width
       (the xterm grid ≈ cols × cell-width), so the pane never filled wide columns and
       maximize only grew it vertically. */
    flex: 1;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    overflow: hidden;
    background: #0b0e14;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    background: var(--sw-bg-secondary);
    border-bottom: 1px solid var(--sw-border);
  }
  .bar[draggable='true'] {
    cursor: grab;
  }
  .bar[draggable='true']:active {
    cursor: grabbing;
  }
  .relaunch {
    width: auto;
    padding: 0 6px;
    color: var(--sw-accent-text);
    font-size: 11px;
  }
  .find {
    position: absolute;
    top: 34px;
    right: 8px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.35);
  }
  .find input {
    width: 160px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--sw-status-up);
    flex-shrink: 0;
  }
  .dot.dead {
    background: var(--sw-status-off);
  }
  .dot.err {
    background: var(--sw-status-down);
  }
  .dot.connecting {
    background: var(--sw-status-warn);
    animation: dot-pulse 1.1s ease-in-out infinite;
  }
  /* Agent status (herdr-style): working = pulsing warn, blocked = pulsing danger,
     done = steady teal (finished, not yet looked at). Idle keeps the default green. */
  .dot.working {
    background: var(--sw-status-warn);
    animation: dot-pulse 1.1s ease-in-out infinite;
  }
  .dot.blocked {
    background: var(--sw-danger, #f85149);
    animation: dot-pulse 0.7s ease-in-out infinite;
  }
  .dot.done {
    background: var(--sw-status-done);
  }
  /* Usage limit hit (21b): a steady strong red — distinct from blocked (pulsing danger) and from
     the amber "working". A ring makes it read as "stopped on quota" at a glance. */
  .dot.limited {
    background: var(--sw-status-down, #ef4444);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--sw-status-down, #ef4444) 35%, transparent);
  }
  @keyframes dot-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
  }
  /* SSH panes get a subtle accent so remote sessions stand out from local ones (#20). */
  .bar.ssh {
    border-left: 3px solid var(--sw-accent-text, #5aa2ff);
    padding-left: 5px;
  }
  .name {
    font-size: var(--sw-text-xs);
    font-weight: 600;
    color: var(--sw-text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 0;
    max-width: 50%;
    cursor: text;
  }
  .rename-input {
    font-size: var(--sw-text-xs);
    font-weight: 600;
    color: var(--sw-text-primary);
    background: var(--sw-input-bg);
    border: 1px solid var(--sw-border-focus);
    border-radius: var(--sw-radius-sm, 6px);
    padding: 1px 4px;
    max-width: 50%;
    outline: none;
  }
  .folder {
    font-size: 11px;
    color: var(--sw-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .argbadge {
    font-size: 11px;
    color: var(--sw-accent-text);
    flex-shrink: 0;
  }
  /* #21f: "will auto-resume" pill — a calm accent chip so the user sees Castellyn has the limit handled. */
  .autoresume {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 999px;
    color: var(--sw-accent-text);
    border: 1px solid color-mix(in srgb, var(--sw-accent-text) 40%, transparent);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .spacer {
    flex: 1;
  }
  .x {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    font-size: 12px;
    padding: 0 4px;
    line-height: 1;
  }
  .x:hover {
    color: var(--sw-text-primary);
  }
  /* Close is set apart from its neighbours and reads as destructive on hover (council A). */
  .x.close {
    margin-left: 6px;
  }
  .x.close:hover {
    color: var(--sw-danger, #f85149);
  }
  /* Rename affordance: hidden until the bar is hovered (council M — dblclick was undiscoverable). */
  .rename-ic {
    opacity: 0;
    font-size: 10px;
    transition: opacity 0.12s;
  }
  .bar:hover .rename-ic,
  .rename-ic:focus-visible {
    opacity: 0.85;
  }
  /* V1: the "clone this agent" action is the 90% path — accent it so it stands out from the muted
     icon row (Ctrl+Shift+D also triggers it). */
  .term {
    flex: 1;
    min-height: 0;
    padding: 4px;
  }
</style>
