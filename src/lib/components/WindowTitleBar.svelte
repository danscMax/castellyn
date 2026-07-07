<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import { base } from '$app/paths';
  import { t } from '$lib/i18n';
  import { runningStore, opName } from '$lib/running.svelte';
  import { navHistory, navGo } from '$lib/navHistory.svelte';
  import { agentSummary } from '$lib/agentStatus.svelte';
  import { peakUtilization } from '$lib/limits.svelte';
  import { uiPrefs } from '$lib/uiPrefs.svelte';
  import { readConfig } from '$lib/ipc';

  const appWin = getCurrentWindow();

  // Wave C-5: native session-status strip. Fed by data Castellyn already tracks (agent_status counts
  // + the limits poll) — the in-terminal statusline stays owned by Claude Code; this only surfaces a
  // parallel, always-visible glance. Initialized from config here (the title bar is always mounted);
  // the Settings toggle updates uiPrefs live. Auto-hides when there's nothing to show.
  const peak = $derived(peakUtilization());
  const sessLive = $derived(agentSummary.working + agentSummary.blocked);
  const showStrip = $derived(
    uiPrefs.loaded &&
      uiPrefs.showSessionStatusBar &&
      (sessLive > 0 || agentSummary.done > 0 || agentSummary.limited > 0 || peak != null)
  );
  const peakClass = $derived(
    !peak ? '' : peak.pct >= 99 ? 'strip-err' : peak.pct >= 85 ? 'strip-warn' : ''
  );

  // Track maximized state so the caption button shows the correct glyph (single square =
  // maximize, overlapping squares = restore) — Windows convention.
  let maximized = $state(false);
  const syncMax = () => appWin.isMaximized().then((v) => (maximized = v)).catch(() => {});
  // V14: dim the custom chrome when the window loses focus (native Windows titlebars do) —
  // otherwise an inactive window looks active. Tauri's focus event covers OS-level focus.
  let winFocused = $state(true);
  onMount(() => {
    syncMax();
    // Seed the shared pref from config once; the Settings toggle keeps it live thereafter.
    readConfig()
      .then((c) => (uiPrefs.showSessionStatusBar = c.showSessionStatusBar ?? true))
      .catch(() => {})
      .finally(() => (uiPrefs.loaded = true));
    let unlisten: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;
    appWin.onResized(syncMax).then((u) => (unlisten = u)).catch(() => {});
    appWin
      .onFocusChanged(({ payload }) => (winFocused = payload))
      .then((u) => (unlistenFocus = u))
      .catch(() => {});
    return () => {
      unlisten?.();
      unlistenFocus?.();
    };
  });

  async function minimize() {
    await appWin.minimize();
  }
  async function toggleMaximize() {
    if (await appWin.isMaximized()) {
      await appWin.unmaximize();
    } else {
      await appWin.maximize();
    }
    syncMax();
  }
  async function close() {
    // Window CloseRequested is intercepted in Rust → hides to tray.
    await appWin.close();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="titlebar" class:inactive={!winFocused} data-tauri-drag-region ondblclick={toggleMaximize}>
  <!-- Browser-style tab history — mirrors mouse buttons 3/4 and Alt+←/→ (handlers in +page). -->
  <div class="nav">
    <button class="tb-btn tb-nav" onclick={() => navGo('back')} disabled={navHistory.back.length === 0}
      aria-label={t('titlebar.navBack')} title={t('titlebar.navBack')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><path d="M7.5 2.5 L4 6 L7.5 9.5" /></svg>
    </button>
    <button class="tb-btn tb-nav" onclick={() => navGo('fwd')} disabled={navHistory.fwd.length === 0}
      aria-label={t('titlebar.navForward')} title={t('titlebar.navForward')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><path d="M4.5 2.5 L8 6 L4.5 9.5" /></svg>
    </button>
  </div>
  <div class="brand" data-tauri-drag-region>
    <img class="logo" src="{base}/favicon.png" alt="" data-tauri-drag-region width="18" height="18" />
    <span class="title" data-tauri-drag-region>{t('titlebar.title')}</span>
    {#if runningStore.op}
      <!-- V13: the indicator is not interactive — keep it a drag region like the rest of the bar -->
      <span class="running" data-tauri-drag-region title={opName(runningStore.op)}>
        <span class="running-dot" data-tauri-drag-region></span>
        <span class="running-label" data-tauri-drag-region>{opName(runningStore.op)}</span>
      </span>
    {/if}
  </div>

  {#if showStrip}
    <!-- Session-status strip: live counts + the profile closest to an Anthropic limit. Drag region
         like the rest of the bar (not interactive). -->
    <div class="sess-strip {peakClass}" data-tauri-drag-region title={t('titlebar.stripHint')}>
      {#if agentSummary.working}
        <span class="ss-item" data-tauri-drag-region><span class="ss-dot ss-work" data-tauri-drag-region></span>{agentSummary.working}</span>
      {/if}
      {#if agentSummary.blocked}
        <span class="ss-item" data-tauri-drag-region><span class="ss-dot ss-block" data-tauri-drag-region></span>{agentSummary.blocked}</span>
      {/if}
      {#if agentSummary.limited}
        <span class="ss-item" data-tauri-drag-region><span class="ss-dot ss-lim" data-tauri-drag-region></span>{agentSummary.limited}</span>
      {/if}
      {#if peak}
        <span class="ss-item ss-peak" data-tauri-drag-region title={t('titlebar.stripPeak', { profile: peak.profile })}>
          {peak.window} {Math.round(peak.pct)}%
        </span>
      {/if}
    </div>
  {/if}

  {#if runningStore.op}
    <div class="tb-progress"></div>
  {/if}

  <div class="controls">
    <button class="tb-btn" onclick={minimize} aria-label={t('titlebar.minimize')} title={t('titlebar.minimize')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><line x1="2" y1="6.5" x2="10" y2="6.5" /></svg>
    </button>
    <button
      class="tb-btn"
      onclick={toggleMaximize}
      aria-label={maximized ? t('titlebar.restore') : t('titlebar.maximize')}
      title={maximized ? t('titlebar.restore') : t('titlebar.maximize')}
    >
      {#if maximized}
        <svg viewBox="0 0 12 12" width="11" height="11">
          <path d="M3.5 3.5 V1.5 H10.5 V8.5 H8.5" />
          <rect x="1.5" y="3.5" width="7" height="7" />
        </svg>
      {:else}
        <svg viewBox="0 0 12 12" width="11" height="11"><rect x="2.5" y="2.5" width="7" height="7" /></svg>
      {/if}
    </button>
    <button class="tb-btn tb-close" onclick={close} aria-label={t('titlebar.close')} title={t('titlebar.close')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" /></svg>
    </button>
  </div>
</div>

<style>
  .titlebar {
    position: relative;
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--sw-sidebar-bg, var(--sw-bg-secondary));
    border-bottom: 1px solid var(--sw-border);
    user-select: none;
  }
  .nav {
    display: flex;
    height: 100%;
    padding-left: 2px;
  }
  .tb-nav {
    width: 36px;
  }
  .tb-nav:disabled {
    color: var(--sw-text-muted);
    opacity: 0.45;
    cursor: default;
    background: transparent;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-left: 8px;
    height: 100%;
    flex: 1;
    min-width: 0;
  }
  .logo {
    width: 18px;
    height: 18px;
    border-radius: 5px;
    flex-shrink: 0;
    object-fit: contain;
  }
  /* V14: inactive-window chrome dims like a native Windows titlebar. */
  .titlebar.inactive .title,
  .titlebar.inactive :global(.running) {
    color: var(--sw-text-muted);
  }
  .titlebar.inactive .logo {
    opacity: 0.6;
  }
  .title {
    font-size: var(--sw-text-xs);
    font-weight: 600;
    color: var(--sw-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* "What's running now" indicator — a pulsing dot + the operation name next to the title. */
  .running {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 10px;
    padding: 2px 8px;
    border-radius: 9999px;
    background: var(--sw-accent-glow);
    font-size: var(--sw-text-xs);
    color: var(--sw-accent-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .running-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--sw-accent);
    flex-shrink: 0;
    animation: tb-pulse 1.2s ease-in-out infinite;
  }
  @keyframes tb-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.3;
    }
  }
  .running-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Wave C-5: session-status strip — compact, sits between the brand and the caption buttons. */
  .sess-strip {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    margin-right: 10px;
    padding: 2px 8px;
    border-radius: 9999px;
    background: var(--sw-bg-secondary);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .sess-strip.strip-warn { color: var(--sw-warn); }
  .sess-strip.strip-err { color: var(--sw-err, #ef4444); }
  .ss-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-variant-numeric: tabular-nums;
  }
  .ss-peak {
    font-weight: 600;
  }
  .ss-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .ss-work { background: var(--sw-ok, #22c55e); }
  .ss-block { background: var(--sw-warn, #f59e0b); }
  /* Same steady red as TerminalPane's `.dot.limited` — "stopped on quota". */
  .ss-lim { background: var(--sw-status-down, #ef4444); }
  .titlebar.inactive .sess-strip { opacity: 0.6; }
  .controls {
    display: flex;
    height: 100%;
  }
  .tb-btn {
    width: 46px;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: background-color 0.12s, color 0.12s;
  }
  .tb-btn svg {
    stroke: currentColor;
    stroke-width: 1.2;
    fill: none;
  }
  .tb-btn:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  .tb-close:hover {
    background: #e81123;
    color: #fff;
  }
  .tb-btn:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--sw-accent);
  }
  .tb-progress {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--sw-accent-glow, rgba(59,130,246,0.3));
    overflow: hidden;
  }
  .tb-progress::after {
    content: '';
    position: absolute;
    inset: 0;
    width: 40%;
    background: var(--sw-accent);
    animation: tb-progress-indeterminate 1.4s ease-in-out infinite;
  }
  @keyframes tb-progress-indeterminate {
    0%   { left: -40%; }
    100% { left: 100%; }
  }
</style>
