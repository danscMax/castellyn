// The single confirm gate standing between the user and every destructive action. Extracted from
// +page.svelte so the three scars it carries are pinned by tests instead of living as unverified
// scar tissue in a 3000-line component (and so the other hand-rolled copies can collapse into it):
//
//   #120  — when the "confirm destructive actions" setting is OFF the action runs immediately,
//           EXCEPT type-to-confirm actions (restore / reinstall), which always ask.
//   L131  — opening a dialog over an already-open one fires the old one's onCancel first; a
//           promise-based confirm resolves through that callback and would otherwise leak forever.
//   cancel — doConfirm clears onCancel before closing so the cancel path cannot double-fire.
//
// The state object is passed in rather than owned here: the caller creates it with `$state(...)`,
// and mutating through that proxy keeps Svelte's reactivity while this module stays rune-free and
// unit-testable against a plain object.
import { t } from '$lib/i18n';

/**
 * One request for confirmation. An object rather than a positional list: the gate needs seven
 * things, three of them optional booleans/strings, and every call site reads better naming them.
 */
export type ConfirmRequest = {
  title: string;
  message: string;
  /** Button wording. Omitted → the generic "confirm"; destructive flows pass their own verb. */
  confirmLabel?: string;
  /** Concrete items the action will affect — shown so the user sees the scope before agreeing. */
  details?: string[];
  /** Type-to-confirm phrase. Its presence also opts the action OUT of the #120 bypass. */
  requireText?: string | null;
  danger?: boolean;
  onCancel?: () => void;
  action: () => void;
};

export type ConfirmState = {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  action: (() => void) | null;
  onCancel: (() => void) | null;
  details: string[];
  requireText: string | null;
  danger: boolean;
};

/** A closed, empty dialog — the shape the caller wraps in `$state` and the reset target on close. */
export function emptyConfirmState(): ConfirmState {
  return {
    open: false,
    title: '',
    message: '',
    confirmLabel: t('common.confirm'),
    action: null,
    onCancel: null,
    details: [],
    requireText: null,
    danger: false
  };
}

/** Close without confirming: reset the dialog, then fire the pending cancel callback. */
export function closeConfirm(state: ConfirmState): void {
  const cancelled = state.onCancel;
  Object.assign(state, emptyConfirmState());
  cancelled?.();
}

/**
 * Request confirmation for `action`. `enabled` is the global "confirm destructive actions" setting;
 * when it is off the action runs at once unless `opts.requireText` marks it type-to-confirm.
 */
export function askConfirm(state: ConfirmState, enabled: boolean, req: ConfirmRequest): void {
  if (!enabled && !req.requireText) {
    req.action();
    return;
  }
  if (state.open) state.onCancel?.();
  Object.assign(state, {
    open: true,
    title: req.title,
    message: req.message,
    confirmLabel: req.confirmLabel ?? t('common.confirm'),
    action: req.action,
    onCancel: req.onCancel ?? null,
    details: req.details ?? [],
    requireText: req.requireText ?? null,
    danger: req.danger ?? false
  });
}

/** Confirmed: close the dialog (without the cancel path) and run the action. */
export function doConfirm(state: ConfirmState): void {
  const a = state.action;
  state.onCancel = null;
  closeConfirm(state);
  a?.();
}
