<script lang="ts">
  type Item = {
    label: string;
    title?: string;
    onClick: () => void;
    disabled?: boolean;
    danger?: boolean;
  };
  let {
    label,
    title,
    items,
    align = 'right',
    variant = 'ghost',
    disabled = false
  }: {
    label?: string;
    title?: string;
    items: Item[];
    align?: 'left' | 'right';
    variant?: 'ghost' | 'primary';
    disabled?: boolean;
  } = $props();

  let open = $state(false);
  let root = $state<HTMLElement | undefined>();

  function toggle() {
    if (!disabled) open = !open;
  }
  function pick(it: Item) {
    if (it.disabled) return;
    open = false;
    it.onClick();
  }
  function onDocClick(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
</script>

<svelte:window onclick={onDocClick} onkeydown={(e) => e.key === 'Escape' && (open = false)} />

<div class="dd" bind:this={root}>
  <button
    class="sw-btn text-sw-xs {variant === 'primary' ? '' : 'sw-btn-ghost'}"
    {disabled}
    onclick={toggle}
    {title}
    aria-haspopup="menu"
    aria-expanded={open}
  >
    {#if label}{label} <span class="caret">▾</span>{:else}<span class="dots">⋯</span>{/if}
  </button>
  {#if open}
    <div class="menu {align}" role="menu">
      {#each items as it (it.label)}
        <button
          class="item"
          class:danger={it.danger}
          disabled={it.disabled}
          role="menuitem"
          title={it.title}
          onclick={() => pick(it)}
        >
          {it.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .dd {
    position: relative;
    display: inline-block;
  }
  .caret {
    opacity: 0.7;
    font-size: 0.8em;
  }
  .dots {
    font-size: 1.1em;
    line-height: 1;
    letter-spacing: 1px;
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    min-width: 180px;
    z-index: 30;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.4);
  }
  .menu.right {
    right: 0;
  }
  .menu.left {
    left: 0;
  }
  .item {
    text-align: left;
    padding: 6px 10px;
    border: none;
    background: transparent;
    color: var(--sw-text-primary);
    border-radius: var(--sw-radius-sm);
    font-size: var(--sw-text-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .item:hover:not(:disabled) {
    background: var(--sw-bg-tertiary, rgba(255, 255, 255, 0.06));
  }
  .item:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .item.danger {
    color: #f87171;
  }
</style>
