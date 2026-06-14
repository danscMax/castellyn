// Strings owned by the top-level page orchestrator (+page.svelte) and the
// run-outcome mapper (outcome.ts): confirm dialogs, run-log lines, toasts.
export default {
  // Operational display names (toasts / log)
  op_backup: 'Backup',
  op_profiles: 'Profiles',
  op_mcp: 'MCP',
  op_sync: 'Sync',
  op_engine: 'Engine',
  op_provider: 'Provider',
  op_schedule: 'Schedule',
  op_plugins: 'Plugins',

  // Generic run-log lines
  log_component: '▶ {name}: {verb}…',
  log_error: '✖ Error: {e}',
  log_warn: '⚠ {e}',
  log_done: '■ Done (exit code {code}).',
  verb_apply: 'applying',
  verb_check: 'checking',

  // Component apply
  confirm_apply_title: 'Apply update?',
  confirm_apply_msg: 'Component “{name}” will ACTUALLY be updated (-Apply). Continue?',
  confirm_apply_btn: 'Apply',

  // Forks
  forks_verb_check: 'checking',
  forks_verb_plan: 'plan (dry-run)',
  forks_verb_action: 'action “{action}”',
  forks_log: '▶ Forks: {verb}{path}…',
  forks_recheck: '▶ Forks: re-checking…',
  confirm_fork_title: 'Modify fork?',
  confirm_fork_msg: '{label}. This will ACTUALLY change the repository. Continue?',
  confirm_fork_btn: 'Run',
  confirm_batchff_title: 'Pull all updates?',
  confirm_batchff_msg:
    'A safe fast-forward will run for {n} forks: {names}. This is fast-forward only (no merge, no force-push). Continue?',
  confirm_batchff_btn: 'Pull',

  // Backup
  backup_verb_snapshot: 'creating snapshot',
  backup_verb_restore_preview: 'restore plan (-WhatIf)',
  backup_verb_restore: 'restoring',
  backup_log: '▶ Backup: {verb}…',
  backup_snap_last: 'latest',
  confirm_restore_title: 'Restore configs?',
  confirm_restore_msg:
    'Snapshot “{snap}” will overwrite the live configs of the selected profiles — irreversible. Continue?',
  confirm_restore_btn: 'Restore',

  // Profiles
  prof_verb_add: 'adding profile {name}',
  prof_verb_remove: 'removing profile {name}',
  prof_verb_rename: 'renaming {name} → {newName}',
  prof_verb_recolor: 'recoloring {name}',
  prof_verb_setlinks: 'shared folders {name}',
  prof_log: '▶ Profiles: {verb}…',
  prof_verb_check: 'checking',
  prof_verb_clean: 'removing sync conflicts',
  prof_verb_repair: 'repairing links {name}',
  prof_verb_reinstall: 'reinstalling profiles',
  confirm_prof_remove_title: 'Delete profile “{name}”?',
  confirm_prof_remove_msg:
    'The ~/.claude-{name} directory will be deleted along with this profile’s saved login and settings. Shared content (skills/projects, etc.) is unaffected. This is irreversible.',
  confirm_prof_remove_btn: 'Delete',
  confirm_reinstall_title: 'Reinstall profiles?',
  confirm_reinstall_msg:
    'Install-ClaudeProfiles.ps1 -Force will recreate the junctions/symlinks of all profiles and requires administrator rights (UAC). Continue?',
  confirm_reinstall_btn: 'Reinstall',
  confirm_clean_title: 'Remove sync conflicts?',
  confirm_clean_msg:
    'Duplicate *.sync-conflict-* files will be removed (originals are untouched; Syncthing keeps versions). Continue?',
  confirm_clean_btn: 'Delete',

  // MCP
  mcp_log: '▶ MCP: deploying to all profiles…',
  confirm_mcp_title: 'Deploy MCP to all profiles?',
  confirm_mcp_msg:
    'Servers from config/.mcp.json will be added to each profile (user-scope, idempotent). Existing ones are overwritten with the same values. Continue?',
  confirm_mcp_btn: 'Deploy',

  // Sync
  sync_log_set: '▶ Sync: applying settings…',
  sync_log_query: '▶ Sync: checking…',
  sync_apply_off:
    'These will stop syncing across machines: {off} (local files are not deleted). .stignore will be regenerated.',
  sync_apply_all: 'All items will sync. .stignore will be regenerated.',
  confirm_sync_title: 'Apply sync settings?',
  confirm_sync_btn: 'Apply',

  // Engines / providers / router
  engine_log: '▶ Engine {id}: {verb}…',
  engine_verb_start: 'starting',
  engine_verb_stop: 'stopping',
  confirm_engine_stop_title: 'Stop engine?',
  confirm_engine_stop_msg: 'The process listening on engine “{id}”’s port will be stopped. Continue?',
  confirm_engine_stop_btn: 'Stop',
  provider_log: '▶ Provider {name}: {verb}…',
  provider_verb_set: 'binding',
  provider_verb_clear: 'reset',
  confirm_provider_clear_title: 'Reset provider?',
  confirm_provider_clear_msg:
    'Profile “{name}” will return to the standard Anthropic login (the provider env will be cleared). Continue?',
  confirm_provider_clear_btn: 'Reset',
  router_install_log: '▶ Router: installing claude-code-router (npm)…',
  confirm_router_title: 'Connect via router?',
  confirm_router_msg:
    'Profile “{profile}” will be switched to “{engine}” (model “{model}”) via ccr: I’ll configure and start claude-code-router and bind the profile to http://127.0.0.1:3456. Restart the profile afterwards. Continue?',
  confirm_router_btn: 'Connect',
  router_log: '▶ Router: {engine} ({model}) → profile {profile}…',

  // Schedule
  sched_verb_enable: 'enabling',
  sched_verb_disable: 'disabling',
  sched_verb_run: 'running',
  sched_verb_create: 'creating schedule',
  sched_verb_delete: 'deleting schedule',
  sched_log: '▶ Schedule ({id}): {verb}…',
  confirm_sched_delete_title: 'Delete task?',
  confirm_sched_delete_msg: 'Task “{id}” will be removed from the Windows Task Scheduler. Continue?',
  confirm_sched_delete_btn: 'Delete',

  // Plugins
  plugin_verb_update: 'updating',
  plugin_verb_enable: 'enabling',
  plugin_verb_disable: 'disabling',
  plugin_log: '▶ Plugin {id}: {verb}…',
  confirm_plugin_disable_title: 'Disable plugin?',
  confirm_plugin_disable_msg: '“{id}” will be disabled in all profiles. Continue?',
  confirm_plugin_disable_btn: 'Disable',

  // Operational toasts
  toast_op_done: '{name}: done',
  toast_op_error: '{name}: error (code {code})',
  toast_op_error_detail: 'See the run log for details.',
  toast_open_log: 'Open log',

  // Misc
  load_error: 'Load error: {e}',
  wip: 'This section is under construction — coming in a future iteration.',

  // Run outcomes (outcome.ts)
  out_duration: 'in {d}',
  out_sec: '{n}s',
  out_fork_conflicts: '{n} with conflicts',
  out_fork_merged: '{n} branches to delete',
  out_fork_open: '{n} open PRs',
  out_forks_need: 'Forks: action needed — {need}',
  out_forks_synced: 'Forks: all in sync',
  out_open_forks: 'Open Forks',
  out_failed_count: '{name}: need attention — {failed}',
  out_failed_problems: '{name}: has problems',
  out_applied: '{name}: updated',
  out_changes_count: '{name}: updates available — {changed}',
  out_changes_any: '{name}: updates available',
  out_changes_detail: 'Press “Update” on the card.',
  out_uptodate: '{name}: up to date'
};
