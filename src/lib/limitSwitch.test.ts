import { describe, it, expect } from 'vitest';
import { pickResumeCandidate, isProfileExhausted, LIMITS_STALE_MS } from './limitSwitch';
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

describe('pickResumeCandidate (#21e)', () => {
  it('picks the least-utilised eligible profile', () => {
    const profiles = [prof('cur'), prof('a'), prof('b')];
    const limits = { a: lim('a', 40), b: lim('b', 10), cur: lim('cur', 100) };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('b');
  });

  it('excludes the current profile even if it is the least utilised', () => {
    const profiles = [prof('cur'), prof('a')];
    const limits = { cur: lim('cur', 5), a: lim('a', 50) };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('a');
  });

  it('excludes non-OAuth, broken-links, missing-dir, and >=85% profiles', () => {
    const profiles = [
      prof('noauth', { credentialsPresent: false }),
      prof('broken', { linksIntact: false }),
      prof('gone', { exists: false }),
      prof('busy'),
      prof('ok')
    ];
    const limits = {
      noauth: lim('noauth', 1),
      broken: lim('broken', 1),
      gone: lim('gone', 1),
      busy: lim('busy', 85), // 85 is NOT < 85
      ok: lim('ok', 84)
    };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('ok');
  });

  it('excludes a profile whose stored tokens are dead, even though the file exists', () => {
    // .credentials.json survives a wipe with empty tokens — resuming there lands in a login prompt.
    const profiles = [prof('dead', { credentialsValid: false }), prof('ok')];
    const limits = { dead: lim('dead', 1), ok: lim('ok', 50) };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('ok');
  });

  it('excludes a profile stranded in the onboarding wizard by /logout', () => {
    const profiles = [prof('stranded', { needsOnboarding: true }), prof('ok')];
    const limits = { stranded: lim('stranded', 1), ok: lim('ok', 50) };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('ok');
  });

  it('keeps profiles from an older snapshot that carries neither health field', () => {
    // Absent (undefined) means "unknown", not "broken" — only a positive false disqualifies.
    const profiles = [prof('legacy')];
    const limits = { legacy: lim('legacy', 10) };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('legacy');
  });

  it('excludes a profile with unknown (null / absent) utilisation', () => {
    const profiles = [prof('nulled'), prof('absent')];
    const limits = { nulled: lim('nulled', null) }; // 'absent' has no datapoint at all
    expect(pickResumeCandidate('cur', profiles, limits)).toBeNull();
  });

  it('returns null when nothing qualifies', () => {
    expect(pickResumeCandidate('cur', [prof('cur')], { cur: lim('cur', 10) })).toBeNull();
  });

  it('L11: excludes candidates already claimed this tick, so a second pane picks the next-best', () => {
    const profiles = [prof('cur'), prof('a'), prof('b')];
    const limits = { a: lim('a', 40), b: lim('b', 10), cur: lim('cur', 100) };
    // b is least-utilised; if it was already claimed this pass, fall through to a.
    expect(pickResumeCandidate('cur', profiles, limits, new Set(['b']))).toBe('a');
    // both claimed → nothing eligible remains.
    expect(pickResumeCandidate('cur', profiles, limits, new Set(['a', 'b']))).toBeNull();
    // no exclude set → unchanged behaviour (picks b).
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('b');
  });

  it('ignores a reading older than LIMITS_STALE_MS — a transport error freezes the last numbers', () => {
    const profiles = [prof('cur'), prof('a')];
    const now = 10_000_000;
    // "a" looks free at 12%, but that datapoint predates two whole poll intervals: the poller has been
    // failing, and the real utilisation is unknown. Switching onto it would resume into a dead profile.
    const stale = { a: { ...lim('a', 12), receivedAt: now - LIMITS_STALE_MS - 1 } };
    expect(pickResumeCandidate('cur', profiles, stale, undefined, now)).toBeNull();

    const fresh = { a: { ...lim('a', 12), receivedAt: now - 1_000 } };
    expect(pickResumeCandidate('cur', profiles, fresh, undefined, now)).toBe('a');
  });

  it('weighs a model-scoped weekly cap: an exhausted model week disqualifies a calm-5h profile', () => {
    const profiles = [prof('cur'), prof('calm'), prof('capped')];
    // "capped" looks best on 5h (2%) but its per-model week is at 91% — not a resume destination.
    const limits = { calm: lim('calm', 40), capped: lim('capped', 2, 91), cur: lim('cur', 100) };
    expect(pickResumeCandidate('cur', profiles, limits)).toBe('calm');
    // A scoped cap below 85 only reorders: max(h5, scoped) is the ranking measure.
    const soft = { calm: lim('calm', 40), capped: lim('capped', 2, 30), cur: lim('cur', 100) };
    expect(pickResumeCandidate('cur', profiles, soft)).toBe('capped');
  });

  it('treats a reading with no receivedAt as fresh, so an upgrade does not disable auto-switch', () => {
    const profiles = [prof('cur'), prof('a')];
    expect(pickResumeCandidate('cur', profiles, { a: lim('a', 5) })).toBe('a');
  });
});

describe('isProfileExhausted (#1 display backstop)', () => {
  it('true when a plan window is pegged ≥99% with no extra credits', () => {
    expect(isProfileExhausted(lim('a', 100))).toBe(true);
    expect(isProfileExhausted(lim('a', 99))).toBe(true);
    expect(isProfileExhausted(lim('a', 10, 99))).toBe(true); // model-scoped week drives it
  });
  it('false when utilisation is below the cap, or profile unknown', () => {
    expect(isProfileExhausted(lim('a', 85))).toBe(false);
    expect(isProfileExhausted(undefined)).toBe(false);
  });
  it('respects pay-as-you-go extra credits — not exhausted while credits remain', () => {
    expect(isProfileExhausted({ ...lim('a', 100), extraEnabled: true, extraPct: 40 })).toBe(false);
    // Extra cap itself spent → exhausted again.
    expect(isProfileExhausted({ ...lim('a', 100), extraEnabled: true, extraPct: 100 })).toBe(true);
  });
  it('a transient 429 (null percentages) reads as not-exhausted — no flickering false tint', () => {
    expect(isProfileExhausted({ ...lim('a', null), rateLimited: true })).toBe(false);
  });
});
