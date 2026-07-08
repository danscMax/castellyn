export default {
  // Header
  title: 'MCP servers',
  subtitle: 'Source of truth (config/.mcp.json) and per-profile deployment',
  colName: 'Server',
  colCommand: 'Command',
  colDeployed: 'Deployed',
  colProfiles: 'Profiles',
  colActions: 'Actions',
  refreshTitle: 'Re-read configs and refresh the matrix (read-only)',
  refreshing: 'Working…',
  refresh: 'Refresh',
  deployTitle:
    'Deploy all servers from config/.mcp.json into every profile (user-scope, idempotent). Fills the gray cells.',
  deployAll: 'Deploy to all profiles',
  selectProfiles: 'Select profiles:',
  selectProfileTip: 'Toggle “{p}” for bulk deploy',
  bulkDeploy: 'Deploy to selected',
  bulkDeployTip: 'Deploy all MCP servers to the checked profiles',

  // Server card
  pluginBadge: 'from plugin',
  pluginBadgeTitle: 'Server comes from the plugin marketplace — not deployed into profiles',
  deployedCountTitle: 'Deployed in {n} of {total} profiles',
  pluginNote: 'Global via the plugin marketplace — not deployed into profiles (this is normal).',
  profileDeployedTitle: 'Deployed in profile {p}',
  deployToProfileTip: 'Deploy all MCP servers to profile {p}',

  // Empty state
  emptyTitle: 'No data',
  emptyHint: 'config/.mcp.json not found or empty.',

  // Extras (out of source of truth)
  extrasHeading: 'Outside the source of truth',
  extrasNote: 'Servers found in profiles but missing from config/.mcp.json:',
  removeExtraTitle: 'Remove this server from profile {p}',

  // Add / edit a canonical server
  addServer: 'Add server',
  addServerTitle: 'Add a server to config/.mcp.json',
  editServerTitle: 'Edit this server in config/.mcp.json',
  removeServerTitle: 'Remove this server from config/.mcp.json',
  formName: 'Server name',
  formJson: 'Definition (JSON)',
  errEmptyName: 'A server name is required',
  errBadJson: 'Invalid JSON',
  savedServer: 'Saved server “{name}”',
  removedServer: 'Removed server “{name}”',
  removedExtra: 'Removed “{name}” from {profile}',
  unsavedTitle: 'Unsaved changes',
  unsavedMsg: 'Close the form and lose what you typed?',
  discardEdits: 'Close without saving',
  keepEditing: 'Keep editing'
};
