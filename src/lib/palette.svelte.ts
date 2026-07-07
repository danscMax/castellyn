// Shared open-signal for the command palette. The palette's open state lives in +page, but the
// visible trigger button lives in the title bar (rendered by +layout) — this tiny bus bridges the
// two without lifting all of +page's palette state up. Bump the tick to request opening.
export const paletteBus = $state({ tick: 0 });
export function requestPalette() {
  paletteBus.tick++;
}
