<script lang="ts">
  import { onMount } from 'svelte';
  import { getVersion } from '@tauri-apps/api/app';
  import {
    readConfig,
    writeConfig,
    appPaths,
    openPath,
    getAutostart,
    setAutostart,
    type HubConfig,
    type AppPaths
  } from '$lib/ipc';
  import type { Theme } from '$lib/theme';
  import { t, locale, getLocaleName, type Locale } from '$lib/i18n';

  let { theme, onSetTheme }: { theme: Theme; onSetTheme: (th: Theme) => void } = $props();

  let cfg = $state<HubConfig>({});
  let scriptsRoot = $state('');
  let fetchTimeout = $state<number | ''>('');
  let ghTimeout = $state<number | ''>('');
  let autostart = $state(false);
  let startHidden = $state(false);
  let paths = $state<AppPaths | null>(null);
  let version = $state('');
  let savedMsg = $state('');

  onMount(async () => {
    try {
      cfg = await readConfig();
      scriptsRoot = cfg.scriptsRoot ?? '';
      fetchTimeout = cfg.fetchTimeoutSec ?? '';
      ghTimeout = cfg.ghTimeoutSec ?? '';
      startHidden = !!cfg.startHidden;
      autostart = await getAutostart();
      paths = await appPaths();
      version = await getVersion();
    } catch (e) {
      console.error(e);
    }
  });

  function flash(m: string) {
    savedMsg = m;
    setTimeout(() => (savedMsg = ''), 2000);
  }

  async function persist(patch: Partial<HubConfig>) {
    cfg = { ...cfg, ...patch };
    await writeConfig(cfg);
  }

  async function saveRoot() {
    await persist({ scriptsRoot: scriptsRoot.trim() || null });
    paths = await appPaths();
    flash(t('settings.savedPath'));
  }
  async function saveTimeouts() {
    await persist({
      fetchTimeoutSec: fetchTimeout === '' ? null : Number(fetchTimeout),
      ghTimeoutSec: ghTimeout === '' ? null : Number(ghTimeout)
    });
    flash(t('settings.savedTimeouts'));
  }
  async function toggleAutostart() {
    autostart = !autostart;
    await setAutostart(autostart);
    flash(autostart ? t('settings.autostartOn') : t('settings.autostartOff'));
  }
  async function toggleStartHidden() {
    startHidden = !startHidden;
    await persist({ startHidden });
    flash(t('settings.saved'));
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-center justify-between">
    <h1 class="text-lg font-semibold">{t('settings.title')}</h1>
    {#if savedMsg}<span class="badge badge-ok">{savedMsg}</span>{/if}
  </header>

  <div class="flex max-w-2xl flex-col gap-sw-4">
    <!-- Theme -->
    <div class="sw-card flex items-center justify-between">
      <div>
        <div class="font-medium">{t('settings.theme')}</div>
        <div class="text-sw-sm text-sw-text-secondary">{t('settings.themeDesc')}</div>
      </div>
      <div class="flex gap-sw-2">
        <button class="sw-btn {theme === 'dark' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
          onclick={() => onSetTheme('dark')} title={t('settings.themeDarkTip')}>{t('settings.themeDark')}</button>
        <button class="sw-btn {theme === 'light' ? 'sw-btn-primary' : 'sw-btn-ghost'}"
          onclick={() => onSetTheme('light')} title={t('settings.themeLightTip')}>{t('settings.themeLight')}</button>
      </div>
    </div>

    <!-- Language -->
    <div class="sw-card flex items-center justify-between">
      <div>
        <div class="font-medium">{t('settings.language')}</div>
        <div class="text-sw-sm text-sw-text-secondary">{t('settings.languageDesc')}</div>
      </div>
      <div class="flex gap-sw-2">
        {#each locale.supported as loc (loc)}
          <button
            class="sw-btn {locale.current === loc ? 'sw-btn-primary' : 'sw-btn-ghost'}"
            onclick={() => locale.set(loc as Locale)}
            title={t('settings.languageTip')}
          >
            {getLocaleName(loc as Locale)}
          </button>
        {/each}
      </div>
    </div>

    <!-- Scripts root -->
    <div class="sw-card flex flex-col gap-sw-2">
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

    <!-- Launch -->
    <div class="sw-card flex flex-col gap-sw-3">
      <div class="font-medium">{t('settings.launch')}</div>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.startWithWindows')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.startWithWindowsDesc')}</span>
        </span>
        <input type="checkbox" checked={autostart} onchange={toggleAutostart} title={t('settings.startWithWindowsTip')} />
      </label>
      <label class="flex items-center justify-between gap-sw-4">
        <span class="text-sw-sm">{t('settings.startHidden')}
          <span class="block text-sw-xs text-sw-text-muted">{t('settings.startHiddenDesc')}</span>
        </span>
        <input type="checkbox" checked={startHidden} onchange={toggleStartHidden} title={t('settings.startHiddenTip')} />
      </label>
    </div>

    <!-- Timeouts -->
    <div class="sw-card flex flex-col gap-sw-2">
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

    <!-- About -->
    <div class="sw-card flex flex-col gap-sw-2">
      <div class="font-medium">{t('settings.about')}</div>
      <dl class="grid grid-cols-[auto_1fr] gap-x-sw-4 gap-y-1 text-sw-sm">
        <dt class="text-sw-text-muted">{t('settings.version')}</dt><dd class="text-sw-text">{version || t('common.dash')}</dd>
        <dt class="text-sw-text-muted">{t('settings.scripts')}</dt><dd class="truncate text-sw-text">{paths?.scriptsRoot ?? t('common.dash')}</dd>
        <dt class="text-sw-text-muted">{t('settings.config')}</dt><dd class="truncate text-sw-text">{paths?.configPath ?? t('common.dash')}</dd>
        <dt class="text-sw-text-muted">{t('settings.app')}</dt><dd class="truncate text-sw-text">{paths?.exe ?? t('common.dash')}</dd>
      </dl>
      <div class="flex gap-sw-2 pt-sw-1">
        {#if paths?.scriptsRoot}
          <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => openPath(paths!.scriptsRoot)}
            title={t('settings.openScriptsFolderTip')}>{t('settings.openScriptsFolder')}</button>
        {/if}
      </div>
    </div>
  </div>
</div>
