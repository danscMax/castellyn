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

<script lang="ts" generics="Row">
  import { onDestroy, type Snippet } from 'svelte';
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
    loading = false,
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
    rowStyle,
    highlightAttr,
    cell,
    expand,
    toolbar,
    bulkbar,
    empty
  }: {
    columns: DTColumn[];
    rows: Row[];
    /** Show canonical skeleton rows instead of data (initial load). Distinct from an empty result. */
    loading?: boolean;
    rowKey: (r: Row) => string;
    // Row left untyped here: callers may derive the sort value via a dynamic `r[key]` lookup
    // (e.g. EnvironmentsTab's skill matrix), which a concrete Row type would reject at compile time.
    sortAccessor?: (r: any, key: string) => string | number;
    search?: boolean;
    searchValue?: (r: Row) => string;
    searchPlaceholder?: string;
    defaultSort?: string;
    defaultDir?: 'asc' | 'desc';
    storageKey?: string; // persist sort in localStorage under this key
    canExpand?: (r: Row) => boolean;
    selectable?: boolean;
    rowMuted?: (r: Row) => boolean; // dim the row (e.g. disabled item)
    rowAccent?: (r: Row) => boolean; // left accent stripe (e.g. update available)
    rowStyle?: (r: Row) => string | undefined; // inline style per row
    highlightAttr?: (r: Row) => string | null | undefined; // data-highlight-id per row
    cell: Snippet<[Row, DTColumn]>;
    expand?: Snippet<[Row]>;
    toolbar?: Snippet;
    bulkbar?: Snippet<[string[], () => void]>;
    empty?: Snippet;
  } = $props();

  const colKeys = $derived(new Set(columns.map((c) => c.key)));
  function readSort(): { k: string; d: 'asc' | 'desc' } {
    if (storageKey) {
      try {
        const s = JSON.parse(localStorage.getItem(`dt-sort-${storageKey}`) ?? 'null');
        // Drop a persisted sort key that no longer matches a current column (columns renamed/removed
        // across an upgrade) — otherwise sorting silently no-ops. Fall back to the default sort.
        if (s && typeof s.k === 'string' && colKeys.has(s.k)) return { k: s.k, d: s.d === 'desc' ? 'desc' : 'asc' };
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
      const raw = JSON.parse(localStorage.getItem(`dt-w-${storageKey}`) ?? '{}') ?? {};
      // Prune persisted width entries whose column no longer exists (dead keys after a column-set
      // change), so stale localStorage can't accumulate or shadow renamed columns.
      const out: Record<string, string> = {};
      for (const k of Object.keys(raw)) if (colKeys.has(k)) out[k] = raw[k];
      return out;
    } catch {
      return {};
    }
  }
  // svelte-ignore state_referenced_locally
  let colW = $state<Record<string, string>>(readWidths());
  let resizing = $state(false);
  // The in-flight drag's window listeners, so onDestroy can remove them if the component
  // unmounts mid-drag (otherwise `up` never runs and the closures over colW/th/handle leak).
  let activeDrag: { move: (ev: PointerEvent) => void; up: () => void } | null = null;
  function startResize(e: PointerEvent, key: string) {
    e.preventDefault();
    e.stopPropagation();
    const handle = e.currentTarget as HTMLElement;
    const th = handle.closest('th') as HTMLElement | null;
    if (!th) return;
    // Capture the pointer so move/up keep firing even if the cursor outruns the 8px handle.
    handle.setPointerCapture?.(e.pointerId);
    const startX = e.clientX;
    const startW = th.offsetWidth;
    resizing = true;
    const move = (ev: PointerEvent) => {
      colW = { ...colW, [key]: `${Math.max(60, startW + (ev.clientX - startX))}px` };
    };
    const up = () => {
      handle.releasePointerCapture?.(e.pointerId);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', up);
      activeDrag = null;
      if (storageKey) {
        try {
          localStorage.setItem(`dt-w-${storageKey}`, JSON.stringify(colW));
        } catch {
          /* ignore */
        }
      }
      setTimeout(() => (resizing = false), 0);
    };
    activeDrag = { move, up };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    // pointercancel (touch/pen interruption, capture loss) otherwise never runs `up`, leaving
    // `resizing` stuck true — every later header click would be swallowed by the sort guard.
    window.addEventListener('pointercancel', up);
  }
  onDestroy(() => {
    if (activeDrag) {
      window.removeEventListener('pointermove', activeDrag.move);
      window.removeEventListener('pointerup', activeDrag.up);
      window.removeEventListener('pointercancel', activeDrag.up);
    }
  });

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
  const visibleKeySet = $derived(new Set(visibleKeys));
  const allSelected = $derived(visibleKeys.length > 0 && visibleKeys.every((k) => selected.has(k)));
  // Partial selection → the header checkbox shows the indeterminate (dash) state instead of empty.
  const someSelected = $derived(visibleKeys.some((k) => selected.has(k)));
  // Bulk actions operate on EVERY selected row, not just the visible ones — so selecting rows and
  // then narrowing the search/filter does not silently drop them from the batch. Preserve order:
  // visible (sorted) keys first, then any selected-but-hidden keys appended.
  const selectedList = $derived([
    ...visibleKeys.filter((k) => selected.has(k)),
    ...[...selected].filter((k) => !visibleKeySet.has(k))
  ]);
  // Prune selections whose row no longer exists in the SOURCE data (a background refresh deleted it) —
  // otherwise selectedList would feed a bulk action ids for gone rows. Query-hidden rows stay in
  // `rows`, so they're correctly preserved; only truly-absent keys are dropped.
  $effect(() => {
    const live = new Set(rows.map(rowKey));
    if ([...selected].some((k) => !live.has(k))) {
      selected = new Set([...selected].filter((k) => live.has(k)));
    }
  });
  function toggleAll() {
    if (allSelected) {
      // Deselect only the visible rows; keep any selected-but-hidden rows intact.
      const n = new Set(selected);
      for (const k of visibleKeys) n.delete(k);
      selected = n;
    } else {
      selected = new Set([...selected, ...visibleKeys]);
    }
  }
  // +1 for the trailing flex spacer column (the sole slack absorber → predictable resize, no reflow).
  const colSpan = $derived(columns.length + (expand ? 1 : 0) + (selectable ? 1 : 0) + 1);
  // Column width source of truth, applied via <colgroup> (best practice for table-layout:fixed). A
  // resized width wins; else the configured width; else a default so the SPACER is the only auto col.
  const colWidth = (c: { key: string; width?: string; grow?: boolean }): string =>
    colW[c.key] ?? c.width ?? (c.grow ? '260px' : '160px');
  const rowExpandable = (r: Row) => !!expand && (!canExpand || canExpand(r));
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
      <colgroup>
        {#if selectable}<col style="width:36px" />{/if}
        {#if expand}<col style="width:28px" />{/if}
        {#each columns as c (c.key)}<col style={`width:${colWidth(c)}`} />{/each}
        <col class="dt-col-spacer" />
      </colgroup>
      <thead>
        <tr>
          {#if selectable}
            <th class="dt-sel">
              <input class="dt-check" type="checkbox" checked={allSelected} indeterminate={someSelected && !allSelected} onchange={toggleAll} aria-label={t('common.selectAll')} />
            </th>
          {/if}
          {#if expand}<th class="dt-exp" aria-hidden="true"></th>{/if}
          {#each columns as c, i (c.key)}
            <th
              class="dt-th align-{c.align ?? 'left'}"
              class:sortable={c.sortable}
              class:active={sortKey === c.key}
              aria-sort={sortKey === c.key ? (sortDir === 'asc' ? 'ascending' : 'descending') : undefined}
            >
              {#if c.sortable}
                <button
                  type="button"
                  class="dt-th-btn"
                  onclick={() => { if (!resizing) toggleSort(c); }}
                  aria-label={t('common.sortBy', { label: c.label })}
                >
                  <span class="dt-thlabel">{c.label}<span class="dt-arrow">{sortKey === c.key ? (sortDir === 'asc' ? '↑' : '↓') : '↕'}</span></span>
                </button>
              {:else}
                <span class="dt-thlabel">{c.label}</span>
              {/if}
              {#if i < columns.length - 1}<span class="dt-resize" onpointerdown={(e) => startResize(e, c.key)} onclick={(e) => e.stopPropagation()} role="separator" aria-hidden="true"></span>{/if}
            </th>
          {/each}
          <th class="dt-spacer" aria-hidden="true"></th>
        </tr>
      </thead>
      <tbody>
        {#if loading}
          {#each Array(6) as _, i (i)}
            <tr class="dt-row dt-skrow">
              {#if selectable}<td class="dt-sel"></td>{/if}
              {#if expand}<td class="dt-exp"></td>{/if}
              {#each columns as c (c.key)}
                <td class="align-{c.align ?? 'left'}"><span class="dt-sk" style="width:{c.align === 'right' ? '58px' : c.grow ? '78%' : '54%'}"></span></td>
              {/each}
              <td class="dt-spacer"></td>
            </tr>
          {/each}
        {:else}
        {#each sorted as row (rowKey(row))}
          {@const k = rowKey(row)}
          {@const exp = rowExpandable(row)}
          <tr class="dt-row" class:open={openKeys.has(k)} class:sel={selected.has(k)} class:clickable={exp}
            class:muted={rowMuted?.(row)} class:accent={rowAccent?.(row)}
            style={rowStyle?.(row)} data-highlight-id={highlightAttr?.(row) ?? undefined} onclick={() => exp && toggleOpen(k)}>
            {#if selectable}
              <td class="dt-sel" onclick={(e) => e.stopPropagation()}>
                <input class="dt-check" type="checkbox" checked={selected.has(k)} onchange={() => toggleSel(k)} aria-label={t('common.selectRow')} />
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
            <td class="dt-spacer"></td>
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
          <tr><td colspan={colSpan} class="dt-empty">{#if empty}{@render empty()}{:else}{t('common.noMatches')}{/if}</td></tr>
        {/if}
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
  /* Canon: the toolbar is its OWN tinted band, clearly separated from the (quieter) column-header
     row — the old flat bar read as one block with the headers ("поиск смешан со столбцами"). */
  .dt-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sw-space-2);
    padding: var(--sw-space-2) var(--sw-space-3);
    background: var(--sw-bg-subtle);
    border-bottom: 1px solid var(--sw-border);
  }
  .dt-search {
    min-width: 200px;
    max-width: 300px;
    flex: 0 1 300px;
    font-size: var(--sw-text-xs);
  }
  /* Optional slack element parents can drop after the search to push trailing controls right. */
  .dt-bar :global(.dt-bar-spacer) {
    flex: 1 1 auto;
  }
  .dt-bulk {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
    padding: var(--sw-space-2) var(--sw-space-3);
    border-bottom: 1px solid var(--sw-border);
    background: var(--sw-accent-glow, rgba(59, 130, 246, 0.12));
    font-size: var(--sw-text-xs);
  }
  /* The sticky <thead> needs a scrollport that actually scrolls VERTICALLY. Without a height cap the
     wrapper is content-height, its vertical overflow is zero, and sticky never engages — the page's
     <main> is the element that scrolls, and it's an ancestor of the scrollport so it can't drive the
     offset. Cap the height here so the header stays put while long tables scroll inside the card. */
  /* Subtract a constant rather than taking a fraction of the viewport: the chrome above the table
     (page header, toolbar, banners) is roughly fixed in px, so `70vh` measured 83px — 1.6 rows —
     PAST the window bottom on a 720px-tall window, and those rows were unreachable at full scroll.
     A fraction also fails the other way on a short window. Erring small only costs scrolling. */
  .dt-scroll {
    overflow-x: auto;
    overflow-y: auto;
    max-height: calc(100vh - 20rem);
  }
  table.dt {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--sw-text-sm);
    font-variant-numeric: tabular-nums;
    table-layout: fixed;
  }
  /* Trailing flex column: the ONLY auto-width column, so it absorbs all slack (data columns stay
     compact, no bloat) and absorbs resize deltas (the dragged border tracks the cursor 1:1). */
  .dt-col-spacer {
    width: auto;
  }
  .dt-spacer {
    padding: 0;
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
    /* Persistent faint 1px divider at the column boundary so it's clear where to grab. */
    background: linear-gradient(to right, transparent 3px, var(--sw-border) 3px, var(--sw-border) 4px, transparent 4px);
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
    padding: var(--sw-space-2) var(--sw-space-3);
    text-align: left;
    font-size: var(--sw-text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
    white-space: nowrap;
    user-select: none;
  }
  .dt-th.sortable {
    padding: 0;
  }
  /* Native <button> inside <th>: gets keyboard focus, Enter/Space activation, AT semantics
     ("button, sort by name") for free — no need for tabindex/role/onkeydown on the <th>. */
  .dt-th-btn {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    height: 100%;
    padding: var(--sw-space-2) var(--sw-space-3);
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    text-transform: inherit;
    letter-spacing: inherit;
  }
  .dt-th-btn:hover {
    color: var(--sw-text-secondary);
  }
  .dt-th-btn:focus-visible {
    outline: 2px solid var(--sw-accent-text);
    outline-offset: -2px;
  }
  .dt-th.active .dt-th-btn {
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
  .dt-th.sortable .dt-th-btn:hover .dt-arrow {
    opacity: 0.5;
  }
  .dt tbody td {
    padding: var(--sw-space-2) var(--sw-space-3);
    border-bottom: 1px solid var(--sw-border);
    vertical-align: middle;
    height: 38px;
    /* Safety net for `table-layout: fixed`: a cell whose content is intrinsically wider than its
       colgroup width (e.g. a `white-space:nowrap` badge, or flex chips) must NOT paint over the
       neighbouring column — clip it to its own box. Content that wraps vertically (chip rows) still
       grows the row height (this only clips the horizontal axis). Chromium/WebView2 honours overflow
       on table cells. Fixes the МCP profiles→actions and Plugins policy→actions bleed. */
    overflow: hidden;
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
    box-shadow: inset 3px 0 0 var(--row-accent, var(--sw-accent-text));
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
    font-size: var(--sw-text-xs);
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
    padding: var(--sw-space-3) 0 var(--sw-space-4);
  }
  .dt-empty {
    text-align: center;
    color: var(--sw-text-muted);
    padding: 22px;
  }
  /* Canonical in-table skeleton (initial load) — a shimmering bar per cell, sized to its column.
     Replaces the ad-hoc per-tab skeletons so every table loads the same way. */
  .dt-sk {
    display: inline-block;
    height: 12px;
    max-width: 100%;
    border-radius: 6px;
    background: linear-gradient(90deg, var(--sw-bg-subtle) 25%, var(--sw-bg-hover) 37%, var(--sw-bg-subtle) 63%);
    background-size: 400% 100%;
    animation: dt-shim 1.3s ease infinite;
  }
  @keyframes dt-shim {
    0% {
      background-position: 100% 0;
    }
    100% {
      background-position: 0 0;
    }
  }
  .dt-skrow:hover td {
    background: none;
  }
  .dt-th.sortable .dt-th-btn:focus-visible,
  .dt-expbtn:focus-visible {
    outline: 2px solid var(--sw-accent-text);
    outline-offset: -2px;
  }
</style>
