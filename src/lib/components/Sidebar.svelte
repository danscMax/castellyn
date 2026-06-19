<script lang="ts">
  import { onMount } from 'svelte';
  import type { Attention } from '$lib/attention';
  import { t } from '$lib/i18n';
  import Spinner from './Spinner.svelte';

  let {
    active,
    onSelect,
    attention = {},
    loading = {}
  }: {
    active: string;
    onSelect: (id: string) => void;
    attention?: Record<string, Attention | null>;
    loading?: Record<string, boolean>;
  } = $props();

  // Labels are resolved reactively in markup via t(it.labelKey) so they follow the UI language.
  // Default order, grouped by intent: run agents (sessions/profiles/providers) → extend them
  // (mcp/extensions) → automate & inspect (schedule/analytics) → maintain (sync/updates/forks/
  // backup) → settings last. Users can drag to reorder; that custom order is persisted and
  // re-seeded from this default whenever ORD_VER below is bumped.
  const items = [
    { id: 'home', labelKey: 'nav.home', tipKey: 'nav.homeTip', icon: '⌂', enabled: true },
    { id: 'sessions', labelKey: 'nav.sessions', tipKey: 'nav.sessionsTip', icon: '▦', enabled: true },
    { id: 'profiles', labelKey: 'nav.profiles', tipKey: 'nav.profilesTip', icon: '☰', enabled: true },
    { id: 'providers', labelKey: 'nav.providers', tipKey: 'nav.providersTip', icon: '⚡', enabled: true },
    { id: 'mcp', labelKey: 'nav.mcp', tipKey: 'nav.mcpTip', icon: '⧉', enabled: true },
    { id: 'extensions', labelKey: 'nav.extensions', tipKey: 'nav.extensionsTip', icon: '🧩', enabled: true },
    { id: 'schedule', labelKey: 'nav.schedule', tipKey: 'nav.scheduleTip', icon: '🕒', enabled: true },
    { id: 'analytics', labelKey: 'nav.analytics', tipKey: 'nav.analyticsTip', icon: '📊', enabled: true },
    { id: 'sync', labelKey: 'nav.sync', tipKey: 'nav.syncTip', icon: '⇄', enabled: true },
    { id: 'updates', labelKey: 'nav.updates', tipKey: 'nav.updatesTip', icon: '⟳', enabled: true },
    { id: 'forks', labelKey: 'nav.forks', tipKey: 'nav.forksTip', icon: '⑂', enabled: true },
    { id: 'backup', labelKey: 'nav.backup', tipKey: 'nav.backupTip', icon: '⛁', enabled: true },
    { id: 'settings', labelKey: 'nav.settings', tipKey: 'nav.settingsTip', icon: '⚙', enabled: true }
  ];

  // Collapsed rail + user tab order, both persisted.
  const COLL_KEY = 'cmh-sidebar-collapsed';
  const ORD_KEY = 'cmh-sidebar-order';
  const ORD_VER_KEY = 'cmh-sidebar-order-ver';
  // Bump whenever the default `items` order above changes — re-seeds everyone to the new default
  // once (overriding a stale saved order), while still letting later manual reorders persist.
  const ORD_VER = '3';
  let collapsed = $state(false);
  let order = $state<string[]>(items.map((i) => i.id));
  const orderedItems = $derived(
    order.map((id) => items.find((i) => i.id === id)).filter((i): i is (typeof items)[number] => !!i)
  );
  onMount(() => {
    try {
      collapsed = localStorage.getItem(COLL_KEY) === '1';
      const saved = JSON.parse(localStorage.getItem(ORD_KEY) ?? '[]');
      // Honor the saved order only if it was stamped with the current default version; otherwise
      // re-seed from the new default and stamp it.
      if (localStorage.getItem(ORD_VER_KEY) === ORD_VER && Array.isArray(saved) && saved.length) {
        const valid = saved.filter((id: string) => items.some((i) => i.id === id));
        const missing = items.map((i) => i.id).filter((id) => !valid.includes(id));
        order = [...valid, ...missing];
      } else {
        order = items.map((i) => i.id);
        localStorage.setItem(ORD_KEY, JSON.stringify(order));
        localStorage.setItem(ORD_VER_KEY, ORD_VER);
      }
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
    const cur = [...order];
    const from = cur.indexOf(dragId);
    const to = cur.indexOf(targetId);
    if (from < 0 || to < 0) return;
    cur.splice(to, 0, cur.splice(from, 1)[0]);
    order = cur;
  }
  function onDrop() {
    try {
      localStorage.setItem(ORD_KEY, JSON.stringify(order));
      localStorage.setItem(ORD_VER_KEY, ORD_VER);
    } catch {
      /* ignore */
    }
    dragId = null;
  }
</script>

<nav class="sidebar" class:collapsed>
  <div class="brand">
    <div class="brand-dot"></div>
    <span class="brand-name">{t('nav.brand')}</span>
    <button class="collapse-btn" onclick={toggleCollapse} title={collapsed ? t('nav.expandSidebar') : t('nav.collapseSidebar')}
      aria-label={collapsed ? t('nav.expandSidebar') : t('nav.collapseSidebar')}>{collapsed ? '»' : '«'}</button>
  </div>
  {#each orderedItems as it (it.id)}
    <button
      class="nav-item"
      class:active={active === it.id}
      class:dragging={dragId === it.id}
      disabled={!it.enabled}
      title={collapsed ? t(it.labelKey) : t(it.tipKey)}
      draggable="true"
      ondragstart={(e) => onDragStart(e, it.id)}
      ondragover={(e) => onDragOver(e, it.id)}
      ondrop={onDrop}
      onclick={() => it.enabled && onSelect(it.id)}
    >
      <span class="nav-icon">{it.icon}</span>
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
</nav>

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
    padding: 11px 0;
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
    padding: 11px 14px;
    border: none;
    border-radius: var(--sw-radius-md);
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: 0.92rem;
    font-family: inherit;
    cursor: pointer;
    text-align: left;
    transition: all 0.15s ease;
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
  .att-info {
    background: var(--sw-accent);
    color: #fff;
  }
  .att-warn {
    background: var(--sw-warn);
    color: #1a1205;
  }
  .att-dot.att-warn {
    background: var(--sw-warn);
  }
  .att-dot.att-info {
    background: var(--sw-accent);
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
</style>
