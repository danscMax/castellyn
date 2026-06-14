// Maps a finished run (component id + exit code + mode + status envelope) to a
// human, actionable outcome shown as a toast. Strings are localized via t(); the
// mapping logic stays deterministic given the current locale (unit-tested with a fixed locale).

import { t } from '$lib/i18n';

export type OutcomeKind = 'success' | 'warn' | 'error' | 'info';
export type OutcomeAction = { kind: 'log' | 'tab'; label: string; target?: string };
export type Outcome = {
  kind: OutcomeKind;
  title: string;
  detail?: string;
  action?: OutcomeAction;
};

export type DeriveInput = {
  id: string;
  name: string;
  code: number;
  mode: 'check' | 'apply';
  status: any;
};

// Read the unified status envelope (counts) with legacy fallbacks, mirroring ComponentCard.
function countOf(s: any, key: 'changed' | 'failed'): number {
  if (s?.counts && typeof s.counts[key] === 'number') return s.counts[key] as number;
  const arr = s?.[key];
  if (Array.isArray(arr)) return arr.length;
  const num = s?.[`plugins_${key}`];
  return typeof num === 'number' ? num : 0;
}

function durationText(s: any): string | undefined {
  if (typeof s?.durationSec === 'number') {
    const total = Math.round(s.durationSec);
    const m = Math.floor(total / 60);
    const sec = total % 60;
    const d = m > 0 ? `${m}:${String(sec).padStart(2, '0')}` : t('page.out_sec', { n: sec });
    return t('page.out_duration', { d });
  }
  return undefined;
}

function forkDetail(s: any): string | undefined {
  if (!s) return undefined;
  const parts: string[] = [];
  if (s.conflict > 0) parts.push(t('page.out_fork_conflicts', { n: s.conflict }));
  if (s.merged > 0) parts.push(t('page.out_fork_merged', { n: s.merged }));
  if (s.open > 0) parts.push(t('page.out_fork_open', { n: s.open }));
  return parts.length ? parts.join(' · ') : undefined;
}

export function deriveOutcome(input: DeriveInput): Outcome {
  const { id, name, code, mode, status } = input;

  // Any non-zero exit is an error — point the user at the log for details.
  if (code !== 0) {
    return {
      kind: 'error',
      title: t('page.toast_op_error', { name, code }),
      detail: t('page.toast_op_error_detail'),
      action: { kind: 'log', label: t('page.toast_open_log') }
    };
  }

  // Forks: summarise from the dedicated summary block and route to the Forks tab.
  if (id === 'forks') {
    const s = status?.summary;
    const need = s?.needHands ?? 0;
    if (need > 0) {
      return {
        kind: 'warn',
        title: t('page.out_forks_need', { need }),
        detail: forkDetail(s),
        action: { kind: 'tab', label: t('page.out_open_forks'), target: 'forks' }
      };
    }
    return {
      kind: 'success',
      title: t('page.out_forks_synced'),
      detail: forkDetail(s),
      action:
        (s?.merged ?? 0) > 0
          ? { kind: 'tab', label: t('page.out_open_forks'), target: 'forks' }
          : undefined
    };
  }

  // Update / maintenance components via the unified envelope.
  const changed = countOf(status, 'changed');
  const failed = countOf(status, 'failed');
  const st = status?.status as string | undefined;

  if (st === 'error' || failed > 0) {
    return {
      kind: 'warn',
      title:
        failed > 0
          ? t('page.out_failed_count', { name, failed })
          : t('page.out_failed_problems', { name }),
      action: { kind: 'log', label: t('page.toast_open_log') }
    };
  }

  if (mode === 'apply') {
    return { kind: 'success', title: t('page.out_applied', { name }), detail: durationText(status) };
  }

  // check mode
  if (st === 'changes' || changed > 0) {
    return {
      kind: 'info',
      title:
        changed > 0
          ? t('page.out_changes_count', { name, changed })
          : t('page.out_changes_any', { name }),
      detail: t('page.out_changes_detail')
    };
  }

  return { kind: 'success', title: t('page.out_uptodate', { name }), detail: durationText(status) };
}
