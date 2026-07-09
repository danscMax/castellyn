import { describe, it, expect } from 'vitest';
import { pickResumeCandidate } from './limitSwitch';
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
const lim = (profile: string, h5: number | null): LimitsStatusEvent => ({
  profile,
  h5,
  d7: null,
  h5Reset: null,
  d7Reset: null,
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
});
