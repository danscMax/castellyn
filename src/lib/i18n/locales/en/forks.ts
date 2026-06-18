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
  needHands_one: 'needs action',
  needHands_few: 'need action',
  needHands_many: 'need action',
  kpiNeedHandsTip:
    'Repositories/branches that need your attention: resolve conflicts, sort out uncommitted changes, pull updates. The exact action is on the repository card below.',
  updatedAt: 'updated: {time}',
  modeLine: 'last run: {mode}',
  refreshing: 'refreshing…',
  filterAll: 'All',
  filterForks: 'Forks',
  filterOwn: 'Own',
  filterTip: 'Filter: all repos / forks only / own only',
  sortTip: 'Sort: by name / by how far behind',
  sortName: 'by name',
  sortBehind: 'by behind',
  githubOnlyHeading: 'More on GitHub — not cloned ({n})',
  githubOnlyTip:
    "Your GitHub repos (including private) that aren't cloned locally. Actions are unavailable until you clone.",
  githubOnlyEmptyFilter: 'No repositories match the current filter.',
  ghPrivate: 'private',
  ghPrivateTip: 'Private repository on GitHub',
  ghOpen: 'Open on GitHub',
  ghOpenTip: 'Open the repository page on GitHub',
  ghColName: 'Repository',
  ghColRepo: 'owner/repo',
  ghColKind: 'Kind',
  ghColActions: 'Actions',

  // ── ForksTab: empty state ──
  emptyTitle: 'No data',
  emptyHint: 'Click “Check” to collect fork status.',

  // ── ForkRepoCard: AI prompt ──
  promptBranchLine: '- branch “{name}”',
  promptPrSuffix: ' (PR #{n})',
  promptConflictFiles: '; conflicting files: {files}',
  promptRepo: 'Repository: {name}  ({path})',
  promptRemotes: 'upstream: {upstream} | fork: {fork} | default branch: {branch}',
  promptTask: 'Task: verify and, if needed, resolve merge conflicts with upstream for branches:',
  promptInstructions:
    'First establish the facts — do not trust the status blindly: run `git fetch upstream` and confirm the ref upstream/{branch} exists (if not, check `git remote -v` and use the real tracking branch). For each branch, verify whether a REAL conflict exists against fresh upstream/{branch} via `git merge-tree` (or a trial merge/rebase). If there is no conflict, do NOT invent work: no empty commits, no token merges, no force-push — just report that there is nothing to resolve. If the conflict is real, switch to the branch, merge/rebase onto fresh upstream/{branch}, carefully resolve the conflicts (keeping meaningful changes from both sides), run the build/tests, and commit. Never force-push without confirmation.',
  promptTaskDirty:
    'Task: the working tree has uncommitted changes (and/or new files outside git). Sort them out and finish cleanly.',
  promptInstructionsDirty:
    'Review the changes (git status, git diff). Understand what they are: group related edits into clear commits; throwaway/junk goes into .gitignore or gets deleted. Special case — vendored/auto-synced files (the file header marks it VENDORED/CANON, or a sync tool exists for it): do NOT commit the local copy blindly — first compare it against the canonical source and, if it diverges, update it from the canon (or run the sync tool), and only then commit, otherwise you would freeze a stale version. Run the build/tests if present. Do not force-push and do not push without confirmation. If the purpose of some change is unclear, leave it untouched and report back.',

  // ── ForkRepoCard: recommended action ──
  recManualPlain: 'resolve manually (an unfinished git operation is in progress)',
  recManualLabel: 'Open terminal',
  recManualTip: 'Unfinished git operation / detached HEAD — resolve manually in the terminal',
  recConflictPlain: 'resolve merge conflicts with upstream',
  recConflictCopied: '✓ Prompt copied',
  recConflictLabel: 'Copy AI prompt',
  recConflictTip: 'Copy the ready-made prompt and ask Claude Code to resolve the conflicts',
  recDirtyPlain: 'sort out uncommitted changes',
  recDirtyCopied: '✓ Prompt copied',
  recDirtyLabel: 'Copy AI prompt',
  recDirtyTip: 'Copy the prompt and ask Claude Code to sort out and commit the changes',
  recFfPlain: 'pull updates from upstream (behind by {n} {commits})',
  recFfLabel: 'Pull from upstream',
  recDeletePlain: 'delete branches already merged into upstream',
  recDeleteLabel: 'Delete merged branches',

  // ── ForkRepoCard: health badge ──
  healthAnalysisError: 'analysis error',
  healthAnalysisErrorTip: 'Failed to analyze the repository',
  healthSkippedTip: 'Repository skipped',
  healthOpName: 'operation',
  healthOpTip: 'An unfinished git operation is in progress — actions are blocked',
  healthDetached: 'no branch',
  healthDetachedTip:
    'HEAD is not on a branch (detached HEAD) — actions are blocked, resolve manually in the terminal',
  healthConflictTip: 'Some branches will not merge without manual conflict resolution',
  healthBehind: 'behind by {n} {commits}',
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
  badgeUntracked: 'new files',
  badgeUntrackedTip: 'There are new files outside version control (untracked) — not yet added to git',
  badgeRolesGuessed: 'remotes — guessed',
  badgeRolesGuessedTip:
    'gh unavailable — which remote is the original (upstream) and which is your fork (origin) was guessed by heuristic',
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
  branchAhead: '+{n} ahead of upstream',
  branchAheadTip: 'Commits in this branch beyond upstream: {n}',

  // ── ForkRepoCard: wip-local (personal integration branch) ──
  wipBehind: 'wip-local behind by {n} {commits}',
  wipBehindTip:
    'Your personal wip-local branch is {n} {commits} behind upstream — consider syncing it',
  wipLabel: 'wip-local',
  wipBehindRow: 'behind by {n} {commits}',
  wipMergedPatches: 'patches merged: {n}',

  // ── ForkRepoCard: action row ──
  recommended: 'Recommended:',
  terminal: 'Terminal',
  terminalTip:
    'Open a session in the repo folder: pick the tool (Claude / opencode / shell) and profile (= provider)',
  externalTerminal: 'External terminal (cmd)',
  externalTerminalTip: 'Open a plain system cmd in the repo folder (for manual git operations)',
  moreActionsTip: 'More actions',
  actionFf: 'Pull from upstream',
  actionDelete: 'Delete merged branches',
  actionRebase: 'Rebase onto upstream',
  actionNormalize: 'Fix remote names',

  // ── Labels passed to onAction (shown in confirm/log UI) ──
  labelFf: '{name}: fast-forward “{branch}” to upstream',
  labelDelete: '{name}: delete merged branches (local + fork)',
  labelRebase: '{name}: rebase open branches onto upstream',
  labelNormalize: '{name}: normalize remotes',
  labelSyncWip: '{name}: sync wip-local with upstream',

  // ── ForkRepoCard: sync wip-local action ──
  recSyncWipPlain: 'sync wip-local with the original (behind by {n})',
  recSyncWipLabel: 'Sync wip-local',
  recSyncWipTip:
    'Rebase your personal wip-local branch onto fresh upstream (local, no push; conflict → aborted)',
  actionSyncWip: 'Sync wip-local',
  syncWipTip: 'Rebase wip-local onto fresh upstream (local, no push)',
  syncWipTipSynced: 'Unavailable: wip-local is already in sync',
  syncWipTipDirty: 'Unavailable: there are uncommitted changes',
  syncWipTipUnavailable: 'Unavailable: no wip-local branch',
  runStarting: 'starting…',
  runDone: 'updated',
  runFailed: 'failed (code {code})',
  runCancel: 'Cancel',
  runCancelTip: 'Abort this repository’s update'
};
