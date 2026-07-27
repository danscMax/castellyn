// The Tab-cycling decision of ModalShell's focus trap, pulled out of the component so it can be
// tested without a DOM. The component keeps the DOM half (querying focusables, calling .focus());
// this file owns the part that is pure logic and the part that silently regresses in a refactor.
//
// Why it earns a file: a broken trap is invisible in every existing gate — svelte-check, vitest and
// clippy all stay green while Tab walks out of an open dialog, which on this overlay also takes the
// Escape handler with it (the listener lives on .overlay). There were no component tests at all.

/** Which end of the focusable ring the trap should jump to; null = let the browser move focus. */
export type TabDecision = 'first' | 'last' | null;

export function trapTab(opts: {
  /** How many VISIBLE focusable elements the card currently has. */
  count: number;
  /** Index of document.activeElement among them; -1 when it is not one of them (indexOf convention,
      which is also the case where the card itself holds focus via tabindex=-1). */
  activeIndex: number;
  /** cardEl.contains(document.activeElement) — false once focus has escaped the dialog. */
  cardHasFocus: boolean;
  shiftKey: boolean;
}): TabDecision {
  const { count, activeIndex, cardHasFocus, shiftKey } = opts;
  // Nothing to cycle through: don't fight the browser, or focus would be pinned nowhere.
  if (count <= 0) return null;
  // Focus is outside the card — the card itself (tabindex=-1) holds focus on open and the backdrop
  // sits BEFORE it in the overlay, so without this Shift+Tab leaves the dialog entirely.
  if (!cardHasFocus) return shiftKey ? 'last' : 'first';
  if (shiftKey && activeIndex === 0) return 'last';
  if (!shiftKey && activeIndex === count - 1) return 'first';
  return null;
}
