<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/i18n';
  import { opName } from '$lib/running.svelte';
  import { copyText } from '$lib/clipboard';

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
  let resizing = $state(false);
  let copied = $state(false);

  async function copyLog() {
    if (await copyText(log.join('\n'))) {
      copied = true;
      setTimeout(() => (copied = false), 1500);
    }
  }

  const HKEY = 'cmh-console-h';
  const CKEY = 'cmh-console-collapsed';

  onMount(() => {
    const h = Number(localStorage.getItem(HKEY));
    if (h > 0) height = Math.min(Math.max(h, 120), Math.round(window.innerHeight * 0.6));
    const c = localStorage.getItem(CKEY);
    // Default collapsed (it's a detail panel, not the main view); the user's explicit choice is
    // remembered in CKEY. A toast's "Open log" can still reveal it on demand via revealSignal.
    collapsed = c != null ? c === '1' : true;
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
    if (logEl && !collapsed)
      requestAnimationFrame(() => {
        // The element can unmount (tab switch / collapse) before the frame fires — re-guard the ref.
        if (logEl && !collapsed) logEl.scrollTop = logEl.scrollHeight;
      });
  });

  function toggle() {
    collapsed = !collapsed;
    localStorage.setItem(CKEY, collapsed ? '1' : '0');
  }

  function onResizeStart(e: PointerEvent) {
    resizing = true;
    const startY = e.clientY;
    const startH = height;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const max = Math.round(window.innerHeight * 0.6);
      height = Math.min(Math.max(startH + (startY - ev.clientY), 120), max);
    };
    const up = (ev: PointerEvent) => {
      resizing = false;
      target.releasePointerCapture(ev.pointerId);
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      localStorage.setItem(HKEY, String(height));
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
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
        onclick={onClear}
        disabled={!!running}
        title={t('console.clearHint')}
      >
        {t('common.clear')}
      </button>
    </div>
  </header>

  {#if !collapsed}
    {#if log.length}
      <div bind:this={logEl} class="log" style="height:{height}px">
        {#each log as line}
          <div
            class="log-line"
            class:log-warn={line.startsWith('⚠')}
            class:log-diag={line.startsWith('[diag]')}
            class:log-ok={line.startsWith('✓')}
          >{line}</div>
        {/each}
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
    gap: 6px;
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
    color: var(--sw-warn);
  }
  .log-diag {
    color: var(--sw-text-dimmed);
  }
  .log-ok {
    color: var(--sw-success);
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
