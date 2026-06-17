<script lang="ts">
  // A styled replacement for the native <select>: app-themed trigger + chevron, a floating panel
  // with hover/keyboard highlight, a check on the current value, optional icons/hints. Bindable.
  type Opt = { value: string; label: string; icon?: string; hint?: string };

  let {
    value = $bindable(''),
    options,
    placeholder = '',
    disabled = false,
    onChange
  }: {
    value?: string;
    options: (Opt | string)[];
    placeholder?: string;
    disabled?: boolean;
    onChange?: (v: string) => void;
  } = $props();

  const opts = $derived(
    options.map((o) => (typeof o === 'string' ? { value: o, label: o } : o)) as Opt[]
  );
  const selected = $derived(opts.find((o) => o.value === value));

  let open = $state(false);
  let root: HTMLDivElement;
  let active = $state(-1);

  function toggle() {
    if (disabled) return;
    open = !open;
    if (open) active = opts.findIndex((o) => o.value === value);
  }
  function choose(v: string) {
    value = v;
    onChange?.(v);
    open = false;
  }
  function onKey(e: KeyboardEvent) {
    if (disabled) return;
    if (!open) {
      if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
        e.preventDefault();
        toggle();
      }
      return;
    }
    if (e.key === 'Escape') open = false;
    else if (e.key === 'ArrowDown') {
      e.preventDefault();
      active = Math.min(opts.length - 1, active + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (active >= 0) choose(opts[active].value);
    }
  }
  $effect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    window.addEventListener('mousedown', onDoc);
    return () => window.removeEventListener('mousedown', onDoc);
  });
</script>

<div class="select" bind:this={root}>
  <button
    type="button"
    class="trigger"
    {disabled}
    onclick={toggle}
    onkeydown={onKey}
    aria-haspopup="listbox"
    aria-expanded={open}
  >
    <span class="val" class:placeholder={!selected}>
      {#if selected?.icon}<span class="ic">{selected.icon}</span>{/if}
      {selected ? selected.label : placeholder}
    </span>
    <span class="chev" class:up={open} aria-hidden="true">▾</span>
  </button>
  {#if open}
    <ul class="panel" role="listbox">
      {#each opts as o, i (o.value)}
        <li>
          <button
            type="button"
            class="opt"
            class:sel={o.value === value}
            class:active={i === active}
            onclick={() => choose(o.value)}
            onmouseenter={() => (active = i)}
            role="option"
            aria-selected={o.value === value}
          >
            {#if o.icon}<span class="ic">{o.icon}</span>{/if}
            <span class="opt-label">{o.label}</span>
            {#if o.hint}<span class="opt-hint">{o.hint}</span>{/if}
            {#if o.value === value}<span class="check">✓</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select {
    position: relative;
    width: 100%;
  }
  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-input, var(--sw-bg-secondary));
    color: var(--sw-text-primary);
    font-size: var(--sw-text-sm);
    cursor: pointer;
    text-align: left;
  }
  .trigger:hover:not(:disabled) {
    border-color: var(--sw-accent-text);
  }
  .trigger:focus-visible {
    outline: none;
    border-color: var(--sw-accent-text);
    box-shadow: 0 0 0 2px var(--sw-accent-glow);
  }
  .trigger:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .val {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .val.placeholder {
    color: var(--sw-text-muted);
  }
  .chev {
    color: var(--sw-text-muted);
    transition: transform 0.15s;
    flex-shrink: 0;
  }
  .chev.up {
    transform: rotate(180deg);
  }
  .panel {
    position: absolute;
    z-index: 60;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    margin: 0;
    padding: 4px;
    list-style: none;
    max-height: 280px;
    overflow-y: auto;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.35);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    border-radius: var(--sw-radius-sm, 6px);
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-sm);
    cursor: pointer;
    text-align: left;
  }
  .opt.active {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
  }
  .opt.sel {
    color: var(--sw-text-primary);
    font-weight: 500;
  }
  .opt-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .opt-hint {
    color: var(--sw-text-muted);
    font-size: var(--sw-text-xs);
  }
  .check {
    color: var(--sw-accent-text);
    flex-shrink: 0;
  }
  .ic {
    flex-shrink: 0;
  }
</style>
