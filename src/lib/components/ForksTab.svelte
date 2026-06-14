<script lang="ts">
  import type { ForkStatus, ForkAction } from '$lib/ipc';
  import { forkMode, t, locale } from '$lib/i18n';
  import ForkRepoCard from './ForkRepoCard.svelte';

  let {
    status,
    running,
    onAction,
    onBatchFf
  }: {
    status: ForkStatus | null | undefined;
    running: string | null;
    onAction: (action: ForkAction, path?: string, label?: string) => void;
    onBatchFf: (names: string[]) => void;
  } = $props();

  const anyRunning = $derived(!!running);
  const repos = $derived(status?.repos ?? []);
  const summary = $derived(status?.summary);

  // Repos that can be safely fast-forwarded (behind upstream, ff-safe, clean tree, forks only).
  const ffable = $derived(
    repos.filter(
      (r) => !r.isOwn && (r.behindBy ?? 0) > 0 && r.ffSafe && !r.dirty && !r.midOp && !r.detached
    )
  );

  function fmtTime(ts?: string) {
    if (!ts) return t('common.dash');
    try {
      const tag = locale.current === 'ru' ? 'ru-RU' : locale.current === 'zh' ? 'zh-CN' : 'en-US';
      return new Date(ts).toLocaleString(tag);
    } catch {
      return ts;
    }
  }

  const kpis = $derived.by(() => {
    const s = summary;
    if (!s) return [];
    return [
      { label: t('forks.kpiRepos'), value: s.repos, cls: 'text-sw-text', tip: t('forks.kpiReposTip') },
      { label: t('forks.kpiMerged'), value: s.merged, cls: 'text-emerald-400', tip: t('forks.kpiMergedTip') },
      { label: t('forks.kpiOpen'), value: s.open, cls: 'text-sky-400', tip: t('forks.kpiOpenTip') },
      { label: t('forks.kpiConflicts'), value: s.conflict, cls: s.conflict > 0 ? 'text-amber-400' : 'text-sw-text', tip: t('forks.kpiConflictsTip') },
      { label: t('forks.kpiNeedHands'), value: s.needHands, cls: s.needHands > 0 ? 'text-amber-400' : 'text-sw-text', tip: t('forks.kpiNeedHandsTip') }
    ];
  });
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('forks.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('forks.intro')}{status?.ghAvailable === false ? t('forks.introGhUnavailable') : ''}
      </p>
    </div>
    <div class="flex shrink-0 gap-sw-2">
      <button class="sw-btn sw-btn-ghost" disabled={anyRunning} onclick={() => onAction('check')}
        title={t('forks.checkTip')}>
        {running === 'forks' ? t('common.busy') : t('common.check')}
      </button>
      <button class="sw-btn sw-btn-ghost" disabled={anyRunning} onclick={() => onAction('plan')}
        title={t('forks.planTip')}>
        {t('forks.planBtn')}
      </button>
      <button class="sw-btn sw-btn-primary" disabled={anyRunning || ffable.length === 0}
        onclick={() => onBatchFf(ffable.map((r) => r.Name))}
        title={ffable.length
          ? t('forks.ffAllTip', { n: ffable.length })
          : t('forks.ffAllNoneTip')}>
        {t('forks.ffAllBtn')}{ffable.length ? ` (${ffable.length})` : ''}
      </button>
    </div>
  </header>

  {#if summary}
    <div class="sw-card mb-sw-4 flex flex-wrap items-center gap-sw-6">
      {#each kpis as k (k.label)}
        <div class="min-w-[92px] text-center" title={k.tip}>
          <div class="text-2xl font-semibold tabular-nums {k.cls}">{k.value}</div>
          <div class="text-sw-xs uppercase tracking-wide text-sw-text-muted">{k.label}</div>
        </div>
      {/each}
      <div class="ml-auto text-right text-sw-xs text-sw-text-muted">
        <div>{forkMode(status?.mode)}</div>
        <div>{t('forks.updatedAt', { time: fmtTime(status?.timestamp ?? status?.generatedAt) })}</div>
      </div>
    </div>
  {/if}

  {#if repos.length}
    <div class="card-grid">
      {#each repos as repo (repo.Path)}
        <ForkRepoCard {repo} {anyRunning} onAction={(a, p, l) => onAction(a, p, l)} />
      {/each}
    </div>
  {:else}
    <div class="grid place-items-center py-sw-6 text-center text-sw-text-muted">
      <div>
        <div class="mb-sw-2 text-2xl">⑂</div>
        <div class="font-medium text-sw-text">{t('forks.emptyTitle')}</div>
        <div class="text-sw-sm">{t('forks.emptyHint')}</div>
      </div>
    </div>
  {/if}
</div>
