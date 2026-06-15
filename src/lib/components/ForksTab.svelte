<script lang="ts">
  import type { ForkStatus, ForkAction, GithubRepo } from '$lib/ipc';
  import { forkMode, t, locale } from '$lib/i18n';
  import ForkRepoCard from './ForkRepoCard.svelte';

  let {
    status,
    githubRepos = [],
    running,
    forkRuns = {},
    onAction,
    onCancelFork,
    onBatchFf,
    onOpenUrl
  }: {
    status: ForkStatus | null | undefined;
    githubRepos?: GithubRepo[];
    running: string | null;
    forkRuns?: Record<string, { line: string; running: boolean; code: number | null }>;
    onAction: (action: ForkAction, path?: string, label?: string) => void;
    onCancelFork?: (path: string) => void;
    onBatchFf: (names: string[]) => void;
    onOpenUrl?: (url: string) => void;
  } = $props();

  const anyRunning = $derived(!!running);
  // A whole-stack action must not run while any single-repo run is in flight (they'd contend on the
  // same repos' git + the shared status file) — and vice-versa (per-repo buttons gate on anyRunning).
  const anyForkRunning = $derived(Object.values(forkRuns).some((r) => r?.running));
  const repos = $derived(status?.repos ?? []);
  const summary = $derived(status?.summary);

  // GitHub repos not present as a local clone (matched by name, case-insensitive) — the
  // "where are my other repos" answer: everything on GitHub that isn't checked out here.
  const localNames = $derived(new Set(repos.map((r) => r.Name.toLowerCase())));
  const githubOnly = $derived(
    (githubRepos ?? [])
      .filter((g) => !localNames.has(g.name.toLowerCase()))
      .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1))
  );
  let ghOpen = $state(true);

  // Filter: all / forks / own — applied to BOTH local repos and the GitHub-only list.
  let repoFilter = $state<'all' | 'fork' | 'own'>('all');
  const filteredRepos = $derived(
    repoFilter === 'fork'
      ? repos.filter((r) => !r.isOwn)
      : repoFilter === 'own'
        ? repos.filter((r) => r.isOwn)
        : repos
  );
  const filteredGithubOnly = $derived(
    repoFilter === 'fork'
      ? githubOnly.filter((g) => g.isFork)
      : repoFilter === 'own'
        ? githubOnly.filter((g) => !g.isFork)
        : githubOnly
  );
  // Counts span local + GitHub-only, so the toggle reflects ALL your repos.
  const ownCount = $derived(
    repos.filter((r) => r.isOwn).length + githubOnly.filter((g) => !g.isFork).length
  );
  const forkCount = $derived(
    repos.filter((r) => !r.isOwn).length + githubOnly.filter((g) => g.isFork).length
  );
  const filterTabs = $derived([
    { id: 'all' as const, label: t('forks.filterAll'), n: repos.length + githubOnly.length },
    { id: 'fork' as const, label: t('forks.filterForks'), n: forkCount },
    { id: 'own' as const, label: t('forks.filterOwn'), n: ownCount }
  ]);

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
  <header class="mb-sw-4 flex flex-wrap items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('forks.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('forks.intro')}{status?.ghAvailable === false ? t('forks.introGhUnavailable') : ''}
      </p>
    </div>
    <div class="flex shrink-0 gap-sw-2">
      <button class="sw-btn sw-btn-ghost" disabled={anyRunning || anyForkRunning} onclick={() => onAction('check')}
        title={t('forks.checkTip')}>
        {running === 'forks' ? t('common.busy') : t('common.check')}
      </button>
      <button class="sw-btn sw-btn-ghost" disabled={anyRunning || anyForkRunning} onclick={() => onAction('plan')}
        title={t('forks.planTip')}>
        {t('forks.planBtn')}
      </button>
      <button class="sw-btn sw-btn-primary" disabled={anyRunning || anyForkRunning || ffable.length === 0}
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
        <div>
          {running === 'forks'
            ? t('forks.refreshing')
            : t('forks.modeLine', { mode: forkMode(status?.mode) })}
        </div>
        <div>{t('forks.updatedAt', { time: fmtTime(status?.timestamp ?? status?.generatedAt) })}</div>
      </div>
    </div>
  {/if}

  {#if repos.length}
    <div class="mb-sw-4 inline-flex gap-1 rounded-sw-md border border-sw-border p-1">
      {#each filterTabs as f (f.id)}
        <button
          class="rounded-sw-sm px-sw-3 py-1 text-sw-xs {repoFilter === f.id
            ? 'bg-sw-bg-secondary font-medium text-sw-text'
            : 'text-sw-text-muted hover:text-sw-text'}"
          onclick={() => (repoFilter = f.id)}
          title={t('forks.filterTip')}
        >
          {f.label} <span class="tabular-nums opacity-70">{f.n}</span>
        </button>
      {/each}
    </div>
    <div class="card-grid">
      {#each filteredRepos as repo (repo.Path)}
        <ForkRepoCard
          {repo}
          {anyRunning}
          run={forkRuns[repo.Path]}
          onAction={(a, p, l) => onAction(a, p, l)}
          onCancel={() => onCancelFork?.(repo.Path)}
        />
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

  {#if githubOnly.length}
    <section class="mt-sw-6">
      <button
        class="mb-sw-2 flex items-center gap-sw-2 text-sw-sm font-semibold text-sw-text-secondary hover:text-sw-text"
        onclick={() => (ghOpen = !ghOpen)}
        title={t('forks.githubOnlyTip')}
      >
        <span class="text-sw-text-muted">{ghOpen ? '▾' : '▸'}</span>
        {t('forks.githubOnlyHeading', { n: filteredGithubOnly.length })}
      </button>
      {#if ghOpen}
        {#if filteredGithubOnly.length}
          <div class="card-grid">
            {#each filteredGithubOnly as g (g.nameWithOwner)}
              <div class="sw-card flex flex-col gap-sw-2">
                <div class="flex items-start justify-between gap-sw-2">
                  <h3 class="truncate font-medium">{g.name}</h3>
                  <div class="flex shrink-0 flex-wrap justify-end gap-sw-2">
                    {#if g.isPrivate}<span class="badge badge-warn" title={t('forks.ghPrivateTip')}>{t('forks.ghPrivate')}</span>{/if}
                    <span class="badge badge-muted">{g.isFork ? t('forks.badgeFork') : t('forks.badgeOwn')}</span>
                  </div>
                </div>
                <p class="truncate font-mono text-sw-xs text-sw-text-muted" title={g.nameWithOwner}>{g.nameWithOwner}</p>
                <div class="mt-auto flex gap-sw-2 border-t border-sw-border pt-sw-2">
                  <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onOpenUrl?.(g.url)} title={t('forks.ghOpenTip')}>{t('forks.ghOpen')}</button>
                </div>
              </div>
            {/each}
          </div>
        {:else}
          <p class="text-sw-xs text-sw-text-muted">{t('forks.githubOnlyEmptyFilter')}</p>
        {/if}
      {/if}
    </section>
  {/if}
</div>
