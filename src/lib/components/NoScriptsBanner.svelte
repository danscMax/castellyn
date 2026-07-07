<script lang="ts">
  // OSS first-run: shown at the top of every script-backed tab when the maintenance scripts are
  // absent under SCRIPTS_ROOT. Explains that this tab drives the owner's PowerShell tooling and
  // how to point at it (Settings → SCRIPTS_ROOT). Dismissable for the session.
  import { t } from '$lib/i18n';
  import { X } from '@lucide/svelte';

  let dismissed = $state(false);
</script>

{#if !dismissed}
  <div class="ns-banner sw-card mb-sw-4">
    <div class="ns-text">
      <div class="ns-title status-warn">{t('page.no_scripts_banner_title')}</div>
      <p class="ns-body">{t('page.no_scripts_banner_body')}</p>
    </div>
    <button class="ns-close sw-btn sw-btn-ghost" onclick={() => (dismissed = true)} aria-label={t('common.close')}>
      <X size={16} strokeWidth={2} />
    </button>
  </div>
{/if}

<style>
  .ns-banner {
    display: flex;
    align-items: flex-start;
    gap: var(--sw-space-3);
    border-left: 3px solid var(--sw-status-warn);
  }
  .ns-text {
    flex: 1;
    min-width: 0;
  }
  .ns-title {
    font-weight: 500;
    margin-bottom: var(--sw-space-1);
  }
  .ns-body {
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
    line-height: 1.5;
    margin: 0;
  }
  .ns-close {
    flex-shrink: 0;
    padding: var(--sw-space-1);
  }
</style>
