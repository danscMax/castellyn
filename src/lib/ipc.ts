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
// F21: global panic button — kills the active run, all fork runs, all PTY sessions, stops bulk plugin.
export const cancelAll = () => invoke('cancel_all');

// --- Forks tab ---
export type ForkAction = 'check' | 'plan' | 'ff' | 'delete' | 'rebase' | 'sync-wip' | 'delete-wip' | 'prune' | 'normalize';

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
  // PowerShell's `Select-Object -Unique` yields a scalar string for a single file (array for 2+).
  conflictFiles: string | string[] | null;
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
  forkOwnerRepo: string | null; // "owner/repo" of your fork (for GitHub compare/PR links)
  parentOwnerRepo: string | null; // "owner/repo" of the upstream/original
  defaultBranch: string | null;
  behindBy: number | null;
  defaultAhead: number | null; // commits in YOUR default branch not upstream (ff-blocker)
  ffSafe: boolean;
  dirty: boolean;
  untracked: boolean;
  midOp: boolean;
  opName: string | null;
  detached: boolean;
  currentBranch: string | null;
  isOwn: boolean;
  rolesGuessed: boolean;
  wipLocal: { behindBy: number | null; mergedPatches: number | null; uniquePatches: number | null } | null;
  upstreamUpdated?: string | null; // ISO date of the upstream tip (how fresh the original is)
  upstreamArchived?: boolean | null; // original is archived on GitHub → fork is dead
  upstreamDefaultBranch?: string | null; // original's current default branch (drift if ≠ defaultBranch)
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
  isArchived: boolean;
  url: string;
  updatedAt: string;
  description: string;
  language: string;
  stars: number;
};

export const listGithubRepos = () => invoke<GithubRepo[]>('list_github_repos');

// --- Backup tab ---
export type BackupAction = 'backup' | 'restore-preview' | 'restore' | 'delete-snapshot';

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
// F9: reveal a weekly archive (weekly-*.zip) selected in Explorer.
export const revealBackup = (name: string) => invoke('reveal_backup', { name });
// F9: delete a weekly archive (confirmed in the UI).
export const deleteBackup = (name: string) => invoke('delete_backup', { name });
// F9: verify a weekly archive's integrity (tar -tf); resolves to the entry count, rejects on a bad zip.
export const verifyBackup = (name: string) => invoke<number>('verify_backup', { name });
// F9: extract a weekly archive to a folder (non-destructive — never over live ~/.claude).
export const extractBackup = (name: string, dest: string) => invoke('extract_backup', { name, dest });

export const runBackup = (action: BackupAction, opts: RestoreOpts = {}) =>
  invoke<number>('run_backup', {
    action,
    timestamp: opts.timestamp ?? null,
    profiles: opts.profiles ?? null,
    includeCredentials: opts.includeCredentials ?? null,
    keepSnapshots: opts.keepSnapshots ?? null
  });

// --- Profiles tab ---
export type ProfileAction = 'check' | 'clean-conflicts' | 'reinstall' | 'repair' | 'create';

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
  isAdmin?: boolean;
  profiles?: ProfileInfo[];
  syncConflicts?: { count: number; files: string[] };
  // REL-4: backup freshness canary written by Get-ProfilesStatus.ps1.
  backup?: { lastRun?: string; lastSnapshot?: string; ageHours?: number; stale?: boolean };
};

export const readProfiles = () => invoke<ProfilesStatus | null>('read_profiles');
export const runProfiles = (action: ProfileAction, name?: string) =>
  invoke<number>('run_profiles', { action, name });
// F23: repair the links of several profiles in one run (Home "Repair All").
export const repairAllProfiles = (names: string[]) =>
  invoke<number>('repair_all_profiles', { names });
// Finish a half-built profile's folder symlinks with admin rights (one-off UAC).
export const repairProfileElevated = (name: string) =>
  invoke<number>('repair_profile_elevated', { name });
// Relaunch the whole app elevated (so inline symlink creation works).
export const relaunchAsAdmin = () => invoke<void>('relaunch_as_admin');
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
export const runStack = (action: 'start' | 'stop' | 'restart', only?: string) =>
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

// --- Parallel terminal sessions (real PTY running claude/opencode/shell/ssh) ---
// session_spawn returns a session id; output streams over the binary Channel passed as onData,
// termination on event `pty:exit:<id>`. Input/resize/kill go back through these commands.
export type SessionTool = 'claude' | 'opencode' | 'codex' | 'shell' | 'ssh';
// PTY output streams as raw bytes over a binary Channel (no base64) — onData.onmessage gets ArrayBuffers.
export const sessionSpawn = (
  profile: string,
  tool: SessionTool | undefined,
  args: string | undefined,
  cwd: string | undefined,
  cols: number,
  rows: number,
  onData: Channel<ArrayBuffer>,
  remoteDir?: string,
  sshTarget?: string
) => invoke<string>('session_spawn', { profile, tool, args, cwd, remoteDir, sshTarget, cols, rows, onData });
export const sessionWrite = (id: string, data: string) => invoke('session_write', { id, data });
export const sessionResize = (id: string, cols: number, rows: number) =>
  invoke('session_resize', { id, cols, rows });
export const sessionKill = (id: string) => invoke('session_kill', { id });

// --- Native folder picker (Windows Explorer) ---
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
export const pickFolder = async (defaultPath?: string): Promise<string | null> => {
  const res = await openDialog({ directory: true, multiple: false, defaultPath: defaultPath || undefined });
  return typeof res === 'string' ? res : null;
};
// Immediate subfolders of a path (for the projects-root quick-pick).
export const listSubdirs = (path: string) => invoke<string[]>('list_subdirs', { path });

// --- SSH host registry (saved JSON under SCRIPTS_ROOT + read-only import from ~/.ssh/config) ---
// No secrets stored — an ssh session is just session_spawn with tool 'ssh' and the target in `args`.
export type SshHost = {
  id: string;
  name: string;
  host: string;
  port?: number | null;
  user?: string | null;
  keyPath?: string | null;
  remoteDir?: string | null; // optional remote start dir (cd into it on connect; Windows/PowerShell)
  source: 'saved' | 'sshconfig';
};
// Build the `ssh` CLI target string from a saved/imported host (user@host -p port -i key).
// Hardening (argv flag-smuggling): host/user are interpolated into `ssh --% -t <target>` and
// RE-TOKENISED by ssh.exe on whitespace, so each must be a single safe token — not just free of a
// leading '-'. A host like `realhost -oProxyCommand=calc` doesn't start with '-' yet smuggles an
// option after the space. So we require a strict charset AND no whitespace, rejecting (throwing on)
// anything else. Values come from saved/SyncThing-synced config, hence treated as untrusted. Port is
// bounds-checked; the key path is quoted (one argv token) with quotes/newlines stripped.
const SSH_HOST_RE = /^[A-Za-z0-9._:[\]-]+$/; // hostname / IPv4 / [IPv6]
const SSH_USER_RE = /^[A-Za-z0-9._-]+$/;
export function sshTarget(h: SshHost): string {
  const host = (h.host ?? '').trim();
  const user = (h.user ?? '').trim();
  if (!host || host.startsWith('-') || !SSH_HOST_RE.test(host)) throw new Error(`unsafe ssh host: ${host}`);
  if (user && (user.startsWith('-') || !SSH_USER_RE.test(user))) throw new Error(`unsafe ssh user: ${user}`);
  let s = user ? `${user}@${host}` : host;
  if (h.port != null && Number.isInteger(h.port) && h.port > 0 && h.port < 65536) s += ` -p ${h.port}`;
  if (h.keyPath) s += ` -i "${h.keyPath.replace(/["\r\n]/g, '')}"`;
  return s;
}
// Parse a typed ssh target ("user@host -p 22 -i ~/.ssh/key") back into structured host fields.
export function parseSshTarget(s: string): { host: string; user: string | null; port: number | null; keyPath: string | null } {
  const toks = s.trim().split(/\s+/).filter(Boolean);
  let host = toks[0] ?? '';
  let user: string | null = null;
  if (host.includes('@')) {
    const at = host.indexOf('@');
    user = host.slice(0, at);
    host = host.slice(at + 1);
  }
  let port: number | null = null;
  let keyPath: string | null = null;
  for (let i = 1; i < toks.length; i++) {
    if (toks[i] === '-p' && toks[i + 1]) {
      const p = parseInt(toks[++i], 10);
      port = Number.isFinite(p) ? p : null;
    } else if (toks[i] === '-i' && toks[i + 1]) {
      keyPath = toks[++i];
    }
  }
  return { host, user, port, keyPath };
}
export const readSshHosts = () => invoke<SshHost[]>('read_ssh_hosts');
export const saveSshHost = (host: SshHost) => invoke<SshHost[]>('save_ssh_host', { host });
export const deleteSshHost = (id: string) => invoke<SshHost[]>('delete_ssh_host', { id });
// Quick TCP reachability probe (host:port, default 22) — does not authenticate.
export const testSshHost = (host: string, port?: number | null) =>
  invoke<boolean>('test_ssh_host', { host, port: port ?? null });

// --- Multi-monitor: pop a live pane onto another monitor (renders the SAME session via attach) ---
export type MonitorInfo = {
  index: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  primary: boolean;
};
export const listMonitors = () => invoke<MonitorInfo[]>('list_monitors');
export const openMonitorWindow = (label: string, monitorIndex: number) =>
  invoke('open_monitor_window', { label, monitorIndex });
// Live session(s) handed off to a detached window via the backend registry. A window can host one
// popped-out pane or a whole monitor's worth of panes (each mirrors its session via attach).
export type DetachPane = {
  sessionId?: string; // present → attach a live session (live move); absent → spawn fresh (restore)
  title: string;
  tool: SessionTool;
  profile?: string;
  cwd?: string;
  args?: string;
  owns?: boolean; // the destination pane owns the session (kills it on close) — true for a live move
};
export type DetachSpec = { panes: DetachPane[] };
export const prepareDetach = (label: string, spec: DetachSpec) => invoke('prepare_detach', { label, spec });
export const takeDetach = (label: string) => invoke<DetachSpec | null>('take_detach', { label });
// Attach an extra output channel to a live session (used by a detached window; no respawn).
// Returns a channel token; pass it to sessionDetach to drop just this window's channel later.
export const sessionAttach = (id: string, onData: Channel<ArrayBuffer>) =>
  invoke<number>('session_attach', { id, onData });
// Drop one window's channel (by token) without killing the session (window closed / pane moved back).
export const sessionDetach = (id: string, token: number) => invoke('session_detach', { id, token });
// Live session ids across all windows — used to re-attach panes after a webview reload.
export const sessionList = () => invoke<string[]>('session_list');
// Open a source location (clicked path:line link in a terminal) in the user's editor.
export const openInEditor = (path: string, line?: number) => invoke('open_in_editor', { path, line });

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
  balanceUrl: string;
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
  balanceUrl?: string;
};
export type ProviderBalance = { ok: boolean; amount?: number; currency?: string; detail: string };
export const listMyProviders = () => invoke<MyProvider[]>('list_my_providers');
// Best-effort balance/credits for a custom provider (#B4); ok=false when unavailable.
export const checkProviderBalance = (id: string) =>
  invoke<ProviderBalance>('check_provider_balance', { id });
export const saveMyProvider = (p: MyProviderInput, apiKey?: string) =>
  invoke<MyProvider>('save_my_provider', { p, apiKey });
export const deleteMyProvider = (id: string) => invoke('delete_my_provider', { id });
export const connectMyProvider = (id: string) => invoke<number>('connect_my_provider', { id });
export const setFreellmapiAuth = (email?: string, password?: string, token?: string) =>
  invoke('set_freellmapi_auth', { email, password, token });
// C2: delete one of {email, password, token} from Credential Manager so a stale or wrong
// credential can be cleanly retired; set_freellmapi_auth only WRITES.
export const deleteFreellmapiAuth = (key: 'email' | 'password' | 'token') =>
  invoke('delete_freellmapi_auth', { key });
export const freellmapiAuthStatus = () =>
  invoke<{ hasEmail: boolean; hasPassword: boolean; hasToken: boolean }>('freellmapi_auth_status');
export const checkMyProvider = (id: string) =>
  invoke<{ ok: boolean; detail: string; count?: number }>('check_my_provider', { id });
// Liveness check for an arbitrary base URL (local engines / stack services — no key).
export const checkProviderUrl = (baseUrl: string, protocol: string) =>
  invoke<{ ok: boolean; detail: string; count?: number }>('check_provider_url', { baseUrl, protocol });
// Read-only view of a profile's CLAUDE.md / settings.json (#80).
export type ProfileUsage = {
  fiveHourPct: number | null;
  sevenDayPct: number | null;
  fiveHourResetsAt: string | null;
  sevenDayResetsAt: string | null;
};
// Claude Code usage limits (5h + 7d remaining) for a profile; null = not logged in / unavailable.
export const readProfileUsage = (profile: string) =>
  invoke<ProfileUsage | null>('read_profile_usage', { profile });
// Durable Sessions-personalization sidecar (item 18): ~/.claude/castellyn/sessions.json.
// Returns null when the file doesn't exist yet (fresh machine → migrate from localStorage).
export const readSessionsPrefs = () => invoke<string | null>('read_sessions_prefs');
export const writeSessionsPrefs = (json: string) => invoke<void>('write_sessions_prefs', { json });
export const readProfileFile =(name: string, which: 'claude' | 'settings') =>
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
export type SyncItem = 'history' | 'projects' | 'skills' | 'agents' | 'commands' | 'keybindings' | 'castellyn';
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

// --- Config drift (shared-config file links; FUN-7) ---
export type ConfigDriftItem = { name: string; state: string };
export type ConfigDriftStatus = {
  generatedAt?: string;
  drifted?: number; // content out of sync (needs Sync now)
  unlinked?: number; // real files instead of symlinks (needs Repair)
  ok?: boolean;
  items?: ConfigDriftItem[];
};
export type ConfigDriftAction = 'check' | 'relink' | 'sync-now';

export const readConfigDrift = () => invoke<ConfigDriftStatus | null>('read_config_drift');
export const runConfigDrift = (action: ConfigDriftAction) =>
  invoke<number>('run_config_drift', { action });

// --- Config-drift diff (Phase 3.2) ---
export type DiffLineKind = 'add' | 'del' | 'same';
export type DiffLine = { kind: DiffLineKind; text: string };
export type DriftDiff = {
  tipPath: string;
  sourcePath: string;
  sourceLines: number;
  tipLines: number;
  lines: DiffLine[];
};

export const readDriftDiff = (name: string) => invoke<DriftDiff | null>('read_drift_diff', { name });

// --- MCP tab ---
export type McpServer = {
  name: string;
  command: string;
  definition: unknown; // the server's full canonical JSON object (for the edit form)
  deployedIn: string[];
};
export type McpExtra = { name: string; presentIn: string[] };
export type McpStatus = { source: McpServer[]; extras: McpExtra[]; profiles: string[] };

export const readMcp = () => invoke<McpStatus>('read_mcp');
export const runMcp = (action: 'deploy', only?: string[]) =>
  invoke<number>('run_mcp', { action, only: only && only.length ? only : null });
// Canonical config\.mcp.json CRUD (definition is the server's JSON object, serialized).
export const mcpUpsertServer = (name: string, definition: string) =>
  invoke<void>('mcp_upsert_server', { name, definition });
export const mcpRemoveServer = (name: string) => invoke<void>('mcp_remove_server', { name });
export const mcpRemoveExtra = (name: string, profile: string) =>
  invoke<void>('mcp_remove_extra', { name, profile });

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
  status?: string; // REL-6: decoded HRESULT (ok/running/never-run/failed(0x..))
  ok?: boolean | null; // REL-6: true when lastResult is 0 or "running"
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
  description?: string;
  mine?: boolean; // from your own local (directory) marketplace
};
// source: 'own' (symlinked) | 'default' | 'plugin:<id>'; mine = authored by you (symlink or local marketplace)
export type SkillInfo = { name: string; description: string; version: string; dir: string; source: string; mine: boolean };
export type PluginAction = 'enable' | 'disable' | 'update' | 'remove';

export type PluginUpdate = { id: string; installed: string; available: string };

export type PluginContents = {
  id: string;
  skills: string[];
  commands: string[];
  agents: string[];
};

export const listPlugins = () => invoke<PluginInfo[]>('list_plugins');
export const listSkills = () => invoke<SkillInfo[]>('list_skills');

// --- Environments tab (cross-harness coverage: skills / providers / RTK) ---
// Read-only overview of every coding harness (Claude Code, OpenCode, Codex, zcode) — how many
// skills each can see, whether plugin-bundled skills reach it, provider count, RTK wiring.
export type EnvInfo = {
  id: string; // 'claude' | 'opencode' | 'codex' | 'zcode'
  name: string;
  installed: boolean;
  configPath: string;
  skillsVisible: number;
  totalSkills: number;
  pluginSkillsVisible: boolean; // all plugin-bundled skills reachable from this harness
  shareableGap: number; // skills "share skills" would newly make visible here (0 = nothing left)
  providers: number;
  mcpServers: number;
  rtk: boolean;
  rtkAvailable: boolean; // false → no RTK integration path for this harness yet
  configOk: boolean; // false → config file exists but failed to parse
};
export const readEnvironments = () => invoke<EnvInfo[]>('read_environments');
// Per-skill visibility matrix across harnesses (diff view). `shareable` = present in a source but
// missing from OpenCode or Codex.
export type SkillRow = { name: string; claude: boolean; opencode: boolean; codex: boolean; shareable: boolean };
export const readSkillMatrix = () => invoke<SkillRow[]>('read_skill_matrix');
// Share all skills (regular + plugin-bundled) into ~/.agents/skills so OpenCode and Codex see them.
// Additive & idempotent (directory junctions, no admin); never deletes. Claude is untouched.
export type ShareResult = { created: number; skipped: number; failed: number; target: string; details: string[] };
export const shareSkills = () => invoke<ShareResult>('share_skills');
// Enable/disable RTK command-rewriting for OpenCode (writes a Windows-safe plugin). Returns new state.
export const runOpencodeRtk = (action: 'enable' | 'disable') =>
  invoke<boolean>('run_opencode_rtk', { action });
// Fan out canonical .mcp.json servers into OpenCode's opencode.json `mcp`. Returns count written.
export const runOpencodeMcp = () => invoke<number>('run_opencode_mcp');
// Fan out the myproviders.json registry into OpenCode's `provider` block (keys stay env-refs only).
export const runOpencodeProviders = () => invoke<number>('run_opencode_providers');
// Attach canonical rule files (config CLAUDE.md/RTK.md) to OpenCode's `instructions` array.
export const runOpencodeInstructions = () => invoke<number>('run_opencode_instructions');
// Fan out canonical .mcp.json servers into Codex via the official `codex mcp add` CLI.
export const runCodexMcp = () => invoke<number>('run_codex_mcp');
// Connect the freellmapi gateway to Codex ([model_providers] + [profiles]); also mirrors the
// gateway key into the user env via setx. Resolves to whether the key was set.
export const runCodexProviders = () => invoke<boolean>('run_codex_providers');
// Delete a skill directory (guarded server-side to ~/.claude/skills).
export const deleteSkill = (dir: string) => invoke('delete_skill', { dir });
export type PluginRelease = {
  tag_name: string;
  name: string;
  body: string;
  published_at: string;
};

export const listPluginUpdates = () => invoke<PluginUpdate[]>('list_plugin_updates');
export const listPluginContents = () => invoke<PluginContents[]>('list_plugin_contents');
export const listPluginReleases = (id: string) => invoke<PluginRelease[]>('list_plugin_releases', { id });
export const runPlugin = (action: PluginAction, id: string) =>
  invoke<number>('run_plugin', { action, id });
// F17: bulk plugin op in its own backend domain (sequential inside, off the global run lock).
export const runPluginsBulk = (action: PluginAction, ids: string[]) =>
  invoke<number>('run_plugins_bulk', { action, ids });

// Plugin sync across profiles: SessionStart hook wiring status + on-demand reconcile.
// wired/unwired hold profile DIR names (".claude", ".claude-cc1", …).
export type PluginSyncStatus = {
  wired: string[];
  unwired: string[];
  scriptInstalled: boolean;
  scriptVersion: number;
};
export const pluginSyncStatus = () => invoke<PluginSyncStatus>('plugin_sync_status');
export const pluginSyncSet = (enabled: boolean) =>
  invoke<PluginSyncStatus>('plugin_sync_set', { enabled });
// Streams into the console as component "pluginsync"; resolves with the exit code on run-done.
export const runPluginSync = () => invoke<number>('run_plugin_sync');

// Agent-status lifecycle hook (Sessions): castellyn_status.py wired into five Claude Code
// events of every profile. `agent-status` events then drive the pane badges.
export type AgentStatusHookState = { wired: string[]; unwired: string[] };
export const agentStatusHookStatus = () => invoke<AgentStatusHookState>('agent_status_hook_status');
export const agentStatusHookSet = (enabled: boolean) =>
  invoke<AgentStatusHookState>('agent_status_hook_set', { enabled });
/** Payload of the backend `agent-status` event (state: working | blocked | idle | unknown).
 * `spawnedAt` is the session's spawn time (unix ms), static — the UI derives elapsed on render. */
export type AgentStatusEvent = { id: string; state: string; claudeSessionId: string | null; spawnedAt?: number };

/** Backend `limits-status` event: per-profile Anthropic usage (5h/7d utilization %). `expired`
 * means the OAuth token was rejected (401). Emitted every poll for each OAuth profile. */
export type LimitsStatusEvent = {
  profile: string;
  h5: number | null;
  d7: number | null;
  h5Reset: string | null;
  d7Reset: string | null;
  expired: boolean;
};
/** Backend `limits-alert` event: a window newly crossed a threshold (85 or 99). The UI toasts it;
 * the backend also rings + OS-notifies at 99. `window` is "5h" | "7d". */
export type LimitsAlertEvent = {
  profile: string;
  window: string;
  level: number;
  utilization: number;
  resetsAt: string | null;
};

// --- Settings ---
export type HubConfig = {
  scriptsRoot?: string | null;
  startHidden?: boolean;
  closeToTray?: boolean;
  fetchTimeoutSec?: number | null;
  ghTimeoutSec?: number | null;
  toggleHotkey?: string | null;
  shortcuts?: Record<string, string> | null;
  language?: string | null;
  // Agent-status notifications (Sessions). Absent = default (on).
  statusSounds?: boolean | null;
  statusNotify?: boolean | null;
  // #21c: auto-continue a limited Claude pane after its 5h reset. Absent = default (on). No UI
  // toggle — a config-only escape hatch; read-only from the app (read-patch-write preserves it).
  autoContinueOnReset?: boolean | null;
  // #21e: after-limit behaviour — 'wait' (default, auto-continue on reset) | 'switchProfile'
  // (respawn under a free OAuth profile immediately).
  limitMode?: string | null;
};

/** A single entry in the global-shortcut mapping. */
export type ShortcutEntry = { action: string; accelerator: string };
export type AppPaths = {
  scriptsRoot: string;
  configPath: string | null;
  exe: string | null;
  stackPath?: string | null;
  backupDir?: string | null;
};

export const readConfig = () => invoke<HubConfig>('read_config');
export const writeConfig = (config: HubConfig) => invoke('write_config', { config });
// Mirror the UI locale into the backend (errors/run-log/tray localize + persist to config).
export const setLanguage = (lang: string) => invoke('set_language', { lang });
export const appPaths = () => invoke<AppPaths>('app_paths');
// F13: freellmapi gateway URL from stack.json — replace hardcoded localhost:13001.
export const gatewayBaseUrl = () => invoke<string | null>('gateway_base_url');
// F24: canonical `~/.claude/skills` path, resolved through symlinks.
export const canonicalSkillsDir = () => invoke<string>('canonical_skills_dir');
// F16/F19: live PTY session count across all windows (the global SESSION_LIMIT pool).
export const globalSessionCount = () => invoke<number>('global_session_count');
// F19: hard-exit the app from the frontend (after the tray-Quit confirm).
export const quitApp = () => invoke('quit_app');
export const openPath = (path: string) => invoke('open_path', { path });
// F10: clone a GitHub repo to a local path via the git CLI (target = full destination dir).
export const cloneRepo = (url: string, target: string) => invoke('clone_repo', { url, target });
// Open a web URL in the default browser (opener plugin) — NOT open_path, which is filesystem-only.
export const openUrl = (url: string) => invoke('open_url', { url });
// Export/import all settings (#117) — file dialogs from the dialog plugin; backend (de)serializes.
export const pickSaveFile = (name: string) =>
  saveDialog({ defaultPath: name, filters: [{ name: 'JSON', extensions: ['json'] }] });
export const pickOpenFile = async (): Promise<string | null> => {
  const r = await openDialog({ directory: false, multiple: false, filters: [{ name: 'JSON', extensions: ['json'] }] });
  return typeof r === 'string' ? r : null;
};
export const exportConfig = (dest: string) => invoke('export_config', { dest });
export const importConfig = (src: string) => invoke<HubConfig>('import_config', { src });
// Register/clear the OS-global show/hide hotkey at runtime (#123). Throws on a bad/taken combo.
export const setToggleHotkey = (accel: string | null) => invoke('set_toggle_hotkey', { accel });
export const readShortcuts = () => invoke<Record<string, string>>('read_shortcuts');
export const setShortcuts = (shortcuts: Record<string, string>) => invoke('set_shortcuts', { shortcuts });
export const getAutostart = () => invoke<boolean>('get_autostart');
export const setAutostart = (enabled: boolean) => invoke('set_autostart', { enabled });
