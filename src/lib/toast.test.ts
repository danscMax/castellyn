import { describe, it, expect } from 'vitest';
import { nextSeqFrom } from './toast.svelte';

// RT-001: the notification panel keys its {#each} by `item.id`. `seq` lives in module scope and
// restarts at 0 on every load, while the history it must not collide with survives in localStorage.
// If the counter did not resume above the restored ids, the first new toast would reuse id 1 and
// Svelte would throw `each_key_duplicate` — the very bug this replaced.
describe('nextSeqFrom', () => {
  it('returns 0 for an empty history so a fresh install starts at id 1', () => {
    expect(nextSeqFrom([])).toBe(0);
  });

  it('resumes above the highest restored id, not the last one', () => {
    // Order is newest-first in the store, so the max is not necessarily items[0].
    expect(nextSeqFrom([{ id: 5 }, { id: 9 }, { id: 2 }])).toBe(9);
  });

  it('ignores position and returns the max even when ids are non-contiguous', () => {
    expect(nextSeqFrom([{ id: 50 }])).toBe(50);
  });
});
