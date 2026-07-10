<script lang="ts">
  import { toastStore, markNotifRead, clearHistory, dismissFromHistory } from '$lib/toast.svelte';
  import { t } from '$lib/i18n';
  import { anchored } from '$lib/floating';
  import EmptyState from './EmptyState.svelte';
  import { Check, TriangleAlert, X, Info, BellOff } from '@lucide/svelte';
  import type { Component } from 'svelte';

  // `anchor` is the bell button (bottom of the sidebar); the panel pins to it via use:anchored,
  // opening above since there's no room below. Falls back to hidden until the anchor is mounted.
  let { open = false, onClose, anchor }: { open: boolean; onClose: () => void; anchor?: HTMLElement } = $props();

  let history = $derived(toastStore.history);

  $effect(() => {
    if (open) markNotifRead();
  });

  // V6: SVG icons (one icon language) instead of the ✓⚠✗ℹ text glyphs.
  const kindIcon: Record<string, Component> = {
    success: Check,
    warn: TriangleAlert,
    error: X,
    info: Info
  };

  function fmtRel(ts: number): string {
    const diff = Date.now() - ts;
    if (diff < 60000) return t('common.justNow');
    if (diff < 3600000) return t('common.minutesAgo', { n: Math.floor(diff / 60000) });
    if (diff < 86400000) return t('common.hoursAgo', { n: Math.floor(diff / 3600000) });
    return t('common.daysAgo', { n: Math.floor(diff / 86400000) });
  }
</script>

<!-- U2: Escape closes the panel (click-outside already works via the backdrop). -->
<svelte:window onkeydown={(e) => e.key === 'Escape' && open && onClose()} />

{#if open && anchor}
  <div class="panel" role="dialog" aria-label={t('page.notifTitle')}
    use:anchored={{ anchor, onOutside: onClose }}>
      <header class="head">
        <h2 class="title">{t('page.notifTitle')}</h2>
        <div class="acts">
          {#if history.items.length}
            <button class="clear-btn" onclick={clearHistory}>{t('page.notifDismissAll')}</button>
          {/if}
          <button class="close-btn" onclick={onClose} aria-label={t('common.close')}>×</button>
        </div>
      </header>
      {#if history.items.length === 0}
        <EmptyState icon={BellOff} description={t('page.notifEmpty')} />
      {:else}
        <div class="list">
          <!-- Keyed by id, never timestamp: dismissAll() pushes the whole stack to history in one
               synchronous loop, so several entries share a Date.now() and Svelte throws on the
               duplicate key. `id` is unique per entry and resumes above the restored history. -->
          {#each history.items as item, i (item.id)}
            {@const Icon = kindIcon[item.kind]}
            <div class="entry {item.kind}">
              <span class="icon" class:icon-success={item.kind === 'success'} class:icon-warn={item.kind === 'warn'} class:icon-error={item.kind === 'error'} class:icon-info={item.kind === 'info'}><Icon size={11} aria-hidden="true" /></span>
              <div class="body">
                <div class="entry-title">{item.title}{#if (item.count ?? 1) > 1}<span class="entry-cnt" title={t('common.repeated')}>×{item.count}</span>{/if}</div>
                {#if item.detail}<div class="entry-detail">{item.detail}</div>{/if}
                <div class="entry-time" title={new Date(item.timestamp).toLocaleString()}>{fmtRel(item.timestamp)}</div>
              </div>
              <button class="entry-x" onclick={() => dismissFromHistory(item.timestamp)} aria-label={t('common.close')}>×</button>
            </div>
          {/each}
        </div>
      {/if}
  </div>
{/if}

<style>
  .panel {
    /* position/top/left set inline by use:anchored (fixed, pins above the sidebar bell). */
    position: fixed;
    z-index: 60;
    width: 380px;
    max-height: min(480px, 80vh);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    border-bottom: 1px solid var(--sw-border);
  }
  .title {
    font-size: var(--sw-text-sm);
    font-weight: 600;
    color: var(--sw-text-primary);
    margin: 0;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .clear-btn {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    background: transparent;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm);
    padding: 3px 8px;
    cursor: pointer;
  }
  .clear-btn:hover {
    color: var(--sw-text-primary);
    border-color: var(--sw-border-focus, #64748b);
  }
  .close-btn {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    font-size: 20px;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
  }
  .close-btn:hover { color: var(--sw-text-primary); }
  .list {
    overflow-y: auto;
    flex: 1;
  }
  .entry {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--sw-border);
  }
  .entry:last-child { border-bottom: none; }
  .icon {
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    font-size: var(--sw-text-xs);
    font-weight: 700;
  }
  .icon-success { background: color-mix(in srgb, var(--sw-success) 20%, transparent); color: var(--sw-success); }
  .icon-warn { background: color-mix(in srgb, var(--sw-warn) 20%, transparent); color: var(--sw-warn); }
  .icon-error { background: color-mix(in srgb, var(--sw-danger) 20%, transparent); color: var(--sw-danger); }
  .icon-info { background: color-mix(in srgb, #38bdf8 20%, transparent); color: #38bdf8; }
  .body { min-width: 0; flex: 1; }
  .entry-title {
    font-size: var(--sw-text-sm);
    font-weight: 500;
    color: var(--sw-text-primary);
    word-break: break-word;
  }
  .entry-cnt {
    margin-left: 6px;
    font-size: var(--sw-text-xs);
    font-weight: 600;
    color: var(--sw-text-secondary);
    background: var(--sw-bg-tertiary, rgba(255, 255, 255, 0.06));
    border: 1px solid var(--sw-border);
    border-radius: 999px;
    padding: 0 6px;
  }
  .entry-detail {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    margin-top: 2px;
    word-break: break-word;
  }
  .entry-time {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    margin-top: 4px;
  }
  .entry-x {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    font-size: 16px;
    padding: 0;
    line-height: 1;
    opacity: 0;
    flex-shrink: 0;
  }
  /* Keyboard parity with the hover reveal: a Tab-focused ✕ must be visible too. */
  .entry:hover .entry-x,
  .entry:focus-within .entry-x,
  .entry-x:focus-visible { opacity: 1; }
  .entry-x:hover { color: var(--sw-text-primary); }
</style>