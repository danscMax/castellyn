<script lang="ts">
  import { readStackHealth, type StackHealth } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { statusFillVar } from '$lib/statusColor';
  import { RefreshCw } from '@lucide/svelte';
  import { listen } from '@tauri-apps/api/event';

  // onStart lets a stopped service be brought up straight from the health list (id → start).
  let { onStart, busy = false }: { onStart?: (id: string) => void; busy?: boolean } = $props();

  let items = $state<StackHealth[]>([]);
  let loading = $state(false);
  let loadedOnce = $state(false);

  async function load() {
    loading = true;
    try {
      items = await readStackHealth();
    } catch {
      items = [];
    } finally {
      loading = false;
      loadedOnce = true;
    }
  }
  $effect(() => {
    load();
    // Live updates from the backend health-poll loop — no manual refresh needed.
    const un = listen<StackHealth[]>('stack-health', (e) => {
      items = e.payload;
      loadedOnce = true;
    });
    return () => {
      un.then((f) => f());
    };
  });

  // up = HTTP-healthy or (port-only) just listening; degraded = listening but health failed;
  // down = port closed. A closed port is usually a deliberately-stopped backend, NOT a fault —
  // only the gateway (the critical hop) being down is treated as an alarm.
  function statusOf(s: StackHealth): 'up' | 'degraded' | 'down' {
    if (!s.portOpen) return 'down';
    if (s.healthy === false) return 'degraded';
    return 'up';
  }
  // Status fill colours come from the shared source (lib/statusColor.ts); these saturated
  // --sw-status-* tokens read fine on both themes (a small dot has no 4.5:1 requirement).
  const dot = {
    up: statusFillVar('up'),
    degraded: statusFillVar('degraded'),
    down: statusFillVar('down'),
    off: statusFillVar('off')
  } as const;
  // Stopped non-gateway backends are neutral grey; a dead gateway / sick service keep alarm colours.
  function dotColor(s: StackHealth): string {
    const st = statusOf(s);
    if (st === 'down') return s.id === 'gateway' ? dot.down : dot.off;
    return dot[st];
  }

  let showDetails = $state(false);

  const enabled = $derived(items.filter((i) => i.enabled));
  const ups = $derived(enabled.filter((i) => statusOf(i) === 'up').length);
  const total = $derived(enabled.length);
  const gateway = $derived(items.find((i) => i.id === 'gateway'));
  const anySick = $derived(enabled.some((i) => statusOf(i) === 'degraded'));
  // ok=all up · degraded=a service is sick (port open, /health fails) · stopped=some backends are
  // just off (normal) · down=gateway unreachable (the only real outage).
  const overall = $derived<'ok' | 'degraded' | 'stopped' | 'down'>(
    total === 0
      ? 'down'
      : gateway && statusOf(gateway) !== 'up'
        ? 'down'
        : anySick
          ? 'degraded'
          : ups === total
            ? 'ok'
            : 'stopped'
  );
  const overallColor = $derived(
    overall === 'ok' ? dot.up : overall === 'degraded' ? dot.degraded : overall === 'stopped' ? dot.off : dot.down
  );
  const overallLabel = $derived(
    overall === 'ok'
      ? t('health.ovOk')
      : overall === 'degraded'
        ? t('health.ovDegraded')
        : overall === 'stopped'
          ? t('health.ovStopped')
          : t('health.ovDown')
  );
  function svcLabel(s: StackHealth): string {
    const st = statusOf(s);
    if (st === 'down') return t('health.svDown');
    if (st === 'degraded') return t('health.svDegraded');
    return s.healthy === null ? t('health.svPortOnly') : t('health.svUp');
  }
</script>

<section class="sw-card mb-sw-6">
  <div class="flex items-start justify-between gap-sw-3">
    <div class="flex items-center gap-sw-3">
      <span class="big-dot" style="background:{overallColor}" aria-hidden="true"></span>
      <div>
        <h2 class="font-semibold">{t('health.title')}</h2>
        <p class="text-sw-xs text-sw-text-secondary">
          <!-- V10: separator built in the expression — template whitespace before `·` got collapsed -->
          {overallLabel}{total > 0 ? ` · ${t('health.summary', { up: ups, total })}` : ''}
        </p>
      </div>
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      {#if enabled.length}
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (showDetails = !showDetails)}
          title={t('health.refreshTip')}>
          {showDetails ? t('health.hide') : t('health.details')} {showDetails ? '▴' : '▾'}
        </button>
      {/if}
      <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={loading} onclick={load}
        title={t('health.refresh') + ' — ' + t('health.refreshTip')} aria-label={t('health.refresh')}>
        {#if loading}{t('health.loading')}{:else}<RefreshCw size={14} />{/if}
      </button>
    </div>
  </div>

  {#if enabled.length && showDetails}
    <div class="mt-sw-3 flex flex-wrap gap-x-sw-4 gap-y-sw-2 border-t border-sw-border pt-sw-3">
      {#each enabled as s (s.id)}
        <div class="flex items-center gap-sw-2" title="{s.name} :{s.port}">
          <span class="dot" style="background:{dotColor(s)}" aria-hidden="true"></span>
          <span class="text-sw-sm">{s.name}</span>
          <span class="font-mono text-[11px] text-sw-text-muted">:{s.port}</span>
          <span class="text-sw-xs text-sw-text-secondary">· {svcLabel(s)}</span>
          {#if onStart && statusOf(s) === 'down'}
            <button class="sw-btn sw-btn-ghost text-[11px]" disabled={busy} onclick={() => onStart?.(s.id)}
              title={t('health.startTip', { name: s.name })}>{t('health.start')}</button>
          {/if}
        </div>
      {/each}
    </div>
  {:else if loadedOnce && !loading && !enabled.length}
    <p class="mt-sw-2 text-sw-xs text-sw-text-muted">{t('health.empty')}</p>
  {/if}
</section>

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
