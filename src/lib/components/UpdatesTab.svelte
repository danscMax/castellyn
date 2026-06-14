<script lang="ts">
  import type { Component } from '$lib/ipc';
  import ComponentCard from './ComponentCard.svelte';
  import { t } from '$lib/i18n';

  let {
    components,
    statuses,
    running,
    onCheck,
    onApply,
    onOpenTab
  }: {
    components: Component[];
    statuses: Record<string, any>;
    running: string | null;
    onCheck: (id: string) => void;
    onApply: (comp: Component) => void;
    onOpenTab?: (id: string) => void;
  } = $props();

  // Preserve manifest order of groups while grouping.
  let groups = $derived.by(() => {
    const order: string[] = [];
    const map: Record<string, Component[]> = {};
    for (const c of components) {
      if (!map[c.group]) {
        map[c.group] = [];
        order.push(c.group);
      }
      map[c.group].push(c);
    }
    return order.map((g) => ({ group: g, items: map[g] }));
  });
</script>

<div class="p-sw-6">
  <header class="mb-sw-4">
    <h1 class="text-lg font-semibold">{t('updates.title')}</h1>
    <p class="text-sw-sm text-sw-text-secondary">
      {t('updates.subtitle')}
    </p>
  </header>

  <!-- Group panels fill the available width: sparse groups (1 card) sit side by side
       instead of leaving a lonely column with empty space on the right. -->
  <div class="group-grid">
    {#each groups as grp (grp.group)}
      <section class="flex flex-col gap-sw-3">
        <h2 class="text-sw-xs font-semibold uppercase tracking-wide text-sw-text-muted">
          {grp.group}
        </h2>
        {#each grp.items as c (c.id)}
          <ComponentCard
            comp={c}
            status={statuses[c.id]}
            busy={running === c.id}
            anyRunning={!!running}
            onCheck={() => onCheck(c.id)}
            onApply={() => onApply(c)}
            onOpenForks={onOpenTab ? () => onOpenTab('forks') : undefined}
          />
        {/each}
      </section>
    {/each}
  </div>
</div>

<style>
  .group-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: var(--sw-space-4);
    align-items: start;
  }
</style>
