export default {
  // Header
  title: 'MCP 服务器',
  subtitle: '真实来源 (config/.mcp.json) 与按配置部署',
  refreshTitle: '重新读取配置并刷新矩阵（只读）',
  refreshing: '处理中…',
  refresh: '刷新',
  deployTitle:
    '将 config/.mcp.json 中的所有服务器部署到每个配置（用户范围，幂等）。会填充灰色单元格。',
  deployAll: '部署到所有配置',
  selectProfiles: '选择配置：',
  selectProfileTip: '切换「{p}」以批量部署',
  bulkDeploy: '部署到所选',
  bulkDeployTip: '将所有 MCP 服务器部署到选中的配置',

  // Server card
  commandTitle: '服务器启动命令',
  pluginBadge: '来自插件',
  pluginBadgeTitle: '该服务器来自插件市场——不会部署到配置中',
  deployedCountTitle: '已部署到 {total} 个配置中的 {n} 个',
  pluginNote: '通过插件市场全局提供——不部署到配置中（这是正常的）。',
  profileDeployedTitle: '已部署到配置 {p}',
  profileNotDeployedTitle: '未部署到 {p}——点击「部署到所有配置」以添加',
  deployToProfileTip: '将所有 MCP 服务器部署到配置 {p}',

  // Empty state
  emptyTitle: '无数据',
  emptyHint: '未找到 config/.mcp.json 或文件为空。',

  // Extras (out of source of truth)
  extrasHeading: '真实来源之外',
  extrasNote: '在配置中找到但不在 config/.mcp.json 中的服务器：',
  extrasProfileTitle: '存在于此配置中，但不在真实来源中'
};
