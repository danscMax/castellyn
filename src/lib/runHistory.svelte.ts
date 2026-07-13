import { t } from '$lib/i18n';

export type RunRecord = {
  component: string;
  durationSec: number;
  timestamp: number;
  status: string;
};

const KEY = 'cmh-run-history';
const MAX = 200;

function load(): RunRecord[] {
  try {
    const raw = localStorage.getItem(KEY);
    // Guard against a corrupted/non-array value (localStorage key collision, manual edit, etc.)
    const v = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? (v as RunRecord[]) : [];
  } catch {
    return [];
  }
}

function save(records: RunRecord[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(records));
  } catch {
    /* quota exceeded — silently truncate */
  }
}

const initial = load();
export const runHistory = $state<{ items: RunRecord[] }>({
  items: initial
});

export function pushRun(rec: RunRecord) {
  runHistory.items = [rec, ...runHistory.items].slice(0, MAX);
  save(runHistory.items);
}

export function clearRunHistory() {
  runHistory.items = [];
  save([]);
}

export function recentDays(days: number): RunRecord[] {
  const cutoff = Date.now() - days * 86400000;
  return runHistory.items.filter((r) => r.timestamp >= cutoff);
}
