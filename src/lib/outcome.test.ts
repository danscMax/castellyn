import { describe, it, expect, beforeAll } from 'vitest';
import { deriveOutcome } from './outcome';
import { locale } from '$lib/i18n';

// Outcome strings are localized; pin the locale so assertions on Russian text are stable.
beforeAll(() => locale.set('ru'));

describe('deriveOutcome', () => {
  it('non-zero exit → error with log action', () => {
    const o = deriveOutcome({ id: 'rtk', name: 'RTK CLI', code: 1, mode: 'check', status: null });
    expect(o.kind).toBe('error');
    expect(o.action?.kind).toBe('log');
  });

  it('forks with needHands → warn + open Forks tab', () => {
    const o = deriveOutcome({
      id: 'forks',
      name: 'Форки',
      code: 0,
      mode: 'check',
      status: { summary: { repos: 6, merged: 1, open: 2, conflict: 3, needHands: 4 } }
    });
    expect(o.kind).toBe('warn');
    expect(o.action).toEqual({ kind: 'tab', label: 'Открыть Форки', target: 'forks' });
    expect(o.detail).toContain('конфликтами');
  });

  it('forks all clean → success', () => {
    const o = deriveOutcome({
      id: 'forks',
      name: 'Форки',
      code: 0,
      mode: 'check',
      status: { summary: { repos: 6, merged: 0, open: 0, conflict: 0, needHands: 0 } }
    });
    expect(o.kind).toBe('success');
    expect(o.action).toBeUndefined();
  });

  it('update component check with changes → info', () => {
    const o = deriveOutcome({
      id: 'speckit',
      name: 'SpecKit',
      code: 0,
      mode: 'check',
      status: { status: 'changes', counts: { changed: 7, failed: 0 } }
    });
    expect(o.kind).toBe('info');
    expect(o.title).toContain('7');
  });

  it('apply success → success', () => {
    const o = deriveOutcome({
      id: 'rtk',
      name: 'RTK CLI',
      code: 0,
      mode: 'apply',
      status: { status: 'ok', counts: { changed: 1, failed: 0 }, durationSec: 2 }
    });
    expect(o.kind).toBe('success');
    expect(o.title).toContain('обновлено');
  });

  it('check up to date → success актуально', () => {
    const o = deriveOutcome({
      id: 'bomfix',
      name: 'BOM-fix',
      code: 0,
      mode: 'check',
      status: { status: 'ok', counts: { changed: 0, failed: 0 } }
    });
    expect(o.kind).toBe('success');
    expect(o.title).toContain('актуально');
  });

  // R1: `held` (exit-0, updates pinned) must not read as "up to date".
  it('held status → info, not success', () => {
    const o = deriveOutcome({
      id: 'cargo',
      name: 'Cargo',
      code: 0,
      mode: 'check',
      status: { status: 'held', counts: { changed: 0, failed: 0 }, durationSec: 2 }
    });
    expect(o.kind).toBe('info');
    expect(o.title).toContain('удержано');
  });

  it('failed count → warn with log action', () => {
    const o = deriveOutcome({
      id: 'plugins',
      name: 'Плагины',
      code: 0,
      mode: 'apply',
      status: { status: 'error', counts: { changed: 0, failed: 2 } }
    });
    expect(o.kind).toBe('warn');
    expect(o.action?.kind).toBe('log');
  });

  // U9: on a non-zero exit, a FRESH envelope summary (written by this run) is shown; a STALE one
  // (a previous run's, left behind if the script died before writing) falls back to generic text.
  it('U9: non-zero exit with a fresh envelope summary → shows the summary', () => {
    const started = Date.parse('2026-07-08T10:00:00Z');
    const o = deriveOutcome({
      id: 'speckit',
      name: 'SpecKit',
      code: 1,
      mode: 'apply',
      status: { status: 'error', summary: 'упал на smoke-тестах', timestamp: '2026-07-08T10:00:05Z' },
      startedAt: started
    });
    expect(o.kind).toBe('error');
    expect(o.detail).toBe('упал на smoke-тестах');
    expect(o.action?.kind).toBe('log');
  });

  it('U9: non-zero exit with a stale envelope → generic detail, not the old summary', () => {
    const started = Date.parse('2026-07-08T10:00:00Z');
    const o = deriveOutcome({
      id: 'speckit',
      name: 'SpecKit',
      code: 1,
      mode: 'apply',
      status: { status: 'error', summary: 'старое резюме', timestamp: '2026-07-08T09:00:00Z' },
      startedAt: started
    });
    expect(o.detail).not.toBe('старое резюме');
  });

  it('U9: non-zero exit without a startedAt reference → generic detail', () => {
    const o = deriveOutcome({
      id: 'speckit',
      name: 'SpecKit',
      code: 1,
      mode: 'apply',
      status: { status: 'error', summary: 'резюме', timestamp: '2026-07-08T10:00:05Z' }
    });
    expect(o.detail).not.toBe('резюме');
  });
});
