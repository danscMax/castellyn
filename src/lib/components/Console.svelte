<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/i18n';
  import { opName } from '$lib/running.svelte';
  import { copyText } from '$lib/clipboard';
  import { classifyLine } from '$lib/logKind';

  let {
    log,
    running,
    busy = false,
    revealSignal = 0,
    onClear,
    onCancel,
    onCancelAll
  }: {
    log: string[];
    running: string | null;
    /** F21: any cancellable work in flight (run / forks / bulk plugin) — shows "Cancel all". */
    busy?: boolean;
    /** Bump this counter to force-expand the console (e.g. a toast's "Open log"). */
    revealSignal?: number;
    onClear: () => void;
    onCancel: () => void;
    onCancelAll?: () => void;
  } = $props();

  let logEl: HTMLDivElement | undefined = $state();
  let height = $state(220);
  let collapsed = $state(true);
  let copied = $state(false);

  // Render only the tail by default (item V-5): the full buffer is capped at 5000 lines, but
  // mounting all of them — and re-diffing a non-keyed each on every front-trim — janks a verbose
  // run. The whole `log` is still kept for copy/search; only the DOM is windowed. "Show all"
  // opts back into the full render on demand (a legitimate 5000-node log dump is a rare click).
  const LOG_WINDOW = 500;
  let showAll = $state(false);
  let filter = $state('');
  const windowed = $derived(showAll || log.length <= LOG_WINDOW ? log : log.slice(-LOG_WINDOW));
  // Search the FULL buffer, not just the windowed tail — the comment above promises "kept for search"
  // and a user filtering a 5000-line run expects every match, not only those in the last 500. A
  // filtered result set is normally small, so rendering all matches is fine.
  const view = $derived(
    filter ? log.filter((l) => l.toLowerCase().includes(filter.toLowerCase())) : windowed
  );
  const hiddenCount = $derived(showAll ? 0 : Math.max(0, log.length - LOG_WINDOW));

  // P7: classify each visible line ONCE per view change (in a derived) instead of running four
  // class:-directive expressions — incl. a regex — on every render of every line. Classifier is a
  // pure, unit-tested module (V11: ru+en failure vocabulary).
  const viewClassified = $derived(view.map((text) => ({ text, kind: classifyLine(text) })));

  // Smart autoscroll: only pin to the bottom when the user is already there; if they've scrolled up
  // to read, hold position and raise a "▾ new lines" pill instead of yanking them back. `atBottom`
  // is a plain (non-reactive) flag on purpose — reading it inside the autoscroll effect must NOT make
  // the effect re-run on every scroll, only on new log lines.
  let atBottom = true;
  let hasNewLines = $state(false);
  function onLogScroll() {
    if (!logEl) return;
    atBottom = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 40;
    if (atBottom) hasNewLines = false;
  }
  function scrollToBottom() {
    if (!logEl) return;
    logEl.scrollTop = logEl.scrollHeight;
    atBottom = true;
    hasNewLines = false;
  }

  // Re-engage windowing for each new run: Clear must undo a prior "Show all" opt-in, otherwise
  // one full-render click disables windowing for the rest of the session.
  function handleClear() {
    showAll = false;
    onClear();
  }

  async function copyLog() {
    if (await copyText(log.join('\n'))) {
      copied = true;
      setTimeout(() => (copied = false), 1500);
    }
  }

  const HKEY = 'cmh-console-h';
  const CKEY = 'cmh-console-collapsed';

  onMount(() => {
    try {
      const h = Number(localStorage.getItem(HKEY));
      if (h > 0) height = Math.min(Math.max(h, 120), Math.round(window.innerHeight * 0.6));
      const c = localStorage.getItem(CKEY);
      // Default collapsed (it's a detail panel, not the main view); the user's explicit choice is
      // remembered in CKEY. A toast's "Open log" can still reveal it on demand via revealSignal.
      collapsed = c != null ? c === '1' : true;
    } catch {
      /* ignore */
    }
  });

  // A run does NOT force the console open — the collapsed/expanded choice is the user's
  // (persisted in CKEY). While collapsed, the header still shows a "live" pulse + line count,
  // and an error toast's "Open log" action can reveal it via revealSignal below.

  // Force-expand on external reveal signal (toast action).
  $effect(() => {
    if (revealSignal > 0) collapsed = false;
  });

  // Autoscroll to bottom on new lines (when visible). Defer the layout write to the next frame so
  // rapid batched appends coalesce into one scroll per frame instead of thrashing layout (item 7).
  $effect(() => {
    log.length;
    if (logEl && !collapsed) {
      if (atBottom) {
        requestAnimationFrame(() => {
          // The element can unmount (tab switch / collapse) before the frame fires — re-guard the ref.
          if (logEl && !collapsed) logEl.scrollTop = logEl.scrollHeight;
        });
      } else {
        hasNewLines = true;
      }
    }
  });

  function toggle() {
    collapsed = !collapsed;
    try {
      localStorage.setItem(CKEY, collapsed ? '1' : '0');
    } catch {
      /* ignore */
    }
  }

  function onResizeStart(e: PointerEvent) {
    const startY = e.clientY;
    const startH = height;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const max = Math.round(window.innerHeight * 0.6);
      height = Math.min(Math.max(startH + (startY - ev.clientY), 120), max);
    };
    const up = (ev: PointerEvent) => {
      target.releasePointerCapture(ev.pointerId);
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      try {
        localStorage.setItem(HKEY, String(height));
      } catch {
        /* ignore */
      }
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    // pointercancel (touch/pen interruption) otherwise leaves move bound + capture unreleased.
    target.addEventListener('pointercancel', up);
  }
</script>

<section class="console" class:collapsed>
  {#if !collapsed}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="resizer"
      onpointerdown={onResizeStart}
      title={t('console.resize')}
      role="separator"
      aria-orientation="horizontal"
    ></div>
  {/if}

  <header>
    <button class="toggle" onclick={toggle} title={collapsed ? t('console.expand') : t('console.collapse')}>
      <span class="chev">{collapsed ? '▸' : '▾'}</span>
      <span class="title">{t('console.title')}</span>
      {#if running}<span class="live">{t('console.live', { id: opName(running) })}</span>{/if}
      {#if collapsed && log.length}<span class="count">{t('console.lines', { n: log.length })}</span>{/if}
    </button>
    <div class="actions">
      {#if !collapsed}
        <input
          class="search"
          type="text"
          bind:value={filter}
          placeholder={t('console.searchPlaceholder')}
          aria-label={t('console.searchPlaceholder')}
        />
      {/if}
      {#if running}
        <button class="sw-btn sw-btn-ghost mini" onclick={onCancel} title={t('console.cancelRun')}>
          {t('common.cancelAction')}
        </button>
      {/if}
      {#if busy && onCancelAll}
        <button class="sw-btn sw-btn-danger mini" onclick={onCancelAll} title={t('console.cancelAllHint')}>
          {t('console.cancelAll')}
        </button>
      {/if}
      <button
        class="sw-btn sw-btn-ghost mini"
        onclick={copyLog}
        disabled={!log.length}
        title={t('console.copyHint')}
      >
        {copied ? t('console.copiedShort') : t('common.copy')}
      </button>
      <button
        class="sw-btn sw-btn-ghost mini"
        onclick={handleClear}
        disabled={!!running}
        title={t('console.clearHint')}
      >
        {t('common.clear')}
      </button>
    </div>
  </header>

  {#if !collapsed}
    {#if log.length}
      {#if hiddenCount > 0}
        <div class="windowed-bar">
          <span>{t('console.windowed', { shown: LOG_WINDOW, total: log.length })}</span>
          <button class="link-btn" onclick={() => (showAll = true)}>{t('console.showAll')}</button>
        </div>
      {/if}
      <div class="log-wrap">
        <div bind:this={logEl} class="log" style="height:{height}px" onscroll={onLogScroll}>
          {#each viewClassified as l}
            <div
              class="log-line"
              class:log-warn={l.kind === 'warn'}
              class:log-diag={l.kind === 'diag'}
              class:log-ok={l.kind === 'ok'}
              class:log-err={l.kind === 'err'}
            >{l.text}</div>
          {/each}
        </div>
        {#if hasNewLines}
          <button class="newlines-pill" onclick={scrollToBottom}>▾ {t('console.newLines')}</button>
        {/if}
      </div>
    {:else}
      <pre class="empty-log" style="height:{height}px">{t('console.empty')}</pre>
    {/if}
  {/if}
</section>

<style>
  .console {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--sw-border);
    background: color-mix(in srgb, var(--sw-bg-secondary) 50%, transparent);
  }
  .resizer {
    height: 10px;
    margin-top: -5px;
    cursor: ns-resize;
    flex-shrink: 0;
    transition: background 0.15s ease;
  }
  .windowed-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sw-space-3);
    padding: 2px var(--sw-space-4);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    background: color-mix(in srgb, var(--sw-bg-secondary) 40%, transparent);
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--sw-accent-text);
    cursor: pointer;
    font-size: var(--sw-text-xs);
    padding: 0;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
  .resizer:hover {
    background: var(--sw-accent);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sw-space-2) var(--sw-space-4);
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    border: none;
    background: transparent;
    color: var(--sw-text-primary);
    cursor: pointer;
    font-size: var(--sw-text-sm);
    font-weight: 500;
    padding: 0;
  }
  .chev {
    color: var(--sw-text-muted);
    width: 12px;
  }
  .live {
    color: var(--sw-accent-text);
    font-size: var(--sw-text-xs);
    animation: pulse 2s infinite;
  }
  .count {
    color: var(--sw-text-muted);
    font-size: var(--sw-text-xs);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .search {
    width: 140px;
    padding: 3px 8px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-primary);
    background: var(--sw-bg-primary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm, 4px);
  }
  .search:focus-visible {
    outline: none;
    border-color: var(--sw-accent);
  }
  .log-wrap {
    position: relative;
  }
  .newlines-pill {
    position: absolute;
    bottom: 10px;
    left: 50%;
    transform: translateX(-50%);
    padding: 4px 12px;
    font-size: var(--sw-text-xs);
    color: #fff;
    background: var(--sw-accent-solid);
    border: none;
    border-radius: 9999px;
    cursor: pointer;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  }
  .mini {
    padding: 3px 10px;
    font-size: var(--sw-text-xs);
  }
  .log,
  pre {
    overflow: auto;
    margin: 0;
    padding: var(--sw-space-4);
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    line-height: 1.5;
  }
  .log-line {
    color: var(--sw-text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .log-warn {
    /* canon amber; --sw-warn now carries a light override, so no per-component light rule needed. */
    color: var(--sw-warn);
  }
  .log-diag {
    color: var(--sw-text-dimmed);
  }
  .log-ok {
    color: var(--sw-success);
  }
  /* z5_4: any line naming error/fail/exception reads as a failure even without the ⚠ prefix. */
  .log-err {
    color: var(--sw-status-bad);
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }
</style>
