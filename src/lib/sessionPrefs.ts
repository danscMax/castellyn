// Durable Sessions-personalization sidecar (item 18 / Gap 1).
//
// Root cause fixed: the Sessions prefs lived ONLY in webview localStorage — lost on reinstall, absent
// from the backup snapshot, and outside the Syncthing-synced ~/.claude set. The backend sidecar
// (~/.claude/castellyn/sessions.json) is their durable home. Model: FILE = truth, localStorage = fast
// mirror.
//   - hydrateSessionPrefs() runs ONCE at startup, awaited in +layout.ts BEFORE any component reads
//     localStorage: file -> localStorage. On a fresh file it migrates the other way (localStorage ->
//     file) exactly once, so an existing install's prefs seed the durable home.
//   - After startup a single wrapper around localStorage.setItem/removeItem schedules a debounced
//     flush back to the file whenever a tracked key changes — DRY vs wiring ~20 call sites across four
//     components. A pagehide / visibility-hidden flush covers change-then-close.
//   - On window focus we re-pull file -> localStorage; because tabs currently remount on switch,
//     returning to Sessions then reflects a pref changed on another machine (Syncthing).
//
// cmh-sessions-live is deliberately NOT tracked: which panes are running is machine-local state, not a
// preference to carry across machines.

import { readSessionsPrefs, writeSessionsPrefs } from './ipc';

const KEYS = [
  'cmh-sessions-folders',
  'cmh-sessions-cols',
  'cmh-sessions-rail',
  'cmh-sessions-spaces',
  'cmh-sessions-space-active',
  'cmh-sessions-workspaces',
  'cmh-sessions-defargs',
  'cmh-remote-recent',
  'cmh-monitor-layout',
  'cmh-sessions-fontsize',
  'cmh-sessions-launcher',
  'cmh-sessions-colfr',
  'cmh-sessions-scrollback',
  'cmh-projects-root',
  'cmh-sessions-favorites',
  'cmh-recent-folders',
  'cmh-fav-folders'
] as const;
const KEYSET: ReadonlySet<string> = new Set(KEYS);

// The genuine, un-wrapped localStorage writers — captured before we install the wrapper, so hydration
// writes never re-schedule a flush (a hydrate is not a user edit).
const rawSet = localStorage.setItem.bind(localStorage);

let installed = false;
let flushTimer: ReturnType<typeof setTimeout> | null = null;

function collect(): Record<string, string> {
  const out: Record<string, string> = {};
  for (const k of KEYS) {
    const v = localStorage.getItem(k);
    if (v != null) out[k] = v;
  }
  return out;
}

async function flush(): Promise<void> {
  flushTimer = null;
  try {
    await writeSessionsPrefs(JSON.stringify(collect()));
  } catch {
    /* backend down / write failed — localStorage still holds the value; the next change retries */
  }
}

function scheduleFlush(): void {
  if (flushTimer) clearTimeout(flushTimer);
  flushTimer = setTimeout(() => void flush(), 800);
}

function flushNow(): void {
  if (flushTimer) {
    clearTimeout(flushTimer);
    void flush();
  }
}

function applyFromFile(stored: string): void {
  try {
    const map = JSON.parse(stored) as Record<string, unknown>;
    for (const k of KEYS) {
      const v = map[k];
      if (typeof v === 'string') rawSet(k, v);
    }
  } catch {
    /* corrupt sidecar — keep localStorage as-is; a later flush overwrites it */
  }
}

/**
 * Run ONCE at app startup, awaited BEFORE any component reads localStorage (see +layout.ts).
 * No-op (keeps localStorage) when there is no backend (pure browser / dev / prerender).
 */
export async function hydrateSessionPrefs(): Promise<void> {
  let stored: string | null;
  try {
    stored = await readSessionsPrefs();
  } catch {
    return; // no Tauri backend here — nothing durable to hydrate from
  }
  if (stored) {
    applyFromFile(stored);
  } else {
    // No sidecar yet -> migrate this install's existing prefs into the durable home, exactly once.
    const cur = collect();
    if (Object.keys(cur).length) {
      try {
        await writeSessionsPrefs(JSON.stringify(cur));
      } catch {
        /* retry on the next pref change */
      }
    }
  }
  install();
}

function install(): void {
  if (installed) return;
  installed = true;

  const origSet = localStorage.setItem.bind(localStorage);
  localStorage.setItem = function (key: string, value: string) {
    origSet(key, value);
    if (KEYSET.has(key)) scheduleFlush();
  };
  const origRemove = localStorage.removeItem.bind(localStorage);
  localStorage.removeItem = function (key: string) {
    origRemove(key);
    if (KEYSET.has(key)) scheduleFlush();
  };

  window.addEventListener('pagehide', flushNow);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flushNow();
  });
  window.addEventListener('focus', () => {
    // A local change is still in flight (debounced) — don't pull the file over it, or the pending
    // flush would then persist the clobbered value and lose the edit. Its own flush runs first.
    if (flushTimer) return;
    readSessionsPrefs()
      .then((s) => {
        if (s) applyFromFile(s);
      })
      .catch(() => {});
  });
}
