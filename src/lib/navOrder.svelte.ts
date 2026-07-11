// U1: single source of truth for the tab order. The Sidebar renders it, drag/keyboard reorder
// mutates it, and +page derives the Ctrl+1..9 jumps and the palette's number hints from it — so
// the shortcuts always match what the user actually sees (they used to follow a separate
// hardcoded list in a different order).
//
// Persistence contract (unchanged from the Sidebar's original implementation): the saved order is
// honored only when stamped with the current ORD_VER; bumping ORD_VER re-seeds everyone to the new
// default once, while later manual reorders persist again.

/** Sidebar sections: items live inside a fixed group; drag/keyboard reorder works within a
 *  group, and the flat `navOrder.ids` is always kept partitioned in group order — so the
 *  visible order, the Ctrl+1..9 jumps and the palette hints all agree. `system` renders
 *  without a header (Settings pinned at the bottom). */
export const NAV_GROUPS: { id: string; labelKey: string; ids: string[] }[] = [
  { id: 'work', labelKey: 'nav.gWork', ids: ['home', 'sessions', 'profiles'] },
  { id: 'setup', labelKey: 'nav.gSetup', ids: ['providers', 'mcp', 'envs', 'extensions', 'agents'] },
  {
    id: 'maintain',
    labelKey: 'nav.gMaintain',
    ids: ['updates', 'forks', 'backup', 'sync', 'schedule', 'analytics']
  },
  { id: 'system', labelKey: '', ids: ['settings'] }
];

export function groupOf(id: string): string {
  return NAV_GROUPS.find((g) => g.ids.includes(id))?.id ?? 'system';
}

/** Stable-partition a flat order by group: group blocks in NAV_GROUPS order, the caller's
 *  relative order preserved within each block. */
function partitionByGroup(ids: string[]): string[] {
  return NAV_GROUPS.flatMap((g) => ids.filter((id) => g.ids.includes(id)));
}

const DEFAULT_ORDER = NAV_GROUPS.flatMap((g) => g.ids);
const ORD_KEY = 'cmh-sidebar-order';
const ORD_VER_KEY = 'cmh-sidebar-order-ver';
const ORD_VER = '5'; // 5: grouped sidebar — order is partitioned by NAV_GROUPS

function initialOrder(): string[] {
  try {
    const saved = JSON.parse(localStorage.getItem(ORD_KEY) ?? '[]');
    if (localStorage.getItem(ORD_VER_KEY) === ORD_VER && Array.isArray(saved) && saved.length) {
      const valid = saved.filter((id: string) => DEFAULT_ORDER.includes(id));
      const missing = DEFAULT_ORDER.filter((id) => !valid.includes(id));
      return partitionByGroup([...valid, ...missing]);
    }
    localStorage.setItem(ORD_KEY, JSON.stringify(DEFAULT_ORDER));
    localStorage.setItem(ORD_VER_KEY, ORD_VER);
  } catch {
    /* first run / storage unavailable */
  }
  return [...DEFAULT_ORDER];
}

/** Reactive ordered tab ids (SPA, ssr=false — safe to read localStorage at module init). */
export const navOrder = $state({ ids: initialOrder() });

/** Set a new order without persisting (live drag preview). */
export function previewNavOrder(ids: string[]) {
  navOrder.ids = ids;
}

/** Set and persist a new order (drop / keyboard move). Always stored partitioned. */
export function setNavOrder(ids: string[]) {
  const normalized = partitionByGroup(ids);
  navOrder.ids = normalized;
  try {
    localStorage.setItem(ORD_KEY, JSON.stringify(normalized));
    localStorage.setItem(ORD_VER_KEY, ORD_VER);
  } catch {
    /* ignore */
  }
}
