<script lang="ts">
  import { t } from '$lib/i18n';
  import ModalShell from './ModalShell.svelte';
  let { open, onClose }: { open: boolean; onClose: () => void } = $props();
  const rows = $derived([
    { k: 'Ctrl + K', d: t('page.hkPalette') },
    { k: 'Ctrl + 1 … 9', d: t('page.hkTabJump') },
    { k: 'Esc', d: t('page.hkCancel') },
    { k: 'Ctrl + T', d: t('page.hkNewSession') },
    { k: 'Alt + 1 / 2 / 3', d: t('page.hkColumns') },
    { k: 'Ctrl + ] / [', d: t('page.hkFocusPane') },
    { k: 'Ctrl + F', d: t('page.hkFind') },
    { k: 'Ctrl + Shift + C / V', d: t('page.hkCopyPaste') },
    { k: '?', d: t('page.hkHelp') }
  ]);
</script>

<ModalShell {open} onClose={onClose} size="sm">
      <h3>{t('page.hkTitle')}</h3>
      <dl class="rows">
        {#each rows as r (r.k)}
          <div class="row"><kbd>{r.k}</kbd><span>{r.d}</span></div>
        {/each}
      </dl>
      <div class="foot">
        <button class="sw-btn sw-btn-ghost" onclick={onClose}>{t('common.close')}</button>
      </div>
</ModalShell>

<style>
  h3 {
    margin: 0 0 var(--sw-space-4);
    font-size: 1rem;
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--sw-space-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--sw-space-4);
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
  }
  kbd {
    flex-shrink: 0;
    min-width: 130px;
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-primary);
    background: var(--sw-input-bg);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm, 6px);
    padding: 2px 6px;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--sw-space-6);
  }
</style>
