<script lang="ts">
  import { openPath, type SyncStatus, type SyncItem, type ConfigDriftStatus, type ConfigDriftAction } from '$lib/ipc';
  import Toggle from './Toggle.svelte';
  import { t } from '$lib/i18n';

  let {
    data,
    running,
    onRefresh,
    onApply,
    driftData = null,
    conflictCount = 0,
    onDriftApply,
    onCleanConflicts
  }: {
    data: SyncStatus | null;
    running: string | null;
    onRefresh: () => void;
    onApply: (enabled: string[]) => void;
    driftData?: ConfigDriftStatus | null;
    conflictCount?: number;
    onDriftApply?: (action: ConfigDriftAction) => void;
    onCleanConflicts?: () => void;
  } = $props();

  const busy = $derived(!!running);

  // ISO timestamp (PowerShell Get-Date -Format 'o') -> "N min ago" style label.
  function fmtAgo(ms?: string) {
    if (!ms) return '';
    const then = Date.parse(ms);
    if (isNaN(then)) return '';
    const ago = Math.round((Date.now() - then) / 1000);
    if (ago < 60) return t('common.justNow');
    if (ago < 3600) return t('common.minutesAgo', { n: Math.floor(ago / 60) });
    if (ago < 86400) return t('common.hoursAgo', { n: Math.floor(ago / 3600) });
    return t('common.daysAgo', { n: Math.floor(ago / 86400) });
  }

  // Static descriptors; user-facing label/desc are resolved reactively via t() in markup.
  const ITEMS: { key: SyncItem; labelKey: string; path: string; descKey: string }[] = [
    { key: 'history', labelKey: 'sync.itemHistoryLabel', path: 'history.jsonl', descKey: 'sync.itemHistoryDesc' },
    { key: 'projects', labelKey: 'sync.itemProjectsLabel', path: 'projects/', descKey: 'sync.itemProjectsDesc' },
    { key: 'skills', labelKey: 'sync.itemSkillsLabel', path: 'skills/', descKey: 'sync.itemSkillsDesc' },
    { key: 'agents', labelKey: 'sync.itemAgentsLabel', path: 'agents/', descKey: 'sync.itemAgentsDesc' },
    { key: 'commands', labelKey: 'sync.itemCommandsLabel', path: 'commands/', descKey: 'sync.itemCommandsDesc' },
    { key: 'keybindings', labelKey: 'sync.itemKeybindingsLabel', path: 'keybindings.json', descKey: 'sync.itemKeybindingsDesc' }
  ];

  // Local editable selection, re-seeded whenever a fresh snapshot arrives.
  // Plain (non-reactive) guard so re-seeding never loops.
  let seededAt: string | undefined = undefined;
  let sel = $state<Record<string, boolean>>(Object.fromEntries(ITEMS.map((i) => [i.key, true])));
  $effect(() => {
    if (data?.generatedAt && data.generatedAt !== seededAt) {
      const items = (data.items ?? {}) as Record<string, boolean>;
      sel = Object.fromEntries(ITEMS.map((i) => [i.key, items[i.key] !== false]));
      seededAt = data.generatedAt;
    }
  });

  const dirty = $derived.by(() => {
    const items = (data?.items ?? {}) as Record<string, boolean>;
    return ITEMS.some((i) => (sel[i.key] ?? true) !== (items[i.key] !== false));
  });

  const st = $derived(data?.syncthing);

  function fmtBytes(n?: number) {
    if (n === undefined || n === null) return t('common.dash');
    // Read units reactively from the dictionary (re-runs on locale change via markup).
    const u = t('sync.byteUnits').split(',');
    let v = n;
    let i = 0;
    while (v >= 1024 && i < u.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
  }

  function stateLabel(s?: string) {
    if (s === 'idle') return t('sync.stateIdle');
    if (s === 'syncing') return t('sync.stateSyncing');
    if (s === 'scanning') return t('sync.stateScanning');
    if (s === 'error') return t('sync.stateError');
    return s ?? t('common.dash');
  }

  function apply() {
    onApply(ITEMS.filter((i) => sel[i.key]).map((i) => i.key));
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('sync.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('sync.subtitle')}
      </p>
    </div>
    <button class="sw-btn sw-btn-ghost shrink-0" disabled={busy} onclick={onRefresh}
      title={t('sync.refreshTitle')}>
      {running === 'sync' ? t('common.busy') : t('common.refresh')}
    </button>
  </header>

  {#if data}
    <!-- Syncthing status -->
    <div class="sw-card mb-sw-4">
      <div class="mb-sw-2 flex items-center gap-sw-2">
        <span class="font-medium">{t('sync.syncthing')}</span>
        {#if st?.available}
          <span class="badge badge-ok" title={t('sync.daemonTitle')}>{t('sync.connected')}{st.version ? ` · ${st.version}` : ''}</span>
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openPath('http://localhost:8384')}
            title={t('sync.openWebUiTip')}>{t('sync.openWebUi')}</button>
        {:else}
          <span class="badge badge-warn" title={t('sync.notFoundTitle')}>{t('sync.notFound')}</span>
        {/if}
      </div>
      {#if st?.available && st.folderShared}
        <dl class="grid grid-cols-2 gap-x-sw-6 gap-y-1 text-sw-sm md:grid-cols-4">
          <div>
            <dt class="text-sw-xs text-sw-text-muted">{t('sync.folder')}</dt>
            <dd title={t('sync.folderIdTitle', { id: st.folderId ?? '' })}>{st.folderLabel ?? st.folderId ?? t('common.dash')}</dd>
          </div>
          <div>
            <dt class="text-sw-xs text-sw-text-muted">{t('sync.state')}</dt>
            <dd>{stateLabel(st.state)}</dd>
          </div>
          <div>
            <dt class="text-sw-xs text-sw-text-muted">{t('sync.completion')}</dt>
            <dd title={t('sync.completionTitle')}>{st.completion ?? t('common.dash')}% · {fmtBytes(st.globalBytes)}</dd>
          </div>
          <div>
            <dt class="text-sw-xs text-sw-text-muted" title={t('sync.connectedDevicesTitle')}>
              {t('sync.connectedDevices')}
            </dt>
            <dd>{st.connectedDevices ?? 0}</dd>
          </div>
        </dl>
      {:else if st?.available}
        <p class="text-sw-sm text-sw-text-muted">{t('sync.folderNotShared')}</p>
      {:else}
        <p class="text-sw-sm text-sw-text-muted">
          {t('sync.noSyncthingYet')}
        </p>
      {/if}
    </div>

    <!-- Config-file drift (shared config: statusline.py, CLAUDE.md, RTK.md, hooks, ...) -->
    {#if driftData}
      {@const drifted = driftData.drifted ?? 0}
      {@const unlinked = driftData.unlinked ?? 0}
      <div class="sw-card mb-sw-4 {drifted > 0 || unlinked > 0 ? 'border border-amber-500/40' : ''}">
        <div class="mb-sw-2 flex items-center gap-sw-2">
          <span class="font-medium">{t('sync.configDrift')}</span>
          {#if drifted > 0}
            <span class="badge badge-warn">{t('sync.driftedBadge', { n: drifted })}</span>
          {:else if unlinked > 0}
            <span class="badge badge-warn">{t('sync.unlinkedBadge', { n: unlinked })}</span>
          {:else}
            <span class="badge badge-ok">{t('sync.configOk')}</span>
          {/if}
          {#if driftData.generatedAt}<span class="text-sw-xs text-sw-text-muted">{t('sync.checkedAt', { time: fmtAgo(driftData.generatedAt) })}</span>{/if}
        </div>
        <p class="text-sw-sm text-sw-text-secondary mb-sw-3">{t('sync.configDriftDesc')}</p>
        <div class="flex flex-wrap gap-sw-2">
          <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={() => onDriftApply?.('check')}
            title={t('sync.driftCheckTip')}>{t('sync.driftCheckBtn')}</button>
          {#if drifted > 0}
            <button class="sw-btn" disabled={busy} onclick={() => onDriftApply?.('sync-now')}
              title={t('sync.syncNowTip')}>{t('sync.syncNowBtn')}</button>
          {/if}
          {#if drifted > 0 || unlinked > 0}
            <button class="sw-btn" disabled={busy} onclick={() => onDriftApply?.('relink')}
              title={t('sync.relinkTip')}>{t('sync.relinkBtn')}</button>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Sync conflicts (USE-8) -->
    {#if conflictCount > 0}
      <div class="sw-card mb-sw-4 flex items-center gap-sw-2 border border-amber-500/40 text-sw-sm">
        <span class="badge badge-warn">{t('sync.conflictsBadge', { n: conflictCount })}</span>
        <span class="text-sw-text-secondary">{t('sync.conflictsDesc')}</span>
        {#if onCleanConflicts}
          <button class="sw-btn sw-btn-ghost ml-auto" disabled={busy} onclick={onCleanConflicts}
            title={t('sync.cleanConflictsTip')}>{t('sync.cleanConflictsBtn')}</button>
        {/if}
      </div>
    {/if}

    <!-- Drift warning -->
    {#if data.stignoreExists && data.stignoreMatches === false}
      <div class="sw-card mb-sw-4 border border-amber-500/40 text-sw-sm">
        <span class="badge badge-warn">{t('sync.needsApplyBadge')}</span>
        {t('sync.driftWarning')}
      </div>
    {/if}

    <!-- Item toggles -->
    <h2 class="mb-sw-2 text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">{t('sync.whatToSync')}</h2>
    <div class="card-grid">
      {#each ITEMS as item (item.key)}
        <div class="sw-card flex items-start gap-sw-3" title={t('sync.itemTitle', { path: item.path })}>
          <div class="mt-0.5"><Toggle bind:checked={sel[item.key]} disabled={busy} title={t('sync.itemToggleTip')} /></div>
          <div class="min-w-0">
            <div class="flex items-center gap-sw-2">
              <span class="font-medium">{t(item.labelKey)}</span>
              <span class="font-mono text-[11px] text-sw-text-muted">{item.path}</span>
            </div>
            <p class="text-sw-xs text-sw-text-secondary">{t(item.descKey)}</p>
          </div>
        </div>
      {/each}
    </div>

    <div class="mt-sw-4 flex items-center gap-sw-3">
      <button class="sw-btn" disabled={busy || !dirty} onclick={apply}
        title={t('sync.applyTitle')}>
        {t('common.apply')}
      </button>
      {#if dirty}<span class="text-sw-xs text-amber-400">{t('sync.unsavedChanges')}</span>{/if}
      {#if !dirty && data.stignoreMatches !== false}<span class="text-sw-xs text-sw-text-muted">{t('sync.allApplied')}</span>{/if}
    </div>
    <p class="mt-sw-2 text-sw-xs text-sw-text-muted">
      {t('sync.footnote')}
    </p>
  {:else}
    <div class="grid place-items-center py-sw-6 text-center text-sw-text-muted">
      <div>
        <div class="mb-sw-2 text-2xl">⇄</div>
        <div class="font-medium text-sw-text">{t('sync.emptyTitle')}</div>
        <div class="text-sw-sm">{t('sync.emptyHint')}</div>
      </div>
    </div>
  {/if}
</div>
