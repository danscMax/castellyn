import { describe, it, expect } from 'vitest';
import { decideMenuContinue, pickBindingResetMs, type MenuContinueState } from './menuContinue';

// A neutral base; each test overrides the fields it exercises. now = 1_000_000.
const NOW = 1_000_000;
const base = (o: Partial<MenuContinueState> = {}): MenuContinueState => ({
  sawRateLimit: false,
  menuUp: false,
  menuDismissed: false,
  continued: false,
  busy: false,
  resetMs: null,
  jitterMs: 30_000,
  menuDismissedAtMs: null,
  nowMs: NOW,
  ...o
});

describe('decideMenuContinue', () => {
  it('re-arms when nothing is active', () => {
    expect(decideMenuContinue(base())).toBe('rearm');
  });

  it('presses option 1 on the limit menu', () => {
    expect(decideMenuContinue(base({ sawRateLimit: true, menuUp: true }))).toBe('press1');
  });

  it('presses option 1 on the resume menu (no rate limit)', () => {
    expect(decideMenuContinue(base({ sawRateLimit: false, menuUp: true }))).toBe('press1');
  });

  it('defers every keystroke while the user is at the pane (busy)', () => {
    expect(decideMenuContinue(base({ sawRateLimit: true, menuUp: true, busy: true }))).toBe('wait');
  });

  it('never acts twice — once continued (and menu gone), it re-arms', () => {
    expect(decideMenuContinue(base({ sawRateLimit: true, menuDismissed: true, continued: true }))).toBe('rearm');
  });

  // ── Rate-limit path (waits for the endpoint reset) ───────────────────────────
  it('limit menu: after pressing 1, waits until the reset window', () => {
    const s = base({ sawRateLimit: true, menuUp: true, menuDismissed: true, menuDismissedAtMs: NOW, resetMs: NOW + 60_000 });
    expect(decideMenuContinue(s)).toBe('wait'); // reset is in the future
  });

  it('limit: no known reset yet → waits for the next poll (never a premature continue)', () => {
    const s = base({ sawRateLimit: true, menuDismissed: true, menuDismissedAtMs: NOW, resetMs: null });
    expect(decideMenuContinue(s)).toBe('wait');
  });

  it('limit: continues once past reset + jitter', () => {
    // now (1_000_000) >= reset (960_000) + jitter (30_000) = 990_000 → ready
    const s = base({ sawRateLimit: true, menuDismissed: true, menuDismissedAtMs: NOW - 100, resetMs: NOW - 40_000, jitterMs: 30_000 });
    expect(decideMenuContinue(s)).toBe('continue');
  });

  it('limit BANNER (no menu): continues after reset without any option-1 keypress', () => {
    const s = base({ sawRateLimit: true, menuUp: false, resetMs: NOW - 40_000, jitterMs: 30_000 });
    expect(decideMenuContinue(s)).toBe('continue');
  });

  it('limit BANNER: before reset → waits', () => {
    const s = base({ sawRateLimit: true, menuUp: false, resetMs: NOW + 60_000 });
    expect(decideMenuContinue(s)).toBe('wait');
  });

  // ── THE FIX: sticky sawRateLimit survives the live-flag flicker ───────────────
  it('limit menu whose live flag flickered off (menu cleared, sawRateLimit sticky): still WAITS for reset, not a premature continue', () => {
    // After pressing 1, PTY output flooded → agent-status cleared 'limited' → but sawRateLimit stays
    // true, so this must NOT be mistaken for a resume menu and fire "continue" early.
    const s = base({ sawRateLimit: true, menuUp: false, menuDismissed: true, menuDismissedAtMs: NOW - 8_000, resetMs: NOW + 3_600_000 });
    expect(decideMenuContinue(s)).toBe('wait');
  });

  // ── Resume-menu path (no rate limit → continue promptly) ─────────────────────
  it('resume menu: still up after pressing 1 → waits for it to clear', () => {
    const s = base({ sawRateLimit: false, menuUp: true, menuDismissed: true, menuDismissedAtMs: NOW - 10_000 });
    expect(decideMenuContinue(s)).toBe('wait');
  });

  it('resume menu: cleared but settle not elapsed → waits', () => {
    const s = base({ sawRateLimit: false, menuUp: false, menuDismissed: true, menuDismissedAtMs: NOW - 3_000 });
    expect(decideMenuContinue(s)).toBe('wait'); // 3s < 6s settle
  });

  it('resume menu: cleared + settle elapsed → continues (no reset wait)', () => {
    const s = base({ sawRateLimit: false, menuUp: false, menuDismissed: true, menuDismissedAtMs: NOW - 8_000 });
    expect(decideMenuContinue(s)).toBe('continue');
  });

  // ── Active-window / re-arm ────────────────────────────────────────────────────
  it('mid-flow (pressed 1, not continued) stays active even with the live flag cleared', () => {
    const s = base({ sawRateLimit: false, menuUp: false, menuDismissed: true, menuDismissedAtMs: NOW - 3_000 });
    expect(decideMenuContinue(s)).not.toBe('rearm');
  });

  it('a rate-limit pane stays active until its (hours-away) reset — no early give-up', () => {
    const s = base({ sawRateLimit: true, menuUp: false, menuDismissed: true, menuDismissedAtMs: NOW - 600_000, resetMs: NOW + 3_600_000 });
    expect(decideMenuContinue(s)).toBe('wait'); // 10 min in, reset 1h away → still waiting, not re-armed
  });
});

describe('pickBindingResetMs', () => {
  const H5 = NOW + 1_000; // near-future 5h reset
  const D7 = NOW + 500_000; // farther weekly reset

  it('picks the LATER reset among exhausted windows (V-18: capped on 7d, 5h resets first)', () => {
    // Both exhausted → the binding reset is the one that clears LAST (d7), not the sooner 5h.
    expect(
      pickBindingResetMs([{ util: 100, resetMs: H5 }, { util: 99, resetMs: D7 }], H5)
    ).toBe(D7);
  });

  it('uses only exhausted (≥99%) windows — a healthy window never binds', () => {
    expect(
      pickBindingResetMs([{ util: 100, resetMs: H5 }, { util: 40, resetMs: D7 }], H5)
    ).toBe(H5); // d7 at 40% is ignored even though it resets later
  });

  it('falls back to the 5h reset when the endpoint has not marked any window exhausted (poll lag)', () => {
    expect(
      pickBindingResetMs([{ util: 50, resetMs: H5 }, { util: 10, resetMs: D7 }], H5)
    ).toBe(H5);
  });

  it('skips an exhausted window whose reset is unknown (NaN), using the next exhausted one', () => {
    expect(
      pickBindingResetMs([{ util: 100, resetMs: NaN }, { util: 100, resetMs: D7 }], NaN)
    ).toBe(D7);
  });

  it('returns null when nothing is exhausted and the 5h fallback is unknown', () => {
    expect(pickBindingResetMs([{ util: 10, resetMs: NaN }], NaN)).toBeNull();
  });
});
