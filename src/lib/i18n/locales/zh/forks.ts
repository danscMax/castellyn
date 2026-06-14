export default {
  // Core helper keys consumed by forkMode() / outcomeLabel() in the i18n module.
  mode_readonly: '只读',
  mode_readonly_nofetch: '只读（不 fetch）',
  mode_dryrun: '计划预览',
  mode_apply: '应用更改',
  outcome_merged: '已合入',
  outcome_clean: '干净',
  outcome_conflict: '冲突',
  outcome_closed_unmerged: '已关闭未合并',
  outcome_local_only: '仅本地',

  // ── ForksTab: header ──
  title: '复刻与仓库',
  intro:
    '跟踪你在 GitHub 上的复刻：它们落后上游多少、哪些分支/PR 已合入、哪里有冲突。操作（快进、变基等）仅适用于复刻；你自己的仓库仅用于查看状态。',
  introGhUnavailable: ' gh 不可用 — PR 按启发式判断。',
  checkTip: '重新生成复刻状态（只读，不改动任何内容）',
  planBtn: '显示计划',
  planTip: '显示安全操作计划（dry-run：快进/删除已合入/变基/规范化）— 不改动任何内容',
  ffAllBtn: '拉取全部更新',
  ffAllTip: '为 {n} 个复刻安全地从上游拉取更新（仅快进）',
  ffAllNoneTip: '没有落后上游的复刻',

  // ── ForksTab: KPIs ──
  kpiRepos: '仓库',
  kpiReposTip: '受管理的仓库总数',
  kpiMerged: '已合入',
  kpiMergedTip: '已合入上游的分支',
  kpiOpen: '开放',
  kpiOpenTip: '进行中的开放分支 / PR',
  kpiConflicts: '冲突',
  kpiConflictsTip: '存在合并冲突的分支',
  kpiNeedHands: '需要操作',
  kpiNeedHandsTip: '有多少仓库/分支需要人工干预',
  updatedAt: '更新于：{time}',

  // ── ForksTab: empty state ──
  emptyTitle: '暂无数据',
  emptyHint: '点击“检查”以收集复刻状态。',

  // ── ForkRepoCard: AI prompt ──
  promptBranchLine: '- 分支“{name}”',
  promptPrSuffix: '（PR #{n}）',
  promptConflictFiles: '；冲突文件：{files}',
  promptRepo: '仓库：{name}  （{path}）',
  promptRemotes: 'upstream：{upstream} | 复刻：{fork} | 默认分支：{branch}',
  promptTask: '任务：为以下分支解决与上游的合并冲突：',
  promptInstructions:
    '对每个分支：切换到该分支，合并/变基到最新的 upstream/{branch}，谨慎解决冲突（保留两侧有意义的更改），运行构建/测试，然后提交。未经确认不要强制推送。',

  // ── ForkRepoCard: recommended action ──
  recManualPlain: '手动处理（有未完成的 git 操作在进行）',
  recManualLabel: '打开终端',
  recManualTip: '未完成的 git 操作 / detached HEAD — 请在终端中手动解决',
  recConflictPlain: '解决与上游的合并冲突',
  recConflictCopied: '✓ 提示词已复制',
  recConflictLabel: '复制 AI 提示词',
  recConflictTip: '复制现成的提示词，让 Claude Code 解决冲突',
  recFfPlain: '从上游拉取更新（落后 {n}）',
  recFfLabel: '从上游拉取',
  recDeletePlain: '删除已合入上游的分支',
  recDeleteLabel: '删除已合入的分支',

  // ── ForkRepoCard: health badge ──
  healthAnalysisError: '分析错误',
  healthAnalysisErrorTip: '无法分析该仓库',
  healthSkippedTip: '仓库已跳过',
  healthOpName: '操作',
  healthOpTip: '仓库中有未完成的 git 操作 — 操作已禁用',
  healthDetached: 'detached HEAD',
  healthDetachedTip: 'HEAD 不在分支上（detached）— 操作已禁用',
  healthConflictTip: '有些分支不手动解决冲突就无法合入',
  healthBehind: '落后 {n}',
  healthBehindTip: '默认分支落后上游 {n} — 可以快进（FF）',
  healthClean: '干净',
  healthCleanTip: '一切已同步，无需操作',

  // ── ForkRepoCard: PR badges ──
  prOpen: 'PR 开放',
  prMerged: 'PR 已合入',
  prClosed: 'PR 已关闭',

  // ── ForkRepoCard: action tips ──
  ffTipNotBehind: '不可用：分支未落后上游',
  ffTipDirty: '不可用：存在未提交的更改',
  ffTipDiverged: '不可用：分支已分叉 — 无法快进',
  ffTipUnavailable: '不可用',
  ffTip: '快进：将“{branch}”拉取到上游（安全，无合并）',
  delTip: '删除已合入上游的分支（本地与复刻上）',
  delTipUnavailable: '不可用：没有已合入的分支',
  rebaseTip: '将开放分支变基到最新上游（本地；冲突时中止）',
  rebaseTipDirty: '不可用：工作区不干净',
  rebaseTipUnavailable: '不可用：没有可变基的开放分支',
  normTip: '将 remote 规范化：origin = 你的复刻，upstream = 上游',

  // ── ForkRepoCard: card body ──
  collapseTip: '收起详情',
  expandTip: '显示分支和 PR',
  badgeOwn: '自有',
  badgeFork: '复刻',
  badgeOwnTip: '你自己的仓库',
  badgeForkTip: '他人仓库的复刻',
  onBranch: ' · 在 {branch}',
  badgeDirty: '已修改文件',
  badgeDirtyTip: '受跟踪文件中存在未提交的更改',
  badgeUntracked: '未跟踪',
  badgeUntrackedTip: '存在不在版本控制内的新文件',
  badgeRolesGuessed: '角色按启发式',
  badgeRolesGuessedTip: 'gh 不可用 — remote 角色按启发式判定',
  upstream: 'upstream',
  upstreamTip: '原始仓库',
  fork: '复刻',
  forkTip: '你的复刻',
  outcomeTip: '分支合入上游的结果',
  prLinkTip: '在 GitHub 上打开 PR',
  ciTip: 'CI 检查状态',
  ciLabel: 'CI：{checks}',
  conflictInFiles: '文件冲突：{files}',
  noTopicBranches: '没有主题分支。',

  // ── ForkRepoCard: action row ──
  recommended: '建议：',
  terminal: '终端',
  terminalTip: '在仓库目录中打开终端（可在那里运行 claude 并手动操作）',
  moreActionsTip: '更多复刻操作',
  actionFf: '从上游拉取',
  actionDelete: '删除已合入的分支',
  actionRebase: '变基到上游',
  actionNormalize: '修正 remote 名称',

  // ── Labels passed to onAction (shown in confirm/log UI) ──
  labelFf: '{name}：将“{branch}”快进到上游',
  labelDelete: '{name}：删除已合入的分支（本地 + 复刻）',
  labelRebase: '{name}：将开放分支变基到上游',
  labelNormalize: '{name}：规范化 remotes'
};
