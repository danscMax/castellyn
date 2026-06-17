<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { Channel } from '@tauri-apps/api/core';
  import '@xterm/xterm/css/xterm.css';
  import { sessionSpawn, sessionWrite, sessionResize, sessionKill, type SessionTool } from '$lib/ipc';
  import { t } from '$lib/i18n';

  let {
    profile,
    tool = 'claude',
    args = '',
    cwd = undefined,
    paneKey = '',
    visible = true,
    maximized = false,
    broadcast = false,
    onClose,
    onToggleMax,
    onDuplicate,
    onDragStart,
    onDragEnter,
    onDrop,
    onInput,
    onIdChange,
    onNewSession
  }: {
    profile: string;
    tool?: SessionTool;
    args?: string;
    cwd?: string;
    paneKey?: string;
    visible?: boolean;
    maximized?: boolean;
    broadcast?: boolean;
    onClose: () => void;
    onToggleMax?: () => void;
    onDuplicate?: () => void;
    onDragStart?: (key: string) => void;
    onDragEnter?: (key: string) => void;
    onDrop?: () => void;
    onInput?: (data: string) => void;
    onIdChange?: (key: string, id: string | null) => void;
    onNewSession?: () => void;
  } = $props();

  // Pane title: tool + the profile (claude) or the folder it's running in (opencode/shell).
  const folderName = $derived(cwd ? cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd : '');
  const label = $derived(
    tool === 'claude' ? `${tool} · ${profile}` : folderName ? `${tool} · ${folderName}` : tool
  );
  // Full hover detail: tool/profile + folder + launch args.
  const fullTitle = $derived(
    [label, cwd, args].filter(Boolean).join(' · ') || t('sessions.paneTitle', { profile: label })
  );

  let host: HTMLDivElement;
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let search: SearchAddon | undefined;
  let id = $state<string | null>(null);
  let exited = $state(false);
  let error = $state('');
  let unlisteners: UnlistenFn[] = [];
  let ro: ResizeObserver | undefined;

  // In-terminal find (Ctrl+F).
  let searchOpen = $state(false);
  let searchInput: HTMLInputElement | undefined = $state();
  let query = $state('');

  async function copySelection() {
    const sel = term?.getSelection();
    if (sel) {
      try {
        await navigator.clipboard.writeText(sel);
      } catch {
        /* clipboard blocked */
      }
    }
  }
  async function paste() {
    try {
      const text = await navigator.clipboard.readText();
      if (text && id && !exited) sessionWrite(id, text);
    } catch {
      /* clipboard blocked */
    }
  }
  function openSearch() {
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
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
  let refitPending = false;
  function refit() {
    if (refitPending) return;
    refitPending = true;
    requestAnimationFrame(() => {
      refitPending = false;
      if (!term || !fit) return;
      try {
        fit.fit();
        if (id) sessionResize(id, term.cols, term.rows);
      } catch {
        /* layout not settled yet — the next observation retries */
      }
    });
  }
  function zoom(delta: number) {
    fontSize = Math.min(28, Math.max(8, fontSize + delta));
    if (term) term.options.fontSize = fontSize;
    try {
      localStorage.setItem(FONT_KEY, String(fontSize));
    } catch {
      /* ignore */
    }
    refit();
  }
  function onWheel(e: WheelEvent) {
    if (!e.ctrlKey) return; // plain wheel → xterm scrollback
    e.preventDefault();
    zoom(e.deltaY < 0 ? 1 : -1);
  }

  // Spawn the session and wire its streams. Re-runnable so a finished pane can relaunch in place.
  async function start() {
    if (!term) return;
    exited = false;
    error = '';
    refit();
    // Binary output channel: raw PTY bytes arrive as ArrayBuffers (no base64/JSON per chunk).
    const chan = new Channel<ArrayBuffer>();
    chan.onmessage = (buf) => term?.write(new Uint8Array(buf));
    try {
      id = await sessionSpawn(profile, tool, args, cwd, term.cols, term.rows, chan);
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

  async function relaunch() {
    unlisteners.forEach((u) => u());
    unlisteners = [];
    if (id) {
      sessionKill(id);
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
    term = new Terminal({
      fontFamily: "'Cascadia Code', 'Consolas', monospace",
      fontSize,
      cursorBlink: true,
      scrollback: 5000,
      theme: { background: '#0b0e14', foreground: '#cdd6f4' }
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    // GPU renderer for smooth output across many panes; fall back to canvas if the context drops.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch {
      /* WebGL unavailable → xterm uses its default renderer */
    }
    search = new SearchAddon();
    term.loadAddon(search);
    // Keystrokes read `id`/`exited` live, so this single handler survives a relaunch. With broadcast
    // on, route input up to the tab so it's mirrored to every pane.
    term.onData((d) => {
      if (broadcast && onInput) {
        onInput(d);
        return;
      }
      if (id && !exited) sessionWrite(id, d);
    });
    // Copy (Ctrl+Shift+C), paste (Ctrl+Shift+V), find (Ctrl+F) — return false so xterm/PTY don't
    // also receive the chord (plain Ctrl+C stays SIGINT).
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== 'keydown') return true;
      if (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'c')) {
        copySelection();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && (e.key === 'V' || e.key === 'v')) {
        paste();
        return false;
      }
      if (e.ctrlKey && !e.shiftKey && (e.key === 'f' || e.key === 'F')) {
        openSearch();
        return false;
      }
      if (e.ctrlKey && !e.shiftKey && (e.key === 't' || e.key === 'T') && onNewSession) {
        onNewSession();
        return false;
      }
      return true;
    });
    ro = new ResizeObserver(() => refit());
    ro.observe(host);
    await start();
  });

  onDestroy(() => {
    ro?.disconnect();
    unlisteners.forEach((u) => u());
    if (id) sessionKill(id);
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
    <span class="dot" class:dead={exited} class:err={!!error}></span>
    <span class="name" title={fullTitle}>{label}</span>
    {#if tool === 'claude' && folderName}<span class="folder" title={cwd}>{folderName}</span>{/if}
    {#if args}<span class="argbadge" title={args}>⚑</span>{/if}
    <span class="spacer"></span>
    {#if exited}
      <button class="x relaunch" onclick={relaunch} title={t('sessions.relaunch')}>↻ {t('sessions.relaunch')}</button>
    {/if}
    <button class="x" onclick={openSearch} title={t('sessions.find')} aria-label={t('sessions.find')}>🔍</button>
    <button class="x" onclick={() => zoom(-1)} title={t('sessions.zoomOut')} aria-label={t('sessions.zoomOut')}>A−</button>
    <button class="x" onclick={() => zoom(1)} title={t('sessions.zoomIn')} aria-label={t('sessions.zoomIn')}>A+</button>
    {#if onDuplicate}
      <button class="x" onclick={onDuplicate} title={t('sessions.duplicate')} aria-label={t('sessions.duplicate')}>⧉</button>
    {/if}
    {#if onToggleMax}
      <button class="x" onclick={onToggleMax}
        title={maximized ? t('sessions.restore') : t('sessions.maximize')}
        aria-label={maximized ? t('sessions.restore') : t('sessions.maximize')}>{maximized ? '⤡' : '⤢'}</button>
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
      <button class="x" onclick={() => (searchOpen = false)} aria-label={t('sessions.closePane')}>✕</button>
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="term" bind:this={host} onwheel={onWheel} oncontextmenu={(e) => { e.preventDefault(); paste(); }}></div>
</div>

<style>
  .pane {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
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
    background: #10b981;
    flex-shrink: 0;
  }
  .dot.dead {
    background: #6b7280;
  }
  .dot.err {
    background: #ef4444;
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
