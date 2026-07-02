<script lang="ts">
  import { onMount } from 'svelte';
  import { base } from '$app/paths';
  import { getVersion } from '@tauri-apps/api/app';
  import {
    readConfig,
    writeConfig,
    appPaths,
    openPath,
    getAutostart,
    setAutostart,
    pickSaveFile,
    pickOpenFile,
    exportConfig,
    importConfig,
    setToggleHotkey,
    readShortcuts,
    setShortcuts,
    setLanguage,
    type HubConfig,
    type AppPaths
  } from '$lib/ipc';
  import type { Theme } from '$lib/theme';
  import { t, locale, getLocaleName, type Locale } from '$lib/i18n';
  import { copyText } from '$lib/clipboard';
  import { pushToast } from '$lib/toast.svelte';
  import Toggle from './Toggle.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import EmptyState from './EmptyState.svelte';
  import { Search } from '@lucide/svelte';
  import { checkForUpdate, installUpdate, type UpdateInfo } from '$lib/updater';

  let {
    theme,
    onSetTheme,
    density = 'comfortable',
    fullWidth = false,
    onSetDensity,
    onSetFullWidth,
    confirmDestructive = true,
    onSetConfirmDestructive
  }: {
    theme: Theme;
    onSetTheme: (th: Theme) => void;
    density?: 'comfortable' | 'compact';
    fullWidth?: boolean;
    onSetDensity?: (d: 'comfortable' | 'compact') => void;
    onSetFullWidth?: (v: boolean) => void;
    confirmDestructive?: boolean;
    onSetConfirmDestructive?: (v: boolean) => void;
  } = $props();

  // #37: filter the settings cards by a free-text query matched against each section's labels.
  let query = $state('');
  function show(...labels: string[]): boolean {
    const q = query.trim().toLowerCase();
    return !q || labels.some((l) => l.toLowerCase().includes(q));
  }
  // True when a non-empty query matches no section — drives the "nothing found" empty state.
  // Keep these label groups in sync with the show(...) guards in the markup below.
  const noResults = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return false;
    const groups = [
      [t('settings.theme'), t('settings.themeDesc')],
      [t('settings.view'), t('settings.density'), t('settings.fullWidth'), t('settings.termScrollback')],
      [t('settings.language'), t('settings.languageDesc')],
      [t('settings.scriptsRoot'), t('settings.scriptsRootDesc')],
      [t('settings.launch'), t('settings.startWithWindows'), t('settings.startHidden'), t('settings.closeToTray'), t('settings.confirmDestructive'), t('settings.toggleHotkey'), t('settings.shortcutsSection')],
      [t('settings.timeouts'), t('settings.timeoutsDesc')],
      [t('settings.backupSection'), t('settings.exportConfig'), t('settings.importConfig')],
      [t('settings.about'), t('settings.version'), t('settings.scripts'), t('settings.config'), t('settings.checkUpdates'), t('settings.openScriptsFolder'), t('settings.openConfigFile'), t('settings.openStackFile'), t('settings.openBackupFolder')]
    ];
    return !groups.some((g) => g.some((l) => l.toLowerCase().includes(q)));
  });

  let cfg = $state<HubConfig>({});
  let scriptsRoot = $state('');
  let fetchTimeout = $state<number | ''>('');
  let ghTimeout = $state<number | ''>('');
  let autostart = $state(false);
  let startHidden = $state(false);
  let closeToTray = $state(true);
  // #123: OS-global show/hide accelerator (empty = off).
  let toggleHotkey = $state('');
  // Phase 4.1: full shortcut mapping
  let shortcuts = $state<Record<string, string>>({});
  // Known actions with their display labels
  const shortcutActions = ['toggle_window'] as const;
  const actionLabel = (a: string): string => {
    const map: Record<string, string> = {
      toggle_window: t('settings.shortcutsToggleWindow')
    };
    return map[a] ?? a;
  };
  let paths = $state<AppPaths | null>(null);
  let version = $state('');
  let savedMsg = $state('');
  let errMsg = $state('');

  // C3: auto-update flow. 'available' opens the release-notes confirm; 'downloading' shows progress.
  let updPhase = $state<'idle' | 'checking' | 'none' | 'available' | 'downloading' | 'error'>('idle');
  let updInfo = $state<UpdateInfo | null>(null);
  let updErr = $state('');
  let updPct = $state(0); // 0..100, or -1 when the server reports no content length
  async function onCheckUpdate() {
    updPhase = 'checking';
    updErr = '';
    try {
      const info = await checkForUpdate();
      if (info) {
        updInfo = info;
        updPhase = 'available';
      } else {
        updPhase = 'none';
      }
    } catch (e) {
      updErr = String(e);
      updPhase = 'error';
    }
  }
  async function onInstallUpdate() {
    if (!updInfo) return;
    updPhase = 'downloading';
    updPct = 0;
    try {
      await installUpdate(updInfo, (pct) => (updPct = pct));
      // installUpdate relaunches on success, so this rarely runs; reset so the UI isn't stuck on
      // "downloading" if relaunch is a no-op (e.g. dev build).
      updPhase = 'idle';
    } catch (e) {
      updErr = String(e);
      updPhase = 'error';
    }
  }
  // Terminal scrollback cap (UI-only, localStorage; read per-pane by TerminalPane on open).
  let termScrollback = $state<number | ''>('');

  // Apply config fields into the form state (reused on mount + after an import).
  function loadConfigFields(c: HubConfig) {
    cfg = c;
    scriptsRoot = c.scriptsRoot ?? '';
    fetchTimeout = c.fetchTimeoutSec ?? '';
    ghTimeout = c.ghTimeoutSec ?? '';
    startHidden = !!c.startHidden;
    closeToTray = c.closeToTray ?? true;
    toggleHotkey = c.toggleHotkey ?? '';
  }

  onMount(async () => {
    try {
      loadConfigFields(await readConfig());
      shortcuts = { toggle_window: toggleHotkey || '' };
      try {
        shortcuts = await readShortcuts();
      } catch { /* backend too old — keep default from config */ }
      autostart = await getAutostart();
      paths = await appPaths();
      version = await getVersion();
      const sb = Number(localStorage.getItem('cmh-sessions-scrollback'));
      if (sb >= 1000 && sb <= 50000) termScrollback = sb;
    } catch (e) {
      // Surface in-app — devtools console is invisible in the packaged build.
      errMsg = `${t('common.error')}: ${e}`;
    }
  });

  // #117: export current config to a file / import a config file back.
  async function doExport() {
    try {
      const dest = await pickSaveFile('castellyn-config.json');
      if (!dest) return;
      await exportConfig(dest);
      flash(t('settings.configExported'));
    } catch (e) {
      errMsg = `${t('common.error')}: ${e}`;
    }
  }
  async function doImport() {
    try {
      const src = await pickOpenFile();
      if (!src) return;
      const c = await importConfig(src);
      await writeConfig(c);
      loadConfigFields(c);
      await setToggleHotkey(c.toggleHotkey ?? null).catch(() => {}); // re-register imported hotkey (ignore a taken combo)
      paths = await appPaths(); // scriptsRoot may have changed → refresh the About "currently used" path
      flash(t('settings.configImported'));
    } catch (e) {
      errMsg = `${t('common.error')}: ${e}`;
    }
  }

  function flash(m: string) {
    savedMsg = m;
    setTimeout(() => (savedMsg = ''), 2000);
  }
  async function copyPath(p?: string | null) {
    if (!p) return;
    if (await copyText(p)) flash(t('common.copied'));
  }
  function resetView() {
    onSetDensity?.('comfortable');
    onSetFullWidth?.(false);
    flash(t('common.done'));
  }
  function saveScrollback() {
    const n = termScrollback === '' ? 5000 : Math.min(50000, Math.max(1000, Number(termScrollback)));
    termScrollback = n;
    try {
      localStorage.setItem('cmh-sessions-scrollback', String(n));
    } catch {
      /* ignore */
    }
    flash(t('settings.saved'));
  }

  // Returns false (and surfaces errMsg + reverts the optimistic cfg) if the write fails, so callers
  // don't flash "saved" or leave a toggle showing a value that never persisted.
  async function persist(patch: Partial<HubConfig>): Promise<boolean> {
    const prev = cfg;
    cfg = { ...cfg, ...patch };
    try {
      await writeConfig(cfg);
      errMsg = '';
      return true;
    } catch (e) {
      cfg = prev;
      errMsg = `${t('common.error')}: ${e}`;
      return false;
    }
  }

  async function saveRoot() {
    if (!(await persist({ scriptsRoot: scriptsRoot.trim() || null }))) return;
    paths = await appPaths();
    flash(t('settings.savedPath'));
  }
  // Enforce the inputs' min=5 on save too (the browser only enforces it on validated submit).
  const clampTimeout = (v: number | '') => (v === '' ? null : Math.max(5, Number(v)));
  async function saveTimeouts() {
    const ok = await persist({
      fetchTimeoutSec: clampTimeout(fetchTimeout),
      ghTimeoutSec: clampTimeout(ghTimeout)
    });
    if (!ok) return;
    flash(t('settings.savedTimeouts'));
  }
  async function toggleAutostart(v: boolean) {
    const prev = autostart;
    autostart = v; // optimistic
    try {
      await setAutostart(v);
      flash(v ? t('settings.autostartOn') : t('settings.autostartOff'));
    } catch (e) {
      autostart = prev; // rollback to match what actually persisted
      errMsg = `${t('settings.autostartError')}: ${e}`;
    }
  }
  async function toggleStartHidden(v: boolean) {
    startHidden = v;
    if (!(await persist({ startHidden: v }))) {
      startHidden = !v; // revert the toggle to match what actually persisted
      return;
    }
    flash(t('settings.saved'));
  }
  async function toggleCloseToTray(v: boolean) {
    closeToTray = v;
    if (!(await persist({ closeToTray: v }))) {
      closeToTray = !v;
      return;
    }
    flash(t('settings.saved'));
  }
  // Phase 4.1: apply the full shortcut mapping (replaces the single toggleHotkey path).
  async function applyShortcuts() {
    errMsg = '';
    try {
      await setShortcuts({ ...shortcuts });
      toggleHotkey = shortcuts.toggle_window ?? '';
      flash(t('settings.shortcutsSaved'));
    } catch (e) {
      errMsg = `${t('settings.toggleHotkeyError')}: ${e}`;
    }
  }

  // (L5: the single-hotkey applyToggleHotkey helper was dead — the markup applies the whole
  // shortcut map via applyShortcuts since Phase 4.1.)
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-center justify-between gap-sw-4">
    <h1 class="text-lg font-semibold">{t('settings.title')}</h1>
    <div class="flex items-center gap-sw-2">
      {#if errMsg}<span class="badge badge-err">{errMsg}</span>{:else if savedMsg}<span class="badge badge-ok">{savedMsg}</span>{/if}
      <input class="sw-input text-sw-xs" style="width:200px" bind:value={query}
        placeholder={t('settings.searchPlaceholder')} spellcheck="false" autocomplete="off" />
    </div>
  </header>

  <div class="flex flex-col gap-sw-4 {fullWidth ? '' : 'max-w-2xl'}">
    <!-- Theme -->
    {#if show(t('settings.theme'), t('settings.themeDesc'))}
    <div class="sw-card flex items-center justify-between" data-highlight-id="settings:theme">
      <div>
        <div class="font-medium">{t('settings.theme')}</div>
        <div class="text-sw-sm text-sw-text-secondary">{t('settings.themeDesc')}</div>
      </div>
      <div class="flex gap-sw-2">
        <button class="sw-btn {theme === 'dark' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
          onclick={() => onSetTheme('dark')} title={t('settings.themeDarkTip')}>{t('settings.themeDark')}</button>
        <button class="sw-btn {theme === 'light' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
          onclick={() => onSetTheme('light')} title={t('settings.themeLightTip')}>{t('settings.themeLight')}</button>
        <button class="sw-btn {theme === 'system' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
          onclick={() => onSetTheme('system')} title={t('settings.themeSystemTip')}>{t('settings.themeSystem')}</button>
      </div>
    </div>
    {/if}

    <!-- View: density + content width -->
    {#if show(t('settings.view'), t('settings.density'), t('settings.fullWidth'), t('settings.termScrollback'))}
    <div class="sw-card flex flex-col gap-sw-3" data-highlight-id="settings:view">
      <div class="flex items-center justify-between gap-sw-2">
        <div class="font-medium">{t('settings.view')}</div>
        <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={resetView} title={t('settings.resetViewTip')}>{t('settings.resetView')}</button>
      </div>
      <div class="flex items-center justify-between gap-sw-4">
        <div class="text-sw-sm text-sw-text-secondary">{t('settings.density')}</div>
        <div class="flex gap-sw-2">
          <button class="sw-btn {density === 'comfortable' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
            onclick={() => onSetDensity?.('comfortable')}>{t('settings.densityComfortable')}</button>
          <button class="sw-btn {density === 'compact' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
            onclick={() => onSetDensity?.('compact')}>{t('settings.densityCompact')}</button>
        </div>
      </div>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.fullWidth')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.fullWidthDesc')}</span>
        </span>
        <Toggle checked={fullWidth} onCheckedChange={(v) => onSetFullWidth?.(v)} title={t('settings.fullWidth')} />
      </label>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.termScrollback')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.termScrollbackDesc')}</span>
        </span>
        <input class="sw-input w-28" type="number" min="1000" max="50000" step="1000"
          bind:value={termScrollback} placeholder="5000" onchange={saveScrollback}
          title={t('settings.termScrollbackTip')} />
      </label>
    </div>
    {/if}

    <!-- Language -->
    {#if show(t('settings.language'), t('settings.languageDesc'))}
    <div class="sw-card flex items-center justify-between" data-highlight-id="settings:language">
      <div>
        <div class="font-medium">{t('settings.language')}</div>
        <div class="text-sw-sm text-sw-text-secondary">{t('settings.languageDesc')}</div>
      </div>
      <div class="flex gap-sw-2">
        {#each locale.supported as loc (loc)}
          <button
            class="sw-btn {locale.current === loc ? 'sw-btn-primary' : 'sw-btn-ghost'}"
            onclick={() => { locale.set(loc as Locale); setLanguage(loc).catch((e) => pushToast({ kind: 'error', title: t('common.error'), detail: String(e) })); }}
            title={t('settings.languageTip')}
          >
            {getLocaleName(loc as Locale)}
          </button>
        {/each}
      </div>
    </div>

    {/if}

    <!-- Scripts root -->
    {#if show(t('settings.scriptsRoot'), t('settings.scriptsRootDesc'))}
    <div class="sw-card flex flex-col gap-sw-2" data-highlight-id="settings:root">
      <div class="font-medium">{t('settings.scriptsRoot')}</div>
      <div class="text-sw-sm text-sw-text-secondary">
        {t('settings.scriptsRootDesc')}
      </div>
      <div class="flex gap-sw-2">
        <input
          class="sw-input flex-1"
          placeholder="E:\Scripts"
          bind:value={scriptsRoot}
          title={t('settings.scriptsRootInputTip')}
        />
        <button class="sw-btn sw-btn-primary" onclick={saveRoot} title={t('settings.savePathTip')}>{t('common.save')}</button>
      </div>
      {#if paths}<div class="text-sw-xs text-sw-text-muted">{t('settings.currentlyUsed', { path: paths.scriptsRoot })}</div>{/if}
    </div>
    {/if}

    <!-- Launch -->
    {#if show(t('settings.launch'), t('settings.startWithWindows'), t('settings.startHidden'), t('settings.closeToTray'), t('settings.confirmDestructive'), t('settings.toggleHotkey'), t('settings.shortcutsSection'))}
    <div class="sw-card flex flex-col gap-sw-3" data-highlight-id="settings:launch">
      <div class="font-medium">{t('settings.launch')}</div>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.startWithWindows')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.startWithWindowsDesc')}</span>
        </span>
        <Toggle checked={autostart} onCheckedChange={toggleAutostart} title={t('settings.startWithWindowsTip')} />
      </label>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.startHidden')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.startHiddenDesc')}</span>
        </span>
        <Toggle checked={startHidden} onCheckedChange={toggleStartHidden} title={t('settings.startHiddenTip')} />
      </label>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.closeToTray')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.closeToTrayDesc')}</span>
        </span>
        <Toggle checked={closeToTray} onCheckedChange={toggleCloseToTray} title={t('settings.closeToTrayTip')} />
      </label>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.confirmDestructive')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.confirmDestructiveDesc')}</span>
        </span>
        <Toggle checked={confirmDestructive} onCheckedChange={(v) => onSetConfirmDestructive?.(v)} title={t('settings.confirmDestructive')} />
      </label>
      <div class="flex flex-col gap-2">
        <span class="text-sw-sm">{t('settings.shortcutsSection')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.shortcutsSectionDesc')}</span>
        </span>
        <div class="flex flex-col gap-sw-2">
          {#each shortcutActions as action}
            <div class="flex items-center gap-sw-2">
              <span class="text-sw-xs w-36 shrink-0">{actionLabel(action)}</span>
              <input
                class="sw-input flex-1"
                value={shortcuts[action] ?? ''}
                oninput={(e) => { shortcuts[action] = e.currentTarget.value; shortcuts = { ...shortcuts }; }}
                placeholder={t('settings.toggleHotkeyPlaceholder')}
                title={t('settings.toggleHotkeyTip')}
              />
            </div>
          {/each}
        </div>
        <div class="flex items-center gap-sw-2 pt-sw-1">
          <button class="sw-btn sw-btn-primary" onclick={applyShortcuts} title={t('settings.toggleHotkeyTip')}>{t('settings.shortcutsApply')}</button>
        </div>
      </div>
    </div>
    {/if}

    <!-- Timeouts -->
    {#if show(t('settings.timeouts'), t('settings.timeoutsDesc'))}
    <div class="sw-card flex flex-col gap-sw-2" data-highlight-id="settings:timeouts">
      <div class="font-medium">{t('settings.timeouts')}</div>
      <div class="text-sw-sm text-sw-text-secondary">{t('settings.timeoutsDesc')}</div>
      <div class="flex flex-wrap items-end gap-sw-4">
        <label class="flex flex-col gap-1 text-sw-xs text-sw-text-muted">
          {t('settings.fetchTimeout')}
          <input class="sw-input w-28" type="number" min="5" bind:value={fetchTimeout} placeholder="120" title={t('settings.fetchTimeoutTip')} />
        </label>
        <label class="flex flex-col gap-1 text-sw-xs text-sw-text-muted">
          {t('settings.ghTimeout')}
          <input class="sw-input w-28" type="number" min="5" bind:value={ghTimeout} placeholder="60" title={t('settings.ghTimeoutTip')} />
        </label>
        <button class="sw-btn sw-btn-primary" onclick={saveTimeouts} title={t('settings.saveTimeoutsTip')}>{t('common.save')}</button>
      </div>
    </div>
    {/if}

    <!-- Settings backup (export/import config.json) -->
    {#if show(t('settings.backupSection'), t('settings.exportConfig'), t('settings.importConfig'))}
    <div class="sw-card flex flex-col gap-sw-2" data-highlight-id="settings:backup">
      <div class="font-medium">{t('settings.backupSection')}</div>
      <div class="text-sw-sm text-sw-text-secondary">{t('settings.backupSectionDesc')}</div>
      <div class="flex flex-wrap gap-sw-2 pt-sw-1">
        <button class="sw-btn sw-btn-ghost" onclick={doExport} title={t('settings.exportTip')}>{t('settings.exportConfig')}</button>
        <button class="sw-btn sw-btn-ghost" onclick={doImport} title={t('settings.importTip')}>{t('settings.importConfig')}</button>
      </div>
    </div>
    {/if}

    <!-- About -->
    <!-- U10: the About card's ACTIONS are searchable too (check updates / open folders) -->
    {#if show(t('settings.about'), t('settings.version'), t('settings.scripts'), t('settings.config'), t('settings.checkUpdates'), t('settings.openScriptsFolder'), t('settings.openConfigFile'), t('settings.openStackFile'), t('settings.openBackupFolder'))}
    <div class="sw-card flex flex-col gap-sw-2" data-highlight-id="settings:about">
      <div class="font-medium">{t('settings.about')}</div>
      <div class="flex items-center gap-sw-3 pb-sw-1">
        <img src="{base}/favicon.png" alt="Castellyn" width="48" height="48" style="border-radius:11px" />
        <div class="min-w-0">
          <div class="text-base font-semibold">Castellyn</div>
          <div class="text-sw-xs text-sw-text-muted">{t('settings.tagline')}</div>
        </div>
      </div>
      <dl class="grid grid-cols-[auto_1fr] gap-x-sw-4 gap-y-1 text-sw-sm">
        <dt class="text-sw-text-muted">{t('settings.version')}</dt>
        <dd class="text-sw-text flex items-center gap-sw-3 flex-wrap">
          <span>{version || t('common.dash')}</span>
          <!-- C3: check the configured updater endpoint for a newer signed release. -->
          <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={updPhase === 'checking' || updPhase === 'downloading'}
            onclick={onCheckUpdate} title={t('settings.checkUpdatesTip')}>
            {updPhase === 'checking' ? t('settings.checkingUpdates') : t('settings.checkUpdates')}
          </button>
          {#if updPhase === 'none'}<span class="text-sw-xs status-ok">{t('settings.upToDate')}</span>{/if}
          {#if updPhase === 'downloading'}<span class="text-sw-xs text-sw-text-muted">{updPct < 0 ? t('settings.downloading') : t('settings.downloadingPct', { pct: updPct })}</span>{/if}
          {#if updPhase === 'error'}<span class="text-sw-xs status-bad" title={updErr}>{t('settings.updateError')}</span>{/if}
        </dd>
        <dt class="text-sw-text-muted">{t('settings.scripts')}</dt>
        <dd class="min-w-0"><button class="copyable" onclick={() => copyPath(paths?.scriptsRoot)} title={t('common.copyPath')}>{paths?.scriptsRoot ?? t('common.dash')}</button></dd>
        <dt class="text-sw-text-muted">{t('settings.config')}</dt>
        <dd class="min-w-0"><button class="copyable" onclick={() => copyPath(paths?.configPath)} title={t('common.copyPath')}>{paths?.configPath ?? t('common.dash')}</button></dd>
        <dt class="text-sw-text-muted">{t('settings.app')}</dt>
        <dd class="min-w-0"><button class="copyable" onclick={() => copyPath(paths?.exe)} title={t('common.copyPath')}>{paths?.exe ?? t('common.dash')}</button></dd>
        <!-- V15: stack/backup paths are visible & copyable like the rest (the open-buttons below
             were the only trace of them) -->
        {#if paths?.stackPath}
          <dt class="text-sw-text-muted">{t('settings.openStackFile')}</dt>
          <dd class="min-w-0"><button class="copyable" onclick={() => copyPath(paths?.stackPath)} title={t('common.copyPath')}>{paths.stackPath}</button></dd>
        {/if}
        {#if paths?.backupDir}
          <dt class="text-sw-text-muted">{t('settings.backupSection')}</dt>
          <dd class="min-w-0"><button class="copyable" onclick={() => copyPath(paths?.backupDir)} title={t('common.copyPath')}>{paths.backupDir}</button></dd>
        {/if}
      </dl>
      <div class="flex flex-wrap gap-sw-2 pt-sw-1">
        {#if paths?.scriptsRoot}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openPath(paths!.scriptsRoot)}
            title={t('settings.openScriptsFolderTip')}>{t('settings.openScriptsFolder')}</button>
        {/if}
        {#if paths?.configPath}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openPath(paths!.configPath!)}
            title={t('settings.openConfigFileTip')}>{t('settings.openConfigFile')}</button>
        {/if}
        {#if paths?.stackPath}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openPath(paths!.stackPath!)}
            title={t('settings.openStackFileTip')}>{t('settings.openStackFile')}</button>
        {/if}
        {#if paths?.backupDir}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openPath(paths!.backupDir!)}
            title={t('settings.openBackupFolderTip')}>{t('settings.openBackupFolder')}</button>
        {/if}
      </div>
    </div>
    {/if}
    <!-- V6/S4: the shared EmptyState primitive (SVG icon) instead of a hand-rolled 🔍 block -->
    {#if noResults}
      <EmptyState icon={Search} description={t('settings.noResults', { query: query.trim() })} />
    {/if}
  </div>
</div>

<!-- C3: release-notes confirm before downloading an update. Notes render as plain lines (raw markdown). -->
<ConfirmDialog
  open={updPhase === 'available'}
  title={t('settings.updateAvailableTitle', { version: updInfo?.version ?? '' })}
  message={t('settings.updateAvailableMsg', { from: updInfo?.currentVersion ?? '', to: updInfo?.version ?? '' })}
  details={updInfo?.notes ? updInfo.notes.split('\n').filter((l) => l.trim()) : []}
  confirmLabel={t('settings.updateNow')}
  onConfirm={onInstallUpdate}
  onCancel={() => {
    updPhase = 'idle';
    updInfo = null;
  }}
/>

<style>
  /* A path/value that copies to the clipboard on click — looks like text, hints on hover. */
  .copyable {
    display: block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    font: inherit;
    color: var(--sw-text);
    cursor: pointer;
  }
  .copyable:hover {
    color: var(--sw-accent-text);
    text-decoration: underline;
  }
</style>
