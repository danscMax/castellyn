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

// Trim from the START (oldest lines) so the newer, more valuable tail survives the cap. Optimizer F5:
// no binary search re-encoding O(n log n) — take the last CAP_BYTES UTF-16 units as a fast first cut
// (ANSI is mostly ASCII, so this is at most 2× over-long in code units), then walk forward from the
// cheap cut only if the tail still exceeds the byte cap. One encode per iteration over a shrinking
// tail beats log(n) encodes of the whole slice.
function trimStartToCap(ansi: string): string {
  if (byteLen(ansi) <= CAP_BYTES) return ansi;
  let tail = ansi.slice(-CAP_BYTES); // ≥ the final cut (bytes ≥ code units for any string)
  let over = byteLen(tail) - CAP_BYTES;
  if (over > 0) tail = tail.slice(over); // near-exact: multi-byte chars make this a slight over-trim
  const c = tail.charCodeAt(0);
  if (c >= 0xdc00 && c <= 0xdfff) tail = tail.slice(1); // don't start on an orphaned low surrogate
  return tail;
}

function scrollbackKeys(): string[] {
  const keys: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k && k.startsWith(PREFIX)) keys.push(k);
  }
  return keys;
}

// Tiny timestamp index (paneId → last-save ms) so the TTL sweep never rehydrates the 256 KiB blobs
// just to read their age (optimizer F4). The blob keeps its own "<ms>\n" prefix too — the index is a
// fast path, the prefix the self-contained fallback for a key that somehow missed the index.
const INDEX_KEY = 'cmh-scrollback-index';

function readIndex(): Record<string, number> {
  try {
    const raw = localStorage.getItem(INDEX_KEY);
    const v = raw ? JSON.parse(raw) : null;
    return v && typeof v === 'object' ? (v as Record<string, number>) : {};
  } catch {
    return {};
  }
}

// TTL prune, run opportunistically on every save so localStorage can't grow unbounded when panes are
// never restored (SessionsTab owns the live-pane set and is out of this module's scope, so a pane
// component can't prune by membership at runtime). Reads only the index — O(index) not O(blobs).
// ponytail: TTL, not a global-size LRU — cap × MAX_PANES is a few MB. Add an LRU only if that bites.
function pruneStale(index: Record<string, number>, now = Date.now()): void {
  try {
    for (const key of scrollbackKeys()) {
      const id = key.slice(PREFIX.length);
      let ts = index[id];
      if (!Number.isFinite(ts)) {
        // Not in the index (legacy/anomaly): one blob read as fallback, then it's indexed or gone.
        const raw = localStorage.getItem(key);
        const nl = raw ? raw.indexOf('\n') : -1;
        ts = nl > 0 ? Number(raw!.slice(0, nl)) : NaN;
      }
      if (!Number.isFinite(ts) || now - ts > STALE_MS) {
        localStorage.removeItem(key);
        delete index[id];
      }
    }
  } catch {
    /* ignore */
  }
}

// value = "<ms>\n<ansi>": a fixed timestamp prefix (drives the TTL prune) + the raw ANSI. Not JSON —
// escaping a 256 KiB control-char payload would bloat it and burn CPU on every 5s save.
export function saveScrollback(paneId: string, ansi: string): void {
  try {
    const now = Date.now();
    const index = readIndex();
    pruneStale(index, now);
    index[paneId] = now;
    localStorage.setItem(INDEX_KEY, JSON.stringify(index));
    localStorage.setItem(PREFIX + paneId, `${now}\n${trimStartToCap(ansi)}`);
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
    const index = readIndex();
    for (const key of scrollbackKeys()) {
      if (!live.has(key.slice(PREFIX.length))) {
        localStorage.removeItem(key);
        delete index[key.slice(PREFIX.length)];
      }
    }
    localStorage.setItem(INDEX_KEY, JSON.stringify(index));
  } catch {
    /* ignore */
  }
}
