<script lang="ts">
  import { onMount } from 'svelte';
  import TerminalPane from './TerminalPane.svelte';
  import SessionLaunchDialog from './SessionLaunchDialog.svelte';
  import Toggle from './Toggle.svelte';
  import { t } from '$lib/i18n';
  import { pickFolder, sessionWrite, type SessionTool } from '$lib/ipc';

  const MAX_PANES = 12; // each pane is a pwsh+tool process — cap to keep the machine responsive

  let { profiles = [], visible = true }: { profiles?: string[]; visible?: boolean } = $props();

  type Pane = { key: string; profile: string; tool: SessionTool; cwd: string; args: string };
  // The key (not the profile) identifies a pane, so the same profile can run in several at once.
  let panes = $state<Pane[]>([]);
  let seq = 0;
  let columns = $state(2);
  let cwd = $state(''); // default folder for quick launches
  let maximized = $state<string | null>(null); // key of the pane shown full-screen, or null

  // Persisted prefs: column count + last folder used per profile (so re-launching a profile lands
  // in the same place).
  const FKEY = 'cmh-sessions-folders';
  const CKEY = 'cmh-sessions-cols';
  const WKEY = 'cmh-sessions-workspaces';
  const AKEY = 'cmh-sessions-askfolder';
  let lastFolders = $state<Record<string, string>>({});
  // When ON, a quick profile launch opens the folder picker first (ask every time).
  let askFolder = $state(false);
  // A workspace is a named set of session configs you can re-launch with one click.
  type WsConfig = { tool: SessionTool; profile: string; cwd: string; args: string };
  let workspaces = $state<Record<string, WsConfig[]>>({});
  onMount(() => {
    try {
      lastFolders = JSON.parse(localStorage.getItem(FKEY) ?? '{}');
      workspaces = JSON.parse(localStorage.getItem(WKEY) ?? '{}');
      askFolder = localStorage.getItem(AKEY) === '1';
      const c = Number(localStorage.getItem(CKEY));
      if (c >= 1 && c <= 3) columns = c;
    } catch {
      /* first run / private mode */
    }
  });
  $effect(() => {
    try {
      localStorage.setItem(CKEY, String(columns));
    } catch {
      /* ignore */
    }
  });
  function rememberFolder(profile: string, folder: string) {
    if (!profile) return;
    lastFolders = { ...lastFolders, [profile]: folder };
    try {
      localStorage.setItem(FKEY, JSON.stringify(lastFolders));
    } catch {
      /* ignore */
    }
  }

  // Broadcast: mirror keystrokes from any pane to every running session.
  let broadcast = $state(false);
  const sessionIds: Record<string, string> = {};
  function onIdChange(key: string, id: string | null) {
    if (id) sessionIds[key] = id;
    else delete sessionIds[key];
  }
  function broadcastInput(data: string) {
    for (const id of Object.values(sessionIds)) sessionWrite(id, data);
  }

  const atLimit = $derived(panes.length >= MAX_PANES);
  function addPane(v: { tool: SessionTool; profile: string; cwd: string; args: string }) {
    if (atLimit) return;
    const key = `${v.tool}:${v.profile || 'sh'}#${seq++}`;
    panes = [...panes, { key, profile: v.profile, tool: v.tool, cwd: v.cwd, args: v.args }];
    if (v.tool === 'claude') rememberFolder(v.profile, v.cwd);
  }
  $effect(() => {
    try {
      localStorage.setItem(AKEY, askFolder ? '1' : '0');
    } catch {
      /* ignore */
    }
  });
  // Quick launch: Claude under a profile. With "ask folder" on, prompt for the folder first
  // (cancel = don't launch); otherwise use the profile's remembered folder (or the default).
  async function quick(profile: string) {
    let dir = lastFolders[profile] ?? cwd;
    if (askFolder) {
      const picked = await pickFolder(dir);
      if (picked === null) return; // cancelled
      dir = picked;
    }
    addPane({ tool: 'claude', profile, cwd: dir, args: '' });
  }
  async function launchAll() {
    // Ask once for a shared folder rather than prompting per profile.
    let dir = cwd;
    if (askFolder) {
      const picked = await pickFolder(dir);
      if (picked === null) return;
      dir = picked;
    }
    for (const p of profiles) addPane({ tool: 'claude', profile: p, cwd: askFolder ? dir : (lastFolders[p] ?? cwd), args: '' });
  }
  function closePane(key: string) {
    panes = panes.filter((p) => p.key !== key);
    if (maximized === key) maximized = null;
  }
  function closeAll() {
    panes = [];
    maximized = null;
  }
  // Resizable columns: per-column fraction weights + draggable dividers between them.
  let colFr = $state<number[]>([1, 1]);
  let gridEl: HTMLDivElement | undefined = $state();
  $effect(() => {
    if (colFr.length !== columns) colFr = Array(columns).fill(1);
  });
  const colBounds = $derived.by(() => {
    const total = colFr.reduce((s, f) => s + f, 0) || 1;
    const out: number[] = [];
    let acc = 0;
    for (let i = 0; i < colFr.length - 1; i++) {
      acc += colFr[i];
      out.push((acc / total) * 100);
    }
    return out; // percent positions of each divider
  });
  function startResize(e: PointerEvent, k: number) {
    e.preventDefault();
    const w = gridEl?.clientWidth || 1;
    const total = colFr.reduce((s, f) => s + f, 0);
    const startX = e.clientX;
    const a = colFr[k];
    const b = colFr[k + 1];
    const move = (ev: PointerEvent) => {
      const dFr = ((ev.clientX - startX) / w) * total;
      const na = Math.max(0.25, a + dFr);
      const nb = Math.max(0.25, b - dFr);
      const next = [...colFr];
      next[k] = na;
      next[k + 1] = nb;
      colFr = next;
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function toggleMax(key: string) {
    maximized = maximized === key ? null : key;
  }

  // Launch dialog (tool / profile / folder / args).
  let dlgOpen = $state(false);
  let dlgProfile = $state('');
  function openDlg(profile = '') {
    dlgProfile = profile;
    dlgOpen = true;
  }
  function onDlgSubmit(v: { tool: SessionTool; profile: string; cwd: string; args: string }) {
    dlgOpen = false;
    addPane(v);
  }
  async function browseMain() {
    const dir = await pickFolder(cwd);
    if (dir) cwd = dir;
  }

  function duplicate(key: string) {
    const p = panes.find((x) => x.key === key);
    if (p) addPane({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args });
  }

  // Drag a pane's title bar over another to reorder (live, as you hover).
  let dragKey = $state<string | null>(null);
  function onDragStart(key: string) {
    dragKey = key;
  }
  function onDragEnter(targetKey: string) {
    if (!dragKey || dragKey === targetKey || maximized) return;
    const from = panes.findIndex((p) => p.key === dragKey);
    const to = panes.findIndex((p) => p.key === targetKey);
    if (from < 0 || to < 0) return;
    const next = [...panes];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    panes = next;
  }
  function onDrop() {
    dragKey = null;
  }

  // Tab-scoped shortcuts (only while the Sessions tab is shown): Ctrl+T new session, Alt+1/2/3 cols.
  function onKey(e: KeyboardEvent) {
    if (!visible) return;
    if (e.ctrlKey && !e.shiftKey && (e.key === 't' || e.key === 'T')) {
      e.preventDefault();
      openDlg();
    } else if (e.altKey && (e.key === '1' || e.key === '2' || e.key === '3')) {
      e.preventDefault();
      columns = Number(e.key);
    }
  }

  // ── Workspaces: save the current set of panes under a name, re-launch it later ──
  let savingWs = $state(false);
  let wsName = $state('');
  const wsNames = $derived(Object.keys(workspaces));
  function persistWs() {
    try {
      localStorage.setItem(WKEY, JSON.stringify(workspaces));
    } catch {
      /* ignore */
    }
  }
  function saveWorkspace() {
    const name = wsName.trim();
    if (!name || !panes.length) return;
    workspaces = {
      ...workspaces,
      [name]: panes.map((p) => ({ tool: p.tool, profile: p.profile, cwd: p.cwd, args: p.args }))
    };
    persistWs();
    savingWs = false;
    wsName = '';
  }
  function launchWorkspace(name: string) {
    for (const c of workspaces[name] ?? []) addPane(c);
  }
  function deleteWorkspace(name: string) {
    const { [name]: _drop, ...rest } = workspaces;
    workspaces = rest;
    persistWs();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="wrap">
  <header class="mb-sw-3 flex items-center justify-between gap-sw-4">
    <div class="flex items-baseline gap-sw-3 min-w-0">
      <h1 class="text-lg font-semibold">{t('sessions.title')}</h1>
      <p class="truncate text-sw-xs text-sw-text-muted">{t('sessions.subtitle')}</p>
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      {#if panes.length > 1}
        <label class="flex cursor-pointer items-center gap-1" title={t('sessions.broadcastTip')}>
          <Toggle bind:checked={broadcast} />
          <span class="text-sw-xs" class:text-sw-text-primary={broadcast} class:text-sw-text-secondary={!broadcast}>{t('sessions.broadcast')}</span>
        </label>
        <span class="text-sw-text-muted">·</span>
      {/if}
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.layout')}</span>
      {#each [1, 2, 3] as c (c)}
        <button class="sw-btn sw-btn-ghost text-sw-xs" class:active={columns === c} onclick={() => (columns = c)}
          title={t('sessions.layoutCols', { n: c })}>{c}</button>
      {/each}
      {#if panes.length}
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={closeAll} title={t('sessions.closeAllTip')}>
          {t('sessions.closeAll')}
        </button>
      {/if}
    </div>
  </header>

  <!-- Launcher: quick-launch a profile (Claude), or open the dialog for tool/folder/args -->
  <div class="launcher">
    <div class="cwd">
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.cwdDefault')}</span>
      <div class="flex items-center gap-sw-3">
        <input class="sw-input text-sw-xs" style="flex:1;min-width:0" bind:value={cwd} placeholder={t('sessions.cwdShort')} spellcheck="false" />
        <button class="sw-btn shrink-0" onclick={browseMain} title={t('sessions.browse')}>📁 {t('sessions.browse')}</button>
        <label class="ask shrink-0" title={t('sessions.askFolderTip')}>
          <Toggle bind:checked={askFolder} />
          <span class="whitespace-nowrap text-sw-xs text-sw-text-secondary">{t('sessions.askFolder')}</span>
        </label>
      </div>
    </div>
    <div class="profiles">
      <button class="sw-btn sw-btn-primary text-sw-xs" onclick={() => openDlg()} title={t('sessions.newSessionTip')}>
        + {t('sessions.newSession')}
      </button>
      {#each profiles as p (p)}
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => quick(p)} title={t('sessions.launchTip', { profile: p })}>
          ▶ {p}
        </button>
      {/each}
      {#if profiles.length > 1}
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={launchAll} title={t('sessions.launchAllTip')}>
          {t('sessions.launchAll')}
        </button>
      {/if}
      {#if savingWs}
        <input class="sw-input text-sw-xs" style="width:160px" bind:value={wsName} placeholder={t('sessions.wsNamePlaceholder')}
          onkeydown={(e) => e.key === 'Enter' && saveWorkspace()} />
        <button class="sw-btn sw-btn-primary text-sw-xs" disabled={!wsName.trim() || !panes.length} onclick={saveWorkspace}>{t('common.save')}</button>
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => (savingWs = false)}>{t('common.cancel')}</button>
      {:else}
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!panes.length} onclick={() => (savingWs = true)}
          title={t('sessions.wsSaveTip')}>{t('sessions.wsSave')}</button>
      {/if}
    </div>
  </div>

  {#if atLimit}
    <p class="mb-sw-2 text-sw-xs" style="color:#f59e0b">{t('sessions.limitNote', { n: MAX_PANES })}</p>
  {/if}

  <!-- Saved workspaces: one click re-opens the whole set of sessions -->
  {#if wsNames.length}
    <div class="workspaces">
      <span class="text-sw-xs text-sw-text-muted">{t('sessions.wsLabel')}</span>
      {#each wsNames as name (name)}
        <span class="ws-chip">
          <button class="ws-go" onclick={() => launchWorkspace(name)} title={t('sessions.wsLaunchTip', { name })}>
            ▶ {name} ({workspaces[name].length})
          </button>
          <button class="ws-del" onclick={() => deleteWorkspace(name)} title={t('sessions.wsDeleteTip', { name })} aria-label="✕">✕</button>
        </span>
      {/each}
    </div>
  {/if}

  {#if panes.length}
    <div
      class="grid"
      bind:this={gridEl}
      style="grid-template-columns: {maximized ? '1fr' : colFr.map((f) => `minmax(0, ${f}fr)`).join(' ')};"
    >
      <!-- Every pane stays MOUNTED (sessions must survive maximize); non-maximized ones are just
           hidden, so the maximized pane fills the single column. -->
      {#each panes as pane (pane.key)}
        <div class="cell" class:hidden={maximized != null && maximized !== pane.key}>
          <TerminalPane
            profile={pane.profile}
            tool={pane.tool}
            args={pane.args}
            cwd={pane.cwd || undefined}
            paneKey={pane.key}
            visible={visible && (maximized == null || maximized === pane.key)}
            maximized={maximized === pane.key}
            {broadcast}
            onInput={broadcastInput}
            {onIdChange}
            onNewSession={() => openDlg()}
            onClose={() => closePane(pane.key)}
            onToggleMax={() => toggleMax(pane.key)}
            onDuplicate={() => duplicate(pane.key)}
            {onDragStart}
            {onDragEnter}
            {onDrop}
          />
        </div>
      {/each}
      {#if !maximized && columns > 1}
        {#each colBounds as pos, k (k)}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <button type="button" class="col-divider" style="left:{pos}%"
            title={t('sessions.resizeCol')} aria-label={t('sessions.resizeCol')}
            onpointerdown={(e) => startResize(e, k)}></button>
        {/each}
      {/if}
    </div>
  {:else}
    <div class="empty">
      <div class="empty-icon">▦</div>
      <div class="font-medium text-sw-text">{t('sessions.emptyTitle')}</div>
      <div class="text-sw-sm text-sw-text-muted">{t('sessions.emptyHint')}</div>
    </div>
  {/if}

  <SessionLaunchDialog
    open={dlgOpen}
    {profiles}
    defaultProfile={dlgProfile}
    defaultCwd={cwd}
    onSubmit={onDlgSubmit}
    onCancel={() => (dlgOpen = false)}
  />
</div>

<style>
  .wrap {
    padding: var(--sw-space-4) var(--sw-space-6) var(--sw-space-3);
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .launcher {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--sw-space-3);
    margin-bottom: var(--sw-space-4);
    padding-bottom: var(--sw-space-3);
    border-bottom: 1px solid var(--sw-border);
  }
  .workspaces {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    margin-bottom: var(--sw-space-3);
  }
  .ws-chip {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--sw-border);
    border-radius: 9999px;
    overflow: hidden;
  }
  .ws-go {
    border: none;
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    padding: 3px 8px;
    font-size: var(--sw-text-xs);
  }
  .ws-go:hover {
    color: var(--sw-text-primary);
    background: var(--sw-accent-glow);
  }
  .ws-del {
    border: none;
    background: transparent;
    color: var(--sw-text-muted);
    cursor: pointer;
    padding: 3px 6px;
    font-size: 10px;
    border-left: 1px solid var(--sw-border);
  }
  .ws-del:hover {
    color: #f87171;
  }
  .ask {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .cwd {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 260px;
    flex: 1;
  }
  .profiles {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sw-space-2);
  }
  .cell {
    min-height: 0;
    min-width: 0;
    display: flex;
  }
  .cell.hidden {
    display: none;
  }
  .col-divider {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 10px;
    transform: translateX(-50%);
    border: none;
    background: transparent;
    cursor: col-resize;
    z-index: 4;
    padding: 0;
  }
  .col-divider::after {
    content: '';
    position: absolute;
    left: 50%;
    top: 0;
    bottom: 0;
    width: 2px;
    transform: translateX(-50%);
    background: transparent;
    transition: background 0.12s;
  }
  .col-divider:hover::after {
    background: var(--sw-accent-text);
  }
  .grid {
    position: relative;
    display: grid;
    gap: var(--sw-space-3);
    flex: 1;
    min-height: 0;
    /* Rows share the available height (so panes fill the page); they only scroll once
       there are too many to fit at a sensible minimum height. */
    grid-auto-rows: minmax(220px, 1fr);
    overflow-y: auto;
    padding-bottom: var(--sw-space-2);
  }
  .empty {
    flex: 1;
    display: grid;
    place-content: center;
    text-align: center;
    gap: 4px;
    color: var(--sw-text-muted);
  }
  .empty-icon {
    font-size: 2rem;
    opacity: 0.5;
  }
  .active {
    background: var(--sw-accent-glow);
    color: var(--sw-text-primary);
  }
</style>
