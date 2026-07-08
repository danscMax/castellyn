// Shared tab-navigation signal. The active tab lives in +page, but some triggers (the title-bar
// session-status strip, rendered by +layout) sit outside it — this tiny bus bridges them without
// lifting +page's `active` state up. Bump the tick with a target tab to request a switch.
export const navBus = $state({ tab: '', tick: 0 });
export function requestTab(tab: string) {
  navBus.tab = tab;
  navBus.tick++;
}
