<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { initTheme } from '$lib/theme';
  import { initLocale } from '$lib/i18n';
  import WindowTitleBar from '$lib/components/WindowTitleBar.svelte';

  let { children } = $props();
  onMount(() => {
    initTheme();
    initLocale();
  });
</script>

<div class="flex h-screen flex-col overflow-hidden">
  <WindowTitleBar />
  <div class="min-h-0 flex-1 overflow-hidden">
    {@render children()}
  </div>
</div>
