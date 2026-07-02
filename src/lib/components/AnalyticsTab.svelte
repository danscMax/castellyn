<script lang="ts">
  import { readFreellmapiAnalytics, type FreellmapiAnalytics, type AnalyticsModel } from '$lib/ipc';
  import { t, locale } from '$lib/i18n';
  import EmptyState from './EmptyState.svelte';
  import { pushToast } from '$lib/toast.svelte';
  import { chartSeriesColor } from '$lib/statusColor';
  import Sparkline from './Sparkline.svelte';
  import { runHistory, clearRunHistory, type RunRecord } from '$lib/runHistory.svelte';
  import { BarChart3 } from '@lucide/svelte';
  import SectionHeader from './SectionHeader.svelte';

  let { onOpenProviders }: { onOpenProviders?: () => void } = $props();

  // Range presets (hours). Self-contained: this tab owns its fetch + range state.
  const ranges = [
    { hours: 1, key: 'analytics.range1h' },
    { hours: 24, key: 'analytics.range24h' },
    { hours: 168, key: 'analytics.range7d' },
    { hours: 720, key: 'analytics.range30d' }
  ];

  let rangeHours = $state(168);
  // V9: the trend sparkline stretches to its card (was fixed 680px — half the card sat empty).
  let trendW = $state(0);
  // L1: envelope statuses are internal enum words — show localized labels (same canon as the
  // Updates cards; 'changes' gets its own noun since there's no per-run count here).
  const runStatusLabel = (s: string): string =>
    s === 'ok'
      ? t('updates.healthUpToDate')
      : s === 'error'
        ? t('updates.healthError')
        : s === 'held'
          ? t('updates.healthHeld')
          : s === 'changes'
            ? t('analytics.statusChanges')
            : s;
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
    } catch (e) {
      // Surface a transient backend error as a toast rather than silently collapsing to the
      // "no usage yet" empty state; keep any last-good data so the view doesn't blank out.
      if (loaded === token) pushToast({ kind: 'error', title: t('common.error'), detail: String(e) });
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
    // CSV-injection guard (OWASP): a cell starting with = + - @ or a tab/CR is treated as a
    // formula by Excel/Sheets — neutralise it with a leading apostrophe before quoting.
    const esc = (c: string) => {
      const s = /^[=+\-@\t\r]/.test(c) ? `'${c}` : c;
      return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
    };
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

  // Shared trend axis (global min bucket + step + bucket count) computed once, so the trend
  // values and their hover labels can't drift apart. n is clamped to the <=1000 render budget.
  const trendAxis = $derived.by(() => {
    const series = data?.series ?? [];
    const step = data?.stepSec ?? 0;
    if (!series.length || step <= 0) return null;
    let lo = Infinity;
    let hi = -Infinity;
    for (const s of series) {
      if (s.bucket < lo) lo = s.bucket;
      if (s.bucket > hi) hi = s.bucket;
    }
    const n = Math.floor((hi - lo) / step) + 1;
    if (n <= 0 || n > 1000) return null;
    return { lo, step, n };
  });

  // Trend: zero-filled requests-per-bucket over a stable axis (global min..max bucket), optionally
  // scoped to the selected model. Axis is global so the shape stays put when filtering.
  const trend = $derived.by(() => {
    const axis = trendAxis;
    const series = data?.series ?? [];
    if (!axis) return [] as number[];
    const { lo, step, n } = axis;
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
    const axis = trendAxis;
    if (!axis) return [] as string[];
    const { lo, step } = axis;
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
            color: chartSeriesColor(i)
          }))
      : []
  );

  // --- Script-run histogram (Phase 3.6): group runs by day, show duration/count bars ---
  type HistDay = {
    label: string;
    dateKey: string;
    durationSec: number;
    count: number;
    durPct: number;
    countPct: number;
    items: RunRecord[];
  };
  let histMode = $state<'duration' | 'runs'>('duration');
  let expandedDay = $state<string | null>(null);
  const dayLabelFmt = $derived(new Intl.DateTimeFormat(fmtLocale, { month: 'short', day: 'numeric' }));
  const histDays = $derived.by<HistDay[]>(() => {
    const items = runHistory.items;
    if (!items.length) return [];
    const groups = new Map<string, RunRecord[]>();
    for (const r of items) {
      const key = new Date(r.timestamp).toDateString();
      let g = groups.get(key);
      if (!g) groups.set(key, (g = []));
      g.push(r);
    }
    const days: HistDay[] = [];
    for (const [dateKey, runs] of groups) {
      days.push({
        label: dayLabelFmt.format(new Date(runs[0].timestamp)),
        dateKey,
        durationSec: runs.reduce((s, r) => s + r.durationSec, 0),
        count: runs.length,
        durPct: 0,
        countPct: 0,
        items: runs
      });
    }
    days.sort((a, b) => b.dateKey.localeCompare(a.dateKey));
    const maxDur = Math.max(...days.map((d) => d.durationSec), 0);
    const maxCount = Math.max(...days.map((d) => d.count), 0);
    for (const d of days) {
      d.durPct = maxDur > 0 ? (d.durationSec / maxDur) * 100 : 0;
      d.countPct = maxCount > 0 ? (d.count / maxCount) * 100 : 0;
    }
    return days;
  });

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

  <!-- Script run metrics — Phase 3.6 histogram by day -->
  <section class="sw-card mb-sw-6">
    <div class="mb-sw-2 flex items-center justify-between gap-sw-2">
      <div>
        <p class="section-title">{t('analytics.scriptMetrics')}</p>
        <p class="text-sw-xs text-sw-text-muted">{t('analytics.scriptMetricsDesc')}</p>
      </div>
      <div class="flex shrink-0 items-center gap-sw-2">
        <div class="flex gap-1" role="tablist">
          <button
            class="sw-btn sw-btn-ghost text-sw-xs {histMode === 'duration' ? 'is-active' : ''}"
            onclick={() => (histMode = 'duration')}>
            {t('analytics.scriptHistDuration')}
          </button>
          <button
            class="sw-btn sw-btn-ghost text-sw-xs {histMode === 'runs' ? 'is-active' : ''}"
            onclick={() => (histMode = 'runs')}>
            {t('analytics.scriptHistRuns')}
          </button>
        </div>
        {#if runHistory.items.length}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={clearRunHistory}>{t('analytics.scriptClear')}</button>
        {/if}
      </div>
    </div>

    {#if !histDays.length}
      <p class="text-sw-sm text-sw-text-muted">{t('analytics.scriptNoData')}</p>
    {:else}
      <div class="flex flex-col gap-1">
        {#each histDays as day (day.dateKey)}
          <button
            class="hist-bar-row"
            class:expanded={expandedDay === day.dateKey}
            onclick={() => (expandedDay = expandedDay === day.dateKey ? null : day.dateKey)}
            title={day.items.length > 1 ? t('analytics.expandTip') : ''}>
            <span class="hist-day-label">{day.label}</span>
            <span class="hist-bar-track">
              <span class="hist-bar" style="width: {histMode === 'duration' ? day.durPct : day.countPct}%"></span>
            </span>
            <span class="hist-value">
              {histMode === 'duration'
                ? `${day.durationSec.toFixed(0)} ${t('analytics.unitS')}`
                : `${day.count}`}
              {#if histMode === 'duration' && day.count > 1}
                <span class="text-sw-text-muted">· {day.count}</span>
              {/if}
            </span>
          </button>
          {#if expandedDay === day.dateKey}
            <div class="hist-detail">
              <table class="w-full text-sw-sm">
                <thead>
                  <tr class="border-b border-sw-border text-left text-sw-xs text-sw-text-muted">
                    <th class="px-sw-2 py-sw-1 font-medium">{t('analytics.scriptColComponent')}</th>
                    <th class="px-sw-2 py-sw-1 text-right font-medium">{t('analytics.scriptColDuration')}</th>
                    <th class="px-sw-2 py-sw-1 text-right font-medium">{t('analytics.scriptColStatus')}</th>
                  </tr>
                </thead>
                <tbody>
                  {#each day.items as run (run.timestamp)}
                    <tr class="border-b border-sw-border last:border-0">
                      <td class="px-sw-2 py-sw-1 font-medium">{run.component}</td>
                      <td class="px-sw-2 py-sw-1 text-right tabular-nums">{run.durationSec.toFixed(1)} {t('analytics.unitS')}</td>
                      <td class="px-sw-2 py-sw-1 text-right">
                        <!-- L1: localized status label instead of the raw envelope word -->
                        <span class="badge badge-{run.status === 'ok' ? 'ok' : run.status === 'changes' ? 'warn' : run.status === 'error' ? 'err' : 'muted'}">{runStatusLabel(run.status)}</span>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  </section>

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
      <EmptyState
        icon={BarChart3}
        title={t('analytics.emptyTitle')}
        description={t('analytics.emptyHint')}
        action={onOpenProviders}
        actionLabel={t('analytics.emptyCta')}
      />
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
        <p class="mt-1 text-2xl font-semibold status-ok">{money(totals?.estimatedCostSavings ?? 0)}</p>
        <p class="mt-1 text-sw-xs text-sw-text-secondary">{t('analytics.savingsHint')}</p>
      </div>
    </div>

    <!-- Top-N insight: most frequent + most expensive models (#112) -->
    {#if topReq.length}
        <div class="card-grid mb-sw-4">
        <div class="sw-card">
          <p class="section-title">{t('analytics.topRequests')}</p>
          <ol class="mt-sw-2 list-none flex flex-col gap-1 text-sw-sm m-0 p-0">
            {#each topReq as m, i (keyOf(m))}
              <li class="flex justify-between gap-sw-2"><span class="truncate" title={m.displayName}>{i + 1}. {m.displayName}</span><span class="tabular-nums text-sw-text-muted">{fmt(m.requests)}</span></li>
            {/each}
          </ol>
        </div>
        {#if topCost.length}
          <div class="sw-card">
            <p class="section-title">{t('analytics.topCost')}</p>
            <ol class="mt-sw-2 list-none flex flex-col gap-1 text-sw-sm m-0 p-0">
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
        <p class="mb-sw-2 section-title">{t('analytics.costByModel')}</p>
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
        <p class="section-title">{t('analytics.trend')}</p>
        <p class="text-sw-xs text-sw-text-muted">{grainLabel}</p>
      </div>
      {#if trend.length >= 2}
        <div class="w-full" bind:clientWidth={trendW}>
          <Sparkline points={trend} labels={trendLabels} width={Math.max(320, trendW || 680)} height={56} title={t('analytics.trend')}
            peakLabel={trend.length ? `↑ ${fmt(Math.max(...trend))}` : ''} />
        </div>
      {:else}
        <p class="text-sw-sm text-sw-text-muted">{t('analytics.trendEmpty')}</p>
      {/if}
    </div>

    <!-- Per-model -->
    <SectionHeader title={t('analytics.perModel')} />
    {#if models.length}
      <div class="overflow-x-auto rounded-sw-md border border-sw-border">
        <table class="w-full text-sw-sm">
          <thead>
            <tr class="border-b border-sw-border text-sw-xs text-sw-text-muted">
              <th class="px-sw-3 py-sw-2 text-left font-medium">{t('analytics.colModel')}</th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button type="button" class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('requests')}>{t('analytics.colRequests')}{sortArrow('requests')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button type="button" class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('successRate')}>{t('analytics.colSuccess')}{sortArrow('successRate')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button type="button" class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('avgLatencyMs')}>{t('analytics.colLatency')}{sortArrow('avgLatencyMs')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button type="button" class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('tokens')}>{t('analytics.colTokens')}{sortArrow('tokens')}</button>
              </th>
              <th class="px-sw-3 py-sw-2 text-right font-medium">
                <button type="button" class="th-sort" title={t('analytics.sortTip')} onclick={() => toggleSort('estimatedCost')}>{t('analytics.colCost')}{sortArrow('estimatedCost')}</button>
              </th>
            </tr>
          </thead>
          <tbody>
            {#each sortedModels as m (keyOf(m))}
              <tr class="border-b border-sw-border last:border-0 {selectedKey === keyOf(m) ? 'row-active' : ''}"
                aria-selected={selectedKey === keyOf(m)}>
                <td class="px-sw-3 py-sw-2">
                  <button type="button" class="row-pick" title={t('analytics.rowTip')} onclick={() => toggleSelect(m)}>
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
  .hist-bar-row {
    display: flex;
    align-items: center;
    gap: var(--sw-space-3);
    width: 100%;
    padding: var(--sw-space-1) var(--sw-space-2);
    border-radius: var(--sw-radius-sm);
    cursor: pointer;
    text-align: left;
    color: inherit;
    font: inherit;
    transition: background 0.1s;
  }
  .hist-bar-row:hover { background: var(--sw-bg-secondary); }
  .hist-bar-row.expanded { background: var(--sw-bg-secondary); }
  .hist-day-label {
    width: 5rem;
    flex-shrink: 0;
    font-size: var(--sw-text-xs);
    font-weight: 500;
    color: var(--sw-text-muted);
  }
  .hist-bar-track {
    flex: 1;
    height: 10px;
    border-radius: 5px;
    background: var(--sw-bg-secondary);
    overflow: hidden;
  }
  .hist-bar {
    display: block;
    height: 100%;
    border-radius: 5px;
    background: var(--sw-btn-primary-bg, #3b82f6);
    opacity: 0.55;
    transition: width 0.2s ease, opacity 0.15s;
  }
  .hist-bar-row:hover .hist-bar { opacity: 0.8; }
  .hist-value {
    width: 6rem;
    flex-shrink: 0;
    text-align: right;
    font-size: var(--sw-text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--sw-text-secondary);
  }
  .hist-detail {
    padding: var(--sw-space-2) var(--sw-space-2) var(--sw-space-2) 5rem;
    border-bottom: 1px solid var(--sw-border);
  }
</style>
