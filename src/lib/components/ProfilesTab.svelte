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
    ProviderArgs
  } from '$lib/ipc';
  import { pProfile, t } from '$lib/i18n';
  import ProfileEditDialog from './ProfileEditDialog.svelte';
  import LaunchConfigDialog from './LaunchConfigDialog.svelte';
  import ProviderEditDialog from './ProviderEditDialog.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import Toggle from './Toggle.svelte';

  let {
    data,
    config,
    launchConfig,
    providers,
    engines,
    running,
    onAction,
    onMgmt,
    onOpen,
    onLaunch,
    onSaveLaunch,
    onMeasure,
    onProviderSet,
    onProviderClear,
    onOpenProviders
  }: {
    data: ProfilesStatus | null;
    config: ProfilesConfig | null;
    launchConfig: LaunchConfigStatus | null;
    providers: ProfileProvider[] | null;
    engines: EngineStatus[] | null;
    running: string | null;
    onAction: (action: ProfileAction, name?: string) => void;
    onMgmt: (args: ProfileMgmtArgs) => void;
    onOpen: (name: string) => void;
    onLaunch: (name: string, mode: 'terminal' | 'vscode') => void;
    onSaveLaunch: (name: string, mode: 'full' | 'lean', mcp: string[], claudeMd: boolean) => Promise<void>;
    onMeasure: (name: string, lean: boolean) => Promise<number>;
    onProviderSet: (args: ProviderArgs) => void;
    onProviderClear: (name: string) => void;
    onOpenProviders: () => void;
  } = $props();

  const busy = $derived(!!running);
  const profiles = $derived(data?.profiles ?? []);
  const conflicts = $derived(data?.syncConflicts);

  // Configured (not just observed) linked folders, per profile.
  const ALL_FOLDERS = ['agents', 'commands', 'hooks', 'plugins', 'skills', 'projects', 'history.jsonl'];
  const cfgByName = $derived(new Map((config?.profiles ?? []).map((p) => [p.name, p])));
  function configuredLinks(name: string): string[] {
    const p = cfgByName.get(name);
    if (p?.linkedFolders) return p.linkedFolders;
    return config?.sharedFoldersDefault ?? ALL_FOLDERS;
  }

  // Per-profile launch config (full vs lean) and provider.
  const launchByName = $derived(new Map((launchConfig?.profiles ?? []).map((p) => [p.name, p])));
  const providerByName = $derived(new Map((providers ?? []).map((p) => [p.name, p])));
  function providerLabel(name: string): string {
    const p = providerByName.get(name);
    if (!p || !p.baseUrl) return t('profiles.providerDefault');
    const eng = (engines ?? []).find((e) => e.baseUrl === p.baseUrl);
    if (eng) return eng.name;
    try {
      return new URL(p.baseUrl).host;
    } catch {
      return p.baseUrl;
    }
  }

  // Lifecycle dialog state.
  let dlgOpen = $state(false);
  let dlgMode = $state<'add' | 'rename' | 'recolor'>('add');
  let dlgCurrent = $state('');
  let dlgColor = $state('White');
  function openDlg(mode: 'add' | 'rename' | 'recolor', name = '', color = 'White') {
    dlgMode = mode;
    dlgCurrent = name;
    dlgColor = color;
    dlgOpen = true;
  }
  function onDlgSubmit(v: { name: string; color: string; description: string }) {
    dlgOpen = false;
    if (dlgMode === 'add') onMgmt({ action: 'add', name: v.name, color: v.color, description: v.description });
    else if (dlgMode === 'rename') onMgmt({ action: 'rename', name: dlgCurrent, newName: v.name });
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

  // Collapsible shared-folder matrix (progressive disclosure).
  let expanded = $state<Record<string, boolean>>({});

  // Per-card shared-folder editor (set-links).
  let linksFor = $state<string | null>(null);
  let linkSel = $state<Record<string, boolean>>({});
  function openLinks(name: string) {
    if (linksFor === name) {
      linksFor = null;
      return;
    }
    const cur = configuredLinks(name);
    linkSel = Object.fromEntries(ALL_FOLDERS.map((f) => [f, cur.includes(f)]));
    linksFor = name;
  }
  function applyLinks(name: string) {
    const enabled = ALL_FOLDERS.filter((f) => linkSel[f]);
    onMgmt({ action: 'set-links', name, enabled });
    linksFor = null;
  }

  // Problems → recommendations.
  const brokenLinks = $derived(profiles.filter((p) => p.exists && !p.linksIntact));
  const missing = $derived(profiles.filter((p) => !p.exists));
  const conflictCount = $derived(conflicts?.count ?? 0);
  const hasIssues = $derived(brokenLinks.length > 0 || missing.length > 0 || conflictCount > 0);

  const COLORS: Record<string, string> = {
    Cyan: '#22d3ee',
    Green: '#34d399',
    Yellow: '#fbbf24',
    Magenta: '#e879f9',
    Red: '#f87171'
  };
  function dot(c: string) {
    return COLORS[c] ?? '#94a3b8';
  }

  function linkLabel(kind: string | null) {
    if (kind === 'Junction') return t('profiles.linkJunction');
    if (kind === 'SymbolicLink') return t('profiles.linkSymlink');
    if (kind === 'HardLink') return t('profiles.linkHardlink');
    if (kind === 'none') return t('profiles.linkNotLink');
    return t('profiles.linkNone');
  }
  function linkCls(kind: string | null) {
    if (kind === 'Junction' || kind === 'SymbolicLink' || kind === 'HardLink') return 'badge-ok';
    if (kind === 'none') return 'badge-warn';
    return 'badge-err';
  }
  function linkTip(folder: string, kind: string | null) {
    if (kind === 'Junction' || kind === 'SymbolicLink' || kind === 'HardLink')
      return t('profiles.linkTipOk', { folder, kind: linkLabel(kind) });
    if (kind === 'none') return t('profiles.linkTipNone', { folder });
    return t('profiles.linkTipMissing', { folder });
  }

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
        label: t('profiles.menuRepair'),
        title: t('profiles.menuRepairTip'),
        onClick: () => onAction('repair', p.name),
        disabled: busy
      });
    }
    // Reset a custom provider back to the Anthropic default lives in the menu (kept off the card to
    // keep every card the same height) and only when there's a custom provider to reset.
    if (p.exists && providerByName.get(p.name)?.baseUrl) {
      items.push({
        label: t('profiles.menuResetProvider'),
        title: t('profiles.menuResetProviderTip'),
        onClick: () => onProviderClear(p.name),
        disabled: busy
      });
    }
    items.push(
      {
        label: t('profiles.menuSharedFolders'),
        title: t('profiles.menuSharedFoldersTip'),
        onClick: () => openLinks(p.name),
        disabled: busy
      },
      {
        label: t('profiles.menuColor'),
        title: t('profiles.menuColorTip'),
        onClick: () => openDlg('recolor', p.name, p.color),
        disabled: busy
      },
      {
        label: t('profiles.menuRename'),
        title: t('profiles.menuRenameTip'),
        onClick: () => openDlg('rename', p.name, p.color),
        disabled: busy
      },
      {
        label: t('profiles.menuDelete'),
        title: t('profiles.menuDeleteTip', { name: p.name }),
        onClick: () => onMgmt({ action: 'remove', name: p.name }),
        disabled: busy,
        danger: true
      }
    );
    return items;
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('profiles.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('profiles.health', { n: profiles.length, profiles: pProfile(profiles.length) })}</p>
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
      <button class="sw-btn sw-btn-danger" disabled={busy} onclick={() => onAction('reinstall')}
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
    onSubmit={onPvSubmit}
    onCancel={() => (pvOpen = false)}
  />

  <!-- Recommendations -->
  {#if data}
    {#if hasIssues}
      <div class="sw-card mb-sw-4 border border-amber-500/40">
        <div class="mb-sw-2 font-medium text-amber-400">{t('profiles.recommendations')}</div>
        <ul class="space-y-2 text-sw-sm">
          {#if brokenLinks.length > 0}
            <li class="flex flex-wrap items-center justify-between gap-sw-2">
              <span>{t('profiles.brokenLinks', { n: brokenLinks.length, profiles: pProfile(brokenLinks.length) })}</span>
              <div class="flex flex-wrap gap-sw-2">
                {#each brokenLinks as p (p.name)}
                  <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => onAction('repair', p.name)}
                    title={t('profiles.repairNameTip', { name: p.name })}>{t('profiles.repairName', { name: p.name })}</button>
                {/each}
              </div>
            </li>
          {/if}
          {#if missing.length > 0}
            <li class="flex items-center justify-between gap-sw-2">
              <span>{t('profiles.missingDirs', { names: missing.map((p) => p.name).join(', ') })}</span>
              <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => onAction('reinstall')}
                title={t('profiles.createTip')}>{t('profiles.create')}</button>
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

  {#if profiles.length}
    <div class="card-grid">
      {#each profiles as p (p.name)}
        {@const links = Object.entries(p.sharedLinks)}
        {@const linked = links.filter(([, k]) => k === 'Junction' || k === 'SymbolicLink' || k === 'HardLink').length}
        {@const lc = launchByName.get(p.name)}
        <div class="sw-card flex flex-col gap-sw-3">
          <!-- Header: dot + name + role + menu -->
          <div class="flex items-start justify-between gap-sw-2">
            <div class="flex min-w-0 items-center gap-sw-2">
              <span class="h-3 w-3 shrink-0 rounded-full" style="background:{dot(p.color)}" title={t('profiles.colorDot')}></span>
              <div class="min-w-0">
                <h3 class="truncate font-medium">{p.name}</h3>
                {#if p.description}<p class="truncate text-sw-xs text-sw-text-muted">{p.description}</p>{/if}
              </div>
            </div>
            <DropdownMenu title={t('profiles.menuTitle')} items={menuItems(p)} />
          </div>

          <!-- Status row -->
          <div class="flex flex-wrap items-center gap-sw-2">
            {#if !p.exists}
              <span class="badge badge-err" title={t('profiles.noDirTip', { name: p.name })}>{t('profiles.noDir')}</span>
            {:else}
              <span class="badge {p.credentialsPresent ? 'badge-ok' : 'badge-muted'}"
                title={p.credentialsPresent ? t('profiles.loggedInTip') : t('profiles.noLoginTip')}>
                {p.credentialsPresent ? t('profiles.loggedIn') : t('profiles.noLogin')}
              </span>
              {#if lc?.mode === 'lean'}
                <span class="badge badge-info" title={t('profiles.leanTip', { flag: lc.tokenAuth ? '--bare' : '--safe-mode' })}>{t('profiles.lean')}</span>
              {/if}
              <button type="button" class="badge {linked === links.length ? 'badge-ok' : 'badge-warn'} cursor-pointer"
                onclick={() => (expanded[p.name] = !expanded[p.name])}
                title={t('profiles.linksTip', { linked, total: links.length })}>
                {t('profiles.links', { linked, total: links.length })} {expanded[p.name] ? '▴' : '▾'}
              </button>
            {/if}
          </div>

          <!-- Provider (single line: value truncates so every card keeps the same height) -->
          {#if p.exists}
            {@const prov = providerByName.get(p.name)}
            <div class="flex min-w-0 items-center gap-sw-2 text-sw-xs">
              <span class="shrink-0 text-sw-text-muted">{t('profiles.providerLabel')}</span>
              <button type="button" class="min-w-0 flex-1 truncate text-left font-medium text-sw-text-secondary underline decoration-dotted underline-offset-2 hover:text-sw-text"
                onclick={onOpenProviders} title={t('profiles.providerOpenTip')}>{providerLabel(p.name)}</button>
              <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy} onclick={() => editProvider(p.name)}
                title={t('profiles.providerEditTip')}>{t('profiles.providerEdit')}</button>
            </div>
          {/if}

          <!-- Expandable shared-folder matrix -->
          {#if p.exists && expanded[p.name]}
            <dl class="grid grid-cols-2 gap-x-sw-4 gap-y-1 rounded-sw-md border border-sw-border p-sw-2 text-sw-xs">
              {#each links as [folder, kind] (folder)}
                <div class="flex items-center justify-between gap-sw-2">
                  <dt class="truncate text-sw-text-muted">{folder}</dt>
                  <dd><span class="badge {linkCls(kind)}" title={linkTip(folder, kind)}>{linkLabel(kind)}</span></dd>
                </div>
              {/each}
            </dl>
          {/if}

          <!-- Shared-folders editor (set-links) -->
          {#if linksFor === p.name}
            <div class="rounded-sw-md border border-sw-border p-sw-2">
              <p class="mb-sw-2 text-sw-xs font-medium text-sw-text-secondary" title={t('profiles.sharedFoldersTip')}>
                {t('profiles.sharedFolders')}
              </p>
              <div class="grid grid-cols-2 gap-1">
                {#each ALL_FOLDERS as f (f)}
                  <div class="flex items-center gap-sw-2 text-sw-xs">
                    <Toggle bind:checked={linkSel[f]} disabled={busy} title={f} />
                    <span class="font-mono">{f}</span>
                  </div>
                {/each}
              </div>
              <div class="mt-sw-2 flex gap-sw-2">
                <button class="sw-btn text-sw-xs" disabled={busy} onclick={() => applyLinks(p.name)}
                  title={t('profiles.applyLinksTip')}>{t('common.apply')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (linksFor = null)}
                  title={t('profiles.linksCancelTip')}>{t('common.cancel')}</button>
              </div>
            </div>
          {/if}

          <!-- Main action -->
          <div class="mt-auto flex flex-wrap items-center gap-sw-2 border-t border-sw-border pt-sw-2">
            <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!p.exists} onclick={() => onLaunch(p.name, 'terminal')}
              title={t('profiles.launchTip', { name: p.name })}>
              {t('profiles.launch')}
            </button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!p.exists} onclick={() => onOpen(p.name)}
              title={t('profiles.folderTip', { name: p.name })}>{t('profiles.folder')}</button>
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="grid place-items-center py-sw-6 text-center text-sw-text-muted">
      <div>
        <div class="mb-sw-2 text-2xl">☰</div>
        <div class="font-medium text-sw-text">{t('profiles.noData')}</div>
        <div class="text-sw-sm">{t('profiles.noDataHint')}</div>
      </div>
    </div>
  {/if}
</div>
