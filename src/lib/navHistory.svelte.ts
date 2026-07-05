// Tab navigation history — walked like a browser: visible ←/→ buttons in the window chrome
// (WindowTitleBar), mouse buttons 3/4 and Alt+←/→ (handlers in +page). The active-tab state
// lives in +page, which registers `apply` via navBind and reports changes via navTrack.
export const navHistory = $state({ back: [] as string[], fwd: [] as string[] });

let apply: ((tab: string) => void) | null = null;
let current = '';

/** +page registers the current tab and the setter that switches tabs. */
export function navBind(tab: string, fn: (tab: string) => void) {
  current = tab;
  apply = fn;
}

/** Called from +page's tracking $effect on every tab change (including navGo's own — a no-op then). */
export function navTrack(tab: string) {
  if (tab === current) return;
  navHistory.back.push(current);
  if (navHistory.back.length > 50) navHistory.back.shift();
  navHistory.fwd = [];
  current = tab;
}

export function navGo(dir: 'back' | 'fwd') {
  const from = dir === 'back' ? navHistory.back : navHistory.fwd;
  const to = dir === 'back' ? navHistory.fwd : navHistory.back;
  const target = from.pop();
  if (!target || !apply) return;
  to.push(current);
  current = target;
  apply(target);
}
