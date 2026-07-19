import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { saveScrollback, takeScrollback, pruneScrollback } from './scrollbackStore';

// vitest runs in node (no jsdom here), so provide a minimal in-memory localStorage before each test.
class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  key(i: number): string | null {
    return [...this.m.keys()][i] ?? null;
  }
  getItem(k: string): string | null {
    return this.m.has(k) ? this.m.get(k)! : null;
  }
  setItem(k: string, v: string): void {
    this.m.set(k, String(v));
  }
  removeItem(k: string): void {
    this.m.delete(k);
  }
  clear(): void {
    this.m.clear();
  }
}

beforeEach(() => {
  (globalThis as unknown as { localStorage: Storage }).localStorage = new MemStorage() as unknown as Storage;
});
afterEach(() => {
  vi.useRealTimers();
});

const CAP = 262144;
const byteLen = (s: string) => new TextEncoder().encode(s).length;

describe('scrollbackStore', () => {
  it('round-trips save/take and keeps the buffer (take != consume)', () => {
    saveScrollback('claude|work||E:/proj|', 'hello\x1b[31m world\x1b[0m');
    expect(takeScrollback('claude|work||E:/proj|')).toBe('hello\x1b[31m world\x1b[0m');
    // A second read still returns it — a webview reload must not lose the buffer.
    expect(takeScrollback('claude|work||E:/proj|')).toBe('hello\x1b[31m world\x1b[0m');
  });

  it('returns null for an unknown pane', () => {
    expect(takeScrollback('nope')).toBeNull();
  });

  it('caps at 256 KiB by trimming the START, keeping the newer tail', () => {
    const head = 'A'.repeat(200000); // older
    const tail = 'B'.repeat(200000); // newer
    saveScrollback('p', head + tail);
    const got = takeScrollback('p')!;
    expect(byteLen(got)).toBeLessThanOrEqual(CAP);
    // The result is a suffix of the original — the oldest bytes were dropped, the tail survived.
    expect(got.endsWith('B'.repeat(1000))).toBe(true);
    expect((head + tail).endsWith(got)).toBe(true);
    // It kept as much of the tail as the cap allows.
    expect(byteLen(got)).toBeGreaterThan(CAP - 4);
  });

  it('never leaves a lone surrogate at the cut when trimming multibyte content', () => {
    // Emoji are surrogate pairs (2 UTF-16 units, 4 UTF-8 bytes) — a naive byte cut could split one.
    const big = '😀'.repeat(80000); // ~320 KB, > cap
    saveScrollback('emoji', big);
    const got = takeScrollback('emoji')!;
    expect(byteLen(got)).toBeLessThanOrEqual(CAP);
    const first = got.charCodeAt(0);
    expect(first >= 0xdc00 && first <= 0xdfff).toBe(false); // not a lone low surrogate
    expect([...got].every((ch) => ch === '😀')).toBe(true); // every code point intact
  });

  // The cap used to subtract a BYTE overage from a UTF-16 index, so any buffer averaging >= 2 B/char
  // was trimmed to nothing. These panes must keep a full cap's worth, not an empty string.
  it.each([
    ['cjk', '中'.repeat(300000)],
    ['cyrillic', 'привет '.repeat(50000)],
    ['emoji', '😀'.repeat(80000)]
  ])('keeps a full cap of %s content instead of over-trimming to empty', (_name, big) => {
    saveScrollback('multibyte', big);
    const got = takeScrollback('multibyte')!;
    expect(byteLen(got)).toBeLessThanOrEqual(CAP);
    expect(byteLen(got)).toBeGreaterThan(CAP - 8); // a whole cap, minus at most one split char
    expect(big.endsWith(got)).toBe(true); // still a suffix: only the OLDEST content was dropped
  });

  it('prunes foreign keys (panes not in the live layout), keeps live ones', () => {
    saveScrollback('live-a', 'a');
    saveScrollback('live-b', 'b');
    saveScrollback('dead-c', 'c');
    pruneScrollback(['live-a', 'live-b']);
    expect(takeScrollback('live-a')).toBe('a');
    expect(takeScrollback('live-b')).toBe('b');
    expect(takeScrollback('dead-c')).toBeNull();
  });

  it('does not touch non-scrollback localStorage keys when pruning', () => {
    localStorage.setItem('cmh-sessions-live', '[...]');
    saveScrollback('dead', 'x');
    pruneScrollback([]); // nothing live
    expect(takeScrollback('dead')).toBeNull();
    expect(localStorage.getItem('cmh-sessions-live')).toBe('[...]');
  });

  it('TTL-prunes buffers older than 2 weeks on the next save', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'));
    saveScrollback('old', 'stale');
    // 15 days later a save for another pane sweeps the stale one.
    vi.setSystemTime(new Date('2026-01-16T00:00:00Z'));
    saveScrollback('fresh', 'new');
    expect(takeScrollback('old')).toBeNull();
    expect(takeScrollback('fresh')).toBe('new');
  });
});
