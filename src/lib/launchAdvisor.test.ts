import { describe, it, expect } from 'vitest';
import { launchAdvisor, effortForTaskClass, type LaunchTaskClass } from './launchAdvisor';
import { LIMITS_STALE_MS } from './limitSwitch';
import type { ProfileInfo, LimitsStatusEvent } from './ipc';

const prof = (name: string, over: Partial<ProfileInfo> = {}): ProfileInfo => ({
  name,
  description: '',
  color: '',
  exists: true,
  credentialsPresent: true,
  settingsPresent: true,
  sharedLinks: {},
  linksIntact: true,
  ...over
});
const lim = (profile: string, h5: number | null, scoped: number | null = null): LimitsStatusEvent => ({
  profile,
  h5,
  d7: null,
  h5Reset: null,
  d7Reset: null,
  scoped,
  scopedLabel: scoped == null ? null : 'Fable',
  scopedReset: null,
  extraEnabled: false,
  extraPct: null,
  expired: false,
  rateLimited: false
});

describe('effortForTaskClass', () => {
  const cases: [LaunchTaskClass, string][] = [
    ['inspect', 'low'],
    ['review', 'low'],
    ['fix', 'medium'],
    ['feature', 'medium'],
    ['debug', 'high'],
    ['architecture', 'high'],
    ['critical', 'max']
  ];
  for (const [tc, effort] of cases) {
    it(`${tc} → ${effort}`, () => expect(effortForTaskClass(tc)).toBe(effort));
  }
});

describe('launchAdvisor', () => {
  it('recommends the least-utilised eligible profile with the task-class effort', () => {
    const profiles = [prof('a'), prof('b'), prof('c')];
    const limits = { a: lim('a', 40), b: lim('b', 10), c: lim('c', 70) };
    const adv = launchAdvisor(profiles, limits, 'feature');
    expect(adv.recommendation).toEqual({ profile: 'b', effort: 'medium', util: 10 });
    expect(adv.eligible.map((e) => e.name)).toEqual(['b', 'a', 'c']);
  });

  it('rejects each unusable profile with a structured reason', () => {
    const profiles = [
      prof('gone', { exists: false }),
      prof('noauth', { credentialsPresent: false }),
      prof('dead', { credentialsValid: false }),
      prof('wizard', { needsOnboarding: true }),
      prof('broken', { linksIntact: false }),
      prof('ok')
    ];
    const limits = {
      gone: lim('gone', 1),
      noauth: lim('noauth', 1),
      dead: lim('dead', 1),
      wizard: lim('wizard', 1),
      broken: lim('broken', 1),
      ok: lim('ok', 20)
    };
    const adv = launchAdvisor(profiles, limits, 'fix');
    expect(adv.recommendation?.profile).toBe('ok');
    const byName = Object.fromEntries(adv.rejected.map((r) => [r.name, r.reason]));
    expect(byName).toEqual({
      gone: 'missing',
      noauth: 'no-credentials',
      dead: 'invalid-credentials',
      wizard: 'needs-onboarding',
      broken: 'broken-links'
    });
  });

  it('treats absent / null / stale / over-threshold usage as unusable, never as zero', () => {
    const now = 10_000_000;
    const profiles = [prof('absent'), prof('nulled'), prof('stale'), prof('hot'), prof('ok')];
    const limits = {
      // 'absent' has no datapoint at all
      nulled: lim('nulled', null),
      stale: { ...lim('stale', 5), receivedAt: now - LIMITS_STALE_MS - 1 },
      hot: lim('hot', 85), // 85 is NOT < 85
      ok: { ...lim('ok', 30), receivedAt: now - 1000 }
    };
    const adv = launchAdvisor(profiles, limits, 'inspect', new Set(), now);
    expect(adv.recommendation?.profile).toBe('ok');
    const byName = Object.fromEntries(adv.rejected.map((r) => [r.name, r.reason]));
    expect(byName.absent).toBe('usage-unknown');
    expect(byName.nulled).toBe('usage-unknown');
    expect(byName.stale).toBe('usage-unknown');
    expect(byName.hot).toBe('over-threshold');
  });

  it('weighs a model-scoped weekly cap over a calm 5h number', () => {
    const profiles = [prof('calm'), prof('capped')];
    // 'capped' looks best on 5h (2%) but its per-model week is at 91% — not a launch destination.
    const limits = { calm: lim('calm', 40), capped: lim('capped', 2, 91) };
    expect(launchAdvisor(profiles, limits, 'debug').recommendation?.profile).toBe('calm');
  });

  it('excludes reserved profiles claimed by same-tick launches', () => {
    const profiles = [prof('a'), prof('b')];
    const limits = { a: lim('a', 40), b: lim('b', 10) };
    expect(launchAdvisor(profiles, limits, 'fix', new Set(['b'])).recommendation?.profile).toBe('a');
    expect(launchAdvisor(profiles, limits, 'fix', new Set(['a', 'b'])).recommendation).toBeNull();
  });

  it('breaks utilisation ties deterministically by profile name', () => {
    const profiles = [prof('zeta'), prof('alpha')];
    const limits = { zeta: lim('zeta', 30), alpha: lim('alpha', 30) };
    expect(launchAdvisor(profiles, limits, 'review').recommendation?.profile).toBe('alpha');
  });

  it('returns a null recommendation with explainable reasons when nothing qualifies', () => {
    const profiles = [prof('a', { credentialsValid: false }), prof('b')];
    const limits = { a: lim('a', 1) }; // 'b' has no datapoint
    const adv = launchAdvisor(profiles, limits, 'critical');
    expect(adv.recommendation).toBeNull();
    expect(adv.rejected).toEqual(
      expect.arrayContaining([
        { name: 'a', reason: 'invalid-credentials' },
        { name: 'b', reason: 'usage-unknown' }
      ])
    );
  });
});
