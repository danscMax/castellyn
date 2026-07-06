<script lang="ts">
  import { toastStore, dismiss, dismissAll, pauseToasts, resumeToasts } from '$lib/toast.svelte';
  import { copyText } from '$lib/clipboard';
  import { t } from '$lib/i18n';
</script>

<!-- Hovering the stack pauses auto-dismiss so a toast can't vanish mid-read / mid-reach.
     Per-toast focus pauses for keyboard/AT users (host div isn't focusable, so focus handlers
     moved onto each toast body which IS focusable through its close button). -->
<div
  class="toast-host"
  role="region"
  aria-label={t('common.notifications')}
  onmouseenter={pauseToasts}
  onmouseleave={resumeToasts}
>
  {#if toastStore.items.length >= 2}
    <button class="dismiss-all" onclick={dismissAll}>{t('common.dismissAll')}</button>
  {/if}
  {#each toastStore.items as toast (toast.id)}
    <!-- Errors/warnings interrupt (assertive); success/info stay polite so a failure is announced. -->
    <div
      class="toast {toast.kind}"
      role={toast.kind === 'error' || toast.kind === 'warn' ? 'alert' : 'status'}
      tabindex="-1"
      onfocusin={pauseToasts}
      onfocusout={resumeToasts}
    >
      <div class="body">
        <div class="title">{toast.title}</div>
        {#if toast.detail}<div class="detail">{toast.detail}</div>{/if}
        <div class="acts">
          {#if toast.action}
            <button class="act" onclick={() => { toast.action!.onClick(); dismiss(toast.id); }}>
              {toast.action.label}
            </button>
          {/if}
          {#if toast.detail && (toast.kind === 'error' || toast.kind === 'warn')}
            <button class="act" onclick={() => copyText(`${toast.title}\n${toast.detail}`)}>{t('common.copy')}</button>
          {/if}
        </div>
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
    /* The container spans the gaps between toasts; without this it would swallow clicks on the
       controls behind it (owner report: bottom-right toasts covered launcher buttons). Only the
       toasts + dismiss-all opt back into pointer events. */
    pointer-events: none;
    /* A burst of sticky error toasts can't grow off-screen / out of reach — the stack scrolls. */
    max-height: calc(100vh - 32px);
    overflow-y: auto;
    overflow-x: hidden;
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
    pointer-events: auto;
  }
  .toast.success { border-left-color: var(--sw-success); }
  .toast.info { border-left-color: #38bdf8; }
  .toast.warn { border-left-color: var(--sw-warn); }
  .toast.error { border-left-color: var(--sw-danger); }
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
  .acts { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 8px; }
  .acts:empty { display: none; }
  .dismiss-all {
    align-self: flex-end;
    pointer-events: auto;
    padding: 2px 10px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-sm);
    cursor: pointer;
  }
  .dismiss-all:hover { color: var(--sw-text-primary); }
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
