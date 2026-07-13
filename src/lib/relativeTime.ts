import { locale, t } from './i18n';

// BCP-47 tag for the active app locale (for Intl / toLocaleString). Single source so date
// formatting reads the same everywhere (was inlined as a ternary across many components).
export function localeTag(): string {
  return locale.current === 'ru' ? 'ru-RU' : locale.current === 'zh' ? 'zh-CN' : 'en-US';
}

// One RelativeTimeFormat per locale tag — constructing the formatter is the expensive part,
// and a 50-row table calls relTime() once per row, so we cache and rebuild only on tag change.
const rtfCache = new Map<string, Intl.RelativeTimeFormat>();
function relTimeFormat(tag: string): Intl.RelativeTimeFormat {
  let rtf = rtfCache.get(tag);
  if (!rtf) {
    rtf = new Intl.RelativeTimeFormat(tag, { numeric: 'auto' });
    rtfCache.set(tag, rtf);
  }
  return rtf;
}

// Locale-aware "2 hours ago" / "через 3 дня" formatter, shared across tabs so relative
// timestamps read the same everywhere. Returns '' for missing/unparseable input.
export function relTime(ts?: string | null, now = Date.now()): string {
  if (!ts) return '';
  const d = new Date(ts).getTime();
  if (Number.isNaN(d)) return '';
  const rtf = relTimeFormat(localeTag());
  const sec = Math.round((d - now) / 1000);
  const abs = Math.abs(sec);
  if (abs < 60) return rtf.format(sec, 'second');
  // Round into each unit, then check the ROUNDED value against the next unit's boundary
  // (not the raw seconds) so e.g. 3599s falls through to "an hour ago" instead of
  // rounding up to "60 minutes ago".
  const min = Math.round(sec / 60);
  if (Math.abs(min) < 60) return rtf.format(min, 'minute');
  const hr = Math.round(sec / 3600);
  if (Math.abs(hr) < 24) return rtf.format(hr, 'hour');
  const day = Math.round(sec / 86400);
  if (Math.abs(day) < 30) return rtf.format(day, 'day');
  return rtf.format(Math.round(sec / 2592000), 'month');
}

// Absolute timestamp -> localized date/time string, tolerant of null AND unparseable input.
// `new Date('2026-06-08_030000')` yields an Invalid Date WITHOUT throwing, so a bare try/catch
// never fires and "Invalid Date" leaks to the UI — guard with Number.isNaN(getTime()) instead.
// Optional snapshotFallback renders non-ISO formats (e.g. snapshot names). Was duplicated, and
// drifted, across 4 tabs (only BackupTab carried this fix).
export function formatAbsTime(
  ts?: string | null,
  snapshotFallback?: (s: string) => string | null
): string {
  if (!ts) return t('common.dash');
  const ms = parseTsMs(ts);
  // Reject epoch values outside Date's valid range (+/-8.64e15ms): new Date(ms) there
  // yields Invalid Date, which would otherwise leak as the literal string "Invalid Date".
  if (!Number.isNaN(ms) && Math.abs(ms) <= 8.64e15) return new Date(ms).toLocaleString(localeTag());
  return snapshotFallback?.(ts) ?? t('common.dash');
}

// Parse a timestamp string to epoch-ms, tolerating both a Date-parseable string (ISO/year) AND a
// bare Unix epoch (seconds or ms) — the limits API may report resets_at as a NUMBER that the backend
// stringifies (e.g. "1751565600"), which Date.parse rejects. Returns NaN when unparseable. Shared so
// display (formatAbsTime) and scheduling logic (SessionsTab auto-continue #21c) agree on the format.
export function parseTsMs(ts?: string | null): number {
  if (!ts) return NaN;
  const d = Date.parse(ts);
  if (!Number.isNaN(d)) return d;
  // Only after Date.parse fails, so an ISO string or a 4-digit year is never mis-read as an epoch.
  if (/^\d{9,}$/.test(ts)) return Number(ts) * (ts.length <= 10 ? 1000 : 1); // ≤10 digits = seconds
  return NaN;
}
