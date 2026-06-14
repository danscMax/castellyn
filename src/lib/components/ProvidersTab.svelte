<script lang="ts">
  import type { EngineStatus, ProfileProvider, ProviderArgs } from '$lib/ipc';
  import { updateEngine } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import ProviderEditDialog from './ProviderEditDialog.svelte';
  import RouterConnectDialog from './RouterConnectDialog.svelte';

  let {
    engines,
    providers,
    running,
    onEngine,
    onProviderSet,
    onProviderClear,
    onRouterInstall,
    onConnectRouter,
    onRefresh,
    onOpenUrl
  }: {
    engines: EngineStatus[] | null;
    providers: ProfileProvider[] | null;
    running: string | null;
    onEngine: (action: 'start' | 'stop', id: string) => void;
    onProviderSet: (args: ProviderArgs) => void;
    onProviderClear: (name: string) => void;
    onRouterInstall: () => void;
    onConnectRouter: (engine: EngineStatus, model: string, profile: string) => void;
    onRefresh: () => void;
    onOpenUrl: (url: string) => void;
  } = $props();

  const busy = $derived(!!running);
  const engineList = $derived(engines ?? []);
  const providerList = $derived(providers ?? []);
  const profileNames = $derived(providerList.map((p) => p.name));

  // Router-connect dialog (pick model + profile).
  let rcOpen = $state(false);
  let rcEngine = $state<EngineStatus | null>(null);
  function openConnect(e: EngineStatus) {
    rcEngine = e;
    rcOpen = true;
  }

  // Inline endpoint (baseUrl/port) editor.
  let editId = $state<string | null>(null);
  let editUrl = $state('');
  let editPort = $state(0);
  function openEdit(e: EngineStatus) {
    if (editId === e.id) {
      editId = null;
      return;
    }
    editId = e.id;
    editUrl = e.baseUrl;
    editPort = e.port;
  }
  async function saveEdit() {
    if (!editId) return;
    try {
      await updateEngine(editId, editUrl.trim(), Number(editPort) || 0);
      editId = null;
      onRefresh();
    } catch (e) {
      /* surfaced via refresh */ editId = null;
    }
  }

  let dlgOpen = $state(false);
  let dlgProfile = $state('');
  let dlgCurrent = $state<ProfileProvider | null>(null);
  function edit(p: ProfileProvider) {
    dlgProfile = p.name;
    dlgCurrent = p;
    dlgOpen = true;
  }
  function onRcSubmit(v: { model: string; profile: string }) {
    rcOpen = false;
    if (!rcEngine) return;
    // Anthropic-native engine → bind the profile straight to it (no ccr). LM Studio needs a
    // non-empty bearer ('lmstudio'); other Anthropic proxies keep any token already set.
    if (rcEngine.protocol === 'anthropic' && !rcEngine.router) {
      const isLm = rcEngine.id === 'lmstudio';
      onProviderSet({
        action: 'set',
        name: v.profile,
        baseUrl: rcEngine.baseUrl,
        token: isLm ? 'lmstudio' : '',
        model: v.model,
        keepToken: !isLm
      });
    } else {
      onConnectRouter(rcEngine, v.model, v.profile);
    }
  }
  function onDlgSubmit(v: {
    baseUrl: string;
    token: string;
    model: string;
    smallModel: string;
    keepToken: boolean;
  }) {
    dlgOpen = false;
    onProviderSet({
      action: 'set',
      name: dlgProfile,
      baseUrl: v.baseUrl,
      token: v.token,
      model: v.model,
      smallModel: v.smallModel,
      keepToken: v.keepToken
    });
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('providers.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('providers.subtitle')}
      </p>
    </div>
    <button class="sw-btn sw-btn-ghost shrink-0" disabled={busy} onclick={onRefresh}
      title={t('providers.refreshTitle')}>
      {running === 'engine' || running === 'provider' ? t('providers.busy') : t('providers.refreshLabel')}
    </button>
  </header>

  <ProviderEditDialog
    open={dlgOpen}
    profileName={dlgProfile}
    current={dlgCurrent}
    engines={engineList}
    onSubmit={onDlgSubmit}
    onCancel={() => (dlgOpen = false)}
  />

  <RouterConnectDialog
    open={rcOpen}
    engine={rcEngine}
    profiles={profileNames}
    onSubmit={onRcSubmit}
    onCancel={() => (rcOpen = false)}
  />

  <!-- Engines -->
  <h2 class="mb-sw-2 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{t('providers.enginesHeading')}</h2>
  {#if engineList.length}
    <div class="card-grid">
      {#each engineList as e (e.id)}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div class="min-w-0">
              <h3 class="truncate font-medium">{e.name}</h3>
              <p class="truncate font-mono text-[11px] text-sw-text-muted">{e.baseUrl} · :{e.port}</p>
            </div>
            <div class="flex shrink-0 flex-col items-end gap-1">
              <span class="badge {e.running ? 'badge-ok' : 'badge-muted'}" title={e.running ? t('providers.portListening') : t('providers.portNotResponding')}>
                {e.running ? t('providers.running') : t('providers.stopped')}
              </span>
              <span class="badge {e.protocol === 'anthropic' ? 'badge-info' : 'badge-warn'}"
                title={e.protocol === 'anthropic' ? t('providers.protoAnthropicTitle') : t('providers.protoOpenaiTitle')}>
                {e.protocol}
              </span>
              {#if e.router && e.installed !== null}
                <span class="badge {e.installed ? 'badge-ok' : 'badge-muted'}"
                  title={e.installed ? t('providers.ccrInstalledTitle') : t('providers.ccrNotInstalledTitle')}>
                  {e.installed ? t('providers.installed') : t('providers.notInstalled')}
                </span>
              {/if}
            </div>
          </div>
          {#if editId === e.id}
            <div class="rounded-sw-md border border-sw-border p-sw-2">
              <p class="mb-sw-2 text-sw-xs font-medium text-sw-text-secondary">{t('providers.endpointEditorTitle')}</p>
              <div class="flex flex-col gap-sw-2">
                <input class="sw-input text-sw-xs" bind:value={editUrl} placeholder="http://localhost:1234" spellcheck="false" title={t('providers.editUrlInputTip')} />
                <input class="sw-input text-sw-xs" type="number" bind:value={editPort} placeholder={t('providers.portPlaceholder')} title={t('providers.editPortInputTip')} />
              </div>
              <div class="mt-sw-2 flex gap-sw-2">
                <button class="sw-btn text-sw-xs" onclick={saveEdit} title={t('providers.saveEngineTitle')}>{t('providers.save')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (editId = null)} title={t('providers.cancelEditTip')}>{t('providers.cancel')}</button>
              </div>
            </div>
          {/if}

          <div class="mt-auto flex flex-wrap gap-sw-2 border-t border-sw-border pt-sw-2">
            {#if e.router && e.installed === false}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={onRouterInstall}
                title={t('providers.installCcrTitle')}>{t('providers.install')}</button>
            {/if}
            {#if e.hasCommand}
              {#if e.running}
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onEngine('stop', e.id)}
                  title={e.router ? t('providers.stopRouterTitle') : t('providers.stopProcessTitle', { port: e.port })}>{t('providers.stop')}</button>
              {:else}
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onEngine('start', e.id)}
                  title={e.router ? t('providers.startRouterTitle') : t('providers.startEngineTitle')}>{t('providers.start')}</button>
              {/if}
            {:else}
              <span class="text-sw-xs text-sw-text-muted">{t('providers.manualStart')}</span>
            {/if}
            {#if e.protocol === 'anthropic' && !e.router}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !e.running} onclick={() => openConnect(e)}
                title={e.running
                  ? t('providers.bindReadyTitle')
                  : t('providers.bindNotReadyTitle')}>
                {t('providers.bindToProfile')}
              </button>
            {/if}
            {#if e.protocol === 'openai' && !e.router}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !e.running} onclick={() => openConnect(e)}
                title={e.running
                  ? t('providers.connectReadyTitle')
                  : t('providers.connectNotReadyTitle')}>
                {t('providers.connectViaRouter')}
              </button>
            {/if}
            {#if e.dashboardUrl}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!e.running} onclick={() => onOpenUrl(e.dashboardUrl)}
                title={e.running ? t('providers.openDashboardTitle', { url: e.dashboardUrl }) : t('providers.dashboardWhenRunningTitle')}>{t('providers.dashboard')}</button>
            {/if}
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => openEdit(e)}
              title={t('providers.editEndpointTitle')}>{t('providers.portUrl')}</button>
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="sw-card text-sw-sm text-sw-text-muted">{t('providers.noEngines')}</div>
  {/if}

  <!-- Provider per profile -->
  <h2 class="mb-sw-2 mt-sw-6 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">
    {t('providers.providerPerProfileHeading')}
  </h2>
  {#if providerList.length}
    <div class="card-grid">
      {#each providerList as p (p.name)}
        {@const custom = !!p.baseUrl}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div class="min-w-0">
              <h3 class="font-medium">{p.name}</h3>
              {#if custom}
                <p class="truncate font-mono text-[11px] text-sw-text-secondary" title={p.baseUrl}>{p.baseUrl}</p>
              {:else}
                <p class="text-sw-xs text-sw-text-muted">{t('providers.defaultProvider')}</p>
              {/if}
            </div>
            {#if custom}
              <span class="badge badge-info shrink-0" title={t('providers.customProviderTitle')}>{t('providers.custom')}</span>
            {/if}
          </div>
          {#if custom && (p.model || p.smallModel || p.hasToken)}
            <div class="flex flex-wrap gap-sw-2">
              {#if p.model}<span class="badge badge-muted" title={t('providers.modelTitle')}>{p.model}</span>{/if}
              {#if p.smallModel}<span class="badge badge-muted" title={t('providers.smallModelTitle')}>{p.smallModel}</span>{/if}
              {#if p.hasToken}<span class="badge badge-ok" title={t('providers.tokenSetTitle')}>{t('providers.tokenSet')}</span>{/if}
            </div>
          {/if}
          <div class="mt-auto flex flex-wrap gap-sw-2 border-t border-sw-border pt-sw-2">
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => edit(p)}
              title={t('providers.editProviderTitle')}>{t('providers.edit')}</button>
            {#if custom}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onProviderClear(p.name)}
                title={t('providers.resetProviderTitle')}>{t('providers.reset')}</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="sw-card text-sw-sm text-sw-text-muted">{t('providers.noProviderData')}</div>
  {/if}

  <p class="mt-sw-4 text-sw-xs text-sw-text-muted">
    {t('providers.footnote')}
  </p>
</div>
