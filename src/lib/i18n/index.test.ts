import { describe, it, expect, afterAll } from 'vitest';
import ru from './locales/ru';
import en from './locales/en';
import zh from './locales/zh';
import { t, locale, resolveInitialLocale, plural, pConflict, pUpdate, forkMode, outcomeLabel } from './index.svelte';

function leafKeys(obj: unknown, prefix = ''): string[] {
  if (typeof obj !== 'object' || obj === null) return [prefix];
  return Object.entries(obj as Record<string, unknown>).flatMap(([k, v]) =>
    leafKeys(v, prefix ? `${prefix}.${k}` : k)
  );
}

// Restore the resolved locale after mutating it in tests.
const original = locale.current;
afterAll(() => locale.set(original));

describe('locale parity', () => {
  const ruKeys = new Set(leafKeys(ru));
  const enKeys = new Set(leafKeys(en));
  const zhKeys = new Set(leafKeys(zh));

  it('en has exactly the same keys as ru', () => {
    expect([...enKeys].filter((k) => !ruKeys.has(k))).toEqual([]);
    expect([...ruKeys].filter((k) => !enKeys.has(k))).toEqual([]);
  });

  it('zh has exactly the same keys as ru', () => {
    expect([...zhKeys].filter((k) => !ruKeys.has(k))).toEqual([]);
    expect([...ruKeys].filter((k) => !zhKeys.has(k))).toEqual([]);
  });
});

describe('resolveInitialLocale', () => {
  it('honours an explicit saved choice', () => {
    expect(resolveInitialLocale('zh', 'en-US')).toBe('zh');
    expect(resolveInitialLocale('ru', 'en-US')).toBe('ru');
  });
  it('detects from the OS/browser locale', () => {
    expect(resolveInitialLocale(null, 'ru-RU')).toBe('ru');
    expect(resolveInitialLocale(null, 'zh-Hans-CN')).toBe('zh');
    expect(resolveInitialLocale(null, 'zh-TW')).toBe('zh');
    expect(resolveInitialLocale(null, 'en-GB')).toBe('en');
  });
  it('falls back to English for unsupported locales', () => {
    expect(resolveInitialLocale(null, 'de-DE')).toBe('en');
    expect(resolveInitialLocale(undefined, '')).toBe('en');
  });
});

describe('t()', () => {
  it('returns the active-locale string', () => {
    locale.set('en');
    expect(t('common.save')).toBe('Save');
    locale.set('zh');
    expect(t('common.save')).toBe('保存');
    locale.set('ru');
    expect(t('common.save')).toBe('Сохранить');
  });
  it('interpolates {vars}', () => {
    locale.set('en');
    expect(t('settings.currentlyUsed', { path: 'E:/X' })).toBe('Currently used: E:/X');
  });
  it('falls back to the key for a missing path', () => {
    expect(t('nope.not_here')).toBe('nope.not_here');
  });
});

describe('pluralization', () => {
  it('selects Russian one/few/many', () => {
    locale.set('ru');
    expect(plural(1, 'a', 'b', 'c')).toBe('a');
    expect(plural(2, 'a', 'b', 'c')).toBe('b');
    expect(plural(5, 'a', 'b', 'c')).toBe('c');
    expect(plural(11, 'a', 'b', 'c')).toBe('c');
    expect(plural(21, 'a', 'b', 'c')).toBe('a');
    expect(`1 ${pConflict(1)}`).toBe('1 конфликт');
    expect(`5 ${pUpdate(5)}`).toBe('5 обновлений');
  });
  it('selects English one/other', () => {
    locale.set('en');
    expect(plural(1, 'a', 'b', 'c')).toBe('a');
    expect(plural(2, 'a', 'b', 'c')).toBe('c');
    expect(`1 ${pConflict(1)}`).toBe('1 conflict');
    expect(`2 ${pConflict(2)}`).toBe('2 conflicts');
  });
  it('Chinese has a single form', () => {
    locale.set('zh');
    expect(pUpdate(1)).toBe(pUpdate(5));
  });
});

describe('helpers', () => {
  it('forkMode maps known modes (ru)', () => {
    locale.set('ru');
    expect(forkMode('read-only unattended')).toBe('только чтение');
    expect(forkMode('dry-run: показ плана')).toBe('предпросмотр плана');
    expect(forkMode(undefined)).toBe('');
  });
  it('outcomeLabel returns label + class', () => {
    locale.set('en');
    expect(outcomeLabel('merged').cls).toBe('badge-ok');
    expect(outcomeLabel('merged').label.length).toBeGreaterThan(0);
    expect(outcomeLabel('conflict').cls).toBe('badge-warn');
    expect(outcomeLabel(null).label).toBe('—');
  });
});
