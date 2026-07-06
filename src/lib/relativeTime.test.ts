import { describe, it, expect } from 'vitest';
import { parseTsMs } from './relativeTime';

// parseTsMs feeds both the reset-time display and the SessionsTab #21c auto-continue scheduling,
// so the numeric-epoch tolerance is behavior-critical (a NaN silently defeats auto-continue).
describe('parseTsMs', () => {
  it('parses ISO-8601', () => {
    expect(parseTsMs('2026-07-05T10:00:00Z')).toBe(Date.parse('2026-07-05T10:00:00Z'));
  });
  it('parses a bare Unix epoch in SECONDS (10 digits) as ms', () => {
    expect(parseTsMs('1751565600')).toBe(1751565600 * 1000);
  });
  it('parses a bare Unix epoch already in MILLISECONDS (13 digits) as-is', () => {
    expect(parseTsMs('1751565600000')).toBe(1751565600000);
  });
  it('does NOT mis-read a 4-digit year as an epoch (Date.parse wins first)', () => {
    expect(parseTsMs('2026')).toBe(Date.parse('2026'));
  });
  it('returns NaN for null / empty / unparseable', () => {
    expect(parseTsMs(null)).toBeNaN();
    expect(parseTsMs('')).toBeNaN();
    expect(parseTsMs('not-a-date')).toBeNaN();
  });
});
