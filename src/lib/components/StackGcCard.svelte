<script lang="ts">
  import type { GcItem } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { fmtBytes } from '$lib/bytes';
  import ModalShell from './ModalShell.svelte';
  import { RefreshCw, Trash2 } from '@lucide/svelte';

  // Ф2-GC: "stack garbage" card on Home. Prop-driven like StackDriftCard — the parent owns the
  // scan result (null = not scanned yet / scanning), this card renders the summary, opens the
  // preview modal, and hands the chosen ids back to the parent to delete (to the Recycle Bin).
  let { items = null, onReload, onDelete, busy = false }:
    {
      items: GcItem[] | null;
      onReload?: () => void | Promise<void>;
      onDelete?: (ids: string[], labels: string[]) => void | Promise<void>;
      busy?: boolean;
    } = $props();

  let working = $state(false); // refresh button spinner
  let modalOpen = $state(false);
  let selected = $state<string[]>([]);

  const all = $derived(items ?? []);
  const deletable = $derived(all.filter((i) => i.deletable));
  const wrongOs = $derived(all.filter((i) => !i.deletable));
  const delBytes = $derived(deletable.reduce((s, i) => s + i.size_bytes, 0));
  const wrongBytes = $derived(wrongOs.reduce((s, i) => s + i.size_bytes, 0));
  const nothing = $derived(items !== null && deletable.length === 0 && wrongOs.length === 0);

  const selectedBytes = $derived(all.filter((i) => selected.includes(i.id)).reduce((s, i) => s + i.size_bytes, 0));

  const CAT_KEY: Record<GcItem['category'], string> = {
    stale_version: 'page.home_gc_cat_stale',
    temp_git: 'page.home_gc_cat_tempgit',
    bak: 'page.home_gc_cat_bak',
    wrong_os: 'page.home_gc_cat_wrongos'
  };

  const fmt = (n: number) => fmtBytes(n, t('sync.byteUnits'));

  async function refresh() {
    working = true;
    try {
      await onReload?.();
    } finally {
      working = false;
    }
  }
  function openModal() {
    if (!deletable.length) return;
    selected = deletable.map((i) => i.id); // deletable pre-selected
    modalOpen = true;
  }
  function toggle(id: string, on: boolean) {
    selected = on ? [...new Set([...selected, id])] : selected.filter((s) => s !== id);
  }
  async function confirmDelete() {
    if (!selected.length) return;
    const labels = all.filter((i) => selected.includes(i.id)).map((i) => i.label);
    await onDelete?.(selected, labels);
    modalOpen = false;
  }
</script>

<section class="sw-card mb-sw-6">
  <div class="flex items-start justify-between gap-sw-3">
    <div class="min-w-0">
      <h2 class="font-semibold">{t('page.home_gc_title')}</h2>
      {#if items === null}
        <p class="text-sw-xs text-sw-text-secondary">{t('page.home_gc_scanning')}</p>
      {:else if nothing}
        <p class="text-sw-xs text-sw-text-secondary">{t('page.home_gc_empty')}</p>
      {:else}
        {#if deletable.length}
          <p class="text-sw-xs text-sw-text-secondary">{t('page.home_gc_summary', { size: fmt(delBytes), n: deletable.length })}</p>
        {/if}
        {#if wrongOs.length}
          <p class="text-sw-xs text-sw-text-muted">{t('page.home_gc_wrongos', { size: fmt(wrongBytes) })}</p>
        {/if}
      {/if}
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      {#if deletable.length}
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || working} onclick={openModal}>
          {t('page.home_gc_show_btn')}
        </button>
      {/if}
      <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || working} onclick={refresh}
        title={t('common.refresh')} aria-label={t('common.refresh')}>
        {#if working}{t('common.loading')}{:else}<RefreshCw size={14} />{/if}
      </button>
    </div>
  </div>
</section>

<ModalShell open={modalOpen} onClose={() => (modalOpen = false)} size="lg">
  <h3 class="mb-sw-3 text-base font-semibold">{t('page.home_gc_title')}</h3>
  <div class="mb-sw-3 flex flex-col">
    {#each all as it (it.id)}
      <div class="gcrow" class:muted={!it.deletable}>
        {#if it.deletable}
          <input type="checkbox" checked={selected.includes(it.id)} disabled={busy}
            onchange={(e) => toggle(it.id, e.currentTarget.checked)} aria-label={it.label} />
        {:else}
          <span class="cbspace" aria-hidden="true"></span>
        {/if}
        <span class="min-w-0 flex-1 truncate" title={it.path}>{it.label}</span>
        <span class="cat">{t(CAT_KEY[it.category])}</span>
        <span class="size">{fmt(it.size_bytes)}</span>
      </div>
    {/each}
  </div>
  <div class="flex items-center justify-between gap-sw-3 border-t border-sw-border pt-sw-3">
    <span class="text-sw-xs text-sw-text-muted">{t('page.home_gc_modal_selected', { size: fmt(selectedBytes) })}</span>
    <button class="sw-btn sw-btn-primary text-sw-sm" disabled={!selected.length || busy} onclick={confirmDelete}>
      <Trash2 size={14} />
      {t('page.home_gc_modal_delete_btn')}
    </button>
  </div>
</ModalShell>

<style>
  .gcrow {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
    padding: 6px 2px;
    border-bottom: 1px solid var(--sw-border);
    font-size: var(--sw-text-sm);
  }
  .gcrow:last-child {
    border-bottom: none;
  }
  .gcrow.muted {
    opacity: 0.55;
  }
  .cbspace {
    width: 13px;
    flex-shrink: 0;
  }
  .cat {
    flex-shrink: 0;
    padding: 1px 7px;
    border-radius: 99px;
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    color: var(--sw-text-muted);
    font-size: var(--sw-text-xs);
    white-space: nowrap;
  }
  .size {
    flex-shrink: 0;
    min-width: 64px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--sw-text-secondary);
  }
</style>
