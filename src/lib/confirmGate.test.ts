import { describe, it, expect, vi, beforeAll } from 'vitest';
import { locale } from '$lib/i18n';
import { askConfirm, closeConfirm, doConfirm, emptyConfirmState } from './confirmGate';

beforeAll(() => locale.set('ru'));

describe('askConfirm — the #120 bypass', () => {
  it('opens the dialog and does NOT run the action while the setting is on', () => {
    const s = emptyConfirmState();
    const action = vi.fn();
    askConfirm(s, true, 'Удалить?', 'навсегда', 'Удалить', action, { danger: true });
    expect(action).not.toHaveBeenCalled();
    expect(s.open).toBe(true);
    expect(s.title).toBe('Удалить?');
    expect(s.confirmLabel).toBe('Удалить');
    expect(s.danger).toBe(true);
  });

  it('runs immediately without opening when the setting is off', () => {
    const s = emptyConfirmState();
    const action = vi.fn();
    askConfirm(s, false, 't', 'm', 'ok', action, { danger: true });
    expect(action).toHaveBeenCalledTimes(1);
    expect(s.open).toBe(false);
  });

  // The exemption that makes the bypass survivable: restore/reinstall must ask even when the
  // setting is off. Drop the `&& !opts.requireText` term and this test goes red.
  it('still asks for a type-to-confirm action when the setting is off', () => {
    const s = emptyConfirmState();
    const action = vi.fn();
    askConfirm(s, false, 't', 'm', 'ok', action, { requireText: '2026-07-19' });
    expect(action).not.toHaveBeenCalled();
    expect(s.open).toBe(true);
    expect(s.requireText).toBe('2026-07-19');
  });

  it('carries details through unchanged and defaults the optional fields', () => {
    const s = emptyConfirmState();
    askConfirm(s, true, 't', 'm', 'ok', () => {}, { details: ['a', 'b'] });
    expect(s.details).toEqual(['a', 'b']);
    expect(s.requireText).toBeNull();
    expect(s.danger).toBe(false);
    expect(s.onCancel).toBeNull();
  });
});

describe('askConfirm — L131 replacing an open dialog', () => {
  // A promise-based confirm resolves through onCancel. Silently overwriting an open dialog would
  // leave that promise pending forever. Remove the `if (state.open) state.onCancel?.()` guard and
  // the first assertion goes red.
  it('fires the displaced dialog’s onCancel exactly once', () => {
    const s = emptyConfirmState();
    const onCancel = vi.fn();
    askConfirm(s, true, 'first', 'm', 'ok', () => {}, { onCancel });
    askConfirm(s, true, 'second', 'm', 'ok', () => {});
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(s.title).toBe('second');
    expect(s.onCancel).toBeNull(); // the replacement carried no cancel callback of its own
  });

  it('does not fire a cancel when nothing was open', () => {
    const s = emptyConfirmState();
    const onCancel = vi.fn();
    askConfirm(s, true, 'only', 'm', 'ok', () => {}, { onCancel });
    expect(onCancel).not.toHaveBeenCalled();
  });
});

describe('doConfirm / closeConfirm', () => {
  it('runs the action and resets the dialog', () => {
    const s = emptyConfirmState();
    const action = vi.fn();
    askConfirm(s, true, 't', 'm', 'ok', action, { details: ['x'], danger: true });
    doConfirm(s);
    expect(action).toHaveBeenCalledTimes(1);
    expect(s.open).toBe(false);
    expect(s.action).toBeNull();
    expect(s.details).toEqual([]);
    expect(s.danger).toBe(false);
  });

  // doConfirm nulls onCancel before closing. Without that, confirming ALSO runs the cancel path —
  // for a promise-based confirm that means resolving both ways.
  it('does not fire onCancel when the action is confirmed', () => {
    const s = emptyConfirmState();
    const onCancel = vi.fn();
    askConfirm(s, true, 't', 'm', 'ok', () => {}, { onCancel });
    doConfirm(s);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('fires onCancel exactly once when dismissed', () => {
    const s = emptyConfirmState();
    const onCancel = vi.fn();
    const action = vi.fn();
    askConfirm(s, true, 't', 'm', 'ok', action, { onCancel });
    closeConfirm(s);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(action).not.toHaveBeenCalled();
    expect(s.open).toBe(false);
    // A second dismissal (a stray onCancel from the dialog) must not re-fire it.
    closeConfirm(s);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('is a no-op on an already-empty state', () => {
    const s = emptyConfirmState();
    doConfirm(s);
    expect(s.open).toBe(false);
  });
});
