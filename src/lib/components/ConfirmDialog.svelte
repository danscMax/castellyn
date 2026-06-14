<script lang="ts">
  import { t } from '$lib/i18n';

  let {
    open,
    title,
    message,
    confirmLabel = t('common.confirm'),
    onConfirm,
    onCancel
  }: {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    onConfirm: () => void;
    onCancel: () => void;
  } = $props();
</script>

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && onCancel()} />

{#if open}
  <div class="overlay">
    <button type="button" class="backdrop" aria-label={t('common.close')} onclick={onCancel}></button>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <h3>{title}</h3>
      <p>{message}</p>
      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" onclick={onConfirm}>{confirmLabel}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
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
  .dialog {
    position: relative;
    width: min(420px, 90vw);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg);
    padding: var(--sw-space-6);
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
  }
  h3 {
    margin: 0 0 var(--sw-space-2);
    font-size: 1rem;
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  p {
    margin: 0 0 var(--sw-space-6);
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
    line-height: 1.5;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
  }
</style>
