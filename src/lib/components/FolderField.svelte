<script lang="ts">
  import { onMount } from 'svelte';
  import { pickFolder } from '$lib/ipc';
  import { anchored } from '$lib/floating';
  import { t } from '$lib/i18n';
  import { FolderOpen, Star } from '@lucide/svelte';

  // A folder input with a quick-pick dropdown: favourites + recent, plus native browse. One folder =
  // one project (owner: "проект = папка") — no separate "projects root" concept in the picker; the
  // root only seeds where Browse opens. Favourites persist; recent is shared (written on each launch).
  let {
    value = $bindable(''),
    placeholder = ''
  }: { value?: string; placeholder?: string } = $props();

  const FAV = 'cmh-fav-folders';
  const REC = 'cmh-recent-folders';
  const ROOT = 'cmh-projects-root';

  let open = $state(false);
  let rootEl = $state<HTMLDivElement>();
  let favorites = $state<string[]>([]);
  let recent = $state<string[]>([]);
  let root = $state(''); // only the Browse start dir now — not a "projects" list source

  function read() {
    try {
      // Guard against valid-JSON-but-not-an-array (corruption / a downgrade) — a non-array would
      // survive JSON.parse and then throw at `favorites.includes(...)` in the isFav derived.
      const f = JSON.parse(localStorage.getItem(FAV) ?? '[]');
      favorites = Array.isArray(f) ? f : [];
      const r = JSON.parse(localStorage.getItem(REC) ?? '[]');
      recent = Array.isArray(r) ? r : [];
      root = localStorage.getItem(ROOT) ?? '';
    } catch {
      /* first run */
    }
  }
  onMount(read);

  function openMenu() {
    read();
    open = true;
  }
  const base = (p: string) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p;
  function choose(f: string) {
    value = f;
    open = false;
  }
  async function browse() {
    const d = await pickFolder(value || root);
    if (d) value = d;
    open = false;
  }
  const isFav = $derived(!!value && favorites.includes(value));
  function toggleFav() {
    if (!value) return;
    favorites = isFav ? favorites.filter((f) => f !== value) : [value, ...favorites];
    localStorage.setItem(FAV, JSON.stringify(favorites));
  }
  // Outside-press dismissal is handled by the `anchored` action (onOutside) — one shared impl.
</script>

<div class="ff" bind:this={rootEl}>
  <input class="sw-input inp text-sw-xs" bind:value {placeholder} spellcheck="false" autocomplete="off" />
  <button class="icon" onclick={() => (open ? (open = false) : openMenu())} title={t('sessions.folderMenu')} aria-label={t('sessions.folderMenu')}>▾</button>
  <!-- V6: SVG icons (one icon language) instead of the 📁 color emoji / ★ glyph -->
  <button class="icon" onclick={browse} title={t('sessions.browse')} aria-label={t('sessions.browse')}><FolderOpen size={14} aria-hidden="true" /></button>
  <button class="icon star" class:on={isFav} onclick={toggleFav} title={t('sessions.fav')} aria-label={t('sessions.fav')}><Star size={14} fill={isFav ? 'currentColor' : 'none'} aria-hidden="true" /></button>
  {#if open}
    <div class="menu" use:anchored={{ anchor: rootEl!, align: 'left', onOutside: () => (open = false) }}>
      {#if favorites.length}
        <div class="sec">{t('sessions.favorites')}</div>
        {#each favorites as f (f)}
          <button class="row" onclick={() => choose(f)} title={f}><span class="b"><Star size={11} fill="currentColor" aria-hidden="true" /> {base(f)}</span><span class="p">{f}</span></button>
        {/each}
      {/if}
      {#if recent.length}
        <div class="sec">{t('sessions.recent')}</div>
        {#each recent.slice(0, 8) as f (f)}
          <button class="row" onclick={() => choose(f)} title={f}><span class="b">{base(f)}</span><span class="p">{f}</span></button>
        {/each}
      {/if}
      {#if !favorites.length && !recent.length}
        <div class="sec">{t('sessions.folderMenuEmpty')}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .ff {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
  }
  .inp {
    flex: 1;
    min-width: 0;
  }
  .icon {
    border: 1px solid var(--sw-border);
    background: var(--sw-bg-secondary);
    color: var(--sw-text-muted);
    border-radius: var(--sw-radius-md);
    cursor: pointer;
    padding: 5px 8px;
    line-height: 1;
    flex-shrink: 0;
  }
  .icon:hover {
    color: var(--sw-text-primary);
  }
  .star.on {
    /* V10: one favorite-star color across the app (SessionsTab stars already use --sw-warn). */
    color: var(--sw-warn);
  }
  .menu {
    /* position/top/left set inline by use:anchored (fixed, escapes overflow ancestors) */
    position: fixed;
    z-index: 60;
    min-width: 320px;
    max-width: 560px;
    max-height: 360px;
    overflow-y: auto;
    padding: 4px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.35);
  }
  .sec {
    padding: 6px 8px 2px;
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    padding: 5px 8px;
    border: none;
    border-radius: var(--sw-radius-sm, 6px);
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    text-align: left;
  }
  .row:hover {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
  }
  .b {
    font-size: var(--sw-text-sm);
    flex-shrink: 0;
  }
  .p {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
