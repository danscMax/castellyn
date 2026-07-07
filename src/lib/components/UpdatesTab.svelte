<script lang="ts">
  import type { Component } from '$lib/ipc';
  import ComponentCard from './ComponentCard.svelte';
  import { t } from '$lib/i18n';
  import { countOf } from '$lib/envelope';
  import { relTime } from '$lib/relativeTime';

  // A check older than this reads as stale — surfaced (not hidden) so "current" doesn't mean "fresh".
  const STALE_MS = 14 * 24 * 60 * 60 * 1000;

  let {
    components,
    statuses,
    running,
    allProgress = null,
    onCheck,
    onApply,
    onOpenTab
  }: {
    components: Component[];
    statuses: Record<string, any>;
    running: string | null;
    allProgress?: string | null;
    onCheck: (id: string) => void;
    onApply: (comp: Component) => void;
    onOpenTab?: (id: string) => void;
  } = $props();

  // Redesign 2B: a roll-up header + sections by REAL state, so the wall of equal cards
  // becomes "what needs action" first and "everything current" collapsed to a counter.
  // The `all` orchestrator card leaves the grid — the header buttons drive it instead.
  const allComp = $derived(components.find((c) => c.id === 'all') ?? null);
  const rest = $derived(components.filter((c) => c.id !== 'all'));

  function hasUpdate(c: Component): boolean {
    const s = statuses[c.id];
    if (!s || c.lastJson === null) return false;
    return s.status === 'changes' || countOf(s, 'changed') > 0;
  }
  const isError = (c: Component) => {
    const s = statuses[c.id];
    return !!s && (s.status === 'error' || countOf(s, 'failed') > 0);
  };
  const isHeld = (c: Component) => statuses[c.id]?.status === 'held';

  const errors = $derived(rest.filter(isError));
  const held = $derived(rest.filter((c) => !isError(c) && isHeld(c)));
  const avail = $derived(rest.filter((c) => !isError(c) && !isHeld(c) && hasUpdate(c)));
  const current = $derived(rest.filter((c) => !isError(c) && !isHeld(c) && !hasUpdate(c)));
  // "Everything current" starts collapsed — it's the boring half; a counter says it all.
  let showCurrent = $state(false);

  function ageStr(h: number) {
    if (h < 1) return t('common.minutesAgo', { n: Math.max(1, Math.round(h * 60)) });
    if (h < 48) return t('common.hoursAgo', { n: Math.round(h) });
    return t('common.daysAgo', { n: Math.round(h / 24) });
  }
  const lastChecked = $derived.by(() => {
    const ts = Object.values(statuses)
      .map((s: any) => Date.parse(s?.timestamp ?? ''))
      .filter(Number.isFinite);
    if (!ts.length) return null;
    return ageStr((Date.now() - Math.max(...ts)) / 3_600_000);
  });
  // Oldest component check — the roll-up flags it when it exceeds STALE_MS (lastChecked stays max).
  const oldest = $derived.by(() => {
    const ts = Object.values(statuses)
      .map((s: any) => Date.parse(s?.timestamp ?? ''))
      .filter(Number.isFinite);
    return ts.length ? Math.min(...ts) : null;
  });
  const oldestStale = $derived(oldest != null && Date.now() - oldest > STALE_MS);
</script>

<div class="p-sw-6">
  <header class="mb-sw-4">
    <h1 class="text-lg font-semibold">{t('updates.title')}</h1>
    <p class="text-sw-sm text-sw-text-secondary">
      {t('updates.subtitle')}
    </p>
  </header>

  <!-- Roll-up strip: how many updates, how fresh the check is, and the two whole-stack actions. -->
  <div class="mb-sw-4 sw-card flex flex-wrap items-center gap-sw-3">
    <span class="badge {avail.length || errors.length ? 'badge-warn' : 'badge-ok'}">
      {errors.length
        ? t('updates.groupErrors', { count: errors.length })
        : avail.length
          ? t('updates.groupHasUpdate', { count: avail.length })
          : t('updates.groupAllClear')}
    </span>
    {#if running === 'all' && allProgress}
      <span class="text-sw-sm text-sw-text-secondary">⏳ {t('updates.updatingNow', { step: allProgress })}</span>
    {:else if lastChecked}
      <span class="text-sw-sm text-sw-text-muted">{t('updates.summaryChecked', { time: lastChecked })}</span>
    {/if}
    {#if oldestStale && oldest != null}
      <span class="text-sw-sm status-warn">{t('updates.staleOldest', { time: relTime(new Date(oldest).toISOString()) })}</span>
    {/if}
    {#if allComp}
      <span class="ml-auto flex flex-wrap gap-sw-2">
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!!running}
          title={t('updates.checkTip')} onclick={() => onCheck('all')}>{t('updates.checkAllBtn')}</button>
        <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!!running}
          title={t('updates.updateTip')} onclick={() => onApply(allComp)}>{t('updates.updateAllBtn')}</button>
      </span>
    {/if}
  </div>

  {#snippet card(c: Component)}
    <ComponentCard
      comp={c}
      status={statuses[c.id]}
      busy={running === c.id}
      anyRunning={!!running}
      onCheck={() => onCheck(c.id)}
      onApply={() => onApply(c)}
      onOpenForks={onOpenTab ? () => onOpenTab('forks') : undefined}
    />
  {/snippet}

  {#if errors.length}
    <h2 class="mb-sw-2 section-title">{t('updates.groupErrors', { count: errors.length })}</h2>
    <div class="group-grid mb-sw-6">
      {#each errors as c (c.id)}{@render card(c)}{/each}
    </div>
  {/if}

  {#if avail.length}
    <h2 class="mb-sw-2 section-title">{t('updates.groupHasUpdate', { count: avail.length })}</h2>
    <div class="group-grid mb-sw-6">
      {#each avail as c (c.id)}{@render card(c)}{/each}
    </div>
  {/if}

  {#if held.length}
    <h2 class="mb-sw-2 section-title">{t('updates.groupHeld', { count: held.length })}</h2>
    <div class="group-grid mb-sw-6">
      {#each held as c (c.id)}{@render card(c)}{/each}
    </div>
  {/if}

  <button class="section-title toggle-current mb-sw-2" onclick={() => (showCurrent = !showCurrent)}
    aria-expanded={showCurrent}>
    <span class="chev" class:open={showCurrent}>▸</span>
    {t('updates.groupUpToDate', { count: current.length })}
  </button>
  {#if showCurrent || (!errors.length && !avail.length && !held.length)}
    <div class="group-grid">
      {#each current as c (c.id)}{@render card(c)}{/each}
    </div>
  {/if}
</div>

<style>
  .group-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: var(--sw-space-4);
    /* stretch → cards in the same row share a height (footers align), so the grid looks even
       instead of ragged when one card (e.g. forks) has extra content. */
    align-items: stretch;
  }
  .toggle-current {
    display: flex;
    align-items: center;
    gap: var(--sw-space-1);
    border: none;
    background: transparent;
    cursor: pointer;
    padding: 0;
  }
  .toggle-current:hover {
    color: var(--sw-text-secondary);
  }
  .chev {
    transition: transform 0.15s ease;
  }
  .chev.open {
    transform: rotate(90deg);
  }
</style>
