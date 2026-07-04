<script lang="ts">
  import { pluginSyncSet, runManagedDeploy, type StackDriftItem } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { statusFillVar } from '$lib/statusColor';
  import { RefreshCw } from '@lucide/svelte';

  // Ф1 reconcile: "does Castellyn own the stack?" card. Prop-driven — the parent owns the drift
  // data (so the sidebar badge reads the same array); this card renders it, runs the repair IPC,
  // then asks the parent to reload. Mirrors StackHealthCard's markup on purpose.
  let { items = null, onReload, busy = false }:
    { items: StackDriftItem[] | null; onReload?: () => void | Promise<void>; busy?: boolean } = $props();

  // Non-null while a button is spinning: '*' = the refresh button, else the item id being fixed.
  let working = $state<string | null>(null);

  const rows = $derived(items ?? []);
  const bad = $derived(rows.filter((r) => r.state !== 'ok').length);
  const anyErr = $derived(rows.some((r) => r.state === 'error'));

  const dot = { up: statusFillVar('up'), degraded: statusFillVar('degraded'), down: statusFillVar('down') } as const;
  function dotColor(state: StackDriftItem['state']): string {
    if (state === 'ok') return dot.up;
    if (state === 'error') return dot.down;
    return dot.degraded; // drift | missing
  }
  const overallColor = $derived(bad === 0 ? dot.up : anyErr ? dot.down : dot.degraded);
  const overallLabel = $derived(bad === 0 ? t('page.home_ok') : t('page.home_issues', { n: bad }));

  function labelOf(id: StackDriftItem['id']): string {
    if (id === 'plugin_sync_file') return t('page.home_drift_label_file');
    if (id === 'plugin_sync_wiring') return t('page.home_drift_label_wiring');
    return t('page.home_drift_label_managed');
  }
  function fixLabel(fix: string): string {
    return fix === 'managed_deploy' ? t('page.home_drift_fix_deploy') : t('page.home_drift_fix_own');
  }

  async function doFix(item: StackDriftItem) {
    if (!item.fix) return;
    working = item.id;
    try {
      if (item.fix === 'plugin_sync') await pluginSyncSet(true);
      else await runManagedDeploy();
      await onReload?.();
    } finally {
      working = null;
    }
  }
  async function refresh() {
    working = '*';
    try {
      await onReload?.();
    } finally {
      working = null;
    }
  }
</script>

{#if rows.length}
  <section class="sw-card mb-sw-6">
    <div class="flex items-start justify-between gap-sw-3">
      <div class="flex items-center gap-sw-3">
        <span class="big-dot" style="background:{overallColor}" aria-hidden="true"></span>
        <div>
          <h2 class="font-semibold">{t('page.home_drift_title')}</h2>
          <p class="text-sw-xs text-sw-text-secondary">{overallLabel}</p>
        </div>
      </div>
      <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={!!working} onclick={refresh}
        title={t('common.refresh')} aria-label={t('common.refresh')}>
        {#if working === '*'}{t('common.loading')}{:else}<RefreshCw size={14} />{/if}
      </button>
    </div>

    <div class="mt-sw-3 flex flex-col gap-sw-2 border-t border-sw-border pt-sw-3">
      {#each rows as r (r.id)}
        <div class="flex flex-wrap items-center gap-sw-2">
          <span class="dot" style="background:{dotColor(r.state)}" aria-hidden="true"></span>
          <span class="text-sw-sm font-medium">{labelOf(r.id)}</span>
          {#if r.detail}<span class="text-sw-xs text-sw-text-secondary">· {r.detail}</span>{/if}
          {#if r.fix}
            <button class="sw-btn sw-btn-ghost text-sw-xs ml-auto shrink-0" disabled={!!working || busy}
              onclick={() => doFix(r)}
              title={r.fix === 'managed_deploy' ? t('page.home_drift_deploy_hint') : undefined}>
              {#if working === r.id}{t('common.loading')}{:else}{fixLabel(r.fix)}{/if}
            </button>
          {/if}
        </div>
      {/each}
    </div>
  </section>
{/if}

<style>
  .big-dot {
    width: 14px;
    height: 14px;
    border-radius: 9999px;
    box-shadow: 0 0 10px currentColor;
    flex-shrink: 0;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 9999px;
    flex-shrink: 0;
  }
</style>
