// Shared UI preferences that need to be reactive across the layout boundary (title bar ↔ settings)
// yet also persist in HubConfig. Initialized from config on mount (WindowTitleBar, always mounted);
// the Settings toggle updates this store live AND persists to config in parallel, so the title-bar
// strip reacts immediately instead of only after a restart. Mirrors the module-store pattern used by
// running.svelte.ts / agentStatus.svelte.ts.

export const uiPrefs = $state({
  // Wave C-5: native session-status strip in the window title bar. Default on.
  showSessionStatusBar: true
});
