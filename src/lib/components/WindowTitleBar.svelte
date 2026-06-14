<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { t } from '$lib/i18n';

  const appWin = getCurrentWindow();

  async function minimize() {
    await appWin.minimize();
  }
  async function toggleMaximize() {
    if (await appWin.isMaximized()) {
      await appWin.unmaximize();
    } else {
      await appWin.maximize();
    }
  }
  async function close() {
    // Window CloseRequested is intercepted in Rust → hides to tray.
    await appWin.close();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="titlebar" data-tauri-drag-region ondblclick={toggleMaximize}>
  <div class="brand" data-tauri-drag-region>
    <span class="dot" data-tauri-drag-region></span>
    <span class="title" data-tauri-drag-region>{t('titlebar.title')}</span>
  </div>

  <div class="controls">
    <button class="tb-btn" onclick={minimize} aria-label={t('titlebar.minimize')} title={t('titlebar.minimize')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><line x1="2" y1="6.5" x2="10" y2="6.5" /></svg>
    </button>
    <button class="tb-btn" onclick={toggleMaximize} aria-label={t('titlebar.maximize')} title={t('titlebar.maximize')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><rect x="2.5" y="2.5" width="7" height="7" /></svg>
    </button>
    <button class="tb-btn tb-close" onclick={close} aria-label={t('titlebar.close')} title={t('titlebar.close')}>
      <svg viewBox="0 0 12 12" width="11" height="11"><line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" /></svg>
    </button>
  </div>
</div>

<style>
  .titlebar {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--sw-sidebar-bg, var(--sw-bg-secondary));
    border-bottom: 1px solid var(--sw-border);
    user-select: none;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-left: 12px;
    height: 100%;
    flex: 1;
    min-width: 0;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--sw-accent);
    box-shadow: 0 0 8px var(--sw-accent-glow);
    flex-shrink: 0;
  }
  .title {
    font-size: var(--sw-text-xs);
    font-weight: 600;
    color: var(--sw-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .controls {
    display: flex;
    height: 100%;
  }
  .tb-btn {
    width: 46px;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--sw-text-secondary);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: background-color 0.12s, color 0.12s;
  }
  .tb-btn svg {
    stroke: currentColor;
    stroke-width: 1.2;
    fill: none;
  }
  .tb-btn:hover {
    background: var(--sw-bg-hover);
    color: var(--sw-text-primary);
  }
  .tb-close:hover {
    background: #e81123;
    color: #fff;
  }
  .tb-btn:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--sw-accent);
  }
</style>
