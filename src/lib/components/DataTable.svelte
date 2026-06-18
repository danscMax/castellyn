<script module lang="ts">
  // Column definition for DataTable (exported so parents can type their column arrays).
  export type DTColumn = {
    key: string;
    label: string;
    align?: 'left' | 'right' | 'center';
    sortable?: boolean;
    width?: string;
    grow?: boolean; // this column absorbs slack width (only one should set it)
    interactive?: boolean; // cell has its own controls → clicks must not toggle row expand
  };
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { slide } from 'svelte/transition';
  import { t } from '$lib/i18n';

  // Reusable dense data table — the DRY base for every list screen. Card container, header-click
  // sorting (persisted), search + toolbar, row selection + bulk bar, and per-row expansion rendered
  // as an indented nested panel. The parent supplies column defs and renders each cell via `cell`.

  let {
    columns,
    rows,
    rowKey,
    sortAccessor,
    search = false,
    searchValue,
    searchPlaceholder = '',
    defaultSort = '',
    defaultDir = 'asc',
    storageKey = '',
    canExpand,
    selectable = false,
    rowMuted,
    rowAccent,
    cell,
    expand,
    toolbar,
    bulkbar,
    empty
  }: {
    columns: DTColumn[];
    rows: any[];
    rowKey: (r: any) => string;
    sortAccessor?: (r: any, key: string) => string | number;
    search?: boolean;
    searchValue?: (r: any) => string;
    searchPlaceholder?: string;
    defaultSort?: string;
    defaultDir?: 'asc' | 'desc';
    storageKey?: string; // persist sort in localStorage under this key
    canExpand?: (r: any) => boolean;
    selectable?: boolean;
    rowMuted?: (r: any) => boolean; // dim the row (e.g. disabled item)
    rowAccent?: (r: any) => boolean; // left accent stripe (e.g. update available)
    cell: Snippet<[any, DTColumn]>;
    expand?: Snippet<[any]>;
    toolbar?: Snippet;
    bulkbar?: Snippet<[string[], () => void]>;
    empty?: Snippet;
  } = $props();

  function readSort(): { k: string; d: 'asc' | 'desc' } {
    if (storageKey) {
      try {
        const s = JSON.parse(localStorage.getItem(`dt-sort-${storageKey}`) ?? 'null');
        if (s && typeof s.k === 'string') return { k: s.k, d: s.d === 'desc' ? 'desc' : 'asc' };
      } catch {
        /* ignore */
      }
    }
    return { k: defaultSort, d: defaultDir };
  }
  const _init = readSort();
  // svelte-ignore state_referenced_locally
  let sortKey = $state(_init.k);
  // svelte-ignore state_referenced_locally
  let sortDir = $state<'asc' | 'desc'>(_init.d);
  let query = $state('');
  let openKeys = $state<Set<string>>(new Set());
  let selected = $state<Set<string>>(new Set());

  // Per-column width overrides (drag-to-resize), persisted with the sort under storageKey.
  function readWidths(): Record<string, string> {
    if (!storageKey) return {};
    try {
      return JSON.parse(localStorage.getItem(`dt-w-${storageKey}`) ?? '{}') ?? {};
    } catch {
      return {};
    }
  }
  // svelte-ignore state_referenced_locally
  let colW = $state<Record<string, string>>(readWidths());
  let resizing = $state(false);
  function startResize(e: PointerEvent, key: string) {
    e.preventDefault();
    e.stopPropagation();
    const th = (e.currentTarget as HTMLElement).closest('th') as HTMLElement | null;
    if (!th) return;
    const startX = e.clientX;
    const startW = th.offsetWidth;
    resizing = true;
    const move = (ev: PointerEvent) => {
      colW = { ...colW, [key]: `${Math.max(60, startW + (ev.clientX - startX))}px` };
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      if (storageKey) {
        try {
          localStorage.setItem(`dt-w-${storageKey}`, JSON.stringify(colW));
        } catch {
          /* ignore */
        }
      }
      setTimeout(() => (resizing = false), 0);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  $effect(() => {
    if (storageKey) {
      try {
        localStorage.setItem(`dt-sort-${storageKey}`, JSON.stringify({ k: sortKey, d: sortDir }));
      } catch {
        /* ignore */
      }
    }
  });

  function toggleSort(c: DTColumn) {
    if (!c.sortable) return;
    if (sortKey === c.key) sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    else {
      sortKey = c.key;
      sortDir = 'asc';
    }
  }
  function toggleOpen(k: string) {
    const n = new Set(openKeys);
    if (n.has(k)) n.delete(k);
    else n.add(k);
    openKeys = n;
  }
  function toggleSel(k: string) {
    const n = new Set(selected);
    if (n.has(k)) n.delete(k);
    else n.add(k);
    selected = n;
  }
  function clearSel() {
    selected = new Set();
  }

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q || !searchValue) return rows;
    return rows.filter((r) => searchValue(r).toLowerCase().includes(q));
  });
  const sorted = $derived.by(() => {
    if (!sortKey || !sortAccessor) return filtered;
    const dir = sortDir === 'asc' ? 1 : -1;
    return [...filtered].sort((a, b) => {
      const av = sortAccessor(a, sortKey);
      const bv = sortAccessor(b, sortKey);
      if (av < bv) return -dir;
      if (av > bv) return dir;
      return 0;
    });
  });
  const visibleKeys = $derived(sorted.map(rowKey));
  const allSelected = $derived(visibleKeys.length > 0 && visibleKeys.every((k) => selected.has(k)));
  const selectedList = $derived(visibleKeys.filter((k) => selected.has(k)));
  function toggleAll() {
    selected = allSelected ? new Set() : new Set(visibleKeys);
  }
  const colSpan = $derived(columns.length + (expand ? 1 : 0) + (selectable ? 1 : 0));
  const rowExpandable = (r: any) => !!expand && (!canExpand || canExpand(r));
</script>

<div class="dt-card">
  {#if search || toolbar}
    <div class="dt-bar">
      {#if search}
        <input class="sw-input dt-search" bind:value={query} placeholder={searchPlaceholder || t('common.search')}
          spellcheck="false" autocomplete="off" />
      {/if}
      {#if toolbar}{@render toolbar()}{/if}
    </div>
  {/if}

  {#if selectable && bulkbar && selectedList.length}
    <div class="dt-bulk" transition:slide={{ duration: 120 }}>
      {@render bulkbar(selectedList, clearSel)}
    </div>
  {/if}

  <div class="dt-scroll">
    <table class="dt">
      <thead>
        <tr>
          {#if selectable}
            <th class="dt-sel">
              <input class="dt-check" type="checkbox" checked={allSelected} onchange={toggleAll} aria-label="select all" />
            </th>
          {/if}
          {#if expand}<th class="dt-exp" aria-hidden="true"></th>{/if}
          {#each columns as c, i (c.key)}
            <th
              class="dt-th align-{c.align ?? 'left'}"
              class:sortable={c.sortable}
              class:active={sortKey === c.key}
              class:grow={c.grow}
              style={colW[c.key] ?? c.width ? `width:${colW[c.key] ?? c.width}` : ''}
              onclick={() => { if (!resizing) toggleSort(c); }}
              aria-sort={sortKey === c.key ? (sortDir === 'asc' ? 'ascending' : 'descending') : undefined}
            >
              <span class="dt-thlabel">{c.label}{#if c.sortable}<span class="dt-arrow">{sortKey === c.key ? (sortDir === 'asc' ? '↑' : '↓') : '↕'}</span>{/if}</span>
              {#if i < columns.length - 1}<span class="dt-resize" onpointerdown={(e) => startResize(e, c.key)} onclick={(e) => e.stopPropagation()} role="separator" aria-hidden="true"></span>{/if}
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each sorted as row (rowKey(row))}
          {@const k = rowKey(row)}
          {@const exp = rowExpandable(row)}
          <tr class="dt-row" class:open={openKeys.has(k)} class:sel={selected.has(k)} class:clickable={exp}
            class:muted={rowMuted?.(row)} class:accent={rowAccent?.(row)}
            onclick={() => exp && toggleOpen(k)}>
            {#if selectable}
              <td class="dt-sel" onclick={(e) => e.stopPropagation()}>
                <input class="dt-check" type="checkbox" checked={selected.has(k)} onchange={() => toggleSel(k)} aria-label="select row" />
              </td>
            {/if}
            {#if expand}
              <td class="dt-exp">
                {#if exp}
                  <button class="dt-expbtn" onclick={(e) => { e.stopPropagation(); toggleOpen(k); }}
                    aria-expanded={openKeys.has(k)} title={t('common.toggle')}>{openKeys.has(k) ? '▾' : '▸'}</button>
                {/if}
              </td>
            {/if}
            {#each columns as c (c.key)}
              <td class="align-{c.align ?? 'left'}" onclick={c.interactive ? (e) => e.stopPropagation() : undefined}>{@render cell(row, c)}</td>
            {/each}
          </tr>
          {#if expand && exp && openKeys.has(k)}
            <tr class="dt-detail">
              <td colspan={colSpan}>
                <div class="dt-detail-inner" transition:slide={{ duration: 150 }}>{@render expand(row)}</div>
              </td>
            </tr>
          {/if}
        {/each}
        {#if !sorted.length}
          <tr><td colspan={colSpan} class="dt-empty">{#if empty}{@render empty()}{:else}—{/if}</td></tr>
        {/if}
      </tbody>
    </table>
  </div>
</div>

<style>
  .dt-card {
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg, 12px);
    background: var(--sw-bg-secondary);
    overflow: hidden;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
  }
  .dt-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    padding: 10px 12px;
    border-bottom: 1px solid var(--sw-border);
  }
  .dt-search {
    min-width: 200px;
    max-width: 320px;
    flex: 1;
    font-size: var(--sw-text-xs);
  }
  .dt-bulk {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
    padding: 8px 12px;
    border-bottom: 1px solid var(--sw-border);
    background: var(--sw-accent-glow, rgba(59, 130, 246, 0.12));
    font-size: var(--sw-text-xs);
  }
  .dt-scroll {
    overflow-x: auto;
  }
  table.dt {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--sw-text-sm);
    font-variant-numeric: tabular-nums;
    table-layout: fixed;
  }
  /* fixed layout → cells never widen the table; long text truncates via the cell content */
  .dt tbody td > :global(.truncate),
  .dt tbody td :global(.namecell) {
    max-width: 100%;
  }
  .dt-check {
    appearance: none;
    -webkit-appearance: none;
    width: 15px;
    height: 15px;
    border: 1.5px solid var(--sw-border);
    border-radius: 4px;
    background: var(--sw-bg-subtle);
    cursor: pointer;
    position: relative;
    vertical-align: middle;
    flex: none;
  }
  .dt-check:hover {
    border-color: var(--sw-text-muted);
  }
  .dt-check:checked {
    background: var(--sw-accent, #3b82f6);
    border-color: var(--sw-accent, #3b82f6);
  }
  .dt-check:checked::after {
    content: '';
    position: absolute;
    left: 4px;
    top: 1px;
    width: 4px;
    height: 8px;
    border: solid #fff;
    border-width: 0 2px 2px 0;
    transform: rotate(45deg);
  }
  .dt-check:focus-visible {
    outline: 2px solid var(--sw-accent-text);
    outline-offset: 1px;
  }
  .dt-resize {
    position: absolute;
    top: 0;
    right: -3px;
    width: 8px;
    height: 100%;
    cursor: col-resize;
    z-index: 2;
  }
  .dt-resize:hover {
    background: linear-gradient(to right, transparent 3px, var(--sw-accent-text) 3px, var(--sw-accent-text) 5px, transparent 5px);
  }
  .dt thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--sw-bg-secondary);
    border-bottom: 1px solid var(--sw-border);
    box-shadow: 0 1px 0 var(--sw-border);
    padding: 8px 14px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
    white-space: nowrap;
    user-select: none;
  }
  .dt-th.sortable {
    cursor: pointer;
  }
  .dt-th.sortable:hover {
    color: var(--sw-text-secondary);
  }
  .dt-th.active {
    color: var(--sw-text-secondary);
  }
  .dt-thlabel {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  /* sort arrow: only shown for the active column, or on header hover */
  .dt-arrow {
    opacity: 0;
    font-size: 10px;
    transition: opacity 0.1s;
  }
  .dt-th.active .dt-arrow {
    opacity: 1;
    color: var(--sw-accent-text);
  }
  .dt-th.sortable:hover .dt-arrow {
    opacity: 0.5;
  }
  .dt tbody td {
    padding: 8px 14px;
    border-bottom: 1px solid var(--sw-border);
    vertical-align: middle;
    height: 38px;
  }
  .dt tbody tr:last-child td {
    border-bottom: none;
  }
  /* zebra + hover */
  .dt-row:nth-child(odd) td {
    background: rgba(255, 255, 255, 0.012);
  }
  .dt-row:hover td {
    background: var(--sw-bg-subtle);
  }
  .dt-row.sel td {
    background: var(--sw-accent-glow, rgba(59, 130, 246, 0.1));
  }
  .dt-row.clickable {
    cursor: pointer;
  }
  .dt-row.muted td {
    opacity: 0.55;
    background: rgba(128, 128, 128, 0.06);
  }
  .dt-row.accent td:first-child {
    box-shadow: inset 3px 0 0 var(--sw-accent-text);
  }
  .dt-row.open td {
    background: var(--sw-bg-subtle);
    border-bottom-color: transparent;
  }
  .align-right,
  th.align-right {
    text-align: right;
  }
  .align-center,
  th.align-center {
    text-align: center;
  }
  .dt-sel {
    width: 36px;
    text-align: center;
    padding-left: 10px !important;
    padding-right: 0 !important;
  }
  .dt-sel input {
    cursor: pointer;
  }
  .dt-exp {
    width: 28px;
    text-align: center;
    padding-left: 4px !important;
    padding-right: 0 !important;
  }
  .dt-expbtn {
    background: none;
    border: none;
    color: var(--sw-text-muted);
    cursor: pointer;
    font-size: 11px;
    padding: 2px 4px;
    line-height: 1;
  }
  .dt-expbtn:hover {
    color: var(--sw-text);
  }
  .dt-detail td {
    background: var(--sw-bg-subtle);
    padding: 0 14px 0 42px;
  }
  .dt-detail-inner {
    padding: 10px 0 14px;
  }
  .dt-empty {
    text-align: center;
    color: var(--sw-text-muted);
    padding: 22px;
  }
  .dt-th.sortable:focus-visible,
  .dt-expbtn:focus-visible {
    outline: 2px solid var(--sw-accent-text);
    outline-offset: -2px;
  }
</style>
