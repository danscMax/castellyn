export default {
  title: '插件与技能',
  subtitle: '已安装的 Claude Code 插件和个人技能',
  refreshTip: '重新读取插件和技能列表',
  refreshing: '处理中…',
  refreshBtn: '刷新列表',

  // Plugins section
  pluginsHeading: '插件 ({count})',
  withUpdateBadge: '{count} 个有更新',
  withUpdateBadgeTip: '有可用更新的插件',
  updateAvailableBadge: '有更新',
  updateAvailableBadgeTip: '可用版本 {version}',
  managedBadge: 'managed',
  managedBadgeTip: '由 managed-settings 在所有配置中强制启用',
  enabledBadge: '已启用',
  disabledBadge: '已禁用',
  enabledTip: '插件已启用',
  disabledTip: '插件已禁用',

  // Plugin contents (details)
  contentsLabel: '组成：',
  contentsToggleTip: '展开/收起插件的技能、命令和代理列表',
  skillsBadge: '{count} 个技能',
  skillsBadgeTip: '插件内的技能',
  commandsBadge: '{count} 个命令',
  commandsBadgeTip: '插件的斜杠命令',
  agentsBadge: '{count} 个代理',
  agentsBadgeTip: '插件的子代理',
  catSkills: '技能',
  catCommands: '命令',
  catAgents: '代理',

  // Plugin actions
  updateBtn: '更新',
  updateBtnTip: '可用版本 {version} — 更新（共享缓存，适用于所有配置）',
  upToDate: '最新',
  upToDateTip: '未发现更新',
  disableBtn: '禁用',
  disableBtnTip:
    '在所有配置中禁用。managed 插件可能在下次会话时被 managed-settings 重新启用',
  enableBtn: '启用',
  enableBtnTip: '在所有配置中启用',
  noPlugins: '未找到插件（或 claude CLI 不可用）。',

  // Skills section
  skillsHeading: '技能 ({count})',
  openSkillsBtn: '打开技能文件夹',
  openSkillsTip: '在文件资源管理器中打开 ~/.claude/skills',
  skillsNote:
    '技能是 ~/.claude/skills 中的文件（由文件和插件管理；没有单独的启用/禁用）。',
  noSkills: '在 ~/.claude/skills 中未找到技能。'
};
