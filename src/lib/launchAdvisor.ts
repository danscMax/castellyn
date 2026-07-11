import type { ProfileInfo } from './ipc';
import {
  evaluateProfiles,
  type LimitsMap,
  type EligibleProfile,
  type RejectedProfile
} from './limitSwitch';

/** Coarse class of the work about to start — maps to a default Claude reasoning effort. */
export type LaunchTaskClass =
  | 'inspect'
  | 'review'
  | 'fix'
  | 'feature'
  | 'debug'
  | 'architecture'
  | 'critical';

export type ClaudeEffort = 'low' | 'medium' | 'high' | 'max';

/** Default effort per task class. Cheap read work stays low; open-ended debugging/architecture goes
 *  high; only explicitly critical work spends `max`. The launcher lets the user override this. */
export function effortForTaskClass(taskClass: LaunchTaskClass): ClaudeEffort {
  switch (taskClass) {
    case 'inspect':
    case 'review':
      return 'low';
    case 'fix':
    case 'feature':
      return 'medium';
    case 'debug':
    case 'architecture':
      return 'high';
    case 'critical':
      return 'max';
  }
}

export type LaunchRecommendation = {
  profile: string;
  effort: ClaudeEffort;
  /** Binding utilisation (max of 5h and model-scoped week) of the recommended profile, 0..100. */
  util: number;
};

export type LaunchAdvice = {
  /** Best eligible profile + a default effort, or null when nothing qualifies (see `rejected`). */
  recommendation: LaunchRecommendation | null;
  /** Every eligible profile, least-utilised first — evidence for the launcher UI. */
  eligible: EligibleProfile[];
  /** Every non-candidate with a structured reason — so the UI can explain an empty recommendation. */
  rejected: RejectedProfile[];
};

/**
 * Recommend a quota-aware `profile + effort` for a NEW Claude session, purely from already-polled
 * usage: it launches nothing, switches nothing, and issues no new poll. Reuses `evaluateProfiles` —
 * the exact eligibility + ranking behind the resume auto-switch — so the recommendation and the
 * auto-switch never disagree about which profile is "free enough".
 *
 * `reserved` holds profiles already claimed by same-tick launches, so two launches in the same
 * moment don't both get pointed at the one free profile. The recommended effort follows the task
 * class; the launcher keeps profile, effort and model fully overridable before anything spawns.
 */
export function launchAdvisor(
  profiles: ProfileInfo[],
  limits: LimitsMap,
  taskClass: LaunchTaskClass,
  reserved: ReadonlySet<string> = new Set(),
  now: number = Date.now()
): LaunchAdvice {
  const { eligible, rejected } = evaluateProfiles(profiles, limits, reserved, now);
  const best = eligible[0] ?? null;
  return {
    recommendation: best
      ? { profile: best.name, effort: effortForTaskClass(taskClass), util: best.util }
      : null,
    eligible,
    rejected
  };
}
