// Barrel re-export of the Svelte 5 runes module. This file exists because
// TypeScript's module resolution doesn't auto-resolve `.svelte.ts` for a bare
// `$lib/i18n` import.
export {
  locale,
  t,
  hasTranslation,
  getLocaleName,
  initLocale,
  resolveInitialLocale,
  LANG_STORAGE_KEY,
  plural,
  pUpdate,
  pConflict,
  pCommit,
  pAction,
  pBranch,
  pSnapshot,
  pProfile,
  forkMode,
  outcomeLabel
} from './index.svelte';
export type { Locale, TranslationDict } from './types';
