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
    repairAllProfiles,
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
    mcpUpsertServer,
    mcpRemoveServer,
    mcpRemoveExtra,
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
    deleteFreellmapiAuth,
    listGithubRepos,
    cloneRepo,
    pickFolder,
    readStack,
    runStack,
    readOpencode,
    runOpencodeProvider,
    canonicalSkillsDir,
    globalSessionCount,
    quitApp,
    openPath,
    openUrl,
    listPlugins,
    listSkills,
    deleteSkill,
    listPluginUpdates,
    listPluginContents,
    runPlugin,
    runPluginsBulk,
    pluginSyncStatus,
    pluginSyncSet,
    runPluginSync,
    readSchedules,
    runSchedule,
    cancelRun,
    cancelAll,
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
    type PluginContents,
    type PluginSyncStatus
  } from '$lib/ipc';
  import {
    updatesAttention,
    forksAttention,
    backupAttention,
    profilesAttention,
    pluginsAttention,
    syncAttention,
    sessionsAttention
  } from '$lib/attention';
  import { agentSummary } from '$lib/agentStatus.svelte';
  import { getTheme, applyTheme, type Theme } from '$lib/theme';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import Spinner from '$lib/components/Spinner.svelte';
  import { navOrder } from '$lib/navOrder.svelte';
  import Console from '$lib/components/Console.svelte';
  import UpdatesTab from '$lib/components/UpdatesTab.svelte';
  import ForksTab from '$lib/components/ForksTab.svelte';
  import BackupTab from '$lib/components/BackupTab.svelte';
  import HotkeyHelp from '$lib/components/HotkeyHelp.svelte';
  import ProfilesTab from '$lib/components/ProfilesTab.svelte';
  import McpTab from '$lib/components/McpTab.svelte';
  import EnvironmentsTab from '$lib/components/EnvironmentsTab.svelte';
  import SyncTab from '$lib/components/SyncTab.svelte';
  import HomeTab from '$lib/components/HomeTab.svelte';
  import ProvidersTab from '$lib/components/ProvidersTab.svelte';
  // SessionsTab pulls in the heavy xterm/WebGL terminal stack (~216 kB gzip). It is dynamically
  // imported on first open (see the template) so a user who never opens Sessions doesn't pay its
  // download/parse on cold start; once opened it stays mounted so running terminals survive.
  import CommandPalette from '$lib/components/CommandPalette.svelte';
  import AnalyticsTab from '$lib/components/AnalyticsTab.svelte';
  import PluginsTab from '$lib/components/PluginsTab.svelte';
  import ScheduleTab from '$lib/components/ScheduleTab.svelte';
  import SettingsTab from '$lib/components/SettingsTab.svelte';
  import OnboardingWizard from '$lib/components/OnboardingWizard.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import ToastHost from '$lib/components/ToastHost.svelte';
  import NotificationPanel from '$lib/components/NotificationPanel.svelte';
  import { pushToast } from '$lib/toast.svelte';
  import { runningStore, opName } from '$lib/running.svelte';
  import { pushRun } from '$lib/runHistory.svelte';
  import { deriveOutcome } from '$lib/outcome';
  import { t, locale } from '$lib/i18n';
  import { setLanguage, readEnvironments, readSkillMatrix, shareSkills, runOpencodeRtk, runOpencodeMcp, runOpencodeProviders, runOpencodeInstructions, runCodexMcp, runCodexProviders, type EnvInfo, type SkillRow } from '$lib/ipc';

  let components = $state<Component[]>([]);
  let statuses = $state<Record<string, any>>({});
  let running = $state<string | null>(null);
  let log = $state<string[]>([]);
  /** Cap the console buffer so a chatty/stuck script can't grow it without bound. */
  const MAX_LOG = 5000;
  /** Append one line to the console buffer, trimming the oldest past MAX_LOG. Svelte 5 $state array:
   *  push/splice mutate reactively and avoid rebuilding the whole array per line (item 7 perf). */
  function appendLog(line: string) {
    log.push(line);
    if (log.length > MAX_LOG) log.splice(0, log.length - MAX_LOG);
  }
  // R2: restore the saved tab HERE, in the state initializer — the persist $effect below is created
  // before onMount runs, so restoring in onMount let the effect overwrite the key with the default
  // first (the app always opened on one tab). U12: default landing tab is the Home overview.
  // Validated against navOrder.ids (module-initialized before this runs) — the one tab-id list.
  let active = $state(
    (() => {
      try {
        const saved = localStorage.getItem('cmh-active-tab');
        return saved && navOrder.ids.includes(saved) ? saved : 'home';
      } catch {
        return 'home';
      }
    })()
  );
  let notifOpen = $state(false);
  let pendingUndo = $state<{ snapshot: string; profiles?: string[]; includeCredentials?: boolean } | null>(null);
  let theme = $state<Theme>('dark');
  let backupData = $state<BackupList | null>(null);
  let profilesData = $state<ProfilesStatus | null>(null);
  let profilesConfig = $state<ProfilesConfig | null>(null);
  let launchConfig = $state<LaunchConfigStatus | null>(null);
  let mcpData = $state<McpStatus | null>(null);
  let envsData = $state<EnvInfo[] | null>(null);
  let envsMatrix = $state<SkillRow[] | null>(null);
  let envsLoaded = $state(false);
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
  let pluginSyncData = $state<PluginSyncStatus | null>(null);
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
    appendLog(t('page.log_error', { e: msg }));
    pushToast({ kind: 'error', title: title ?? t('page.toast_generic_error'), detail: msg });
  }

  // Shared spawn-rejection handler for the start* runners: a backend spawn that rejects (busy
  // slot / script-not-found) only logs operationally, so clear the run lock and append the error
  // line in one place. String(e) so the typed t() slot receives a string, not a raw Error.
  function onSpawnErr(e: unknown) {
    running = null;
    appendLog(t('page.log_error', { e: String(e) }));
    // R5: a failed spawn must be visible — the log dock is collapsed by default, so a silent
    // append reads as "the click did nothing". Toast + reveal the console.
    toastErr(String(e));
    consoleReveal++;
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

  // True while a bulk plugin op is in flight. F17: the bulk run now lives in its OWN backend domain
  // (run_plugins_bulk), so it does NOT take the global `running` lock — only plugin-tab buttons are
  // gated (via the derived prop below), leaving backup/forks/etc. usable. The run-done listener still
  // skips plugin-mgr while this is set (the await drives completion, not the event).
  let bulkActive = $state(false);

  // F21: any cancellable work in flight (run / fork runs / bulk plugin) — gates the "Cancel all"
  // button. PTY sessions aren't tracked here (they live in SessionsTab); cancel_all still kills them.
  const anyActivity = $derived(!!running || bulkActive || Object.values(forkRuns).some((r) => r?.running));

  function startRun(id: string, mode: 'check' | 'apply', append = false) {
    if (running) return;
    const comp = components.find((c) => c.id === id);
    running = id;
    lastRunMode = mode;
    const verb = mode === 'apply' ? t('page.verb_apply') : t('page.verb_check');
    const line = t('page.log_component', { name: comp?.name ?? id, verb });
    if (append) appendLog(line);
    else log = [line];
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
  let sessionFolderReq = $state<{ path: string; tool?: import('$lib/ipc').SessionTool; profile?: string } | null>(null);
  function openSessionFor(path: string, tool?: import('$lib/ipc').SessionTool, profile?: string) {
    active = 'sessions';
    sessionFolderReq = { path, tool, profile };
  }

  function onForkAction(action: ForkAction, path?: string, label?: string) {
    if (path && action === 'ff') {
      // Fast-forward is non-destructive (fork-sync backs up refs) — run directly, no confirm.
      startForkRepo(action, path);
    } else if (path) {
      // Per-repo mutation -> confirm, then run CONCURRENTLY (each repo independent).
      askConfirm(
        t('page.confirm_fork_title'),
        t('page.confirm_fork_msg', { label: label ?? action }),
        t('page.confirm_fork_btn'),
        () => startForkRepo(action, path),
        { danger: action === 'delete' || action === 'delete-wip' || action === 'prune' }
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
      appendLog(t('page.log_error', { e }));
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
      t('page.confirm_batchff_msg', { n: names.length }),
      t('page.confirm_batchff_btn'),
      () => startForks('ff'),
      { details: names }
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
          : action === 'delete-snapshot'
            ? t('page.backup_verb_delete')
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
        () => {
          pendingUndo = opts ? { snapshot: opts.timestamp ?? '', profiles: opts.profiles, includeCredentials: opts.includeCredentials } : null;
          startBackup('restore', opts);
        },
        { danger: true, requireText: opts?.timestamp ?? null }
      );
    } else if (action === 'delete-snapshot') {
      askConfirm(
        t('page.confirm_delsnap_title'),
        t('page.confirm_delsnap_msg', { snap: opts?.timestamp ?? '' }),
        t('page.confirm_delsnap_btn'),
        () => startBackup('delete-snapshot', opts),
        { danger: true }
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
        run,
        { danger: true }
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
    const profiles = target ? (Array.isArray(target) ? target : [target]) : null;
    // Gate broad writes (all profiles, or a multi-profile selection) behind a confirm; a single
    // explicit profile chip the user clicked deploys directly. Idempotent either way. (Was: only
    // the all-profiles path confirmed, while a multi-select bulk overwrote silently.)
    if (profiles && profiles.length === 1) {
      startMcp('deploy', profiles);
      return;
    }
    askConfirm(t('page.confirm_mcp_title'), t('page.confirm_mcp_msg'), t('page.confirm_mcp_btn'), () =>
      startMcp('deploy', profiles ?? undefined)
    );
  }

  // Canonical .mcp.json CRUD (native invokes; reload + toast, no run-log stream).
  async function onMcpUpsert(name: string, definition: string) {
    try {
      await mcpUpsertServer(name, definition);
      await reloadMcp();
      pushToast({ kind: 'success', title: t('mcp.savedServer', { name }) });
    } catch (e) {
      toastErr(e);
    }
  }
  function onMcpRemoveServer(name: string) {
    askConfirm(
      t('page.confirm_mcp_remove_title'),
      t('page.confirm_mcp_remove_msg', { name }),
      t('common.delete'),
      async () => {
        try {
          await mcpRemoveServer(name);
          await reloadMcp();
          pushToast({ kind: 'success', title: t('mcp.removedServer', { name }) });
        } catch (e) {
          toastErr(e);
        }
      },
      { danger: true }
    );
  }
  function onMcpRemoveExtra(name: string, profile: string) {
    askConfirm(
      t('page.confirm_mcp_extra_title'),
      t('page.confirm_mcp_extra_msg', { name, profile }),
      t('common.delete'),
      async () => {
        try {
          await mcpRemoveExtra(name, profile);
          await reloadMcp();
          pushToast({ kind: 'success', title: t('mcp.removedExtra', { name, profile }) });
        } catch (e) {
          toastErr(e);
        }
      },
      { danger: true }
    );
  }

  // --- Environments tab (read-only cross-harness overview) ---
  async function reloadEnvs() {
    try {
      envsData = await readEnvironments();
    } catch {
      envsData = null;
    }
  }
  // Lazy-load on first open — cheap native reads, no script spawn.
  $effect(() => {
    if (active === 'envs' && !envsLoaded) {
      envsLoaded = true;
      setLoading('envs', true);
      reloadEnvs().finally(() => setLoading('envs', false));
    }
  });
  // Enable/disable RTK command-rewriting for OpenCode (writes/removes a Windows-safe plugin).
  async function doEnvRtk(enable: boolean) {
    try {
      await runOpencodeRtk(enable ? 'enable' : 'disable');
      pushToast({
        kind: 'success',
        title: enable ? t('environments.rtkEnabledToast') : t('environments.rtkDisabledToast')
      });
      await reloadEnvs();
    } catch (e) {
      pushToast({ kind: 'error', title: t('environments.rtkError'), detail: String(e) });
    }
  }
  function onEnvRtk(id: string, enable: boolean) {
    if (id !== 'opencode') return; // only OpenCode is wired today
    if (enable) {
      doEnvRtk(true);
    } else {
      // Disabling deletes the plugin file — gate it behind a confirm (#7).
      askConfirm(
        t('environments.rtkDisableTitle'),
        t('environments.rtkDisableConfirm'),
        t('environments.rtkDisable'),
        () => doEnvRtk(false),
        { danger: true }
      );
    }
  }

  async function reloadSkillMatrix() {
    try {
      envsMatrix = await readSkillMatrix();
    } catch {
      envsMatrix = null;
    }
  }

  // One shape for every harness fan-out button: run, toast the count, refresh the cards.
  async function deployToHarness(run: () => Promise<number>, doneKey: string, errKey: string) {
    try {
      const n = await run();
      pushToast({ kind: 'success', title: t(doneKey, { n }) });
      await reloadEnvs();
    } catch (e) {
      pushToast({ kind: 'error', title: t(errKey), detail: String(e) });
    }
  }
  const onDeployMcp = (id: string) => {
    if (id === 'opencode')
      void deployToHarness(runOpencodeMcp, 'environments.deployMcpDone', 'environments.deployMcpError');
    else if (id === 'codex')
      void deployToHarness(runCodexMcp, 'environments.deployMcpDoneCodex', 'environments.deployMcpErrorCodex');
  };
  const onDeployProviders = (id: string) => {
    if (id === 'opencode')
      void deployToHarness(runOpencodeProviders, 'environments.deployProvidersDone', 'environments.deployProvidersError');
    else if (id === 'codex')
      // Not deployToHarness: the result is "was the key mirrored", which picks the toast text.
      void (async () => {
        try {
          const keySet = await runCodexProviders();
          pushToast({
            kind: keySet ? 'success' : 'warn',
            title: t(keySet ? 'environments.connectGatewayDone' : 'environments.connectGatewayDoneNoKey')
          });
          await reloadEnvs();
        } catch (e) {
          pushToast({ kind: 'error', title: t('environments.connectGatewayError'), detail: String(e) });
        }
      })();
  };
  const onDeployInstructions = (id: string) => {
    if (id === 'opencode')
      void deployToHarness(runOpencodeInstructions, 'environments.deployInstrDone', 'environments.deployInstrError');
  };

  // Share skills into ~/.agents/skills (additive junctions) so OpenCode + Codex see them all.
  function onShareSkills() {
    askConfirm(
      t('environments.shareConfirmTitle'),
      t('environments.shareConfirmMsg'),
      t('environments.shareConfirmBtn'),
      async () => {
        try {
          const r = await shareSkills();
          pushToast({
            kind: r.failed ? 'error' : 'success',
            title: t('environments.shareDone', { created: r.created, skipped: r.skipped, failed: r.failed }),
            detail: r.failed && r.details.length ? r.details.join('\n') : undefined
          });
          await reloadEnvs();
          if (envsMatrix !== null) await reloadSkillMatrix();
        } catch (e) {
          pushToast({ kind: 'error', title: t('environments.shareError'), detail: String(e) });
        }
      }
    );
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
  // F23: Home now shows stack + live-session chips, so refresh those too.
  let homeSessionCount = $state<number | null>(null);
  async function reloadHome() {
    await Promise.all([
      reloadProfiles(),
      reloadConfigDrift(),
      reloadSync(),
      reloadSchedules(),
      reloadStack(),
      globalSessionCount()
        .then((n) => (homeSessionCount = n))
        .catch(() => (homeSessionCount = null))
    ]);
  }

  // F23: Home quick actions / per-chip actions → the parent's existing handlers.
  function onHomeAction(id: string) {
    switch (id) {
      case 'check-all':
        active = 'updates';
        startRun('all', 'check');
        break;
      case 'refresh-forks':
        onForkAction('check');
        break;
      case 'start-stack':
        onStack('start');
        break;
      case 'stop-stack':
        onStack('stop');
        break;
      case 'relink':
        onSyncDrift('relink');
        break;
      case 'clean-conflicts':
        onProfileAction('clean-conflicts');
        break;
      case 'repair-profiles': {
        // F23: repair every broken profile's links in one run (backend loops the repair script).
        if (running) break;
        const broken = (profilesData?.profiles ?? []).filter((p) => !p.linksIntact).map((p) => p.name);
        if (!broken.length) break;
        running = 'profiles';
        log = [t('page.prof_log', { verb: t('page.prof_verb_repair', { name: t('page.home_repairAll') }) })];
        repairAllProfiles(broken).catch(onSpawnErr);
        break;
      }
    }
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
    // Enabling/keeping sync items is additive and safe — only gate behind a confirm when something
    // is actually being DISABLED (the destructive direction).
    if (!off.length) {
      startSync('set', enabled);
      return;
    }
    askConfirm(t('page.confirm_sync_title'), t('page.sync_apply_off', { off: off.join(', ') }),
      t('page.confirm_sync_btn'), () => startSync('set', enabled));
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

  // Used by ProfilesTab's per-profile "reset provider" menu item (ProfilesTab.svelte:281).
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
    openUrl(url).catch(toastErr);
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
      .then(() => appendLog(t('myProviders.loginSaved')))
      .catch(toastErr);
  }
  // C2: clear one stored freellmapi credential. UI fires after a confirm dialog in ProvidersTab.
  function onDeleteFreellmapiAuth(key: 'email' | 'password' | 'token') {
    deleteFreellmapiAuth(key)
      .then(() => {
        appendLog(t('myProviders.authDeleted', { key }));
        reloadProviders();
      })
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
  // list_plugin_updates spawns the claude CLI — throttle the on-focus refresh so alt-tabbing
  // doesn't fire a spawn every time the window regains focus (explicit calls stay unthrottled).
  let lastFocusPluginCheck = 0;

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
    try {
      pluginSyncData = await pluginSyncStatus();
    } catch {
      pluginSyncData = null;
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
    extensions: pluginsAttention(pluginUpdates.length),
    sessions: sessionsAttention(agentSummary)
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
  // Forks status is cached on disk and loaded on mount; we do NOT auto-run a check on tab open —
  // that sweep is slow (sequential git fetch + gh per repo) and holds the single global run lock,
  // which would block unrelated backup/restore. The user refreshes explicitly via the Check button.

  // Native gh call (not run-locked) — load the full GitHub repo list once on first open.
  $effect(() => {
    if (active === 'forks' && !githubLoaded) {
      githubLoaded = true;
      listGithubRepos()
        .then((r) => (githubRepos = r))
        .catch(() => (githubRepos = []));
    }
  });

  // F10: clone a GitHub-only repo locally. Pick a PARENT folder, clone into <parent>/<name>, then
  // rescan forks so the fresh clone shows up in the local list (verify step).
  let cloningRepo = $state<string | null>(null);
  async function onCloneRepo(repo: GithubRepo) {
    if (cloningRepo) return;
    const parent = await pickFolder().catch(() => null);
    if (!parent) return;
    const target = `${parent.replace(/[\\/]+$/, '')}\\${repo.name}`;
    cloningRepo = repo.nameWithOwner;
    try {
      await cloneRepo(repo.url, target);
      pushToast({ kind: 'success', title: t('page.clone_done', { name: repo.name }), detail: target });
      if (!running) startForks('check'); // rescan so the new clone appears in the local repo list
    } catch (e) {
      pushToast({ kind: 'error', title: t('page.clone_failed', { name: repo.name }), detail: String(e) });
    } finally {
      cloningRepo = null;
    }
  }

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
    // 'disable' is reversible (re-enable any time) → no confirm, matching bulk-disable. Only the
    // irreversible 'remove' gates behind a danger confirm.
    if (action === 'remove') {
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

  // F17: one backend call runs the whole bulk in its own domain (sequential there — no config race),
  // streaming id-tagged lines to the run log. We don't set the global `running` lock, so unrelated
  // work stays available; only the plugin tab is gated via `bulkActive`.
  async function runBulkPlugins(action: PluginAction, ids: string[]) {
    if (!ids.length || bulkActive) return;
    const verb =
      action === 'update'
        ? t('page.plugin_verb_update')
        : action === 'enable'
          ? t('page.plugin_verb_enable')
          : action === 'remove'
            ? t('page.plugin_verb_remove')
            : t('page.plugin_verb_disable');
    bulkActive = true;
    log = [t('page.plugin_bulk_log', { n: ids.length, verb })];
    try {
      await runPluginsBulk(action, ids);
    } catch (e) {
      appendLog(t('page.log_error', { e: String(e) }));
    } finally {
      bulkActive = false;
      reloadExtensions();
    }
  }
  // Cross-profile plugin sync: one-off reconcile (streams into the console, run-done
  // releases the lock + toasts via the generic operational path).
  function onPluginSyncNow() {
    if (running) return;
    running = 'pluginsync';
    log = [t('page.log_component', { name: opName('pluginsync'), verb: t('page.verb_apply') })];
    runPluginSync().catch(onSpawnErr);
  }
  // Wire/unwire the SessionStart auto-sync hook in every profile (quick file edits, no run lock).
  async function onPluginSyncHookToggle(enabled: boolean) {
    try {
      pluginSyncData = await pluginSyncSet(enabled);
      pushToast({ kind: 'success', title: t(enabled ? 'page.pluginsync_hook_on' : 'page.pluginsync_hook_off') });
    } catch (e) {
      toastErr(e);
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
        { danger: true, details: ids }
      );
    } else {
      runBulkPlugins(action, ids);
    }
  }

  function onOpenSkills() {
    canonicalSkillsDir().then(d => openPath(d)).catch(toastErr);
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
      appendLog(t('page.log_warn', { e: String(e) }));
    }
  }

  // F21: kill everything — the active run, every fork run, every PTY session, the bulk plugin sweep.
  // The backend emits 'cancel-all-done'; the listener resets FE state + toasts.
  async function cancelAllNow() {
    try {
      await cancelAll();
    } catch (e) {
      appendLog(t('page.log_warn', { e: String(e) }));
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

  // Sessions tab is lazy: mount it (and pull the xterm chunk) only once the user first opens it,
  // then keep it mounted so running terminals survive tab switches.
  let sessionsEverOpened = $state(false);
  $effect(() => {
    if (active === 'sessions') sessionsEverOpened = true;
  });

  // Command palette (Ctrl+K): jump to any tab + a few quick actions.
  let paletteOpen = $state(false);
  let hotkeyHelpOpen = $state(false);
  // Phase 4.2 — after a palette navigation, briefly scroll to + highlight a specific item in the target tab.
  let highlightTarget = $state<{ tab: string; id: string } | null>(null);
  // Consume highlightTarget after it has been applied (re-render cycle + transition completes).
  $effect(() => {
    const tgt = highlightTarget;
    if (!tgt) return;
    // The target tab's DOM may not be mounted yet if it just switched — wait for the next frame.
    requestAnimationFrame(() => {
      const el = document.querySelector(`[data-highlight-id="${CSS.escape(tgt.id)}"]`);
      if (el) {
        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
        el.classList.add('highlight-flash');
        setTimeout(() => el.classList.remove('highlight-flash'), 2000);
      }
      highlightTarget = null;
    });
  });
  // Item 14: palette run verbs self-guard on the run lock and silently no-op while busy — a dead
  // Enter with no feedback. Wrap them so a busy run instead surfaces an info toast naming what's
  // running. The underlying start* guards stay (defense in depth); UI toggles/stops aren't wrapped.
  function runOrToast(fn: () => void) {
    if (running) {
      pushToast({ kind: 'info', title: t('page.busy_running', { name: opName(running) }) });
      return;
    }
    fn();
  }
  const paletteCommands = $derived([
    // Mirror the Ctrl+1..9 jumps (first 9 tabs) as visible hints so the shortcuts are discoverable.
    // U1: both follow the SIDEBAR's live order (navOrder), so the numbers match what's on screen.
    ...navOrder.ids.map((id, i) => ({
      id: `tab:${id}`,
      label: t(`nav.${id}`),
      hint: i < 9 ? `Ctrl+${i + 1}` : undefined,
      run: () => (active = id)
    })),
    {
      id: 'act:density',
      label: `${t('settings.density')}: ${density === 'compact' ? t('settings.densityComfortable') : t('settings.densityCompact')}`,
      run: () => setDensity(density === 'compact' ? 'comfortable' : 'compact')
    },
    {
      id: 'act:theme',
      label: `${t('settings.theme')}: ${theme === 'dark' ? t('settings.themeLight') : t('settings.themeDark')}`,
      run: () => setTheme(theme === 'dark' ? 'light' : 'dark')
    },
    // High-frequency verbs so the daily loop is Ctrl+K → type → Enter (each handler self-guards on
    // the run lock, so a no-op while busy is harmless).
    { id: 'act:checkall', label: t('page.cmd_check_all'), run: () => runOrToast(() => startRun('all', 'check')) },
    { id: 'act:forks', label: t('page.cmd_refresh_forks'), run: () => runOrToast(() => startForks('check')) },
    { id: 'act:backup', label: t('page.cmd_backup_now'), run: () => runOrToast(() => startBackup('backup')) },
    { id: 'act:stack_start', label: t('page.cmd_stack_start'), run: () => runOrToast(() => onStack('start')) },
    { id: 'act:stack_stop', label: t('page.cmd_stack_stop'), run: () => runOrToast(() => onStack('stop')) },
    { id: 'act:log', label: t('page.cmd_open_log'), run: () => consoleReveal++ },
    // U9: the palette could start runs but not stop them; new-session was likewise unreachable.
    { id: 'act:cancel_all', label: t('page.cmd_cancel_all'), run: () => cancelAllNow() },
    { id: 'act:new_session', label: t('page.cmd_new_session'), run: () => (active = 'sessions') },
    // U5: the cheatsheet is reachable from the palette too, not only via the «?» key.
    { id: 'act:hotkeys', label: t('page.hkTitle'), run: () => (hotkeyHelpOpen = true) },
    // Per-component check/apply so "check rtk" / "apply plugins" are one Ctrl+K away, not a tab hunt.
    ...components.flatMap((c) => {
      const verbs = [
        { id: `check:${c.id}`, label: `${t('common.check')}: ${c.name}`, run: () => runOrToast(() => startRun(c.id, 'check')) }
      ];
      if (c.supportsApply) {
        verbs.push({ id: `apply:${c.id}`, label: `${t('common.apply')}: ${c.name}`, run: () => runOrToast(() => startRun(c.id, 'apply')) });
      }
      return verbs;
    }),
    // Phase 4.2 — data-driven deep links (profiles, MCP servers, repos, plugins, settings).
    // Each result switches to the corresponding tab and briefly highlights the item.
    ...(profilesData?.profiles ?? []).map((p) => ({
      id: `pf:${p.name}`,
      label: `${t('nav.profiles')} · ${p.name}`,
      run: () => { active = 'profiles'; highlightTarget = { tab: 'profiles', id: `profile:${p.name}` }; }
    })),
    ...(mcpData?.source ?? []).map((s) => ({
      id: `mcp:${s.name}`,
      label: `${t('nav.mcp')} · ${s.name}`,
      run: () => { active = 'mcp'; highlightTarget = { tab: 'mcp', id: `mcp:${s.name}` }; }
    })),
    ...(pluginsData ?? []).map((pl) => ({
      id: `pl:${pl.id}`,
      label: `${t('nav.extensions')} · ${pl.id}`,
      run: () => { active = 'extensions'; highlightTarget = { tab: 'extensions', id: `plugin:${pl.id}` }; }
    })),
    ...githubRepos.map((r) => ({
      id: `gh:${r.nameWithOwner}`,
      label: `${t('nav.forks')} · ${r.nameWithOwner}`,
      run: () => { active = 'forks'; highlightTarget = { tab: 'forks', id: `repo:${r.nameWithOwner}` }; }
    })),
    // Settings section deep links (statics).
    { id: 'set:view', label: `${t('nav.settings')} · ${t('settings.view')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:view' }; } },
    { id: 'set:theme', label: `${t('nav.settings')} · ${t('settings.theme')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:theme' }; } },
    { id: 'set:lang', label: `${t('nav.settings')} · ${t('settings.language')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:language' }; } },
    { id: 'set:root', label: `${t('nav.settings')} · ${t('settings.scriptsRoot')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:root' }; } },
    { id: 'set:launch', label: `${t('nav.settings')} · ${t('settings.launch')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:launch' }; } },
    { id: 'set:timeouts', label: `${t('nav.settings')} · ${t('settings.timeouts')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:timeouts' }; } },
    { id: 'set:backup', label: `${t('nav.settings')} · ${t('settings.backupSection')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:backup' }; } },
    { id: 'set:about', label: `${t('nav.settings')} · ${t('settings.about')}`, run: () => { active = 'settings'; highlightTarget = { tab: 'settings', id: 'settings:about' }; } }
  ]);
  function onGlobalKey(e: KeyboardEvent) {
    if (e.ctrlKey && (e.key === 'k' || e.key === 'K')) {
      e.preventDefault();
      paletteOpen = !paletteOpen;
      notifOpen = false; // U2: don't leave the notification popover hanging under the palette
      return;
    }
    const tgt = e.target as HTMLElement | null;
    const typing = !!tgt && (tgt.tagName === 'INPUT' || tgt.tagName === 'TEXTAREA' || tgt.isContentEditable);
    // F21: Ctrl+Shift+Backspace = Cancel all. (The plan named Ctrl+Shift+Esc, but Windows reserves
    // that combo for Task Manager — the webview never receives it; the tray entry covers discovery.)
    if (e.ctrlKey && e.shiftKey && e.key === 'Backspace') {
      e.preventDefault();
      cancelAllNow();
      return;
    }
    // Ctrl+1..9 jumps straight to the Nth VISIBLE tab (U1: follows the sidebar's live order).
    if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key >= '1' && e.key <= '9') {
      const idx = Number(e.key) - 1;
      if (idx < navOrder.ids.length) {
        e.preventDefault();
        active = navOrder.ids[idx];
      }
      return;
    }
    // Esc cancels a running operation — but only when no dialog/palette is open (they own their Esc)
    // and the user isn't typing, so it never steals a field's or modal's own Escape.
    if (e.key === 'Escape' && running && !typing && !paletteOpen && !hotkeyHelpOpen && !confirm.open && !notifOpen) {
      e.preventDefault();
      cancel();
      return;
    }
    // "?" opens the keyboard-shortcut cheatsheet — but not while typing in a field.
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
      // (Last-open tab is restored in the `active` state initializer — see R2 note there.)
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
        // Backend coalesces rapid lines into one event joined by '\n' (item 7): split back so each
        // gets its own row and the per-line '⚠ ' err prefix, preserving FIFO order.
        const prefix = p.stream === 'err' ? '⚠ ' : '';
        for (const ln of p.line.split('\n')) appendLog(prefix + ln);
      })
    );
    unlisten.push(
      await listen<{ component: string; code: number }>('run-done', async (e) => {
        appendLog(t('page.log_done', { code: e.payload.code }));
        // During a bulk plugin op, runBulkPlugins awaits the single backend call and does the reload
        // itself afterward — skip this handler's per-item lifecycle for the bulk's run-done.
        if (e.payload.component === 'plugin-mgr' && bulkActive) return;
        // Only release the lock if THIS event owns it. F17: a bulk run (own domain, never sets
        // `running`) can emit run-done while an unrelated op holds the lock — don't unlock that op.
        const id = e.payload.component;
        if (running === id) running = null;
        const code = e.payload.code;
        const wasApply = lastRunMode === 'apply';
        lastRunMode = 'check';
        const forkAct = lastForkAction;
        lastForkAction = null;
        const c = components.find((x) => x.id === id);
        if (c) {
          await loadStatus(c);
          const s = statuses[id];
          const dur = s?.durationSec;
          if (typeof dur === 'number') {
            pushRun({ component: id, durationSec: dur, timestamp: Date.now(), status: s?.status ?? '' });
          }
        }
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
          appendLog(t('page.forks_recheck'));
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
            if (id === 'backup' && pendingUndo) {
              const undo = pendingUndo;
              pendingUndo = null;
              if (code === 0) {
                pushToast({ kind: 'success', title: t('page.toast_op_done', { name }) });
                pushToast({
                  kind: 'info',
                  title: t('page.backup_restore_undo_title'),
                  detail: t('page.backup_restore_undo_detail', { snap: undo.snapshot }),
                  action: {
                    label: t('page.backup_restore_undo_btn'),
                    onClick: () => startBackup('restore', { timestamp: undo.snapshot, profiles: undo.profiles, includeCredentials: undo.includeCredentials })
                  }
                }, 15000);
              } else {
                pushToast({ kind: 'error', title: t('page.toast_op_error', { name, code }), detail: t('page.toast_op_error_detail'), action: { label: t('page.toast_open_log'), onClick: () => consoleReveal++ } });
              }
            } else if (code === 0) {
              // R1: exit-0 is not proof — if this operational run wrote a FRESH status envelope,
              // trust it over the process code (a script can exit 0 with status:error inside).
              let env: { status?: string; timestamp?: string; summary?: string; counts?: { failed?: number } } | null = null;
              try {
                env = await readStatus(`${id}.last.json`);
              } catch {
                /* no envelope for this op — judge by exit code */
              }
              const fresh = env?.timestamp ? Date.now() - Date.parse(env.timestamp) < 15 * 60_000 : false;
              if (env && fresh && (env.status === 'error' || (env.counts?.failed ?? 0) > 0)) {
                pushToast({
                  kind: 'error',
                  title: t('page.toast_op_env_error', { name }),
                  detail: env.summary ?? t('page.toast_op_error_detail'),
                  action: { label: t('page.toast_open_log'), onClick: () => consoleReveal++ }
                });
              } else {
                pushToast({ kind: 'success', title: t('page.toast_op_done', { name }) });
              }
            } else {
              pushToast({ kind: 'error', title: t('page.toast_op_error', { name, code }), detail: t('page.toast_op_error_detail'), action: { label: t('page.toast_open_log'), onClick: () => consoleReveal++ } });
            }
          }
        } catch {
          /* surfacing the outcome must never break the run lifecycle */
        }
      })
    );
    unlisten.push(
      // F21: backend killed everything — reset the FE locks/maps and confirm.
      await listen('cancel-all-done', () => {
        running = null;
        forkRuns = {};
        bulkActive = false;
        appendLog(t('page.cancel_all_done'));
        pushToast({ kind: 'info', title: t('page.cancel_all_done') });
      })
    );
    // Per-repo concurrent fork runs (component = repo path).
    unlisten.push(
      await listen<{ component: string; stream: string; line: string }>('fork-log', (e) => {
        const p = e.payload;
        const name = p.component.split(/[\\/]/).pop() || p.component;
        // Same coalesced-event split as run-log: one row per line, repo tag + err prefix per line.
        const prefix = `[${name}] ${p.stream === 'err' ? '⚠ ' : ''}`;
        for (const ln of p.line.split('\n')) appendLog(prefix + ln);
        const prev = forkRuns[p.component];
        forkRuns = { ...forkRuns, [p.component]: { line: p.line, running: true, code: prev?.code ?? null } };
      })
    );
    unlisten.push(
      await listen<{ component: string; code: number }>('fork-done', async (e) => {
        const path = e.payload.component;
        const code = e.payload.code;
        const name = path.split(/[\\/]/).pop() || path;
        appendLog(`[${name}] ${t('page.log_done', { code })}`);
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
        // F20: don't yank the user off their current tab (or out of an input mid-type). Only jump to
        // Updates when they're already there AND not typing; otherwise run in the background + toast.
        const el = document.activeElement;
        const typing = !!el && /^(INPUT|TEXTAREA)$/.test(el.tagName);
        if (active !== 'updates' || typing) {
          startRun('all', 'check');
          pushToast({ kind: 'info', title: t('page.bgCheckStarted') });
          return;
        }
        // DIAGNOSTIC: this is the ONLY code path that force-switches to Updates. If the tab jumps
        // without you clicking the tray's "Проверить всё", this line will show in the log — proving
        // a spurious tray event is the source.
        appendLog(`[diag] tray-check-all @ ${new Date().toLocaleTimeString()}`);
        active = 'updates';
        startRun('all', 'check');
      })
    );
    unlisten.push(
      // F19: tray Quit no longer hard-exits — it asks here first, surfacing how many live sessions die.
      await listen('tray-quit-request', async () => {
        const n = await globalSessionCount().catch(() => 0);
        askConfirm(
          t('page.quitTitle'),
          n > 0 ? t('page.quitMsgSessions', { n }) : t('page.quitMsg'),
          t('page.quitBtn'),
          () => quitApp(),
          { danger: true }
        );
      })
    );
    // F18: extended tray entries → the same handlers the in-app buttons use.
    unlisten.push(await listen('tray-refresh-forks', () => onForkAction('check')));
    unlisten.push(await listen('tray-refresh-providers', () => reloadProviders()));
    unlisten.push(await listen('tray-stack-start', () => onStack('start')));
    unlisten.push(await listen('tray-stack-stop', () => onStack('stop')));
    unlisten.push(await listen('tray-cancel-all', () => cancelAllNow()));
    unlisten.push(
      await listen<string>('tray-open-tab', (e) => {
        if (e.payload) active = e.payload;
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
      // pluginUpdates also feeds the sidebar "extensions" badge, so keep it fresh app-wide — but
      // at most once every 5 min on focus (the CLI spawn isn't worth firing on every alt-tab).
      const now = Date.now();
      if (now - lastFocusPluginCheck > 5 * 60_000) {
        lastFocusPluginCheck = now;
        reloadPluginUpdates();
      }
    };
    window.addEventListener('focus', onFocus);
    unlisten.push(() => window.removeEventListener('focus', onFocus));
  });

  onDestroy(() => unlisten.forEach((u) => u()));

</script>

<svelte:window onkeydown={onGlobalKey} />
<CommandPalette open={paletteOpen} commands={paletteCommands} placeholder={t('common.paletteSearch')} onClose={() => (paletteOpen = false)} />

<div class="flex h-full overflow-hidden" class:dense={density === 'compact'}>
  <Sidebar {active} onSelect={(id) => (active = id)} {attention} loading={tabLoading}
    notifOpen={notifOpen} onToggleNotif={() => (notifOpen = !notifOpen)} />

  <NotificationPanel open={notifOpen} onClose={() => (notifOpen = false)} />

  <div class="flex min-w-0 flex-1 flex-col">
    <main class="relative min-h-0 flex-1 overflow-auto">
      <div class="relative mx-auto w-full {fullWidth ? '' : 'max-w-[1600px]'}">
      {#if loadError}
        <div class="m-sw-6 sw-card status-bad">{t('page.load_error', { e: loadError })}</div>
      {/if}

      {#if tabRefreshing}
        <div
          class="pointer-events-none absolute left-1/2 top-sw-4 z-10 flex -translate-x-1/2 items-center gap-sw-2 rounded-full border border-sw-border bg-sw-bg-secondary px-sw-3 py-sw-1 text-sw-sm text-sw-text-secondary shadow"
        >
          <Spinner size={16} />
          {t('common.refreshing')}
        </div>
      {/if}

      {#key active}
      <div
        class="transition-opacity duration-200 animate-[sw-fade-in_0.2s_ease-out]"
        class:opacity-40={blockingRefresh}
        class:pointer-events-none={blockingRefresh}
      >
      {#if active === 'home'}
        <HomeTab profiles={profilesData} sync={syncData} drift={driftData} schedules={schedulesData}
          stack={stackData} sessionCount={homeSessionCount} busy={!!running} {components} {statuses}
          onOpen={(id) => (active = id)} onRefresh={reloadHome} onAction={onHomeAction} />
      {:else if active === 'updates'}
        <UpdatesTab {components} {statuses} {running} {onCheck} {onApply} onOpenTab={(id) => (active = id)} />
      {:else if active === 'forks'}
        <ForksTab status={statuses.forks} {githubRepos} {running} {forkRuns} onAction={onForkAction} {onCancelFork} onCancelCheck={cancel} {onBatchFf} {onOpenUrl} onOpenSession={openSessionFor} onClone={onCloneRepo} {cloningRepo} profiles={(profilesData?.profiles ?? []).map((p) => p.name)} />
      {:else if active === 'backup'}
        <BackupTab data={backupData} {running} {log} {confirmDestructive} profiles={(profilesData?.profiles ?? []).map((p) => p.name)} onAction={onBackupAction} onRefresh={reloadBackup} />
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
          myProviders={myProvidersData}
          {onRepairElevated}
          {onRelaunchAdmin}
        />
      {:else if active === 'mcp'}
        <McpTab data={mcpData} {running} onRefresh={reloadMcp} onDeploy={onMcpDeploy}
          onUpsert={onMcpUpsert} onRemoveServer={onMcpRemoveServer} onRemoveExtra={onMcpRemoveExtra} />
      {:else if active === 'envs'}
        <EnvironmentsTab data={envsData} {running} matrix={envsMatrix} onRefresh={reloadEnvs}
          onShare={onShareSkills} onRtk={onEnvRtk} onLoadMatrix={reloadSkillMatrix}
          onOpenConfig={(p) => openPath(p).catch(toastErr)} onOpenProviders={() => (active = 'providers')}
          onOpenMcp={() => (active = 'mcp')} onDeployMcp={onDeployMcp}
          onDeployProviders={onDeployProviders} onDeployInstructions={onDeployInstructions}
          onOpenUrl={(u) => openUrl(u).catch(toastErr)} />
      {:else if active === 'sync'}
        <SyncTab data={syncData} {running} onRefresh={onSyncRefresh} onApply={onSyncApply}
          driftData={driftData} conflictCount={profilesData?.syncConflicts?.count ?? 0}
          onDriftApply={onSyncDrift} onCleanConflicts={() => onProfileAction('clean-conflicts')} />
      {:else if active === 'providers'}
        <ProvidersTab
          engines={enginesData}
          providers={providersData}
          {confirmDestructive}
          stack={stackData}
          {running}
          onEngine={onEngineAction}
          onStack={onStack}
          onProviderSet={onProviderSet}
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
          {onDeleteFreellmapiAuth}
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
          running={bulkActive ? 'plugin-mgr' : running}
          syncStatus={pluginSyncData}
          onAction={onPluginAction}
          {onBulkPlugin}
          onRefresh={reloadExtensions}
          {onOpenSkills}
          {onOpenSkill}
          {onDeleteSkill}
          onSyncNow={onPluginSyncNow}
          onSyncHookToggle={onPluginSyncHookToggle}
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
      {/key}
      </div>

      <!-- Sessions tab is full-bleed and, once first opened, stays MOUNTED (display-toggled) so
           running terminals survive switching to another tab. The heavy xterm chunk is dynamically
           imported on first open (sessionsEverOpened) instead of shipping in the startup bundle. -->
      {#if sessionsEverOpened}
        <div class="absolute inset-0 {active === 'sessions' ? '' : 'hidden'}">
          <!-- R6: pending → spinner (the xterm chunk takes a beat on first open); catch → visible
               error instead of a forever-blank tab (stale asset hash after an in-place update). -->
          {#await import('$lib/components/SessionsTab.svelte')}
            <div class="grid h-full place-items-center text-sw-text-muted"><Spinner size={22} /></div>
          {:then { default: SessionsTab }}
            <SessionsTab visible={active === 'sessions'} {confirmDestructive} profiles={(profilesData?.profiles ?? []).map((p) => p.name)} folderReq={sessionFolderReq} onFolderReqConsumed={() => (sessionFolderReq = null)} />
          {:catch e}
            <div class="m-sw-6 sw-card status-bad">{t('page.load_error', { e: String(e) })}</div>
          {/await}
        </div>
      {/if}
    </main>

    <Console {log} {running} busy={anyActivity} revealSignal={consoleReveal} onClear={() => (log = [])} onCancel={cancel} onCancelAll={cancelAllNow} />
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
