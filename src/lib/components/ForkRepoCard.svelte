<script lang="ts">
  import type { ForkRepo, ForkAction, SessionTool } from '$lib/ipc';
  import { openTerminal, openUrl, confFiles } from '$lib/ipc';
  import { pConflict, pBranch, pCommit, outcomeLabel, t } from '$lib/i18n';
  import DropdownMenu from './DropdownMenu.svelte';
  import { copyText } from '$lib/clipboard';
  import { relTime } from '$lib/relativeTime';
  import { statusTextClass } from '$lib/statusColor';

  let {
    repo,
    anyRunning,
    run,
    onAction,
    onCancel,
    onOpenSession,
    profiles = [],
    refreshing = false
  }: {
    repo: ForkRepo;
    anyRunning: boolean;
    run?: { line: string; running: boolean; code: number | null };
    onAction: (action: ForkAction, path: string, label: string) => void;
    onCancel?: () => void;
    onOpenSession?: (path: string, tool?: SessionTool, profile?: string) => void;
    profiles?: string[];
    // A whole-stack forks "check" is in flight: this card's status is being refreshed.
    refreshing?: boolean;
  } = $props();

  let open = $state(false);
  let copied = $state(false);

  // This repo's own run state (concurrent, independent of other repos).
  const busy = $derived(!!run?.running);
  const lastCode = $derived(run && !run.running ? run.code : null);

  const branches = $derived(repo.branches ?? []);
  const conflictBranches = $derived(branches.filter((b) => b.outcome === 'conflict'));

  // conflictFiles arrives as string | string[] | null — normalize with the shared confFiles().

  // Auto-assemble ONE AI prompt from whatever is actually wrong with this repo. Every detected
  // problem (mid-op, detached HEAD, branch conflicts, diverged/behind default, dirty tree, upstream
  // rename/archive, redundant/behind wip-local…) contributes a line, so combinations are covered by
  // construction and no hand-off state is left without a prompt. References derived state declared
  // further down — fine because this only runs on click, long after those are initialized.
  function buildPrompt(): string {
    const dash = t('common.dash');
    const branch = repo.defaultBranch ?? 'main';
    const issues: string[] = [];

    if (repo.midOp) issues.push(t('forks.promptIssueMidOp', { op: repo.opName ?? t('forks.healthOpName') }));
    if (repo.detached) issues.push(t('forks.promptIssueDetached'));

    if (conflictBranches.length) {
      issues.push(t('forks.promptIssueConflicts', { branch }));
      for (const b of conflictBranches) {
        const files = confFiles(b.conflictFiles);
        issues.push(
          '  ' +
            t('forks.promptBranchLine', { name: b.name }) +
            (b.prNumber ? t('forks.promptPrSuffix', { n: b.prNumber }) : '') +
            (files.length ? t('forks.promptConflictFiles', { files: files.join(', ') }) : '')
        );
      }
    }

    const behind = repo.behindBy ?? 0;
    if (behind > 0 && !repo.ffSafe)
      issues.push(t('forks.promptIssueDiverged', { branch, behind, ahead: repo.defaultAhead ?? 0 }));
    else if (behind > 0) issues.push(t('forks.promptIssueBehind', { branch, behind }));
    else if ((repo.defaultAhead ?? 0) > 0) issues.push(t('forks.promptIssueDefaultAhead', { n: repo.defaultAhead ?? 0 }));

    if (repo.dirty) issues.push(t('forks.promptIssueDirty'));
    if (repo.untracked) issues.push(t('forks.promptIssueUntracked'));

    if (upstreamRenamed)
      issues.push(t('forks.promptIssueUpstreamRenamed', { branch: repo.upstreamDefaultBranch ?? '', old: branch }));
    if (repo.upstreamArchived) issues.push(t('forks.promptIssueUpstreamArchived'));

    if (wipRedundant) issues.push(t('forks.promptIssueWipRedundant'));
    else if (wipBehind > 0) issues.push(t('forks.promptIssueWipBehind', { n: wipBehind }));

    if (!issues.length) issues.push(t('forks.promptIssueNone'));

    const onBranch =
      repo.currentBranch && repo.currentBranch !== repo.defaultBranch
        ? [t('forks.promptOnBranch', { branch: repo.currentBranch })]
        : [];

    return [
      t('forks.promptRepo', { name: repo.Name, path: repo.Path }),
      t('forks.promptRemotes', { upstream: repo.upstream ?? dash, fork: repo.fork ?? dash, branch }),
      ...onBranch,
      '',
      t('forks.promptTaskGeneric'),
      '',
      t('forks.promptSituation'),
      ...issues,
      '',
      t('forks.promptInstructionsGeneric', { branch })
    ].join('\n');
  }

  async function flashCopy(text: string) {
    if (await copyText(text)) {
      copied = true;
      setTimeout(() => (copied = false), 1500);
    }
  }
  const copyPrompt = () => flashCopy(buildPrompt());
  const isDirty = $derived(repo.dirty || repo.untracked);
  const hasMerged = $derived(branches.some((b) => b.outcome === 'merged'));
  const hasClean = $derived(branches.some((b) => b.outcome === 'clean'));
  const safeTree = $derived(!repo.midOp && !repo.detached);
  // How far the personal wip-local integration branch trails upstream (separate from the
  // default branch's behindBy — a repo sitting on wip-local can be behind while main is synced).
  const wipBehind = $derived(repo.wipLocal?.behindBy ?? 0);
  const wipMerged = $derived(repo.wipLocal?.mergedPatches ?? 0);
  const wipExists = $derived(!!repo.wipLocal);
  // Unique commits in wip-local that are NOT yet upstream (git cherry '+'). 0 ⇒ the branch holds
  // nothing new ⇒ it's redundant and can be deleted instead of endlessly "synced".
  const wipUnique = $derived(repo.wipLocal?.uniquePatches ?? null);
  const wipRedundant = $derived(wipExists && wipUnique === 0);
  // " · "-joined detail line for the expanded view (built in script to avoid Svelte
  // whitespace-collapsing the separator between inline {#if} blocks).
  const wipDetail = $derived(
    [
      wipRedundant
        ? t('forks.wipRedundantRow')
        : wipUnique && wipUnique > 0
          ? t('forks.wipUniqueRow', { n: wipUnique })
          : null,
      wipBehind > 0 ? t('forks.wipBehindRow', { n: wipBehind, commits: pCommit(wipBehind) }) : null,
      wipMerged > 0 ? t('forks.wipMergedPatches', { n: wipMerged }) : null
    ]
      .filter(Boolean)
      .join(' · ')
  );

  const canFf = $derived((repo.behindBy ?? 0) > 0 && repo.ffSafe && !repo.dirty && safeTree);
  const canDelete = $derived(hasMerged && safeTree);
  const canRebase = $derived(!repo.dirty && hasClean && safeTree);
  const canNormalize = $derived(safeTree);
  // wip-local can be synced (rebased onto fresh upstream) when it trails and the tree is safe.
  const canSyncWip = $derived(wipBehind > 0 && !repo.dirty && safeTree);
  // wip-local can be DELETED when it holds no unique commits (redundant) and the tree is safe.
  const canDeleteWip = $derived(wipRedundant && !repo.dirty && safeTree);

  // GitHub compare URL (original's default ... your fork's <branch>) — for "compare on GitHub" and
  // "contribute back" (?expand=1 opens the PR-to-upstream form). null when remotes aren't resolved.
  function compareUrl(branch: string): string | null {
    if (!repo.parentOwnerRepo || !repo.forkOwnerRepo || !repo.defaultBranch || !branch) return null;
    const forkOwner = repo.forkOwnerRepo.split('/')[0];
    return `https://github.com/${repo.parentOwnerRepo}/compare/${repo.defaultBranch}...${forkOwner}:${branch}`;
  }
  const repoCompareUrl = $derived(compareUrl(repo.defaultBranch ?? ''));
  // Upstream renamed its default branch (e.g. master→main) and the fork still tracks the old one.
  const upstreamRenamed = $derived(
    !!repo.upstreamDefaultBranch && !!repo.defaultBranch && repo.upstreamDefaultBranch !== repo.defaultBranch
  );

  // Single recommended next action for this repo (the "what do I do now" answer).
  const rec = $derived.by(() => {
    if (repo.midOp || repo.detached)
      return { key: 'manual', plain: t('forks.recManualPlain'), label: copied ? t('forks.recCopyCopied') : t('forks.recCopyLabel'), tip: t('forks.recManualTip'), run: copyPrompt, disabled: false };
    if (conflictBranches.length)
      return { key: 'conflict', plain: t('forks.recConflictPlain', { n: conflictBranches.length }), label: copied ? t('forks.recCopyCopied') : t('forks.recCopyLabel'), tip: t('forks.recConflictTip'), run: copyPrompt, disabled: false };
    if (!repo.isOwn && canFf)
      return { key: 'ff', plain: t('forks.recFfPlain', { n: repo.behindBy ?? 0, commits: pCommit(repo.behindBy ?? 0) }), label: t('forks.recFfLabel'), tip: ffTip(), run: () => onAction('ff', repo.Path, t('forks.labelFf', { name: repo.Name, branch: repo.defaultBranch ?? '' })), disabled: anyRunning || busy };
    if (!repo.isOwn && canDelete)
      return { key: 'delete', plain: t('forks.recDeletePlain'), label: t('forks.recDeleteLabel'), tip: delTip, run: () => onAction('delete', repo.Path, t('forks.labelDelete', { name: repo.Name })), disabled: anyRunning || busy };
    // wip-local with NO unique commits is redundant — recommend deleting it (beats "sync forever").
    if (canDeleteWip)
      return { key: 'delwip', plain: t('forks.recDeleteWipPlain'), label: t('forks.recDeleteWipLabel'), tip: t('forks.recDeleteWipTip'), run: () => onAction('delete-wip', repo.Path, t('forks.labelDeleteWip', { name: repo.Name })), disabled: anyRunning || busy };
    if (canSyncWip)
      return { key: 'syncwip', plain: t('forks.recSyncWipPlain', { n: wipBehind }), label: t('forks.recSyncWipLabel'), tip: t('forks.recSyncWipTip'), run: () => onAction('sync-wip', repo.Path, t('forks.labelSyncWip', { name: repo.Name })), disabled: anyRunning || busy };
    // Default branch diverged from upstream (behind but NOT ff-able) — no safe automated action;
    // hand it to an AI agent via a tailored prompt (terminal stays as the secondary button).
    if (!repo.isOwn && (repo.behindBy ?? 0) > 0 && !repo.ffSafe && safeTree && !repo.dirty)
      return { key: 'diverged', plain: t('forks.recDivergedPlain'), label: copied ? t('forks.recCopyCopied') : t('forks.recCopyLabel'), tip: t('forks.recDivergedTip'), run: copyPrompt, disabled: false };
    // Fallback for repos with local work but no sync action: uncommitted/untracked changes.
    if (isDirty)
      return { key: 'dirty', plain: t('forks.recDirtyPlain'), label: copied ? t('forks.recCopyCopied') : t('forks.recCopyLabel'), tip: t('forks.recDirtyTip'), run: copyPrompt, disabled: false };
    return null;
  });

  const health = $derived.by(() => {
    if (repo.Skipped === 'error') return { label: t('forks.healthAnalysisError'), cls: 'badge-err', tip: t('forks.healthAnalysisErrorTip') };
    if (repo.Skipped) return { label: repo.Skipped, cls: 'badge-muted', tip: t('forks.healthSkippedTip') };
    if (repo.midOp) return { label: repo.opName ?? t('forks.healthOpName'), cls: 'badge-warn', tip: t('forks.healthOpTip') };
    if (repo.detached) return { label: t('forks.healthDetached'), cls: 'badge-warn', tip: t('forks.healthDetachedTip') };
    const conflicts = branches.filter((b) => b.outcome === 'conflict').length;
    if (conflicts > 0) return { label: `${conflicts} ${pConflict(conflicts)}`, cls: 'badge-warn', tip: t('forks.healthConflictTip') };
    if ((repo.behindBy ?? 0) > 0) {
      const n = repo.behindBy ?? 0;
      // Behind + ff-able → safe pull; behind + NOT ff-able → diverged (needs a manual rebase).
      return repo.ffSafe
        ? { label: t('forks.healthBehind', { n, commits: pCommit(n) }), cls: 'badge-info', tip: t('forks.healthBehindTip', { n }) }
        : { label: t('forks.healthDiverged'), cls: 'badge-warn', tip: t('forks.healthDivergedTip') };
    }
    if (wipBehind > 0) return { label: t('forks.wipBehind', { n: wipBehind, commits: pCommit(wipBehind) }), cls: 'badge-info', tip: t('forks.wipBehindTip', { n: wipBehind, commits: pCommit(wipBehind) }) };
    return { label: t('forks.healthClean'), cls: 'badge-ok', tip: t('forks.healthCleanTip') };
  });

  function prBadge(state: string | null) {
    switch (state) {
      case 'OPEN': return { label: t('forks.prOpen'), cls: 'badge-info' };
      case 'MERGED': return { label: t('forks.prMerged'), cls: 'badge-ok' };
      case 'CLOSED': return { label: t('forks.prClosed'), cls: 'badge-muted' };
      default: return null;
    }
  }

  function ffTip() {
    if (!canFf) {
      if ((repo.behindBy ?? 0) === 0) return t('forks.ffTipNotBehind');
      if (repo.dirty) return t('forks.ffTipDirty');
      if (!repo.ffSafe) return t('forks.ffTipDiverged');
      return t('forks.ffTipUnavailable');
    }
    return t('forks.ffTip', { branch: repo.defaultBranch ?? '' });
  }
  const delTip = $derived(canDelete ? t('forks.delTip') : t('forks.delTipUnavailable'));
  const rebaseTip = $derived(canRebase ? t('forks.rebaseTip') : repo.dirty ? t('forks.rebaseTipDirty') : t('forks.rebaseTipUnavailable'));
  const syncWipTip = $derived(
    canSyncWip
      ? t('forks.syncWipTip')
      : wipBehind === 0
        ? t('forks.syncWipTipSynced')
        : repo.dirty
          ? t('forks.syncWipTipDirty')
          : t('forks.syncWipTipUnavailable')
  );
  const deleteWipTip = $derived(
    canDeleteWip
      ? t('forks.deleteWipTip')
      : !wipExists
        ? t('forks.deleteWipTipNone')
        : (wipUnique ?? -1) > 0
          ? t('forks.deleteWipTipHasUnique', { n: wipUnique ?? 0 })
          : t('forks.deleteWipTipUnavailable')
  );
  const normTip = $derived(t('forks.normTip'));
</script>

<div class="sw-card fork-card flex flex-col gap-sw-2" class:fork-busy={busy} class:fork-refreshing={refreshing && !busy}>
  <div class="flex items-start justify-between gap-sw-2">
    <button class="flex min-w-0 items-center gap-sw-2 text-left" onclick={() => (open = !open)} title={open ? t('forks.collapseTip') : t('forks.expandTip')}>
      <span class="text-sw-text-muted">{open ? '▾' : '▸'}</span>
      <div class="min-w-0">
        <div class="flex items-center gap-sw-2">
          {#if busy}<span class="busy-dot shrink-0" title={t('common.busy')}></span>{/if}
          <h3 class="truncate font-medium" title={repo.Name}>{repo.Name}</h3>
        </div>
        <!-- Clicker-audit #10: long branch names truncate — the full text lives in title. -->
        <p class="truncate text-sw-xs text-sw-text-muted"
          title="{repo.defaultBranch ?? ''}{repo.currentBranch && repo.currentBranch !== repo.defaultBranch ? t('forks.onBranch', { branch: repo.currentBranch }) : ''}">
          <span class="badge {repo.isOwn ? 'badge-muted' : 'badge-info'}" title={repo.isOwn ? t('forks.badgeOwnTip') : t('forks.badgeForkTip')}>{repo.isOwn ? t('forks.badgeOwn') : t('forks.badgeFork')}</span>
          {repo.defaultBranch ?? t('common.dash')}{repo.currentBranch && repo.currentBranch !== repo.defaultBranch ? t('forks.onBranch', { branch: repo.currentBranch }) : ''}
          · {branches.length} {pBranch(branches.length)}
        </p>
      </div>
    </button>
    <span class="badge {health.cls} shrink-0" title={health.tip}>{health.label}</span>
  </div>

  {#if repo.dirty || repo.untracked || repo.rolesGuessed || (repo.defaultAhead ?? 0) > 0 || repo.upstreamArchived || upstreamRenamed}
    <div class="flex flex-wrap gap-sw-2 text-sw-xs">
      {#if repo.upstreamArchived}<span class="badge badge-warn" title={t('forks.badgeUpstreamArchivedTip')}>{t('forks.badgeUpstreamArchived')}</span>{/if}
      {#if upstreamRenamed}<span class="badge badge-warn" title={t('forks.badgeUpstreamRenamedTip', { branch: repo.upstreamDefaultBranch ?? '' })}>{t('forks.badgeUpstreamRenamed', { branch: repo.upstreamDefaultBranch ?? '' })}</span>{/if}
      {#if (repo.defaultAhead ?? 0) > 0}<span class="badge badge-warn" title={t('forks.badgeMainAheadTip', { n: repo.defaultAhead ?? 0 })}>{t('forks.badgeMainAhead', { n: repo.defaultAhead ?? 0 })}</span>{/if}
      {#if repo.dirty}<span class="badge badge-warn" title={t('forks.badgeDirtyTip')}>{t('forks.badgeDirty')}</span>{/if}
      {#if repo.untracked}<span class="badge badge-muted" title={t('forks.badgeUntrackedTip')}>{t('forks.badgeUntracked')}</span>{/if}
      {#if repo.rolesGuessed}<span class="badge badge-muted" title={t('forks.badgeRolesGuessedTip')}>{t('forks.badgeRolesGuessed')}</span>{/if}
    </div>
  {/if}

  {#if open}
    <dl class="space-y-1 border-t border-sw-border pt-sw-2 text-sw-xs text-sw-text-secondary">
      {#if repo.upstream}
        <div class="flex justify-between gap-sw-2"><dt>{t('forks.upstream')}</dt><dd class="truncate text-sw-text" title={t('forks.upstreamTip')}>{repo.upstream}</dd></div>
      {/if}
      {#if repo.fork}
        <div class="flex justify-between gap-sw-2"><dt>{t('forks.fork')}</dt><dd class="truncate text-sw-text" title={t('forks.forkTip')}>{repo.fork}</dd></div>
      {/if}
      {#if repo.upstreamUpdated}
        <div class="flex justify-between gap-sw-2">
          <dt>{t('forks.upstreamUpdated')}</dt>
          <dd class="text-sw-text" title={repo.upstreamUpdated}>{relTime(repo.upstreamUpdated) || repo.upstreamUpdated}</dd>
        </div>
      {/if}
      {#if wipDetail}
        <div class="flex justify-between gap-sw-2">
          <dt>{t('forks.wipLabel')}</dt>
          <dd class="text-sw-text" title={t('forks.wipBehindTip', { n: wipBehind, commits: pCommit(wipBehind) })}>{wipDetail}</dd>
        </div>
      {/if}
    </dl>

    {#if branches.length}
      <ul class="flex flex-col gap-sw-2 border-t border-sw-border pt-sw-2">
        {#each branches as b (b.name)}
          {@const ob = outcomeLabel(b.outcome)}
          {@const pb = prBadge(b.prState)}
          <li class="text-sw-sm">
            <div class="flex flex-wrap items-center gap-sw-2">
              <span class="font-mono text-sw-text">{b.name}</span>
              <span class="badge {ob.cls}" title={t('forks.outcomeTip')}>{ob.label}</span>
              {#if pb}
                {#if b.url}
                  <!-- openUrl, not <a target=_blank>: nothing routes a WebView2 new-window request to
                       the system browser, and open_url is the scheme-guarded path every other link uses. -->
                  <button type="button" class="badge {pb.cls} cursor-pointer hover:underline" onclick={() => b.url && openUrl(b.url)} title={t('forks.prLinkTip')}>{pb.label}{b.prNumber ? ` #${b.prNumber}` : ''}</button>
                {:else}
                  <span class="badge {pb.cls}">{pb.label}{b.prNumber ? ` #${b.prNumber}` : ''}</span>
                {/if}
              {/if}
              {#if b.aheadOfUpstream && b.aheadOfUpstream > 0}<span class="text-sw-xs text-sw-text-muted" title={t('forks.branchAheadTip', { n: b.aheadOfUpstream })}>{t('forks.branchAhead', { n: b.aheadOfUpstream })}</span>{/if}
              {#if b.checks && b.checks !== 'none'}<span class="text-sw-xs text-sw-text-muted" title={t('forks.ciTip')}>{t('forks.ciLabel', { checks: b.checks })}</span>{/if}
            </div>
            {#if b.action}<p class="text-sw-xs text-sw-text-muted">{b.action}</p>{/if}
            {#if (b.aheadOfUpstream ?? 0) > 0 && (b.outcome === 'clean' || b.outcome === 'local-only')}
              {@const cu = compareUrl(b.name)}
              {#if cu}<button type="button" class="text-sw-xs cursor-pointer hover:underline" style="color:var(--sw-accent-text)" onclick={() => openUrl(`${cu}?expand=1`)} title={t('forks.contributeTip')}>{t('forks.contribute')} ↗</button>{/if}
            {/if}
            {#if confFiles(b.conflictFiles).length}
              <p class="text-sw-xs {statusTextClass('bad')}">{t('forks.conflictInFiles', { files: confFiles(b.conflictFiles).join(', ') })}</p>
            {/if}
          </li>
        {/each}
      </ul>
    {:else}
      <p class="border-t border-sw-border pt-sw-2 text-sw-xs text-sw-text-muted">{t('forks.noTopicBranches')}</p>
    {/if}
  {/if}

  {#if !repo.Skipped}
    <div class="flex flex-col gap-sw-2 border-t border-sw-border pt-sw-2">
      {#if rec}
        <p class="text-sw-xs text-sw-text-secondary">
          {t('forks.recommended')} <span class="font-medium text-sw-text">{rec.plain}</span>
        </p>
      {/if}
      <div class="flex flex-wrap items-center gap-sw-2">
        {#if rec}
          <button class="sw-btn {rec.key === 'delete' ? 'sw-btn-danger' : 'sw-btn-primary'} text-sw-xs" disabled={rec.disabled} title={rec.tip} onclick={rec.run}>
            {rec.label}
          </button>
        {/if}
        <DropdownMenu
          label={t('forks.terminal')}
          title={t('forks.terminalTip')}
          items={[
            ...profiles.map((p) => ({ label: `claude · ${p}`, onClick: () => onOpenSession?.(repo.Path, 'claude', p) })),
            { label: 'opencode', onClick: () => onOpenSession?.(repo.Path, 'opencode') },
            { label: 'codex', onClick: () => onOpenSession?.(repo.Path, 'codex') },
            { label: 'shell', onClick: () => onOpenSession?.(repo.Path, 'shell') }
          ]}
        />
        <DropdownMenu
          title={t('forks.moreActionsTip')}
          items={[
            { label: t('forks.recCopyLabel'), title: t('forks.actionCopyPromptTip'), onClick: copyPrompt },
            { label: t('forks.externalTerminal'), title: t('forks.externalTerminalTip'), onClick: () => openTerminal(repo.Path) },
            ...(repo.isOwn
              ? []
              : [
                  { label: t('forks.compareGithub'), title: t('forks.compareGithubTip'), disabled: !repoCompareUrl, onClick: () => repoCompareUrl && openUrl(repoCompareUrl) },
                  { label: t('forks.actionFf'), title: ffTip(), disabled: anyRunning || busy || !canFf, onClick: () => onAction('ff', repo.Path, t('forks.labelFf', { name: repo.Name, branch: repo.defaultBranch ?? '' })) },
                  { label: t('forks.actionDelete'), title: delTip, disabled: anyRunning || busy || !canDelete, danger: true, onClick: () => onAction('delete', repo.Path, t('forks.labelDelete', { name: repo.Name })) },
                  { label: t('forks.actionRebase'), title: rebaseTip, disabled: anyRunning || busy || !canRebase, onClick: () => onAction('rebase', repo.Path, t('forks.labelRebase', { name: repo.Name })) },
                  { label: t('forks.actionSyncWip'), title: syncWipTip, disabled: anyRunning || busy || !canSyncWip, onClick: () => onAction('sync-wip', repo.Path, t('forks.labelSyncWip', { name: repo.Name })) },
                  { label: t('forks.actionDeleteWip'), title: deleteWipTip, disabled: anyRunning || busy || !canDeleteWip, danger: true, onClick: () => onAction('delete-wip', repo.Path, t('forks.labelDeleteWip', { name: repo.Name })) },
                  { label: t('forks.actionPrune'), title: t('forks.pruneTip'), disabled: anyRunning || busy || !safeTree, danger: true, onClick: () => onAction('prune', repo.Path, t('forks.labelPrune', { name: repo.Name })) },
                  { label: t('forks.actionNormalize'), title: normTip, disabled: anyRunning || busy || !canNormalize, onClick: () => onAction('normalize', repo.Path, t('forks.labelNormalize', { name: repo.Name })) }
                ])
          ]}
        />
      </div>
      {#if run}
        <div class="flex items-center gap-sw-2 text-sw-xs">
          {#if busy}
            <span class="fork-spin" aria-hidden="true"></span>
            <span class="min-w-0 flex-1 truncate text-sw-text-secondary" title={run.line}>{run.line || t('forks.runStarting')}</span>
            {#if onCancel}<button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" onclick={onCancel} title={t('forks.runCancelTip')}>{t('forks.runCancel')}</button>{/if}
          {:else}
            <span class="shrink-0 {lastCode === 0 ? statusTextClass('ok') : statusTextClass('bad')}">{lastCode === 0 ? '✓' : '✗'}</span>
            <span class="min-w-0 flex-1 truncate text-sw-text-muted" title={run.line}>{lastCode === 0 ? t('forks.runDone') : t('forks.runFailed', { code: lastCode ?? -1 })}</span>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .fork-spin {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    border-radius: 9999px;
    border: 2px solid var(--sw-border);
    border-top-color: var(--sw-accent);
    animation: fork-spin 0.7s linear infinite;
  }
  @keyframes fork-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* This repo is being mutated right now (ff/delete/rebase/sync) — strong accent glow. */
  .fork-busy {
    border-color: var(--sw-accent-text);
    box-shadow:
      0 0 0 1px var(--sw-accent-text),
      0 0 14px -2px var(--sw-accent-glow);
  }
  .busy-dot {
    width: 8px;
    height: 8px;
    border-radius: 9999px;
    background: var(--sw-accent-text);
    animation: busypulse 1s ease-in-out infinite;
  }
  /* A whole-stack check is in flight: instead of greying the card out (the old opacity:0.4 +
     staggered "wave" reveal that looked janky), the card stays fully readable and a soft accent
     light sweeps across it — the familiar "loading" shimmer. Non-interactive while stale.
     IMPORTANT: the shimmer lives on a `::after` overlay animating only `background-position` —
     NOT a `filter`/`transform` on the card itself, which would make the card a containing block
     and break the anchored "⋯" popover. See $lib/floating.ts. */
  .fork-refreshing {
    pointer-events: none;
  }
  .fork-refreshing::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    pointer-events: none;
    background: linear-gradient(
      105deg,
      transparent 38%,
      var(--sw-accent-glow) 50%,
      transparent 62%
    );
    background-size: 220% 100%;
    animation: fork-shimmer 1.3s ease-in-out infinite;
  }
  @keyframes fork-shimmer {
    0% {
      background-position: 130% 0;
    }
    100% {
      background-position: -30% 0;
    }
  }
  @keyframes busypulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.4;
      transform: scale(0.8);
    }
  }
</style>
