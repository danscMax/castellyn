import { describe, it, expect } from 'vitest';
import {
  updatesAttention,
  forksAttention,
  backupAttention,
  profilesAttention,
  pluginsAttention,
  syncAttention,
  sessionsAttention
} from './attention';

describe('attention', () => {
  it('updates sum changed counts', () => {
    const comps = [{ id: 'a' }, { id: 'b' }] as any;
    const st = { a: { counts: { changed: 1 } }, b: { counts: { changed: 7 } } };
    expect(updatesAttention(comps, st)).toEqual({ level: 'info', count: 8 });
    expect(updatesAttention(comps, { a: { counts: { changed: 0 } } })).toBeNull();
  });

  it('forks reflect needHands', () => {
    expect(forksAttention({ summary: { needHands: 4 } } as any)).toEqual({ level: 'warn', count: 4 });
    expect(forksAttention({ summary: { needHands: 0 } } as any)).toBeNull();
    expect(forksAttention(null)).toBeNull();
  });

  it('backup flags staleness > 2 days', () => {
    expect(backupAttention({ state: { lastRun: new Date().toISOString() } } as any)).toBeNull();
    const old = new Date(Date.now() - 5 * 86_400_000).toISOString();
    expect(backupAttention({ state: { lastRun: old } } as any)).toEqual({ level: 'warn' });
    expect(backupAttention(null)).toBeNull();
  });

  it('profiles flag broken links', () => {
    // A present link is not broken.
    expect(
      profilesAttention({ profiles: [{ exists: true, sharedLinks: { agents: 'SymbolicLink' } }] } as any)
    ).toBeNull();
    // A missing link (null) is broken.
    expect(
      profilesAttention({ profiles: [{ exists: true, sharedLinks: { agents: null } }] } as any)
    ).toEqual({ level: 'warn', count: 1 });
    // Real data ("none") is NOT broken — regression guard for the false "1" badge on ccfree.
    expect(
      profilesAttention({ profiles: [{ exists: true, sharedLinks: { plugins: 'none' } }] } as any)
    ).toBeNull();
    // sync conflicts alone must still count (regression: count used to drop conflicts → "0").
    expect(
      profilesAttention({
        profiles: [{ exists: true, sharedLinks: { agents: 'SymbolicLink' } }],
        syncConflicts: { count: 2 }
      } as any)
    ).toEqual({ level: 'warn', count: 1 });
  });

  it('plugins reflect update count', () => {
    expect(pluginsAttention(3)).toEqual({ level: 'info', count: 3 });
    expect(pluginsAttention(0)).toBeNull();
  });

  it('sync flags stignore drift only when deployed', () => {
    expect(syncAttention({ stignoreExists: true, stignoreMatches: false } as any)).toEqual({
      level: 'warn'
    });
    expect(syncAttention({ stignoreExists: true, stignoreMatches: true } as any)).toBeNull();
    expect(syncAttention({ stignoreExists: false, stignoreMatches: false } as any)).toBeNull();
    expect(syncAttention(null)).toBeNull();
  });

  it('sessions: blocked=danger, done=teal, none otherwise (#10 herdr palette)', () => {
    expect(sessionsAttention({ blocked: 2, done: 1 })).toEqual({ level: 'danger', count: 2 });
    expect(sessionsAttention({ blocked: 0, done: 3 })).toEqual({ level: 'done', count: 3 });
    expect(sessionsAttention({ blocked: 0, done: 0 })).toBeNull();
  });
});
