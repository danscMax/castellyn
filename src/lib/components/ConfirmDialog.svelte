<script lang="ts">
  import { t } from '$lib/i18n';
  import ModalShell from './ModalShell.svelte';

  let {
    open,
    title,
    message,
    confirmLabel = t('common.confirm'),
    details = [],
    requireText = null,
    danger = false,
    onConfirm,
    onCancel
  }: {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    /** Concrete items the action will affect (branches/files) — shown so the user sees the scope. */
    details?: string[];
    /** When set, the confirm button is enabled only after the user types this exact string. */
    requireText?: string | null;
    /** Render the confirm button in the destructive (red) style. */
    danger?: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  } = $props();

  let typed = $state('');
  // Reset the type-to-confirm field each time the dialog opens.
  $effect(() => {
    if (open) typed = '';
  });
  const blocked = $derived(!!requireText && typed.trim() !== requireText.trim());
  function confirm() {
    if (!blocked) onConfirm();
  }
</script>

<!-- For a destructive confirm, focus the SAFE choice (Cancel) on open and do NOT let a stray Enter
     fire the action: onEnter is wired only for benign confirms; requireText dialogs let the input's
     own Enter handle it (and are gated on the typed match). -->
<ModalShell
  {open}
  onClose={onCancel}
  onEnter={!danger && !requireText ? confirm : undefined}
  initialFocus={requireText ? 'input' : danger ? '[data-confirm-cancel]' : null}
  size="sm"
  role="alertdialog"
  labelledBy="dlg-title"
  describedBy={requireText ? 'dlg-msg dlg-type' : 'dlg-msg'}
>
      <h3 id="dlg-title">{title}</h3>
      <p id="dlg-msg">{message}</p>
      {#if details.length}
        <ul class="details">
          {#each details as d (d)}<li>{d}</li>{/each}
        </ul>
      {/if}
      {#if requireText}
        <label class="confirm-type">
          <span>{t('common.typeToConfirm', { text: requireText })}</span>
          <input
            id="dlg-type"
            class="sw-input"
            bind:value={typed}
            placeholder={requireText}
            autocomplete="off"
            spellcheck="false"
            onkeydown={(e) => e.key === 'Enter' && confirm()}
          />
        </label>
      {/if}
      <div class="row">
        <button class="sw-btn sw-btn-ghost" data-confirm-cancel onclick={onCancel}>{t('common.cancel')}</button>
        <button
          class="sw-btn {danger ? 'sw-btn-danger' : 'sw-btn-primary'}"
          disabled={blocked}
          aria-describedby={requireText ? 'dlg-type' : undefined}
          onclick={confirm}>{confirmLabel}</button>
      </div>
</ModalShell>

<style>
  h3 {
    margin: 0 0 var(--sw-space-2);
    font-size: var(--sw-text-lg);
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  p {
    margin: 0 0 var(--sw-space-4);
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
    line-height: 1.5;
  }
  .details {
    margin: 0 0 var(--sw-space-4);
    padding: var(--sw-space-2) var(--sw-space-3);
    list-style: none;
    max-height: 180px;
    overflow: auto;
    background: var(--sw-bg-hover);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    font-family: 'Cascadia Code', 'Consolas', monospace;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-primary);
  }
  .details li {
    padding: 2px 0;
  }
  .confirm-type {
    display: block;
    margin: 0 0 var(--sw-space-4);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .confirm-type .sw-input {
    width: 100%;
    margin-top: var(--sw-space-1);
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
  }
</style>
