// Helpers for reading the unified status envelope (`{ status, counts:{changed,failed}, … }`).
// Shared by outcome.ts, ComponentCard and UpdatesTab so the envelope-reading rules (and their
// legacy fallbacks) can't drift apart between the toast layer and the cards.

// The `status` values ScriptKit's Write-StatusJson is allowed to emit (its own ValidateSet).
// Anything else is a newer schema or a writer that bypassed the helper: readers must surface it as
// unknown rather than let it fall through to "success", which is what a green badge would imply.
export const KNOWN_STATUSES: ReadonlySet<string> = new Set(['ok', 'changes', 'error', 'held']);

/** True when the envelope carries a status this build does not understand. */
export function isUnknownStatus(status: unknown): status is string {
  return typeof status === 'string' && status.length > 0 && !KNOWN_STATUSES.has(status);
}

// counts.changed / counts.failed, falling back to the legacy `changed[]` array length and the
// older `plugins_changed` / `plugins_failed` numbers for any not-yet-migrated writer.
export function countOf(s: any, key: 'changed' | 'failed'): number {
  if (s?.counts && typeof s.counts[key] === 'number') return s.counts[key] as number;
  const arr = s?.[key];
  if (Array.isArray(arr)) return arr.length;
  const num = s?.[`plugins_${key}`];
  return typeof num === 'number' ? num : 0;
}
