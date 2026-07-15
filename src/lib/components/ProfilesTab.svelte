<script lang="ts">
  import type {
    ProfilesStatus,
    ProfileAction,
    ProfilesConfig,
    ProfileMgmtArgs,
    LaunchConfigStatus,
    ProfileLaunch,
    ProfileProvider,
    EngineStatus,
    ProviderArgs,
    MyProvider,
    OrphanInfo
  } from '$lib/ipc';
  import { pProfile, t } from '$lib/i18n';
  import { readProfileFile } from '$lib/ipc';
  import { Users, Play, FolderOpen } from '@lucide/svelte';
  import EmptyState from './EmptyState.svelte';
  import { copyText } from '$lib/clipboard';
  import { redactSecrets } from '$lib/redact';
  import { relTime } from '$lib/relativeTime';
  import { urlHost } from '$lib/url';
  import { profileDotColor } from '$lib/statusColor';
  import { profileHasMissingLink } from '$lib/attention';
  import ProfileEditDialog from './ProfileEditDialog.svelte';
  import LaunchConfigDialog from './LaunchConfigDialog.svelte';
  import ProviderEditDialog from './ProviderEditDialog.svelte';
  import ModalShell from './ModalShell.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import ProfileUsageBadge from './ProfileUsageBadge.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';
  import MatrixRowEditor from './MatrixRowEditor.svelte';
  import MatrixControls from './MatrixControls.svelte';
  import Segmented from './Segmented.svelte';
  import { MatrixState } from '$lib/matrixState.svelte';
  import type { MatrixApply } from '$lib/ipc';

  let {
    data,
    config,
    launchConfig,
    providers,
    engines,
    running,
    onAction,
    onMgmt,
    orphans = [],
    onDeleteOrphan,
    onOpen,
    onLaunch,
    onSaveLaunch,
    onMeasure,
    onProviderSet,
    onProviderClear,
    myProviders = null,
    onRepairElevated,
    onRelaunchAdmin,
    onApplyMatrix,
    onMcpDeployProfile,
    onMcpRemoveExtra,
    mcpTick = 0
  }: {
    data: ProfilesStatus | null;
    config: ProfilesConfig | null;
    launchConfig: LaunchConfigStatus | null;
    providers: ProfileProvider[] | null;
    engines: EngineStatus[] | null;
    running: string | null;
    onAction: (action: ProfileAction, name?: string) => void;
    onMgmt: (args: ProfileMgmtArgs) => void;
    orphans?: OrphanInfo[];
    onDeleteOrphan: (name: string) => void;
    onOpen: (name: string) => void;
    onLaunch: (name: string, mode: 'terminal' | 'vscode') => void;
    onSaveLaunch: (name: string, mode: 'full' | 'lean', mcp: string[], claudeMd: boolean) => Promise<void>;
    onMeasure: (name: string, lean: boolean) => Promise<number>;
    onProviderSet: (args: ProviderArgs) => void;
    onProviderClear: (name: string) => void;
    myProviders?: MyProvider[] | null;
    onRepairElevated: (name: string) => void;
    onRelaunchAdmin: () => void;
    onApplyMatrix: (changes: MatrixApply) => Promise<{ skipped: string[] }>;
    onMcpDeployProfile: (profile: string) => void;
    onMcpRemoveExtra: (server: string, profile: string) => void;
    mcpTick?: number;
  } = $props();

  const busy = $derived(!!running);
  const profiles = $derived(data?.profiles ?? []);
  const conflicts = $derived(data?.syncConflicts);
  const isAdmin = $derived(data?.isAdmin ?? false);

  // B3: per-profile config (provider/proxy/folders/plugins/mcp) is edited inside each row's expand
  // (MatrixRowEditor) with one shared controls bar (MatrixControls). A single MatrixState owns the
  // accumulate → preview → apply model (moved verbatim from the old standalone ProfilesMatrix).
  const mtx = new MatrixState();
  $effect(() => {
    mtx.engines = engines;
    mtx.myProviders = myProviders;
    mtx.running = running;
    mtx.onApplyMatrix = onApplyMatrix;
    mtx.onMcpDeployProfile = onMcpDeployProfile;
    mtx.onMcpRemoveExtra = onMcpRemoveExtra;
  });
  // Load once when the tab first renders.
  $effect(() => {
    if (!mtx.loaded) mtx.load();
  });
  // An MCP deploy/remove elsewhere bumps mcpTick → re-read mcp facts without dropping the draft.
  let lastMcpTick: number | null = null;
  $effect(() => {
    const tick = mcpTick;
    if (!mtx.loaded) return;
    if (lastMcpTick === null) {
      lastMcpTick = tick;
      return;
    }
    if (tick === lastMcpTick) return;
    lastMcpTick = tick;
    mtx.load(false);
  });

  // Folder symlinks need admin. When elevated, repair inline (streamed); otherwise offer the
  // elevate dialog (one-off UAC repair or relaunch the whole app as admin).
  let elevOpen = $state(false);
  let elevProfile = $state('');
  function finishProfile(name: string) {
    if (isAdmin) {
      onAction('repair', name);
    } else {
      elevProfile = name;
      elevOpen = true;
    }
  }


  // Per-profile launch config (full vs lean) and provider.
  const launchByName = $derived(new Map((launchConfig?.profiles ?? []).map((p) => [p.name, p])));
  const providerByName = $derived(new Map((providers ?? []).map((p) => [p.name, p])));
  // Precompute labels once: the provider-column sort comparator ran engines.find + new URL() per
  // comparison (O(E)+parse on every compare); this turns each into an O(1) lookup.
  const providerLabelByName = $derived.by(() => {
    const m = new Map<string, string>();
    for (const p of providers ?? []) m.set(p.name, providerLabel(p.name));
    return m;
  });
  function providerLabel(name: string): string {
    const p = providerByName.get(name);
    if (!p || !p.baseUrl) return t('profiles.providerDefault');
    const eng = (engines ?? []).find((e) => e.baseUrl === p.baseUrl);
    if (eng) return eng.name;
    return urlHost(p.baseUrl);
  }

  // Lifecycle dialog state.
  let dlgOpen = $state(false);
  let dlgMode = $state<'add' | 'rename' | 'recolor' | 'redescribe'>('add');
  let dlgCurrent = $state('');
  let dlgColor = $state('White');
  let dlgDescription = $state('');
  function openDlg(mode: 'add' | 'rename' | 'recolor' | 'redescribe', name = '', color = 'White', description = '') {
    dlgMode = mode;
    dlgCurrent = name;
    dlgColor = color;
    dlgDescription = description;
    dlgOpen = true;
  }
  function onDlgSubmit(v: { name: string; color: string; description: string }) {
    dlgOpen = false;
    if (dlgMode === 'add') onMgmt({ action: 'add', name: v.name, color: v.color, description: v.description });
    else if (dlgMode === 'rename') onMgmt({ action: 'rename', name: dlgCurrent, newName: v.name });
    else if (dlgMode === 'redescribe') onMgmt({ action: 'redescribe', name: dlgCurrent, description: v.description });
    else onMgmt({ action: 'recolor', name: dlgCurrent, color: v.color });
  }

  // Launch-config (lean tool set) dialog.
  let lcOpen = $state(false);
  let lcProfile = $state<ProfileLaunch | null>(null);
  function openLaunchCfg(name: string) {
    lcProfile = launchByName.get(name) ?? null;
    lcOpen = true;
  }

  // Provider dialog (per-profile LLM provider, reused from Providers tab).
  let pvOpen = $state(false);
  let pvName = $state('');
  let pvCurrent = $state<ProfileProvider | null>(null);
  function editProvider(name: string) {
    pvName = name;
    pvCurrent = providerByName.get(name) ?? null;
    pvOpen = true;
  }
  function onPvSubmit(v: {
    baseUrl: string;
    token: string;
    model: string;
    smallModel: string;
    keepToken: boolean;
  }) {
    pvOpen = false;
    onProviderSet({
      action: 'set',
      name: pvName,
      baseUrl: v.baseUrl,
      token: v.token,
      model: v.model,
      smallModel: v.smallModel,
      keepToken: v.keepToken
    });
  }

  // Read-only config viewer (#80): CLAUDE.md / settings.json of a profile in a modal.
  let viewerOpen = $state(false);
  let viewerName = $state('');
  let viewerWhich = $state<'claude' | 'settings'>('settings');
  let viewerContent = $state('');
  let viewerErr = $state('');
  let viewerLoading = $state(false);
  // L17: mask secret-shaped values before they hit the DOM or the clipboard. settings.json commonly
  // embeds MCP-server env API keys in plaintext; this viewer was the one surface that showed them raw.
  const viewerDisplay = $derived(redactSecrets(viewerContent));
  async function loadViewer() {
    viewerLoading = true;
    viewerErr = '';
    viewerContent = '';
    try {
      viewerContent = await readProfileFile(viewerName, viewerWhich);
    } catch (e) {
      viewerErr = String(e);
    } finally {
      viewerLoading = false;
    }
  }
  function openViewer(name: string) {
    viewerName = name;
    viewerWhich = 'settings';
    viewerOpen = true;
    loadViewer();
  }
  function setWhich(w: 'claude' | 'settings') {
    if (viewerWhich === w) return;
    viewerWhich = w;
    loadViewer();
  }

  // Problems → recommendations. A profile is broken only when a shared folder is MISSING its link
  // (status null); real data ("none") or a present link is fine — so a clean repair clears it and a
  // folder kept as real data no longer nags. Shared with the sidebar badge (profileHasMissingLink)
  // so the card and the badge can never disagree.
  const brokenLinks = $derived(profiles.filter((p) => p.exists && profileHasMissingLink(p)));
  const missing = $derived(profiles.filter((p) => !p.exists));
  const conflictCount = $derived(conflicts?.count ?? 0);
  const hasIssues = $derived(brokenLinks.length > 0 || missing.length > 0 || conflictCount > 0);

  // Profile colour-name -> dot hex (shared source; falls back to neutral slate for unknown names).
  const dot = (c: string) => profileDotColor(c);

  // Overflow menu items for a profile card.
  function menuItems(p: (typeof profiles)[number]) {
    const items: { label: string; title?: string; onClick: () => void; disabled?: boolean; danger?: boolean }[] = [
      {
        label: t('profiles.menuTools'),
        title: t('profiles.menuToolsTip'),
        onClick: () => openLaunchCfg(p.name),
        disabled: !p.exists
      }
    ];
    if (p.exists) {
      items.push({
        label: t('profiles.menuViewConfig'),
        title: t('profiles.menuViewConfigTip'),
        onClick: () => openViewer(p.name)
      });
      items.push({
        label: t('profiles.menuRepair'),
        // Immediate action (kicks off a run) — disable while busy, but say WHY (a mute grey read as
        // "broken"). Dialog-openers below stay enabled: they only open a dialog, the mutation on
        // submit is already guarded by the run lock.
        title: busy ? t('common.busyDisabled') : t('profiles.menuRepairTip'),
        onClick: () => finishProfile(p.name),
        disabled: busy
      });
    }
    // Only offered on the damaged shape — on a healthy profile the script is a no-op, on a
    // never-logged-in one it refuses, and with dead tokens restoring the flag wouldn't spare the
    // user the login anyway. Any of those would make the entry noise.
    if (p.needsOnboarding && p.credentialsValid !== false) {
      items.push({
        label: t('profiles.menuFixOnboarding'),
        title: busy ? t('common.busyDisabled') : t('profiles.menuFixOnboardingTip'),
        onClick: () => onAction('fix-onboarding', p.name),
        disabled: busy
      });
    }
    // Reset a custom provider back to the Anthropic default lives in the menu (kept off the card to
    // keep every card the same height) and only when there's a custom provider to reset.
    if (p.exists && providerByName.get(p.name)?.baseUrl) {
      items.push({
        label: t('profiles.menuResetProvider'),
        title: busy ? t('common.busyDisabled') : t('profiles.menuResetProviderTip'),
        onClick: () => onProviderClear(p.name),
        disabled: busy
      });
    }
    items.push(
      {
        label: t('profiles.menuColor'),
        title: t('profiles.menuColorTip'),
        onClick: () => openDlg('recolor', p.name, p.color)
      },
      {
        label: t('profiles.menuRename'),
        title: t('profiles.menuRenameTip'),
        onClick: () => openDlg('rename', p.name, p.color)
      },
      {
        label: t('profiles.menuDescribe'),
        title: t('profiles.menuDescribeTip'),
        onClick: () => openDlg('redescribe', p.name, p.color, p.description ?? '')
      },
      {
        label: t('profiles.menuDelete'),
        title: busy ? t('common.busyDisabled') : t('profiles.menuDeleteTip', { name: p.name }),
        onClick: () => onMgmt({ action: 'remove', name: p.name }),
        disabled: busy,
        danger: true
      }
    );
    return items;
  }

  type Prof = (typeof profiles)[number];
  // Column widths trimmed so the six columns fit a ~1150px content pane without horizontal scroll
  // clipping the Действия column (the owner's "table doesn't fit / gets cut off"). name still grows.
  const COLS: DTColumn[] = $derived([
    { key: 'name', label: t('profiles.colName'), grow: true, sortable: true },
    { key: 'status', label: t('profiles.colStatus'), width: '120px', sortable: true },
    // Widened: the usage badge (5h/7d/scoped + reset) wrapped to 3-4 lines at 140px; the 3 action
    // buttons (Launch/Folder/⋯) overflowed 210px and the leftmost (Launch) was clipped on the left.
    { key: 'usage', label: t('profiles.colUsage'), width: '208px' },
    { key: 'provider', label: t('profiles.colProvider'), width: '156px', interactive: true, sortable: true },
    { key: 'links', label: t('profiles.colLinks'), width: '80px', align: 'center', sortable: true },
    { key: 'actions', label: t('profiles.colActions'), width: '200px', interactive: true }
  ]);
  function linkedCount(p: Prof): number {
    return Object.values(p.sharedLinks).filter(
      (k) => k === 'Junction' || k === 'SymbolicLink' || k === 'HardLink'
    ).length;
  }
  function profSort(p: Prof, key: string): string | number {
    // Broken-but-signed-in sorts BELOW healthy and ABOVE never-logged-in: those rows want attention,
    // but a missing dir is still worse.
    if (key === 'status') {
      if (!p.exists) return 3;
      const broken = p.needsOnboarding || (p.credentialsPresent && p.credentialsValid === false);
      return broken ? 1 : p.credentialsPresent ? 0 : 2;
    }
    if (key === 'provider') return (providerLabelByName.get(p.name) ?? t('profiles.providerDefault')).toLowerCase();
    if (key === 'links') return linkedCount(p);
    return p.name.toLowerCase();
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('profiles.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('profiles.health', { n: profiles.length, profiles: pProfile(profiles.length) })}</p>
      {#if data?.generatedAt}<p class="text-sw-xs text-sw-text-muted mt-0.5">{t('profiles.checkedAt', { time: relTime(data.generatedAt) })}</p>{/if}
    </div>
    <div class="flex shrink-0 gap-sw-2">
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={() => onAction('check')}
        title={t('profiles.checkTip')}>
        {running === 'profiles' ? t('profiles.checking') : t('common.check')}
      </button>
      <button class="sw-btn" disabled={busy} onclick={() => openDlg('add')}
        title={t('profiles.addProfileTip')}>
        {t('profiles.addProfile')}
      </button>
      <!-- U7: outlined, not solid red — the most destructive action must not be the tab's loudest CTA -->
      <button class="sw-btn sw-btn-danger-ghost" disabled={busy} onclick={() => onAction('reinstall')}
        title={t('profiles.reinstallTip')}>
        {t('profiles.reinstall')}
      </button>
    </div>
  </header>

  <ProfileEditDialog
    open={dlgOpen}
    mode={dlgMode}
    current={dlgCurrent}
    currentColor={dlgColor}
    currentDescription={dlgDescription}
    onSubmit={onDlgSubmit}
    onCancel={() => (dlgOpen = false)}
  />

  <LaunchConfigDialog
    open={lcOpen}
    profile={lcProfile}
    availableMcp={launchConfig?.availableMcp ?? []}
    onSave={(v) => onSaveLaunch(lcProfile!.name, v.mode, v.mcp, v.claudeMd)}
    onMeasure={(lean) => onMeasure(lcProfile!.name, lean)}
    onCancel={() => (lcOpen = false)}
  />

  <ProviderEditDialog
    open={pvOpen}
    profileName={pvName}
    current={pvCurrent}
    engines={engines ?? []}
    myProviders={myProviders ?? []}
    onSubmit={onPvSubmit}
    onCancel={() => (pvOpen = false)}
  />

  <ModalShell open={elevOpen} onClose={() => (elevOpen = false)} size="sm" role="alertdialog">
    <div class="flex flex-col gap-sw-3 p-sw-1">
      <h2 class="text-base font-semibold">{t('profiles.elevateTitle', { name: elevProfile })}</h2>
      <p class="text-sw-sm text-sw-text-secondary">{t('profiles.elevateMsg')}</p>
      <div class="mt-sw-2 flex flex-col gap-sw-2">
        <button class="sw-btn sw-btn-primary" onclick={() => { elevOpen = false; onRepairElevated(elevProfile); }}
          title={t('profiles.elevateRepairOnceTip')}>{t('profiles.elevateRepairOnce')}</button>
        <button class="sw-btn" onclick={() => { elevOpen = false; onRelaunchAdmin(); }}
          title={t('profiles.elevateRelaunchTip')}>{t('profiles.elevateRelaunch')}</button>
        <button class="sw-btn sw-btn-ghost" onclick={() => (elevOpen = false)}>{t('common.cancel')}</button>
      </div>
    </div>
  </ModalShell>

  <!-- Recommendations -->
  {#if data}
    {#if hasIssues}
      <div class="sw-card mb-sw-4 border border-amber-500/40">
        <div class="mb-sw-2 font-medium status-warn">{t('profiles.recommendations')}</div>
        <ul class="space-y-2 text-sw-sm">
          {#if brokenLinks.length > 0}
            <li class="flex flex-wrap items-center justify-between gap-sw-2">
              <span>{t('profiles.brokenLinks', { n: brokenLinks.length, profiles: pProfile(brokenLinks.length) })}</span>
              <div class="flex flex-wrap gap-sw-2">
                {#each brokenLinks as p (p.name)}
                  <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => finishProfile(p.name)}
                    title={isAdmin ? t('profiles.repairNameTip', { name: p.name }) : t('profiles.finishAdminTip', { name: p.name })}>
                    {isAdmin ? t('profiles.repairName', { name: p.name }) : t('profiles.finishAdmin', { name: p.name })}</button>
                {/each}
              </div>
            </li>
          {/if}
          {#if missing.length > 0}
            <li class="flex flex-wrap items-center justify-between gap-sw-2">
              <span>{t('profiles.missingDirs', { names: missing.map((p) => p.name).join(', ') })}</span>
              <div class="flex flex-wrap gap-sw-2">
                {#each missing as p (p.name)}
                  <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => onAction('create', p.name)}
                    title={t('profiles.createNameTip', { name: p.name })}>{t('profiles.createName', { name: p.name })}</button>
                {/each}
              </div>
            </li>
          {/if}
          {#if conflictCount > 0}
            <li class="flex items-center justify-between gap-sw-2">
              <span>{t('profiles.syncConflicts', { n: conflictCount })}</span>
              <button class="sw-btn sw-btn-danger text-sw-xs shrink-0" disabled={busy} onclick={() => onAction('clean-conflicts')}
                title={t('profiles.cleanConflictsTip')}>{t('profiles.cleanConflicts')}</button>
            </li>
          {/if}
        </ul>
      </div>
    {:else}
      <div class="sw-card mb-sw-4 flex items-center gap-sw-2 border border-emerald-500/30">
        <span class="badge badge-ok">{t('profiles.allGood')}</span>
        <span class="text-sw-sm text-sw-text-secondary">{t('profiles.allGoodHint')}</span>
      </div>
    {/if}
  {/if}

  <!-- Orphan dirs: ~/.claude-<name> on disk that aren't canon profiles (abandoned/foreign CC configs) -->
  {#if orphans.length > 0}
    <div class="sw-card mb-sw-4 border border-amber-500/40">
      <div class="mb-sw-1 font-medium status-warn">{t('profiles.orphansTitle', { n: orphans.length })}</div>
      <p class="mb-sw-3 text-sw-sm text-sw-text-secondary">{t('profiles.orphansHint')}</p>
      <ul class="space-y-2 text-sw-sm">
        {#each orphans as o (o.name)}
          <li class="flex flex-wrap items-center justify-between gap-sw-2">
            <span class="min-w-0 break-all">
              <code class="text-sw-text">.claude-{o.name}</code>
              {#if o.modified > 0}
                <span class="text-sw-xs text-sw-text-muted"> · {t('profiles.orphanModified', { time: relTime(new Date(o.modified * 1000).toISOString()) })}</span>
              {/if}
            </span>
            <div class="flex flex-wrap gap-sw-2">
              <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => onOpen(o.name)}
                title={t('profiles.folderTip', { name: o.name })}>{t('profiles.folder')}</button>
              <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => onMgmt({ action: 'add', name: o.name })}
                title={t('profiles.orphanAdoptTip', { name: o.name })}>{t('profiles.orphanAdopt')}</button>
              <button class="sw-btn sw-btn-danger text-sw-xs shrink-0" disabled={busy} onclick={() => onDeleteOrphan(o.name)}
                title={t('profiles.orphanDeleteTip', { name: o.name })}>{t('profiles.orphanDelete')}</button>
            </div>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if data === null}
    <div class="flex flex-col gap-sw-2">
      {#each Array(4) as _, i (i)}
        <div class="skeleton" style="height:2.4rem"></div>
      {/each}
    </div>
  {:else if profiles.length}
    <DataTable
      columns={COLS}
      rows={profiles}
      rowKey={(p) => p.name}
      sortAccessor={profSort}
      search
      searchValue={(p) => `${p.name} ${p.description ?? ''}`}
      searchPlaceholder={t('profiles.searchPlaceholder')}
      defaultSort="name"
      storageKey="profiles"
      canExpand={(p) => p.exists}
      rowMuted={(p) => !p.exists}
      rowAccent={(p) => p.exists}
      highlightAttr={(p) => `profile:${p.name}`}
      rowStyle={(p) => p.exists ? `--row-accent:${dot(p.color)}` : undefined}
    >
      {#snippet cell(p, col)}
        {@const links = Object.entries(p.sharedLinks)}
        {@const linked = linkedCount(p)}
        {@const lc = launchByName.get(p.name)}
        {#if col.key === 'name'}
          <span class="namecell">
            <span class="h-3 w-3 shrink-0 rounded-full" style="background:{dot(p.color)}" title={t('profiles.colorDot')}></span>
            <span class="min-w-0">
              <span class="block truncate font-medium" title={p.name}>{p.name}</span>
              {#if p.description}<span class="block truncate text-sw-xs text-sw-text-muted" title={p.description}>{p.description}</span>{/if}
            </span>
          </span>
        {:else if col.key === 'status'}
          <span class="flex flex-wrap items-center gap-sw-1">
            {#if !p.exists}
              <span class="badge badge-err" title={t('profiles.noDirTip', { name: p.name })}>{t('profiles.noDir')}</span>
            {:else if p.credentialsPresent && p.credentialsValid === false}
              <!-- Checked before needsOnboarding: with dead tokens the wizard is unavoidable, so
                   "fix onboarding" would be a lie — this profile needs a real login. -->
              <span class="badge badge-warn" title={t('profiles.staleLoginTip')}>{t('profiles.staleLogin')}</span>
            {:else if p.needsOnboarding}
              <span class="badge badge-warn"
                title={p.logoutResidue ? t('profiles.needsOnboardingLogoutTip') : t('profiles.needsOnboardingTip')}>
                {t('profiles.needsOnboarding')}
              </span>
            {:else}
              <span class="badge {p.credentialsPresent ? 'badge-ok' : 'badge-muted'}"
                title={p.credentialsPresent ? t('profiles.loggedInTip') : t('profiles.noLoginTip')}>
                {p.credentialsPresent ? t('profiles.loggedIn') : t('profiles.noLogin')}
              </span>
              {#if lc?.mode === 'lean'}
                <span class="badge badge-info" title={t('profiles.leanTip', { flag: lc.tokenAuth ? '--bare' : '--safe-mode' })}>{t('profiles.lean')}</span>
              {/if}
            {/if}
          </span>
        {:else if col.key === 'usage'}
          {#if p.exists && p.credentialsPresent && p.credentialsValid !== false}
            <ProfileUsageBadge profile={p.name} />
          {:else}
            <span class="text-sw-text-muted">—</span>
          {/if}
        {:else if col.key === 'provider'}
          {#if p.exists}
            <button type="button" class="block min-w-0 max-w-full truncate text-left text-sw-xs font-medium text-sw-text-secondary hover:text-sw-text disabled:opacity-60"
              disabled={busy} onclick={() => editProvider(p.name)} title={t('profiles.providerEditTip')}>{providerLabel(p.name)}</button>
          {:else}
            <span class="text-sw-text-muted">—</span>
          {/if}
        {:else if col.key === 'links'}
          {#if p.exists}
            <span class="badge {linked === links.length ? 'badge-ok' : 'badge-warn'}"
              title={t('profiles.linksTip', { linked, total: links.length })}>{linked}/{links.length}</span>
          {:else}
            <span class="text-sw-text-muted">—</span>
          {/if}
        {:else if col.key === 'actions'}
          <!-- Canon: primary action stays a labelled button; secondary (folder) + overflow (⋯) become
               a compact icon cluster so the column never clips. -->
          <span class="flex items-center justify-end gap-sw-2">
            <button class="sw-btn sw-btn-primary text-sw-xs inline-flex items-center gap-sw-1" disabled={!p.exists} onclick={() => onLaunch(p.name, 'terminal')}
              title={t('profiles.launchTip', { name: p.name })}><Play size={12} />{t('profiles.launch')}</button>
            <span class="acts">
              <button class="iact" disabled={!p.exists} onclick={() => onOpen(p.name)}
                title={t('profiles.folderTip', { name: p.name })} aria-label={t('profiles.folder')}><FolderOpen size={15} /></button>
              <DropdownMenu title={t('profiles.menuTitle')} items={menuItems(p)} />
            </span>
          </span>
        {/if}
      {/snippet}

      {#snippet expand(p)}
        <MatrixRowEditor st={mtx} name={p.name} />
      {/snippet}
    </DataTable>
  {:else}
    <EmptyState icon={Users} title={t('profiles.noData')} description={t('profiles.noDataHint')} />
  {/if}

  {#if profiles.length}
    <MatrixControls st={mtx} />
  {/if}
</div>

<ModalShell open={viewerOpen} onClose={() => (viewerOpen = false)} size="lg">
  <div class="mb-sw-3 flex flex-wrap items-center justify-between gap-sw-2">
    <h3 class="font-semibold">{t('profiles.menuViewConfig')}: {viewerName}</h3>
    <div class="flex gap-sw-2">
      <Segmented
        compact
        value={viewerWhich}
        options={[
          { value: 'settings', label: t('profiles.viewSettings') },
          { value: 'claude', label: t('profiles.viewClaudeMd') }
        ]}
        onChange={(v) => setWhich(v)}
      />
      <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!viewerContent} onclick={() => copyText(viewerDisplay)}
        title={t('common.copy')}>{t('common.copy')}</button>
    </div>
  </div>
  {#if viewerLoading}
    <p class="text-sw-sm text-sw-text-muted">{t('common.loading')}</p>
  {:else if viewerErr}
    <p class="text-sw-sm status-bad">{viewerErr}</p>
  {:else}
    <pre class="cfg-view">{viewerDisplay}</pre>
  {/if}
</ModalShell>

<style>
  .cfg-view {
    max-height: 60vh;
    overflow: auto;
    margin: 0;
    padding: var(--sw-space-3);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
