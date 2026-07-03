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
  expired: false
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

  it('excludes a profile with unknown (null / absent) utilisation', () => {
    const profiles = [prof('nulled'), prof('absent')];
    const limits = { nulled: lim('nulled', null) }; // 'absent' has no datapoint at all
    expect(pickResumeCandidate('cur', profiles, limits)).toBeNull();
  });

  it('returns null when nothing qualifies', () => {
    expect(pickResumeCandidate('cur', [prof('cur')], { cur: lim('cur', 10) })).toBeNull();
  });
});
