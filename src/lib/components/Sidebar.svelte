<script lang="ts">
  import type { Attention } from '$lib/attention';
  import { t } from '$lib/i18n';

  let {
    active,
    onSelect,
    attention = {}
  }: {
    active: string;
    onSelect: (id: string) => void;
    attention?: Record<string, Attention | null>;
  } = $props();

  // Labels are resolved reactively in markup via t(it.labelKey) so they follow the UI language.
  const items = [
    { id: 'updates', labelKey: 'nav.updates', tipKey: 'nav.updatesTip', icon: '⟳', enabled: true },
    { id: 'forks', labelKey: 'nav.forks', tipKey: 'nav.forksTip', icon: '⑂', enabled: true },
    { id: 'backup', labelKey: 'nav.backup', tipKey: 'nav.backupTip', icon: '⛁', enabled: true },
    { id: 'profiles', labelKey: 'nav.profiles', tipKey: 'nav.profilesTip', icon: '☰', enabled: true },
    { id: 'mcp', labelKey: 'nav.mcp', tipKey: 'nav.mcpTip', icon: '⧉', enabled: true },
    { id: 'sync', labelKey: 'nav.sync', tipKey: 'nav.syncTip', icon: '⇄', enabled: true },
    { id: 'providers', labelKey: 'nav.providers', tipKey: 'nav.providersTip', icon: '⚡', enabled: true },
    { id: 'extensions', labelKey: 'nav.extensions', tipKey: 'nav.extensionsTip', icon: '🧩', enabled: true },
    { id: 'schedule', labelKey: 'nav.schedule', tipKey: 'nav.scheduleTip', icon: '🕒', enabled: true },
    { id: 'settings', labelKey: 'nav.settings', tipKey: 'nav.settingsTip', icon: '⚙', enabled: true }
  ];
</script>

<nav class="sidebar">
  <div class="brand">
    <div class="brand-dot"></div>
    <span>{t('nav.brand')}</span>
  </div>
  {#each items as it (it.id)}
    <button
      class="nav-item"
      class:active={active === it.id}
      disabled={!it.enabled}
      title={t(it.tipKey)}
      onclick={() => it.enabled && onSelect(it.id)}
    >
      <span class="nav-icon">{it.icon}</span>
      <span class="nav-label">{t(it.labelKey)}</span>
      {#if !it.enabled}<span class="soon">{t('nav.soon')}</span>{/if}
      {#if attention[it.id]}
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
    color: var(--sw-accent);
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
  .att-info {
    background: var(--sw-accent);
    color: #fff;
  }
  .att-warn {
    background: #f59e0b;
    color: #1a1205;
  }
  .att-dot.att-warn {
    background: #f59e0b;
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
