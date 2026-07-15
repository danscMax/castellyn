<script lang="ts">
  import { confFiles, pickFolder, readForkConfig, writeForkConfig, type ForkConfig, type ForkStatus, type ForkAction, type GithubRepo } from '$lib/ipc';
  import { forkMode, t, plural, pRepo, pConflict } from '$lib/i18n';
  import { relTime, formatAbsTime } from '$lib/relativeTime';
  import { statusTextClass } from '$lib/statusColor';
  import { pushToast } from '$lib/toast.svelte';
  import ForkRepoCard from './ForkRepoCard.svelte';
  import EmptyState from './EmptyState.svelte';
  import NoScriptsBanner from './NoScriptsBanner.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';
  import { GitFork, ArrowUpDown, FolderPlus, X } from '@lucide/svelte';
  import { onMount } from 'svelte';

  let {
    status,
    githubRepos = [],
    running,
    forkRuns = {},
    onAction,
    onCancelFork,
    onCancelCheck,
    onBatchFf,
    onOpenUrl,
    onOpenSession,
    onClone,
    cloningRepo = null,
    profiles = [],
    scriptsAvail = true
  }: {
    status: ForkStatus | null | undefined;
    githubRepos?: GithubRepo[];
    running: string | null;
    forkRuns?: Record<string, { line: string; running: boolean; code: number | null }>;
    onAction: (action: ForkAction, path?: string, label?: string) => void;
    onCancelFork?: (path: string) => void;
    onCancelCheck?: () => void;
    onBatchFf: (names: string[]) => void;
    onOpenUrl?: (url: string) => void;
    onOpenSession?: (path: string, tool?: import('$lib/ipc').SessionTool, profile?: string) => void;
    onClone?: (repo: GithubRepo) => void;
    cloningRepo?: string | null;
    profiles?: string[];
    scriptsAvail?: boolean;
  } = $props();

  // --- Fork discovery config (de-hardcode): user-editable roots/paths/ownPaths, durable in %APPDATA%.
  let cfgOpen = $state(false);
  let forkCfg = $state<ForkConfig | null>(null);
  let cfgSaving = $state(false);
  let cfgDirty = $state(false);
  onMount(async () => {
    try {
      forkCfg = await readForkConfig();
    } catch {
      forkCfg = { roots: [], paths: [], ownPaths: [] };
    }
  });
  async function addFolder(kind: 'roots' | 'paths' | 'ownPaths') {
    if (!forkCfg) return;
    const picked = await pickFolder();
    if (!picked) return;
    if (forkCfg[kind].includes(picked)) return; // no dupes
    forkCfg[kind] = [...forkCfg[kind], picked];
    cfgDirty = true;
  }
  function removeFolder(kind: 'roots' | 'paths' | 'ownPaths', value: string) {
    if (!forkCfg) return;
    forkCfg[kind] = forkCfg[kind].filter((v) => v !== value);
    cfgDirty = true;
  }
  async function saveForkCfg() {
    if (!forkCfg) return;
    cfgSaving = true;
    try {
      await writeForkConfig(forkCfg);
      cfgDirty = false; // saved: keep cfgDirty=true on failure below so the user can retry
      // Don't kick off a whole-stack "check" while a fork run is already in flight — it would
      // contend on the same repos' git + the shared status file.
      if (!anyRunning && !anyForkRunning) onAction('check'); // re-scan with the new config so the cards reflect it
    } catch (e) {
      pushToast({ kind: 'error', title: String(e) });
    } finally {
      cfgSaving = false;
    }
  }
  const cfgGroups = $derived(
    forkCfg
      ? ([
          { kind: 'roots' as const, label: t('forks.cfgRoots'), hint: t('forks.cfgRootsHint'), items: forkCfg.roots },
          { kind: 'paths' as const, label: t('forks.cfgPaths'), hint: t('forks.cfgPathsHint'), items: forkCfg.paths },
          { kind: 'ownPaths' as const, label: t('forks.cfgOwn'), hint: t('forks.cfgOwnHint'), items: forkCfg.ownPaths }
        ])
      : []
  );

  const anyRunning = $derived(!!running);
  // Per-repo action buttons must NOT be disabled by an unrelated global run (sync/rtk/plugins): the
  // backend `run_fork_repo` only blocks on a whole-stack fork sweep or the same repo. So the cards
  // gate on a fork sweep specifically, not "any run holds the slot". (Fixes the greyed wip-sync while
  // the card recommends it — it was disabled by the stuck sync run.)
  const forkSweepRunning = $derived(running === 'forks');
  // A whole-stack action must not run while any single-repo run is in flight (they'd contend on the
  // same repos' git + the shared status file); the whole-stack buttons below gate on `anyRunning`.
  const anyForkRunning = $derived(Object.values(forkRuns).some((r) => r?.running));
  const repos = $derived(status?.repos ?? []);
  const summary = $derived(status?.summary);

  // GitHub repos not present as a local clone (matched by name, case-insensitive) — the
  // "where are my other repos" answer: everything on GitHub that isn't checked out here.
  const localNames = $derived(new Set(repos.map((r) => r.Name.toLowerCase())));
  const githubOnly = $derived(
    (githubRepos ?? [])
      .filter((g) => !localNames.has(g.name.toLowerCase()))
      .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : a.updatedAt > b.updatedAt ? -1 : 0))
  );
  let ghOpen = $state(true);

  // Filter: all / forks / own — applied to BOTH local repos and the GitHub-only list.
  let repoFilter = $state<'all' | 'fork' | 'own'>('all');
  // Status filter — toggled by clicking a KPI tile. null = no filter (the "Repos" tile clears).
  let statusFilter = $state<
    'conflict' | 'needHands' | 'merged' | 'open' | null
  >(null);
  // NOTE: these predicates classify REPOS (used for the card filter), while the KPI tile numbers
  // below come from the backend `summary` counts, which count BRANCH occurrences across all repos
  // (e.g. a repo with 2 conflicting branches adds 2 to summary.conflict but is 1 repo in the
  // filter). The tile count and the number of cards a click reveals can legitimately differ.
  const repoHasConflict = (r: import('$lib/ipc').ForkRepo) =>
    (r.branches ?? []).some((b) => b.outcome === 'conflict' || confFiles(b.conflictFiles).length > 0);
  const repoNeedsHands = (r: import('$lib/ipc').ForkRepo) =>
    r.dirty || r.untracked || r.midOp || r.detached || repoHasConflict(r) || (r.behindBy ?? 0) > 0;
  const repoHasMergedPr = (r: import('$lib/ipc').ForkRepo) =>
    (r.branches ?? []).some((b) => b.prState === 'MERGED');
  const repoHasOpenPr = (r: import('$lib/ipc').ForkRepo) =>
    (r.branches ?? []).some((b) => b.prState === 'OPEN');
  const matchesStatus = (r: import('$lib/ipc').ForkRepo) => {
    switch (statusFilter) {
      case 'conflict':
        return repoHasConflict(r);
      case 'needHands':
        return repoNeedsHands(r);
      case 'merged':
        return repoHasMergedPr(r);
      case 'open':
        return repoHasOpenPr(r);
      case null:
        return true;
    }
  };
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
  // A repo the scanner found under a root but that has no GitHub parent and isn't declared own
  // (Skipped='not-a-fork') is un-actionable — collapse these out of the main grid so they stop
  // crowding the real cards. From the group you can reclassify (add to ownPaths / drop the root).
  const actionableRepos = $derived(sortedRepos.filter((r) => r.Skipped !== 'not-a-fork'));
  const notForks = $derived(sortedRepos.filter((r) => r.Skipped === 'not-a-fork'));
  let nfOpen = $state(false);
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

  const fmtTime = (ts?: string) => formatAbsTime(ts);

  const kpis = $derived.by(() => {
    const s = summary;
    if (!s) return [];
    return [
      { label: pRepo(s.repos), value: s.repos, cls: 'text-sw-text', tip: t('forks.kpiReposTip'), filter: 'repos' as const },
      { label: t('forks.kpiMerged'), value: s.merged, cls: s.merged > 0 ? statusTextClass('ok') : 'text-sw-text', tip: t('forks.kpiMergedTip'), filter: 'merged' as const },
      { label: t('forks.kpiOpen'), value: s.open, cls: s.open > 0 ? statusTextClass('info') : 'text-sw-text', tip: t('forks.kpiOpenTip'), filter: 'open' as const },
      { label: pConflict(s.conflict), value: s.conflict, cls: s.conflict > 0 ? statusTextClass('warn') : 'text-sw-text', tip: t('forks.kpiConflictsTip'), filter: 'conflict' as const },
      { label: plural(s.needHands, t('forks.needHands_one'), t('forks.needHands_few'), t('forks.needHands_many')), value: s.needHands, cls: s.needHands > 0 ? statusTextClass('warn') : 'text-sw-text', tip: t('forks.kpiNeedHandsTip'), filter: 'needHands' as const }
    ];
  });
  function clickKpi(filter: 'conflict' | 'needHands' | 'merged' | 'open' | 'repos') {
    // 'repos' = the "show all" tile; toggles the filter off (same as clicking the active tile again).
    if (filter === 'repos') statusFilter = null;
    else statusFilter = statusFilter === filter ? null : filter;
  }

  type Gh = GithubRepo;
  const GH_COLS: DTColumn[] = $derived([
    { key: 'name', label: t('forks.ghColName'), grow: true, sortable: true },
    // V2: the actions cell holds TWO buttons (~190px) — its old 110px pushed the table into a
    // horizontal scroll on a 1440px window and clipped «Клонировать». Secondary columns are
    // trimmed to their real content so everything fits without scrolling down to 1280px.
    { key: 'full', label: t('forks.ghColRepo'), width: '180px', sortable: true },
    { key: 'language', label: t('forks.ghColLang'), width: '80px', sortable: true },
    { key: 'stars', label: t('forks.ghColStars'), width: '56px', align: 'right', sortable: true },
    { key: 'updated', label: t('forks.ghColUpdated'), width: '100px', sortable: true },
    { key: 'kind', label: t('forks.ghColKind'), width: '90px' },
    { key: 'actions', label: t('forks.ghColActions'), width: '165px', align: 'right', interactive: true }
  ]);
  function ghSort(g: Gh, key: string): string | number {
    if (key === 'full') return g.nameWithOwner.toLowerCase();
    if (key === 'updated') return g.updatedAt;
    if (key === 'language') return g.language.toLowerCase();
    if (key === 'stars') return g.stars;
    return g.name.toLowerCase();
  }
</script>

<div class="p-sw-6">
  {#if !scriptsAvail}<NoScriptsBanner />{/if}
  <header class="mb-sw-4 flex flex-wrap items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('forks.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('forks.intro')}{status?.ghAvailable === false ? t('forks.introGhUnavailable') : ''}
      </p>
    </div>
    <div class="flex shrink-0 gap-sw-2">
      {#if running === 'forks'}
        <!-- While a whole-stack refresh runs, the Check button becomes a clear Cancel (was only a
             hover-revealed ✕ in the corner — easy to miss). Stops the run via cancel_run. -->
        <button class="sw-btn sw-btn-ghost forks-cancel-btn" onclick={() => onCancelCheck?.()}
          title={t('forks.cancelCheckTip')}>
          <span class="refresh-dot"></span>
          {t('forks.cancelCheck')}
        </button>
      {:else}
        <button class="sw-btn sw-btn-ghost" disabled={anyRunning || anyForkRunning} onclick={() => onAction('check')}
          title={t('forks.checkTip')}>
          {t('common.check')}
        </button>
      {/if}
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

  <!-- De-hardcode: which folders/repos are tracked is user config, editable here (durable in
       %APPDATA%), not baked in. "Scan a folder" = add it as a root; the next check discovers repos
       under it and classifies fork/own/not-a-fork. -->
  {#if forkCfg}
    <section class="mb-sw-4">
      <button
        class="mb-sw-2 flex items-center gap-sw-2 text-sw-sm font-semibold text-sw-text-secondary hover:text-sw-text"
        onclick={() => (cfgOpen = !cfgOpen)}
        title={t('forks.cfgTip')}
      >
        <span class="text-sw-text-muted">{cfgOpen ? '▾' : '▸'}</span>
        {t('forks.cfgHeading')}
      </button>
      {#if cfgOpen}
        <div class="sw-card flex flex-col gap-sw-4">
          {#each cfgGroups as g (g.kind)}
            <div>
              <div class="mb-sw-1 flex items-center justify-between gap-sw-2">
                <div class="min-w-0">
                  <span class="text-sw-sm font-medium">{g.label}</span>
                  <span class="ml-sw-2 text-sw-xs text-sw-text-muted">{g.hint}</span>
                </div>
                <button class="sw-btn sw-btn-ghost mini shrink-0" onclick={() => addFolder(g.kind)} title={t('forks.cfgAddTip')}>
                  <FolderPlus size={13} aria-hidden="true" /> {t('forks.cfgAdd')}
                </button>
              </div>
              {#if g.items.length}
                <div class="flex flex-col gap-sw-1">
                  {#each g.items as item (item)}
                    <div class="flex items-center gap-sw-2 rounded-sw-sm bg-sw-bg-secondary px-sw-2 py-1">
                      <span class="min-w-0 flex-1 truncate font-mono text-sw-xs" title={item}>{item}</span>
                      <button class="text-sw-text-muted hover:text-sw-danger" onclick={() => removeFolder(g.kind, item)} title={t('forks.cfgRemoveTip')} aria-label={t('forks.cfgRemoveTip')}>
                        <X size={13} aria-hidden="true" />
                      </button>
                    </div>
                  {/each}
                </div>
              {:else}
                <p class="text-sw-xs text-sw-text-muted">{t('forks.cfgEmpty')}</p>
              {/if}
            </div>
          {/each}
          <div class="flex items-center gap-sw-3">
            <button class="sw-btn sw-btn-primary" disabled={!cfgDirty || cfgSaving} onclick={saveForkCfg} title={t('forks.cfgSaveTip')}>
              {cfgSaving ? t('common.busy') : t('forks.cfgSave')}
            </button>
            {#if cfgDirty}<span class="text-sw-xs status-warn">{t('forks.cfgUnsaved')}</span>{/if}
          </div>
        </div>
      {/if}
    </section>
  {/if}

  {#if summary}
    <div class="sw-card mb-sw-4 flex flex-wrap items-center gap-sw-6">
      {#each kpis as k (k.filter)}
        <button class="min-w-[92px] cursor-pointer rounded-sw-md border text-center {statusFilter === k.filter || (k.filter === 'repos' && !statusFilter) ? 'border-sw-accent bg-sw-accent-glow' : 'border-transparent hover:border-sw-border'}"
          title={k.tip} onclick={() => clickKpi(k.filter)}>
          <div class="text-2xl font-semibold tabular-nums {k.cls}">{k.value}</div>
          <div class="text-sw-xs uppercase tracking-wide text-sw-text-muted">{k.label}</div>
        </button>
      {/each}
      <div class="ml-auto text-right text-sw-xs text-sw-text-muted">
        <div>
          {#if running === 'forks'}
            <button class="refresh-chip" onclick={() => onCancelCheck?.()} aria-label={t('forks.cancelCheck')} title={t('forks.cancelCheckTip')}>
              <span class="refresh-dot"></span>
              {t('forks.refreshing')}
              <span class="cancel-x" aria-hidden="true"><X size={12} /></span>
            </button>
          {:else}
            {t('forks.modeLine', { mode: forkMode(status?.mode) })}
          {/if}
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
        <ArrowUpDown size={12} aria-hidden="true" /> {sortBy === 'behind' ? t('forks.sortBehind') : t('forks.sortName')}
      </button>
    </div>
    <div class="card-grid">
      {#each actionableRepos as repo (repo.Path)}
        <div data-highlight-id={repo.Name ? `repo:${repo.Name}` : undefined}>
        <ForkRepoCard
          {repo}
          anyRunning={forkSweepRunning}
          run={forkRuns[repo.Path]}
          onAction={(a, p, l) => onAction(a, p, l)}
          onCancel={() => onCancelFork?.(repo.Path)}
          {onOpenSession}
          {profiles}
          refreshing={running === 'forks'}
        />
        </div>
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
    <EmptyState icon={GitFork} title={t('forks.emptyTitle')} description={t('forks.emptyHint')} />
  {/if}

  {#if notForks.length}
    <section class="mt-sw-6">
      <button
        class="mb-sw-2 flex items-center gap-sw-2 text-sw-sm font-semibold text-sw-text-secondary hover:text-sw-text"
        onclick={() => (nfOpen = !nfOpen)}
        title={t('forks.notForksTip')}
      >
        <span class="text-sw-text-muted">{nfOpen ? '▾' : '▸'}</span>
        {t('forks.notForksHeading', { n: notForks.length })}
      </button>
      {#if nfOpen}
        <div class="card-grid">
          {#each notForks as repo (repo.Path)}
            <div data-highlight-id={repo.Name ? `repo:${repo.Name}` : undefined}>
              <ForkRepoCard
                {repo}
                anyRunning={forkSweepRunning}
                run={forkRuns[repo.Path]}
                onAction={(a, p, l) => onAction(a, p, l)}
                onCancel={() => onCancelFork?.(repo.Path)}
                {onOpenSession}
                {profiles}
                refreshing={running === 'forks'}
              />
            </div>
          {/each}
        </div>
      {/if}
    </section>
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
                <span class="font-medium truncate" title={g.description || g.name}>{g.name}</span>
              {:else if col.key === 'full'}
                <span class="font-mono text-sw-xs text-sw-text-muted truncate block" title={g.nameWithOwner}>{g.nameWithOwner}</span>
              {:else if col.key === 'language'}
                <span class="text-sw-xs text-sw-text-muted truncate block">{g.language}</span>
              {:else if col.key === 'stars'}
                <span class="text-sw-xs text-sw-text-muted tabular-nums whitespace-nowrap">{g.stars > 0 ? `★ ${g.stars}` : ''}</span>
              {:else if col.key === 'updated'}
                <span class="text-sw-xs text-sw-text-muted whitespace-nowrap" title={fmtTime(g.updatedAt)}>{relTime(g.updatedAt) || fmtTime(g.updatedAt)}</span>
              {:else if col.key === 'kind'}
                <span class="flex flex-wrap gap-sw-1">
                  {#if g.isArchived}<span class="badge badge-warn" title={t('forks.ghArchivedTip')}>{t('forks.ghArchived')}</span>{/if}
                  {#if g.isPrivate}<span class="badge badge-warn" title={t('forks.ghPrivateTip')}>{t('forks.ghPrivate')}</span>{/if}
                  <span class="badge badge-muted">{g.isFork ? t('forks.badgeFork') : t('forks.badgeOwn')}</span>
                </span>
              {:else if col.key === 'actions'}
                <span class="flex gap-sw-1 whitespace-nowrap">
                  <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onOpenUrl?.(g.url)} title={t('forks.ghOpenTip')}>{t('forks.ghOpenShort')}</button>
                  {#if onClone}
                    <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={cloningRepo === g.nameWithOwner}
                      onclick={() => onClone(g)} title={t('forks.ghCloneTip')}>
                      {cloningRepo === g.nameWithOwner ? t('common.busy') : t('forks.ghCloneShort')}
                    </button>
                  {/if}
                </span>
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

<style>
  /* Fork cards: (1) equal-height per row — the shared .card-grid sets align-items:start, so a card
     with more badges/actions stood taller than its row-mates (uneven grid); (2) wider min column —
     at 280px a long health badge (e.g. "разошлась с оригиналом") squeezed the repo name to "Free…".
     Scoped to the forks tab so other card-grids (Home/Analytics/Providers) are untouched. */
  .card-grid {
    align-items: stretch;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  }

  /* Live refresh status — the whole chip is a click target that stops the refresh (the cancel ✕
     used to be opacity:0 until hover, so it read as "no cancel exists"). Now always visible. */
  .refresh-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--sw-accent-text);
    border: none;
    background: transparent;
    font: inherit;
    cursor: pointer;
    border-radius: 9999px;
    padding: 1px 4px;
    transition: color 0.15s;
  }
  .refresh-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--sw-accent-text);
    animation: refreshpulse 1s ease-in-out infinite;
  }
  @keyframes refreshpulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.3;
    }
  }
  /* The pulsing dot inside the header "Cancel" button. */
  .forks-cancel-btn .refresh-dot {
    margin-right: 2px;
  }
  .cancel-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    color: var(--sw-text-muted);
    font-size: 10px;
    line-height: 1;
    transition:
      color 0.15s,
      border-color 0.15s;
  }
  .refresh-chip:hover {
    color: var(--sw-text);
  }
  .refresh-chip:hover .cancel-x {
    color: var(--sw-danger);
    border-color: var(--sw-danger);
  }
</style>
