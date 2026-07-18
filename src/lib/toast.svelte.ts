// Tiny app-wide toast store (Svelte 5 runes). Surfaces operation outcomes so users get a
// glanceable result without reading the execution log. Errors linger (15s+) but auto-dismiss.
export type ToastKind = 'success' | 'warn' | 'error' | 'info';
export type ToastAction = { label: string; onClick: () => void };
export type Toast = {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
  action?: ToastAction;
  /** How many identical arrivals this toast represents (rendered as ×N when > 1). */
  count?: number;
};

export type ToastWithMeta = Toast & { timestamp: number };

// History survives restarts (last 50 in localStorage). Actions hold live closures — they are
// stripped on save and absent after reload (the panel never renders them anyway).
const HIST_KEY = 'cmh-notif-history';

/// Reassign ids over a restored history so they are unique, newest-first.
///
/// `seq` lives in module scope and restarts at 0 on every load, so entries persisted across several
/// runs carry colliding ids — a real history here held 50 entries with only 16 distinct ids. The panel
/// keys its `{#each}` by id and Svelte *throws* on a duplicate key, so restored ids cannot be trusted:
/// they are renumbered on load, and `seq` resumes above them. Descending order keeps the key of a given
/// entry stable as newer toasts are unshifted in front of it. Pure, so the invariant is unit-testable.
export function renumberHistory<T extends { id: number }>(items: readonly T[]): T[] {
  const n = items.length;
  return items.map((x, i) => ({ ...x, id: n - i }));
}

function loadHistory(): ToastWithMeta[] {
  try {
    const arr = JSON.parse(localStorage.getItem(HIST_KEY) ?? '[]') as unknown;
    if (!Array.isArray(arr)) return [];
    const items = renumberHistory(
      (arr as ToastWithMeta[])
        .filter((x) => x && typeof x.title === 'string' && typeof x.timestamp === 'number')
        .slice(0, 50)
    );
    seq = items.length;
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

// Tracks hover-pause state so a duplicate arrival (see pushToast) doesn't re-arm a live timer
// while the user is hovering — otherwise it would defeat the hover-pause guarantee below.
let paused = false;

function arm(id: number, ttl: number): void {
  timers.set(id, { ttl, handle: setTimeout(() => dismiss(id), ttl) });
}

export function pushToast(t: Omit<Toast, 'id'>, ttlMs = 6000): number {
  // A repeat of an identical visible toast bumps a ×N counter instead of stacking a clone — a
  // flapping poller or a bulk run can't wallpaper the corner with the same message. The countdown
  // restarts so the (still-arriving) message doesn't vanish mid-burst.
  const dup = toastStore.items.find(
    (x) => x.kind === t.kind && x.title === t.title && x.detail === t.detail
  );
  if (dup) {
    dup.count = (dup.count ?? 1) + 1;
    // The key is kind+title+detail only — the newer arrival's action can differ (e.g. "Open log"
    // closing over a different runId). Adopt it so the button targets the LATEST event, not the first.
    dup.action = t.action;
    const tm = timers.get(dup.id);
    if (tm) {
      clearTimeout(tm.handle);
      if (!paused) arm(dup.id, tm.ttl);
    }
    return dup.id;
  }
  const id = ++seq;
  toastStore.items.push({ ...t, id });
  // Errors linger longer (they matter more) but no longer stick FOREVER — a sticky error survived
  // navigation across several tabs long after its cause was fixed (iso-audit 2026-07-18). 15s is
  // ample to read; hover-pause still freezes it while the user reaches for its action button.
  if (ttlMs > 0) arm(id, t.kind === 'error' ? Math.max(ttlMs, 15000) : ttlMs);
  return id;
}

// Pause/resume every pending auto-dismiss — wired to the toast host's hover so an actionable toast
// (Open log / jump-to-tab) doesn't vanish mid-read or while the user reaches for its button.
export function pauseToasts(): void {
  paused = true;
  for (const tm of timers.values()) clearTimeout(tm.handle);
}
export function resumeToasts(): void {
  paused = false;
  for (const [id, tm] of [...timers]) arm(id, tm.ttl);
}

function pushToHistory(t: Toast): void {
  // A persistently-flapping background error (a broken plugins file failing every reloadExtensions,
  // an unreachable service polled on a timer) would otherwise unshift an identical entry every ~15s
  // toast lifecycle and bury the rest of the log. Collapse a consecutive identical arrival into the
  // newest entry's ×N count + refreshed timestamp — the same dedup pushToast does for visible toasts,
  // extended to the persisted history. Only index 0 is checked: interleaved notifications still get
  // their own timeline entry; only pure back-to-back repeats merge.
  const [newest] = toastStore.history.items;
  if (newest && newest.kind === t.kind && newest.title === t.title && newest.detail === t.detail) {
    newest.count = (newest.count ?? 1) + (t.count ?? 1);
    newest.timestamp = Date.now();
    toastStore.history.unread++;
    saveHistory();
    return;
  }
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

export function dismissFromHistory(id: number): void {
  // Filter by the unique id (renumberHistory keeps them unique + newest-first), NOT the timestamp:
  // dismissAll() pushes a burst of items whose Date.now() stamps collide within a millisecond, so a
  // timestamp filter would drop several entries on a single ×-click.
  toastStore.history.items = toastStore.history.items.filter((x) => x.id !== id);
  saveHistory();
}
