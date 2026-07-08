<script lang="ts">
  // Ctrl+K command palette: fuzzy-filter a flat command list, run on Enter. Keyboard-first.
  import { t } from '$lib/i18n';
  type Command = { id: string; label: string; hint?: string; icon?: string; run: () => void };

  let {
    open = false,
    commands = [],
    placeholder = '',
    onClose
  }: { open?: boolean; commands?: Command[]; placeholder?: string; onClose: () => void } = $props();

  let query = $state('');
  let active = $state(0);
  let input: HTMLInputElement | undefined = $state();

  // Simple subsequence match + rank by earliest match position.
  function score(label: string, q: string): number {
    if (!q) return 0;
    const l = label.toLowerCase();
    const s = q.toLowerCase();
    let i = 0;
    let first = -1;
    for (let j = 0; j < l.length && i < s.length; j++) {
      if (l[j] === s[i]) {
        if (first < 0) first = j;
        i++;
      }
    }
    return i === s.length ? first : -1;
  }
  const filtered = $derived(
    (query.trim()
      ? commands
          .map((c) => ({ c, s: score(c.label, query.trim()) }))
          .filter((x) => x.s >= 0)
          .sort((a, b) => a.s - b.s)
          .map((x) => x.c)
      : commands
    ).slice(0, 60)
  );

  $effect(() => {
    if (open) {
      query = '';
      active = 0;
      queueMicrotask(() => input?.focus());
    }
  });
  $effect(() => {
    query;
    active = 0;
  });

  function run(c: Command) {
    onClose();
    c.run();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
    else if (e.key === 'ArrowDown') {
      e.preventDefault();
      active = Math.min(filtered.length - 1, active + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (filtered[active]) run(filtered[active]);
    }
  }
</script>

{#if open}
  <div class="overlay" role="dialog" aria-modal="true">
    <button type="button" class="backdrop" aria-label={t('common.close')} onclick={onClose}></button>
    <div class="palette">
      <input
        bind:this={input}
        bind:value={query}
        class="q"
        {placeholder}
        spellcheck="false"
        autocomplete="off"
        onkeydown={onKey}
      />
      <ul class="list">
        {#each filtered as c, i (c.id)}
          <li>
            <button type="button" class="row" class:active={i === active} onmouseenter={() => (active = i)} onclick={() => run(c)}>
              {#if c.icon}<span class="ic">{c.icon}</span>{/if}
              <span class="lbl">{c.label}</span>
              {#if c.hint}<span class="hint">{c.hint}</span>{/if}
            </button>
          </li>
        {/each}
        {#if !filtered.length}
          <li class="empty">{t('common.noMatches')}</li>
        {/if}
      </ul>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(2px);
    cursor: default;
  }
  .palette {
    position: relative;
    width: min(560px, 92vw);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg);
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.5);
    overflow: hidden;
  }
  .q {
    width: 100%;
    padding: 14px 16px;
    border: none;
    border-bottom: 1px solid var(--sw-border);
    background: transparent;
    color: var(--sw-text-primary);
    font-size: var(--sw-text-lg);
    outline: none;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 6px;
    max-height: 50vh;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 9px 10px;
    border: none;
    border-radius: var(--sw-radius-md);
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-sm);
    cursor: pointer;
    text-align: left;
  }
  .row.active {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
  }
  .ic {
    width: 18px;
    text-align: center;
    flex-shrink: 0;
  }
  .lbl {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    color: var(--sw-text-muted);
    font-size: var(--sw-text-xs);
  }
  .empty {
    padding: 16px;
    text-align: center;
    color: var(--sw-text-muted);
  }
</style>
