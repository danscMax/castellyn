// Supported UI locales. 'zh' is Simplified Chinese (zh-Hans).
export type Locale = 'ru' | 'en' | 'zh';

// The Russian dictionary is the structural source of truth: its shape defines
// TranslationDict, and en/zh are typed against it so any missing/extra key is a
// compile-time error (svelte-check), backed at runtime by the parity test.
import ru from './locales/ru';
export type TranslationDict = typeof ru;
