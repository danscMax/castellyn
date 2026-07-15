// Maps a finished run (component id + exit code + mode + status envelope) to a
// human, actionable outcome shown as a toast. Strings are localized via t(); the
// mapping logic stays deterministic given the current locale (unit-tested with a fixed locale).

import { t } from '$lib/i18n';
import { countOf, isUnknownStatus } from '$lib/envelope';

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
  /** Epoch-ms when this run began. Used to reject a STALE envelope's summary: a script that died
   *  before writing leaves the previous run's envelope, whose summary would lie about this run. */
  startedAt?: number;
};

// True only when the envelope was (re)written by THIS run — its ISO `timestamp` is at/after the
// run's start (small skew tolerance). Without a start reference, treat it as stale to be safe.
function envelopeFresh(status: any, startedAt?: number): boolean {
  if (!startedAt) return false;
  const ts = Date.parse(status?.timestamp ?? '');
  if (Number.isNaN(ts)) return false;
  return ts >= startedAt - 2000;
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
  const { id, name, code, mode, status, startedAt } = input;

  // Any non-zero exit is an error. U9: if the script wrote a FRESH one-line summary for this run
  // (e.g. "failed on smoke tests"), show it — it's more useful than the generic "open the log".
  // A stale envelope (script died before writing) falls back to the generic text so we never lie.
  if (code !== 0) {
    const summaryStr =
      typeof status?.summary === 'string' && status.summary ? status.summary : undefined;
    return {
      kind: 'error',
      title: t('page.toast_op_error', { name, code }),
      detail:
        summaryStr && envelopeFresh(status, startedAt)
          ? summaryStr
          : t('page.toast_op_error_detail'),
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

  // Update / maintenance components via the unified envelope. A STALE envelope (older than this run's
  // start = the script exited 0 without rewriting it — e.g. Write-StatusJson threw) must NOT drive the
  // verdict with a prior run's status/counts; ignore them and fall through to the exit-0 success. Only
  // gate when we actually have a run-start reference (>0) — no reference keeps the prior behavior.
  const stale = startedAt ? !envelopeFresh(status, startedAt) : false;
  const changed = stale ? 0 : countOf(status, 'changed');
  const failed = stale ? 0 : countOf(status, 'failed');
  const st = stale ? undefined : (status?.status as string | undefined);
  // The script's own one-line summary, when it wrote one — preferred over generic fallback detail.
  const summaryStr =
    typeof status?.summary === 'string' && status.summary ? status.summary : undefined;
  // Same staleness gate as the error path above: a leftover summary from a prior run of this
  // component must not be shown as this run's detail.
  const freshSummary = summaryStr && envelopeFresh(status, startedAt) ? summaryStr : undefined;

  if (st === 'error' || failed > 0) {
    return {
      kind: 'warn',
      title:
        failed > 0
          ? t('page.out_failed_count', { name, failed })
          : t('page.out_failed_problems', { name }),
      detail: freshSummary,
      action: { kind: 'log', label: t('page.toast_open_log') }
    };
  }

  // R1: `held` (updates deliberately pinned/held back) is neither an error nor "up to date" —
  // without this branch it fell through to the success toast and contradicted the card badge.
  if (st === 'held') {
    return { kind: 'info', title: t('page.out_held', { name }), detail: freshSummary ?? durationText(status) };
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
      detail: freshSummary ?? t('page.out_changes_detail')
    };
  }

  // A status this build doesn't know (newer envelope schema, or a writer that bypassed
  // Write-StatusJson) is not success. Say so instead of reporting a green "up to date".
  if (isUnknownStatus(st)) {
    return {
      kind: 'warn',
      title: t('page.out_unknown_status', { name, status: st }),
      detail: freshSummary ?? durationText(status)
    };
  }

  return { kind: 'success', title: t('page.out_uptodate', { name }), detail: durationText(status) };
}
