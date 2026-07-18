// Cold-restore scrollback for Sessions panes (W2). After Castellyn restarts, the PTYs are dead —
// a restored pane replays its previous xterm buffer (serialized ANSI) as INERT scrollback before the
// fresh PTY attaches, so the user sees where they left off. localStorage-only, exactly like
// cmh-sessions-live: which panes ran and what they printed is machine-local state, NOT a preference to
// carry across machines — so these keys are deliberately absent from sessionPrefs' tracked set and
// never reach the sync sidecar.

const PREFIX = 'cmh-scrollback:';
const CAP_BYTES = 262144; // 256 KiB per pane
const STALE_MS = 14 * 24 * 60 * 60 * 1000; // TTL: drop buffers of panes not restored in 2 weeks

const enc = new TextEncoder();
const byteLen = (s: string): number => enc.encode(s).length;

// Trim from the START (oldest lines) so the newer, more valuable tail survives the cap. Binary-search
// the cut on a UTF-16 code-unit boundary (safe for ANSI per contract), then step past an orphaned low
// surrogate so the returned string stays valid.
function trimStartToCap(ansi: string): string {
  if (byteLen(ansi) <= CAP_BYTES) return ansi;
  let lo = 0;
  let hi = ansi.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (byteLen(ansi.slice(mid)) > CAP_BYTES) lo = mid + 1;
    else hi = mid;
  }
  if (lo < ansi.length) {
    const c = ansi.charCodeAt(lo);
    if (c >= 0xdc00 && c <= 0xdfff) lo++; // don't split a surrogate pair
  }
  return ansi.slice(lo);
}

function scrollbackKeys(): string[] {
  const keys: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k && k.startsWith(PREFIX)) keys.push(k);
  }
  return keys;
}

// TTL prune, run opportunistically on every save so localStorage can't grow unbounded when panes are
// never restored (SessionsTab owns the live-pane set and is out of this module's scope, so a pane
// component can't prune by membership at runtime).
// ponytail: TTL, not a global-size LRU — cap × MAX_PANES is a few MB. Add an LRU only if that bites.
function pruneStale(now = Date.now()): void {
  try {
    for (const key of scrollbackKeys()) {
      const raw = localStorage.getItem(key);
      const nl = raw ? raw.indexOf('\n') : -1;
      const ts = nl > 0 ? Number(raw!.slice(0, nl)) : NaN;
      if (!Number.isFinite(ts) || now - ts > STALE_MS) localStorage.removeItem(key);
    }
  } catch {
    /* ignore */
  }
}

// value = "<ms>\n<ansi>": a fixed timestamp prefix (drives the TTL prune) + the raw ANSI. Not JSON —
// escaping a 256 KiB control-char payload would bloat it and burn CPU on every 5s save.
export function saveScrollback(paneId: string, ansi: string): void {
  try {
    pruneStale();
    localStorage.setItem(PREFIX + paneId, `${Date.now()}\n${trimStartToCap(ansi)}`);
  } catch {
    /* quota exceeded / storage disabled — a missing buffer just means no replay, never fatal */
  }
}

// take = read + KEEP (not consume): a webview reload after a restore must still find the buffer.
export function takeScrollback(paneId: string): string | null {
  try {
    const raw = localStorage.getItem(PREFIX + paneId);
    if (raw == null) return null;
    const nl = raw.indexOf('\n');
    return nl < 0 ? raw : raw.slice(nl + 1);
  } catch {
    return null;
  }
}

// Contract prune: drop every buffer whose pane is no longer in the layout. `liveIds` are pane ids
// (without the prefix). This is the tested entry point; the runtime prune is TTL-based (pruneStale)
// because the only caller with the full live set is SessionsTab, which is out of W2's scope.
export function pruneScrollback(liveIds: string[]): void {
  const live = new Set(liveIds);
  try {
    for (const key of scrollbackKeys()) {
      if (!live.has(key.slice(PREFIX.length))) localStorage.removeItem(key);
    }
  } catch {
    /* ignore */
  }
}
