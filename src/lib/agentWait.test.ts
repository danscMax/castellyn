import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { agentWait, clearAgentWait, noteWait } from './agentStatus.svelte';

// Backlog 25. The metric is an accumulator fed by a stream of transitions, which is exactly the
// shape that silently produces a plausible-looking wrong number: a wait counted twice, a wait whose
// clock never stopped, `limited` folded in as if the user could answer it. Pin the arithmetic.
describe('agentWait', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    clearAgentWait();
  });
  afterEach(() => vi.useRealTimers());

  it('measures a wait from blocked until the pane moves on', () => {
    noteWait('s1', 'blocked');
    vi.advanceTimersByTime(5_000);
    noteWait('s1', 'working');
    expect(agentWait.count).toBe(1);
    expect(agentWait.totalMs).toBe(5_000);
    expect(agentWait.longestMs).toBe(5_000);
  });

  it('keeps the original start when blocked is re-reported', () => {
    // The backend re-emits on every flag change, not only on a state change — a repeat `blocked`
    // must not restart the clock, or a long wait would report as the gap between two events.
    noteWait('s1', 'blocked');
    vi.advanceTimersByTime(3_000);
    noteWait('s1', 'blocked');
    vi.advanceTimersByTime(3_000);
    noteWait('s1', 'idle');
    expect(agentWait.totalMs).toBe(6_000);
    expect(agentWait.count).toBe(1);
  });

  it('counts each pane separately and keeps the longest', () => {
    noteWait('s1', 'blocked');
    noteWait('s2', 'blocked');
    vi.advanceTimersByTime(2_000);
    noteWait('s1', 'done');
    vi.advanceTimersByTime(8_000);
    noteWait('s2', 'done');
    expect(agentWait.count).toBe(2);
    expect(agentWait.totalMs).toBe(12_000); // 2s + 10s
    expect(agentWait.longestMs).toBe(10_000);
  });

  it('ignores states that were never a wait', () => {
    // `limited` is the agent parked on quota — not something the user can answer, so it must not
    // inflate "how long did agents wait for ME". A close with no open wait is a no-op, not a NaN.
    noteWait('s1', 'limited');
    vi.advanceTimersByTime(9_000);
    noteWait('s1', 'working');
    noteWait('s2', 'idle');
    expect(agentWait.count).toBe(0);
    expect(agentWait.totalMs).toBe(0);
  });

  it('never bills a negative wait when the clock steps backwards', () => {
    // Wall-clock deltas, and the totals are cumulative — a single negative would corrupt them for
    // good, with no recomputation to heal it.
    noteWait('s1', 'blocked');
    vi.setSystemTime(Date.now() - 10_000);
    noteWait('s1', 'idle');
    expect(agentWait.totalMs).toBe(0);
    expect(agentWait.longestMs).toBe(0);
  });

  it('drops an open wait on reset instead of billing it to the next one', () => {
    noteWait('s1', 'blocked');
    vi.advanceTimersByTime(4_000);
    clearAgentWait();
    vi.advanceTimersByTime(4_000);
    noteWait('s1', 'idle');
    expect(agentWait.count).toBe(0);
    expect(agentWait.totalMs).toBe(0);
  });
});
