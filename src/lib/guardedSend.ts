/**
 * Two-phase, honest delivery of text into a live agent TUI (Orca's guarded paste + Enter, §1.3).
 * A blind `write(text + '\r')` into a busy CLI races the agent: if the TUI is mid-work or asking for
 * permission, the text and the Enter interleave with its own redraws and either get eaten or submit
 * half-formed. So: gate on readiness, bracketed-paste the text WITHOUT Enter, settle, re-check, then
 * send Enter only if still idle.
 *
 * Pure: `write` and the busy signal are injected, so no ipc/xterm dependency → unit-testable.
 */
const BEGIN = '\x1b[200~';
const END = '\x1b[201~';

export type GuardedSendResult = 'sent' | 'not-ready' | 'partial';

export type GuardedSendOpts = {
  /** Settle time between the paste and the readiness re-check before Enter. */
  delayMs?: number;
  /** false = paste only, never submit (a hand-picked snippet the user reviews before sending, #57).
   *  The busy-gate still applies; only the auto-Enter phase is skipped. Default true. */
  enter?: boolean;
};

export async function guardedSend(
  write: (data: string) => Promise<void>,
  text: string,
  getBusy: () => boolean,
  opts: GuardedSendOpts = {}
): Promise<GuardedSendResult> {
  if (getBusy()) return 'not-ready'; // busy TUI — don't paste at all
  // Strip any nested paste-end so text can't break out of bracketed-paste mode and inject control seqs.
  const sanitized = text.split(END).join('');
  await write(BEGIN + sanitized + END); // paste only, no Enter yet
  if (opts.enter === false) return 'sent';
  await new Promise((r) => setTimeout(r, opts.delayMs ?? 150));
  if (getBusy()) return 'partial'; // agent got busy between paste and Enter — leave text unsent
  await write('\r');
  return 'sent';
}
