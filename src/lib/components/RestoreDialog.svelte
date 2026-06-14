<script lang="ts">
  import type { RestoreOpts } from '$lib/ipc';
  import { t } from '$lib/i18n';

  let {
    open,
    snapshot,
    busy,
    onPreview,
    onRestore,
    onClose
  }: {
    open: boolean;
    snapshot: string;
    busy: boolean;
    onPreview: (opts: RestoreOpts) => void;
    onRestore: (opts: RestoreOpts) => void;
    onClose: () => void;
  } = $props();

  const ALL = ['ccmy', 'cc1', 'cc2', 'cc3', 'cc4', 'cc5'];
  let sel = $state<Record<string, boolean>>(Object.fromEntries(ALL.map((p) => [p, true])));
  let includeCreds = $state(false);
  let hasPreviewed = $state(false);

  const selected = $derived(ALL.filter((p) => sel[p]));

  // New snapshot => force a fresh preview before a real restore is allowed.
  $effect(() => {
    void snapshot;
    hasPreviewed = false;
  });

  function toggle(p: string) {
    sel[p] = !sel[p];
    hasPreviewed = false; // selection changed -> preview is stale
  }
  function toggleCreds() {
    includeCreds = !includeCreds;
    hasPreviewed = false;
  }

  function opts(): RestoreOpts {
    return { timestamp: snapshot, profiles: selected, includeCredentials: includeCreds };
  }
  function preview() {
    onPreview(opts());
    hasPreviewed = true;
  }
  function restore() {
    onRestore(opts());
  }
</script>

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && onClose()} />

{#if open}
  <div class="overlay">
    <button type="button" class="backdrop" aria-label={t('common.close')} onclick={onClose}></button>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <h3>{t('backup.dialogTitle')}</h3>
      <p class="snap">{snapshot}</p>

      <div class="section">
        <div class="section-title">{t('backup.profiles')}</div>
        <div class="profiles">
          {#each ALL as p (p)}
            <label class="chk">
              <input type="checkbox" checked={sel[p]} onchange={() => toggle(p)}
                title={t('backup.profileToggleTip')} />
              <span>{p}</span>
            </label>
          {/each}
        </div>
      </div>

      <label class="chk creds">
        <input type="checkbox" checked={includeCreds} onchange={toggleCreds}
          title={t('backup.includeCredsTip')} />
        <span>{t('backup.includeCreds')}</span>
      </label>

      <p class="warn">
        {t('backup.warn')}
      </p>

      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onClose} title={t('backup.closeTitle')}>{t('common.close')}</button>
        <button class="sw-btn sw-btn-ghost" disabled={busy || selected.length === 0} onclick={preview}
          title={t('backup.previewTitle')}>
          {t('backup.showPlan')}
        </button>
        <button
          class="sw-btn sw-btn-primary"
          disabled={busy || !hasPreviewed || selected.length === 0}
          onclick={restore}
          title={hasPreviewed
            ? t('backup.restoreTitle')
            : t('backup.restoreNeedsPreview')}
        >
          {t('backup.restore')}
        </button>
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
    width: min(460px, 92vw);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg);
    padding: var(--sw-space-6);
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
  }
  h3 {
    margin: 0 0 var(--sw-space-1);
    font-size: 1rem;
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .snap {
    margin: 0 0 var(--sw-space-4);
    font-family: monospace;
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
  }
  .section {
    margin-bottom: var(--sw-space-4);
  }
  .section-title {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sw-text-muted);
    margin-bottom: var(--sw-space-2);
  }
  .profiles {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sw-space-2) var(--sw-space-4);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--sw-text-sm);
    color: var(--sw-text);
    cursor: pointer;
  }
  .creds {
    margin-bottom: var(--sw-space-4);
  }
  .warn {
    margin: 0 0 var(--sw-space-6);
    font-size: var(--sw-text-sm);
    color: #fbbf24;
    line-height: 1.5;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
  }
</style>
