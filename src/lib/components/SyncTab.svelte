<script lang="ts">
  import { openPath, readDriftDiff, type SyncStatus, type SyncItem, type ConfigDriftStatus, type ConfigDriftAction, type DriftDiff } from '$lib/ipc';
  import SectionHeader from './SectionHeader.svelte';
  import NoScriptsBanner from './NoScriptsBanner.svelte';
  import Toggle from './Toggle.svelte';
  import { t, pConflict } from '$lib/i18n';
  import { relTime } from '$lib/relativeTime';
  import { fmtBytes as fmtBytesShared } from '$lib/bytes';

  let {
    data,
    running,
    onRefresh,
    onApply,
    driftData = null,
    conflictCount = 0,
    conflictFiles = [],
    onResolveConflict,
    onDriftApply,
    onCleanConflicts,
    scriptsAvail = true
  }: {
    data: SyncStatus | null;
    running: string | null;
    onRefresh: () => void;
    onApply: (enabled: string[]) => void;
    driftData?: ConfigDriftStatus | null;
    conflictCount?: number;
    conflictFiles?: string[];
    onResolveConflict?: (path: string, action: 'keep-local' | 'keep-other') => void;
    onDriftApply?: (action: ConfigDriftAction) => void;
    onCleanConflicts?: () => void;
    scriptsAvail?: boolean;
  } = $props();

  let showConflicts = $state(false);
  // Parent folder of a conflict file (both slash styles), for the "open folder" action.
  const parentDir = (p: string) => p.replace(/[\\/][^\\/]*$/, '');
  // Guard: no separator means parentDir(p) === p, so don't slice past the end.
  const baseName = (p: string) => {
    const d = parentDir(p);
    return d === p ? p : p.slice(d.length + 1);
  };

  const busy = $derived(!!running);

  // Static descriptors; user-facing label/desc are resolved reactively via t() in markup.
  const ITEMS: { key: SyncItem; labelKey: string; path: string; descKey: string }[] = [
    { key: 'history', labelKey: 'sync.itemHistoryLabel', path: 'history.jsonl', descKey: 'sync.itemHistoryDesc' },
    { key: 'projects', labelKey: 'sync.itemProjectsLabel', path: 'projects/', descKey: 'sync.itemProjectsDesc' },
    { key: 'skills', labelKey: 'sync.itemSkillsLabel', path: 'skills/', descKey: 'sync.itemSkillsDesc' },
    { key: 'agents', labelKey: 'sync.itemAgentsLabel', path: 'agents/', descKey: 'sync.itemAgentsDesc' },
    { key: 'commands', labelKey: 'sync.itemCommandsLabel', path: 'commands/', descKey: 'sync.itemCommandsDesc' },
    { key: 'keybindings', labelKey: 'sync.itemKeybindingsLabel', path: 'keybindings.json', descKey: 'sync.itemKeybindingsDesc' },
    { key: 'castellyn', labelKey: 'sync.itemCastellynLabel', path: 'castellyn/', descKey: 'sync.itemCastellynDesc' }
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
    // Units read reactively from the dictionary (re-runs on locale change via markup).
    return fmtBytesShared(n, t('sync.byteUnits'));
  }

  // L3: config-file drift states are internal enum words — show localized labels.
  const FILE_STATE_KEYS: Record<string, string> = {
    ok: 'sync.fstateOk',
    linked: 'sync.fstateLinked',
    master: 'sync.fstateMaster',
    drifted: 'sync.fstateDrifted',
    unlinked: 'sync.fstateUnlinked'
  };
  const fileStateLabel = (s: string) => (FILE_STATE_KEYS[s] ? t(FILE_STATE_KEYS[s]) : s);

  function stateLabel(s?: string) {
    if (s === 'idle') return t('sync.stateIdle');
    if (s === 'syncing') return t('sync.stateSyncing');
    if (s === 'scanning') return t('sync.stateScanning');
    if (s === 'error' || s === 'outofsync') return t('sync.stateError');
    return s ?? t('common.dash');
  }

  // Drift-diff expand state
  let expanded = $state<string | null>(null);
  let diffCache = $state<Record<string, DriftDiff>>({});
  let diffLoading = $state<Record<string, boolean>>({});

  async function toggleDiff(name: string) {
    if (expanded === name) {
      expanded = null;
      return;
    }
    expanded = name;
    if (diffCache[name]) return;
    diffLoading[name] = true;
    try {
      const d = await readDriftDiff(name);
      if (d) diffCache[name] = d;
    } catch {
      // ignore
    } finally {
      diffLoading[name] = false;
    }
  }

  function apply() {
    onApply(ITEMS.filter((i) => sel[i.key]).map((i) => i.key));
  }
</script>

<div class="p-sw-6">
  {#if !scriptsAvail}<NoScriptsBanner />{/if}
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
        {:else if st?.keyConfigured}
          <span class="badge badge-warn" title={t('sync.stConfiguredButDown')}>{t('sync.stConfiguredButDown')}</span>
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
          {#if driftData.generatedAt}<span class="text-sw-xs text-sw-text-muted">{t('sync.checkedAt', { time: relTime(driftData.generatedAt) })}</span>{/if}
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
      {#if driftData.items && driftData.items.length > 0}
        <div class="mt-sw-3 space-y-1">
          {#each driftData.items as item (item.name)}
            <div class="flex items-center gap-sw-2 rounded bg-sw-surface px-sw-3 py-1.5 text-sw-sm">
              <code class="font-mono flex-1 truncate">{item.name}</code>
              <span class="badge badge-{item.state === 'ok' || item.state === 'linked' || item.state === 'master' ? 'ok' : 'warn'}">{fileStateLabel(item.state)}</span>
              {#if item.state === 'drifted'}
                <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" onclick={() => toggleDiff(item.name)}
                  title={t('sync.diffTitle')}>
                  {expanded === item.name ? t('sync.hideDiff') : t('sync.showDiff')}
                </button>
              {/if}
            </div>
            {#if expanded === item.name}
              <div class="overflow-x-auto rounded bg-sw-surface-alt px-sw-3 py-sw-2 text-sw-xs font-mono leading-relaxed">
                {#if diffLoading[item.name]}
                  <span class="text-sw-text-muted">…</span>
                {:else if diffCache[item.name]}
                  <table class="w-full border-collapse">
                    <tbody>
                      {#each diffCache[item.name].lines as line}
                        <tr class="diff-{line.kind}">
                          <td class="w-4 select-none text-center text-sw-text-muted">
                            {line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : ' '}
                          </td>
                          <td class="whitespace-pre">{line.text}</td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                {:else}
                  <span class="text-sw-text-muted">{t('common.dash')}</span>
                {/if}
              </div>
            {/if}
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- Sync conflicts (USE-8) -->
  {#if conflictCount > 0}
      <div class="sw-card mb-sw-4 border border-amber-500/40 text-sw-sm">
        <div class="flex items-center gap-sw-2">
          <span class="badge badge-warn">{conflictCount} {pConflict(conflictCount)}</span>
          <span class="text-sw-text-secondary">{t('sync.conflictsDesc')}</span>
          {#if conflictFiles.length > 0 && onResolveConflict}
            <button class="sw-btn sw-btn-ghost ml-auto" onclick={() => (showConflicts = !showConflicts)}
              >{t('sync.conflictShow')}</button>
          {/if}
          {#if onCleanConflicts}
            <button class="sw-btn sw-btn-ghost {conflictFiles.length > 0 && onResolveConflict ? '' : 'ml-auto'}"
              disabled={busy} onclick={onCleanConflicts}
              title={t('sync.cleanConflictsTip')}>{t('sync.cleanConflictsBtn')}</button>
          {/if}
        </div>
        {#if showConflicts && onResolveConflict}
          <div class="mt-sw-3 space-y-1">
            {#each conflictFiles as f (f)}
              <div class="flex items-center gap-sw-2 rounded bg-sw-surface px-sw-3 py-1.5">
                <code class="font-mono flex-1 truncate" title={f}>{baseName(f)}</code>
                <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" onclick={() => openPath(parentDir(f))}
                  >{t('sync.folder')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy}
                  onclick={() => onResolveConflict(f, 'keep-local')}>{t('sync.keepLocal')}</button>
                <button class="sw-btn sw-btn-ghost text-sw-xs shrink-0" disabled={busy}
                  onclick={() => onResolveConflict(f, 'keep-other')}>{t('sync.keepOther')}</button>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    <!-- Drift warning — carries the Apply action itself so the banner isn't a dead reference to a
         greyed button far below. -->
    {#if data.stignoreExists && data.stignoreMatches === false}
      <div class="sw-card mb-sw-4 border border-amber-500/40 text-sw-sm flex items-center gap-sw-3 flex-wrap">
        <span class="badge badge-warn">{t('sync.needsApplyBadge')}</span>
        <span class="min-w-0 flex-1">{t('sync.driftWarning')}</span>
        <button class="sw-btn sw-btn-primary" disabled={busy} onclick={apply} title={t('sync.applyTitle')}>
          {t('common.apply')}
        </button>
      </div>
    {/if}

    <!-- Item toggles -->
    <SectionHeader title={t('sync.whatToSync')} />
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
      <button class="sw-btn" disabled={busy || (!dirty && data.stignoreMatches !== false)} onclick={apply}
        title={t('sync.applyTitle')}>
        {t('common.apply')}
      </button>
      {#if dirty}<span class="text-sw-xs status-warn">{t('sync.unsavedChanges')}</span>{/if}
      {#if !dirty && data.stignoreMatches !== false}<span class="text-sw-xs text-sw-text-muted">{t('sync.allApplied')}</span>{/if}
    </div>
    <p class="mt-sw-2 text-sw-xs text-sw-text-muted">
      {t('sync.footnote')}
    </p>

    <!-- What does NOT travel by sync — so a new machine isn't assumed fully migrated. -->
    <div class="sw-card mt-sw-4 text-sw-sm">
      <div class="mb-sw-1 font-medium">{t('sync.memoTitle')}</div>
      <p class="text-sw-text-secondary whitespace-pre-line">{t('sync.memoBody')}</p>
    </div>
  {:else}
    <!-- data is null only on first open (read_sync pending) — skeleton, not a misleading empty pane. -->
    <div class="flex flex-col gap-sw-2">
      {#each Array(5) as _, i (i)}
        <div class="skeleton" style="height:2.4rem;width:100%"></div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .diff-add td {
    background-color: color-mix(in srgb, var(--sw-status-up) 10%, transparent);
  }
  .diff-del td {
    background-color: color-mix(in srgb, var(--sw-status-down) 10%, transparent);
  }
</style>
