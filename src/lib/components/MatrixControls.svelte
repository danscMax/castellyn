<script lang="ts">
  // B3: the single (non-per-row) matrix controls — anchored popover (folders/plugins/mcp), the
  // sticky Apply/Discard bar and the preview modal. Rendered once by ProfilesTab; the per-row
  // editors (MatrixRowEditor) live inside each table row's expand. State lives in MatrixState.
  import type { MatrixState } from '$lib/matrixState.svelte';
  import { t } from '$lib/i18n';
  import { anchored } from '$lib/floating';
  import Toggle from './Toggle.svelte';
  import ModalShell from './ModalShell.svelte';

  let { st }: { st: MatrixState } = $props();
</script>

{#if st.popFor && st.popAnchor}
  {@const r = st.rowByName.get(st.popFor)}
  {#if r}
    <div class="popover" use:anchored={{ anchor: st.popAnchor, onOutside: () => (st.popFor = null) }}>
      {#if st.popKind === 'folders'}
        {#each r.folders as f (f.name)}
          <label class="poprow">
            <Toggle
              checked={st.draft[st.popFor].folders.includes(f.name)}
              disabled={st.busy || st.applying}
              onCheckedChange={(v) => st.toggleFolder(st.popFor!, f.name, v)}
              title={f.name}
            />
            <span class="font-mono text-sw-xs">{f.name}</span>
            {#if f.desired && f.actual !== 'linked'}
              <span class="status-warn text-sw-xs" title={t('profiles.matrixFolderRealTip')}>{f.actual === 'real' ? t('profiles.matrixFolderReal') : t('profiles.matrixFolderMissing')}</span>
            {/if}
          </label>
        {/each}
        <div class="warnnote status-warn">{t('profiles.matrixRelinkNote')}</div>
      {:else if st.popKind === 'plugins'}
        {#each r.plugins as p (p.id)}
          <label class="poprow">
            <Toggle
              checked={st.pluginOn(st.popFor, p)}
              disabled={st.busy || st.applying}
              onCheckedChange={(v) => st.togglePlugin(st.popFor!, p.id, v)}
              title={p.id}
            />
            <span class="min-w-0 flex-1 truncate text-sw-xs" title={p.id}>{st.pluginShort(p.id)}</span>
            {#if p.state === 'unset' && st.draft[st.popFor].plugins[p.id] === undefined}
              <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixPluginInherited')}</span>
            {:else if p.state === 'off'}
              <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixPluginOff')}</span>
            {/if}
          </label>
        {/each}
      {:else}
        {@const missing = st.mcpMissing(r)}
        {#if missing.length}
          <div class="popsec">{t('profiles.matrixMcpMissing')}</div>
          <div class="mcprow">
            <span class="min-w-0 flex-1 break-words font-mono text-sw-xs">{missing.join(', ')}</span>
            <button
              type="button"
              class="mcpbtn"
              disabled={st.busy}
              onclick={() => st.onMcpDeployProfile(st.popFor!)}
            >{t('profiles.matrixMcpDeployBtn')}</button>
          </div>
        {/if}
        {#if r.mcp.extras.length}
          <div class="popsec">{t('profiles.matrixMcpExtras')}</div>
          {#each r.mcp.extras as ex (ex)}
            <div class="mcprow">
              <span class="min-w-0 flex-1 truncate font-mono text-sw-xs" title={ex}>{ex}</span>
              <button
                type="button"
                class="xbtn status-warn"
                disabled={st.busy}
                onclick={() => st.onMcpRemoveExtra(ex, st.popFor!)}
                title={t('profiles.matrixMcpRemoveTip')}
              >✕</button>
            </div>
          {/each}
        {/if}
        {#if !missing.length && !r.mcp.extras.length}
          <div class="text-sw-xs text-sw-text-muted">{t('profiles.matrixMcpInSync')}</div>
        {/if}
      {/if}
    </div>
  {/if}
{/if}

{#if st.dirtyNames.length > 0}
  <div class="applybar has-changes">
    <span class="applybar-count">{t('profiles.matrixPending', { n: st.dirtyNames.length })}</span>
    <button class="sw-btn sw-btn-primary" disabled={!st.canApply} onclick={() => st.openPreview()} title={t('profiles.matrixApplyTip')}>
      {st.applying ? t('profiles.matrixApplying') : t('profiles.matrixApply', { n: st.dirtyNames.length })}
    </button>
    <button class="sw-btn sw-btn-ghost" disabled={st.applying} onclick={() => st.resetDraft()} title={t('profiles.matrixResetTip')}>
      {t('profiles.matrixReset')}
    </button>
    <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixNoWrite')}</span>
  </div>
{/if}

<ModalShell open={st.previewOpen} onClose={() => (st.previewOpen = false)} size="md">
  <h3 class="mb-sw-3 text-base font-semibold">{t('profiles.matrixPreviewTitle')}</h3>
  <div class="mb-sw-3 flex flex-col">
    {#each st.preview as c (c.who + '\u0000' + c.cat)}
      <div class="chg">
        <span class="who">{c.who}</span>
        <span class="cat">{c.cat}</span>
        <span class="min-w-0 break-words">{c.text}</span>
      </div>
    {/each}
  </div>
  <div class="flex items-center justify-end gap-sw-2">
    <button class="sw-btn sw-btn-ghost" onclick={() => (st.previewOpen = false)}>{t('profiles.matrixPreviewBack')}</button>
    <button class="sw-btn sw-btn-primary" onclick={() => st.confirmApply()}>{t('profiles.matrixPreviewConfirm')}</button>
  </div>
  <p class="mt-sw-2 text-sw-xs text-sw-text-muted">{t('profiles.matrixPreviewNote')}</p>
</ModalShell>

<style>
  .popover {
    position: fixed;
    z-index: 60;
    min-width: 210px;
    padding: var(--sw-space-3);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.35);
  }
  .poprow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
    cursor: pointer;
  }
  .warnnote {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--sw-border);
    font-size: var(--sw-text-xs);
  }
  .popsec {
    margin: 6px 0 2px;
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
  }
  .popsec:first-child {
    margin-top: 0;
  }
  .mcprow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
  }
  .xbtn {
    flex-shrink: 0;
    border: 1px solid var(--sw-border);
    border-radius: 6px;
    background: var(--sw-bg-secondary);
    font-size: 11px;
    line-height: 1;
    padding: 3px 6px;
    cursor: pointer;
  }
  .xbtn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .mcpbtn {
    flex-shrink: 0;
    border: 1px solid var(--sw-accent);
    border-radius: 6px;
    background: var(--sw-bg-secondary);
    color: var(--sw-accent);
    font-size: var(--sw-text-xs);
    line-height: 1;
    padding: 4px 8px;
    cursor: pointer;
  }
  .mcpbtn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .applybar {
    display: flex;
    align-items: center;
    gap: var(--sw-space-3);
    margin-top: var(--sw-space-4);
    /* Stick to the bottom of the viewport so pending edits + Apply stay reachable while scrolling. */
    position: sticky;
    bottom: 0;
    padding: var(--sw-space-3);
    border-radius: var(--sw-radius);
    background: color-mix(in srgb, var(--sw-accent) 10%, var(--sw-bg-primary));
    box-shadow: 0 -2px 8px rgb(0 0 0 / 0.12), inset 0 0 0 1px var(--sw-accent);
  }
  .applybar-count {
    font-size: var(--sw-text-sm);
    font-weight: 600;
    color: var(--sw-accent-text);
  }
  .chg {
    display: flex;
    gap: var(--sw-space-2);
    align-items: baseline;
    padding: 6px 2px;
    border-bottom: 1px solid var(--sw-border);
    font-size: var(--sw-text-sm);
  }
  .chg:last-child {
    border-bottom: none;
  }
  .chg .who {
    min-width: 64px;
    font-weight: 600;
  }
  .chg .cat {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
    white-space: nowrap;
  }
</style>
