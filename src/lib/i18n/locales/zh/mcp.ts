export default {
  // Header
  title: 'MCP 服务器',
  subtitle: '真实来源 (config/.mcp.json) 与按配置部署',
  colName: '服务器',
  colCommand: '命令',
  colDeployed: '已部署',
  colProfiles: '配置',
  colActions: '操作',
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
  pluginBadge: '来自插件',
  pluginBadgeTitle: '该服务器来自插件市场——不会部署到配置中',
  deployedCountTitle: '已部署到 {total} 个配置中的 {n} 个',
  pluginNote: '通过插件市场全局提供——不部署到配置中（这是正常的）。',
  profileDeployedTitle: '已部署到配置 {p}',
  deployToProfileTip: '将所有 MCP 服务器部署到配置 {p}',

  // Empty state
  emptyTitle: '无数据',
  emptyHint: '未找到 config/.mcp.json 或文件为空。',

  // Extras (out of source of truth)
  extrasHeading: '真实来源之外',
  extrasNote: '在配置中找到但不在 config/.mcp.json 中的服务器：',
  removeExtraTitle: '从配置 {p} 中移除此服务器',

  // Add / edit a canonical server
  addServer: '添加服务器',
  addServerTitle: '向 config/.mcp.json 添加服务器',
  editServerTitle: '在 config/.mcp.json 中编辑此服务器',
  removeServerTitle: '从 config/.mcp.json 中删除此服务器',
  formName: '服务器名称',
  formJson: '定义（JSON）',
  errEmptyName: '需要服务器名称',
  errBadJson: 'JSON 无效',
  savedServer: '已保存服务器「{name}」',
  removedServer: '已删除服务器「{name}」',
  removedExtra: '已从 {profile} 移除「{name}」',
  unsavedTitle: '未保存的更改',
  unsavedMsg: '关闭表单并丢失已输入的内容？',
  discardEdits: '不保存并关闭',
  keepEditing: '继续编辑'
};
