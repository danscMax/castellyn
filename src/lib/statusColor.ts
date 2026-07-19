// Single shared source for status/semantic colours across the UI. Before this, several
// components hardcoded `text-emerald-400 / text-amber-400 / text-red-400` for status text with
// NO light-theme gating — on the near-white light card those measure ~1.7-2.9:1 (WCAG normal
// text needs >=4.5:1). The fix mirrors the existing `.badge-*` canon: a tiny `.status-*` utility
// class (defined in app.css) carries the dark `*-400` colour PLUS a `.light .status-* { *-700 }`
// override, so both themes pass. Dark theme keeps the exact `*-400` values it had before.

export type StatusLevel = 'ok' | 'warn' | 'bad' | 'muted' | 'info';

// Status level -> theme-aware text-colour utility class. The class itself resolves to
// emerald/amber/red/sky/slate-400 in dark and -700 in light (see `.status-*` in app.css).
const STATUS_TEXT: Record<StatusLevel, string> = {
  ok: 'status-ok',
  warn: 'status-warn',
  bad: 'status-bad',
  muted: 'status-muted',
  info: 'status-info'
};

/** Theme-aware status TEXT colour class (>=4.5:1 in both themes). */
export function statusTextClass(level: StatusLevel): string {
  return STATUS_TEXT[level];
}

// Status level -> a CSS-variable colour for FILLS (dots, bars). These use the saturated
// `--sw-status-*` tokens, which read fine on both light and dark surfaces, so no override is
// needed — unlike text, a small filled dot doesn't have a 4.5:1 requirement.
const STATUS_FILL_VAR: Record<'up' | 'degraded' | 'down' | 'off', string> = {
  up: 'var(--sw-status-up)',
  degraded: 'var(--sw-status-degraded)',
  down: 'var(--sw-status-down)',
  off: 'var(--sw-status-off)'
};

/** Status fill colour (CSS `var(--sw-status-*)`) for dots/indicators. */
export function statusFillVar(level: 'up' | 'degraded' | 'down' | 'off'): string {
  return STATUS_FILL_VAR[level];
}

// --- Profile name -> swatch hex -------------------------------------------------------------
// PowerShell console colour names (the values profiles are stored with) mapped to a display hex.
// Single source shared by the profile colour picker (ProfileEditDialog) and the profile list dot
// (ProfilesTab), which previously each kept their own copy that had drifted out of sync.
export const PROFILE_SWATCH: Record<string, string> = {
  Cyan: '#22d3ee',
  Green: '#34d399',
  Yellow: '#fbbf24',
  Magenta: '#e879f9',
  Blue: '#60a5fa',
  Red: '#f87171',
  White: '#e5e7eb',
  Gray: '#9ca3af',
  DarkCyan: '#0e7490',
  DarkGreen: '#15803d',
  DarkYellow: '#a16207',
  DarkMagenta: '#a21caf',
  DarkBlue: '#1d4ed8',
  DarkRed: '#b91c1c'
};

// The colour names offered in the picker, in display order — derived from the swatch map so a
// colour can't be pickable-but-grey (or vice versa). Object literal insertion order IS the order.
export const PROFILE_COLORS: string[] = Object.keys(PROFILE_SWATCH);

const PROFILE_DOT_FALLBACK = '#94a3b8';
/** Hex for a profile's stored colour name; a neutral slate for unknown/legacy names. */
export function profileDotColor(name: string | null | undefined): string {
  return (name && PROFILE_SWATCH[name]) || PROFILE_DOT_FALLBACK;
}

// --- Categorical chart palette --------------------------------------------------------------
// Cycled-by-index colours for stacked bars / legends / sparklines (Analytics). These are
// categorical series colours, not status levels, so they stay as a fixed hex palette.
export const CHART_SERIES_COLORS: string[] = [
  '#3b82f6',
  '#10b981',
  '#f59e0b',
  '#ef4444',
  '#8b5cf6',
  '#ec4899',
  '#14b8a6',
  '#f97316'
];

/** N-th categorical series colour, cycling the palette. */
export function chartSeriesColor(i: number): string {
  return CHART_SERIES_COLORS[i % CHART_SERIES_COLORS.length];
}
