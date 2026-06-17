import { invoke, Channel } from '@tauri-apps/api/core';

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
export type ForkAction = 'check' | 'plan' | 'ff' | 'delete' | 'rebase' | 'sync-wip' | 'normalize';

export const runForks = (action: ForkAction, path?: string) =>
  invoke<number>('run_forks', { action, path: path ?? null });
// Per-repo concurrent run: streams to fork-log / fork-done (component = repo path).
export const runForkRepo = (action: ForkAction, path: string) =>
  invoke<number>('run_fork_repo', { action, path });
export const cancelForkRepo = (path: string) => invoke('cancel_fork_repo', { path });
// Fresh state of one repo after a -Single run (for merging into the list without a full rescan).
export const readForkRepoStatus = (path: string) => invoke<ForkRepo | null>('read_fork_repo_status', { path });

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

// A repo on the user's GitHub account (from `gh repo list`), used to surface repos
// that aren't locally cloned. Reconciled with ForkRepo by name in the Forks tab.
export type GithubRepo = {
  owner: string;
  name: string;
  nameWithOwner: string;
  isPrivate: boolean;
  isFork: boolean;
  url: string;
  updatedAt: string;
};

export const listGithubRepos = () => invoke<GithubRepo[]>('list_github_repos');

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
  keepSnapshots?: number; // backup action only: how many snapshots to retain
};

export const listBackups = () => invoke<BackupList>('list_backups');

export const runBackup = (action: BackupAction, opts: RestoreOpts = {}) =>
  invoke<number>('run_backup', {
    action,
    timestamp: opts.timestamp ?? null,
    profiles: opts.profiles ?? null,
    includeCredentials: opts.includeCredentials ?? null,
    keepSnapshots: opts.keepSnapshots ?? null
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
export type ProfileMgmtAction = 'add' | 'remove' | 'rename' | 'recolor' | 'redescribe' | 'set-links';
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

// --- LLM stack (llm-stack\stack.json — single source of truth for the gateway + fork backends) ---
export type StackService = {
  id: string;
  name: string;
  group: string; // 'core' | 'router'
  port: number;
  protocol: string; // 'openai' | 'anthropic' | 'openai+anthropic'
  dashboard: string; // '' if none
  dir: string;
  enabled: boolean;
  running: boolean; // live port probe
};
export const readStack = () => invoke<StackService[]>('read_stack');
// `only` = a single service id → act on just that service (per-card start/stop); omit for whole stack.
export const runStack = (action: 'start' | 'stop', only?: string) =>
  invoke<number>('run_stack', { action, only });

// --- stack health (TCP port probe + real HTTP /health when configured in stack.json) ---
export type StackHealth = {
  id: string;
  name: string;
  group: string; // 'core' | 'router'
  port: number;
  enabled: boolean;
  portOpen: boolean; // TCP accepts a connection
  healthy: boolean | null; // HTTP 2xx; null = port-only (no health endpoint)
};
export const readStackHealth = () => invoke<StackHealth[]>('read_stack_health');

// --- stack process info (PID + uptime per listening port, one pwsh snapshot) ---
export type StackProc = { port: number; pid: number; uptimeSec: number };
export const readStackProcs = () => invoke<StackProc[]>('read_stack_procs');

// --- Parallel terminal sessions (real PTY running each profile's `claude`) ---
// session_spawn returns a session id; output arrives on event `pty:data:<id>` (base64),
// termination on `pty:exit:<id>`. Input/resize/kill go back through these commands.
export type SessionTool = 'claude' | 'opencode' | 'shell';
// PTY output streams as raw bytes over a binary Channel (no base64) — onData.onmessage gets ArrayBuffers.
export const sessionSpawn = (
  profile: string,
  tool: SessionTool | undefined,
  args: string | undefined,
  cwd: string | undefined,
  cols: number,
  rows: number,
  onData: Channel<ArrayBuffer>
) => invoke<string>('session_spawn', { profile, tool, args, cwd, cols, rows, onData });
export const sessionWrite = (id: string, data: string) => invoke('session_write', { id, data });
export const sessionResize = (id: string, cols: number, rows: number) =>
  invoke('session_resize', { id, cols, rows });
export const sessionKill = (id: string) => invoke('session_kill', { id });

// --- Native folder picker (Windows Explorer) ---
import { open as openDialog } from '@tauri-apps/plugin-dialog';
export const pickFolder = async (defaultPath?: string): Promise<string | null> => {
  const res = await openDialog({ directory: true, multiple: false, defaultPath: defaultPath || undefined });
  return typeof res === 'string' ? res : null;
};
// Immediate subfolders of a path (for the projects-root quick-pick).
export const listSubdirs = (path: string) => invoke<string[]>('list_subdirs', { path });

// --- freellmapi analytics (read-only over the gateway's SQLite via a node helper) ---
export type AnalyticsTotals = {
  totalRequests: number;
  successRate: number; // percent
  totalInputTokens: number;
  totalOutputTokens: number;
  avgLatencyMs: number;
  estimatedCostSavings: number; // $ vs paid APIs
  firstRequestAt: string | null;
};
export type AnalyticsModel = {
  platform: string;
  modelId: string;
  displayName: string;
  requests: number;
  successRate: number;
  avgLatencyMs: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  estimatedCost: number;
};
export type AnalyticsSeriesPoint = {
  bucket: number; // unix-epoch second, floored to stepSec
  platform: string;
  modelId: string;
  requests: number;
  totalInputTokens: number;
  totalOutputTokens: number;
};
export type FreellmapiAnalytics = {
  available: boolean; // false → gateway DB/node/data missing (empty state)
  totals: AnalyticsTotals;
  perModel: AnalyticsModel[];
  series: AnalyticsSeriesPoint[]; // per-model usage over time (sparkline source)
  stepSec: number; // bucket width in seconds
};
export const readFreellmapiAnalytics = (rangeHours: number) =>
  invoke<FreellmapiAnalytics>('read_freellmapi_analytics', { rangeHours });

// --- opencode agent (single global config: ~/.config/opencode/opencode.json) ---
export type OpencodeProvider = {
  id: string;
  name: string;
  baseUrl: string;
  hasKey: boolean; // apiKey value is never exposed, only whether one is set
};
export type OpencodeStatus = {
  installed: boolean; // config file exists
  model: string; // active model "<id>/<model>"
  providers: OpencodeProvider[];
};
export type OpencodeProviderArgs = {
  action: 'set' | 'clear';
  providerId: string;
  name?: string;
  baseUrl?: string;
  model?: string;
  key?: string; // literal apiKey
  envKey?: string; // → "{env:envKey}" placeholder
  keepKey?: boolean; // keep existing apiKey
};
export const readOpencode = () => invoke<OpencodeStatus>('read_opencode');
export const runOpencodeProvider = (args: OpencodeProviderArgs) =>
  invoke<number>('run_opencode_provider', {
    action: args.action,
    providerId: args.providerId,
    name: args.name ?? null,
    baseUrl: args.baseUrl ?? null,
    model: args.model ?? null,
    key: args.key ?? null,
    envKey: args.envKey ?? null,
    keepKey: args.keepKey ?? null
  });
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

// --- Custom provider registry (own list of external LLM providers; keys in Credential Manager) ---
export type MyProvider = {
  id: string;
  name: string;
  baseUrl: string;
  protocol: 'anthropic' | 'openai';
  authScheme: string;
  model: string;
  smallModel: string;
  connectVia: 'freellmapi' | 'direct';
  targetProfile: string;
  createdAt: string;
  hasKey: boolean;
  keyCount: number; // keys in the rotation pool (0 = legacy single key)
  activeKey: number; // index of the active key within the pool
};
export type MyProviderInput = {
  id?: string; // omitted/empty → create
  name: string;
  baseUrl: string;
  protocol: 'anthropic' | 'openai';
  authScheme?: string;
  model?: string;
  smallModel?: string;
  connectVia: 'freellmapi' | 'direct';
  targetProfile?: string;
};
export const listMyProviders = () => invoke<MyProvider[]>('list_my_providers');
export const saveMyProvider = (p: MyProviderInput, apiKey?: string) =>
  invoke<MyProvider>('save_my_provider', { p, apiKey });
export const deleteMyProvider = (id: string) => invoke('delete_my_provider', { id });
export const connectMyProvider = (id: string) => invoke<number>('connect_my_provider', { id });
export const setFreellmapiAuth = (email?: string, password?: string, token?: string) =>
  invoke('set_freellmapi_auth', { email, password, token });
export const freellmapiAuthStatus = () =>
  invoke<{ hasEmail: boolean; hasToken: boolean }>('freellmapi_auth_status');
export const checkMyProvider = (id: string) =>
  invoke<{ ok: boolean; detail: string; count?: number }>('check_my_provider', { id });
// Liveness check for an arbitrary base URL (local engines / stack services — no key).
export const checkProviderUrl = (baseUrl: string, protocol: string) =>
  invoke<{ ok: boolean; detail: string; count?: number }>('check_provider_url', { baseUrl, protocol });
// Read-only view of a profile's CLAUDE.md / settings.json (#80).
export const readProfileFile = (name: string, which: 'claude' | 'settings') =>
  invoke<string>('read_profile_file', { name, which });
// Multi-key rotation pool (e.g. several aerolink keys rotated on balance exhaustion).
export const addProviderKey = (id: string, apiKey: string) =>
  invoke<MyProvider>('add_provider_key', { id, apiKey });
export const removeProviderKey = (id: string, index: number) =>
  invoke<MyProvider>('remove_provider_key', { id, index });
export const nextProviderKey = (id: string) => invoke<number>('next_provider_key', { id });

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
export const runMcp = (action: 'deploy', only?: string[]) =>
  invoke<number>('run_mcp', { action, only: only && only.length ? only : null });

// --- Schedule tab ---
export type ScheduleTask = {
  id: string;
  label: string;
  tn: string;
  exists: boolean;
  enabled: boolean;
  time: string | null;
  nextRun: string | null;
  lastRun: string | null;
  lastResult: number | null;
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
  closeToTray?: boolean;
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
