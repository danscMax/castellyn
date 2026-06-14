<script lang="ts">
  import type { McpStatus } from '$lib/ipc';
  import { t } from '$lib/i18n';

  let {
    data,
    running,
    onRefresh,
    onDeploy
  }: {
    data: McpStatus | null;
    running: string | null;
    onRefresh: () => void;
    onDeploy: () => void;
  } = $props();

  const busy = $derived(!!running);
  const ALL_PROFILES = ['ccmy', 'cc1', 'cc2', 'cc3', 'cc4', 'cc5'];
  // Provided by the plugin marketplace, not deployed per-profile (installer skips them).
  const PLUGIN_PROVIDED = ['context7', 'serena'];

  const source = $derived(data?.source ?? []);
  const extras = $derived(data?.extras ?? []);

  function isPlugin(name: string) {
    return PLUGIN_PROVIDED.includes(name);
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
      <button class="sw-btn sw-btn-primary" disabled={busy} onclick={onDeploy}
        title={t('mcp.deployTitle')}>
        {t('mcp.deployAll')}
      </button>
    </div>
  </header>

  {#if source.length}
    <div class="card-grid">
      {#each source as srv (srv.name)}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div class="min-w-0">
              <h3 class="font-medium">{srv.name}</h3>
              <p class="truncate font-mono text-sw-xs text-sw-text-muted" title={t('mcp.commandTitle')}>{srv.command}</p>
            </div>
            {#if isPlugin(srv.name)}
              <span class="badge badge-info shrink-0" title={t('mcp.pluginBadgeTitle')}>{t('mcp.pluginBadge')}</span>
            {:else}
              <span class="badge {srv.deployedIn.length === ALL_PROFILES.length ? 'badge-ok' : srv.deployedIn.length > 0 ? 'badge-warn' : 'badge-err'} shrink-0"
                title={t('mcp.deployedCountTitle', { n: srv.deployedIn.length, total: ALL_PROFILES.length })}>
                {srv.deployedIn.length}/{ALL_PROFILES.length}
              </span>
            {/if}
          </div>

          {#if isPlugin(srv.name)}
            <p class="text-sw-xs text-sw-text-muted">
              {t('mcp.pluginNote')}
            </p>
          {:else}
            <div class="flex flex-wrap gap-sw-2">
              {#each ALL_PROFILES as p (p)}
                {@const ok = srv.deployedIn.includes(p)}
                <span class="badge {ok ? 'badge-ok' : 'badge-muted'}"
                  title={ok
                    ? t('mcp.profileDeployedTitle', { p })
                    : t('mcp.profileNotDeployedTitle', { p })}>{p}</span>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
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
