<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { initTheme } from '$lib/theme';
  import { initLocale } from '$lib/i18n';
  import WindowTitleBar from '$lib/components/WindowTitleBar.svelte';
  // DetachedView pulls in xterm (~480K). Only detached windows ever render it, so import it
  // lazily — otherwise the static import drags xterm into the main window's startup chunk.

  let { children } = $props();

  // Detached per-monitor / popped-out windows (label != "main") render ONLY the mirrored pane —
  // never the full tabbed UI — so +page's data-fetching never runs in them.
  let isDetached = $state(false);
  try {
    isDetached = getCurrentWindow().label !== 'main';
  } catch {
    isDetached = false;
  }

  onMount(() => {
    initTheme();
    initLocale();
  });

  // Suppress WebView2's native context menu (Refresh/Print/…) app-wide — it reads as a broken
  // right-click. Keep it where copy/paste genuinely helps: editables and the terminal.
  function onContextMenu(e: MouseEvent) {
    const el = e.target as HTMLElement | null;
    if (el?.closest('input, textarea, [contenteditable="true"], .xterm')) return;
    e.preventDefault();
  }
</script>

<svelte:window oncontextmenu={onContextMenu} />

{#if isDetached}
  {#await import('$lib/components/DetachedView.svelte') then { default: DetachedView }}
    <DetachedView />
  {/await}
{:else}
  <div class="flex h-screen flex-col overflow-hidden">
    <WindowTitleBar />
    <div class="min-h-0 flex-1 overflow-hidden">
      {@render children()}
    </div>
  </div>
{/if}
