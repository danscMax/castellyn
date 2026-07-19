import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  renumberHistory,
  pushToast,
  dismiss,
  clearHistory,
  toastStore,
  pauseToasts,
  resumeToasts
} from './toast.svelte';

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

// The host pauses auto-dismiss on hover; each toast also pauses on focus. Those overlap, so the
// pause is refcounted — a single shared flag let whichever resume fired first unfreeze the timers
// while the other source was still holding them, and the toast vanished mid-read.
describe('pause refcounting', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('keeps the toast alive while one pause source is still holding it', () => {
    vi.useFakeTimers();
    const id = pushToast({ kind: 'info', title: 'hold me' }, 1000);
    pauseToasts(); // host hover
    pauseToasts(); // focus lands on the toast
    resumeToasts(); // focus leaves — but the pointer is STILL over the host
    vi.advanceTimersByTime(5000);
    expect(toastStore.items.some((x) => x.id === id)).toBe(true);
    resumeToasts(); // pointer finally leaves
    vi.advanceTimersByTime(5000);
    expect(toastStore.items.some((x) => x.id === id)).toBe(false);
  });

  it('does not strand the next pause after an unpaired resume', () => {
    vi.useFakeTimers();
    resumeToasts(); // stray focusout with no matching focusin
    const id = pushToast({ kind: 'info', title: 'clamp' }, 1000);
    pauseToasts();
    vi.advanceTimersByTime(5000);
    expect(toastStore.items.some((x) => x.id === id)).toBe(true);
    resumeToasts();
    vi.advanceTimersByTime(5000);
    expect(toastStore.items.some((x) => x.id === id)).toBe(false);
  });
});

// A background error that keeps re-firing (e.g. list_plugins failing on every reloadExtensions) dismisses
// a fresh toast every ~15s — each landing in history. Without a merge, the panel fills with identical
// entries and buries everything else. History collapses consecutive identical dismissals into one ×N.
describe('pushToHistory dedup (×N across dismissals)', () => {
  it('merges a consecutive identical dismissed toast into the newest history entry', () => {
    clearHistory();
    dismiss(pushToast({ kind: 'error', title: 'plug', detail: 'z' }));
    dismiss(pushToast({ kind: 'error', title: 'plug', detail: 'z' }));
    const hits = toastStore.history.items.filter((x) => x.title === 'plug' && x.detail === 'z');
    expect(hits).toHaveLength(1);
    expect(hits[0].count).toBe(2);
  });

  it('keeps a different notification as its own timeline entry', () => {
    clearHistory();
    dismiss(pushToast({ kind: 'error', title: 'plug', detail: 'z' }));
    dismiss(pushToast({ kind: 'info', title: 'other', detail: 'q' }));
    dismiss(pushToast({ kind: 'error', title: 'plug', detail: 'z' }));
    // The interleaved 'other' breaks the run, so the second 'plug' can't merge into the first.
    expect(toastStore.history.items).toHaveLength(3);
  });
});
