<script lang="ts">
  import type {
    PluginInfo,
    SkillInfo,
    PluginAction,
    PluginUpdate,
    PluginContents
  } from '$lib/ipc';
  import { t } from '$lib/i18n';

  let {
    plugins,
    skills,
    updates = [],
    contents = [],
    running,
    onAction,
    onRefresh,
    onOpenSkills
  }: {
    plugins: PluginInfo[] | null;
    skills: SkillInfo[] | null;
    updates?: PluginUpdate[];
    contents?: PluginContents[];
    running: string | null;
    onAction: (action: PluginAction, id: string) => void;
    onRefresh: () => void;
    onOpenSkills: () => void;
  } = $props();

  const busy = $derived(!!running);
  const pluginList = $derived(plugins ?? []);
  const skillList = $derived(skills ?? []);
  const updateMap = $derived(new Map(updates.map((u) => [u.id, u.available])));
  const contentMap = $derived(new Map(contents.map((c) => [c.id, c])));

  function split(id: string): { name: string; market: string } {
    const i = id.lastIndexOf('@');
    return i > 0 ? { name: id.slice(0, i), market: id.slice(i + 1) } : { name: id, market: '' };
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('plugins.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('plugins.subtitle')}</p>
    </div>
    <button class="sw-btn sw-btn-ghost shrink-0" disabled={busy} onclick={onRefresh}
      title={t('plugins.refreshTip')}>
      {running === 'plugin-mgr' ? t('plugins.refreshing') : t('plugins.refreshBtn')}
    </button>
  </header>

  <!-- Plugins -->
  <h2 class="mb-sw-2 flex items-center gap-sw-2 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">
    {t('plugins.pluginsHeading', { count: pluginList.length })}
    {#if updates.length}<span class="badge badge-info" title={t('plugins.withUpdateBadgeTip')}>{t('plugins.withUpdateBadge', { count: updates.length })}</span>{/if}
  </h2>
  {#if pluginList.length}
    <div class="card-grid">
      {#each pluginList as p (p.id)}
        {@const s = split(p.id)}
        {@const c = contentMap.get(p.id)}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div class="min-w-0">
              <h3 class="truncate font-medium">{s.name}</h3>
              <p class="truncate text-sw-xs text-sw-text-muted">
                {s.market}{p.version && p.version !== 'unknown' ? ` · v${p.version}` : ''}
              </p>
            </div>
            <div class="flex shrink-0 flex-wrap items-center justify-end gap-sw-2">
              {#if updateMap.has(p.id)}<span class="badge badge-info" title={t('plugins.updateAvailableBadgeTip', { version: updateMap.get(p.id) ?? '' })}>{t('plugins.updateAvailableBadge')}</span>{/if}
              {#if p.scope === 'managed'}<span class="badge badge-muted" title={t('plugins.managedBadgeTip')}>{t('plugins.managedBadge')}</span>{/if}
              <span class="badge {p.enabled ? 'badge-ok' : 'badge-warn'}" title={p.enabled ? t('plugins.enabledTip') : t('plugins.disabledTip')}>
                {p.enabled ? t('plugins.enabledBadge') : t('plugins.disabledBadge')}
              </span>
            </div>
          </div>
          {#if c && (c.skills.length || c.commands.length || c.agents.length)}
            <details class="group rounded-sw-md border border-sw-border">
              <summary class="flex cursor-pointer list-none items-center gap-sw-2 px-sw-2 py-1 text-sw-xs text-sw-text-secondary"
                title={t('plugins.contentsToggleTip')}>
                <span class="transition-transform group-open:rotate-90">▸</span>
                <span>{t('plugins.contentsLabel')}</span>
                {#if c.skills.length}<span class="badge badge-muted" title={t('plugins.skillsBadgeTip')}>{t('plugins.skillsBadge', { count: c.skills.length })}</span>{/if}
                {#if c.commands.length}<span class="badge badge-muted" title={t('plugins.commandsBadgeTip')}>{t('plugins.commandsBadge', { count: c.commands.length })}</span>{/if}
                {#if c.agents.length}<span class="badge badge-muted" title={t('plugins.agentsBadgeTip')}>{t('plugins.agentsBadge', { count: c.agents.length })}</span>{/if}
              </summary>
              <div class="flex flex-col gap-sw-2 border-t border-sw-border px-sw-2 py-sw-2">
                {#each [{ label: t('plugins.catSkills'), items: c.skills }, { label: t('plugins.catCommands'), items: c.commands }, { label: t('plugins.catAgents'), items: c.agents }] as cat (cat.label)}
                  {#if cat.items.length}
                    <div>
                      <p class="mb-1 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{cat.label}</p>
                      <div class="flex flex-wrap gap-1">
                        {#each cat.items as item (item)}
                          <span class="rounded bg-sw-bg-secondary px-1.5 py-0.5 font-mono text-[11px] text-sw-text-secondary">{item}</span>
                        {/each}
                      </div>
                    </div>
                  {/if}
                {/each}
              </div>
            </details>
          {/if}
          <div class="mt-auto flex flex-wrap items-center gap-sw-2 border-t border-sw-border pt-sw-2">
            {#if updateMap.has(p.id)}
              <button class="sw-btn sw-btn-primary text-sw-xs" disabled={busy} onclick={() => onAction('update', p.id)}
                title={t('plugins.updateBtnTip', { version: updateMap.get(p.id) ?? '' })}>{t('plugins.updateBtn')}</button>
            {:else}
              <span class="text-sw-xs text-sw-text-muted" title={t('plugins.upToDateTip')}>{t('plugins.upToDate')}</span>
            {/if}
            {#if p.enabled}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction('disable', p.id)}
                title={t('plugins.disableBtnTip')}>{t('plugins.disableBtn')}</button>
            {:else}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction('enable', p.id)}
                title={t('plugins.enableBtnTip')}>{t('plugins.enableBtn')}</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="sw-card text-sw-sm text-sw-text-muted">{t('plugins.noPlugins')}</div>
  {/if}

  <!-- Skills -->
  <div class="mb-sw-2 mt-sw-6 flex items-center justify-between">
    <h2 class="text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">
      {t('plugins.skillsHeading', { count: skillList.length })}
    </h2>
    {#if skillList.length}
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={onOpenSkills}
        title={t('plugins.openSkillsTip')}>{t('plugins.openSkillsBtn')}</button>
    {/if}
  </div>
  <p class="mb-sw-2 text-sw-xs text-sw-text-muted">
    {t('plugins.skillsNote')}
  </p>
  {#if skillList.length}
    <div class="grid grid-cols-1 gap-sw-2 md:grid-cols-2">
      {#each skillList as sk (sk.dir)}
        <div class="sw-card py-sw-2">
          <div class="flex items-center gap-sw-2">
            <span class="truncate font-medium">{sk.name}</span>
            {#if sk.version}<span class="badge badge-muted shrink-0">v{sk.version}</span>{/if}
          </div>
          {#if sk.description}<p class="mt-1 text-sw-xs text-sw-text-secondary">{sk.description}</p>{/if}
        </div>
      {/each}
    </div>
  {:else}
    <div class="sw-card text-sw-sm text-sw-text-muted">{t('plugins.noSkills')}</div>
  {/if}
</div>
