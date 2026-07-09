// Tiny app-wide toast store (Svelte 5 runes). Surfaces operation outcomes so users get a
// glanceable result without reading the execution log. Errors are sticky (manual dismiss).
export type ToastKind = 'success' | 'warn' | 'error' | 'info';
export type ToastAction = { label: string; onClick: () => void };
export type Toast = {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
  action?: ToastAction;
};

export type ToastWithMeta = Toast & { timestamp: number };

// History survives restarts (last 50 in localStorage). Actions hold live closures — they are
// stripped on save and absent after reload (the panel never renders them anyway).
const HIST_KEY = 'cmh-notif-history';

/// Highest id among restored entries (0 when there are none). `seq` must resume above this: it lives
/// in module scope and restarts at 0 on every load, while the history it is compared against survives
/// in localStorage. Pure so the invariant is unit-testable without a DOM.
export function nextSeqFrom(items: readonly { id: number }[]): number {
  return items.reduce((max, i) => (i.id > max ? i.id : max), 0);
}

function loadHistory(): ToastWithMeta[] {
  try {
    const arr = JSON.parse(localStorage.getItem(HIST_KEY) ?? '[]') as unknown;
    if (!Array.isArray(arr)) return [];
    // `id` is now load-bearing (the panel keys its {#each} by it), so an entry without one is dropped
    // rather than rendered with an `undefined` key — two of those would collide exactly as timestamps did.
    const items = (arr as ToastWithMeta[])
      .filter(
        (x) =>
          x && typeof x.title === 'string' && typeof x.timestamp === 'number' && typeof x.id === 'number'
      )
      .slice(0, 50);
    seq = nextSeqFrom(items);
    return items;
  } catch {
    return []; // no localStorage (tests) or corrupt payload — start empty
  }
}
function saveHistory(): void {
  try {
    localStorage.setItem(
      HIST_KEY,
      JSON.stringify(toastStore.history.items.map(({ action: _a, ...rest }) => rest))
    );
  } catch {
    /* ignore */
  }
}

let seq = 0;
export const toastStore = $state<{ items: Toast[]; history: { items: ToastWithMeta[]; unread: number } }>({
  items: [],
  history: { items: loadHistory(), unread: 0 }
});

// Live auto-dismiss timers, keyed by toast id, so the stack can pause while the user hovers/reads it
// (errors are sticky and never armed). The remembered ttl lets resume restart a fresh countdown.
const timers = new Map<number, { ttl: number; handle: ReturnType<typeof setTimeout> }>();

function arm(id: number, ttl: number): void {
  timers.set(id, { ttl, handle: setTimeout(() => dismiss(id), ttl) });
}

export function pushToast(t: Omit<Toast, 'id'>, ttlMs = 6000): number {
  const id = ++seq;
  toastStore.items.push({ ...t, id });
  if (t.kind !== 'error' && ttlMs > 0) arm(id, ttlMs);
  return id;
}

// Pause/resume every pending auto-dismiss — wired to the toast host's hover so an actionable toast
// (Open log / jump-to-tab) doesn't vanish mid-read or while the user reaches for its button.
export function pauseToasts(): void {
  for (const tm of timers.values()) clearTimeout(tm.handle);
}
export function resumeToasts(): void {
  for (const [id, tm] of [...timers]) arm(id, tm.ttl);
}

function pushToHistory(t: Toast): void {
  toastStore.history.items = [{ ...t, timestamp: Date.now() }, ...toastStore.history.items].slice(0, 50);
  toastStore.history.unread++;
  saveHistory();
}

export function dismiss(id: number): void {
  const tm = timers.get(id);
  if (tm) clearTimeout(tm.handle);
  timers.delete(id);
  const item = toastStore.items.find((x) => x.id === id);
  if (item) pushToHistory(item);
  toastStore.items = toastStore.items.filter((x) => x.id !== id);
}

// Clear the whole stack at once (a bulk failure can spawn many sticky error toasts).
export function dismissAll(): void {
  for (const tm of timers.values()) clearTimeout(tm.handle);
  timers.clear();
  for (const item of toastStore.items) pushToHistory(item);
  toastStore.items = [];
}

export function markNotifRead(): void {
  toastStore.history.unread = 0;
}

export function clearHistory(): void {
  toastStore.history.items = [];
  toastStore.history.unread = 0;
  saveHistory();
}

export function dismissFromHistory(timestamp: number): void {
  toastStore.history.items = toastStore.history.items.filter((x) => x.timestamp !== timestamp);
  saveHistory();
}
