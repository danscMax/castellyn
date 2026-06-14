<script lang="ts">
  import { toastStore, dismiss } from '$lib/toast.svelte';
  import { t } from '$lib/i18n';
</script>

<div class="toast-host">
  {#each toastStore.items as toast (toast.id)}
    <div class="toast {toast.kind}" role="status">
      <div class="body">
        <div class="title">{toast.title}</div>
        {#if toast.detail}<div class="detail">{toast.detail}</div>{/if}
        {#if toast.action}
          <button class="act" onclick={() => { toast.action!.onClick(); dismiss(toast.id); }}>
            {toast.action.label}
          </button>
        {/if}
      </div>
      <button class="x" aria-label={t('common.close')} title={t('common.close')} onclick={() => dismiss(toast.id)}>×</button>
    </div>
  {/each}
</div>

<style>
  .toast-host {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: min(380px, 90vw);
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--sw-border);
    border-left-width: 3px;
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-secondary);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.4);
    animation: slidein 0.16s ease;
  }
  .toast.success { border-left-color: #34d399; }
  .toast.info { border-left-color: #38bdf8; }
  .toast.warn { border-left-color: #f59e0b; }
  .toast.error { border-left-color: #f87171; }
  .body { min-width: 0; flex: 1; }
  .title {
    font-size: var(--sw-text-sm);
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .detail {
    margin-top: 2px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    word-break: break-word;
  }
  .act {
    margin-top: 8px;
    padding: 3px 10px;
    font-size: var(--sw-text-xs);
    font-weight: 500;
    color: var(--sw-text-primary);
    background: var(--sw-bg-tertiary, rgba(255, 255, 255, 0.06));
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm);
    cursor: pointer;
  }
  .act:hover { border-color: var(--sw-border-focus, #64748b); }
  .x {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    padding: 0 2px;
  }
  .x:hover { color: var(--sw-text-primary); }
  @keyframes slidein {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
