export default {
  title: 'Plugins and skills',
  subtitle: 'Installed plugins and personal Claude Code skills',
  refreshTip: 'Re-read the plugin and skill lists',
  refreshing: 'Working…',
  refreshBtn: 'Refresh list',

  // Plugins section
  pluginsHeading: 'Plugins ({count})',
  withUpdateBadge: '{count} with update',
  withUpdateBadgeTip: 'Plugins with an available update',
  updateAvailableBadge: 'update available',
  updateAvailableBadgeTip: 'Version {version} available',
  managedBadge: 'managed',
  managedBadgeTip: 'Forced by managed-settings across all profiles',
  enabledBadge: 'enabled',
  disabledBadge: 'disabled',
  enabledTip: 'Plugin enabled',
  disabledTip: 'Plugin disabled',

  // Plugin contents (details)
  contentsLabel: 'Contents:',
  contentsToggleTip: 'Expand/collapse the list of the plugin’s skills, commands and agents',
  skillsBadge: '{count} skills',
  skillsBadgeTip: 'Skills inside the plugin',
  commandsBadge: '{count} commands',
  commandsBadgeTip: 'Plugin slash commands',
  agentsBadge: '{count} agents',
  agentsBadgeTip: 'Plugin subagents',
  catSkills: 'Skills',
  catCommands: 'Commands',
  catAgents: 'Agents',

  // Plugin actions
  updateBtn: 'Update',
  updateBtnTip: 'Version {version} available — update (shared cache, for all profiles)',
  upToDate: 'up to date',
  upToDateTip: 'No updates found',
  disableBtn: 'Disable',
  disableBtnTip:
    'Disable across all profiles. A managed plugin may be re-enabled by managed-settings on the next session',
  enableBtn: 'Enable',
  enableBtnTip: 'Enable across all profiles',
  noPlugins: 'No plugins found (or the claude CLI is unavailable).',

  // Skills section
  skillsHeading: 'Skills ({count})',
  openSkillsBtn: 'Open skills folder',
  openSkillsTip: 'Open ~/.claude/skills in the file explorer',
  skillsNote:
    'Skills are files in ~/.claude/skills (managed by files and plugins; there is no per-skill enable/disable).',
  noSkills: 'No skills found in ~/.claude/skills.'
};
