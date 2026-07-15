// Pin a popover (dropdown menu / select panel / folder picker) to its anchor with
// `position: fixed`, so it escapes any overflow-clipping ancestor — scrollable tables
// (`.dt`/`.dt-scroll` clip on both axes) and modal bodies (`overflow-y: auto`). Re-pins on
// scroll/resize. The popover stays a DOM descendant of its anchor's container, so each
// component's existing "click outside → close" containment check keeps working.
//
// Assumes no ancestor establishes a fixed-positioning containing block (a persistent
// transform/filter/contain). The only animated transform in the app — ModalShell's enter —
// is non-persistent (no animation-fill-mode), so this holds once the 0.18s animation ends.
type AnchoredParams = {
  anchor: HTMLElement;
  align?: 'left' | 'right'; // which edge of the anchor the popover lines up with (default left)
  matchWidth?: boolean; // force the popover width to the anchor's (for full-width selects)
  // Called on a pointer press outside BOTH the popover and its anchor — the one place the
  // "click outside → close" check lives (Select/FolderField/DropdownMenu used to each hand-roll it).
  onOutside?: () => void;
};

export function anchored(node: HTMLElement, params: AnchoredParams) {
  const MARGIN = 8;
  let p = params;

  function place() {
    if (typeof window === 'undefined' || !p.anchor) return;
    const a = p.anchor.getBoundingClientRect();
    node.style.position = 'fixed';
    node.style.right = '';
    if (p.matchWidth) node.style.width = `${a.width}px`;
    else node.style.width = ''; // clear a width forced by a previous matchWidth:true (params can flip)

    const w = node.offsetWidth || 200;
    const h = node.offsetHeight || 0;

    // Intended VIEWPORT coordinates. Vertical: open below; flip above only if below would overflow
    // the viewport bottom and there's room above (fixes the clipped last-row menu).
    let top = a.bottom + 4;
    if (top + h > window.innerHeight - MARGIN && a.top - h - 4 >= MARGIN) top = a.top - h - 4;
    top = Math.max(MARGIN, top);
    // Horizontal: align to the chosen edge, then clamp into the viewport.
    let left = p.matchWidth || p.align !== 'right' ? a.left : a.right - w;
    if (left + w > window.innerWidth - MARGIN) left = window.innerWidth - MARGIN - w;
    left = Math.max(MARGIN, left);

    // Apply, then SELF-CORRECT: a `transform`/`filter`/`backdrop-filter`/`contain` ancestor makes this
    // fixed element's containing block ≠ the viewport, so the px we set land somewhere else. Measure the
    // actual rect and shift by the delta so the popover ends up at the intended viewport position.
    // (`.sw-card` uses backdrop-filter, so every in-card popover hits this.)
    node.style.top = `${top}px`;
    node.style.left = `${left}px`;
    const got = node.getBoundingClientRect();
    const dx = left - got.left;
    const dy = top - got.top;
    if (dx || dy) {
      node.style.top = `${top + dy}px`;
      node.style.left = `${left + dx}px`;
    }
  }

  place();
  // capture=true so scrolling an inner overflow container (e.g. the table) re-pins too.
  const onScroll = () => place();
  window.addEventListener('scroll', onScroll, true);
  window.addEventListener('resize', onScroll);
  // Outside-press dismissal: a press inside the popover OR its anchor (the trigger lives there, and
  // toggles open itself) is NOT outside. capture=true so it fires before inner handlers.
  const onOutside = (e: PointerEvent) => {
    if (!p.onOutside) return;
    const t = e.target as Node;
    if (!node.contains(t) && !p.anchor.contains(t)) p.onOutside();
  };
  window.addEventListener('pointerdown', onOutside, true);

  return {
    update(next: AnchoredParams) {
      p = next;
      place();
    },
    destroy() {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', onScroll);
      window.removeEventListener('pointerdown', onOutside, true);
    }
  };
}
