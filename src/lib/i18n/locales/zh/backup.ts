export default {
  // Header
  title: '备份',
  subtitle: 'Claude Code 配置文件的快照',
  createTitle: '立即为所有配置文件创建新快照',
  makeBackup: '创建备份',
  retention: '保留：',
  retentionTip: '保留多少个最近的快照（备份时清理更旧的）',

  // Freshness badge
  fresh: '最新',
  staling: '即将过期',
  stale: '已过期',
  relToday: '今天',
  relYesterday: '昨天',
  relDaysAgo: '{n} 天前',

  // Status card
  lastBackup: '最近备份',
  lastSnapshot: '最近快照',
  snapshotsWeekly: '快照 / 每周',
  weeklyArchive: '每周归档',

  // Snapshots list
  snapshotsHeading: '快照 ({n})',
  latest: '最新',
  restoreItemTitle: '从此快照恢复配置（会先显示预览）',
  restore: '恢复',
  emptyTitle: '暂无快照',
  emptyHint: '点击「创建备份」以创建第一个。',

  // Restore dialog
  dialogTitle: '从快照恢复',
  profiles: '配置文件',
  profileToggleTip: '将此配置文件纳入恢复；未勾选的配置文件保持不变',
  includeCreds: '恢复凭据（不会覆盖现有凭据）',
  includeCredsTip: '从快照补全缺失的凭据；现有令牌保持不变',
  warn: '真正的恢复会覆盖所选配置文件的实时配置——不可逆。请先预览方案。',
  closeTitle: '关闭且不做更改',
  previewTitle: '预览 (-WhatIf)：显示将被覆盖的内容——不做任何更改',
  showPlan: '显示方案',
  restoreTitle: '从快照恢复所选配置文件（不可逆）',
  restoreNeedsPreview: '请先点击「显示方案」',

  // In-dialog plan summary (human-readable; raw script output tucked under details)
  planWhat: '恢复将执行的操作',
  planProfiles: '将覆盖 {n} 个配置的设置：{list}',
  planCredsOn: '将从快照补全缺失的凭据（保留现有凭据）。',
  planCredsOff: '不会改动凭据。',
  planUntouched: '共享与系统托管文件保持不变。',
  planDetails: '技术输出'
};
