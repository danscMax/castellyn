<script lang="ts">
  import { readFreellmapiAnalytics, type FreellmapiAnalytics, type AnalyticsModel } from '$lib/ipc';
  import { t, locale } from '$lib/i18n';
  import Sparkline from './Sparkline.svelte';

  let { onOpenProviders }: { onOpenProviders?: () => void } = $props();

  // Stacked-bar / legend colours, cycled by model index.
  const SEG_COLORS = ['#3b82f6', '#10b981', '#f59e0b', '#ef4444', '#8b5cf6', '#ec4899', '#14b8a6', '#f97316'];

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

  // Row interactivity: a selected model re-scopes the totals + the trend; column sort orders the table.
  let selectedKey = $state<string | null>(null);
  type SortKey = 'requests' | 'successRate' | 'avgLatencyMs' | 'tokens' | 'estimatedCost';
  let sortKey = $state<SortKey>('requests');
  let sortDir = $state<'asc' | 'desc'>('desc');

  const keyOf = (m: { platform: string; modelId: string }) => `${m.platform}/${m.modelId}`;

  // Per-range cache so switching 1h↔24h↔7d↔30d is instant within the TTL; "Refresh" forces a fetch.
  const cache = new Map<number, { ts: number; data: FreellmapiAnalytics }>();
  const CACHE_TTL = 60_000;
  async function load(h: number, force = false) {
    const hit = cache.get(h);
    if (!force && hit && Date.now() - hit.ts < CACHE_TTL) {
      data = hit.data;
      return;
    }
    loading = true;
    const token = `${h}:${Date.now()}`;
    loaded = token;
    try {
      const r = await readFreellmapiAnalytics(h);
      if (loaded === token) {
        data = r;
        cache.set(h, { ts: Date.now(), data: r });
      }
    } catch {
      if (loaded === token) data = null;
    } finally {
      if (loaded === token) loading = false;
    }
  }

  // Load on mount and whenever the range changes; a range switch clears any model filter
  // (the selected model may not exist in the new window).
  $effect(() => {
    const h = rangeHours;
    selectedKey = null;
    load(h);
  });

  // Locale-aware formatters (re-derive on language change) — numbers, currency, percent.
  const fmtLocale = $derived(
    locale.current === 'ru' ? 'ru-RU' : locale.current === 'zh' ? 'zh-CN' : 'en-US'
  );
  const nf = $derived(new Intl.NumberFormat(fmtLocale));
  const cf = $derived(new Intl.NumberFormat(fmtLocale, { style: 'currency', currency: 'USD', maximumFractionDigits: 2 }));
  const fmt = (n: number) => nf.format(n ?? 0);
  const money = (n: number) => cf.format(n ?? 0);
  const pct = (n: number) => `${(n ?? 0).toFixed(1)}%`;

  // Export the per-model table to a CSV file (client-side blob download; no backend).
  function exportCsv() {
    const head = ['model', 'platform', 'modelId', 'requests', 'successRate', 'avgLatencyMs', 'tokens', 'estimatedCost'];
    const esc = (c: string) => (/[",\n]/.test(c) ? `"${c.replace(/"/g, '""')}"` : c);
    const rows = [head.join(',')];
    for (const m of models) {
      rows.push(
        [m.displayName, m.platform, m.modelId, m.requests, m.successRate, m.avgLatencyMs, m.totalInputTokens + m.totalOutputTokens, m.estimatedCost]
          .map((c) => esc(String(c)))
          .join(',')
      );
    }
    const blob = new Blob([rows.join('\n')], { type: 'text/csv;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `castellyn-analytics-${rangeHours}h.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }
  const available = $derived(!!data?.available);
  const models = $derived(data?.perModel ?? []);

  const selectedModel = $derived(
    selectedKey ? (models.find((m) => keyOf(m) === selectedKey) ?? null) : null
  );

  // Totals cards reflect the selected model when one is picked — derived entirely from its
  // already-fetched per-model row (savings = its estimated cost), so no extra backend call.
  const totals = $derived.by(() => {
    const t0 = data?.totals;
    const m = selectedModel;
    if (!m) return t0;
    return {
      totalRequests: m.requests,
      successRate: m.successRate,
      totalInputTokens: m.totalInputTokens,
      totalOutputTokens: m.totalOutputTokens,
      avgLatencyMs: m.avgLatencyMs,
      estimatedCostSavings: m.estimatedCost,
      firstRequestAt: t0?.firstRequestAt ?? null
    };
  });

  // Sorted table rows.
  const valOf = (m: AnalyticsModel, k: SortKey) =>
    k === 'tokens' ? m.totalInputTokens + m.totalOutputTokens : m[k];
  const sortedModels = $derived.by(() => {
    const dir = sortDir === 'asc' ? 1 : -1;
    return [...models].sort((a, b) => (valOf(a, sortKey) - valOf(b, sortKey)) * dir);
  });
  function toggleSort(k: SortKey) {
    if (sortKey === k) sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    else {
      sortKey = k;
      sortDir = 'desc';
    }
  }
  const sortArrow = (k: SortKey) => (sortKey === k ? (sortDir === 'asc' ? ' ▲' : ' ▼') : '');

  function toggleSelect(m: AnalyticsModel) {
    const k = keyOf(m);
    selectedKey = selectedKey === k ? null : k;
  }

  // Trend: zero-filled requests-per-bucket over a stable axis (global min..max bucket), optionally
  // scoped to the selected model. Axis is global so the shape stays put when filtering.
  const trend = $derived.by(() => {
    const series = data?.series ?? [];
    const step = data?.stepSec ?? 0;
    if (!series.length || step <= 0) return [] as number[];
    const buckets = series.map((s) => s.bucket);
    const lo = Math.min(...buckets);
    const hi = Math.max(...buckets);
    const n = Math.floor((hi - lo) / step) + 1;
    if (n <= 0 || n > 1000) return [];
    const sums = new Array(n).fill(0);
    for (const s of series) {
      if (selectedKey && keyOf(s) !== selectedKey) continue;
      const idx = Math.round((s.bucket - lo) / step);
      if (idx >= 0 && idx < n) sums[idx] += s.requests;
    }
    return sums;
  });

  // Hover labels for the sparkline: "<bucket time> · <N>" per bucket (#22). Mirrors trend's axis.
  const tf = $derived.by(() => {
    const step = data?.stepSec ?? 0;
    const opts: Intl.DateTimeFormatOptions =
      step >= 86400 ? { month: 'short', day: 'numeric' } : { hour: '2-digit', minute: '2-digit' };
    return new Intl.DateTimeFormat(fmtLocale, opts);
  });
  const trendLabels = $derived.by(() => {
    const series = data?.series ?? [];
    const step = data?.stepSec ?? 0;
    if (!series.length || step <= 0) return [] as string[];
    const lo = Math.min(...series.map((s) => s.bucket));
    return trend.map((v, i) => `${tf.format(new Date((lo + i * step) * 1000))} · ${fmt(v)}`);
  });

  // Top-N insight (#112): most expensive + most frequent models, derived client-side.
  const topCost = $derived(
    [...models].filter((m) => m.estimatedCost > 0).sort((a, b) => b.estimatedCost - a.estimatedCost).slice(0, 3)
  );
  const topReq = $derived([...models].sort((a, b) => b.requests - a.requests).slice(0, 3));

  // Cost breakdown by model as a stacked bar (#23). Hidden when everything is free (cost 0).
  const totalCost = $derived(models.reduce((s, m) => s + (m.estimatedCost ?? 0), 0));
  const costSegs = $derived(
    totalCost > 0
      ? [...models]
          .filter((m) => m.estimatedCost > 0)
          .sort((a, b) => b.estimatedCost - a.estimatedCost)
          .map((m, i) => ({
            name: m.displayName,
            cost: m.estimatedCost,
            pct: (m.estimatedCost / totalCost) * 100,
            color: SEG_COLORS[i % SEG_COLORS.length]
          }))
      : []
  );

  const grainLabel = $derived.by(() => {
    const step = data?.stepSec ?? 0;
    const k =
      step === 300
        ? 'analytics.grain5m'
        : step === 3600
          ? 'analytics.grain1h'
          : step === 21600
            ? 'analytics.grain6h'
            : step === 86400
              ? 'analytics.grain1d'
              : '';
    return k ? t(k) : '';
  });
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">
        {t('analytics.title')}
        <span class="help" title={t('analytics.help')}>?</span>
      </h1>
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
      {#if available && models.length}
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={exportCsv} title={t('analytics.exportCsvTip')}>
          {t('analytics.exportCsv')}
        </button>
      {/if}
      <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={loading} onclick={() => load(rangeHours, true)}
        title={t('analytics.refreshTip')}>
        {loading ? t('analytics.loading') : t('analytics.refresh')}
      </button>
    </div>
  </header>

  {#if !available}
    {#if loading}
      <!-- First load: skeleton totals instead of a bare "loading…" line (#147). -->
      <div class="card-grid mb-sw-4">
        {#each Array(4) as _, i (i)}
          <div class="sw-card flex flex-col gap-sw-2">
            <div class="skeleton" style="height:0.7rem;width:50%"></div>
            <div class="skeleton" style="height:1.6rem;width:70%"></div>
          </div>
        {/each}
      </div>
    {:else}
      <!-- Empty state with a CTA pointing at the gateway/providers (#47). -->
      <div class="grid place-items-center py-sw-8 text-center text-sw-text-muted">
        <div class="flex flex-col items-center gap-sw-2">
          <div class="text-2xl opacity-50">📊</div>
          <div class="font-medium text-sw-text">{t('analytics.emptyTitle')}</div>
          <div class="text-sw-sm">{t('analytics.emptyHint')}</div>
          {#if onOpenProviders}
            <button class="sw-btn sw-btn-primary text-sw-xs mt-sw-2" onclick={onOpenProviders}>
              {t('analytics.emptyCta')}
            </button>
          {/if}
        </div>
      </div>
    {/if}
  {:else}
    {#if selectedModel}
      <div class="mb-sw-3 flex items-center gap-sw-2 text-sw-sm">
        <span class="badge badge-info">{t('analytics.selectedFilter', { model: selectedModel.displayName })}</span>
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (selectedKey = null)}>
          {t('analytics.clearFilter')}
        </button>
      </div>
    {/if}

    <!-- Totals (reflect the selected model when filtered) -->
    <div class="card-grid mb-sw-4">
      <div class="sw-card">
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.totalRequests')}</p>
        <p class="mt-1 text-2xl font-semibold">{fmt(totals?.totalRequests ?? 0)}</p>
        <p class="mt-1 text-sw-xs text-sw-text-secondary">
          {t('analytics.successRate')}: {pct(totals?.successRate ?? 0)}
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
        <p class="mt-1 text-2xl font-semibold">{fmt(totals?.avgLatencyMs ?? 0)}<span class="text-sw-sm font-normal"> {t('analytics.unitMs')}</span></p>
      </div>
      <div class="sw-card">
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.savings')}</p>
        <p class="mt-1 text-2xl font-semibold text-emerald-500">{money(totals?.estimatedCostSavings ?? 0)}</p>
        <p class="mt-1 text-sw-xs text-sw-text-secondary">{t('analytics.savingsHint')}</p>
      </div>
    </div>

    <!-- Top-N insight: most frequent + most expensive models (#112) -->
    {#if topReq.length}
      <div class="card-grid mb-sw-4">
        <div class="sw-card">
          <p class="text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{t('analytics.topRequests')}</p>
          <ol class="mt-sw-2 flex flex-col gap-1 text-sw-sm">
            {#each topReq as m, i (keyOf(m))}
              <li class="flex justify-between gap-sw-2"><span class="truncate" title={m.displayName}>{i + 1}. {m.displayName}</span><span class="tabular-nums text-sw-text-muted">{fmt(m.requests)}</span></li>
            {/each}
          </ol>
        </div>
        {#if topCost.length}
          <div class="sw-card">
            <p class="text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{t('analytics.topCost')}</p>
            <ol class="mt-sw-2 flex flex-col gap-1 text-sw-sm">
              {#each topCost as m, i (keyOf(m))}
                <li class="flex justify-between gap-sw-2"><span class="truncate" title={m.displayName}>{i + 1}. {m.displayName}</span><span class="tabular-nums text-sw-text-muted">{money(m.estimatedCost)}</span></li>
              {/each}
            </ol>
          </div>
        {/if}
      </div>
    {/if}

    <!-- Cost breakdown by model as a stacked bar (#23); hidden when all usage is free. -->
    {#if costSegs.length}
      <div class="sw-card mb-sw-4">
        <p class="mb-sw-2 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{t('analytics.costByModel')}</p>
        <div class="costbar">
          {#each costSegs as s (s.name)}
            <div class="seg" style="width:{s.pct}%;background:{s.color}" title="{s.name}: {money(s.cost)} ({s.pct.toFixed(1)}%)"></div>
          {/each}
        </div>
        <div class="mt-sw-2 flex flex-wrap gap-x-sw-4 gap-y-1 text-sw-xs">
          {#each costSegs as s (s.name)}
            <span class="flex items-center gap-1"><span class="legend-dot" style="background:{s.color}"></span><span class="truncate" style="max-width:160px" title={s.name}>{s.name}</span><span class="text-sw-text-muted">{s.pct.toFixed(0)}%</span></span>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Trend -->
    <div class="sw-card mb-sw-6">
      <div class="mb-sw-2 flex items-baseline justify-between gap-sw-2">
        <p class="text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{t('analytics.trend')}</p>
        <p class="text-sw-xs text-sw-text-muted">{grainLabel}</p>
      </div>
      {#if trend.length >= 2}
        <Sparkline points={trend} labels={trendLabels} width={680} height={56} title={t('analytics.trend')}
          peakLabel={trend.length ? `↑ ${fmt(Math.max(...trend))}` : ''} />
      {:else}
        <p class="text-sw-sm text-sw-text-muted">{t('analytics.trendEmpty')}</p>
      {/if}
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
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('requests')}>{t('analytics.colRequests')}{sortArrow('requests')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('successRate')}>{t('analytics.colSuccess')}{sortArrow('successRate')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('avgLatencyMs')}>{t('analytics.colLatency')}{sortArrow('avgLatencyMs')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('tokens')}>{t('analytics.colTokens')}{sortArrow('tokens')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('estimatedCost')}>{t('analytics.colCost')}{sortArrow('estimatedCost')}</button>
              </th>
            </tr>
          </thead>
          <tbody>
            {#each sortedModels as m (keyOf(m))}
              <tr class="border-b border-sw-border last:border-0 {selectedKey === keyOf(m) ? 'row-active' : ''}"
                aria-selected={selectedKey === keyOf(m)}>
                <td class="px-sw-3 py-sw-2">
                  <button class="row-pick" title={t('analytics.rowTip')} onclick={() => toggleSelect(m)}>
                    <span class="block truncate font-medium" title={m.displayName}>{m.displayName}</span>
                    <span class="block truncate font-mono text-[11px] text-sw-text-muted">{m.platform}/{m.modelId}</span>
                  </button>
                </td>
                <td class="px-sw-3 py-sw-2 text-right">{fmt(m.requests)}</td>
                <td class="px-sw-3 py-sw-2 text-right">{pct(m.successRate)}</td>
                <td class="px-sw-3 py-sw-2 text-right">{fmt(m.avgLatencyMs)} {t('analytics.unitMs')}</td>
                <td class="px-sw-3 py-sw-2 text-right">{fmt(m.totalInputTokens + m.totalOutputTokens)}</td>
                <td class="px-sw-3 py-sw-2 text-right">{money(m.estimatedCost)}</td>
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
  .th-sort {
    cursor: pointer;
    font: inherit;
    color: inherit;
  }
  .th-sort:hover {
    color: var(--sw-text-primary);
  }
  .row-active {
    background: var(--sw-bg-secondary);
  }
  .row-pick {
    display: block;
    width: 100%;
    text-align: left;
    cursor: pointer;
    color: inherit;
    font: inherit;
  }
  .help {
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    border: 1px solid var(--sw-border);
    font-size: 10px;
    font-weight: 600;
    color: var(--sw-text-muted);
    cursor: help;
    vertical-align: middle;
  }
  .costbar {
    display: flex;
    height: 14px;
    border-radius: var(--sw-radius-sm);
    overflow: hidden;
    background: var(--sw-bg-secondary);
  }
  .costbar .seg {
    height: 100%;
  }
  .legend-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
</style>
