<script lang="ts">
  import type { PluginInfo, SkillInfo, PluginAction, PluginUpdate, PluginContents, PluginRelease, PluginSyncStatus, BumpLevel } from '$lib/ipc';
  import { listPluginReleases, openPath } from '$lib/ipc';
  import { pushToast } from '$lib/toast.svelte';
  import { t, pSkill, pCommand, pAgent, pPlugin } from '$lib/i18n';
  import Toggle from './Toggle.svelte';
  import Spinner from './Spinner.svelte';
  import DropdownMenu from './DropdownMenu.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';
  import { Puzzle, SquareSlash, Bot } from '@lucide/svelte';

  let {
    plugins,
    skills,
    updates = [],
    contents = [],
    running,
    syncStatus = null,
    onAction,
    onBulkPlugin,
    onBump,
    onRefresh,
    onOpenSkills,
    onOpenSkill,
    onDeleteSkill,
    onSyncNow,
    onSyncHookToggle,
    onUnblock
  }: {
    plugins: PluginInfo[] | null;
    skills: SkillInfo[] | null;
    updates?: PluginUpdate[];
    contents?: PluginContents[];
    running: string | null;
    syncStatus?: PluginSyncStatus | null;
    onAction: (action: PluginAction, id: string) => void;
    onBulkPlugin: (action: PluginAction, ids: string[]) => void;
    /** Ф3: dual-manifest version bump of an own-marketplace plugin (+ cache refresh). */
    onBump?: (id: string, level: BumpLevel) => void;
    onRefresh: () => void;
    onOpenSkills: () => void;
    onOpenSkill: (dir: string) => void;
    onDeleteSkill: (dir: string, name: string) => void;
    onSyncNow: () => void;
    onSyncHookToggle: (enabled: boolean) => void;
    /** Unblock a managed-policy-blocked plugin: source edit + redeploy, wired in +page. */
    onUnblock?: (id: string) => void;
  } = $props();

  // Auto-sync toggle reflects FULL coverage; a partial wiring (e.g. a profile added after
  // enabling) shows as off + the coverage counter, and re-enabling wires the missing ones.
  const syncTotal = $derived(syncStatus ? syncStatus.wired.length + syncStatus.unwired.length : 0);
  const syncHookOn = $derived(!!syncStatus && syncTotal > 0 && syncStatus.unwired.length === 0);

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
  // Master-detail expansion: selected item key ("cmd:dev" / "skill:name") per expanded plugin.
  let detSel = $state<Record<string, string>>({});
  /** File reference for the detail pane, relative to the plugin root (from the last
   *  commands/skills/agents segment); full path stays in the tooltip. */
  function relTail(p: string): string {
    const seg = p.split(/[\\/]/).filter(Boolean);
    const i = seg.findLastIndex((s) => s === 'commands' || s === 'skills' || s === 'agents');
    return (i >= 0 ? seg.slice(i) : seg.slice(-2)).join('\\');
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
  let onlyManaged = $state(false);
  try {
    onlyUpdates = localStorage.getItem('cmh-plugins-only-updates') === '1';
    onlyEnabled = localStorage.getItem('cmh-plugins-only-enabled') === '1';
    onlyManaged = localStorage.getItem('cmh-plugins-only-managed') === '1';
  } catch {
    /* ignore */
  }
  $effect(() => {
    try {
      localStorage.setItem('cmh-plugins-only-updates', onlyUpdates ? '1' : '0');
      localStorage.setItem('cmh-plugins-only-enabled', onlyEnabled ? '1' : '0');
      localStorage.setItem('cmh-plugins-only-managed', onlyManaged ? '1' : '0');
    } catch {
      /* ignore */
    }
  });

  const managedCount = $derived(pluginList.filter((p) => p.managedPolicy === false).length);
  // A persisted-on filter whose toolbar chip is hidden (nothing matches anymore) must not apply:
  // it would empty the table with no visible way to untoggle it.
  const pluginRows = $derived(
    pluginList.filter(
      (p) =>
        (!onlyUpdates || updates.length === 0 || updateMap.has(p.id)) &&
        (!onlyEnabled || p.enabled) &&
        (!onlyManaged || managedCount === 0 || p.managedPolicy === false)
    )
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
  let changelogPlugin = $state<string | null>(null);
  let changelogReleases = $state<PluginRelease[] | null>(null);
  let changelogError = $state<string | null>(null);
  let changelogLoading = $state(false);
  async function openChangelog(id: string) {
    changelogPlugin = id;
    changelogReleases = null;
    changelogError = null;
    changelogLoading = true;
    try {
      changelogReleases = await listPluginReleases(id);
    } catch (e) {
      changelogError = String(e);
    }
    changelogLoading = false;
  }
  function closeChangelog() {
    changelogPlugin = null;
    changelogReleases = null;
    changelogError = null;
    changelogLoading = false;
  }
  const skillRows = $derived(
    skillSource === 'all' ? skillList : skillList.filter((s) => skillKind(s) === skillSource)
  );
  const ownSkillCount = $derived(skillList.filter((s) => s.mine).length);
  const pluginSkillCount = $derived(
    skillList.filter((s) => s.source.startsWith('plugin:') && !s.mine).length
  );

  // Semver-aware sort key for the version column. DataTable compares with </<> on the accessor
  // value, so we map a version to a zero-padded, comparable string: each dot-group is right-aligned
  // numerically (v10 > v9, 0.10.0 > 0.9.0). A trailing pre-release/non-numeric tag (e.g. "-rc.1")
  // sorts BEFORE the same release (1.0.0-rc < 1.0.0) per semver; missing versions sort first.
  function semverKey(v: string | null | undefined): string {
    const s = (v ?? '').trim().replace(/^v/i, '');
    if (!s) return '';
    const [core, ...rest] = s.split('-');
    const pre = rest.join('-');
    const nums = core
      .split('.')
      .map((g) => {
        const n = parseInt(g, 10);
        return Number.isNaN(n) ? g.toLowerCase().padStart(10, '0') : String(n).padStart(10, '0');
      })
      .join('.');
    // A release (no pre-release) must sort AFTER its pre-releases → append '~' (high ASCII) when
    // there's no tag, and the lowercased tag otherwise (so '1.0.0' > '1.0.0-rc').
    return `${nums}|${pre ? pre.toLowerCase() : '~'}`;
  }

  const PLUGIN_COLS: DTColumn[] = $derived([
    { key: 'name', label: t('plugins.colName'), sortable: true, grow: true },
    // V2: the fixed columns summed past the content width at 1440px → horizontal scroll with the
    // status/actions columns clipped. Trimmed to real content; name (grow) + spacer absorb slack.
    { key: 'market', label: t('plugins.colMarket'), sortable: true, width: '110px' },
    { key: 'version', label: t('plugins.colVersion'), sortable: true, width: '110px' },
    { key: 'desc', label: t('plugins.skillColDesc'), width: '240px' },
    { key: 'contents', label: t('plugins.colContents'), width: '110px' },
    { key: 'status', label: t('plugins.colStatus'), sortable: true, align: 'center', width: '74px', interactive: true },
    { key: 'actions', label: t('plugins.colActions'), align: 'right', width: '124px', interactive: true }
  ]);
  function pluginSort(p: PluginInfo, key: string): string | number {
    if (key === 'name') return split(p.id).name.toLowerCase();
    if (key === 'market') return shortMarket(split(p.id).market).toLowerCase();
    if (key === 'version') return semverKey(p.version);
    if (key === 'status') return p.enabled ? 1 : 0;
    return '';
  }

  const SKILL_COLS: DTColumn[] = $derived([
    { key: 'name', label: t('plugins.colName'), sortable: true, width: '230px' },
    { key: 'source', label: t('plugins.colSource'), sortable: true, width: '150px' },
    { key: 'version', label: t('plugins.colVersion'), sortable: true, width: '90px' },
    { key: 'desc', label: t('plugins.skillColDesc'), grow: true },
    { key: 'actions', label: t('plugins.colActions'), align: 'right', width: '128px', interactive: true }
  ]);
  const SRANK: Record<string, number> = { own: 0, default: 1, plugin: 2 };
  function skillSort(s: SkillInfo, key: string): string | number {
    if (key === 'name') return s.name.toLowerCase();
    if (key === 'version') return semverKey(s.version);
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

{#snippet listIcon()}
  <svg class="ico" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" />
    <line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />
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

  <!-- Cross-profile plugin sync -->
  <div class="sw-card synccard mb-sw-4">
    <div class="syncmeta">
      <p class="synctitle">{t('plugins.syncTitle')}</p>
      <p class="syncdesc">{t('plugins.syncDesc')}</p>
    </div>
    <div class="syncactions">
      {#if syncStatus}
        <span class="synccov" title={syncStatus.wired.join(', ')}>
          {t('plugins.syncCoverage', { wired: syncStatus.wired.length, total: syncTotal })}
        </span>
      {/if}
      <label class="synctoggle" title={t('plugins.syncHookTip')}>
        <Toggle checked={syncHookOn} disabled={busy || !syncStatus} onCheckedChange={onSyncHookToggle} />
        <span>{t('plugins.syncHookLabel')}</span>
      </label>
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={onSyncNow} title={t('plugins.syncNowTip')}>
        {running === 'pluginsync' ? t('plugins.refreshing') : t('plugins.syncNowBtn')}
      </button>
    </div>
  </div>

  {#if plugins === null && skills === null}
    <div class="flex flex-col gap-sw-2">
      {#each Array(4) as _, i (i)}
        <div class="skeleton" style="height:2.4rem"></div>
      {/each}
    </div>
  {:else}
  <!-- Plugins -->
  <div class="dt-summary mb-sw-2">
    {t('plugins.summary', { plugins: `${pluginList.length} ${pPlugin(pluginList.length)}`, updates: updateIds.length, off: disabledCount })}
  </div>
  <!-- #4: the whole tab's action buttons share one global run-lock (busy) so concurrent `claude
       plugin` writes can't race ~/.claude. Without this note a disabled ↑/toggle looked broken —
       explain WHY it's inert instead of leaving the user guessing. -->
  {#if busy}
    <div class="busybar" role="status">⏳ {t('plugins.busyNote')}</div>
  {/if}
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
      highlightAttr={(p) => `plugin:${p.id}`}
      selectable
    >
      {#snippet toolbar()}
        {#if updates.length}
          <button class="sw-btn text-sw-xs {onlyUpdates ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (onlyUpdates = !onlyUpdates)}>{t('plugins.filterUpdates')}</button>
        {/if}
        <button class="sw-btn text-sw-xs {onlyEnabled ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (onlyEnabled = !onlyEnabled)}>{t('plugins.filterEnabled')}</button>
        {#if managedCount}
          <button class="sw-btn text-sw-xs {onlyManaged ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={() => (onlyManaged = !onlyManaged)}>🔒 {t('plugins.filterManaged', { n: managedCount })}</button>
        {/if}
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
            {#if p.version && p.version !== 'unknown'}v{p.version}{:else}<span class="ph">—</span>{/if}{#if up}<button class="upto" disabled={busy} onclick={() => act('update', p.id)} title={p.mine ? t('plugins.syncCacheTip', { version: up }) : t('plugins.updateBtnTip', { version: up })}>→ v{up}</button>{/if}
          </span>
        {:else if col.key === 'desc'}
          {#if p.description}<span class="desc" title={p.description}>{p.description}</span>{:else}<span class="ph">—</span>{/if}
        {:else if col.key === 'contents'}
          {@const c = contentMap.get(p.id)}
          {#if c && hasContents(p.id)}
            <!-- V6: SVG icons instead of 🧩 / mac-⌘ / 🤖 emoji (⌘ is a macOS key on a Windows app) -->
            <span class="comp">
              {#if c.skills.length}<span title={pSkill(c.skills.length)}><Puzzle size={12} aria-hidden="true" /> {c.skills.length}</span>{/if}
              {#if c.commands.length}<span title={pCommand(c.commands.length)}><SquareSlash size={12} aria-hidden="true" /> {c.commands.length}</span>{/if}
              {#if c.agents.length}<span title={pAgent(c.agents.length)}><Bot size={12} aria-hidden="true" /> {c.agents.length}</span>{/if}
            </span>
          {:else}<span class="ph">—</span>{/if}
        {:else if col.key === 'status'}
          <span class="statuscell">
            {#if actingId === p.id && busy}<Spinner size={13} />{/if}
            {#if p.managedPolicy === false}
              <!-- Managed policy blocks this plugin in EVERY profile — a toggle would just fail
                   nine times. Offer the real fix: unblock in the source + redeploy (UAC). -->
              <button class="lockbtn" disabled={busy || !onUnblock} onclick={() => onUnblock?.(p.id)}
                title={`${t('plugins.blockedBadge')} — ${t('plugins.blockedTip')}`} aria-label={t('plugins.blockedBadge')}>🔒</button>
            {:else}
              <Toggle checked={p.enabled} disabled={busy} onCheckedChange={() => act(p.enabled ? 'disable' : 'enable', p.id)}
                title={p.enabled ? t('plugins.disableBtnTip') : t('plugins.enableBtnTip')} />
            {/if}
          </span>
        {:else if col.key === 'actions'}
          <span class="act">
            {#if p.mine && onBump}
              <!-- Ф3: own-marketplace version bump (patch/minor/major) → dual-manifest write + refresh. -->
              <DropdownMenu glyph="⇪" title={t('plugins.bumpBtnTip')} disabled={busy}
                items={['patch', 'minor', 'major'].map((lv) => ({
                  label: t('plugins.bumpLevel', { level: lv }),
                  onClick: () => { actingId = p.id; onBump(p.id, lv as BumpLevel); }
                }))} />
            {/if}
            <button class="iconbtn" onclick={() => openChangelog(p.id)} title={t('plugins.changelogBtnTip')} aria-label={t('plugins.changelogBtn')}>{@render listIcon()}</button>
            <button class="iconbtn danger" disabled={busy} onclick={() => act('remove', p.id)} title={t('plugins.removeBtnTip')} aria-label={t('plugins.removeBtn')}>{@render trashIcon()}</button>
          </span>
        {/if}
      {/snippet}

      {#snippet expand(p)}
        {@const c = contentMap.get(p.id)}
        {#if c}
          <!-- Master-detail (owner-picked mockup #2): item list left, full description + file right. -->
          {@const groups = [
            { key: 'cmd', label: t('plugins.catCommands'), typeLabel: t('plugins.detTypeCommand'), items: c.commands, icon: SquareSlash },
            { key: 'skill', label: t('plugins.catSkills'), typeLabel: t('plugins.detTypeSkill'), items: c.skills, icon: Puzzle },
            { key: 'agent', label: t('plugins.catAgents'), typeLabel: t('plugins.detTypeAgent'), items: c.agents, icon: Bot }
          ].filter((g) => g.items.length)}
          {@const flat = groups.flatMap((g) => g.items.map((it) => ({ g, it, key: `${g.key}:${it.name}` })))}
          {@const cur = flat.find((f) => f.key === detSel[p.id]) ?? flat[0]}
          {@const pname = split(p.id).name}
          <div class="detail md">
            <!-- Plain buttons, not a fake listbox: role=listbox/option promises arrow-key
                 navigation we don't implement — buttons already give correct Tab semantics. -->
            <div class="mdlist" role="group" aria-label={t('plugins.detListLabel')}>
              {#each groups as g (g.key)}
                {@const GIcon = g.icon}
                <p class="detlabel"><GIcon size={12} aria-hidden="true" /> {g.label} <span class="ph">{g.items.length}</span></p>
                {#each g.items as item (item.name)}
                  {@const k = `${g.key}:${item.name}`}
                  <button type="button" class="mditem" class:sel={cur?.key === k} aria-pressed={cur?.key === k}
                    onclick={() => (detSel[p.id] = k)}>
                    {g.key === 'cmd' ? `/${pname}:${item.name}` : item.name}
                  </button>
                {/each}
              {/each}
            </div>
            {#if cur}
              <div class="mdpane">
                <h3 class="mdname">{cur.g.key === 'cmd' ? `/${pname}:${cur.it.name}` : cur.it.name}</h3>
                <p class="mdtype">{cur.g.typeLabel} · <span title={cur.it.path}>{relTail(cur.it.path)}</span></p>
                <p class="mddesc" class:ph={!cur.it.description}>{cur.it.description ?? t('plugins.detNoDesc')}</p>
                <button class="sw-btn sw-btn-ghost text-sw-xs mdopen"
                  onclick={() => openPath(cur.it.path).catch((e) => pushToast({ kind: 'error', title: String(e) }))}
                  title={cur.it.path}>{t('plugins.detOpenFile')}</button>
              </div>
            {/if}
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
          <span class="act act-always">
            <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onOpenSkill(s.dir)} title={t('plugins.skillOpenTip')}>{t('plugins.skillOpen')}</button>
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
  {/if}
</div>

<!-- Escape closes the changelog, like every other overlay in the app. -->
<svelte:window onkeydown={(e) => e.key === 'Escape' && changelogPlugin !== null && closeChangelog()} />

{#if changelogPlugin !== null}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="cl-overlay" role="presentation" onclick={closeChangelog} ontouchstart={closeChangelog}>
    <div class="cl-modal" role="dialog" aria-label={t('plugins.changelogTitle')} tabindex="-1" onclick={(e) => e.stopPropagation()} ontouchstart={(e) => e.stopPropagation()}>
      <div class="cl-header">
        <h2 class="cl-title">{t('plugins.changelogTitle')} — {changelogPlugin}</h2>
        <button class="iconbtn" onclick={closeChangelog} aria-label={t('common.close')}>✕</button>
      </div>
      <div class="cl-body">
        {#if changelogLoading}
          <p class="cl-status">{t('plugins.changelogLoading')}</p>
        {:else if changelogError}
          <p class="cl-status status-bad">{t('plugins.changelogError')}<br><span class="cl-errdetail">{changelogError}</span></p>
        {:else if changelogReleases && changelogReleases.length}
          {#each changelogReleases as rel (rel.tag_name)}
            <div class="cl-release">
              <div class="cl-release-head">
                <span class="cl-tag">{rel.tag_name}</span>
                <span class="cl-date">{rel.published_at.slice(0, 10)}</span>
              </div>
              {#if rel.name && rel.name !== rel.tag_name}
                <p class="cl-release-name">{rel.name}</p>
              {/if}
              {#if rel.body}
                <pre class="cl-notes">{rel.body}</pre>
              {/if}
            </div>
          {/each}
        {:else}
          <p class="cl-status">{t('plugins.changelogNoReleases')}</p>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .synccard {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .syncmeta {
    min-width: 0;
  }
  .synctitle {
    font-size: var(--sw-text-sm);
    font-weight: 600;
  }
  .syncdesc {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .syncactions {
    display: inline-flex;
    align-items: center;
    gap: 14px;
    flex-shrink: 0;
  }
  .synccov {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    white-space: nowrap;
  }
  .synctoggle {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    cursor: pointer;
    white-space: nowrap;
  }
  .dt-summary {
    font-size: var(--sw-text-xs);
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
    font-size: var(--sw-text-xs);
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
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    white-space: nowrap;
  }
  /* Clickable update/sync affordance in the version cell — the single "get the newer version"
     control (the old duplicate ↑ action button was removed to kill the two-up-arrows confusion). */
  .upto {
    margin-left: 6px;
    padding: 0 4px;
    border: 1px solid transparent;
    border-radius: var(--sw-radius-sm, 6px);
    background: none;
    font: inherit;
    color: var(--sw-accent-text);
    cursor: pointer;
  }
  .upto:hover:not(:disabled) {
    border-color: var(--sw-accent);
  }
  .upto:disabled {
    cursor: not-allowed;
    opacity: 0.6;
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
    font-size: var(--sw-text-xs);
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
  /* Skills' primary action ("Открыть") must always be visible, not faint-until-hover — it's the
     point of the row, not a secondary icon. */
  .act.act-always {
    opacity: 1;
  }
  .iconbtn:disabled {
    cursor: not-allowed;
  }
  /* #4: tab is busy (global run-lock held) → explains why the action buttons are inert. */
  .busybar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: var(--sw-space-2);
    padding: var(--sw-space-2) var(--sw-space-3);
    border: 1px solid var(--sw-border);
    border-left: 3px solid var(--sw-status-warn);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-subtle);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
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
  .lockbtn {
    background: none;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm, 6px);
    padding: 2px 8px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    cursor: pointer;
    white-space: nowrap;
  }
  .lockbtn:hover:not(:disabled) {
    border-color: var(--sw-accent);
    color: var(--sw-accent);
  }
  .lockbtn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .iconbtn:hover {
    color: var(--sw-text);
    border-color: var(--sw-text-muted);
  }
  .iconbtn.danger:hover {
    color: var(--sw-danger);
    border-color: var(--sw-danger);
  }
  .iconbtn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .grow {
    flex: 1;
  }
  /* expanded contents — master-detail: grouped item list left, description pane right */
  .detail.md {
    display: grid;
    grid-template-columns: 280px 1fr;
    max-width: 1100px;
    border: 1px solid var(--sw-border);
    border-radius: 10px;
    overflow: hidden;
    background: var(--sw-bg-card);
  }
  .mdlist {
    border-right: 1px solid var(--sw-border);
    padding: 8px 0;
    max-height: 340px;
    overflow-y: auto;
  }
  .detlabel {
    margin: 8px 14px 3px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .detlabel:first-child {
    margin-top: 0;
  }
  .mditem {
    display: block;
    width: 100%;
    text-align: left;
    padding: 4px 14px;
    border: none;
    border-right: 2px solid transparent;
    background: transparent;
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mditem:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  .mditem.sel {
    background: var(--sw-accent-glow);
    color: var(--sw-accent-text);
    border-right-color: var(--sw-accent);
  }
  .mdpane {
    padding: 14px 20px;
    min-width: 0;
  }
  .mdname {
    margin: 0;
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-sm);
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .mdtype {
    margin: 2px 0 10px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
  }
  .mddesc {
    margin: 0;
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
    max-width: 640px;
    max-height: 220px;
    overflow-y: auto;
    white-space: pre-line;
  }
  .mdopen {
    margin-top: 12px;
  }
  /* Changelog modal */
  .cl-overlay {
    position: fixed;
    inset: 0;
    z-index: 70;
    background: rgba(0,0,0,0.45);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .cl-modal {
    background: var(--sw-bg-primary);
    border: 1px solid var(--sw-border);
    border-radius: 12px;
    width: min(680px, calc(100vw - 40px));
    max-height: min(80vh, 600px);
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0,0,0,0.35);
  }
  .cl-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--sw-border);
  }
  .cl-title {
    font-size: 13px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cl-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
  }
  .cl-status {
    text-align: center;
    color: var(--sw-text-muted);
    font-size: 12px;
    padding: 32px 0;
  }
  .cl-errdetail {
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    opacity: 0.7;
  }
  .cl-release {
    margin-bottom: 14px;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--sw-border);
  }
  .cl-release:last-child {
    border-bottom: none;
    margin-bottom: 0;
    padding-bottom: 0;
  }
  .cl-release-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 4px;
  }
  .cl-tag {
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: 12px;
    font-weight: 600;
    color: var(--sw-accent-text);
  }
  .cl-date {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
  }
  .cl-release-name {
    font-size: 12px;
    color: var(--sw-text-secondary);
    margin-bottom: 4px;
  }
  .cl-notes {
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    line-height: 1.6;
    color: var(--sw-text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
  }
</style>
