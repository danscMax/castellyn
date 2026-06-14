export default {
  // Core helper keys consumed by forkMode() / outcomeLabel() in the i18n module.
  mode_readonly: 'read-only',
  mode_readonly_nofetch: 'read-only (no fetch)',
  mode_dryrun: 'plan preview',
  mode_apply: 'applying changes',
  outcome_merged: 'merged',
  outcome_clean: 'clean',
  outcome_conflict: 'conflict',
  outcome_closed_unmerged: 'closed without merge',
  outcome_local_only: 'local only',

  // ── ForksTab: header ──
  title: 'Forks and repositories',
  intro:
    'Tracks your GitHub forks: how far behind the upstream they are, which branches/PRs are merged, where there are conflicts. Actions (fast-forward, rebase, etc.) apply to forks only; your own repositories are shown for status.',
  introGhUnavailable: ' gh unavailable — PRs by heuristic.',
  checkTip: 'Rebuild fork status (read-only, changes nothing)',
  planBtn: 'Show plan',
  planTip:
    'Show the plan of safe actions (dry-run: ff / delete merged / rebase / normalize) — changes nothing',
  ffAllBtn: 'Pull all updates',
  ffAllTip: 'Safely pull updates from upstream for {n} forks (fast-forward only)',
  ffAllNoneTip: 'No forks behind upstream',

  // ── ForksTab: KPIs ──
  kpiRepos: 'repositories',
  kpiReposTip: 'Total managed repositories',
  kpiMerged: 'merged',
  kpiMergedTip: 'Branches merged into upstream',
  kpiOpen: 'open',
  kpiOpenTip: 'Open branches / PRs in progress',
  kpiConflicts: 'conflicts',
  kpiConflictsTip: 'Branches with merge conflicts',
  kpiNeedHands: 'need action',
  kpiNeedHandsTip: 'How many repositories/branches need manual intervention',
  updatedAt: 'updated: {time}',

  // ── ForksTab: empty state ──
  emptyTitle: 'No data',
  emptyHint: 'Click “Check” to collect fork status.',

  // ── ForkRepoCard: AI prompt ──
  promptBranchLine: '- branch “{name}”',
  promptPrSuffix: ' (PR #{n})',
  promptConflictFiles: '; conflicting files: {files}',
  promptRepo: 'Repository: {name}  ({path})',
  promptRemotes: 'upstream: {upstream} | fork: {fork} | default branch: {branch}',
  promptTask: 'Task: resolve merge conflicts with upstream for branches:',
  promptInstructions:
    'For each branch: switch to it, merge/rebase onto fresh upstream/{branch}, carefully resolve conflicts (keeping meaningful changes from both sides), run the build/tests, and commit. Do not force-push without confirmation.',

  // ── ForkRepoCard: recommended action ──
  recManualPlain: 'resolve manually (an unfinished git operation is in progress)',
  recManualLabel: 'Open terminal',
  recManualTip: 'Unfinished git operation / detached HEAD — resolve manually in the terminal',
  recConflictPlain: 'resolve merge conflicts with upstream',
  recConflictCopied: '✓ Prompt copied',
  recConflictLabel: 'Copy AI prompt',
  recConflictTip: 'Copy the ready-made prompt and ask Claude Code to resolve the conflicts',
  recFfPlain: 'pull updates from upstream (behind by {n})',
  recFfLabel: 'Pull from upstream',
  recDeletePlain: 'delete branches already merged into upstream',
  recDeleteLabel: 'Delete merged branches',

  // ── ForkRepoCard: health badge ──
  healthAnalysisError: 'analysis error',
  healthAnalysisErrorTip: 'Failed to analyze the repository',
  healthSkippedTip: 'Repository skipped',
  healthOpName: 'operation',
  healthOpTip: 'An unfinished git operation is in progress — actions are blocked',
  healthDetached: 'detached HEAD',
  healthDetachedTip: 'HEAD is not on a branch (detached) — actions are blocked',
  healthConflictTip: 'Some branches will not merge without manual conflict resolution',
  healthBehind: 'behind by {n}',
  healthBehindTip: 'The default branch is behind upstream by {n} — can fast-forward (FF)',
  healthClean: 'clean',
  healthCleanTip: 'Everything is in sync, no action required',

  // ── ForkRepoCard: PR badges ──
  prOpen: 'PR open',
  prMerged: 'PR merged',
  prClosed: 'PR closed',

  // ── ForkRepoCard: action tips ──
  ffTipNotBehind: 'Unavailable: branch is not behind upstream',
  ffTipDirty: 'Unavailable: there are uncommitted changes',
  ffTipDiverged: 'Unavailable: branch has diverged — fast-forward is impossible',
  ffTipUnavailable: 'Unavailable',
  ffTip: 'Fast-forward: pull “{branch}” to upstream (safe, no merge)',
  delTip: 'Delete branches already merged into upstream (locally and on the fork)',
  delTipUnavailable: 'Unavailable: no merged branches',
  rebaseTip: 'Rebase open branches onto fresh upstream (locally; aborts on conflict)',
  rebaseTipDirty: 'Unavailable: dirty working tree',
  rebaseTipUnavailable: 'Unavailable: no open branches to rebase',
  normTip: 'Normalize remotes to canonical: origin = your fork, upstream = the original',

  // ── ForkRepoCard: card body ──
  collapseTip: 'Collapse details',
  expandTip: 'Show branches and PRs',
  badgeOwn: 'own',
  badgeFork: 'fork',
  badgeOwnTip: 'Your own repository',
  badgeForkTip: "Fork of someone else's repository",
  onBranch: ' · on {branch}',
  badgeDirty: 'modified files',
  badgeDirtyTip: 'There are uncommitted changes in tracked files',
  badgeUntracked: 'untracked',
  badgeUntrackedTip: 'There are new files outside version control',
  badgeRolesGuessed: 'roles by heuristic',
  badgeRolesGuessedTip: 'gh unavailable — remote roles determined by heuristic',
  upstream: 'upstream',
  upstreamTip: 'Original repository',
  fork: 'fork',
  forkTip: 'Your fork',
  outcomeTip: 'Outcome of integrating the branch into upstream',
  prLinkTip: 'Open PR on GitHub',
  ciTip: 'CI check status',
  ciLabel: 'CI: {checks}',
  conflictInFiles: 'conflict in files: {files}',
  noTopicBranches: 'No topic branches.',

  // ── ForkRepoCard: action row ──
  recommended: 'Recommended:',
  terminal: 'Terminal',
  terminalTip: 'Open a terminal in the repository directory (run claude there and work manually)',
  moreActionsTip: 'More fork actions',
  actionFf: 'Pull from upstream',
  actionDelete: 'Delete merged branches',
  actionRebase: 'Rebase onto upstream',
  actionNormalize: 'Fix remote names',

  // ── Labels passed to onAction (shown in confirm/log UI) ──
  labelFf: '{name}: fast-forward “{branch}” to upstream',
  labelDelete: '{name}: delete merged branches (local + fork)',
  labelRebase: '{name}: rebase open branches onto upstream',
  labelNormalize: '{name}: normalize remotes'
};
