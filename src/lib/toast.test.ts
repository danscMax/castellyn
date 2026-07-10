import { describe, it, expect } from 'vitest';
import { renumberHistory, pushToast, dismiss, toastStore } from './toast.svelte';

// RT-001: the notification panel keys its {#each} by `item.id`, and Svelte throws on a duplicate key.
// `seq` restarts at 0 on every load, so a history persisted across several runs carries colliding ids
// (a real one on disk had 50 entries and 16 distinct ids). Restored ids are therefore not trustworthy
// and get reassigned — this is the invariant that keeps the panel renderable.
describe('renumberHistory', () => {
  it('makes ids unique even when the stored history repeats them', () => {
    const stored = [{ id: 1 }, { id: 3 }, { id: 1 }, { id: 2 }, { id: 3 }];
    const ids = renumberHistory(stored).map((x) => x.id);
    expect(new Set(ids).size).toBe(stored.length);
  });

  it('numbers newest-first, so a later unshift gets a higher id and existing keys stay put', () => {
    // Store order is newest → oldest.
    expect(renumberHistory([{ id: 9 }, { id: 9 }, { id: 9 }]).map((x) => x.id)).toEqual([3, 2, 1]);
  });

  it('keeps every other field intact', () => {
    const [only] = renumberHistory([{ id: 7, title: 'done', timestamp: 42 }]);
    expect(only).toEqual({ id: 1, title: 'done', timestamp: 42 });
  });

  it('handles an empty history so a fresh install starts the counter at 0', () => {
    expect(renumberHistory([])).toEqual([]);
  });
});

// A repeat of an identical visible toast must bump a ×N counter, not stack a clone — a flapping
// poller emitting the same error every round used to wallpaper the corner with duplicates.
describe('pushToast dedup (×N)', () => {
  it('collapses an identical repeat into a counter on the existing toast', () => {
    const id1 = pushToast({ kind: 'error', title: 'boom', detail: 'x' });
    const id2 = pushToast({ kind: 'error', title: 'boom', detail: 'x' });
    expect(id2).toBe(id1);
    expect(toastStore.items.filter((t) => t.title === 'boom')).toHaveLength(1);
    expect(toastStore.items.find((t) => t.id === id1)?.count).toBe(2);
    dismiss(id1);
  });

  it('does not merge across a different kind or detail', () => {
    const a = pushToast({ kind: 'error', title: 'boom', detail: 'x' });
    const b = pushToast({ kind: 'warn', title: 'boom', detail: 'x' });
    const c = pushToast({ kind: 'error', title: 'boom', detail: 'y' });
    expect(new Set([a, b, c]).size).toBe(3);
    [a, b, c].forEach(dismiss);
  });
});
