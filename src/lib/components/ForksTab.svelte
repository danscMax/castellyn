<script lang="ts">
  import type { ForkStatus, ForkAction, GithubRepo } from '$lib/ipc';
  import { forkMode, t, locale, plural, pRepo, pConflict } from '$lib/i18n';
  import { relTime } from '$lib/relativeTime';
  import ForkRepoCard from './ForkRepoCard.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';

  let {
    status,
    githubRepos = [],
    running,
    forkRuns = {},
    onAction,
    onCancelFork,
    onBatchFf,
    onOpenUrl,
    onOpenSession
  }: {
    status: ForkStatus | null | undefined;
    githubRepos?: GithubRepo[];
    running: string | null;
    forkRuns?: Record<string, { line: string; running: boolean; code: number | null }>;
    onAction: (action: ForkAction, path?: string, label?: string) => void;
    onCancelFork?: (path: string) => void;
    onBatchFf: (names: string[]) => void;
    onOpenUrl?: (url: string) => void;
    onOpenSession?: (path: string) => void;
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
  // Extra status filter toggled by clicking a KPI (conflict / needs-hands). Local repos only.
  let statusFilter = $state<'conflict' | 'needHands' | null>(null);
  const repoHasConflict = (r: import('$lib/ipc').ForkRepo) =>
    (r.branches ?? []).some((b) => b.outcome === 'conflict' || (b.conflictFiles?.length ?? 0) > 0);
  const repoNeedsHands = (r: import('$lib/ipc').ForkRepo) =>
    r.dirty || r.untracked || r.midOp || r.detached || repoHasConflict(r) || (r.behindBy ?? 0) > 0;
  const matchesStatus = (r: import('$lib/ipc').ForkRepo) =>
    !statusFilter || (statusFilter === 'conflict' ? repoHasConflict(r) : repoNeedsHands(r));
  const byKind = $derived(
    repoFilter === 'fork'
      ? repos.filter((r) => !r.isOwn)
      : repoFilter === 'own'
        ? repos.filter((r) => r.isOwn)
        : repos
  );
  const filteredRepos = $derived(byKind.filter(matchesStatus));
  // #106: sort cycles name → most-behind (by behindBy desc).
  let sortBy = $state<'name' | 'behind'>('name');
  const sortedRepos = $derived.by(() => {
    const list = [...filteredRepos];
    return sortBy === 'behind'
      ? list.sort((a, b) => (b.behindBy ?? 0) - (a.behindBy ?? 0) || a.Name.localeCompare(b.Name))
      : list.sort((a, b) => a.Name.localeCompare(b.Name));
  });
  const filteredGithubOnly = $derived(
    // Not-cloned repos have no local status, so hide them while a status filter is active.
    statusFilter
      ? []
      : repoFilter === 'fork'
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
      { label: pRepo(s.repos), value: s.repos, cls: 'text-sw-text', tip: t('forks.kpiReposTip'), filter: null },
      { label: t('forks.kpiMerged'), value: s.merged, cls: 'text-emerald-400', tip: t('forks.kpiMergedTip'), filter: null },
      { label: t('forks.kpiOpen'), value: s.open, cls: 'text-sky-400', tip: t('forks.kpiOpenTip'), filter: null },
      { label: pConflict(s.conflict), value: s.conflict, cls: s.conflict > 0 ? 'text-amber-400' : 'text-sw-text', tip: t('forks.kpiConflictsTip'), filter: 'conflict' as const },
      { label: plural(s.needHands, t('forks.needHands_one'), t('forks.needHands_few'), t('forks.needHands_many')), value: s.needHands, cls: s.needHands > 0 ? 'text-amber-400' : 'text-sw-text', tip: t('forks.kpiNeedHandsTip'), filter: 'needHands' as const }
    ];
  });
  function clickKpi(filter: 'conflict' | 'needHands' | null) {
    statusFilter = filter && statusFilter !== filter ? filter : null;
  }

  type Gh = GithubRepo;
  const GH_COLS: DTColumn[] = [
    { key: 'name', label: t('forks.ghColName'), grow: true, sortable: true },
    { key: 'full', label: t('forks.ghColRepo'), width: '300px', sortable: true },
    { key: 'kind', label: t('forks.ghColKind'), width: '150px' },
    { key: 'actions', label: t('forks.ghColActions'), width: '100px', align: 'right', interactive: true }
  ];
  function ghSort(g: Gh, key: string): string | number {
    if (key === 'full') return g.nameWithOwner.toLowerCase();
    return g.name.toLowerCase();
  }
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
        {#if k.filter}
          <button class="min-w-[92px] cursor-pointer rounded-sw-md border text-center {statusFilter === k.filter ? 'border-sw-accent bg-sw-accent-glow' : 'border-transparent hover:border-sw-border'}"
            title={k.tip} onclick={() => clickKpi(k.filter)}>
            <div class="text-2xl font-semibold tabular-nums {k.cls}">{k.value}</div>
            <div class="text-sw-xs uppercase tracking-wide text-sw-text-muted">{k.label}</div>
          </button>
        {:else}
          <div class="min-w-[92px] text-center" title={k.tip}>
            <div class="text-2xl font-semibold tabular-nums {k.cls}">{k.value}</div>
            <div class="text-sw-xs uppercase tracking-wide text-sw-text-muted">{k.label}</div>
          </div>
        {/if}
      {/each}
      <div class="ml-auto text-right text-sw-xs text-sw-text-muted">
        <div>
          {running === 'forks'
            ? t('forks.refreshing')
            : t('forks.modeLine', { mode: forkMode(status?.mode) })}
        </div>
        <div title={fmtTime(status?.timestamp ?? status?.generatedAt)}>{t('forks.updatedAt', { time: relTime(status?.timestamp ?? status?.generatedAt) || fmtTime(status?.timestamp ?? status?.generatedAt) })}</div>
      </div>
    </div>
  {/if}

  {#if repos.length}
    <div class="mb-sw-4 flex flex-wrap items-center gap-sw-3">
      <div class="inline-flex gap-1 rounded-sw-md border border-sw-border p-1">
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
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (sortBy = sortBy === 'name' ? 'behind' : 'name')}
        title={t('forks.sortTip')}>
        ⇅ {sortBy === 'behind' ? t('forks.sortBehind') : t('forks.sortName')}
      </button>
    </div>
    <div class="card-grid">
      {#each sortedRepos as repo (repo.Path)}
        <ForkRepoCard
          {repo}
          {anyRunning}
          run={forkRuns[repo.Path]}
          onAction={(a, p, l) => onAction(a, p, l)}
          onCancel={() => onCancelFork?.(repo.Path)}
          {onOpenSession}
          refreshing={running === 'forks'}
        />
      {/each}
    </div>
  {:else if running === 'forks'}
    <!-- First load: show skeleton cards instead of a blank pane until the check completes. -->
    <div class="card-grid">
      {#each Array(6) as _, i (i)}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="skeleton" style="height:1.1rem;width:55%"></div>
          <div class="skeleton" style="height:0.7rem;width:80%"></div>
          <div class="skeleton" style="height:1.8rem;width:100%"></div>
        </div>
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
          <DataTable
            columns={GH_COLS}
            rows={filteredGithubOnly}
            rowKey={(g) => g.nameWithOwner}
            sortAccessor={ghSort}
            search
            searchValue={(g) => g.nameWithOwner}
            searchPlaceholder={t('forks.ghColName')}
            storageKey="forks-gh"
          >
            {#snippet cell(g, col)}
              {#if col.key === 'name'}
                <span class="font-medium truncate" title={g.name}>{g.name}</span>
              {:else if col.key === 'full'}
                <span class="font-mono text-sw-xs text-sw-text-muted truncate block" title={g.nameWithOwner}>{g.nameWithOwner}</span>
              {:else if col.key === 'kind'}
                <span class="flex flex-wrap gap-sw-1">
                  {#if g.isPrivate}<span class="badge badge-warn" title={t('forks.ghPrivateTip')}>{t('forks.ghPrivate')}</span>{/if}
                  <span class="badge badge-muted">{g.isFork ? t('forks.badgeFork') : t('forks.badgeOwn')}</span>
                </span>
              {:else if col.key === 'actions'}
                <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onOpenUrl?.(g.url)} title={t('forks.ghOpenTip')}>{t('forks.ghOpen')}</button>
              {/if}
            {/snippet}
          </DataTable>
        {:else}
          <p class="text-sw-xs text-sw-text-muted">{t('forks.githubOnlyEmptyFilter')}</p>
        {/if}
      {/if}
    </section>
  {/if}
</div>
