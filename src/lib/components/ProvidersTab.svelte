<script lang="ts">
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
  import EmptyState from './EmptyState.svelte';
  import { pushToast } from '$lib/toast.svelte';
  import { statusTextClass } from '$lib/statusColor';
  import MyProviderEditDialog from './MyProviderEditDialog.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import RouterConnectDialog from './RouterConnectDialog.svelte';
  import StackHealthCard from './StackHealthCard.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { Plug, Puzzle } from '@lucide/svelte';
  import SectionHeader from './SectionHeader.svelte';

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
  // Engines that are running AND expose a dashboard → "open all" target.
  const runningDashboards = $derived(engineList.filter((e) => e.running && e.dashboardUrl));
  // LLM-stack services (from stack.json, the single source of truth).
  const stackList = $derived(stack ?? []);

  // Collapsible heavy sections (this screen is long) — open by default.
  let stackOpen = $state(true);
  let enginesOpen = $state(true);
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
  async function saveEdit() {
    if (!editId) return;
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
  let removeKeyTarget = $state<{ id: string; index: number } | null>(null);
  function confirmRemoveKey(id: string, index: number) {
    // R8: honor the global confirm-destructive toggle.
    if (!confirmDestructive) {
      onMyProviderRemoveKey(id, index);
      return;
    }
    removeKeyTarget = { id, index };
  }
  function doRemoveKey() {
    const tgt = removeKeyTarget;
    removeKeyTarget = null;
    if (tgt) onMyProviderRemoveKey(tgt.id, tgt.index);
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
    open={removeKeyTarget !== null}
    title={t('myProviders.removeKey')}
    message={t('myProviders.removeKeyTitle')}
    confirmLabel={t('myProviders.removeKey')}
    danger={true}
    onConfirm={doRemoveKey}
    onCancel={() => (removeKeyTarget = null)}
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
    <button class="badge badge-muted chip-btn" onclick={() => jumpTo('sec-stack')}>{t('providers.stackHeading')}</button>
    <button class="badge badge-muted chip-btn" onclick={() => jumpTo('sec-engines')}>{t('providers.enginesHeading')}</button>
    <button class="badge badge-muted chip-btn" onclick={() => jumpTo('sec-my')}>{t('myProviders.title')}</button>
  </div>

  <!-- LLM stack (single source of truth: stack.json) -->
  {#if stackList.length}
    <section class="mb-sw-6" id="sec-stack">
      <div class="mb-sw-2 flex items-start justify-between gap-sw-2">
        <button class="flex min-w-0 items-center gap-sw-2 border-0 bg-transparent p-0 text-left" onclick={() => (stackOpen = !stackOpen)}>
          <span class="text-sw-text-muted transition-transform" class:rotate-90={stackOpen}>▸</span>
          <span class="min-w-0">
            <span class="block section-title">{t('providers.stackHeading')}</span>
            <span class="block text-sw-xs text-sw-text-muted">{t('providers.stackSub')}</span>
          </span>
        </button>
        <div class="flex shrink-0 gap-sw-2">
          <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stackBusy} onclick={() => onStack?.('start')}
            title={t('providers.stackStartTip')}>{stackBusy ? t('providers.busy') : t('providers.stackStartAll')}</button>
          <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stopBusy} onclick={() => onStack?.('stop')}
            title={t('providers.stackStopTip')}>{stopBusy ? t('providers.busy') : t('providers.stackStopAll')}</button>
        </div>
      </div>
      {#if stackOpen}
      <div class="card-grid">
        {#each stackList as s (s.id)}
          {@const hc = health['stack:' + s.id]}
          <div class="sw-card flex flex-col gap-sw-2" class:opacity-60={!s.enabled}>
            <div class="flex items-start justify-between gap-sw-2">
              <div class="min-w-0">
                <h3 class="truncate font-medium" title={s.name}>{s.name}</h3>
                <p class="truncate font-mono text-[11px] text-sw-text-muted">:{s.port} · {s.protocol}</p>
                {#if s.running}
                  {@const pr = procByPort.get(s.port)}
                  {#if pr}
                    <!-- Diagnostics live behind "Details" — the headline stays name/port/state. -->
                    <button class="adv-toggle" onclick={() => (advOpen[s.id] = !advOpen[s.id])} aria-expanded={!!advOpen[s.id]}>
                      {advOpen[s.id] ? '▾' : '▸'} {t('providers.advanced')}
                    </button>
                    {#if advOpen[s.id]}
                      <p class="truncate font-mono text-[11px] text-sw-text-muted" title={t('providers.procTitle', { pid: pr.pid })}>
                        PID {pr.pid}{fmtUptime(pr.uptimeSec) ? ` · ${fmtUptime(pr.uptimeSec)}` : ''}
                      </p>
                    {/if}
                  {/if}
                {/if}
              </div>
              <div class="flex shrink-0 flex-col items-end gap-1">
                {#if s.enabled}
                  <span class="badge {s.running ? 'badge-ok' : 'badge-muted'}"
                    title={s.running ? t('providers.portListening') : t('providers.portNotResponding')}>
                    {s.running ? t('providers.running') : t('providers.stopped')}
                  </span>
                {:else}
                  <span class="badge badge-muted" title={t('providers.svcDisabled')}>{t('providers.svcDisabled')}</span>
                {/if}
                {#if s.group === 'router'}
                  <span class="badge badge-muted" title={t('providers.stackPaidTip')}>💲 {t('providers.stackPaid')}</span>
                {/if}
              </div>
            </div>
            <div class="mt-auto flex flex-wrap gap-sw-2 border-t border-sw-border pt-sw-2">
              {#if s.enabled}
                {#if s.running}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stopBusy} onclick={() => onStack?.('stop', s.id)}
                    title={t('providers.stackStopOneTip', { name: s.name })}>{t('providers.stop')}</button>
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stackBusy} onclick={() => onStack?.('restart', s.id)}
                    title={t('providers.stackRestartOneTip', { name: s.name })}>{t('providers.restart')}</button>
                {:else}
                  <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={stackBusy} onclick={() => onStack?.('start', s.id)}
                    title={t('providers.stackStartOneTip', { name: s.name })}>{t('providers.start')}</button>
                  <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openStackLog(s.id)}
                    title={t('providers.openLogTip', { name: s.name })}>{t('providers.openLog')}</button>
                {/if}
              {/if}
              {#if s.dashboard}
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!s.running} onclick={() => onOpenUrl(s.dashboard)}
                  title={s.running ? t('providers.openDashboardTitle', { url: s.dashboard }) : t('providers.dashboardWhenRunningTitle')}>
                  {t('providers.dashboard')}
                </button>
              {/if}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || hc === 'checking'} onclick={() => checkUrl('stack:' + s.id, 'http://127.0.0.1:' + s.port, s.protocol)}
                title={t('providers.checkTip')}>
                {hc === 'checking' ? t('common.busy') : t('common.check')}
              </button>
            </div>
            {#if hc && hc !== 'checking'}
              <p class="text-sw-xs {statusTextClass(hc.ok ? 'ok' : 'bad')}">{hc.ok ? '✓' : '✗'} {hc.detail}</p>
            {/if}
          </div>
        {/each}
      </div>
      {/if}
    </section>
  {/if}

  <!-- Engines -->
  <div class="mb-sw-2 flex items-center justify-between gap-sw-2" id="sec-engines">
    <button class="flex min-w-0 items-center gap-sw-2 border-0 bg-transparent p-0 text-left" onclick={() => (enginesOpen = !enginesOpen)}>
      <span class="text-sw-text-muted transition-transform" class:rotate-90={enginesOpen}>▸</span>
      <span class="section-title">{t('providers.enginesHeading')}</span>
    </button>
    {#if engineList.length}
      <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !runningDashboards.length}
        onclick={() => runningDashboards.forEach((e) => onOpenUrl(e.dashboardUrl))}
        title={runningDashboards.length
          ? t('providers.openAllDashboardsTitle', { n: runningDashboards.length })
          : t('providers.openAllDashboardsNoneTitle')}>
        {t('providers.openAllDashboards')}{runningDashboards.length ? ` (${runningDashboards.length})` : ''}
      </button>
    {/if}
  </div>
  {#if enginesOpen}
  <p class="mb-sw-2 text-sw-xs text-sw-text-muted">{t('providers.enginesDesc')}</p>
  {#if engineList.length}
    <div class="card-grid">
      {#each engineList as e (e.id)}
        {@const he = health['engine:' + e.id]}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div class="min-w-0">
              <h3 class="truncate font-medium" title={e.name}>{e.name}</h3>
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
                <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (editId = null)} title={t('providers.cancelEditTip')}>{t('common.cancel')}</button>
              </div>
            </div>
          {/if}

          <div class="mt-auto flex flex-wrap gap-sw-2 border-t border-sw-border pt-sw-2">
            {#if e.router && e.installed === false}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={onRouterInstall}
                title={t('providers.installCcrTitle')}>{t('providers.install')}</button>
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
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || he === 'checking'} onclick={() => checkUrl('engine:' + e.id, e.baseUrl, e.protocol)}
              title={t('providers.checkTip')}>
              {he === 'checking' ? t('common.busy') : t('common.check')}
            </button>
          </div>
          {#if he && he !== 'checking'}
            <p class="text-sw-xs {statusTextClass(he.ok ? 'ok' : 'bad')}">{he.ok ? '✓' : '✗'} {he.detail}</p>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <EmptyState icon={Plug} description={t('providers.noEngines')} />
  {/if}
  {/if}
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
              <h3 class="truncate font-medium" title={p.name}>{p.name}</h3>
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
          <div class="mt-auto flex flex-wrap gap-sw-2 border-t border-sw-border pt-sw-2">
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !p.hasKey || openaiDirect}
              onclick={() => onMyProviderConnect(p.id)}
              title={openaiDirect ? t('myProviders.openaiNeedsRouter') : !p.hasKey ? t('myProviders.noKey') : t('myProviders.connectTitle')}>
              {t('myProviders.connect')}
            </button>
            {#if p.keyCount > 1}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                onclick={() => onMyProviderNextKey(p.id)} title={t('myProviders.nextKeyTitle')}>
                {t('myProviders.nextKey')}
              </button>
            {/if}
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !p.hasKey || h === 'checking'}
              onclick={() => check(p.id)} title={t('myProviders.checkTitle')}>
              {h === 'checking' ? t('myProviders.checking') : t('myProviders.check')}
            </button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !p.hasKey || bal === 'checking'}
              onclick={() => checkBalance(p.id)} title={t('myProviders.balanceTitle')}>
              {bal === 'checking' ? t('myProviders.checking') : t('myProviders.balance')}
            </button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => toggleKeys(p.id)}
              title={t('myProviders.keysTitle')}>
              {t('myProviders.keys')}{p.keyCount > 1 ? ` (${p.keyCount})` : ''}
            </button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => mpEdit(p)}
              title={t('myProviders.editTitle')}>{t('myProviders.edit')}</button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onMyProviderDelete(p.id)}
              title={t('myProviders.deleteTitle')}>{t('myProviders.delete')}</button>
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
