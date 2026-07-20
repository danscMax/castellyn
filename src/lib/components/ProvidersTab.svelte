<script lang="ts">
  import {
    askConfirm as gateAsk,
    doConfirm as gateDo,
    closeConfirm as gateClose,
    emptyConfirmState,
    type ConfirmState
  } from '$lib/confirmGate';
  import type {
    EngineStatus,
    ProfileProvider,
    ProviderArgs,
    StackService,
    MyProvider,
    MyProviderInput
  } from '$lib/ipc';
  import { updateEngine, checkMyProvider, checkProviderUrl, checkProviderBalance, readStackProcs, freellmapiAuthStatus, gatewayBaseUrl, stackLogPath, openPath, type StackProc, type ProviderBalance } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { isValidHttpUrl } from '$lib/url';
  import EmptyState from './EmptyState.svelte';
  import { pushToast } from '$lib/toast.svelte';
  import { statusTextClass } from '$lib/statusColor';
  import MyProviderEditDialog from './MyProviderEditDialog.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import RouterConnectDialog from './RouterConnectDialog.svelte';
  import StackHealthCard from './StackHealthCard.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { Plug, Puzzle } from '@lucide/svelte';

  let {
    engines,
    providers,
    stack = null,
    running,
    stackRunning = null,
    confirmDestructive = true,
    onStack,
    onProviderSet,
    onRouterInstall,
    onConnectRouter,
    onConnectOpencode,
    onRefresh,
    onOpenUrl,
    onOpenProfiles,
    myProviders = null,
    onMyProviderSave,
    onMyProviderDelete,
    onMyProviderConnect,
    onMyProviderAddKey,
    onMyProviderRemoveKey,
    onMyProviderNextKey,
    onSetFreellmapiAuth,
    onDeleteFreellmapiAuth
  }: {
    engines: EngineStatus[] | null;
    providers: ProfileProvider[] | null;
    stack?: StackService[] | null;
    running: string | null;
    /** LLM-stack busy state — its own concurrency domain, separate from `running`. Stop is never
     *  gated by it (the backend preempts an in-flight start); only start/restart are. */
    stackRunning?: string | null;
    /** R8: mirror the global "confirm destructive actions" toggle (settings #120). */
    confirmDestructive?: boolean;
    onStack?: (action: 'start' | 'stop' | 'restart', only?: string) => void;
    onProviderSet: (args: ProviderArgs) => void;
    onRouterInstall: () => void;
    onConnectRouter: (engine: EngineStatus, model: string, profile: string) => void;
    onConnectOpencode?: (engine: EngineStatus, model: string, key: string) => void;
    onRefresh: () => void;
    onOpenUrl: (url: string) => void;
    onOpenProfiles?: () => void;
    myProviders?: MyProvider[] | null;
    onMyProviderSave: (p: MyProviderInput, apiKey: string) => void;
    onMyProviderDelete: (id: string) => void;
    onMyProviderConnect: (id: string) => void;
    onMyProviderAddKey: (id: string, apiKey: string) => void;
    onMyProviderRemoveKey: (id: string, index: number) => void;
    onMyProviderNextKey: (id: string) => void;
    onSetFreellmapiAuth: (email: string, password: string, token: string) => void;
    onDeleteFreellmapiAuth: (key: 'email' | 'password' | 'token') => void;
  } = $props();

  const busy = $derived(!!running);
  // Stack start/restart gate on the stack's own busy; Stop is NEVER gated (backend preempts a
  // running start) except against a stop already in flight (avoid double-fire).
  const stackBusy = $derived(!!stackRunning);
  const stopBusy = $derived(stackRunning === 'stop');
  const engineList = $derived(engines ?? []);
  const providerList = $derived(providers ?? []);
  const profileNames = $derived(providerList.map((p) => p.name));
  // LLM-stack services (from stack.json, the single source of truth).
  const stackList = $derived(stack ?? []);

  // B2: unified "Services" — one card per physical service (joined by port), merging the LLM-stack
  // lifecycle view and the engines wiring view. A port present only in stack.json (fork backends)
  // shows lifecycle actions; one that is also a bindable engine adds wiring. Order: stack first.
  type MergedService = {
    key: string;
    name: string;
    port: number;
    protocol: string;
    running: boolean;
    baseUrl: string;
    dashboard: string;
    paid: boolean;
    stack: StackService | null;
    engine: EngineStatus | null;
  };
  const serviceList = $derived.by<MergedService[]>(() => {
    const byPort = new Map<number, MergedService>();
    for (const s of stackList) {
      byPort.set(s.port, {
        key: 'stk:' + s.id,
        name: s.name,
        port: s.port,
        protocol: s.protocol,
        running: s.running,
        baseUrl: 'http://127.0.0.1:' + s.port,
        dashboard: s.dashboard,
        paid: s.group === 'router',
        stack: s,
        engine: null
      });
    }
    for (const e of engineList) {
      const cur = byPort.get(e.port);
      if (cur) {
        cur.engine = e;
        if (e.baseUrl) cur.baseUrl = e.baseUrl;
        if (e.dashboardUrl) cur.dashboard = e.dashboardUrl;
        cur.running = cur.running || e.running;
      } else {
        byPort.set(e.port, {
          key: 'eng:' + e.id,
          name: e.name,
          port: e.port,
          protocol: e.protocol,
          running: e.running,
          baseUrl: e.baseUrl,
          dashboard: e.dashboardUrl,
          paid: false,
          stack: null,
          engine: e
        });
      }
    }
    return [...byPort.values()];
  });
  // Running services that expose a dashboard → "open all" target (stack or engine origin).
  const openableDashboards = $derived(serviceList.filter((s) => s.running && s.dashboard));
  // Secondary per-card actions live in a kebab (keeps the card from becoming a wall of buttons).
  function svcMenu(svc: MergedService) {
    const items: { label: string; title?: string; onClick: () => void; disabled?: boolean }[] = [];
    if (svc.dashboard) {
      items.push({
        label: t('providers.dashboard'),
        title: svc.running
          ? t('providers.openDashboardTitle', { url: svc.dashboard })
          : t('providers.dashboardWhenRunningTitle'),
        disabled: !svc.running,
        onClick: () => onOpenUrl(svc.dashboard)
      });
    }
    if (svc.engine) {
      const e = svc.engine;
      items.push({
        label: t('providers.portUrl'),
        title: t('providers.editEndpointTitle'),
        disabled: busy,
        onClick: () => openEdit(e)
      });
    }
    if (svc.stack && !svc.running) {
      const s = svc.stack;
      items.push({
        label: t('providers.openLog'),
        title: t('providers.openLogTip', { name: svc.name }),
        onClick: () => openStackLog(s.id)
      });
    }
    const hc = health['svc:' + svc.port];
    items.push({
      label: t('common.check'),
      title: t('providers.checkTip'),
      disabled: busy || hc === 'checking',
      onClick: () => checkUrl('svc:' + svc.port, svc.baseUrl, svc.protocol)
    });
    return items;
  }

  // Collapsible heavy section (this screen is long) — open by default.
  let servicesOpen = $state(true);
  // Redesign 2C: per-card "details" expander — PID/uptime are diagnostics, not headline info.
  let advOpen = $state<Record<string, boolean>>({});
  // Anchor chips: the tab is long — jump straight to a section.
  function jumpTo(id: string) {
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  // F13: resolve freellmapi gateway URL from stack.json at mount; fallback to hardcoded port.
  let freellmapiUrl = $state('http://localhost:13001');
  $effect(() => { gatewayBaseUrl().then(u => { if (u) freellmapiUrl = u; }).catch(() => {}); });

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
  // Same accept rule as the two provider-edit dialogs: an empty or non-http baseUrl saved here
  // lands in engines.json and then propagates into a profile on the next bind.
  const editUrlOk = $derived(isValidHttpUrl(editUrl.trim()));
  async function saveEdit() {
    if (!editId || !editUrlOk) return;
    try {
      await updateEngine(editId, editUrl.trim(), Number(editPort) || 0);
      editId = null;
      onRefresh();
    } catch (e) {
      // Surface the failure instead of silently closing — keep the editor open so the user can retry.
      pushToast({ kind: 'error', title: t('common.error'), detail: String(e) });
    }
  }

  function onRcSubmit(v: { model: string; profile: string; key?: string }) {
    rcOpen = false;
    if (!rcEngine) return;
    // opencode target → write opencode.json directly (OpenAI-native, no ccr).
    if (v.profile === '__opencode__') {
      onConnectOpencode?.(rcEngine, v.model, v.key ?? '');
      return;
    }
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
  // Stack process info (PID + uptime per port), refreshed whenever the stack list changes.
  let procs = $state<StackProc[]>([]);
  const procByPort = $derived(new Map(procs.map((p) => [p.port, p])));
  let procsSeq = 0; // generation token: ignore a probe that a newer one has superseded.
  $effect(() => {
    stackList.length; // re-probe when the set of services changes
    const seq = ++procsSeq;
    readStackProcs()
      .then((r) => {
        if (seq === procsSeq) procs = r;
      })
      .catch(() => {
        if (seq === procsSeq) procs = [];
      });
  });
  function fmtUptime(sec: number): string {
    if (sec <= 0) return '';
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    if (h >= 24) return t('providers.uptimeD', { d: Math.floor(h / 24), h: h % 24 });
    if (h > 0) return t('providers.uptimeH', { h, m });
    return t('providers.uptimeM', { m: Math.max(1, m) });
  }

  // Custom provider registry (own list; keys in Credential Manager).
  const myProviderList = $derived(myProviders ?? []);
  let mpDlgOpen = $state(false);
  let mpCurrent = $state<MyProvider | null>(null);
  function mpAdd() {
    mpCurrent = null;
    mpDlgOpen = true;
  }
  function mpEdit(p: MyProvider) {
    mpCurrent = p;
    mpDlgOpen = true;
  }
  // A2: secondary my-provider actions (balance/keys/rotate/edit/delete) live in a per-card kebab so
  // the card shows just Connect + Check inline instead of a 7-button wall.
  function mpMenu(p: MyProvider, balChecking: boolean) {
    const items: { label: string; title?: string; onClick: () => void; disabled?: boolean; danger?: boolean }[] = [];
    if (p.keyCount > 1) {
      items.push({ label: t('myProviders.nextKey'), title: t('myProviders.nextKeyTitle'), disabled: busy, onClick: () => onMyProviderNextKey(p.id) });
    }
    items.push({ label: t('myProviders.balance'), title: t('myProviders.balanceTitle'), disabled: busy || !p.hasKey || balChecking, onClick: () => checkBalance(p.id) });
    items.push({ label: t('myProviders.keys') + (p.keyCount > 1 ? ` (${p.keyCount})` : ''), title: t('myProviders.keysTitle'), onClick: () => toggleKeys(p.id) });
    items.push({ label: t('myProviders.edit'), title: t('myProviders.editTitle'), disabled: busy, onClick: () => mpEdit(p) });
    items.push({ label: t('myProviders.delete'), title: t('myProviders.deleteTitle'), disabled: busy, danger: true, onClick: () => onMyProviderDelete(p.id) });
    return items;
  }
  function mpDlgSubmit(p: MyProviderInput, apiKey: string) {
    mpDlgOpen = false;
    onMyProviderSave(p, apiKey);
  }
  // Inline freellmapi login (email+password preferred, token fallback) for the "via freellmapi" path.
  let loginOpen = $state(false);
  let loginEmail = $state('');
  let loginPassword = $state('');
  let loginToken = $state('');
  // Whether email/token are already stored (Credential Manager) — shown when the panel opens.
  let authStatus = $state<{ hasEmail: boolean; hasPassword: boolean; hasToken: boolean } | null>(null);
  async function toggleLogin() {
    loginOpen = !loginOpen;
    if (loginOpen) {
      try {
        authStatus = await freellmapiAuthStatus();
      } catch {
        authStatus = null;
      }
    }
  }
  function saveLogin() {
    if (!loginEmail.trim() && !loginToken.trim()) return;
    onSetFreellmapiAuth(loginEmail.trim(), loginPassword, loginToken.trim());
    loginEmail = '';
    loginPassword = '';
    loginToken = '';
    loginOpen = false;
  }
  // Per-provider liveness check result (id -> {ok, detail} | 'checking').
  let health = $state<Record<string, { ok: boolean; detail: string } | 'checking'>>({});
  async function check(id: string) {
    health = { ...health, [id]: 'checking' };
    try {
      const r = await checkMyProvider(id);
      health = { ...health, [id]: { ok: r.ok, detail: r.detail } };
    } catch (e) {
      health = { ...health, [id]: { ok: false, detail: String(e) } };
    }
  }
  // U12: check every custom provider at once (bounded concurrency so we don't hammer them), then
  // report a "N ok / M" summary. Per-row spinners come free from the shared `health` map.
  let checkingAll = $state(false);
  async function checkAll() {
    if (checkingAll || !myProviderList.length) return;
    checkingAll = true;
    try {
      const ids = myProviderList.map((p) => p.id);
      const CONC = 3;
      for (let i = 0; i < ids.length; i += CONC) {
        await Promise.all(ids.slice(i, i + CONC).map((id) => check(id)));
      }
      const ok = ids.filter((id) => {
        const h = health[id];
        return h !== 'checking' && h?.ok;
      }).length;
      pushToast({
        kind: ok === ids.length ? 'success' : 'warn',
        title: t('myProviders.checkAllDone', { ok, total: ids.length })
      });
    } finally {
      checkingAll = false;
    }
  }
  // U1: open a stack service's log file directly (the failure message points at it, but there was
  // no way to open it from the app).
  async function openStackLog(id: string) {
    try {
      const p = await stackLogPath(id);
      if (p) await openPath(p);
      else pushToast({ kind: 'info', title: t('providers.noLogYet') });
    } catch (e) {
      pushToast({ kind: 'error', title: t('common.error'), detail: String(e) });
    }
  }
  // Liveness check for an arbitrary base URL (engines / stack services), keyed separately so
  // results don't collide with my-provider checks. No key — local backends.
  async function checkUrl(key: string, baseUrl: string, protocol: string) {
    health = { ...health, [key]: 'checking' };
    try {
      const r = await checkProviderUrl(baseUrl, protocol);
      health = { ...health, [key]: { ok: r.ok, detail: r.detail } };
    } catch (e) {
      health = { ...health, [key]: { ok: false, detail: String(e) } };
    }
  }

  // Per-provider balance/credits (#B4): id -> result | 'checking'. Fetched on demand (a button),
  // since balance endpoints are slow/rate-limited and provider-specific.
  let balance = $state<Record<string, ProviderBalance | 'checking'>>({});
  async function checkBalance(id: string) {
    balance = { ...balance, [id]: 'checking' };
    try {
      balance = { ...balance, [id]: await checkProviderBalance(id) };
    } catch (e) {
      balance = { ...balance, [id]: { ok: false, detail: String(e) } };
    }
  }
  function fmtBalance(b: ProviderBalance): string {
    const a = b.amount ?? 0;
    return b.currency ? `${a} ${b.currency}` : `${a}`;
  }

  // Key rotation pool: expandable per-card panel + inline "add key" input.
  let keysOpen = $state<Record<string, boolean>>({});
  let newKey = $state<Record<string, string>>({});
  function toggleKeys(id: string) {
    keysOpen = { ...keysOpen, [id]: !keysOpen[id] };
  }
  function addKey(id: string) {
    const k = (newKey[id] ?? '').trim();
    if (!k) return;
    onMyProviderAddKey(id, k);
    newKey = { ...newKey, [id]: '' };
  }
  // Removing a key deletes it from Credential Manager (irreversible) — gate behind a confirm,
  // consistent with the provider Delete/Clear actions. The parent owns the global confirm dialog,
  // but key removal is local to this surface, so confirm here via the canonical ConfirmDialog.
  let confirm = $state<ConfirmState>(emptyConfirmState());
  const confirmRemoveKey = (id: string, index: number) =>
    gateAsk(confirm, confirmDestructive, {
      title: t('myProviders.removeKey'),
      message: t('myProviders.removeKeyTitle'),
      confirmLabel: t('myProviders.removeKey'),
      danger: true,
      action: () => onMyProviderRemoveKey(id, index)
    });
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
      {busy ? t('providers.busy') : t('providers.refreshLabel')}
    </button>
  </header>

  <RouterConnectDialog
    open={rcOpen}
    engine={rcEngine}
    profiles={profileNames}
    onSubmit={onRcSubmit}
    onCancel={() => (rcOpen = false)}
  />

  <MyProviderEditDialog
    open={mpDlgOpen}
    current={mpCurrent}
    profiles={profileNames}
    onSubmit={mpDlgSubmit}
    onCancel={() => (mpDlgOpen = false)}
  />

  <ConfirmDialog
    open={confirm.open}
    title={confirm.title}
    message={confirm.message}
    details={confirm.details}
    confirmLabel={confirm.confirmLabel}
    requireText={confirm.requireText}
    danger={confirm.danger}
    onConfirm={() => gateDo(confirm)}
    onCancel={() => gateClose(confirm)}
  />

  {#if stack === null && engines === null}
    <div class="flex flex-col gap-sw-2">
      {#each Array(4) as _, i (i)}
        <div class="skeleton" style="height:2.4rem"></div>
      {/each}
    </div>
  {:else}
  <!-- System health (real /health probes of the stack services) -->
  <StackHealthCard {busy} onStart={(id) => onStack?.('start', id)} />

  <!-- Anchor chips (redesign 2C): quick jumps down the long page. -->
  <div class="mb-sw-4 flex flex-wrap gap-sw-2">
    <button class="badge badge-muted chip-btn" onclick={() => jumpTo('sec-services')}>{t('providers.servicesHeading')}</button>
    <button class="badge badge-muted chip-btn" onclick={() => jumpTo('sec-my')}>{t('myProviders.title')}</button>
  </div>

  <!-- B2: unified Services — LLM-stack lifecycle + engine wiring merged into one card per port. -->
  <section class="mb-sw-6" id="sec-services">
    <div class="mb-sw-2 flex items-start justify-between gap-sw-2">
      <button class="flex min-w-0 items-center gap-sw-2 border-0 bg-transparent p-0 text-left" onclick={() => (servicesOpen = !servicesOpen)}>
        <span class="text-sw-text-muted transition-transform" class:rotate-90={servicesOpen}>▸</span>
        <span class="min-w-0">
          <span class="block section-title">{t('providers.servicesHeading')}</span>
          <span class="block text-sw-xs text-sw-text-muted">{t('providers.servicesSub')}</span>
        </span>
      </button>
      <div class="flex shrink-0 flex-wrap justify-end gap-sw-2">
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stackBusy} onclick={() => onStack?.('start')}
          title={t('providers.stackStartTip')}>{stackBusy ? t('providers.busy') : t('providers.stackStartAll')}</button>
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stopBusy} onclick={() => onStack?.('stop')}
          title={t('providers.stackStopTip')}>{stopBusy ? t('providers.busy') : t('providers.stackStopAll')}</button>
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !openableDashboards.length}
          onclick={() => openableDashboards.forEach((s) => onOpenUrl(s.dashboard))}
          title={openableDashboards.length
            ? t('providers.openAllDashboardsTitle', { n: openableDashboards.length })
            : t('providers.openAllDashboardsNoneTitle')}>
          {t('providers.openAllDashboards')}{openableDashboards.length ? ` (${openableDashboards.length})` : ''}
        </button>
      </div>
    </div>
    {#if servicesOpen}
      {#if serviceList.length}
      <div class="card-grid">
        {#each serviceList as svc (svc.key)}
          {@const hc = health['svc:' + svc.port]}
          {@const s = svc.stack}
          {@const e = svc.engine}
          <div class="sw-card flex flex-col gap-sw-2" class:opacity-60={s ? !s.enabled : false}>
            <div class="flex items-start justify-between gap-sw-2">
              <div class="min-w-0">
                <h3 class="line-clamp-2 font-medium leading-tight" title={svc.name}>{svc.name}</h3>
                <p class="truncate font-mono text-[11px] text-sw-text-muted">{svc.baseUrl} · :{svc.port} · {svc.protocol}</p>
                {#if svc.running && s}
                  {@const pr = procByPort.get(svc.port)}
                  {#if pr}
                    <!-- Diagnostics live behind "Details" — the headline stays name/port/state. -->
                    <button class="adv-toggle" onclick={() => (advOpen[svc.key] = !advOpen[svc.key])} aria-expanded={!!advOpen[svc.key]}>
                      {advOpen[svc.key] ? '▾' : '▸'} {t('providers.advanced')}
                    </button>
                    {#if advOpen[svc.key]}
                      <p class="truncate font-mono text-[11px] text-sw-text-muted" title={t('providers.procTitle', { pid: pr.pid })}>
                        PID {pr.pid}{fmtUptime(pr.uptimeSec) ? ` · ${fmtUptime(pr.uptimeSec)}` : ''}
                      </p>
                    {/if}
                  {/if}
                {/if}
              </div>
              <div class="flex shrink-0 flex-col items-end gap-1">
                {#if s && !s.enabled}
                  <span class="badge badge-muted" title={t('providers.svcDisabled')}>{t('providers.svcDisabled')}</span>
                {:else}
                  <span class="badge {svc.running ? 'badge-ok' : 'badge-muted'}"
                    title={svc.running ? t('providers.portListening') : t('providers.portNotResponding')}>
                    {svc.running ? t('providers.running') : t('providers.stopped')}
                  </span>
                {/if}
                {#if e}
                  <span class="badge {svc.protocol === 'anthropic' ? 'badge-info' : 'badge-warn'}"
                    title={svc.protocol === 'anthropic' ? t('providers.protoAnthropicTitle') : t('providers.protoOpenaiTitle')}>
                    {svc.protocol}
                  </span>
                {/if}
                {#if svc.paid}
                  <span class="badge badge-muted" title={t('providers.stackPaidTip')}>💲 {t('providers.stackPaid')}</span>
                {/if}
                {#if e && e.router && e.installed !== null}
                  <span class="badge {e.installed ? 'badge-ok' : 'badge-muted'}"
                    title={e.installed ? t('providers.ccrInstalledTitle') : t('providers.ccrNotInstalledTitle')}>
                    {e.installed ? t('providers.installed') : t('providers.notInstalled')}
                  </span>
                {/if}
                {#if s}
                  <span class="badge badge-muted" title={t('providers.managedTip')}>{t('providers.managed')}</span>
                {/if}
              </div>
            </div>
            {#if e && editId === e.id}
              <div class="rounded-sw-md border border-sw-border p-sw-2">
                <p class="mb-sw-2 text-sw-xs font-medium text-sw-text-secondary">{t('providers.endpointEditorTitle')}</p>
                <div class="flex flex-col gap-sw-2">
                  <input class="sw-input text-sw-xs" bind:value={editUrl} placeholder="http://localhost:1234" spellcheck="false" title={t('providers.editUrlInputTip')} />
                  {#if !editUrlOk}<span class="text-sw-xs text-sw-danger">{t('myProviders.errInvalidUrl')}</span>{/if}
                  <input class="sw-input text-sw-xs" type="number" bind:value={editPort} placeholder={t('providers.portPlaceholder')} title={t('providers.editPortInputTip')} />
                </div>
                <div class="mt-sw-2 flex gap-sw-2">
                  <button class="sw-btn text-sw-xs" disabled={!editUrlOk} onclick={saveEdit} title={t('providers.saveEngineTitle')}>{t('providers.save')}</button>
                  <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (editId = null)} title={t('providers.cancelEditTip')}>{t('common.cancel')}</button>
                </div>
              </div>
            {/if}
            <div class="mt-auto flex flex-wrap items-center gap-sw-2 border-t border-sw-border pt-sw-2">
              {#if s && s.enabled}
                {#if svc.running}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stopBusy} onclick={() => onStack?.('stop', s.id)}
                    title={t('providers.stackStopOneTip', { name: svc.name })}>{t('providers.stop')}</button>
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stackBusy} onclick={() => onStack?.('restart', s.id)}
                    title={t('providers.stackRestartOneTip', { name: svc.name })}>{t('providers.restart')}</button>
                {:else}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stackBusy} onclick={() => onStack?.('start', s.id)}
                    title={t('providers.stackStartOneTip', { name: svc.name })}>{t('providers.start')}</button>
                {/if}
              {/if}
              {#if e}
                {#if e.router && e.installed === false}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={onRouterInstall}
                    title={t('providers.installCcrTitle')}>{t('providers.install')}</button>
                {:else if e.protocol === 'anthropic' && !e.router}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !e.running} onclick={() => openConnect(e)}
                    title={e.running ? t('providers.bindReadyTitle') : t('providers.bindNotReadyTitle')}>
                    {t('providers.bindToProfile')}
                  </button>
                {:else if e.protocol === 'openai' && !e.router}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !e.running} onclick={() => openConnect(e)}
                    title={e.running ? t('providers.connectReadyTitle') : t('providers.connectNotReadyTitle')}>
                    {t('providers.connectViaRouter')}
                  </button>
                {/if}
              {/if}
              <DropdownMenu title={t('providers.moreActions')} items={svcMenu(svc)} />
            </div>
            {#if hc && hc !== 'checking'}
              <p class="text-sw-xs {statusTextClass(hc.ok ? 'ok' : 'bad')}">{hc.ok ? '✓' : '✗'} {hc.detail}</p>
            {/if}
          </div>
        {/each}
      </div>
      {:else}
        <EmptyState icon={Plug} description={t('providers.noEngines')} />
      {/if}
    {/if}
  </section>
  {/if}

  <!-- Provider per profile lives on the Profiles tab (single source of truth) — no duplicate
       controls here, just a jump. -->
  <div class="mb-sw-2 mt-sw-6 flex items-center justify-between gap-sw-2">
    <h2 class="section-title">
      {t('providers.providerPerProfileHeading')}
    </h2>
    {#if onOpenProfiles}
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={onOpenProfiles}
        title={t('providers.openProfilesTip')}>{t('providers.openProfiles')}</button>
    {/if}
  </div>
  <p class="mb-sw-2 text-sw-xs text-sw-text-muted">{t('providers.perProfileMovedNote')}</p>

  <!-- Custom provider registry (own list; keys in Credential Manager) -->
  <div class="mb-sw-2 mt-sw-6 flex items-center justify-between gap-sw-2" id="sec-my">
    <h2 class="section-title">{t('myProviders.title')}</h2>
    <div class="flex shrink-0 items-center gap-sw-2">
      {#if myProviderList.length > 1}
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || checkingAll} onclick={checkAll} title={t('myProviders.checkAllTip')}>
          {checkingAll ? t('common.busy') : t('myProviders.checkAll')}
        </button>
      {/if}
      <button class="sw-btn sw-btn-primary text-sw-xs" disabled={busy} onclick={mpAdd} title={t('myProviders.addTitle')}>
        {t('myProviders.add')}
      </button>
      <DropdownMenu
        title={t('myProviders.moreActions')}
        items={[
          { label: t('myProviders.setLogin'), onClick: toggleLogin },
          { label: t('myProviders.openFreellmapi'), onClick: () => onOpenUrl(freellmapiUrl) }
        ]}
      />
    </div>
  </div>
  <p class="mb-sw-2 text-sw-xs text-sw-text-muted">{t('myProviders.sectionDesc')}</p>

  {#if loginOpen}
    <div class="sw-card mb-sw-3 flex flex-col gap-sw-2">
      <p class="text-sw-xs font-medium text-sw-text-secondary">{t('myProviders.loginTitle')}</p>
      <p class="text-sw-xs text-sw-text-muted">{t('myProviders.loginHint')}</p>
      {#if authStatus}
        <div class="flex flex-wrap items-center gap-sw-2">
          <span class="badge {authStatus.hasEmail ? 'badge-ok' : 'badge-muted'}">{authStatus.hasEmail ? t('myProviders.statusEmail') : t('myProviders.statusNone')}</span>
          {#if authStatus.hasEmail}<button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onDeleteFreellmapiAuth('email')} title={t('myProviders.deleteAuthTip')} aria-label={t('myProviders.deleteEmail')}>×</button>{/if}
          <span class="badge {authStatus.hasPassword ? 'badge-ok' : 'badge-muted'}">{authStatus.hasPassword ? t('myProviders.statusPassword') : t('myProviders.statusNone')}</span>
          {#if authStatus.hasPassword}<button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onDeleteFreellmapiAuth('password')} title={t('myProviders.deleteAuthTip')} aria-label={t('myProviders.deletePassword')}>×</button>{/if}
          <span class="badge {authStatus.hasToken ? 'badge-ok' : 'badge-muted'}">{authStatus.hasToken ? t('myProviders.statusToken') : t('myProviders.statusNone')}</span>
          {#if authStatus.hasToken}<button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onDeleteFreellmapiAuth('token')} title={t('myProviders.deleteAuthTip')} aria-label={t('myProviders.deleteToken')}>×</button>{/if}
        </div>
      {/if}
      <div class="flex gap-sw-2">
        <input class="sw-input flex-1" type="email" bind:value={loginEmail} autocomplete="off" placeholder={t('myProviders.loginEmail')} />
        <input class="sw-input flex-1" type="password" bind:value={loginPassword} autocomplete="off" placeholder={t('myProviders.loginPassword')} />
      </div>
      <input class="sw-input" type="password" bind:value={loginToken} autocomplete="off" placeholder={t('myProviders.loginTokenOpt')} />
      <div class="flex justify-end gap-sw-2">
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (loginOpen = false)}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!loginEmail.trim() && !loginToken.trim()} onclick={saveLogin}>{t('myProviders.save')}</button>
      </div>
    </div>
  {/if}

  {#if myProviderList.length}
    <div class="card-grid">
      {#each myProviderList as p (p.id)}
        {@const openaiDirect = p.connectVia === 'direct' && p.protocol === 'openai'}
        {@const h = health[p.id]}
        {@const bal = balance[p.id]}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div class="min-w-0">
              <h3 class="line-clamp-2 font-medium leading-tight" title={p.name}>{p.name}</h3>
              <p class="truncate font-mono text-[11px] text-sw-text-secondary" title={p.baseUrl}>{p.baseUrl}</p>
            </div>
            <div class="flex shrink-0 flex-col items-end gap-1">
              <span class="badge {p.hasKey ? 'badge-ok' : 'badge-muted'}"
                title={p.hasKey ? t('myProviders.hasKey') : t('myProviders.noKey')}>
                {p.hasKey ? t('myProviders.keySet') : t('myProviders.noKeyShort')}
              </span>
              {#if h === 'checking'}
                <span class="badge badge-muted">{t('myProviders.checking')}</span>
              {:else if h}
                <span class="badge {h.ok ? 'badge-ok' : 'badge-warn'}" title={h.detail}>
                  {h.ok ? t('myProviders.alive') : t('myProviders.dead')}
                </span>
              {/if}
              {#if bal && bal !== 'checking' && bal.ok}
                <span class="badge badge-info" title={t('myProviders.balanceTitle')}>{fmtBalance(bal)}</span>
              {/if}
            </div>
          </div>
          <div class="flex flex-wrap gap-sw-2">
            <span class="badge badge-muted">{p.connectVia === 'freellmapi' ? t('myProviders.viaFreellmapi') : t('myProviders.viaDirect')}</span>
            {#if p.model}<span class="badge badge-muted">{p.model}</span>{/if}
            {#if p.connectVia === 'direct' && p.targetProfile}<span class="badge badge-muted">→ {p.targetProfile}</span>{/if}
            {#if p.keyCount > 1}
              <span class="badge badge-muted" title={t('myProviders.keyPoolTitle')}>
                🔑 {p.activeKey + 1}/{p.keyCount}
              </span>
            {/if}
          </div>
          <div class="mt-auto flex flex-wrap items-center gap-sw-2 border-t border-sw-border pt-sw-2">
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !p.hasKey || openaiDirect}
              onclick={() => onMyProviderConnect(p.id)}
              title={openaiDirect ? t('myProviders.openaiNeedsRouter') : !p.hasKey ? t('myProviders.noKey') : t('myProviders.connectTitle')}>
              {t('myProviders.connect')}
            </button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !p.hasKey || h === 'checking'}
              onclick={() => check(p.id)} title={t('myProviders.checkTitle')}>
              {h === 'checking' ? t('myProviders.checking') : t('myProviders.check')}
            </button>
            <DropdownMenu title={t('providers.moreActions')} items={mpMenu(p, bal === 'checking')} />
          </div>
          {#if bal && bal !== 'checking' && !bal.ok}
            <p class="text-sw-xs text-sw-text-muted">{bal.detail}</p>
          {/if}
          {#if keysOpen[p.id]}
            <div class="flex flex-col gap-sw-2 rounded border border-sw-border bg-sw-bg-secondary p-sw-2">
              <p class="text-sw-xs text-sw-text-muted">{t('myProviders.keysHint')}</p>
              {#if p.hasKey}
                <!-- Show the slot list whenever a key exists; legacy single keys report keyCount 0,
                     so normalize to at least one slot so the lone key is still manageable. -->
                {@const slotCount = Math.max(1, p.keyCount)}
                <ul class="flex flex-col gap-1">
                  {#each Array(slotCount) as _, i (i)}
                    <li class="flex items-center justify-between gap-sw-2 text-sw-xs">
                      <span class={i === p.activeKey ? 'font-medium text-sw-text' : 'text-sw-text-secondary'}>
                        {t('myProviders.keySlot')} {i + 1}{i === p.activeKey ? ` — ${t('myProviders.keyActive')}` : ''}
                      </span>
                      <button class="sw-btn sw-btn-ghost text-[11px]" disabled={busy}
                        onclick={() => confirmRemoveKey(p.id, i)} title={t('myProviders.removeKeyTitle')}>
                        {t('myProviders.removeKey')}
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
              <div class="flex gap-sw-2">
                <input class="sw-input flex-1 text-sw-xs" type="password" autocomplete="off"
                  bind:value={newKey[p.id]} placeholder={t('myProviders.addKeyPlaceholder')} />
                <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!(newKey[p.id] ?? '').trim()}
                  onclick={() => addKey(p.id)}>{t('myProviders.addKey')}</button>
              </div>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <EmptyState icon={Puzzle} description={t('myProviders.empty')} />
  {/if}

  <p class="mt-sw-4 text-sw-xs text-sw-text-muted">
    {t('providers.footnote')}
  </p>
</div>

<style>
  .chip-btn {
    border: 1px solid var(--sw-border);
    background: transparent;
    cursor: pointer;
    font-family: inherit;
  }
  .chip-btn:hover {
    color: var(--sw-text-primary);
    border-color: var(--sw-border-focus);
  }
  .adv-toggle {
    border: none;
    background: transparent;
    padding: 0;
    font-family: inherit;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    cursor: pointer;
    text-align: left;
  }
  .adv-toggle:hover {
    color: var(--sw-text-secondary);
  }
</style>
