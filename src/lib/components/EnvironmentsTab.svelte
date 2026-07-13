<script lang="ts">
  import type { EnvInfo, SkillRow } from '$lib/ipc';
  import EmptyState from './EmptyState.svelte';
  import { t } from '$lib/i18n';
  import Toggle from './Toggle.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';
  import Segmented from './Segmented.svelte';
  import { Compass, Check, X } from '@lucide/svelte';

  let {
    data,
    running,
    matrix,
    onRefresh,
    onShare,
    onRtk,
    onOpenConfig,
    onOpenProviders,
    onOpenMcp,
    onDeployMcp,
    onDeployProviders,
    onDeployInstructions,
    onConnectOmniroute,
    onOpenUrl,
    onLoadMatrix
  }: {
    data: EnvInfo[] | null;
    running: string | null;
    matrix: SkillRow[] | null;
    onRefresh: () => void;
    onShare: () => void;
    onRtk: (id: string, enable: boolean) => void;
    onOpenConfig: (path: string) => void;
    onOpenProviders: () => void;
    onOpenMcp: () => void;
    onDeployMcp: (id: string) => void;
    onDeployProviders: (id: string) => void;
    onDeployInstructions: (id: string) => void;
    onConnectOmniroute: () => void;
    onOpenUrl: (url: string) => void;
    onLoadMatrix: () => void;
  } = $props();

  const busy = $derived(!!running);
  const envs = $derived(data ?? []);
  // Show the share action only when a harness has a gap sharing can actually close (#1) — not the
  // permanent residual (skills that live only in OpenCode/Codex and can't be pushed into Claude).
  const hasGap = $derived(envs.some((e) => e.installed && e.shareableGap > 0));

  // Install docs per harness for the not-installed empty state (#17).
  const INSTALL_URL: Record<string, string> = {
    claude: 'https://docs.claude.com/en/docs/claude-code',
    opencode: 'https://opencode.ai/docs/',
    codex: 'https://developers.openai.com/codex',
    zcode: 'https://zcode.z.ai/en/docs/welcome'
  };

  let view = $state<'cards' | 'table'>('cards');
  function showTable() {
    view = 'table';
    if (matrix === null) onLoadMatrix();
  }

  // Skills badge colour keys off the *closable* gap, so a harness with only the unshareable residual
  // still reads green (#18) instead of a misleading amber.
  const skillsClass = (e: EnvInfo) => (e.shareableGap > 0 ? 'badge-warn' : 'badge-ok');
  // Only OpenCode's RTK is actionable from here today; Claude's is its own hook, Codex has no path.
  const canRtk = (e: EnvInfo) => e.id === 'opencode' && e.rtkAvailable;

  const COLS: DTColumn[] = $derived([
    { key: 'name', label: t('environments.colSkill'), grow: true, sortable: true },
    { key: 'claude', label: 'Claude', width: '110px', align: 'center' },
    { key: 'opencode', label: 'OpenCode', width: '110px', align: 'center' },
    { key: 'codex', label: 'Codex', width: '110px', align: 'center' }
  ]);
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('environments.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('environments.subtitle')}</p>
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      <!-- Cards / Table view switch (#20) -->
      <Segmented
        compact
        value={view}
        options={[
          { value: 'cards', label: t('environments.viewCards') },
          { value: 'table', label: t('environments.viewTable') }
        ]}
        onChange={(v) => (v === 'table' ? showTable() : (view = 'cards'))}
        ariaLabel={t('environments.viewCards')}
      />
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={onRefresh}
        title={t('environments.refreshTitle')}>
        {running === 'envs' ? t('common.busy') : t('common.refresh')}
      </button>
      {#if data !== null && hasGap}
        <button class="sw-btn sw-btn-primary" disabled={busy} onclick={onShare}
          title={t('environments.shareTitle')}>
          {t('environments.shareBtn')}
        </button>
      {:else if data !== null}
        <span class="badge badge-ok" title={t('environments.skillsSyncedTip')}>{t('environments.skillsSynced')}</span>
      {/if}
    </div>
  </header>

  {#if data === null}
    <div class="grid gap-sw-3 md:grid-cols-2">
      {#each Array(4) as _, i (i)}
        <div class="skeleton" style="height:8rem;width:100%"></div>
      {/each}
    </div>
  {:else if view === 'table'}
    <!-- Per-skill diff across harnesses (#20) -->
    {#if matrix === null}
      <div class="flex flex-col gap-sw-2">
        {#each Array(6) as _, i (i)}<div class="skeleton" style="height:2.2rem;width:100%"></div>{/each}
      </div>
    {:else}
      <DataTable
        columns={COLS}
        rows={matrix}
        rowKey={(r) => r.name}
        sortAccessor={(r, k) => (k === 'name' ? r.name.toLowerCase() : r[k] ? 0 : 1)}
        rowAccent={(r) => r.shareable}
        search
        searchValue={(r) => r.name}
        searchPlaceholder={t('environments.colSkill')}
        storageKey="env-skill-matrix"
      >
        {#snippet cell(r: SkillRow, col: DTColumn)}
          {#if col.key === 'name'}
            <span class="font-medium truncate" title={r.name}>{r.name}</span>
            {#if r.shareable}<span class="badge badge-warn ml-sw-2 text-sw-xs">{t('environments.matrixShareable')}</span>{/if}
          {:else}
            {@const ok = (r as any)[col.key] as boolean}
            <span class="badge {ok ? 'badge-ok' : 'badge-muted'}">{#if ok}<Check size={12} aria-hidden="true" />{:else}<X size={12} aria-hidden="true" />{/if}</span>
          {/if}
        {/snippet}
      </DataTable>
    {/if}
  {:else if envs.length}
    <div class="grid gap-sw-3 md:grid-cols-2">
      {#each envs as e (e.id)}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-center justify-between gap-sw-2">
            <span class="text-base font-semibold">{e.name}</span>
            {#if e.installed}
              <span class="badge badge-ok">{t('environments.installed')}</span>
            {:else}
              <span class="badge badge-muted">{t('environments.notInstalled')}</span>
            {/if}
          </div>

          {#if e.installed}
            <!-- Metric strip (#13): packs Skills · Providers · MCP · RTK into one dense band -->
            <div class="grid grid-cols-2 gap-sw-3 md:grid-cols-4">
              <!-- Skills -->
              <div class="flex flex-col gap-sw-1">
                <span class="text-sw-xs text-sw-text-muted">{t('environments.skills')}</span>
                <div class="flex items-center gap-sw-1">
                  <span class="badge {skillsClass(e)}" title={t('environments.skillsTip', { n: e.skillsVisible, total: e.totalSkills })}>
                    {e.skillsVisible}/{e.totalSkills}
                  </span>
                  <span class={e.pluginSkillsVisible ? 'text-sw-text-secondary' : 'text-sw-text-muted'}
                    title={e.pluginSkillsVisible ? t('environments.pluginSkillsOk') : t('environments.pluginSkillsMissing')}>
                    {#if e.pluginSkillsVisible}<Check size={12} aria-hidden="true" />{:else}<X size={12} aria-hidden="true" />{/if}
                  </span>
                </div>
              </div>

              <!-- Providers (#15) -->
              <div class="flex flex-col gap-sw-1">
                <span class="text-sw-xs text-sw-text-muted">{t('environments.providers')}</span>
                {#if e.providers > 0}
                  <button class="badge badge-info w-fit" onclick={onOpenProviders} title={t('environments.openProvidersTip')}>{e.providers}</button>
                {:else}
                  <button class="badge badge-muted w-fit" onclick={onOpenProviders} title={t('environments.addProviderTip')}>{t('environments.none')}</button>
                {/if}
              </div>

              <!-- MCP (#19) + fan-out to OpenCode -->
              <div class="flex flex-col gap-sw-1">
                <span class="text-sw-xs text-sw-text-muted">{t('environments.mcp')}</span>
                <div class="flex items-center gap-sw-2">
                  {#if e.mcpServers > 0}
                    <button class="badge badge-info" onclick={onOpenMcp} title={t('environments.openMcpTip')}>{e.mcpServers}</button>
                  {:else}
                    <button class="badge badge-muted" onclick={onOpenMcp} title={t('environments.openMcpTip')}>{t('environments.none')}</button>
                  {/if}
                  {#if e.mcpDrift}
                    <span class="badge badge-warn" title={t('environments.mcpDriftTip')}>{t('environments.mcpDrift')}</span>
                  {/if}
                </div>
              </div>

              <!-- RTK (#16): one control vocabulary for every harness -->
              <div class="flex flex-col gap-sw-1">
                <span class="text-sw-xs text-sw-text-muted">{t('environments.rtk')}</span>
                <div class="flex items-center gap-sw-2">
                  <Toggle checked={e.rtk} disabled={!canRtk(e) || busy}
                    title={canRtk(e)
                      ? (e.rtk ? t('environments.rtkDisableTitle') : t('environments.rtkEnableTitle'))
                      : e.id === 'claude'
                        ? t('environments.rtkClaudeTip')
                        : t('environments.rtkNaTip')}
                    onCheckedChange={(v) => onRtk(e.id, v)} />
                  <span class="text-sw-xs text-sw-text-muted">
                    {!e.rtkAvailable ? t('environments.rtkNa') : e.rtk ? t('environments.rtkOn') : t('environments.rtkOff')}
                  </span>
                </div>
              </div>
            </div>

            <!-- V7: card actions live BELOW the metric strip — an inline button used to inflate
                 the MCP column and misalign the 4-column grid against sibling cards -->
            {#if e.id === 'opencode'}
              <div class="flex flex-wrap gap-sw-2">
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                  onclick={() => onDeployMcp(e.id)} title={t('environments.deployMcpTitle')}>{t('environments.deployMcp')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                  onclick={() => onDeployProviders(e.id)} title={t('environments.deployProvidersTitle')}>{t('environments.deployProviders')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                  onclick={() => onDeployInstructions(e.id)} title={t('environments.deployInstrTitle')}>{t('environments.deployInstr')}</button>
              </div>
            {:else if e.id === 'codex'}
              <div class="flex flex-wrap gap-sw-2">
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                  onclick={() => onDeployMcp(e.id)} title={t('environments.deployMcpTitleCodex')}>{t('environments.deployMcp')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                  onclick={() => onDeployProviders(e.id)} title={t('environments.connectGatewayTitle')}>{t('environments.connectGateway')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy}
                  onclick={onConnectOmniroute} title={t('environments.connectOmnirouteTitle')}>{t('environments.connectOmniroute')}</button>
              </div>
            {:else if e.id === 'claude'}
              <!-- C2: the home harness has no deploy actions of its own — say so instead of leaving the
                   card looking half-empty next to OpenCode/Codex. -->
              <p class="text-sw-xs text-sw-text-muted">{t('environments.claudeHomeNote')}</p>
            {/if}

            {#if !e.configOk}
              <span class="badge badge-warn w-fit" title={e.configPath}>{t('environments.errorRead')}</span>
            {/if}

            {#if e.configPath}
              <div class="flex items-center justify-between gap-sw-2 border-t border-sw-border pt-sw-2">
                <span class="text-sw-xs text-sw-text-muted">{t('environments.configLabel')}</span>
                <button class="font-mono text-sw-xs text-sw-text-secondary hover:text-sw-text truncate max-w-[70%]"
                  onclick={() => onOpenConfig(e.configPath)} title={`${e.configPath} — ${t('environments.openConfigTitle')}`}>{e.configPath}</button>
              </div>
            {/if}
          {:else}
            <!-- Not installed (#17): an actionable empty state, not a dimmed "broken" card -->
            <div class="flex flex-col items-start gap-sw-2 py-sw-2">
              <p class="text-sw-sm text-sw-text-muted">{t('environments.notInstalledHint')}</p>
              {#if INSTALL_URL[e.id]}
                <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onOpenUrl(INSTALL_URL[e.id])}>
                  {t('environments.installHow')} ↗
                </button>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    <EmptyState icon={Compass} title={t('environments.emptyTitle')} description={t('environments.emptyHint')} />
  {/if}
</div>
