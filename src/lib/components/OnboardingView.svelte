<script lang="ts">
  import type { OnbStep } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { statusFillVar } from '$lib/statusColor';
  import { RefreshCw } from '@lucide/svelte';

  // New-machine deployment checklist (reconciler, not a one-shot wizard): the parent owns the
  // steps array and every action (fixes route to existing commands + the run-lock/console).
  // Shown full-screen on a bare machine (first run) and re-openable from Settings.
  let { steps = null, busy = false, onFix, onRunAll, onRefresh, onDismiss }:
    {
      steps: OnbStep[] | null;
      busy?: boolean;
      onFix: (step: OnbStep) => boolean | Promise<boolean>;
      onRunAll: () => void | Promise<void>;
      onRefresh: () => void | Promise<void>;
      onDismiss: () => void;
    } = $props();

  let working = $state<string | null>(null); // '*' = refresh/run-all, else the step id

  const rows = $derived(steps ?? []);
  const allOk = $derived(rows.length > 0 && rows.every((r) => r.state !== 'todo'));
  const runnable = $derived(rows.some((r) => r.fix && (r.state === 'todo' || r.state === 'unknown')));

  const dot = { up: statusFillVar('up'), degraded: statusFillVar('degraded'), down: statusFillVar('down') } as const;
  function dotColor(state: OnbStep['state']): string {
    if (state === 'ok') return dot.up;
    if (state === 'todo') return dot.degraded;
    return 'var(--sw-text-muted)'; // blocked | unknown — neutral
  }
  function labelOf(id: string): string {
    const key = `page.onb_step_${id}`;
    const v = t(key);
    return v === key ? id : v; // unknown future step: show its id rather than mislabeling
  }
  const FIX_LABEL: Record<string, string> = {
    install_profiles: 'onb_fix_install',
    mcp_deploy: 'onb_fix_mcp',
    managed_deploy: 'onb_fix_managed',
    junction: 'onb_fix_junction',
    syncthing: 'onb_fix_run',
    verify: 'onb_fix_verify',
    backup_tab: 'onb_fix_backup'
  };

  async function doFix(step: OnbStep) {
    working = step.id;
    try {
      await onFix(step);
    } finally {
      working = null;
    }
  }
  async function withStar(fn: () => void | Promise<void>) {
    working = '*';
    try {
      await fn();
    } finally {
      working = null;
    }
  }
</script>

<div class="onb-wrap">
  <section class="sw-card onb-card">
    <div class="flex items-start justify-between gap-sw-3">
      <div>
        <h2 class="text-sw-lg font-semibold">{t('page.onb_title')}</h2>
        <p class="text-sw-xs text-sw-text-secondary">{t('page.onb_subtitle')}</p>
      </div>
      <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={!!working || busy}
        onclick={() => withStar(onRefresh)} title={t('common.refresh')} aria-label={t('common.refresh')}>
        {#if working === '*'}{t('common.loading')}{:else}<RefreshCw size={14} />{/if}
      </button>
    </div>

    {#if steps === null}
      <p class="mt-sw-4 text-sw-sm text-sw-text-muted">{t('common.loading')}</p>
    {:else}
      <div class="mt-sw-4 flex flex-col gap-sw-2 border-t border-sw-border pt-sw-3">
        {#each rows as r (r.id)}
          <div class="flex flex-wrap items-center gap-sw-2" class:dim={r.state === 'blocked'}>
            <span class="dot" style="background:{dotColor(r.state)}" aria-hidden="true"></span>
            <span class="text-sw-sm font-medium">{labelOf(r.id)}</span>
            {#if r.detail}
              <span class="text-sw-xs text-sw-text-secondary det" title={r.detail}>· {r.detail}</span>
            {:else if r.state === 'blocked'}
              <span class="text-sw-xs text-sw-text-muted">· {t('page.onb_state_blocked')}</span>
            {/if}
            {#if r.fix && r.state !== 'blocked' && r.state !== 'ok'}
              <button class="sw-btn sw-btn-ghost text-sw-xs ml-auto shrink-0" disabled={!!working || busy}
                onclick={() => doFix(r)}>
                {#if working === r.id}{t('common.loading')}{:else}{t(`page.${FIX_LABEL[r.fix] ?? 'onb_fix_run'}`)}{/if}
              </button>
            {/if}
          </div>
        {/each}
      </div>

      <div class="mt-sw-4 flex items-center gap-sw-2 border-t border-sw-border pt-sw-3">
        {#if allOk}
          <span class="text-sw-sm" style="color:{dot.up}">{t('page.onb_all_ok')}</span>
        {:else if runnable}
          <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!!working || busy}
            onclick={() => withStar(onRunAll)}>{t('page.onb_run_all')}</button>
        {/if}
        <button class="sw-btn sw-btn-ghost text-sw-xs ml-auto" onclick={onDismiss}>{t('page.onb_continue')}</button>
      </div>
    {/if}
  </section>
</div>

<style>
  .onb-wrap {
    height: 100%;
    overflow-y: auto;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 32px 24px;
  }
  .onb-card {
    width: min(720px, 100%);
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 9999px;
    flex-shrink: 0;
  }
  .dim {
    opacity: 0.55;
  }
  .det {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 60%;
  }
</style>
