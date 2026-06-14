import { describe, it, expect } from 'vitest';
import {
  updatesAttention,
  forksAttention,
  backupAttention,
  profilesAttention,
  pluginsAttention,
  syncAttention
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
    expect(profilesAttention({ profiles: [{ exists: true, linksIntact: true }] } as any)).toBeNull();
    expect(profilesAttention({ profiles: [{ exists: true, linksIntact: false }] } as any)).toEqual({
      level: 'warn',
      count: 1
    });
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
});
