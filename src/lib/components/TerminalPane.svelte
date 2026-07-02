<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal, type ILink } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import { Unicode11Addon } from '@xterm/addon-unicode11';
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
  import DropdownMenu from './DropdownMenu.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { markMoved, consumeMoved } from '$lib/sessionMove';
  import { MSG_SNIPPETS } from '$lib/sessionPresets';
  import { t } from '$lib/i18n';
  import ProfileUsageBadge from './ProfileUsageBadge.svelte';
  import { copyText, pasteText } from '$lib/clipboard';
  import { pushToast } from '$lib/toast.svelte';

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
    onIdChange,
    onNewSession,
    onActivity,
    onFocus,
    displayName = '',
    onRename
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
    displayName?: string;
    onRename?: (key: string, name: string) => void;
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
  let id = $state<string | null>(null);
  let myToken = 0; // this pane's fan-out channel token (0 = spawner; attach returns its own) — for detach
  let gotData = $state(false); // first PTY byte seen → drives the ssh connecting→connected dot (#17)
  let exited = $state(false);
  let error = $state('');
  let unlisteners: UnlistenFn[] = [];
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
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
  }
  // Dump the full scrollback to a .log file (client-side download, no backend).
  function exportLog() {
    if (!term) return;
    const buf = term.buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buf.length; i++) {
      const line = buf.getLine(i);
      if (line) lines.push(line.translateToString(true));
    }
    const text = lines.join('\n').replace(/\s+$/, '') + '\n';
    const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${label.replace(/[^\w.-]+/g, '_') || 'session'}.log`;
    a.click();
    URL.revokeObjectURL(url);
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
          if (id) sessionResize(id, c, r);
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
      if (!gotData) gotData = true;
      term?.write(new Uint8Array(buf));
      // Mark unread when output lands in a pane that isn't currently on screen.
      if (!visible) onActivity?.(paneKey);
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
    onIdChange?.(paneKey, id);
    unlisteners.push(
      await listen<number>(`pty:exit:${id}`, () => {
        exited = true;
        term?.writeln(`\r\n\x1b[90m${t('sessions.ended')}\x1b[0m`);
      })
    );
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
    term = new Terminal({
      fontFamily: "'Cascadia Code', 'Consolas', monospace",
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
    // GPU renderer for smooth output across many panes; fall back to canvas if the context drops.
    try {
      const webgl = new WebglAddon();
      // On a runtime GPU context loss (driver reset, suspend/resume, browser reclaiming contexts
      // when many panes are open) dispose the dead WebGL addon so xterm reverts to its DOM renderer,
      // then force a reflow/redraw so this pane keeps repainting instead of freezing until reopened.
      // We do NOT re-load WebGL — the DOM renderer keeps the pane alive without a GPU context.
      webgl.onContextLoss(() => {
        webgl.dispose();
        term?.refresh(0, term.rows - 1);
        refit();
      });
      term.loadAddon(webgl);
    } catch {
      /* WebGL unavailable → xterm uses its default renderer */
    }
    search = new SearchAddon();
    term.loadAddon(search);
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
      if (broadcast && onInput) {
        onInput(d);
        return;
      }
      if (id && !exited) sessionWrite(id, d);
    });
    // Windows-Terminal-style copy/paste so plain Ctrl+C/V behave as users expect (and so apps that
    // inject text via a simulated Ctrl+V, e.g. Sweet Whisper, land in the PTY): Ctrl+C copies when
    // there's a selection else falls through as SIGINT; Ctrl+V always pastes. Ctrl+Shift+C/V kept
    // for muscle memory. find = Ctrl+Shift+F, new session = Ctrl+Shift+T (Windows-Terminal style) so
    // plain Ctrl+F/Ctrl+T reach the shell (readline forward-char / transpose-char). return false →
    // xterm/PTY don't also receive the chord.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;
      if (e.ctrlKey && !e.shiftKey && (e.key === 'c' || e.key === 'C')) {
        if (term?.hasSelection()) {
          copySelection();
          term.clearSelection(); // so a 2nd Ctrl+C interrupts instead of re-copying a stale selection
          return false;
        }
        return true; // no selection → let Ctrl+C through as SIGINT (interrupt)
      }
      if (e.ctrlKey && !e.shiftKey && (e.key === 'v' || e.key === 'V')) {
        paste();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'c')) {
        copySelection();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'V' || e.key === 'v')) {
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
    await start();
  });

  onDestroy(() => {
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
    term?.dispose();
  });

  // A hidden pane (other tab active, or another pane maximized) has zero size; re-fit when shown.
  $effect(() => {
    visible;
    maximized;
    if (term && fit && visible) requestAnimationFrame(() => refit());
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
      title={isSsh && !exited ? (gotData ? t('sessions.sshConnected') : t('sessions.sshConnecting')) : undefined}
    ></span>
    {#if renaming}
      <input class="rename-input" bind:this={renameInput} bind:value={editName}
        onkeydown={(e) => { if (e.key === 'Enter') commitRename(); else if (e.key === 'Escape') renaming = false; }}
        onblur={commitRename} placeholder={label} spellcheck="false" />
    {:else}
      <span class="name" title={onRename ? t('sessions.renameHint') : fullTitle} ondblclick={startRename}>{displayName || label}</span>
    {/if}
    {#if tool === 'claude' && folderName}<span class="folder" title={cwd}>{folderName}</span>{/if}
    <!-- F12: open the working folder in Explorer. Local panes only — an SSH pane's cwd is a remote path. -->
    {#if cwd && !sshTarget}
      <button class="x" onclick={() => openPath(cwd).catch((e) => pushToast({ kind: 'error', title: String(e) }))}
        title={t('sessions.openCwd')} aria-label={t('sessions.openCwd')}>📁</button>
    {/if}
    {#if args}<span class="argbadge" title={args}>⚑</span>{/if}
    {#if tool === 'claude' && profile}<ProfileUsageBadge {profile} compact />{/if}
    <span class="spacer"></span>
    {#if exited || error}
      <button class="x relaunch" onclick={() => (confirmRelaunch = true)} title={t('sessions.relaunch')}>↻ {t('sessions.relaunch')}</button>
    {/if}
    <DropdownMenu glyph="❡" title={t('sessions.snippets')} items={snipItems} />
    <button class="x" onclick={openSearch} title={t('sessions.find')} aria-label={t('sessions.find')}>🔍</button>
    <button class="x" onclick={() => term?.clear()} title={t('sessions.clearOutput')} aria-label={t('sessions.clearOutput')}>⌫</button>
    <button class="x" onclick={exportLog} title={t('sessions.exportLog')} aria-label={t('sessions.exportLog')}>⭳</button>
    <button class="x" onclick={() => zoom(-1)} title={t('sessions.zoomOut')} aria-label={t('sessions.zoomOut')}>A−</button>
    <button class="x" onclick={() => zoom(1)} title={t('sessions.zoomIn')} aria-label={t('sessions.zoomIn')}>A+</button>
    {#if !attachId && id && !exited && monitors.length > 1}
      <!-- U6: glyph (not label) so the accessible name comes from title, not the «⬈» symbol -->
      <DropdownMenu glyph="⬈" title={t('sessions.toMonitorTip')} items={monItems} />
    {/if}
    {#if onDuplicate}
      <button class="x" onclick={onDuplicate} title={t('sessions.duplicate')} aria-label={t('sessions.duplicate')}>⧉</button>
    {/if}
    {#if onBackground}
      <button class="x" onclick={onBackground} title={t('sessions.backgroundPane')} aria-label={t('sessions.backgroundPane')}>🗕</button>
    {/if}
    {#if onToggleMax}
      <button class="x" onclick={onToggleMax}
        title={maximized ? t('sessions.restore') : t('sessions.maximize')}
        aria-label={maximized ? t('sessions.restore') : t('sessions.maximize')}>{maximized ? '⤡' : '⤢'}</button>
    {/if}
    {#if onReturnToMain}
      <button class="x" onclick={onReturnToMain} title={t('sessions.returnToMain')} aria-label={t('sessions.returnToMain')}>←</button>
    {/if}
    <button class="x" onclick={onClose} title={t('sessions.closePane')} aria-label={t('sessions.closePane')}>✕</button>
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
      <button class="x" onclick={() => runSearch(false)} title={t('sessions.findPrev')} aria-label={t('sessions.findPrev')}>↑</button>
      <button class="x" onclick={() => runSearch(true)} title={t('sessions.findNext')} aria-label={t('sessions.findNext')}>↓</button>
      <button class="x" onclick={() => (searchOpen = false)} aria-label={t('sessions.closeFind')}>✕</button>
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
    background: var(--sw-status-warn, #e0b341);
    animation: dot-pulse 1.1s ease-in-out infinite;
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
  .spacer {
    flex: 1;
  }
  .x {
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
  .term {
    flex: 1;
    min-height: 0;
    padding: 4px;
  }
</style>
