import { invoke } from '@tauri-apps/api/core';

export type Component = {
  id: string;
  name: string;
  group: string;
  lastJson: string | null;
  supportsApply: boolean;
};

export type RunMode = 'check' | 'apply';

export const listComponents = () => invoke<Component[]>('list_components');
export const readStatus = (path: string) => invoke<any>('read_status', { path });
export const runComponent = (id: string, mode: RunMode) =>
  invoke<number>('run_component', { id, mode });
export const cancelRun = () => invoke('cancel_run');

// --- Forks tab ---
export type ForkAction = 'check' | 'plan' | 'ff' | 'delete' | 'rebase' | 'normalize';

export const runForks = (action: ForkAction, path?: string) =>
  invoke<number>('run_forks', { action, path: path ?? null });

export type ForkBranch = {
  name: string;
  prNumber: number | null;
  prState: string | null;
  url: string | null;
  outcome: string | null;
  conflictFiles: string[] | null;
  aheadOfUpstream: number | null;
  cherryPlus: number | null;
  divergedFromForkAhead: number | null;
  checks: string | null;
  action: string | null;
};

export type ForkRepo = {
  Name: string;
  Path: string;
  upstream: string | null;
  fork: string | null;
  defaultBranch: string | null;
  behindBy: number | null;
  ffSafe: boolean;
  dirty: boolean;
  untracked: boolean;
  midOp: boolean;
  opName: string | null;
  detached: boolean;
  currentBranch: string | null;
  isOwn: boolean;
  rolesGuessed: boolean;
  wipLocal: { behindBy: number | null; mergedPatches: number | null } | null;
  branches: ForkBranch[];
  Skipped: string | null;
};

export type ForkStatus = {
  schemaVersion?: number;
  status?: string;
  timestamp?: string;
  generatedAt?: string;
  mode?: string;
  ghAvailable?: boolean;
  durationSec?: number;
  summary?: { repos: number; merged: number; open: number; conflict: number; needHands: number };
  repos?: ForkRepo[];
};

// --- Backup tab ---
export type BackupAction = 'backup' | 'restore-preview' | 'restore';

export type BackupState = {
  lastRun?: string;
  lastManifestHash?: string;
  lastWeekly?: string | null;
  lastSnapshot?: string | null;
};

export type BackupList = {
  snapshots: string[];
  weeklies: string[];
  state: BackupState | null;
};

export type RestoreOpts = {
  timestamp?: string;
  profiles?: string[];
  includeCredentials?: boolean;
};

export const listBackups = () => invoke<BackupList>('list_backups');

export const runBackup = (action: BackupAction, opts: RestoreOpts = {}) =>
  invoke<number>('run_backup', {
    action,
    timestamp: opts.timestamp ?? null,
    profiles: opts.profiles ?? null,
    includeCredentials: opts.includeCredentials ?? null
  });

// --- Profiles tab ---
export type ProfileAction = 'check' | 'clean-conflicts' | 'reinstall' | 'repair';

export type ProfileInfo = {
  name: string;
  description: string;
  color: string;
  exists: boolean;
  credentialsPresent: boolean;
  settingsPresent: boolean;
  sharedLinks: Record<string, string | null>;
  linksIntact: boolean;
};

export type ProfilesStatus = {
  generatedAt?: string;
  profiles?: ProfileInfo[];
  syncConflicts?: { count: number; files: string[] };
};

export const readProfiles = () => invoke<ProfilesStatus | null>('read_profiles');
export const runProfiles = (action: ProfileAction, name?: string) =>
  invoke<number>('run_profiles', { action, name });
export const openProfileDir = (name: string) => invoke('open_profile_dir', { name });
export const openTerminal = (path: string) => invoke('open_terminal', { path });
export const launchProfile = (name: string, mode: 'terminal' | 'vscode') =>
  invoke('launch_profile', { name, mode });

// Profile lifecycle (Manage-Profiles.ps1).
export type ProfileMgmtAction = 'add' | 'remove' | 'rename' | 'recolor' | 'set-links';
export type ProfileConfig = {
  name: string;
  color: string;
  description?: string;
  linkedFolders?: string[] | null;
};
export type ProfilesConfig = {
  schemaVersion?: number;
  sharedFoldersDefault?: string[];
  profiles?: ProfileConfig[];
};
export type ProfileMgmtArgs = {
  action: ProfileMgmtAction;
  name: string;
  newName?: string;
  color?: string;
  description?: string;
  enabled?: string[];
};
export const readProfilesConfig = () => invoke<ProfilesConfig | null>('read_profiles_config');
export const runProfileMgmt = (a: ProfileMgmtArgs) =>
  invoke<number>('run_profile_mgmt', {
    action: a.action,
    name: a.name,
    newName: a.newName,
    color: a.color,
    description: a.description,
    enabled: a.enabled
  });

// --- Providers / engines tab ---
export type EngineStatus = {
  id: string;
  name: string;
  baseUrl: string;
  protocol: string; // 'anthropic' | 'openai'
  port: number;
  dashboardUrl: string;
  hasCommand: boolean;
  router: boolean; // claude-code-router bridge entry
  installed: boolean | null; // router only: is ccr on PATH?
  running: boolean;
};
export type ProfileProvider = {
  name: string;
  baseUrl: string;
  model: string;
  smallModel: string;
  hasToken: boolean;
};
export type ProviderArgs = {
  action: 'set' | 'clear';
  name: string;
  baseUrl?: string;
  token?: string;
  model?: string;
  smallModel?: string;
  keepToken?: boolean;
};
export const readEngines = () => invoke<EngineStatus[]>('read_engines');
export const updateEngine = (id: string, baseUrl: string, port: number) =>
  invoke('update_engine', { id, baseUrl, port });
export const runEngine = (action: 'start' | 'stop', id: string) =>
  invoke<number>('run_engine', { action, id });
export const runRouter = (
  action: 'install' | 'configure',
  backend?: string,
  model?: string,
  name?: string
) => invoke<number>('run_router', { action, backend, model, name });
export const runConnectRouter = (backend: string, model: string, profile: string, name?: string) =>
  invoke<number>('run_connect_router', { backend, model, profile, name });
export const readEngineModels = (baseUrl: string) =>
  invoke<string[]>('read_engine_models', { baseUrl });
export const readProviders = () => invoke<ProfileProvider[]>('read_providers');
export const runProvider = (a: ProviderArgs) =>
  invoke<number>('run_provider', {
    action: a.action,
    name: a.name,
    baseUrl: a.baseUrl,
    token: a.token,
    model: a.model,
    smallModel: a.smallModel,
    keepToken: a.keepToken
  });

// --- Per-profile launch config (lean mode + tool set + context size) ---
export type ProfileLaunch = {
  name: string;
  mode: 'full' | 'lean';
  mcp: string[]; // MCP servers re-included when lean
  claudeMd: boolean;
  tokenAuth: boolean; // true → lean uses --bare; false (OAuth) → --safe-mode
};
export type LaunchConfigStatus = {
  profiles: ProfileLaunch[];
  availableMcp: string[];
};
export const readLaunchConfig = () => invoke<LaunchConfigStatus>('read_launch_config');
export const setLaunchConfig = (name: string, mode: 'full' | 'lean', mcp: string[], claudeMd: boolean) =>
  invoke('set_launch_config', { name, mode, mcp, claudeMd });
export const measureContext = (name: string, lean: boolean) =>
  invoke<number>('measure_context', { name, lean });

// --- Sync tab ---
export type SyncItem = 'history' | 'projects' | 'skills' | 'agents' | 'commands' | 'keybindings';
export type SyncAction = 'query' | 'set';

export type SyncthingStatus = {
  available: boolean;
  version?: string;
  folderId?: string;
  folderLabel?: string;
  folderShared?: boolean;
  state?: string;
  globalBytes?: number;
  needBytes?: number;
  completion?: number;
  connectedDevices?: number;
};

export type SyncStatus = {
  generatedAt?: string;
  items?: Record<SyncItem, boolean>;
  stignoreMatches?: boolean;
  stignoreExists?: boolean;
  syncthing?: SyncthingStatus;
};

export const readSync = () => invoke<SyncStatus | null>('read_sync');
export const runSync = (action: SyncAction, enabled?: string[]) =>
  invoke<number>('run_sync', { action, enabled });

// --- MCP tab ---
export type McpServer = { name: string; command: string; deployedIn: string[] };
export type McpExtra = { name: string; presentIn: string[] };
export type McpStatus = { source: McpServer[]; extras: McpExtra[]; profiles: string[] };

export const readMcp = () => invoke<McpStatus>('read_mcp');
export const runMcp = (action: 'deploy') => invoke<number>('run_mcp', { action });

// --- Schedule tab ---
export type ScheduleTask = {
  id: string;
  label: string;
  tn: string;
  exists: boolean;
  enabled: boolean;
  time: string | null;
  nextRun: string | null;
  defaultTime: string;
};
export type SchedulesStatus = { generatedAt?: string; tasks?: ScheduleTask[] };
export type ScheduleAction = 'enable' | 'disable' | 'run' | 'create' | 'delete';

export const readSchedules = () => invoke<SchedulesStatus | null>('read_schedules');
export const runSchedule = (action: ScheduleAction, id: string, time?: string) =>
  invoke<number>('run_schedule', { action, id, time: time ?? null });

// --- Plugins & Skills tab ---
export type PluginInfo = {
  id: string;
  version: string;
  scope: string;
  enabled: boolean;
  installedAt?: string;
  lastUpdated?: string;
};
export type SkillInfo = { name: string; description: string; version: string; dir: string };
export type PluginAction = 'enable' | 'disable' | 'update';

export type PluginUpdate = { id: string; installed: string; available: string };

export type PluginContents = {
  id: string;
  skills: string[];
  commands: string[];
  agents: string[];
};

export const listPlugins = () => invoke<PluginInfo[]>('list_plugins');
export const listSkills = () => invoke<SkillInfo[]>('list_skills');
export const listPluginUpdates = () => invoke<PluginUpdate[]>('list_plugin_updates');
export const listPluginContents = () => invoke<PluginContents[]>('list_plugin_contents');
export const runPlugin = (action: PluginAction, id: string) =>
  invoke<number>('run_plugin', { action, id });

// --- Settings ---
export type HubConfig = {
  scriptsRoot?: string | null;
  startHidden?: boolean;
  fetchTimeoutSec?: number | null;
  ghTimeoutSec?: number | null;
};
export type AppPaths = { scriptsRoot: string; configPath: string | null; exe: string | null };

export const readConfig = () => invoke<HubConfig>('read_config');
export const writeConfig = (config: HubConfig) => invoke('write_config', { config });
export const appPaths = () => invoke<AppPaths>('app_paths');
export const openPath = (path: string) => invoke('open_path', { path });
export const getAutostart = () => invoke<boolean>('get_autostart');
export const setAutostart = (enabled: boolean) => invoke('set_autostart', { enabled });
