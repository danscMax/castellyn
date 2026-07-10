<script lang="ts">
  import { onMount } from 'svelte';
  import { navOrder, previewNavOrder, setNavOrder, NAV_GROUPS, groupOf } from '$lib/navOrder.svelte';
  import type { Attention } from '$lib/attention';
  import { t } from '$lib/i18n';
  import Spinner from './Spinner.svelte';

  import { toastStore } from '$lib/toast.svelte';

  let {
    active,
    onSelect,
    attention = {},
    loading = {},
    notifOpen = false,
    onToggleNotif,
    notifAnchor = $bindable()
  }: {
    active: string;
    onSelect: (id: string) => void;
    attention?: Record<string, Attention | null>;
    loading?: Record<string, boolean>;
    notifOpen?: boolean;
    onToggleNotif?: () => void;
    // Bound out so the sibling NotificationPanel can pin itself to the bell button.
    notifAnchor?: HTMLElement;
  } = $props();

  let notifCount = $derived(toastStore.history.unread);

  // Heroicons 20x20 solid — inline SVG strings inheriting currentColor via fill.
  const I = (d: string) => `<svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true"><path d="${d}"/></svg>`;
  const ICONS = {
    home: I('M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z'),
    sessions: I('M3.25 3A2.25 2.25 0 001 5.25v9.5A2.25 2.25 0 003.25 17h13.5A2.25 2.25 0 0019 14.75v-9.5A2.25 2.25 0 0016.75 3H3.25zm.943 8.752a.75.75 0 01.055-1.06L6.167 9.5l-1.92-1.192a.75.75 0 01.944-1.166l2.5 1.55a.75.75 0 010 1.216l-2.5 1.55a.75.75 0 01-1.058-.106zM9.25 12.25a.75.75 0 000 1.5h3a.75.75 0 000-1.5h-3z'),
    profiles: I('M10 8a3 3 0 100-6 3 3 0 000 6zM3.465 14.493a1.23 1.23 0 00.41 1.412A9.957 9.957 0 0010 18c2.31 0 4.438-.784 6.131-2.1.43-.333.604-.903.408-1.41a7.002 7.002 0 00-13.074.003z'),
    providers: I('M11.983 1.907a.75.75 0 00-1.292-.657l-8.5 9.5A.75.75 0 002.75 12h6.572l-1.305 6.093a.75.75 0 001.292.657l8.5-9.5A.75.75 0 0017.25 8h-6.572l1.305-6.093z'),
    mcp: I('M10.362 1.093a.75.75 0 00-.724 0L2.523 5.018a.75.75 0 000 1.342l5.115 2.883a.75.75 0 00.724 0l5.115-2.883a.75.75 0 000-1.342L10.362 1.093zM3.606 7.976l-.888 2.158a.75.75 0 00.362.96l5.115 2.882a.75.75 0 00.724 0l5.115-2.882a.75.75 0 00.362-.96l-.888-2.158-4.589 2.585a.75.75 0 01-.724 0L3.606 7.976zM3.606 12.726l-.888 2.158a.75.75 0 00.362.96l5.115 2.882a.75.75 0 00.724 0l5.115-2.882a.75.75 0 00.362-.96l-.888-2.158-4.589 2.585a.75.75 0 01-.724 0L3.606 12.726z'),
    envs: I('M4.25 2A2.25 2.25 0 002 4.25v2.5A2.25 2.25 0 004.25 9h2.5A2.25 2.25 0 009 6.75v-2.5A2.25 2.25 0 006.75 2h-2.5zm0 9A2.25 2.25 0 002 13.25v2.5A2.25 2.25 0 004.25 18h2.5A2.25 2.25 0 009 15.75v-2.5A2.25 2.25 0 006.75 11h-2.5zm9-9A2.25 2.25 0 0011 4.25v2.5A2.25 2.25 0 0013.25 9h2.5A2.25 2.25 0 0018 6.75v-2.5A2.25 2.25 0 0015.75 2h-2.5zm0 9A2.25 2.25 0 0011 13.25v2.5A2.25 2.25 0 0013.25 18h2.5A2.25 2.25 0 0018 15.75v-2.5A2.25 2.25 0 0015.75 11h-2.5z'),
    extensions: I('M11.25 1.5A2.75 2.75 0 008.5 4.25v.25H5a2 2 0 00-2 2v3a2 2 0 002 2h.25v.25A2.75 2.75 0 008 14.5a2.75 2.75 0 002.75-2.75V11.5H11a2 2 0 002-2v-3a2 2 0 00-2-2h-.25v-.25A2.75 2.75 0 0010 1.5h1.25z'),
    schedule: I('M10 18a8 8 0 100-16 8 8 0 000 16zm.75-13a.75.75 0 00-1.5 0v5.25a.75.75 0 00.428.679l3.75 2a.75.75 0 10.644-1.357l-3.322-1.77V5z'),
    analytics: I('M15.5 2A1.5 1.5 0 0014 3.5v13a1.5 1.5 0 001.5 1.5h1a1.5 1.5 0 001.5-1.5v-13A1.5 1.5 0 0016.5 2h-1zM9.5 6A1.5 1.5 0 008 7.5v9A1.5 1.5 0 009.5 18h1a1.5 1.5 0 001.5-1.5v-9A1.5 1.5 0 0010.5 6h-1zM3.5 10A1.5 1.5 0 002 11.5v5A1.5 1.5 0 003.5 18h1A1.5 1.5 0 006 16.5v-5A1.5 1.5 0 004.5 10h-1z'),
    sync: I('M13.78 2.72a.75.75 0 010 1.06l-2.47 2.47H16.5a.75.75 0 010 1.5h-5.19l2.47 2.47a.75.75 0 11-1.06 1.06l-3.75-3.75a.75.75 0 010-1.06l3.75-3.75a.75.75 0 011.06 0zm-7.5 8a.75.75 0 010 1.06L3.81 14.25H9a.75.75 0 010 1.5H3.81l2.47 2.47a.75.75 0 11-1.06 1.06l-3.75-3.75a.75.75 0 010-1.06l3.75-3.75a.75.75 0 011.06 0z'),
    updates: I('M15.312 11.424a5.5 5.5 0 01-9.201 2.466l-.312-.311h2.433a.75.75 0 000-1.5H3.989a.75.75 0 00-.75.75v4.242a.75.75 0 001.5 0v-2.43l.31.31a7 7 0 0011.712-3.138.75.75 0 00-1.45-.382zm1.442-5.532a.75.75 0 00-1.45.382 5.5 5.5 0 01-9.202 2.466l-.312-.311h2.433a.75.75 0 000-1.5H3.989a.75.75 0 00-.75.75v4.242a.75.75 0 001.5 0v-2.43l.31.31a7 7 0 0011.712-3.138.75.75 0 00-.348-.557z'),
    forks: I('M6.28 5.22a.75.75 0 010 1.06L2.56 10l3.72 3.72a.75.75 0 01-1.06 1.06L.97 10.53a.75.75 0 010-1.06l4.25-4.25a.75.75 0 011.06 0zm7.44 0a.75.75 0 011.06 0l4.25 4.25a.75.75 0 010 1.06l-4.25 4.25a.75.75 0 01-1.06-1.06L17.44 10l-3.72-3.72a.75.75 0 010-1.06z'),
    backup: I('M2 3a1 1 0 00-1 1v1a1 1 0 001 1h16a1 1 0 001-1V4a1 1 0 00-1-1H2zM2 7.5h16l-.811 7.71a2 2 0 01-1.99 1.79H4.802a2 2 0 01-1.99-1.79L2 7.5zM7 11a1 1 0 011-1h4a1 1 0 110 2H8a1 1 0 01-1-1z'),
    settings: I('M7.84 1.804A1 1 0 018.82 1h2.36a1 1 0 01.98.804l.331 1.652a6.993 6.993 0 011.929 1.115l1.598-.54a1 1 0 011.186.447l1.18 2.044a1 1 0 01-.205 1.251l-1.267 1.113a7.047 7.047 0 010 2.228l1.267 1.113a1 1 0 01.205 1.25l-1.18 2.045a1 1 0 01-1.186.447l-1.598-.54a6.993 6.993 0 01-1.929 1.115l-.33 1.652A1 1 0 0111.18 19H8.82a1 1 0 01-.98-.804l-.331-1.652a6.993 6.993 0 01-1.929-1.115l-1.598.54a1 1 0 01-1.186-.447l-1.18-2.044a1 1 0 01.205-1.251l1.267-1.113a7.05 7.05 0 010-2.228L1.82 7.593a1 1 0 01-.205-1.25l1.18-2.045a1 1 0 011.186-.447l1.598.54A6.993 6.993 0 017.51 3.456l.33-1.652zM10 13a3 3 0 100-6 3 3 0 000 6z')
  };

  // Labels are resolved reactively in markup via t(it.labelKey) so they follow the UI language.
  // Default order, grouped by intent: run agents (sessions/profiles/providers) → extend them
  // (mcp/extensions) → automate & inspect (schedule/analytics) → maintain (sync/updates/forks/
  // backup) → settings last. Users can drag to reorder; that custom order is persisted and
  // re-seeded from this default whenever ORD_VER below is bumped.
  const items = [
    { id: 'home', labelKey: 'nav.home', tipKey: 'nav.homeTip', icon: ICONS.home, enabled: true },
    { id: 'sessions', labelKey: 'nav.sessions', tipKey: 'nav.sessionsTip', icon: ICONS.sessions, enabled: true },
    { id: 'profiles', labelKey: 'nav.profiles', tipKey: 'nav.profilesTip', icon: ICONS.profiles, enabled: true },
    { id: 'providers', labelKey: 'nav.providers', tipKey: 'nav.providersTip', icon: ICONS.providers, enabled: true },
    { id: 'mcp', labelKey: 'nav.mcp', tipKey: 'nav.mcpTip', icon: ICONS.mcp, enabled: true },
    { id: 'envs', labelKey: 'nav.envs', tipKey: 'nav.envsTip', icon: ICONS.envs, enabled: true },
    { id: 'extensions', labelKey: 'nav.extensions', tipKey: 'nav.extensionsTip', icon: ICONS.extensions, enabled: true },
    { id: 'schedule', labelKey: 'nav.schedule', tipKey: 'nav.scheduleTip', icon: ICONS.schedule, enabled: true },
    { id: 'analytics', labelKey: 'nav.analytics', tipKey: 'nav.analyticsTip', icon: ICONS.analytics, enabled: true },
    { id: 'sync', labelKey: 'nav.sync', tipKey: 'nav.syncTip', icon: ICONS.sync, enabled: true },
    { id: 'updates', labelKey: 'nav.updates', tipKey: 'nav.updatesTip', icon: ICONS.updates, enabled: true },
    { id: 'forks', labelKey: 'nav.forks', tipKey: 'nav.forksTip', icon: ICONS.forks, enabled: true },
    { id: 'backup', labelKey: 'nav.backup', tipKey: 'nav.backupTip', icon: ICONS.backup, enabled: true },
    { id: 'settings', labelKey: 'nav.settings', tipKey: 'nav.settingsTip', icon: ICONS.settings, enabled: true }
  ];

  // Collapsed rail persisted here; the tab ORDER lives in the shared navOrder module (U1) so the
  // Ctrl+1..9 jumps and the palette hints always match the rendered order.
  const COLL_KEY = 'cmh-sidebar-collapsed';
  let collapsed = $state(false);
  onMount(() => {
    try {
      collapsed = localStorage.getItem(COLL_KEY) === '1';
    } catch {
      /* first run */
    }
  });
  function toggleCollapse() {
    collapsed = !collapsed;
    try {
      localStorage.setItem(COLL_KEY, collapsed ? '1' : '0');
    } catch {
      /* ignore */
    }
  }

  // Sections (NAV_GROUPS): headers + per-group collapse, so 14 flat rows become 3 short scannable
  // blocks. All groups start OPEN so a newcomer sees "Maintain" without hunting for it; a manual
  // collapse still persists via GROUPS_KEY (a saved state wins over this default). A closed group
  // still surfaces its aggregated attention on the header, and the group holding the ACTIVE tab
  // is always rendered open (palette/hotkey jumps must never land on a hidden item).
  const GROUPS_KEY = 'cmh-sidebar-groups-closed';
  let closedGroups = $state<Record<string, boolean>>({});
  onMount(() => {
    try {
      const saved = JSON.parse(localStorage.getItem(GROUPS_KEY) ?? 'null');
      if (saved && typeof saved === 'object') closedGroups = saved;
    } catch {
      /* first run */
    }
  });
  function toggleGroup(gid: string) {
    closedGroups = { ...closedGroups, [gid]: !closedGroups[gid] };
    try {
      localStorage.setItem(GROUPS_KEY, JSON.stringify(closedGroups));
    } catch {
      /* ignore */
    }
  }
  const sections = $derived(
    NAV_GROUPS.map((g) => {
      const its = navOrder.ids
        .filter((id) => g.ids.includes(id))
        .map((id) => items.find((i) => i.id === id))
        .filter((i): i is (typeof items)[number] => !!i);
      const closed = !!closedGroups[g.id] && !its.some((i) => i.id === active);
      // Aggregated attention for a closed header: total count + the highest-urgency level present.
      // #10: danger (blocked) outranks warn outranks info/done, so a closed group shows the most
      // urgent colour among its items.
      let attCount = 0;
      let attLevel: string | null = null;
      const rank: Record<string, number> = { done: 0, info: 1, warn: 2, danger: 3 };
      for (const i of its) {
        const a = attention[i.id];
        if (!a) continue;
        attCount += a.count ?? 0;
        if (attLevel === null || (rank[a.level] ?? 0) > (rank[attLevel] ?? 0)) attLevel = a.level;
      }
      return { ...g, items: its, closed, attCount, attLevel };
    })
  );

  // Drag a nav item over another to reorder (live), persisted on drop.
  let dragId = $state<string | null>(null);
  function onDragStart(e: DragEvent, id: string) {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(e: DragEvent, targetId: string) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    if (!dragId || dragId === targetId) return;
    // Items live inside their section — reorder only within the same group.
    if (groupOf(dragId) !== groupOf(targetId)) return;
    const cur = [...navOrder.ids];
    const from = cur.indexOf(dragId);
    const to = cur.indexOf(targetId);
    if (from < 0 || to < 0) return;
    cur.splice(to, 0, cur.splice(from, 1)[0]);
    previewNavOrder(cur);
  }
  function onDrop() {
    setNavOrder(navOrder.ids);
    dragId = null;
  }

  // Keyboard reorder (Alt+ArrowUp/Down on a focused item) — same splice + persist as drag,
  // so AT/keyboard users can personalize the order too (drag is pointer-only). Keeps focus
  // on the moved item by re-focusing its button after the DOM updates.
  function moveItem(e: KeyboardEvent, id: string) {
    if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    const cur = [...navOrder.ids];
    const from = cur.indexOf(id);
    const to = from + (e.key === 'ArrowUp' ? -1 : 1);
    if (from < 0 || to < 0 || to >= cur.length) return;
    if (groupOf(cur[to]) !== groupOf(id)) return; // stay within the section
    e.preventDefault();
    cur.splice(to, 0, cur.splice(from, 1)[0]);
    setNavOrder(cur);
    const btn = e.currentTarget as HTMLButtonElement;
    requestAnimationFrame(() => btn.focus());
  }
</script>

<nav class="sidebar" class:collapsed>
  <div class="brand">
    <div class="brand-dot"></div>
    <span class="brand-name">{t('nav.brand')}</span>
    <button class="collapse-btn" onclick={toggleCollapse}
      title={`${collapsed ? t('nav.expandSidebar') : t('nav.collapseSidebar')} · ${t('nav.reorderHint')}`}
      aria-label={collapsed ? t('nav.expandSidebar') : t('nav.collapseSidebar')}>{collapsed ? '»' : '«'}</button>
  </div>
  <div class="nav-scroll">
    {#each sections as g (g.id)}
      {#if g.labelKey && !collapsed}
        <button class="group-head" onclick={() => toggleGroup(g.id)} aria-expanded={!g.closed}>
          <span class="group-chev" class:closed={g.closed}>▾</span>
          <span class="group-label">{t(g.labelKey)}</span>
          {#if g.closed && g.attCount}
            <span class="att att-{g.attLevel ?? 'info'}">{g.attCount}</span>
          {:else if g.closed && g.attLevel}
            <span class="att-dot att-{g.attLevel}"></span>
          {/if}
        </button>
      {:else if collapsed && g.id !== 'work'}
        <div class="group-sep" role="separator"></div>
      {/if}
      {#if !g.closed || collapsed}
        {#each g.items as it (it.id)}
          <button
            class="nav-item"
            class:active={active === it.id}
            class:dragging={dragId === it.id}
            aria-current={active === it.id ? 'page' : undefined}
            disabled={!it.enabled}
            title={collapsed ? t(it.labelKey) : t(it.tipKey)}
            draggable="true"
            aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
            ondragstart={(e) => onDragStart(e, it.id)}
            ondragover={(e) => onDragOver(e, it.id)}
            ondrop={onDrop}
            onkeydown={(e) => moveItem(e, it.id)}
            onclick={() => it.enabled && onSelect(it.id)}
            onpointerup={(e) => e.currentTarget.blur()}
          >
            <span class="nav-icon">{@html it.icon}</span>
            <span class="nav-label">{t(it.labelKey)}</span>
            {#if !it.enabled}<span class="soon">{t('nav.soon')}</span>{/if}
            {#if loading[it.id]}
              <span class="spin-wrap" title={t('common.refreshing')}><Spinner size={13} /></span>
            {:else if attention[it.id]}
              {@const att = attention[it.id]}
              {#if att?.count}
                <span class="att att-{att.level}" title={t('nav.attentionCount', { count: att.count })}>{att.count}</span>
              {:else}
                <span class="att-dot att-{att?.level}" title={t('nav.attention')}></span>
              {/if}
            {/if}
          </button>
        {/each}
      {/if}
    {/each}
  </div>
  <div class="notif-area">
    <!-- A popup trigger, not a tab: say so, so a screen reader announces the panel's open state and
         the button does not read as another page in the nav. `.open` keeps the highlight tied to the
         panel rather than to whatever last had focus. -->
    <button
      class="notif-btn"
      class:open={notifOpen}
      bind:this={notifAnchor}
      onclick={onToggleNotif}
      title={t('common.notifications')}
      aria-label={t('common.notifications')}
      aria-haspopup="dialog"
      aria-expanded={notifOpen}
    >
      <span class="nav-icon">{@html NOTIF_ICON}</span>
      <span class="nav-label">{t('common.notifications')}</span>
      {#if notifCount}
        <span class="att att-info">{notifCount > 99 ? '99+' : notifCount}</span>
      {/if}
    </button>
  </div>
</nav>

<script module>
  const NOTIF_ICON = `<svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true"><path d="M10 2a6 6 0 00-6 6v3.586l-.707.707A1 1 0 004 14h12a1 1 0 00.707-1.707L16 11.586V8a6 6 0 00-6-6zM10 18a3 3 0 01-3-3h6a3 3 0 01-3 3z"/></svg>`;
</script>

<style>
  .sidebar {
    width: var(--sw-sidebar-width);
    flex-shrink: 0;
    background: var(--sw-sidebar-bg);
    border-right: 1px solid var(--sw-border);
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--sw-space-3);
    transition: width 0.22s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .sidebar.collapsed {
    width: 60px;
  }
  .collapsed .nav-label,
  .collapsed .brand-name,
  .collapsed .soon {
    display: none;
  }
  .collapsed .nav-item {
    justify-content: center;
    padding: var(--sw-space-3) 0;
    gap: 0;
  }
  .collapsed .att {
    /* count badge → a dot in the rail */
    min-width: 8px;
    width: 8px;
    height: 8px;
    padding: 0;
    font-size: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: var(--sw-space-2) var(--sw-space-3);
    margin-bottom: var(--sw-space-3);
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .collapsed .brand {
    padding: var(--sw-space-2) 0;
    justify-content: center;
    gap: 0;
  }
  .brand-name {
    flex: 1;
  }
  .collapse-btn {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    font-size: 1rem;
    padding: 2px 6px;
    border-radius: var(--sw-radius-sm, 6px);
    line-height: 1;
  }
  .collapse-btn:hover {
    color: var(--sw-text-primary);
    background: var(--sw-sidebar-item-hover);
  }
  .nav-item.dragging {
    opacity: 0.5;
  }
  .nav-scroll {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .group-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: var(--sw-space-3);
    padding: 3px 14px 3px 10px;
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    font-family: inherit;
    font-size: var(--sw-text-xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border-radius: var(--sw-radius-sm, 6px);
    text-align: left;
  }
  .group-head:hover {
    color: var(--sw-text-secondary);
  }
  .group-head:first-child {
    margin-top: 0;
  }
  .group-label {
    flex: 1;
  }
  .group-chev {
    font-size: 0.6rem;
    transition: transform 0.15s ease;
  }
  .group-chev.closed {
    transform: rotate(-90deg);
  }
  .group-sep {
    height: 1px;
    background: var(--sw-border);
    margin: var(--sw-space-2) 8px;
    flex-shrink: 0;
  }
  .brand-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--sw-accent);
    box-shadow: 0 0 10px var(--sw-accent-glow);
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: var(--sw-space-3);
    border: none;
    border-radius: var(--sw-radius-md);
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-base);
    font-family: inherit;
    /* grab cursor hints the items are draggable/reorderable (see collapse-btn tooltip) */
    cursor: grab;
    text-align: left;
    transition: all 0.15s ease;
  }
  .nav-item:active {
    cursor: grabbing;
  }
  .nav-item:hover:not(:disabled) {
    background: var(--sw-sidebar-item-hover);
    color: var(--sw-text-primary);
  }
  .nav-item.active {
    background: var(--sw-sidebar-item-active);
    color: var(--sw-accent-text);
  }
  .nav-item:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .nav-icon {
    width: 22px;
    font-size: 1.05rem;
    text-align: center;
  }
  .nav-label {
    flex: 1;
  }
  .att {
    min-width: 20px;
    height: 19px;
    padding: 0 6px;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .att-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .spin-wrap {
    display: inline-flex;
    align-items: center;
  }
  /* Badges carry white text, so they take the -solid fills; the .att-dot.* rules below are more
     specific and keep the brand hues for the dots, which carry no text. */
  .att-info {
    background: var(--sw-accent-solid);
    color: #fff;
  }
  .att-warn {
    background: var(--sw-warn);
    color: #1a1205;
  }
  .att-danger {
    background: var(--sw-danger-solid);
    color: #fff;
  }
  .att-done {
    background: var(--sw-status-done);
    color: #04231f;
  }
  .att-dot.att-warn {
    background: var(--sw-warn);
  }
  .att-dot.att-info {
    background: var(--sw-accent);
  }
  .att-dot.att-danger {
    background: var(--sw-danger);
  }
  .att-dot.att-done {
    background: var(--sw-status-done);
  }
  .soon {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--sw-bg-hover);
    color: var(--sw-text-muted);
  }
  .notif-area {
    margin-top: auto;
    border-top: 1px solid var(--sw-border);
    padding-top: 4px;
  }
  .notif-btn {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: var(--sw-space-3);
    border: none;
    border-radius: var(--sw-radius-md);
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-base);
    font-family: inherit;
    cursor: pointer;
    text-align: left;
    transition: all 0.15s ease;
  }
  .notif-btn:hover,
  .notif-btn.open {
    background: var(--sw-sidebar-item-hover);
    color: var(--sw-text-primary);
  }
  .collapsed .notif-btn {
    justify-content: center;
    padding: var(--sw-space-3) 0;
  }
  .collapsed .notif-area {
    /* same padding as .collapsed .brand for alignment */
    padding: var(--sw-space-2) 0;
  }
</style>
