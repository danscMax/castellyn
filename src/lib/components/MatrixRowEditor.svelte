<script lang="ts">
  // B3: per-profile config editor rendered inside a profile row's expand detail — provider, proxy
  // and the folders/plugins/mcp chips. Edits accumulate in the shared MatrixState; the popover,
  // Apply/Discard bar and preview modal are rendered once by MatrixControls.
  import type { MatrixState } from '$lib/matrixState.svelte';
  import { t } from '$lib/i18n';
  import Select from './Select.svelte';

  let { st, name }: { st: MatrixState; name: string } = $props();
  const row = $derived(st.rowByName.get(name));
</script>

{#if row}
  {@const r = row}
  <div class="matrix-editor">
    <div class="me-field">
      <span class="me-label">{t('profiles.colProvider')}</span>
      <Select bind:value={st.draft[name].provider} options={st.providerOptions} disabled={st.busy || st.applying} />
    </div>
    <div class="me-field">
      <span class="me-label">{t('profiles.matrixColProxy')}</span>
      <input
        class="sw-input text-sw-sm"
        bind:value={st.draft[name].proxy}
        placeholder={t('profiles.matrixProxyNone')}
        spellcheck="false"
        autocomplete="off"
        disabled={st.busy || st.applying}
        title={t('profiles.matrixProxyTip')}
      />
      {#if !st.proxyValid(name)}<span class="warn status-warn">{t('profiles.matrixProxyInvalid')}</span>{/if}
    </div>
    <div class="me-chipwrap">
      <span class="me-label">{t('profiles.matrixColFolders')}</span>
      <button
        type="button"
        class="chip"
        class:warn={st.folderWarn(r, name)}
        disabled={st.busy || st.applying}
        onclick={(e) => st.togglePop(name, 'folders', e.currentTarget)}
        title={t('profiles.matrixFoldersTip')}
      >
        {st.draft[name].folders.length}/{r.folders.length}
      </button>
    </div>
    <div class="me-chipwrap">
      <span class="me-label">{t('profiles.matrixColPlugins')}</span>
      <button
        type="button"
        class="chip"
        class:dirtychip={st.pluginsChanged(name)}
        disabled={st.busy || st.applying}
        onclick={(e) => st.togglePop(name, 'plugins', e.currentTarget)}
        title={t('profiles.matrixPluginsTip')}
      >
        {st.pluginOnCount(name)}/{r.plugins.length}
      </button>
    </div>
    <div class="me-chipwrap">
      <span class="me-label">{t('profiles.matrixColMcp')}</span>
      <button
        type="button"
        class="chip"
        class:warn={st.mcpWarn(r)}
        disabled={st.busy}
        onclick={(e) => st.togglePop(name, 'mcp', e.currentTarget)}
        title={t('profiles.matrixMcpTip')}
      >
        {r.mcp.deployed.length}/{r.mcp.canon.length}{#if r.mcp.extras.length}&nbsp;+{r.mcp.extras.length}{/if}
      </button>
    </div>
  </div>
{/if}

<style>
  .matrix-editor {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sw-space-4);
    align-items: flex-end;
  }
  .me-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 12rem;
    flex: 1 1 12rem;
    max-width: 22rem;
  }
  .me-chipwrap {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .me-label {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sw-text-muted);
    font-weight: 600;
  }
  .warn {
    display: block;
    margin-top: 2px;
    font-size: var(--sw-text-xs);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 11px;
    border: 1px solid var(--sw-border);
    border-radius: 99px;
    background: var(--sw-bg-secondary);
    color: var(--sw-text-primary);
    font-size: var(--sw-text-xs);
    cursor: pointer;
    align-self: flex-start;
  }
  .chip:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .chip.warn {
    border-color: var(--sw-warn);
  }
  .chip.dirtychip {
    border-color: var(--sw-accent);
  }
</style>
