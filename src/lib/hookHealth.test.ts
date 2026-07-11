import { describe, it, expect } from 'vitest';
import { hookHealth } from './hookHealth';
import type { AgentStatusHookState } from './ipc';

const st = (p: Partial<AgentStatusHookState>): AgentStatusHookState => ({
  wired: [],
  unwired: [],
  partial: [],
  scriptPresent: true,
  ...p
});

describe('hookHealth', () => {
  it('null backend → unavailable', () => {
    expect(hookHealth(null).status).toBe('unavailable');
  });

  it('nothing wired → off (nothing to fix)', () => {
    expect(hookHealth(st({ unwired: ['main', 'work'] })).status).toBe('off');
  });

  it('wired but script gone → script-missing (silent no-op trap), outranks partial', () => {
    const h = hookHealth(st({ wired: ['main'], unwired: ['work'], scriptPresent: false }));
    expect(h.status).toBe('script-missing');
    expect(h.total).toBe(2);
  });

  it('some profiles unwired → partial', () => {
    const h = hookHealth(st({ wired: ['main'], unwired: ['work'] }));
    expect(h.status).toBe('partial');
    expect(h.wired).toBe(1);
    expect(h.total).toBe(2);
  });

  it('event-level drift → partial with a drift count', () => {
    const h = hookHealth(
      st({ wired: ['main'], partial: [{ profile: 'work', missing: ['Stop', 'SessionEnd'] }] })
    );
    // `work` is wired-for-some, so it is not in `unwired`; total counts only wired+unwired here.
    expect(h.status).toBe('partial');
    expect(h.drift).toBe(1);
  });

  it('all wired + script present → healthy', () => {
    const h = hookHealth(st({ wired: ['main', 'work'] }));
    expect(h.status).toBe('healthy');
    expect(h.wired).toBe(2);
    expect(h.total).toBe(2);
    expect(h.drift).toBe(0);
  });
});
