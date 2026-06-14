export default {
  // Header
  title: 'MCP servers',
  subtitle: 'Source of truth (config/.mcp.json) and per-profile deployment',
  refreshTitle: 'Re-read configs and refresh the matrix (read-only)',
  refreshing: 'Working…',
  refresh: 'Refresh',
  deployTitle:
    'Deploy all servers from config/.mcp.json into every profile (user-scope, idempotent). Fills the gray cells.',
  deployAll: 'Deploy to all profiles',

  // Server card
  commandTitle: 'Server launch command',
  pluginBadge: 'from plugin',
  pluginBadgeTitle: 'Server comes from the plugin marketplace — not deployed into profiles',
  deployedCountTitle: 'Deployed in {n} of {total} profiles',
  pluginNote: 'Global via the plugin marketplace — not deployed into profiles (this is normal).',
  profileDeployedTitle: 'Deployed in profile {p}',
  profileNotDeployedTitle: 'NOT deployed in {p} — click “Deploy to all profiles” to add it',

  // Empty state
  emptyTitle: 'No data',
  emptyHint: 'config/.mcp.json not found or empty.',

  // Extras (out of source of truth)
  extrasHeading: 'Outside the source of truth',
  extrasNote: 'Servers found in profiles but missing from config/.mcp.json:',
  extrasProfileTitle: 'Present in this profile, but not in the source of truth'
};
