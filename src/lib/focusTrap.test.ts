import { describe, it, expect } from 'vitest';
import { trapTab } from './focusTrap';

// Guards ModalShell's focus trap — the one accessibility behaviour in this app that every other
// gate is blind to. Each case below corresponds to a way the trap has to hold, and each fails if
// the matching branch is deleted from trapTab.
describe('trapTab', () => {
  it('wraps forward off the last element and backward off the first', () => {
    expect(trapTab({ count: 3, activeIndex: 2, cardHasFocus: true, shiftKey: false })).toBe('first');
    expect(trapTab({ count: 3, activeIndex: 0, cardHasFocus: true, shiftKey: true })).toBe('last');
  });

  it('lets the browser move focus in the middle of the ring', () => {
    expect(trapTab({ count: 3, activeIndex: 1, cardHasFocus: true, shiftKey: false })).toBeNull();
    expect(trapTab({ count: 3, activeIndex: 1, cardHasFocus: true, shiftKey: true })).toBeNull();
  });

  it('pulls focus back in when it has escaped the card', () => {
    // The card holds focus on open with tabindex=-1 and the backdrop precedes it, so Shift+Tab
    // used to walk out of the dialog — taking the Escape handler on .overlay with it.
    expect(trapTab({ count: 2, activeIndex: -1, cardHasFocus: false, shiftKey: true })).toBe('last');
    expect(trapTab({ count: 2, activeIndex: -1, cardHasFocus: false, shiftKey: false })).toBe('first');
  });

  it('does nothing when the card has no focusable content', () => {
    expect(trapTab({ count: 0, activeIndex: -1, cardHasFocus: true, shiftKey: false })).toBeNull();
    expect(trapTab({ count: 0, activeIndex: -1, cardHasFocus: false, shiftKey: true })).toBeNull();
  });

  it('wraps a single-element ring onto itself in both directions', () => {
    // A one-button dialog must still not let Tab leave: index 0 is both first and last.
    expect(trapTab({ count: 1, activeIndex: 0, cardHasFocus: true, shiftKey: false })).toBe('first');
    expect(trapTab({ count: 1, activeIndex: 0, cardHasFocus: true, shiftKey: true })).toBe('last');
  });
});
