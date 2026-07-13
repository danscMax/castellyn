/**
 * Internationalization (i18n) for Castellyn.
 *
 * Svelte 5 runes: t() reads $state(_locale) internally, so it is automatically
 * reactive in component templates and $derived contexts — no store subscription,
 * no cleanup. Pattern adapted from the sibling SweetWhisper project, extended to
 * three locales (ru / en / zh-Hans).
 */

import type { Locale, TranslationDict } from './types';
import ru from './locales/ru';
import en from './locales/en';
import zh from './locales/zh';

const translations: Record<Locale, TranslationDict> = { ru, en, zh };

/** localStorage key for the user's explicit UI-language choice. */
export const LANG_STORAGE_KEY = 'cmh-language';

const SUPPORTED: Locale[] = ['ru', 'en', 'zh'];

function isLocale(v: unknown): v is Locale {
  return v === 'ru' || v === 'en' || v === 'zh';
}

/**
 * Pure locale-resolution logic — exported for testability.
 *
 * Order: explicit user choice (`saved`) > OS/browser locale (`navLang`) > 'en'.
 * `ru*` → ru, `zh*` (incl. zh-TW/zh-HK, mapped to Simplified) → zh, `en*` → en,
 * everything else falls through to 'en'.
 */
export function resolveInitialLocale(
  saved: string | null | undefined,
  navLang: string | undefined
): Locale {
  if (isLocale(saved)) return saved;
  const lang = (navLang ?? '').toLowerCase();
  if (lang.startsWith('ru')) return 'ru';
  if (lang.startsWith('zh')) return 'zh';
  if (lang.startsWith('en')) return 'en';
  return 'en';
}

// Tauri's WebView inherits `navigator.language` from the OS (Windows LCID → BCP 47).
function getInitialLocale(): Locale {
  const saved =
    typeof localStorage !== 'undefined' ? localStorage.getItem(LANG_STORAGE_KEY) : null;
  const navLang =
    typeof navigator !== 'undefined'
      ? navigator.language || navigator.languages?.[0] || ''
      : '';
  return resolveInitialLocale(saved, navLang);
}

// Current locale as reactive $state — anything reading it via t()/plural()
// becomes reactive automatically.
let _locale = $state<Locale>(getInitialLocale());

export const locale = {
  /** Current locale (reactive — reads $state). */
  get current(): Locale {
    return _locale;
  },
  /** Set locale and persist the choice. Triggers a reactive update everywhere. */
  set(newLocale: Locale): void {
    if (!isLocale(newLocale)) return;
    _locale = newLocale;
    if (typeof localStorage !== 'undefined') localStorage.setItem(LANG_STORAGE_KEY, newLocale);
    syncHtmlLang(newLocale);
  },
  get supported(): Locale[] {
    return [...SUPPORTED];
  }
};

// Mirror the active locale onto <html lang> so a screen reader picks the right TTS voice and
// pronunciation rules — otherwise zh/ru content is announced by an English voice (zh unintelligible).
// 'zh' → BCP-47 'zh-Hans' (Simplified). Guarded for non-DOM contexts.
function syncHtmlLang(loc: Locale): void {
  if (typeof document !== 'undefined') {
    document.documentElement.lang = loc === 'zh' ? 'zh-Hans' : loc;
  }
}

/**
 * Re-resolve the locale from storage/OS. Locale is already resolved at module
 * load; this exists for symmetry with initTheme() and future side effects.
 */
export function initLocale(): void {
  _locale = getInitialLocale();
  syncHtmlLang(_locale);
}

function traverseKeys(loc: Locale, key: string): unknown {
  const keys = key.split('.');
  let value: unknown = translations[loc];
  for (const k of keys) {
    if (value && typeof value === 'object' && k in value) {
      value = (value as Record<string, unknown>)[k];
    } else {
      return undefined;
    }
  }
  return value;
}

function interpolate(text: string, vars?: Record<string, string | number>): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (_, name: string) => {
    const v = vars[name];
    return v !== undefined ? String(v) : `{${name}}`;
  });
}

/**
 * Translate by dot-separated key with optional `{var}` interpolation.
 * Falls back to English, then to the raw key, when a translation is missing.
 */
export function t(key: string, vars?: Record<string, string | number>): string {
  const loc = _locale; // reads $state — establishes the reactive dependency
  let value = traverseKeys(loc, key);
  if (typeof value !== 'string' && loc !== 'en') value = traverseKeys('en', key);
  if (typeof value !== 'string') {
    if (typeof console !== 'undefined') console.warn(`[i18n] missing key: ${key} (${loc})`);
    return key;
  }
  return interpolate(value, vars);
}

export function hasTranslation(key: string): boolean {
  return typeof traverseKeys(_locale, key) === 'string';
}

export function getLocaleName(loc: Locale): string {
  const names: Record<Locale, string> = {
    ru: 'Русский',
    en: 'English',
    zh: '简体中文'
  };
  return names[loc];
}

// One Intl.PluralRules instance per locale, built once and reused (they're
// stateless and locale-only, so constructing a fresh one per plural() call
// is wasted work).
const prCache = new Map<Locale, Intl.PluralRules>();

/**
 * Locale-aware plural form selection. Picks one/few/many via Intl.PluralRules
 * for the current locale (ru: one|few|many|other, en: one|other, zh: other).
 * `other` and `many` both resolve to the `many` argument.
 */
export function plural(n: number, one: string, few: string, many: string): string {
  let pr = prCache.get(_locale);
  if (!pr) {
    pr = new Intl.PluralRules(_locale);
    prCache.set(_locale, pr);
  }
  const cat = pr.select(n);
  if (cat === 'one') return one;
  if (cat === 'few') return few;
  return many;
}

// Reactive count-noun helpers — read the dictionary, so they re-run on locale change.
const pluralOf =
  (base: string) =>
  (n: number): string =>
    plural(n, t(`common.${base}_one`), t(`common.${base}_few`), t(`common.${base}_many`));

export const pUpdate = pluralOf('update');
export const pConflict = pluralOf('conflict');
export const pCommit = pluralOf('commit');
export const pAction = pluralOf('action');
export const pBranch = pluralOf('branch');
export const pSnapshot = pluralOf('snapshot');
export const pProfile = pluralOf('profile');
export const pRepo = pluralOf('repo');
export const pSkill = pluralOf('skill');
export const pCommand = pluralOf('command');
export const pAgent = pluralOf('agent');
export const pFile = pluralOf('file');
export const pPlugin = pluralOf('plugin');

/** fork-sync mode string -> localized label. */
export function forkMode(m?: string): string {
  if (!m) return '';
  if (m === 'read-only (no fetch)') return t('forks.mode_readonly_nofetch');
  if (m.startsWith('read-only')) return t('forks.mode_readonly');
  if (m.startsWith('dry-run')) return t('forks.mode_dryrun');
  if (m.startsWith('APPLY')) return t('forks.mode_apply');
  return m;
}

/** Branch outcome -> {label, badge class}. */
export function outcomeLabel(o: string | null): { label: string; cls: string } {
  switch (o) {
    case 'merged':
      return { label: t('forks.outcome_merged'), cls: 'badge-ok' };
    case 'clean':
      return { label: t('forks.outcome_clean'), cls: 'badge-info' };
    case 'conflict':
      return { label: t('forks.outcome_conflict'), cls: 'badge-warn' };
    case 'closed-unmerged':
      return { label: t('forks.outcome_closed_unmerged'), cls: 'badge-muted' };
    case 'local-only':
      return { label: t('forks.outcome_local_only'), cls: 'badge-muted' };
    default:
      return { label: o ?? t('common.dash'), cls: 'badge-muted' };
  }
}
