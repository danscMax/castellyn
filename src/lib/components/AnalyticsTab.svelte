<script lang="ts">
  import { readFreellmapiAnalytics, type FreellmapiAnalytics } from '$lib/ipc';
  import { t } from '$lib/i18n';

  // Range presets (hours). Self-contained: this tab owns its fetch + range state.
  const ranges = [
    { hours: 1, key: 'analytics.range1h' },
    { hours: 24, key: 'analytics.range24h' },
    { hours: 168, key: 'analytics.range7d' },
    { hours: 720, key: 'analytics.range30d' }
  ];

  let rangeHours = $state(168);
  let data = $state<FreellmapiAnalytics | null>(null);
  let loading = $state(false);
  let loaded = '';

  async function load(h: number) {
    loading = true;
    const token = `${h}:${Date.now()}`;
    loaded = token;
    try {
      const r = await readFreellmapiAnalytics(h);
      if (loaded === token) data = r;
    } catch {
      if (loaded === token) data = null;
    } finally {
      if (loaded === token) loading = false;
    }
  }

  // Load on mount and whenever the range changes.
  $effect(() => {
    const h = rangeHours;
    load(h);
  });

  const nf = new Intl.NumberFormat();
  const fmt = (n: number) => nf.format(n ?? 0);
  const totals = $derived(data?.totals);
  const models = $derived(data?.perModel ?? []);
  const available = $derived(!!data?.available);
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('analytics.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('analytics.subtitle')}</p>
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      <div class="flex gap-1">
        {#each ranges as r (r.hours)}
          <button
            class="sw-btn sw-btn-ghost text-sw-xs {rangeHours === r.hours ? 'is-active' : ''}"
            aria-pressed={rangeHours === r.hours}
            disabled={loading}
            onclick={() => (rangeHours = r.hours)}>{t(r.key)}</button>
        {/each}
      </div>
      <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={loading} onclick={() => load(rangeHours)}
        title={t('analytics.refreshTip')}>
        {loading ? t('analytics.loading') : t('analytics.refresh')}
      </button>
    </div>
  </header>

  {#if !available}
    <div class="sw-card text-sw-sm text-sw-text-muted">
      {loading ? t('analytics.loading') : t('analytics.empty')}
    </div>
  {:else}
    <!-- Totals -->
    <div class="card-grid mb-sw-6">
      <div class="sw-card">
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.totalRequests')}</p>
        <p class="mt-1 text-2xl font-semibold">{fmt(totals?.totalRequests ?? 0)}</p>
        <p class="mt-1 text-sw-xs text-sw-text-secondary">
          {t('analytics.successRate')}: {totals?.successRate ?? 0}%
        </p>
      </div>
      <div class="sw-card">
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.tokens')}</p>
        <p class="mt-1 text-2xl font-semibold">{fmt((totals?.totalInputTokens ?? 0) + (totals?.totalOutputTokens ?? 0))}</p>
        <p class="mt-1 text-sw-xs text-sw-text-secondary">
          {t('analytics.in')} {fmt(totals?.totalInputTokens ?? 0)} · {t('analytics.out')} {fmt(totals?.totalOutputTokens ?? 0)}
        </p>
      </div>
      <div class="sw-card">
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.avgLatency')}</p>
        <p class="mt-1 text-2xl font-semibold">{fmt(totals?.avgLatencyMs ?? 0)}<span class="text-sw-sm font-normal"> ms</span></p>
      </div>
      <div class="sw-card">
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.savings')}</p>
        <p class="mt-1 text-2xl font-semibold text-emerald-500">${totals?.estimatedCostSavings ?? 0}</p>
        <p class="mt-1 text-sw-xs text-sw-text-secondary">{t('analytics.savingsHint')}</p>
      </div>
    </div>

    <!-- Per-model -->
    <h2 class="mb-sw-2 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">
      {t('analytics.perModel')}
    </h2>
    {#if models.length}
      <div class="overflow-x-auto rounded-sw-md border border-sw-border">
        <table class="w-full text-sw-sm">
          <thead>
            <tr class="border-b border-sw-border text-left text-sw-xs text-sw-text-muted">
              <th class="px-sw-3 py-sw-2 font-medium">{t('analytics.colModel')}</th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">{t('analytics.colRequests')}</th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">{t('analytics.colSuccess')}</th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">{t('analytics.colLatency')}</th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">{t('analytics.colTokens')}</th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">{t('analytics.colCost')}</th>
            </tr>
          </thead>
          <tbody>
            {#each models as m (m.platform + '/' + m.modelId)}
              <tr class="border-b border-sw-border last:border-0">
                <td class="px-sw-3 py-sw-2">
                  <div class="truncate font-medium" title={m.displayName}>{m.displayName}</div>
                  <div class="truncate font-mono text-[11px] text-sw-text-muted">{m.platform}/{m.modelId}</div>
                </td>
                <td class="px-sw-3 py-sw-2 text-right">{fmt(m.requests)}</td>
                <td class="px-sw-3 py-sw-2 text-right">{m.successRate}%</td>
                <td class="px-sw-3 py-sw-2 text-right">{fmt(m.avgLatencyMs)} ms</td>
                <td class="px-sw-3 py-sw-2 text-right">{fmt(m.totalInputTokens + m.totalOutputTokens)}</td>
                <td class="px-sw-3 py-sw-2 text-right">${m.estimatedCost}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="sw-card text-sw-sm text-sw-text-muted">{t('analytics.noModels')}</div>
    {/if}

    <p class="mt-sw-4 text-sw-xs text-sw-text-muted">{t('analytics.footnote')}</p>
  {/if}
</div>

<style>
  .is-active {
    border-color: var(--sw-border-focus);
    color: var(--sw-text-primary);
  }
  table th {
    position: sticky;
    top: 0;
    background: var(--sw-bg-secondary);
  }
</style>
