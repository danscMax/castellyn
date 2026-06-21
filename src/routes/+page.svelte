<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    listComponents,
    readStatus,
    runComponent,
    runForks,
    runForkRepo,
    cancelForkRepo,
    readForkRepoStatus,
    listBackups,
    runBackup,
    readProfiles,
    runProfiles,
    readProfilesConfig,
    runProfileMgmt,
    repairProfileElevated,
    relaunchAsAdmin,
    openProfileDir,
    launchProfile,
    readLaunchConfig,
    setLaunchConfig,
    measureContext,
    readMcp,
    runMcp,
    readSync,
    runSync,
    readConfigDrift,
    runConfigDrift,
    readEngines,
    runEngine,
    runRouter,
    runConnectRouter,
    readEngineModels,
    readProviders,
    runProvider,
    listMyProviders,
    saveMyProvider,
    deleteMyProvider,
    connectMyProvider,
    addProviderKey,
    removeProviderKey,
    nextProviderKey,
    setFreellmapiAuth,
    listGithubRepos,
    readStack,
    runStack,
    readOpencode,
    runOpencodeProvider,
    openPath,
    listPlugins,
    listSkills,
    deleteSkill,
    listPluginUpdates,
    listPluginContents,
    runPlugin,
    readSchedules,
    runSchedule,
    cancelRun,
    readConfig,
    type Component,
    type ForkAction,
    type GithubRepo,
    type StackService,
    type OpencodeStatus,
    type BackupAction,
    type BackupList,
    type RestoreOpts,
    type ProfileAction,
    type ConfigDriftStatus,
    type ConfigDriftAction,
    type ProfilesStatus,
    type ProfilesConfig,
    type ProfileMgmtArgs,
    type LaunchConfigStatus,
    type McpStatus,
    type SyncStatus,
    type EngineStatus,
    type ProfileProvider,
    type ProviderArgs,
    type MyProvider,
    type MyProviderInput,
    type SchedulesStatus,
    type ScheduleAction,
    type PluginInfo,
    type SkillInfo,
    type PluginAction,
    type PluginUpdate,
    type PluginContents
  } from '$lib/ipc';
  import {
    updatesAttention,
    forksAttention,
    backupAttention,
    profilesAttention,
    pluginsAttention,
    syncAttention
  } from '$lib/attention';
  import { getTheme, applyTheme, type Theme } from '$lib/theme';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  import Console from '$lib/components/Console.svelte';
  import UpdatesTab from '$lib/components/UpdatesTab.svelte';
  import ForksTab from '$lib/components/ForksTab.svelte';
  import BackupTab from '$lib/components/BackupTab.svelte';
  import HotkeyHelp from '$lib/components/HotkeyHelp.svelte';
  import ProfilesTab from '$lib/components/ProfilesTab.svelte';
  import McpTab from '$lib/components/McpTab.svelte';
  import SyncTab from '$lib/components/SyncTab.svelte';
  import HomeTab from '$lib/components/HomeTab.svelte';
  import ProvidersTab from '$lib/components/ProvidersTab.svelte';
  import SessionsTab from '$lib/components/SessionsTab.svelte';
  import CommandPalette from '$lib/components/CommandPalette.svelte';
  import AnalyticsTab from '$lib/components/AnalyticsTab.svelte';
  import PluginsTab from '$lib/components/PluginsTab.svelte';
  import ScheduleTab from '$lib/components/ScheduleTab.svelte';
  import SettingsTab from '$lib/components/SettingsTab.svelte';
  import OnboardingWizard from '$lib/components/OnboardingWizard.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import ToastHost from '$lib/components/ToastHost.svelte';
  import { pushToast } from '$lib/toast.svelte';
  import { runningStore, opName } from '$lib/running.svelte';
  import { deriveOutcome } from '$lib/outcome';
  import { t, locale } from '$lib/i18n';
  import { setLanguage } from '$lib/ipc';

  let components = $state<Component[]>([]);
  let statuses = $state<Record<string, any>>({});
  let running = $state<string | null>(null);
  let log = $state<string[]>([]);
  /** Cap the console buffer so a chatty/stuck script can't grow it without bound. */
  const MAX_LOG = 5000;
  let active = $state('updates');
  let theme = $state<Theme>('dark');
  let backupData = $state<BackupList | null>(null);
  let profilesData = $state<ProfilesStatus | null>(null);
  let profilesConfig = $state<ProfilesConfig | null>(null);
  let launchConfig = $state<LaunchConfigStatus | null>(null);
  let mcpData = $state<McpStatus | null>(null);
  let syncData = $state<SyncStatus | null>(null);
  let driftData = $state<ConfigDriftStatus | null>(null);
  let syncLoaded = $state(false);
  let enginesData = $state<EngineStatus[] | null>(null);
  let providersData = $state<ProfileProvider[] | null>(null);
  let myProvidersData = $state<MyProvider[] | null>(null);
  let stackData = $state<StackService[] | null>(null);
  let opencodeData = $state<OpencodeStatus | null>(null);
  let providersLoaded = $state(false);
  let schedulesData = $state<SchedulesStatus | null>(null);
  let schedulesLoaded = $state(false);
  let pluginsData = $state<PluginInfo[] | null>(null);
  let skillsData = $state<SkillInfo[] | null>(null);
  let pluginUpdates = $state<PluginUpdate[]>([]);
  let pluginContents = $state<PluginContents[]>([]);
  let extensionsLoaded = $state(false);
  let loadError = $state<string | null>(null);
  // OU-04 first-run onboarding wizard. Shown once when a fresh user has neither a configured
  // Scripts root NOR any profiles. Dismissal persists in localStorage (the app's existing
  // UI-state store) — not HubConfig, since the typed write_config round-trip would drop an
  // unknown field, and we must not touch src-tauri.
  const ONBOARDED_KEY = 'cmh-onboarded';
  let onboardingOpen = $state(false);
  let onboardingChecked = false;
  // Per-tab "fetching fresh data" flags → drive the refresh overlay + sidebar spinner.
  let loading = $state<Record<string, boolean>>({});
  const setLoading = (id: string, v: boolean) => {
    loading[id] = v;
  };
  // Forks status is cached on disk; refresh it the first time its tab opens this session.
  let forksChecked = $state(false);
  // All of the user's GitHub repos (gh repo list) — to surface repos not cloned locally.
  let githubRepos = $state<GithubRepo[]>([]);
  let githubLoaded = $state(false);
  let confirm = $state<{
    open: boolean;
    title: string;
    message: string;
    confirmLabel: string;
    action: (() => void) | null;
    details: string[];
    requireText: string | null;
    danger: boolean;
  }>({
    open: false,
    title: '',
    message: '',
    confirmLabel: t('common.confirm'),
    action: null,
    details: [],
    requireText: null,
    danger: false
  });

  let unlisten: UnlistenFn[] = [];

  async function loadStatus(c: Component) {
    if (!c.lastJson) return;
    try {
      statuses[c.id] = await readStatus(c.lastJson);
    } catch {
      statuses[c.id] = null;
    }
  }

  // Mirror the run lock into a tiny store so the title bar (a sibling in +layout) can show
  // "what's running now" without prop-drilling. opName lives alongside the store now.
  $effect(() => {
    runningStore.op = running;
  });

  // Surface an invoke/IPC failure both in the log and as a glanceable error toast (#150).
  function toastErr(e: unknown, title?: string) {
    const msg = String((e as { message?: string })?.message ?? e);
    log = [...log, t('page.log_error', { e: msg })].slice(-MAX_LOG);
    pushToast({ kind: 'error', title: title ?? t('page.toast_generic_error'), detail: msg });
  }

  // Shared spawn-rejection handler for the start* runners: a backend spawn that rejects (busy
  // slot / script-not-found) only logs operationally, so clear the run lock and append the error
  // line in one place. String(e) so the typed t() slot receives a string, not a raw Error.
  function onSpawnErr(e: unknown) {
    running = null;
    log = [...log, t('page.log_error', { e: String(e) })].slice(-MAX_LOG);
  }

  // --- First-run onboarding (OU-04) ---
  // Decide whether to show the wizard from REAL state: never onboarded before AND the setup is
  // empty (no Scripts root configured AND no profiles). A configured user is never nagged.
  async function maybeShowOnboarding() {
    if (onboardingChecked) return;
    onboardingChecked = true;
    try {
      if (localStorage.getItem(ONBOARDED_KEY) === '1') return;
    } catch {
      /* ignore */
    }
    let hasScriptsRoot = false;
    try {
      const cfg = await readConfig();
      hasScriptsRoot = !!cfg.scriptsRoot && cfg.scriptsRoot.trim().length > 0;
    } catch {
      /* treat an unreadable config as "not configured" */
    }
    const hasProfiles = (profilesData?.profiles?.length ?? 0) > 0;
    if (!hasScriptsRoot && !hasProfiles) onboardingOpen = true;
  }

  // Finish/skip: mark onboarded (persist the flag), close, refresh the data the setup touched,
  // and optionally kick off a first "check all".
  function finishOnboarding(runCheck: boolean) {
    onboardingOpen = false;
    try {
      localStorage.setItem(ONBOARDED_KEY, '1');
    } catch {
      /* ignore */
    }
    // The wizard may have set a Scripts root / created a profile — reload so the tabs reflect it.
    reloadProfiles();
    components.forEach(loadStatus);
    if (runCheck && !running) {
      active = 'updates';
      startRun('all', 'check');
    }
  }

  // Bumped to force-expand the console (toast "Open log" action).
  let consoleReveal = $state(0);
  // Last fork action, so run-done can auto-recheck after a mutating one.
  let lastForkAction: ForkAction | null = null;

  // Per-repo concurrent fork runs (keyed by repo path) — each card shows its own progress/result
  // without the global run lock blocking the whole tab.
  type ForkRunState = { line: string; running: boolean; code: number | null };
  let forkRuns = $state<Record<string, ForkRunState>>({});

  // Tracks the mode of the in-flight component run so run-done can auto-refresh availability
  // after an apply (apply scripts report what they CHANGED, which the UI must not keep showing
  // as "update available" — a fresh check is the authoritative signal).
  let lastRunMode: 'check' | 'apply' = 'check';

  // True while runBulkPlugins is iterating: the bulk loop owns the run lock and the single
  // post-loop reload, so the global run-done listener must skip its per-item lifecycle
  // (clearing `running`, reloading extensions, toasting) for plugin-mgr events.
  let bulkActive = false;

  function startRun(id: string, mode: 'check' | 'apply', append = false) {
    if (running) return;
    const comp = components.find((c) => c.id === id);
    running = id;
    lastRunMode = mode;
    const verb = mode === 'apply' ? t('page.verb_apply') : t('page.verb_check');
    const line = t('page.log_component', { name: comp?.name ?? id, verb });
    log = (append ? [...log, line] : [line]).slice(-MAX_LOG);
    runComponent(id, mode).catch(onSpawnErr);
  }

  function onCheck(id: string) {
    startRun(id, 'check');
  }

  function closeConfirm() {
    confirm = {
      open: false,
      title: '',
      message: '',
      confirmLabel: t('common.confirm'),
      action: null,
      details: [],
      requireText: null,
      danger: false
    };
  }

  function askConfirm(
    title: string,
    message: string,
    confirmLabel: string,
    action: () => void,
    opts: { details?: string[]; requireText?: string | null; danger?: boolean } = {}
  ) {
    // #120: when confirm-prompts are disabled, run immediately — except type-to-confirm actions
    // (restore/reinstall), which are destructive enough to always require explicit confirmation.
    if (!confirmDestructive && !opts.requireText) {
      action();
      return;
    }
    confirm = {
      open: true,
      title,
      message,
      confirmLabel,
      action,
      details: opts.details ?? [],
      requireText: opts.requireText ?? null,
      danger: opts.danger ?? false
    };
  }

  function doConfirm() {
    const a = confirm.action;
    closeConfirm();
    a?.();
  }

  function onApply(comp: Component) {
    askConfirm(
      t('page.confirm_apply_title'),
      t('page.confirm_apply_msg', { name: comp.name }),
      t('page.confirm_apply_btn'),
      () => startRun(comp.id, 'apply')
    );
  }

  // --- Forks tab ---
  function startForks(action: ForkAction, path?: string) {
    // A whole-stack run must not overlap per-repo runs (shared git/status-file contention).
    if (running || Object.values(forkRuns).some((r) => r?.running)) return;
    lastForkAction = action;
    running = 'forks';
    const verb =
      action === 'check'
        ? t('page.forks_verb_check')
        : action === 'plan'
          ? t('page.forks_verb_plan')
          : action === 'sync-wip'
            ? t('page.forks_verb_syncwip')
            : t('page.forks_verb_action', { action });
    log = [t('page.forks_log', { verb, path: path ? ` — ${path}` : '' })];
    runForks(action, path).catch(onSpawnErr);
  }

  // Deep-link a folder (e.g. a fork repo) into the Sessions launcher: switch tabs, seed the dialog.
  let sessionFolderReq = $state<string | null>(null);
  function openSessionFor(path: string) {
    active = 'sessions';
    sessionFolderReq = path;
  }

  function onForkAction(action: ForkAction, path?: string, label?: string) {
    if (path) {
      // Per-repo mutation -> confirm, then run CONCURRENTLY (each repo independent).
      askConfirm(
        t('page.confirm_fork_title'),
        t('page.confirm_fork_msg', { label: label ?? action }),
        t('page.confirm_fork_btn'),
        () => startForkRepo(action, path),
        { danger: action === 'delete' }
      );
    } else {
      startForks(action);
    }
  }

  // Concurrent per-repo run: marks this repo busy, streams via fork-log/fork-done. Does NOT touch
  // the global `running` lock, so other repos and the rest of the tab stay interactive.
  function startForkRepo(action: ForkAction, path: string) {
    if (forkRuns[path]?.running) return;
    forkRuns = { ...forkRuns, [path]: { line: t('page.forks_starting'), running: true, code: null } };
    runForkRepo(action, path).catch((e) => {
      log = [...log, t('page.log_error', { e })].slice(-MAX_LOG);
      forkRuns = { ...forkRuns, [path]: { line: String(e), running: false, code: -1 } };
    });
  }

  function onCancelFork(path: string) {
    cancelForkRepo(path).catch(() => {});
  }

  // Safe batch: fast-forward all behind forks (ff is non-destructive; fork-sync backs up refs).
  function onBatchFf(names: string[]) {
    if (running || names.length === 0) return;
    askConfirm(
      t('page.confirm_batchff_title'),
      t('page.confirm_batchff_msg', { n: names.length, names: names.join(', ') }),
      t('page.confirm_batchff_btn'),
      () => startForks('ff')
    );
  }

  // --- Backup tab ---
  async function reloadBackup() {
    try {
      backupData = await listBackups();
    } catch {
      backupData = null;
    }
  }

  function startBackup(action: BackupAction, opts?: RestoreOpts) {
    if (running) return;
    running = 'backup';
    const verb =
      action === 'backup'
        ? t('page.backup_verb_snapshot')
        : action === 'restore-preview'
          ? t('page.backup_verb_restore_preview')
          : t('page.backup_verb_restore');
    log = [t('page.backup_log', { verb })];
    runBackup(action, opts).catch(onSpawnErr);
  }

  function onBackupAction(action: BackupAction, opts?: RestoreOpts) {
    if (action === 'restore') {
      const snap = opts?.timestamp ?? t('page.backup_snap_last');
      askConfirm(
        t('page.confirm_restore_title'),
        t('page.confirm_restore_msg', { snap }),
        t('page.confirm_restore_btn'),
        () => startBackup('restore', opts),
        { danger: true, requireText: opts?.timestamp ?? null }
      );
    } else {
      startBackup(action, opts);
    }
  }

  // --- Profiles tab ---
  async function reloadProfiles() {
    try {
      profilesData = await readProfiles();
    } catch {
      profilesData = null;
    }
    try {
      profilesConfig = await readProfilesConfig();
    } catch {
      profilesConfig = null;
    }
    try {
      launchConfig = await readLaunchConfig();
    } catch {
      launchConfig = null;
    }
  }

  async function onSaveLaunch(
    name: string,
    mode: 'full' | 'lean',
    mcp: string[],
    claudeMd: boolean
  ) {
    try {
      await setLaunchConfig(name, mode, mcp, claudeMd);
      await reloadProfiles();
    } catch (e) {
      toastErr(e);
    }
  }

  function onMeasure(name: string, lean: boolean) {
    return measureContext(name, lean);
  }

  function onProfileMgmt(args: ProfileMgmtArgs) {
    if (running) return;
    const run = () => {
      running = 'profiles';
      const verb: Record<string, string> = {
        add: t('page.prof_verb_add', { name: args.name }),
        remove: t('page.prof_verb_remove', { name: args.name }),
        rename: t('page.prof_verb_rename', { name: args.name, newName: args.newName ?? '' }),
        recolor: t('page.prof_verb_recolor', { name: args.name }),
        'set-links': t('page.prof_verb_setlinks', { name: args.name })
      };
      log = [t('page.prof_log', { verb: verb[args.action] ?? args.action })];
      runProfileMgmt(args).catch(onSpawnErr);
    };
    if (args.action === 'remove') {
      askConfirm(
        t('page.confirm_prof_remove_title', { name: args.name }),
        t('page.confirm_prof_remove_msg', { name: args.name }),
        t('page.confirm_prof_remove_btn'),
        run
      );
    } else {
      run();
    }
  }

  function startProfiles(action: ProfileAction, name?: string) {
    if (running) return;
    running = 'profiles';
    const verb =
      action === 'check'
        ? t('page.prof_verb_check')
        : action === 'clean-conflicts'
          ? t('page.prof_verb_clean')
          : action === 'repair'
            ? t('page.prof_verb_repair', { name: name ?? '' })
            : t('page.prof_verb_reinstall');
    log = [t('page.prof_log', { verb })];
    runProfiles(action, name).catch(onSpawnErr);
  }

  function onProfileAction(action: ProfileAction, name?: string) {
    if (action === 'repair') {
      startProfiles('repair', name);
    } else if (action === 'reinstall') {
      askConfirm(
        t('page.confirm_reinstall_title'),
        t('page.confirm_reinstall_msg'),
        t('page.confirm_reinstall_btn'),
        () => startProfiles('reinstall'),
        { danger: true, requireText: t('page.confirm_reinstall_word') }
      );
    } else if (action === 'clean-conflicts') {
      askConfirm(
        t('page.confirm_clean_title'),
        t('page.confirm_clean_msg'),
        t('page.confirm_clean_btn'),
        () => startProfiles('clean-conflicts'),
        { danger: true }
      );
    } else {
      startProfiles(action);
    }
  }

  // Finish a half-built profile's folder symlinks with a one-off elevated repair (UAC).
  // Routes through the 'profiles' run slot, so run-done reloads the tab like any repair.
  function onRepairElevated(name: string) {
    if (running) return;
    running = 'profiles';
    log = [t('page.prof_log', { verb: t('page.prof_verb_repair', { name }) })];
    repairProfileElevated(name).catch(onSpawnErr);
  }

  // Relaunch the whole app elevated. On UAC-decline the Rust command returns an error
  // (the app stays open) → surface it as a toast.
  function onRelaunchAdmin() {
    relaunchAsAdmin().catch(toastErr);
  }

  function onProfileOpen(name: string) {
    openProfileDir(name).catch(toastErr);
  }

  function onProfileLaunch(name: string, mode: 'terminal' | 'vscode') {
    launchProfile(name, mode).catch(toastErr);
  }

  // --- MCP tab ---
  async function reloadMcp() {
    try {
      mcpData = await readMcp();
    } catch {
      mcpData = null;
    }
  }

  function startMcp(action: 'deploy', only?: string[]) {
    if (running) return;
    running = 'mcp';
    log = [t('page.mcp_log')];
    runMcp(action, only).catch(onSpawnErr);
  }

  // No arg → deploy to all profiles (confirm). A profile name or array → deploy just those.
  function onMcpDeploy(target?: string | string[]) {
    if (target) {
      startMcp('deploy', Array.isArray(target) ? target : [target]);
    } else {
      askConfirm(
        t('page.confirm_mcp_title'),
        t('page.confirm_mcp_msg'),
        t('page.confirm_mcp_btn'),
        () => startMcp('deploy')
      );
    }
  }

  // --- Sync tab ---
  async function reloadSync() {
    try {
      syncData = await readSync();
    } catch {
      syncData = null;
    }
  }

  async function reloadConfigDrift() {
    try {
      driftData = await readConfigDrift();
    } catch {
      driftData = null;
    }
  }

  // --- Home / Overview tab (USE-1): aggregates the other tabs' data ---
  let homeLoaded = $state(false);
  async function reloadHome() {
    await Promise.all([reloadProfiles(), reloadConfigDrift(), reloadSync(), reloadSchedules()]);
  }
  $effect(() => {
    if (active === 'home' && !homeLoaded) {
      homeLoaded = true;
      setLoading('home', true);
      reloadHome().finally(() => setLoading('home', false));
    }
  });

  function startSync(action: 'query' | 'set', enabled?: string[]) {
    if (running) return;
    running = 'sync';
    log = [action === 'set' ? t('page.sync_log_set') : t('page.sync_log_query')];
    runSync(action, enabled).catch(onSpawnErr);
  }

  // Lazy-load on first open + run a fresh query to fetch live Syncthing status.
  $effect(() => {
    if (active === 'sync' && !syncLoaded) {
      syncLoaded = true;
      setLoading('sync', true);
      Promise.all([reloadSync(), reloadConfigDrift(), reloadProfiles()])
        .then(() => startSync('query'))
        .finally(() => setLoading('sync', false));
    }
  });

  function onSyncRefresh() {
    startSync('query');
  }

  function onSyncApply(enabled: string[]) {
    const all = ['history', 'projects', 'skills', 'agents', 'commands', 'keybindings'];
    const off = all.filter((i) => !enabled.includes(i));
    const detail = off.length
      ? t('page.sync_apply_off', { off: off.join(', ') })
      : t('page.sync_apply_all');
    askConfirm(t('page.confirm_sync_title'), detail, t('page.confirm_sync_btn'), () =>
      startSync('set', enabled)
    );
  }

  // --- Config drift (FUN-7): shares the 'sync' run slot + outcome/toast ---
  function startConfigDrift(action: ConfigDriftAction) {
    if (running) return;
    running = 'sync';
    const verb =
      action === 'relink'
        ? t('page.drift_verb_relink')
        : action === 'sync-now'
          ? t('page.drift_verb_sync')
          : t('page.drift_verb_check');
    log = [t('page.drift_log', { verb })];
    runConfigDrift(action).catch(onSpawnErr);
  }

  function onSyncDrift(action: ConfigDriftAction) {
    if (action === 'check') {
      startConfigDrift('check');
    } else if (action === 'relink') {
      askConfirm(t('page.confirm_relink_title'), t('page.confirm_relink_msg'),
        t('page.confirm_relink_btn'), () => startConfigDrift('relink'));
    } else {
      askConfirm(t('page.confirm_driftsync_title'), t('page.confirm_driftsync_msg'),
        t('page.confirm_driftsync_btn'), () => startConfigDrift('sync-now'));
    }
  }

  // --- Providers / engines tab ---
  async function reloadProviders() {
    try {
      enginesData = await readEngines();
    } catch {
      enginesData = null;
    }
    try {
      providersData = await readProviders();
    } catch {
      providersData = null;
    }
    try {
      myProvidersData = await listMyProviders();
    } catch {
      myProvidersData = null;
    }
  }

  async function reloadStack() {
    try {
      stackData = await readStack();
    } catch {
      stackData = null;
    }
  }

  async function reloadOpencode() {
    try {
      opencodeData = await readOpencode();
    } catch {
      opencodeData = null;
    }
  }

  $effect(() => {
    // Providers/engines are shown both on their own tab and inside profile cards.
    if ((active === 'providers' || active === 'profiles') && !providersLoaded) {
      providersLoaded = true;
      const tab = active;
      setLoading(tab, true);
      reloadProviders().finally(() => setLoading(tab, false));
    }
    if (active === 'providers' && !stackData) reloadStack();
    if (active === 'providers' && !opencodeData) reloadOpencode();
  });

  // Start (-Router, incl. paid GLM) or stop (-All) the whole LLM stack via stack scripts.
  function onStack(action: 'start' | 'stop' | 'restart', only?: string) {
    if (running) return;
    const verb =
      action === 'start'
        ? t('page.stack_verb_start')
        : action === 'restart'
          ? t('page.stack_verb_restart')
          : t('page.stack_verb_stop');
    const go = () => {
      running = 'engine';
      log = [t('page.stack_log', { verb })];
      runStack(action, only).catch(onSpawnErr);
    };
    // Confirm only the destructive "stop the whole stack"; single-service stop is cheap to undo.
    if (action === 'stop' && !only) {
      askConfirm(
        t('page.confirm_stack_stop_title'),
        t('page.confirm_stack_stop_msg'),
        t('page.confirm_stack_stop_btn'),
        go
      );
    } else {
      go();
    }
  }

  function onEngineAction(action: 'start' | 'stop', id: string) {
    if (running) return;
    const run = () => {
      running = 'engine';
      log = [
        t('page.engine_log', {
          id,
          verb: action === 'start' ? t('page.engine_verb_start') : t('page.engine_verb_stop')
        })
      ];
      runEngine(action, id).catch(onSpawnErr);
    };
    if (action === 'stop') {
      askConfirm(
        t('page.confirm_engine_stop_title'),
        t('page.confirm_engine_stop_msg', { id }),
        t('page.confirm_engine_stop_btn'),
        run
      );
    } else {
      run();
    }
  }

  function startProvider(args: ProviderArgs) {
    if (running) return; // match every sibling start* — don't clobber an in-flight run
    running = 'provider';
    log = [
      t('page.provider_log', {
        name: args.name,
        verb: args.action === 'set' ? t('page.provider_verb_set') : t('page.provider_verb_clear')
      })
    ];
    runProvider(args).catch(onSpawnErr);
  }

  function onProviderSet(args: ProviderArgs) {
    if (running) return;
    startProvider(args);
  }

  function onProviderClear(name: string) {
    if (running) return;
    askConfirm(
      t('page.confirm_provider_clear_title'),
      t('page.confirm_provider_clear_msg', { name }),
      t('page.confirm_provider_clear_btn'),
      () => startProvider({ action: 'clear', name })
    );
  }

  function onOpenUrl(url: string) {
    openPath(url).catch(toastErr);
  }

  // --- Custom provider registry handlers ---
  function onMyProviderSave(p: MyProviderInput, apiKey: string) {
    saveMyProvider(p, apiKey || undefined)
      .then(() => reloadProviders())
      .catch(toastErr);
  }
  function onMyProviderDelete(id: string) {
    askConfirm(
      t('myProviders.confirmDeleteTitle'),
      t('myProviders.confirmDeleteMsg'),
      t('myProviders.delete'),
      () =>
        deleteMyProvider(id)
          .then(() => reloadProviders())
          .catch(toastErr)
    );
  }
  function onMyProviderConnect(id: string) {
    if (running) return;
    running = 'provider';
    log = [t('myProviders.connectLog')];
    connectMyProvider(id).catch(onSpawnErr);
  }
  function onMyProviderAddKey(id: string, apiKey: string) {
    addProviderKey(id, apiKey)
      .then(() => reloadProviders())
      .catch(toastErr);
  }
  function onMyProviderRemoveKey(id: string, index: number) {
    removeProviderKey(id, index)
      .then(() => reloadProviders())
      .catch(toastErr);
  }
  function onMyProviderNextKey(id: string) {
    if (running) return;
    running = 'provider';
    log = [t('myProviders.nextKeyLog')];
    nextProviderKey(id).catch(onSpawnErr);
  }
  function onSetFreellmapiAuth(email: string, password: string, token: string) {
    setFreellmapiAuth(email || undefined, password || undefined, token || undefined)
      .then(() => (log = [...log, t('myProviders.loginSaved')]))
      .catch(toastErr);
  }

  function onRouterInstall() {
    if (running) return;
    running = 'engine';
    log = [t('page.router_install_log')];
    runRouter('install').catch(onSpawnErr);
  }

  function onConnectRouter(engine: EngineStatus, model: string, profile: string) {
    if (running) return;
    askConfirm(
      t('page.confirm_router_title'),
      t('page.confirm_router_msg', { profile, engine: engine.name, model }),
      t('page.confirm_router_btn'),
      () => {
        running = 'provider';
        log = [t('page.router_log', { engine: engine.name, model, profile })];
        runConnectRouter(engine.baseUrl, model, profile, engine.id).catch(onSpawnErr);
      }
    );
  }

  // Point opencode at an OpenAI-compatible engine (writes opencode.json). The gateway engine
  // reuses the existing "freellmapi" provider id (reconciles its config). apiKey: literal if
  // typed; else keep the existing one; else an {env:FREELLMAPI_KEY} placeholder.
  function onConnectOpencode(engine: EngineStatus, model: string, key: string) {
    if (running) return;
    const providerId = engine.id === 'llmstack' ? 'freellmapi' : engine.id;
    const existing = opencodeData?.providers?.find((p) => p.id === providerId);
    const args: import('$lib/ipc').OpencodeProviderArgs = {
      action: 'set',
      providerId,
      name: engine.name,
      baseUrl: engine.baseUrl,
      model
    };
    if (key.trim()) args.key = key.trim();
    else if (existing?.hasKey) args.keepKey = true;
    else args.envKey = 'FREELLMAPI_KEY';
    askConfirm(
      t('page.confirm_opencode_title'),
      t('page.confirm_opencode_msg', { engine: engine.name, model }),
      t('page.confirm_opencode_btn'),
      () => {
        running = 'provider';
        log = [t('page.opencode_log', { engine: engine.name, model })];
        runOpencodeProvider(args).catch(onSpawnErr);
      }
    );
  }

  // --- Schedule tab ---
  async function reloadSchedules() {
    try {
      schedulesData = await readSchedules();
      schedulesLoaded = true;
    } catch {
      schedulesData = null;
    }
  }

  // Lazy-load schedules the first time the tab is opened (query spawns pwsh).
  $effect(() => {
    if (active === 'schedule' && !schedulesLoaded) {
      setLoading('schedule', true);
      reloadSchedules().finally(() => setLoading('schedule', false));
    }
  });

  function startSchedule(action: ScheduleAction, id: string, time?: string) {
    if (running) return;
    running = 'schedule';
    const verb: Record<ScheduleAction, string> = {
      enable: t('page.sched_verb_enable'),
      disable: t('page.sched_verb_disable'),
      run: t('page.sched_verb_run'),
      create: t('page.sched_verb_create'),
      delete: t('page.sched_verb_delete')
    };
    log = [t('page.sched_log', { id, verb: verb[action] })];
    runSchedule(action, id, time).catch(onSpawnErr);
  }

  function onScheduleAction(action: ScheduleAction, id: string, time?: string) {
    if (action === 'delete') {
      askConfirm(
        t('page.confirm_sched_delete_title'),
        t('page.confirm_sched_delete_msg', { id }),
        t('page.confirm_sched_delete_btn'),
        () => startSchedule('delete', id)
      );
    } else {
      startSchedule(action, id, time);
    }
  }

  // --- Plugins & Skills tab ---
  async function reloadPluginUpdates() {
    try {
      pluginUpdates = await listPluginUpdates();
    } catch {
      pluginUpdates = [];
    }
  }

  async function reloadExtensions() {
    try {
      pluginsData = await listPlugins();
    } catch {
      pluginsData = null;
    }
    try {
      skillsData = await listSkills();
    } catch {
      skillsData = null;
    }
    try {
      pluginContents = await listPluginContents();
    } catch {
      pluginContents = [];
    }
    await reloadPluginUpdates();
    extensionsLoaded = true;
  }

  // Sidebar "needs attention" indicators, from already-loaded data.
  const attention = $derived({
    updates: updatesAttention(components, statuses),
    forks: forksAttention(statuses.forks),
    backup: backupAttention(backupData),
    profiles: profilesAttention(profilesData),
    sync: syncAttention(syncData),
    extensions: pluginsAttention(pluginUpdates.length)
  });

  // Lazy-load on first open (list_plugins spawns the claude CLI).
  $effect(() => {
    if (active === 'extensions' && !extensionsLoaded) {
      setLoading('extensions', true);
      reloadExtensions().finally(() => setLoading('extensions', false));
    }
  });

  // A tab shows the "refreshing" overlay + sidebar spinner while it fetches fresh data.
  // Forks piggybacks on the global run lock (its check is a script run, not a native read).
  const tabLoading = $derived.by(() => {
    const m: Record<string, boolean> = { ...loading };
    if (running === 'forks') m.forks = true;
    return m;
  });
  const tabRefreshing = $derived(!!tabLoading[active]);
  // Forks "check" is a long (~40s) script run; dimming + disabling the whole tab made it feel frozen.
  // Keep the lightweight "refreshing" pill, but don't block the forks tab — cards stay readable,
  // scrollable and expandable (per-repo mutation buttons still gate on `running` for git safety).
  const blockingRefresh = $derived(tabRefreshing && active !== 'forks');

  // Lazy: refresh forks on first open this session (cached on disk → can be stale).
  // Re-runs once the run lock frees if something else was running when the tab opened.
  $effect(() => {
    if (active === 'forks' && !forksChecked && !running) {
      forksChecked = true;
      startForks('check');
    }
  });

  // Native gh call (not run-locked) — load the full GitHub repo list once on first open.
  $effect(() => {
    if (active === 'forks' && !githubLoaded) {
      githubLoaded = true;
      listGithubRepos()
        .then((r) => (githubRepos = r))
        .catch(() => (githubRepos = []));
    }
  });

  function startPlugin(action: PluginAction, id: string) {
    if (running) return;
    running = 'plugin-mgr';
    const verb =
      action === 'update'
        ? t('page.plugin_verb_update')
        : action === 'enable'
          ? t('page.plugin_verb_enable')
          : action === 'remove'
            ? t('page.plugin_verb_remove')
            : t('page.plugin_verb_disable');
    log = [t('page.plugin_log', { id, verb })];
    runPlugin(action, id).catch(onSpawnErr);
  }

  function onPluginAction(action: PluginAction, id: string) {
    if (action === 'disable') {
      askConfirm(
        t('page.confirm_plugin_disable_title'),
        t('page.confirm_plugin_disable_msg', { id }),
        t('page.confirm_plugin_disable_btn'),
        () => startPlugin('disable', id)
      );
    } else if (action === 'remove') {
      askConfirm(
        t('page.confirm_plugin_remove_title'),
        t('page.confirm_plugin_remove_msg', { id }),
        t('page.confirm_plugin_remove_btn'),
        () => startPlugin('remove', id),
        { danger: true }
      );
    } else {
      startPlugin(action, id);
    }
  }

  // Bulk plugin ops run sequentially through the single run slot (one op at a time).
  async function runBulkPlugins(action: PluginAction, ids: string[]) {
    if (!ids.length || running) return;
    const verb =
      action === 'update'
        ? t('page.plugin_verb_update')
        : action === 'enable'
          ? t('page.plugin_verb_enable')
          : action === 'remove'
            ? t('page.plugin_verb_remove')
            : t('page.plugin_verb_disable');
    running = 'plugin-mgr';
    bulkActive = true;
    try {
      for (const id of ids) {
        log = [...log, t('page.plugin_log', { id, verb })];
        try {
          await runPlugin(action, id);
        } catch (e) {
          log = [...log, t('page.log_error', { e: String(e) })];
        }
      }
    } finally {
      bulkActive = false;
      running = null;
      reloadExtensions();
    }
  }
  function onBulkPlugin(action: PluginAction, ids: string[]) {
    if (!ids.length) return;
    if (action === 'remove') {
      askConfirm(
        t('page.confirm_plugin_remove_title'),
        t('page.confirm_plugin_bulk_remove_msg', { count: ids.length }),
        t('page.confirm_plugin_remove_btn'),
        () => runBulkPlugins('remove', ids),
        { danger: true }
      );
    } else {
      runBulkPlugins(action, ids);
    }
  }

  function onOpenSkills() {
    const d = skillsData?.[0]?.dir;
    if (!d) return;
    const parent = d.slice(0, Math.max(d.lastIndexOf('\\'), d.lastIndexOf('/')));
    if (parent) openPath(parent).catch(toastErr);
  }

  function onOpenSkill(dir: string) {
    openPath(dir).catch(toastErr);
  }
  function onDeleteSkill(dir: string, name: string) {
    askConfirm(
      t('page.confirm_skill_delete_title'),
      t('page.confirm_skill_delete_msg', { name }),
      t('page.confirm_skill_delete_btn'),
      () => {
        deleteSkill(dir)
          .then(() => reloadExtensions())
          .catch(toastErr);
      },
      { danger: true }
    );
  }

  async function cancel() {
    try {
      await cancelRun();
    } catch (e) {
      log = [...log, t('page.log_warn', { e: String(e) })];
    }
  }

  function setTheme(th: Theme) {
    theme = th;
    applyTheme(th);
  }

  // View prefs (UI-only, persisted in localStorage): compact density + full-width content.
  let density = $state<'comfortable' | 'compact'>('comfortable');
  let fullWidth = $state(true);
  // #120: gate confirm dialogs for destructive actions (type-to-confirm ones always prompt).
  let confirmDestructive = $state(true);
  function setConfirmDestructive(v: boolean) {
    confirmDestructive = v;
    try {
      localStorage.setItem('cmh-confirm-destructive', v ? '1' : '0');
    } catch {
      /* ignore */
    }
  }
  function setDensity(d: 'comfortable' | 'compact') {
    density = d;
    try {
      localStorage.setItem('cmh-density', d);
    } catch {
      /* ignore */
    }
  }
  function setFullWidth(v: boolean) {
    fullWidth = v;
    try {
      localStorage.setItem('cmh-fullwidth', v ? '1' : '0');
    } catch {
      /* ignore */
    }
  }

  // Command palette (Ctrl+K): jump to any tab + a few quick actions.
  const NAV_IDS = ['home', 'updates', 'forks', 'backup', 'profiles', 'mcp', 'sync', 'providers', 'sessions', 'analytics', 'extensions', 'schedule', 'settings'];
  let paletteOpen = $state(false);
  let hotkeyHelpOpen = $state(false);
  const paletteCommands = $derived([
    ...NAV_IDS.map((id) => ({ id: `tab:${id}`, label: t(`nav.${id}`), run: () => (active = id) })),
    {
      id: 'act:density',
      label: `${t('settings.density')}: ${density === 'compact' ? t('settings.densityComfortable') : t('settings.densityCompact')}`,
      run: () => setDensity(density === 'compact' ? 'comfortable' : 'compact')
    },
    {
      id: 'act:theme',
      label: `${t('settings.theme')}: ${theme === 'dark' ? t('settings.themeLight') : t('settings.themeDark')}`,
      run: () => setTheme(theme === 'dark' ? 'light' : 'dark')
    }
  ]);
  function onGlobalKey(e: KeyboardEvent) {
    if (e.ctrlKey && (e.key === 'k' || e.key === 'K')) {
      e.preventDefault();
      paletteOpen = !paletteOpen;
      return;
    }
    // "?" opens the keyboard-shortcut cheatsheet — but not while typing in a field.
    const tgt = e.target as HTMLElement | null;
    const typing = !!tgt && (tgt.tagName === 'INPUT' || tgt.tagName === 'TEXTAREA' || tgt.isContentEditable);
    if (e.key === '?' && !typing) {
      e.preventDefault();
      hotkeyHelpOpen = true;
    }
  }

  // Persist the active tab so the app reopens where you left off.
  $effect(() => {
    try {
      localStorage.setItem('cmh-active-tab', active);
    } catch {
      /* ignore */
    }
  });

  onMount(async () => {
    theme = getTheme();
    // Mirror the resolved UI locale into the backend so errors/run-log/tray match, even before
    // the user ever opens the language switcher.
    setLanguage(locale.current).catch(() => {});
    try {
      if (localStorage.getItem('cmh-density') === 'compact') density = 'compact';
      fullWidth = localStorage.getItem('cmh-fullwidth') !== '0';
      confirmDestructive = localStorage.getItem('cmh-confirm-destructive') !== '0';
      // Restore the last-open tab (validated against the known set).
      const savedTab = localStorage.getItem('cmh-active-tab');
      if (savedTab && NAV_IDS.includes(savedTab)) active = savedTab;
    } catch {
      /* ignore */
    }
    try {
      components = await listComponents();
      await Promise.all(components.map(loadStatus));
    } catch (e) {
      loadError = String(e);
    }
    reloadBackup();
    // Await profiles so the first-run check sees real data (no profiles → empty-setup signal).
    await reloadProfiles();
    reloadMcp();
    reloadPluginUpdates();
    // First-run onboarding: decide once, after config + profiles are known.
    maybeShowOnboarding();

    unlisten.push(
      await listen<{ component: string; stream: string; line: string }>('run-log', (e) => {
        const p = e.payload;
        log = [...log, (p.stream === 'err' ? '⚠ ' : '') + p.line].slice(-MAX_LOG);
      })
    );
    unlisten.push(
      await listen<{ component: string; code: number }>('run-done', async (e) => {
        log = [...log, t('page.log_done', { code: e.payload.code })].slice(-MAX_LOG);
        // During a bulk plugin op, runBulkPlugins holds the run lock and does a single
        // reload/lifecycle after the whole loop — skip the per-item side effects here.
        if (e.payload.component === 'plugin-mgr' && bulkActive) return;
        running = null;
        const id = e.payload.component;
        const code = e.payload.code;
        const wasApply = lastRunMode === 'apply';
        lastRunMode = 'check';
        const forkAct = lastForkAction;
        lastForkAction = null;
        const c = components.find((x) => x.id === id);
        if (c) await loadStatus(c);
        // Component-specific data reloads (keep fresh before surfacing the outcome).
        if (id === 'backup') await reloadBackup();
        if (id === 'profiles') await reloadProfiles();
        if (id === 'mcp') await reloadMcp();
        if (id === 'sync') {
          await reloadSync();
          await reloadConfigDrift();
          await reloadProfiles();
        }
        if (id === 'engine' || id === 'provider') await reloadProviders();
        if (id === 'engine') await reloadStack();
        if (id === 'provider') await reloadOpencode();
        if (id === 'schedule') await reloadSchedules();
        if (id === 'plugin-mgr') await reloadExtensions();

        // Auto-recheck after a successful single-component apply (apply scripts write the applied
        // count, not availability) — toast appears after the follow-up check, not the apply.
        if (wasApply && code === 0 && c && c.supportsApply && c.id !== 'all') {
          startRun(c.id, 'check', true);
          return;
        }
        // Auto-recheck after a mutating fork action so the cards reflect the new state.
        if (id === 'forks' && code === 0 && forkAct && forkAct !== 'check' && forkAct !== 'plan') {
          log = [...log, t('page.forks_recheck')];
          startForks('check');
          return;
        }

        // Surface a glanceable, actionable outcome (toast) — not just the log.
        try {
          if (c) {
            // Update/forks components: rich outcome from the status envelope.
            const out = deriveOutcome({
              id,
              name: c.name,
              code,
              mode: wasApply ? 'apply' : 'check',
              status: statuses[id]
            });
            pushToast({
              kind: out.kind,
              title: out.title,
              detail: out.detail,
              action: out.action
                ? {
                    label: out.action.label,
                    onClick: () => {
                      if (out.action!.kind === 'log') consoleReveal++;
                      else if (out.action!.kind === 'tab' && out.action!.target) active = out.action!.target;
                    }
                  }
                : undefined
            });
          } else {
            // Operational actions (provider/mcp/sync/backup/profiles/schedule/plugins): simple result.
            const name = opName(id);
            if (code === 0) {
              pushToast({ kind: 'success', title: t('page.toast_op_done', { name }) });
            } else {
              pushToast({
                kind: 'error',
                title: t('page.toast_op_error', { name, code }),
                detail: t('page.toast_op_error_detail'),
                action: { label: t('page.toast_open_log'), onClick: () => consoleReveal++ }
              });
            }
          }
        } catch {
          /* surfacing the outcome must never break the run lifecycle */
        }
      })
    );
    // Per-repo concurrent fork runs (component = repo path).
    unlisten.push(
      await listen<{ component: string; stream: string; line: string }>('fork-log', (e) => {
        const p = e.payload;
        const name = p.component.split(/[\\/]/).pop() || p.component;
        log = [...log, `[${name}] ${p.stream === 'err' ? '⚠ ' : ''}${p.line}`].slice(-MAX_LOG);
        const prev = forkRuns[p.component];
        forkRuns = { ...forkRuns, [p.component]: { line: p.line, running: true, code: prev?.code ?? null } };
      })
    );
    unlisten.push(
      await listen<{ component: string; code: number }>('fork-done', async (e) => {
        const path = e.payload.component;
        const code = e.payload.code;
        const name = path.split(/[\\/]/).pop() || path;
        log = [...log, `[${name}] ${t('page.log_done', { code })}`].slice(-MAX_LOG);
        const prev = forkRuns[path];
        forkRuns = { ...forkRuns, [path]: { line: prev?.line ?? '', running: false, code } };
        // Merge ONLY this repo's fresh state (from its per-repo JSON) — no full rescan, no race.
        if (code === 0) {
          const updated = await readForkRepoStatus(path).catch(() => null);
          const cur = statuses.forks;
          if (updated && cur?.repos) {
            statuses = {
              ...statuses,
              forks: { ...cur, repos: cur.repos.map((r: any) => (r.Path === path ? updated : r)) }
            };
          }
        }
        pushToast(
          code === 0
            ? { kind: 'success', title: t('page.toast_fork_done', { name }) }
            : {
                kind: 'error',
                title: t('page.toast_fork_error', { name, code }),
                action: { label: t('page.toast_open_log'), onClick: () => consoleReveal++ }
              }
        );
      })
    );
    unlisten.push(
      await listen('tray-check-all', () => {
        // A run is already in flight: startRun would no-op anyway, so don't force-switch the tab
        // or emit the diag line for a request we can't honor (silent busy — the active run's own
        // progress/outcome is the feedback).
        if (running) return;
        // DIAGNOSTIC: this is the ONLY code path that force-switches to Updates. If the tab jumps
        // without you clicking the tray's "Проверить всё", this line will show in the log — proving
        // a spurious tray event is the source.
        log = [...log, `[diag] tray-check-all @ ${new Date().toLocaleTimeString()}`];
        active = 'updates';
        startRun('all', 'check');
      })
    );

    // Refresh statuses when the window regains focus.
    const onFocus = () => {
      if (running) return; // don't reload statuses mid-run (avoids extra pwsh spawns + flicker)
      // Cheap: re-read the on-disk *.last.json envelopes (back the Updates tab + sidebar badges).
      components.forEach(loadStatus);
      // Heavy (each spawns pwsh / the claude CLI): only refresh the dataset whose tab is visible —
      // off-screen tabs re-fetch lazily on next open, so don't burst N pwsh spawns on every alt-tab.
      if (active === 'backup') reloadBackup();
      if (active === 'profiles' || active === 'home' || active === 'sync') reloadProfiles();
      if (active === 'mcp') reloadMcp();
      // pluginUpdates also feeds the sidebar "extensions" badge, so keep it fresh app-wide.
      reloadPluginUpdates();
    };
    window.addEventListener('focus', onFocus);
    unlisten.push(() => window.removeEventListener('focus', onFocus));
  });

  onDestroy(() => unlisten.forEach((u) => u()));

</script>

<svelte:window onkeydown={onGlobalKey} />
<CommandPalette open={paletteOpen} commands={paletteCommands} placeholder={t('common.paletteSearch')} onClose={() => (paletteOpen = false)} />

<div class="flex h-full overflow-hidden" class:dense={density === 'compact'}>
  <Sidebar {active} onSelect={(id) => (active = id)} {attention} loading={tabLoading} />

  <div class="flex min-w-0 flex-1 flex-col">
    <main class="relative min-h-0 flex-1 overflow-auto">
      <div class="relative mx-auto w-full {fullWidth ? '' : 'max-w-[1600px]'}">
      {#if loadError}
        <div class="m-sw-6 sw-card text-red-400">{t('page.load_error', { e: loadError })}</div>
      {/if}

      {#if tabRefreshing}
        <div
          class="pointer-events-none absolute left-1/2 top-sw-4 z-10 flex -translate-x-1/2 items-center gap-sw-2 rounded-full border border-sw-border bg-sw-bg-secondary px-sw-3 py-sw-1 text-sw-sm text-sw-text-secondary shadow"
        >
          <Spinner size={16} />
          {t('common.refreshing')}
        </div>
      {/if}

      <div
        class="transition-opacity duration-200"
        class:opacity-40={blockingRefresh}
        class:pointer-events-none={blockingRefresh}
      >
      {#if active === 'home'}
        <HomeTab profiles={profilesData} sync={syncData} drift={driftData} schedules={schedulesData}
          onOpen={(id) => (active = id)} onRefresh={reloadHome} />
      {:else if active === 'updates'}
        <UpdatesTab {components} {statuses} {running} {onCheck} {onApply} onOpenTab={(id) => (active = id)} />
      {:else if active === 'forks'}
        <ForksTab status={statuses.forks} {githubRepos} {running} {forkRuns} onAction={onForkAction} {onCancelFork} {onBatchFf} {onOpenUrl} onOpenSession={openSessionFor} />
      {:else if active === 'backup'}
        <BackupTab data={backupData} {running} profiles={(profilesData?.profiles ?? []).map((p) => p.name)} onAction={onBackupAction} />
      {:else if active === 'profiles'}
        <ProfilesTab
          data={profilesData}
          config={profilesConfig}
          {launchConfig}
          providers={providersData}
          engines={enginesData}
          {running}
          onAction={onProfileAction}
          onMgmt={onProfileMgmt}
          onOpen={onProfileOpen}
          onLaunch={onProfileLaunch}
          {onSaveLaunch}
          {onMeasure}
          {onProviderSet}
          {onProviderClear}
          onOpenProviders={() => (active = 'providers')}
          {onRepairElevated}
          {onRelaunchAdmin}
        />
      {:else if active === 'mcp'}
        <McpTab data={mcpData} {running} onRefresh={reloadMcp} onDeploy={onMcpDeploy} />
      {:else if active === 'sync'}
        <SyncTab data={syncData} {running} onRefresh={onSyncRefresh} onApply={onSyncApply}
          driftData={driftData} conflictCount={profilesData?.syncConflicts?.count ?? 0}
          onDriftApply={onSyncDrift} onCleanConflicts={() => onProfileAction('clean-conflicts')} />
      {:else if active === 'providers'}
        <ProvidersTab
          engines={enginesData}
          providers={providersData}
          stack={stackData}
          {running}
          onEngine={onEngineAction}
          onStack={onStack}
          onProviderSet={onProviderSet}
          onProviderClear={onProviderClear}
          onRouterInstall={onRouterInstall}
          onConnectRouter={onConnectRouter}
          onConnectOpencode={onConnectOpencode}
          onOpenProfiles={() => (active = 'profiles')}
          myProviders={myProvidersData}
          {onMyProviderSave}
          {onMyProviderDelete}
          {onMyProviderConnect}
          {onMyProviderAddKey}
          {onMyProviderRemoveKey}
          {onMyProviderNextKey}
          {onSetFreellmapiAuth}
          onRefresh={() => {
            reloadProviders();
            reloadStack();
            reloadOpencode();
          }}
          {onOpenUrl}
        />
      {:else if active === 'analytics'}
        <AnalyticsTab onOpenProviders={() => (active = 'providers')} />
      {:else if active === 'extensions'}
        <PluginsTab
          plugins={pluginsData}
          skills={skillsData}
          updates={pluginUpdates}
          contents={pluginContents}
          {running}
          onAction={onPluginAction}
          {onBulkPlugin}
          onRefresh={reloadExtensions}
          {onOpenSkills}
          {onOpenSkill}
          {onDeleteSkill}
        />
      {:else if active === 'schedule'}
        <ScheduleTab data={schedulesData} {running} onAction={onScheduleAction} onRefresh={reloadSchedules} />
      {:else if active === 'settings'}
        <SettingsTab {theme} onSetTheme={setTheme} {density} {fullWidth} onSetDensity={setDensity} onSetFullWidth={setFullWidth} {confirmDestructive} onSetConfirmDestructive={setConfirmDestructive} />
      {:else if active !== 'sessions'}
        <div class="grid h-full place-items-center p-sw-6 text-center text-sw-text-muted">
          <div>
            <div class="mb-sw-2 text-2xl">🛠</div>
            <div class="font-medium text-sw-text">{t('nav.' + active)}</div>
            <div class="text-sw-sm">{t('page.wip')}</div>
          </div>
        </div>
      {/if}
      </div>
      </div>

      <!-- Sessions tab is full-bleed and stays MOUNTED (display-toggled), so running
           terminals survive switching to another tab. -->
      <div class="absolute inset-0 {active === 'sessions' ? '' : 'hidden'}">
        <SessionsTab visible={active === 'sessions'} profiles={(profilesData?.profiles ?? []).map((p) => p.name)} folderReq={sessionFolderReq} onFolderReqConsumed={() => (sessionFolderReq = null)} />
      </div>
    </main>

    <Console {log} {running} revealSignal={consoleReveal} onClear={() => (log = [])} onCancel={cancel} />
  </div>
</div>

<ToastHost />
<HotkeyHelp open={hotkeyHelpOpen} onClose={() => (hotkeyHelpOpen = false)} />

<OnboardingWizard
  open={onboardingOpen}
  profileCount={profilesData?.profiles?.length ?? 0}
  busy={running === 'profiles'}
  onCreateProfile={onProfileMgmt}
  onOpenProfiles={() => {
    onboardingOpen = false;
    try {
      localStorage.setItem(ONBOARDED_KEY, '1');
    } catch {
      /* ignore */
    }
    active = 'profiles';
  }}
  onFinish={finishOnboarding}
/>

<ConfirmDialog
  open={confirm.open}
  title={confirm.title}
  message={confirm.message}
  confirmLabel={confirm.confirmLabel}
  details={confirm.details}
  requireText={confirm.requireText}
  danger={confirm.danger}
  onConfirm={doConfirm}
  onCancel={closeConfirm}
/>
