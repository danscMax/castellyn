// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;
// SPA mode: no prerendering — the app is fully client-rendered behind Tauri.
export const prerender = false;

// DEV-only screenshot harness: when launched as `/?shot`, mock the whole Tauri IPC layer with
// public demo fixtures so the UI renders populated tabs in a plain browser (no backend). Guarded
// by `import.meta.env.DEV`, so it is dead-stripped from release builds. Capture: tools/shoot.py.
if (import.meta.env.DEV && typeof window !== 'undefined' && window.location.search.includes('shot')) {
  const [{ mockIPC, mockWindows }, { emit }, { fixtureFor }] = await Promise.all([
    import('@tauri-apps/api/mocks'),
    import('@tauri-apps/api/event'),
    import('$lib/shot/fixtures')
  ]);
  mockWindows('main');
  // run_* commands stream a `run-done` event from the real backend to clear the run lock; the mock
  // returns a code but emits nothing, so synthesize the completion (component = run_forks → 'forks').
  const runComponent: Record<string, string> = { run_forks: 'forks', run_sync: 'sync', run_mcp: 'mcp', run_profiles: 'profiles' };
  mockIPC((cmd, args) => {
    if (cmd in runComponent) setTimeout(() => emit('run-done', { component: runComponent[cmd], code: 0 }), 0);
    return fixtureFor(cmd, args as Record<string, unknown>);
  }, { shouldMockEvents: true });
} else if (typeof window !== 'undefined') {
  // Item 18: pull the durable Sessions-prefs sidecar (~/.claude/castellyn/sessions.json) into
  // localStorage BEFORE any component reads it — so after a reinstall or on another machine the
  // synced prefs are already present at first render. Top-level await → SvelteKit blocks render until
  // it resolves. No-op without a Tauri backend (dev browser): the invoke rejects, localStorage stays.
  const { hydrateSessionPrefs } = await import('$lib/sessionPrefs');
  await hydrateSessionPrefs();
}
