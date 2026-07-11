<script lang="ts">
  import type { RestoreOpts } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import Toggle from './Toggle.svelte';
  import ModalShell from './ModalShell.svelte';
  import Spinner from './Spinner.svelte';

  let {
    open,
    snapshot,
    busy,
    log = [],
    profiles = [],
    onPreview,
    onRestore,
    onClose
  }: {
    open: boolean;
    snapshot: string;
    busy: boolean;
    log?: string[];
    profiles?: string[];
    onPreview: (opts: RestoreOpts) => void;
    onRestore: (opts: RestoreOpts) => void;
    onClose: () => void;
  } = $props();

  // Real profile list from the backup payload; falls back to the canonical set on first paint.
  // 'main' (~/.claude) is a restorable target too (stored under main\ in every snapshot); list it as
  // a selectable chip so a scoped restore includes/excludes it like any other profile — the restore
  // script now gates the main files on `-Profiles` containing 'main'.
  const FALLBACK = ['ccmy', 'cc1', 'cc2', 'cc3', 'cc4', 'cc5'];
  const list = $derived(['main', ...(profiles.length ? profiles : FALLBACK).filter((p) => p !== 'main')]);
  let sel = $state<Record<string, boolean>>({});
  let includeCreds = $state(false);
  let hasPreviewed = $state(false);
  // True once a preview/restore has been triggered from THIS dialog — gates the in-dialog output
  // panel. Reset on any input change (snapshot/selection/creds) so a stale plan never lingers.
  let ran = $state(false);
  // R4: restore completion tracking. A finished restore used to leave the danger button live
  // (busy fell, hasPreviewed stayed true) with no "done" signal — one stray click re-ran the whole
  // destructive restore. Now we detect the busy rise→fall of a restore run, show "done", and force
  // a fresh preview before another restore can fire.
  let restoreArmed = $state(false); // a restore was requested; watching its run to finish
  let restoreSawBusy = $state(false);
  let restoreDone = $state(false);
  $effect(() => {
    if (!restoreArmed) return;
    if (busy) {
      restoreSawBusy = true;
      return;
    }
    if (restoreSawBusy) {
      restoreArmed = false;
      restoreDone = true;
      hasPreviewed = false; // disarm the danger button: a new restore needs a new preview
    }
  });

  // Default every (newly seen) profile to selected.
  $effect(() => {
    for (const p of list) if (sel[p] === undefined) sel[p] = true;
  });
  const selected = $derived(list.filter((p) => sel[p]));
  const allOn = $derived(list.length > 0 && list.every((p) => sel[p]));
  function setAll(v: boolean) {
    for (const p of list) sel[p] = v;
    hasPreviewed = false;
    ran = false;
  }

  // New snapshot => force a fresh preview before a real restore is allowed.
  $effect(() => {
    void snapshot;
    hasPreviewed = false;
    ran = false;
  });

  function toggle(p: string) {
    sel[p] = !sel[p];
    hasPreviewed = false; // selection changed -> preview is stale
    ran = false;
  }
  function toggleCreds(v: boolean) {
    includeCreds = v;
    hasPreviewed = false;
    ran = false;
  }

  function opts(): RestoreOpts {
    return { timestamp: snapshot, profiles: selected, includeCredentials: includeCreds };
  }
  function preview() {
    ran = true;
    restoreDone = false; // a fresh plan clears the previous "done" badge
    onPreview(opts());
    hasPreviewed = true;
  }
  function restore() {
    ran = true;
    restoreDone = false;
    restoreArmed = true;
    restoreSawBusy = false;
    onRestore(opts());
  }
</script>

<ModalShell {open} onClose={onClose} size="sm">
      <h3>{t('backup.dialogTitle')}</h3>
      <p class="snap">{snapshot}</p>

      <div class="section">
        <div class="section-head">
          <span class="section-title">{t('backup.profiles')}</span>
          <button class="selall" onclick={() => setAll(!allOn)}>
            {allOn ? t('common.deselectAll') : t('common.selectAll')}
          </button>
        </div>
        <div class="profiles">
          {#each list as p (p)}
            <button type="button" class="pchip" class:on={sel[p]} onclick={() => toggle(p)}
              title={t('backup.profileToggleTip')}>{p}</button>
          {/each}
        </div>
      </div>

      <label class="creds">
        <Toggle checked={includeCreds} onCheckedChange={toggleCreds} title={t('backup.includeCredsTip')} />
        <span>{t('backup.includeCreds')}</span>
      </label>

      <p class="warn">
        {t('backup.warn')}
      </p>

      <!-- In-dialog run output: the restore-preview / restore stream lands here (not only in the
           bottom run-log), so the plan is readable in context and the running state is visible. -->
      {#if ran}
        <div class="plan">
          <div class="plan-head">
            <span class="plan-title">{t('backup.planWhat')}</span>
            {#if busy}<span class="plan-run"><Spinner size={13} /> {t('common.busy')}</span>
            {:else if restoreDone}<span class="plan-done status-ok">{t('backup.restoreDone')}</span>{/if}
          </div>
          <ul class="plan-summary">
            <li>{t('backup.planProfiles', { n: selected.length, list: selected.join(', ') })}</li>
            <li>{includeCreds ? t('backup.planCredsOn') : t('backup.planCredsOff')}</li>
            <li>{t('backup.planUntouched')}</li>
          </ul>
          <details class="plan-raw">
            <summary>{t('backup.planDetails')}</summary>
            <pre class="plan-body">{log.join('\n')}</pre>
          </details>
        </div>
      {/if}

      <div class="row">
        <button class="sw-btn {restoreDone ? 'sw-btn-primary' : 'sw-btn-ghost'}" onclick={onClose} title={t('backup.closeTitle')}>{t('common.close')}</button>
        <button class="sw-btn sw-btn-ghost" disabled={busy || selected.length === 0} onclick={preview}
          title={t('backup.previewTitle')}>
          {t('backup.showPlan')}
        </button>
        <button
          class="sw-btn sw-btn-danger"
          disabled={busy || !hasPreviewed || selected.length === 0}
          onclick={restore}
          title={hasPreviewed
            ? t('backup.restoreTitle')
            : t('backup.restoreNeedsPreview')}
        >
          {t('backup.restore')}
        </button>
      </div>
</ModalShell>

<style>
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
  .section-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: var(--sw-space-2);
  }
  .section-title {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sw-text-muted);
  }
  .selall {
    border: none;
    background: transparent;
    color: var(--sw-accent-text);
    cursor: pointer;
    font-size: var(--sw-text-xs);
    padding: 0;
  }
  .selall:hover {
    text-decoration: underline;
  }
  .profiles {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sw-space-2);
  }
  .pchip {
    padding: 4px 12px;
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    background: transparent;
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-sm);
    cursor: pointer;
  }
  .pchip:hover {
    color: var(--sw-text-primary);
  }
  .pchip.on {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
    border-color: var(--sw-accent);
  }
  .creds {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
    margin-bottom: var(--sw-space-4);
    font-size: var(--sw-text-sm);
    color: var(--sw-text);
    cursor: pointer;
  }
  .warn {
    margin: 0 0 var(--sw-space-6);
    font-size: var(--sw-text-sm);
    color: var(--sw-warn);
    line-height: 1.5;
  }
  .plan {
    margin: 0 0 var(--sw-space-4);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    /* --sw-bg is undefined (only --sw-bg-primary/-secondary exist) → was transparent; use the
       opaque nested-panel token so the plan list has a solid surface in both themes. */
    background: var(--sw-bg-secondary);
    overflow: hidden;
  }
  .plan-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sw-space-2) var(--sw-space-3);
    border-bottom: 1px solid var(--sw-border);
  }
  .plan-title {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sw-text-muted);
  }
  .plan-run {
    display: inline-flex;
    align-items: center;
    gap: var(--sw-space-2);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .plan-done {
    font-size: var(--sw-text-xs);
    font-weight: 600;
  }
  .plan-summary {
    margin: 0;
    padding: var(--sw-space-3) var(--sw-space-3) var(--sw-space-3) calc(var(--sw-space-3) + 1.1em);
    list-style: none;
    font-size: var(--sw-text-sm);
    line-height: 1.5;
    color: var(--sw-text);
  }
  .plan-summary li {
    position: relative;
    margin: 0 0 var(--sw-space-1);
  }
  .plan-summary li:last-child {
    margin-bottom: 0;
  }
  .plan-summary li::before {
    content: '•';
    position: absolute;
    left: -1.1em;
    color: var(--sw-accent);
  }
  .plan-raw {
    border-top: 1px solid var(--sw-border);
  }
  .plan-raw > summary {
    padding: var(--sw-space-2) var(--sw-space-3);
    cursor: pointer;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    user-select: none;
  }
  .plan-raw > summary:hover {
    color: var(--sw-text-secondary);
  }
  .plan-body {
    margin: 0;
    max-height: 200px;
    overflow: auto;
    padding: 0 var(--sw-space-3) var(--sw-space-3);
    font-family: monospace;
    font-size: var(--sw-text-xs);
    line-height: 1.5;
    color: var(--sw-text-muted);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
  }
</style>
