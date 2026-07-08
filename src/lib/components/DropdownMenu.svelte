<script lang="ts">
  import { tick } from 'svelte';
  import { anchored } from '$lib/floating';
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
    disabled = false,
    glyph
  }: {
    label?: string;
    title?: string;
    items: Item[];
    align?: 'left' | 'right';
    variant?: 'ghost' | 'primary';
    disabled?: boolean;
    glyph?: string; // custom trigger glyph (no caret); falls back to ⋯ when no label/glyph given
  } = $props();

  let open = $state(false);
  let root = $state<HTMLElement | undefined>();
  let menuEl = $state<HTMLElement | undefined>();
  let triggerEl = $state<HTMLButtonElement | undefined>();

  function toggle() {
    if (disabled) return;
    open = !open;
  }
  // U6 (WAI-ARIA menu-button): closing via Escape or item activation returns focus to the
  // trigger — focus used to die on the unmounted menu item and fall to <body>. Click-outside
  // deliberately does NOT restore (the user focused something else on purpose).
  function closeRestoring() {
    if (!open) return;
    open = false;
    triggerEl?.focus();
  }
  function pick(it: Item) {
    if (it.disabled) return;
    closeRestoring();
    it.onClick();
  }
  // Outside-press dismissal is handled by the `anchored` action (onOutside) — one shared impl.
  // Roving focus across menuitems with the arrow keys (plus Home/End).
  function onMenuKey(e: KeyboardEvent) {
    const keys = ['ArrowDown', 'ArrowUp', 'Home', 'End'];
    if (!keys.includes(e.key) || !menuEl) return;
    const btns = Array.from(menuEl.querySelectorAll<HTMLButtonElement>('.item:not(:disabled)'));
    if (!btns.length) return;
    e.preventDefault();
    const cur = btns.indexOf(document.activeElement as HTMLButtonElement);
    let next: number;
    if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = btns.length - 1;
    else if (e.key === 'ArrowDown') next = cur < 0 ? 0 : (cur + 1) % btns.length;
    else next = cur <= 0 ? btns.length - 1 : cur - 1;
    btns[next].focus();
  }

  // Move focus into the menu when it opens (WAI-ARIA menu-button pattern): without this, focus stays
  // on the trigger after Enter/Space and the arrow-key roving above does nothing until a blind Tab.
  $effect(() => {
    if (!open || !menuEl) return;
    tick().then(() => menuEl?.querySelector<HTMLButtonElement>('.item:not(:disabled)')?.focus());
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && closeRestoring()} />

<div class="dd" bind:this={root}>
  <button
    bind:this={triggerEl}
    class="sw-btn text-sw-xs {variant === 'primary' ? '' : 'sw-btn-ghost'}"
    {disabled}
    onclick={toggle}
    {title}
    aria-label={label ? undefined : title}
    aria-haspopup="menu"
    aria-expanded={open}
  >
    {#if label}{label} <span class="caret">▾</span>{:else if glyph}<span class="dots" aria-hidden="true">{glyph}</span>{:else}<span class="dots" aria-hidden="true">⋯</span>{/if}
  </button>
  {#if open}
    <div class="menu" role="menu" tabindex="-1" bind:this={menuEl} onkeydown={onMenuKey} use:anchored={{ anchor: root!, align, onOutside: () => (open = false) }}>
      {#each items as it (it.label)}
        <button
          class="item"
          class:status-bad={it.danger}
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
    /* position/top/left set inline by use:anchored (fixed, escapes table overflow) */
    position: fixed;
    min-width: 180px;
    max-width: min(280px, calc(100vw - 16px));
    z-index: 60;
    display: flex;
    flex-direction: column;
    padding: var(--sw-space-1);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.4);
  }
  .item {
    text-align: left;
    padding: var(--sw-space-2) var(--sw-space-3);
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
</style>
