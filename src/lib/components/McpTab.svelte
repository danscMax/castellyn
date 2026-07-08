<script lang="ts">
  import type { McpStatus, McpServer } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import EmptyState from './EmptyState.svelte';
  import DataTable, { type DTColumn } from './DataTable.svelte';
  import ModalShell from './ModalShell.svelte';
  import { Server } from '@lucide/svelte';
  import SectionHeader from './SectionHeader.svelte';

  let {
    data,
    running,
    onRefresh,
    onDeploy,
    onUpsert,
    onRemoveServer,
    onRemoveExtra
  }: {
    data: McpStatus | null;
    running: string | null;
    onRefresh: () => void;
    onDeploy: (target?: string | string[]) => void;
    onUpsert: (name: string, definition: string) => Promise<void>;
    onRemoveServer: (name: string) => void;
    onRemoveExtra: (name: string, profile: string) => void;
  } = $props();

  const busy = $derived(!!running);

  // --- Add/edit a canonical server (config\.mcp.json) ---
  const DEFAULT_DEF = '{\n  "command": "npx",\n  "args": []\n}';
  let formOpen = $state(false);
  let formName = $state('');
  let formJson = $state(DEFAULT_DEF);
  let formEditing = $state(false); // true = name locked (editing an existing server)
  let formError = $state('');
  let submitting = $state(false);
  // U4: initial values to detect unsaved edits before discarding the form; a confirm gates close.
  let formInitName = $state('');
  let formInitJson = $state(DEFAULT_DEF);
  let confirmDiscard = $state(false);
  function openAdd() {
    formName = '';
    formJson = DEFAULT_DEF;
    formInitName = '';
    formInitJson = DEFAULT_DEF;
    formEditing = false;
    formError = '';
    confirmDiscard = false;
    formOpen = true;
  }
  function openEdit(srv: McpServer) {
    formName = srv.name;
    formJson = JSON.stringify(srv.definition, null, 2);
    formInitName = formName;
    formInitJson = formJson;
    formEditing = true;
    formError = '';
    confirmDiscard = false;
    formOpen = true;
  }
  const formDirty = $derived(formName !== formInitName || formJson !== formInitJson);
  function requestClose() {
    // U4: don't silently drop typed JSON — confirm first if the form has unsaved changes.
    if (formDirty) {
      confirmDiscard = true;
      return;
    }
    formOpen = false;
  }
  function discardAndClose() {
    confirmDiscard = false;
    formOpen = false;
  }
  async function submitForm() {
    if (submitting) return;
    if (!formName.trim()) {
      formError = t('mcp.errEmptyName');
      return;
    }
    try {
      JSON.parse(formJson); // validate before sending; backend re-checks
    } catch (e) {
      formError = `${t('mcp.errBadJson')}: ${e}`;
      return;
    }
    submitting = true;
    formError = '';
    try {
      // U4: await the backend result — only close on success. A rejected upsert keeps the form
      // (and the typed JSON) open with the reason, instead of closing as if it saved.
      await onUpsert(formName.trim(), formJson);
      formOpen = false;
    } catch (e) {
      formError = String((e as { message?: string })?.message ?? e);
    } finally {
      submitting = false;
    }
  }
  // Bulk MCP deploy (#76): pick profiles, deploy to all of them in one run.
  let bulkSel = $state<Record<string, boolean>>({});
  const bulkCount = $derived(Object.values(bulkSel).filter(Boolean).length);
  function toggleBulk(p: string) {
    bulkSel = { ...bulkSel, [p]: !bulkSel[p] };
  }
  function deployBulk() {
    const only = ALL_PROFILES.filter((p) => bulkSel[p]);
    if (only.length) onDeploy(only);
  }
  // Real profile list from the backend (read_mcp). Empty until it resolves — do NOT fall back to a
  // canned cc1..cc5 list: a fresh user (or anyone whose profiles are named differently) would see
  // wrong chips and a lying n/total badge. Empty-on-first-paint is honest; the chips fill in on load.
  const ALL_PROFILES = $derived(data?.profiles ?? []);
  // Provided by the plugin marketplace, not deployed per-profile (installer skips them).
  const PLUGIN_PROVIDED = ['context7', 'serena'];

  const source = $derived(data?.source ?? []);
  const extras = $derived(data?.extras ?? []);

  function isPlugin(name: string) {
    return PLUGIN_PROVIDED.includes(name);
  }

  // Surface servers that still need attention (deployed to fewer profiles than exist) first;
  // fully-deployed next; plugin-provided (not deployable) last.
  function rank(srv: { name: string; deployedIn: string[] }): number {
    if (isPlugin(srv.name)) return 2;
    return srv.deployedIn.length < ALL_PROFILES.length ? 0 : 1;
  }
  const sortedSource = $derived([...source].sort((a, b) => rank(a) - rank(b)));

  const COLS: DTColumn[] = $derived([
    { key: 'name', label: t('mcp.colName'), grow: true, sortable: true },
    // V1: budget the fixed columns to the real content — profiles (6 chips / the plugin note)
    // needs the room the truncating monospace command column was hogging; at the old widths the
    // note and the last chip clipped against the actions column on a 1440px window.
    { key: 'command', label: t('mcp.colCommand'), width: '200px' },
    { key: 'deployed', label: t('mcp.colDeployed'), width: '100px', align: 'center', sortable: true },
    { key: 'profiles', label: t('mcp.colProfiles'), width: '300px', interactive: true },
    { key: 'actions', label: t('mcp.colActions'), width: '200px', align: 'right', interactive: true }
  ]);
  type Srv = (typeof sortedSource)[number];
  function sortVal(s: Srv, key: string): string | number {
    if (key === 'deployed') return rank(s) * 100 + s.deployedIn.length;
    return s.name.toLowerCase();
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('mcp.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('mcp.subtitle')}
      </p>
    </div>
    <div class="flex shrink-0 gap-sw-2">
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={onRefresh}
        title={t('mcp.refreshTitle')}>
        {running === 'mcp' ? t('common.busy') : t('common.refresh')}
      </button>
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={openAdd}
        title={t('mcp.addServerTitle')}>
        {t('mcp.addServer')}
      </button>
      <button class="sw-btn sw-btn-primary" disabled={busy} onclick={() => onDeploy()}
        title={t('mcp.deployTitle')}>
        {t('mcp.deployAll')}
      </button>
    </div>
  </header>

  {#if source.length}
    <DataTable
      columns={COLS}
      rows={sortedSource}
      rowKey={(s) => s.name}
      sortAccessor={sortVal}
      highlightAttr={(s) => `mcp:${s.name}`}
      search
      searchValue={(s) => `${s.name} ${s.command}`}
      searchPlaceholder={t('mcp.colName')}
      storageKey="mcp"
    >
      {#snippet toolbar()}
        <span class="text-sw-xs text-sw-text-muted">{t('mcp.selectProfiles')}</span>
        {#each ALL_PROFILES as p (p)}
          <button class="badge {bulkSel[p] ? 'badge-info' : 'badge-muted'}" onclick={() => toggleBulk(p)}
            title={t('mcp.selectProfileTip', { p })}>{p}</button>
        {/each}
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy || !bulkCount} onclick={deployBulk}
          title={t('mcp.bulkDeployTip')}>
          {t('mcp.bulkDeploy')}{bulkCount ? ` (${bulkCount})` : ''}
        </button>
      {/snippet}

      {#snippet cell(srv, col)}
        {#if col.key === 'name'}
          <span class="font-medium truncate" title={srv.name}>{srv.name}</span>
        {:else if col.key === 'command'}
          <span class="font-mono text-sw-xs text-sw-text-muted truncate block" title={srv.command}>{srv.command}</span>
        {:else if col.key === 'deployed'}
          {#if isPlugin(srv.name)}
            <span class="badge badge-info" title={t('mcp.pluginBadgeTitle')}>{t('mcp.pluginBadge')}</span>
          {:else}
            <span class="badge {srv.deployedIn.length === ALL_PROFILES.length ? 'badge-ok' : srv.deployedIn.length > 0 ? 'badge-warn' : 'badge-err'}"
              title={t('mcp.deployedCountTitle', { n: srv.deployedIn.length, total: ALL_PROFILES.length })}>
              {srv.deployedIn.length}/{ALL_PROFILES.length}
            </span>
          {/if}
        {:else if col.key === 'profiles'}
          {#if isPlugin(srv.name)}
            <span class="text-sw-xs text-sw-text-muted" title={t('mcp.pluginNote')}>{t('mcp.pluginNote')}</span>
          {:else}
            <div class="flex flex-wrap gap-sw-1">
              {#each ALL_PROFILES as p (p)}
                {@const ok = srv.deployedIn.includes(p)}
                {#if ok}
                  <span class="badge badge-ok" title={t('mcp.profileDeployedTitle', { p })}>{p}</span>
                {:else}
                  <button class="badge badge-muted" disabled={busy} onclick={() => onDeploy(p)}
                    title={t('mcp.deployToProfileTip', { p })}>{p}</button>
                {/if}
              {/each}
            </div>
          {/if}
        {:else if col.key === 'actions'}
          <div class="flex justify-end gap-sw-1">
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => openEdit(srv)}
              title={t('mcp.editServerTitle')}>{t('common.edit')}</button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onRemoveServer(srv.name)}
              title={t('mcp.removeServerTitle')}>{t('common.delete')}</button>
          </div>
        {/if}
      {/snippet}
    </DataTable>
  {:else if data === null}
    <!-- First open: skeleton rows until read_mcp resolves, instead of a misleading "empty" pane. -->
    <div class="flex flex-col gap-sw-2">
      {#each Array(4) as _, i (i)}
        <div class="skeleton" style="height:2.4rem;width:100%"></div>
      {/each}
    </div>
  {:else}
    <EmptyState icon={Server} title={t('mcp.emptyTitle')} description={t('mcp.emptyHint')} />
  {/if}

  {#if extras.length}
    <SectionHeader title={t('mcp.extrasHeading')} />
    <div class="sw-card flex flex-col gap-sw-2">
      <p class="text-sw-xs text-sw-text-muted">
        {t('mcp.extrasNote')}
      </p>
      {#each extras as ex (ex.name)}
        <div class="flex items-center justify-between gap-sw-2 text-sw-sm">
          <span class="font-mono text-sw-text">{ex.name}</span>
          <div class="flex flex-wrap gap-sw-2">
            {#each ex.presentIn as p (p)}<button class="badge badge-warn" disabled={busy}
              onclick={() => onRemoveExtra(ex.name, p)} title={t('mcp.removeExtraTitle', { p })}>{p} ✕</button>{/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<ModalShell open={formOpen} onClose={requestClose} size="md">
  <h3 class="dlg-h">{formEditing ? t('mcp.editServerTitle') : t('mcp.addServerTitle')}</h3>
  <label class="dlg-fld">
    <span>{t('mcp.formName')}</span>
    <input class="sw-input" bind:value={formName} readonly={formEditing} placeholder="my-server"
      spellcheck="false" autocomplete="off" />
  </label>
  <label class="dlg-fld">
    <span>{t('mcp.formJson')}</span>
    <textarea class="sw-input font-mono text-sw-xs" bind:value={formJson} rows="8" spellcheck="false"></textarea>
  </label>
  {#if formError}<p class="warn text-sw-xs">{formError}</p>{/if}
  {#if confirmDiscard}
    <div class="dlg-row items-center">
      <span class="mr-auto text-sw-xs text-sw-text-secondary">{t('mcp.unsavedMsg')}</span>
      <button class="sw-btn sw-btn-ghost" onclick={() => (confirmDiscard = false)}>{t('mcp.keepEditing')}</button>
      <button class="sw-btn sw-btn-danger" onclick={discardAndClose}>{t('mcp.discardEdits')}</button>
    </div>
  {:else}
    <div class="dlg-row">
      <button class="sw-btn sw-btn-ghost" onclick={requestClose}>{t('common.cancel')}</button>
      <button class="sw-btn sw-btn-primary" disabled={submitting} onclick={submitForm}>
        {submitting ? t('common.busy') : t('common.save')}
      </button>
    </div>
  {/if}
</ModalShell>
