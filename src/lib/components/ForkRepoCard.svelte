<script lang="ts">
  import type { ForkRepo, ForkAction } from '$lib/ipc';
  import { openTerminal } from '$lib/ipc';
  import { pConflict, pBranch, outcomeLabel, t } from '$lib/i18n';
  import DropdownMenu from './DropdownMenu.svelte';

  let {
    repo,
    anyRunning,
    onAction
  }: {
    repo: ForkRepo;
    anyRunning: boolean;
    onAction: (action: ForkAction, path: string, label: string) => void;
  } = $props();

  let open = $state(false);
  let copied = $state(false);

  const branches = $derived(repo.branches ?? []);
  const conflictBranches = $derived(branches.filter((b) => b.outcome === 'conflict'));

  function aiPrompt(): string {
    const lines = conflictBranches.map(
      (b) =>
        t('forks.promptBranchLine', { name: b.name }) +
        (b.prNumber ? t('forks.promptPrSuffix', { n: b.prNumber }) : '') +
        (b.conflictFiles?.length
          ? t('forks.promptConflictFiles', { files: b.conflictFiles.join(', ') })
          : '')
    );
    return [
      t('forks.promptRepo', { name: repo.Name, path: repo.Path }),
      t('forks.promptRemotes', {
        upstream: repo.upstream ?? t('common.dash'),
        fork: repo.fork ?? t('common.dash'),
        branch: repo.defaultBranch ?? t('common.dash')
      }),
      '',
      t('forks.promptTask'),
      ...lines,
      '',
      t('forks.promptInstructions', { branch: repo.defaultBranch ?? 'main' })
    ].join('\n');
  }

  async function copyPrompt() {
    try {
      await navigator.clipboard.writeText(aiPrompt());
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }
  const hasMerged = $derived(branches.some((b) => b.outcome === 'merged'));
  const hasClean = $derived(branches.some((b) => b.outcome === 'clean'));
  const safeTree = $derived(!repo.midOp && !repo.detached);

  const canFf = $derived((repo.behindBy ?? 0) > 0 && repo.ffSafe && !repo.dirty && safeTree);
  const canDelete = $derived(hasMerged && safeTree);
  const canRebase = $derived(!repo.dirty && hasClean && safeTree);
  const canNormalize = $derived(safeTree);

  // Single recommended next action for this repo (the "what do I do now" answer).
  const rec = $derived.by(() => {
    if (repo.midOp || repo.detached)
      return { key: 'manual', plain: t('forks.recManualPlain'), label: t('forks.recManualLabel'), tip: t('forks.recManualTip'), run: () => openTerminal(repo.Path), disabled: false };
    if (conflictBranches.length)
      return { key: 'conflict', plain: t('forks.recConflictPlain'), label: copied ? t('forks.recConflictCopied') : t('forks.recConflictLabel'), tip: t('forks.recConflictTip'), run: copyPrompt, disabled: false };
    if (!repo.isOwn && canFf)
      return { key: 'ff', plain: t('forks.recFfPlain', { n: repo.behindBy ?? 0 }), label: t('forks.recFfLabel'), tip: ffTip(), run: () => onAction('ff', repo.Path, t('forks.labelFf', { name: repo.Name, branch: repo.defaultBranch ?? '' })), disabled: anyRunning };
    if (!repo.isOwn && canDelete)
      return { key: 'delete', plain: t('forks.recDeletePlain'), label: t('forks.recDeleteLabel'), tip: delTip, run: () => onAction('delete', repo.Path, t('forks.labelDelete', { name: repo.Name })), disabled: anyRunning };
    return null;
  });

  const health = $derived.by(() => {
    if (repo.Skipped === 'error') return { label: t('forks.healthAnalysisError'), cls: 'badge-err', tip: t('forks.healthAnalysisErrorTip') };
    if (repo.Skipped) return { label: repo.Skipped, cls: 'badge-muted', tip: t('forks.healthSkippedTip') };
    if (repo.midOp) return { label: repo.opName ?? t('forks.healthOpName'), cls: 'badge-warn', tip: t('forks.healthOpTip') };
    if (repo.detached) return { label: t('forks.healthDetached'), cls: 'badge-warn', tip: t('forks.healthDetachedTip') };
    const conflicts = branches.filter((b) => b.outcome === 'conflict').length;
    if (conflicts > 0) return { label: `${conflicts} ${pConflict(conflicts)}`, cls: 'badge-warn', tip: t('forks.healthConflictTip') };
    if ((repo.behindBy ?? 0) > 0) return { label: t('forks.healthBehind', { n: repo.behindBy ?? 0 }), cls: 'badge-info', tip: t('forks.healthBehindTip', { n: repo.behindBy ?? 0 }) };
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
  const normTip = $derived(t('forks.normTip'));
</script>

<div class="sw-card flex flex-col gap-sw-2">
  <div class="flex items-start justify-between gap-sw-2">
    <button class="flex min-w-0 items-center gap-sw-2 text-left" onclick={() => (open = !open)} title={open ? t('forks.collapseTip') : t('forks.expandTip')}>
      <span class="text-sw-text-muted">{open ? '▾' : '▸'}</span>
      <div class="min-w-0">
        <div class="flex items-center gap-sw-2">
          <h3 class="truncate font-medium">{repo.Name}</h3>
          <span class="badge {repo.isOwn ? 'badge-muted' : 'badge-info'}" title={repo.isOwn ? t('forks.badgeOwnTip') : t('forks.badgeForkTip')}>{repo.isOwn ? t('forks.badgeOwn') : t('forks.badgeFork')}</span>
        </div>
        <p class="truncate text-sw-xs text-sw-text-muted">
          {repo.defaultBranch ?? t('common.dash')}{repo.currentBranch && repo.currentBranch !== repo.defaultBranch ? t('forks.onBranch', { branch: repo.currentBranch }) : ''}
          · {branches.length} {pBranch(branches.length)}
        </p>
      </div>
    </button>
    <span class="badge {health.cls} shrink-0" title={health.tip}>{health.label}</span>
  </div>

  {#if repo.dirty || repo.untracked || repo.rolesGuessed}
    <div class="flex flex-wrap gap-sw-2 text-sw-xs">
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
                  <a class="badge {pb.cls} hover:underline" href={b.url} target="_blank" rel="noreferrer" title={t('forks.prLinkTip')}>{pb.label}{b.prNumber ? ` #${b.prNumber}` : ''}</a>
                {:else}
                  <span class="badge {pb.cls}">{pb.label}{b.prNumber ? ` #${b.prNumber}` : ''}</span>
                {/if}
              {/if}
              {#if b.checks && b.checks !== 'none'}<span class="text-sw-xs text-sw-text-muted" title={t('forks.ciTip')}>{t('forks.ciLabel', { checks: b.checks })}</span>{/if}
            </div>
            {#if b.action}<p class="text-sw-xs text-sw-text-muted">{b.action}</p>{/if}
            {#if b.conflictFiles?.length}
              <p class="text-sw-xs text-red-400">{t('forks.conflictInFiles', { files: b.conflictFiles.join(', ') })}</p>
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
          <button class="sw-btn sw-btn-primary text-sw-xs" disabled={rec.disabled} title={rec.tip} onclick={rec.run}>
            {rec.label}
          </button>
        {/if}
        {#if rec?.key !== 'manual'}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openTerminal(repo.Path)}
            title={t('forks.terminalTip')}>
            {t('forks.terminal')}
          </button>
        {/if}
        {#if !repo.isOwn}
          <DropdownMenu
            title={t('forks.moreActionsTip')}
            items={[
              { label: t('forks.actionFf'), title: ffTip(), disabled: anyRunning || !canFf, onClick: () => onAction('ff', repo.Path, t('forks.labelFf', { name: repo.Name, branch: repo.defaultBranch ?? '' })) },
              { label: t('forks.actionDelete'), title: delTip, disabled: anyRunning || !canDelete, onClick: () => onAction('delete', repo.Path, t('forks.labelDelete', { name: repo.Name })) },
              { label: t('forks.actionRebase'), title: rebaseTip, disabled: anyRunning || !canRebase, onClick: () => onAction('rebase', repo.Path, t('forks.labelRebase', { name: repo.Name })) },
              { label: t('forks.actionNormalize'), title: normTip, disabled: anyRunning || !canNormalize, onClick: () => onAction('normalize', repo.Path, t('forks.labelNormalize', { name: repo.Name })) }
            ]}
          />
        {/if}
      </div>
    </div>
  {/if}
</div>
