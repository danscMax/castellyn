<script lang="ts">
  import type { McpStatus } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import DataTable, { type DTColumn } from './DataTable.svelte';

  let {
    data,
    running,
    onRefresh,
    onDeploy
  }: {
    data: McpStatus | null;
    running: string | null;
    onRefresh: () => void;
    onDeploy: (target?: string | string[]) => void;
  } = $props();

  const busy = $derived(!!running);
  // Bulk MCP deploy (#76): pick profiles, deploy to all of them in one run.
  let bulkSel = $state<Record<string, boolean>>({});
  const bulkCount = $derived(Object.values(bulkSel).filter(Boolean).length);
  function toggleBulk(p: string) {
    bulkSel = { ...bulkSel, [p]: !bulkSel[p] };
  }
  function deployBulk() {
    const only = ALL_PROFILES.filter((p) => bulkSel[p]);
    if (only.length) onDeploy(only);
  }
  // Real profile list from the backend (read_mcp); falls back to the canonical set on first paint
  // so the n/total badge and the per-profile chips never lie when profiles are added/removed.
  const ALL_PROFILES = $derived(
    data?.profiles?.length ? data.profiles : ['ccmy', 'cc1', 'cc2', 'cc3', 'cc4', 'cc5']
  );
  // Provided by the plugin marketplace, not deployed per-profile (installer skips them).
  const PLUGIN_PROVIDED = ['context7', 'serena'];

  const source = $derived(data?.source ?? []);
  const extras = $derived(data?.extras ?? []);

  function isPlugin(name: string) {
    return PLUGIN_PROVIDED.includes(name);
  }

  // Surface servers that still need attention (deployed to fewer profiles than exist) first;
  // fully-deployed next; plugin-provided (not deployable) last.
  function rank(srv: { name: string; deployedIn: string[] }): number {
    if (isPlugin(srv.name)) return 2;
    return srv.deployedIn.length < ALL_PROFILES.length ? 0 : 1;
  }
  const sortedSource = $derived([...source].sort((a, b) => rank(a) - rank(b)));

  const COLS: DTColumn[] = [
    { key: 'name', label: t('mcp.colName'), grow: true, sortable: true },
    { key: 'command', label: t('mcp.colCommand'), width: '300px' },
    { key: 'deployed', label: t('mcp.colDeployed'), width: '100px', align: 'center', sortable: true },
    { key: 'profiles', label: t('mcp.colProfiles'), width: '240px', interactive: true }
  ];
  type Srv = (typeof sortedSource)[number];
  function sortVal(s: Srv, key: string): string | number {
    if (key === 'deployed') return rank(s) * 100 + s.deployedIn.length;
    return s.name.toLowerCase();
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('mcp.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('mcp.subtitle')}
      </p>
    </div>
    <div class="flex shrink-0 gap-sw-2">
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={onRefresh}
        title={t('mcp.refreshTitle')}>
        {running === 'mcp' ? t('common.busy') : t('common.refresh')}
      </button>
      <button class="sw-btn sw-btn-primary" disabled={busy} onclick={() => onDeploy()}
        title={t('mcp.deployTitle')}>
        {t('mcp.deployAll')}
      </button>
    </div>
  </header>

  {#if source.length}
    <DataTable
      columns={COLS}
      rows={sortedSource}
      rowKey={(s) => s.name}
      sortAccessor={sortVal}
      search
      searchValue={(s) => `${s.name} ${s.command}`}
      searchPlaceholder={t('mcp.colName')}
      storageKey="mcp"
    >
      {#snippet toolbar()}
        <span class="text-sw-xs text-sw-text-muted">{t('mcp.selectProfiles')}</span>
        {#each ALL_PROFILES as p (p)}
          <button class="badge {bulkSel[p] ? 'badge-info' : 'badge-muted'}" onclick={() => toggleBulk(p)}
            title={t('mcp.selectProfileTip', { p })}>{p}</button>
        {/each}
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !bulkCount} onclick={deployBulk}
          title={t('mcp.bulkDeployTip')}>
          {t('mcp.bulkDeploy')}{bulkCount ? ` (${bulkCount})` : ''}
        </button>
      {/snippet}

      {#snippet cell(srv, col)}
        {#if col.key === 'name'}
          <span class="font-medium truncate" title={srv.name}>{srv.name}</span>
        {:else if col.key === 'command'}
          <span class="font-mono text-sw-xs text-sw-text-muted truncate block" title={srv.command}>{srv.command}</span>
        {:else if col.key === 'deployed'}
          {#if isPlugin(srv.name)}
            <span class="badge badge-info" title={t('mcp.pluginBadgeTitle')}>{t('mcp.pluginBadge')}</span>
          {:else}
            <span class="badge {srv.deployedIn.length === ALL_PROFILES.length ? 'badge-ok' : srv.deployedIn.length > 0 ? 'badge-warn' : 'badge-err'}"
              title={t('mcp.deployedCountTitle', { n: srv.deployedIn.length, total: ALL_PROFILES.length })}>
              {srv.deployedIn.length}/{ALL_PROFILES.length}
            </span>
          {/if}
        {:else if col.key === 'profiles'}
          {#if isPlugin(srv.name)}
            <span class="text-sw-xs text-sw-text-muted">{t('mcp.pluginNote')}</span>
          {:else}
            <div class="flex flex-wrap gap-sw-1">
              {#each ALL_PROFILES as p (p)}
                {@const ok = srv.deployedIn.includes(p)}
                {#if ok}
                  <span class="badge badge-ok" title={t('mcp.profileDeployedTitle', { p })}>{p}</span>
                {:else}
                  <button class="badge badge-muted" disabled={busy} onclick={() => onDeploy(p)}
                    title={t('mcp.deployToProfileTip', { p })}>{p}</button>
                {/if}
              {/each}
            </div>
          {/if}
        {/if}
      {/snippet}
    </DataTable>
  {:else}
    <div class="grid place-items-center py-sw-6 text-center text-sw-text-muted">
      <div>
        <div class="mb-sw-2 text-2xl">⧉</div>
        <div class="font-medium text-sw-text">{t('mcp.emptyTitle')}</div>
        <div class="text-sw-sm">{t('mcp.emptyHint')}</div>
      </div>
    </div>
  {/if}

  {#if extras.length}
    <h2 class="mb-sw-2 mt-sw-6 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">
      {t('mcp.extrasHeading')}
    </h2>
    <div class="sw-card flex flex-col gap-sw-2">
      <p class="text-sw-xs text-sw-text-muted">
        {t('mcp.extrasNote')}
      </p>
      {#each extras as ex (ex.name)}
        <div class="flex items-center justify-between gap-sw-2 text-sw-sm">
          <span class="font-mono text-sw-text">{ex.name}</span>
          <div class="flex flex-wrap gap-sw-2">
            {#each ex.presentIn as p (p)}<span class="badge badge-warn" title={t('mcp.extrasProfileTitle')}>{p}</span>{/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>
