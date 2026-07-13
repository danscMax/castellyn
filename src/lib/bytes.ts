import { localeTag } from './relativeTime';

// Human-readable byte size. `units` is the comma-joined unit list from i18n (sync.byteUnits),
// passed in so this stays pure and the caller reads it reactively via t().
export function fmtBytes(n: number, units: string): string {
  const u = units.split(',');
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  // Use the locale decimal separator (ru/zh use ','), matching relativeTime.ts's formatting.
  const formatted = v.toLocaleString(localeTag(), { maximumFractionDigits: v < 10 && i > 0 ? 1 : 0 });
  return `${formatted} ${u[i]}`;
}
