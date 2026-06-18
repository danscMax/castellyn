<script lang="ts">
  import type { PluginInfo, SkillInfo, PluginAction, PluginUpdate, PluginContents } from '$lib/ipc';
  import { t, pSkill, pCommand, pAgent } from '$lib/i18n';
  import Toggle from './Toggle.svelte';
  import Spinner from './Spinner.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';

  let {
    plugins,
    skills,
    updates = [],
    contents = [],
    running,
    onAction,
    onBulkPlugin,
    onRefresh,
    onOpenSkills,
    onOpenSkill,
    onDeleteSkill
  }: {
    plugins: PluginInfo[] | null;
    skills: SkillInfo[] | null;
    updates?: PluginUpdate[];
    contents?: PluginContents[];
    running: string | null;
    onAction: (action: PluginAction, id: string) => void;
    onBulkPlugin: (action: PluginAction, ids: string[]) => void;
    onRefresh: () => void;
    onOpenSkills: () => void;
    onOpenSkill: (dir: string) => void;
    onDeleteSkill: (dir: string, name: string) => void;
  } = $props();

  const busy = $derived(!!running);
  let actingId = $state<string | null>(null);
  $effect(() => {
    if (!running) actingId = null;
  });
  function act(action: PluginAction, id: string) {
    actingId = id;
    onAction(action, id);
  }

  const pluginList = $derived(plugins ?? []);
  const skillList = $derived(skills ?? []);
  const updateMap = $derived(new Map(updates.map((u) => [u.id, u.available])));
  const contentMap = $derived(new Map(contents.map((c) => [c.id, c])));

  function split(id: string): { name: string; market: string } {
    const i = id.lastIndexOf('@');
    return i > 0 ? { name: id.slice(0, i), market: id.slice(i + 1) } : { name: id, market: '' };
  }
  const MARKET_SHORT: Record<string, string> = {
    'claude-plugins-official': 'official',
    'claude-code-workflows': 'workflows'
  };
  function shortMarket(m: string): string {
    if (!m) return '';
    return MARKET_SHORT[m] ?? m.replace(/-marketplace$/, '').replace(/^claude-/, '');
  }
  function hasContents(id: string): boolean {
    const c = contentMap.get(id);
    return !!c && !!(c.skills.length || c.commands.length || c.agents.length);
  }
  // Stable-ish accent colour for the name avatar.
  const AV = ['#60a5fa', '#a78bfa', '#34d399', '#f59e0b', '#f472b6', '#22d3ee', '#fb7185', '#4ade80'];
  function avatar(name: string): { ch: string; color: string } {
    let h = 0;
    for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
    return { ch: name.slice(0, 2).toUpperCase(), color: AV[h % AV.length] };
  }

  // Toolbar filters (persisted). Search + sort live inside DataTable.
  let onlyUpdates = $state(false);
  let onlyEnabled = $state(false);
  try {
    onlyUpdates = localStorage.getItem('cmh-plugins-only-updates') === '1';
    onlyEnabled = localStorage.getItem('cmh-plugins-only-enabled') === '1';
  } catch {
    /* ignore */
  }
  $effect(() => {
    try {
      localStorage.setItem('cmh-plugins-only-updates', onlyUpdates ? '1' : '0');
      localStorage.setItem('cmh-plugins-only-enabled', onlyEnabled ? '1' : '0');
    } catch {
      /* ignore */
    }
  });

  const pluginRows = $derived(
    pluginList.filter((p) => (!onlyUpdates || updateMap.has(p.id)) && (!onlyEnabled || p.enabled))
  );
  const disabledCount = $derived(pluginList.filter((p) => !p.enabled).length);
  const updateIds = $derived(pluginList.filter((p) => updateMap.has(p.id)).map((p) => p.id));

  // Skill category for the filter: own (yours — symlinked OR from your local marketplace) /
  // plugin (third-party) / default (plain dir).
  function skillKind(s: SkillInfo): 'own' | 'plugin' | 'default' {
    if (s.mine) return 'own';
    if (s.source.startsWith('plugin:')) return 'plugin';
    return 'default';
  }
  function pluginNameOf(source: string): string {
    const id = source.slice('plugin:'.length);
    const at = id.lastIndexOf('@');
    return at > 0 ? id.slice(0, at) : id;
  }
  // Badge label: symlinked → "свой"; from a plugin → that plugin's name; plain → "дефолт".
  function sourceLabel(s: SkillInfo): string {
    if (s.source.startsWith('plugin:')) return pluginNameOf(s.source);
    if (s.source === 'own') return t('plugins.sourceOwn');
    return t('plugins.sourceDefault');
  }
  let skillSource = $state<'all' | 'own' | 'plugin' | 'default'>('all');
  const skillRows = $derived(
    skillSource === 'all' ? skillList : skillList.filter((s) => skillKind(s) === skillSource)
  );
  const ownSkillCount = $derived(skillList.filter((s) => s.mine).length);
  const pluginSkillCount = $derived(
    skillList.filter((s) => s.source.startsWith('plugin:') && !s.mine).length
  );

  const PLUGIN_COLS: DTColumn[] = $derived([
    { key: 'name', label: t('plugins.colName'), sortable: true, grow: true },
    { key: 'market', label: t('plugins.colMarket'), sortable: true, width: '140px' },
    { key: 'version', label: t('plugins.colVersion'), sortable: true, width: '130px' },
    { key: 'desc', label: t('plugins.skillColDesc'), width: '300px' },
    { key: 'contents', label: t('plugins.colContents'), width: '140px' },
    { key: 'status', label: t('plugins.colStatus'), sortable: true, align: 'center', width: '74px', interactive: true },
    { key: 'actions', label: '', align: 'right', width: '90px', interactive: true }
  ]);
  function pluginSort(p: PluginInfo, key: string): string | number {
    if (key === 'name') return split(p.id).name.toLowerCase();
    if (key === 'market') return shortMarket(split(p.id).market).toLowerCase();
    if (key === 'version') return p.version ?? '';
    if (key === 'status') return p.enabled ? 1 : 0;
    return '';
  }

  const SKILL_COLS: DTColumn[] = $derived([
    { key: 'name', label: t('plugins.colName'), sortable: true, width: '230px' },
    { key: 'source', label: t('plugins.colSource'), sortable: true, width: '150px' },
    { key: 'version', label: t('plugins.colVersion'), sortable: true, width: '90px' },
    { key: 'desc', label: t('plugins.skillColDesc'), grow: true },
    { key: 'actions', label: '', align: 'right', width: '110px', interactive: true }
  ]);
  const SRANK: Record<string, number> = { own: 0, default: 1, plugin: 2 };
  function skillSort(s: SkillInfo, key: string): string | number {
    if (key === 'name') return s.name.toLowerCase();
    if (key === 'version') return s.version ?? '';
    if (key === 'source') return `${SRANK[skillKind(s)]}${sourceLabel(s).toLowerCase()}`;
    return '';
  }
</script>

{#snippet trashIcon()}
  <svg class="ico" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m2 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
    <line x1="10" y1="11" x2="10" y2="17" /><line x1="14" y1="11" x2="14" y2="17" />
  </svg>
{/snippet}

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('plugins.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('plugins.subtitle')}</p>
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      {#if updateIds.length}
        <button class="sw-btn sw-btn-primary" disabled={busy} onclick={() => onBulkPlugin('update', updateIds)}
          title={t('plugins.updateAllTip')}>{t('plugins.updateAll', { count: updateIds.length })}</button>
      {/if}
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={onRefresh} title={t('plugins.refreshTip')}>
        {running === 'plugin-mgr' ? t('plugins.refreshing') : t('plugins.refreshBtn')}
      </button>
    </div>
  </header>

  <!-- Plugins -->
  <div class="dt-summary mb-sw-2">
    {t('plugins.summary', { total: pluginList.length, updates: updateIds.length, off: disabledCount })}
  </div>
  {#if pluginList.length}
    <DataTable
      columns={PLUGIN_COLS}
      rows={pluginRows}
      rowKey={(p) => p.id}
      sortAccessor={pluginSort}
      defaultSort="name"
      storageKey="plugins"
      search
      searchValue={(p) => p.id}
      searchPlaceholder={t('plugins.searchPlaceholder')}
      canExpand={(p) => hasContents(p.id)}
      rowMuted={(p) => !p.enabled}
      rowAccent={(p) => updateMap.has(p.id)}
      selectable
    >
      {#snippet toolbar()}
        {#if updates.length}
          <button class="sw-btn text-sw-xs {onlyUpdates ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (onlyUpdates = !onlyUpdates)}>{t('plugins.filterUpdates')}</button>
        {/if}
        <button class="sw-btn text-sw-xs {onlyEnabled ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (onlyEnabled = !onlyEnabled)}>{t('plugins.filterEnabled')}</button>
      {/snippet}

      {#snippet bulkbar(ids, clear)}
        <span class="font-medium">{t('plugins.bulkSelected', { count: ids.length })}</span>
        <span class="grow"></span>
        <button class="iconbtn" disabled={busy} onclick={() => { onBulkPlugin('enable', ids); clear(); }}>{t('plugins.bulkEnable')}</button>
        <button class="iconbtn" disabled={busy} onclick={() => { onBulkPlugin('disable', ids); clear(); }}>{t('plugins.bulkDisable')}</button>
        <button class="iconbtn" disabled={busy} onclick={() => { onBulkPlugin('update', ids.filter((i) => updateMap.has(i))); clear(); }}>{t('plugins.bulkUpdate')}</button>
        <button class="iconbtn danger" disabled={busy} onclick={() => { onBulkPlugin('remove', ids); clear(); }}>{t('plugins.bulkRemove')}</button>
        <button class="iconbtn" onclick={clear}>{t('common.cancel')}</button>
      {/snippet}

      {#snippet cell(p, col)}
        {#if col.key === 'name'}
          {@const av = avatar(split(p.id).name)}
          <span class="namecell">
            <span class="avatar" style="background:{av.color}22;color:{av.color}">{av.ch}</span>
            <span class="font-medium truncate" title={p.id}>{split(p.id).name}</span>
            {#if p.mine}<span class="srcbadge own" title={t('plugins.mineTip')}>{t('plugins.sourceOwn')}</span>{/if}
          </span>
        {:else if col.key === 'market'}
          {@const m = split(p.id).market}
          {#if m}<span class="src" title={m}>{shortMarket(m)}</span>{:else}<span class="ph">—</span>{/if}
        {:else if col.key === 'version'}
          {@const up = updateMap.get(p.id)}
          <span class="ver">
            {#if p.version && p.version !== 'unknown'}v{p.version}{:else}<span class="ph">—</span>{/if}{#if up}<span class="upto" title={t('plugins.updateAvailableBadgeTip', { version: up })}>→ v{up}</span>{/if}
          </span>
        {:else if col.key === 'desc'}
          {#if p.description}<span class="desc" title={p.description}>{p.description}</span>{:else}<span class="ph">—</span>{/if}
        {:else if col.key === 'contents'}
          {@const c = contentMap.get(p.id)}
          {#if c && hasContents(p.id)}
            <span class="comp">
              {#if c.skills.length}<span title={pSkill(c.skills.length)}>🧩 {c.skills.length}</span>{/if}
              {#if c.commands.length}<span title={pCommand(c.commands.length)}>⌘ {c.commands.length}</span>{/if}
              {#if c.agents.length}<span title={pAgent(c.agents.length)}>🤖 {c.agents.length}</span>{/if}
            </span>
          {:else}<span class="ph">—</span>{/if}
        {:else if col.key === 'status'}
          <span class="statuscell">
            {#if actingId === p.id}<Spinner size={13} />{/if}
            <Toggle checked={p.enabled} disabled={busy} onCheckedChange={() => act(p.enabled ? 'disable' : 'enable', p.id)}
              title={p.enabled ? t('plugins.disableBtnTip') : t('plugins.enableBtnTip')} />
          </span>
        {:else if col.key === 'actions'}
          <span class="act">
            {#if updateMap.has(p.id)}
              <button class="iconbtn" disabled={busy} onclick={() => act('update', p.id)} title={t('plugins.updateBtnTip', { version: updateMap.get(p.id) ?? '' })} aria-label={t('plugins.updateBtn')}>↑</button>
            {/if}
            <button class="iconbtn danger" disabled={busy} onclick={() => act('remove', p.id)} title={t('plugins.removeBtnTip')} aria-label={t('plugins.removeBtn')}>{@render trashIcon()}</button>
          </span>
        {/if}
      {/snippet}

      {#snippet expand(p)}
        {@const c = contentMap.get(p.id)}
        {#if c}
          <div class="detail">
            {#each [{ label: t('plugins.catSkills'), items: c.skills, icon: '🧩' }, { label: t('plugins.catCommands'), items: c.commands, icon: '⌘' }, { label: t('plugins.catAgents'), items: c.agents, icon: '🤖' }] as cat (cat.label)}
              {#if cat.items.length}
                <div class="detgroup">
                  <p class="detlabel">{cat.icon} {cat.label} <span class="ph">{cat.items.length}</span></p>
                  <div class="chips">
                    {#each cat.items as item (item)}<span class="chip">{item}</span>{/each}
                  </div>
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      {/snippet}

      {#snippet empty()}{t('plugins.noMatch')}{/snippet}
    </DataTable>
  {:else}
    <div class="sw-card text-sw-sm text-sw-text-muted">{t('plugins.noPlugins')}</div>
  {/if}

  <!-- Skills -->
  <div class="mb-sw-2 mt-sw-6 flex items-center justify-between">
    <div class="dt-summary">{t('plugins.skillsHeading', { count: skillList.length })}</div>
    {#if skillList.length}
      <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={onOpenSkills} title={t('plugins.openSkillsTip')}>{t('plugins.openSkillsBtn')}</button>
    {/if}
  </div>
  {#if skillList.length}
    <DataTable
      columns={SKILL_COLS}
      rows={skillRows}
      rowKey={(s) => s.dir}
      sortAccessor={skillSort}
      defaultSort="source"
      storageKey="skills"
      search
      searchValue={(s) => `${s.name} ${s.description ?? ''}`}
      searchPlaceholder={t('plugins.searchPlaceholder')}
    >
      {#snippet toolbar()}
        <button class="sw-btn text-sw-xs {skillSource === 'all' ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (skillSource = 'all')}>{t('plugins.srcAll', { count: skillList.length })}</button>
        <button class="sw-btn text-sw-xs {skillSource === 'own' ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (skillSource = 'own')}>{t('plugins.srcOwn', { count: ownSkillCount })}</button>
        <button class="sw-btn text-sw-xs {skillSource === 'plugin' ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (skillSource = 'plugin')}>{t('plugins.srcPlugin', { count: pluginSkillCount })}</button>
        <button class="sw-btn text-sw-xs {skillSource === 'default' ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (skillSource = 'default')}>{t('plugins.srcDefault', { count: skillList.length - ownSkillCount - pluginSkillCount })}</button>
      {/snippet}

      {#snippet cell(s, col)}
        {#if col.key === 'name'}
          {@const av = avatar(s.name)}
          <span class="namecell">
            <span class="avatar" style="background:{av.color}22;color:{av.color}">{av.ch}</span>
            <span class="font-medium truncate">{s.name}</span>
          </span>
        {:else if col.key === 'source'}
          <span class="srcbadge {skillKind(s)}" title={s.source.startsWith('plugin:') ? s.source.slice(7) : sourceLabel(s)}>{sourceLabel(s)}</span>
        {:else if col.key === 'version'}
          <span class="ver">{#if s.version}v{s.version}{:else}<span class="ph">—</span>{/if}</span>
        {:else if col.key === 'desc'}
          <span class="desc" title={s.description ?? ''}>{s.description ?? ''}</span>
        {:else if col.key === 'actions'}
          <span class="act">
            <button class="iconbtn" onclick={() => onOpenSkill(s.dir)} title={t('plugins.skillOpenTip')}>{t('plugins.skillOpen')}</button>
            {#if !s.source.startsWith('plugin:')}
              <button class="iconbtn danger" onclick={() => onDeleteSkill(s.dir, s.name)} title={t('plugins.skillDeleteTip')} aria-label={t('plugins.skillDelete')}>{@render trashIcon()}</button>
            {/if}
          </span>
        {/if}
      {/snippet}

      {#snippet empty()}{t('plugins.noMatch')}{/snippet}
    </DataTable>
  {:else}
    <div class="sw-card text-sw-sm text-sw-text-muted">{t('plugins.noSkills')}</div>
  {/if}
</div>

<style>
  .dt-summary {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--sw-text-muted);
  }
  .namecell {
    display: inline-flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
    max-width: 100%;
  }
  .avatar {
    flex: none;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .src {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 9999px;
    background: var(--sw-bg-subtle);
    border: 1px solid var(--sw-border);
    font-size: 11px;
    color: var(--sw-text-secondary);
    white-space: nowrap;
  }
  .statuscell {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    vertical-align: middle;
  }
  .ico {
    display: block;
  }
  .ver {
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: 11px;
    color: var(--sw-text-secondary);
    white-space: nowrap;
  }
  .upto {
    margin-left: 6px;
    color: var(--sw-accent-text);
  }
  .ph {
    color: var(--sw-text-muted);
    opacity: 0.6;
  }
  .comp {
    display: inline-flex;
    gap: 10px;
    font-size: 12px;
    color: var(--sw-text-secondary);
    white-space: nowrap;
  }
  .desc {
    display: block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .srcbadge {
    display: inline-block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 1px 8px;
    border-radius: 9999px;
    border: 1px solid var(--sw-border);
    font-size: 11px;
    vertical-align: middle;
  }
  .srcbadge.own {
    color: var(--sw-accent-text);
    border-color: var(--sw-accent-text);
  }
  .srcbadge.plugin {
    color: var(--sw-text-secondary);
    background: var(--sw-bg-subtle);
  }
  .srcbadge.default {
    color: var(--sw-text-muted);
  }
  /* actions: faint until row hover / focus */
  .act {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    opacity: 0.3;
    transition: opacity 0.12s;
  }
  :global(.dt-row:hover) .act,
  .act:focus-within {
    opacity: 1;
  }
  .iconbtn {
    background: none;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm, 6px);
    padding: 2px 8px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--sw-text-secondary);
    cursor: pointer;
    white-space: nowrap;
  }
  .iconbtn:hover {
    color: var(--sw-text);
    border-color: var(--sw-text-muted);
  }
  .iconbtn.danger:hover {
    color: #f87171;
    border-color: #f87171;
  }
  .iconbtn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .grow {
    flex: 1;
  }
  /* expanded contents */
  .detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 1100px;
  }
  .detlabel {
    margin-bottom: 5px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .chip {
    padding: 2px 8px;
    border-radius: 9999px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: 11px;
    color: var(--sw-text-secondary);
  }
</style>
