// One-click launch-flag presets, shared by the launch dialog and the panel's default-args bar.
// Keyed by tool; the values are exact CLI flags toggled into the args string.
export const ARG_PRESETS: Record<string, string[]> = {
  claude: ['--dangerously-skip-permissions', '--effort max', '--effort high', '--continue', '--resume'],
  opencode: ['--continue'],
  // --yolo = codex's skip-approvals mode (alias of --dangerously-bypass-approvals-and-sandbox).
  codex: ['--yolo', '--full-auto', '--search']
};

// Flags that disable a safety prompt (permission / approval / sandbox). The launch form tints
// their chip as a warning so a risky default (clicker-audit #3) is never silently active.
const RISKY_FLAGS = [
  '--dangerously-skip-permissions',
  '--yolo',
  '--full-auto',
  '--dangerously-bypass-approvals-and-sandbox'
];
export function isRiskyFlag(flag: string): boolean {
  return RISKY_FLAGS.includes(flag.trim());
}

// First-message snippet templates inserted into a pane (no auto-Enter). Common Claude slash
// commands + a couple of nudges. Inserted as-is so the user can review before sending.
export const MSG_SNIPPETS: string[] = ['/clear', '/compact', '/context', 'continue', 'go on'];

// Flags match on TOKEN boundaries, never as a bare substring: '--search' used to "find" itself
// inside a user's '--search-all' and strip the prefix, leaving a mangled '-all' on the command line.
// Multi-token presets ('--effort max') tolerate any run of whitespace between their words.
function flagRe(flag: string): RegExp {
  const esc = flag
    .trim()
    .replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    .replace(/\s+/g, '\\s+');
  return new RegExp(`(?<=^|\\s)${esc}(?=\\s|$)`, 'g');
}

// The custom remainder of an args string once every preset flag is removed. The launch form
// renders presets as chips and shows ONLY this remainder in its text input, so a flag never
// appears twice (chip + literal text) — the duplication the 2026-07 redesign removed.
export function stripFlags(args: string, flags: string[]): string {
  let out = args;
  for (const f of flags) out = out.replace(flagRe(f), ' ');
  return out.replace(/\s+/g, ' ').trim();
}

// Toggle a flag in a space-separated args string (add if absent, strip if present).
export function toggleFlag(args: string, flag: string): string {
  if (flagRe(flag).test(args)) {
    return args.replace(flagRe(flag), ' ').replace(/\s+/g, ' ').trim();
  }
  // --effort presets are mutually exclusive (max vs high): strip any existing
  // --effort <level> before adding the newly selected one, so both callers can't
  // produce a malformed "--effort max --effort high" args string.
  const base = flag.startsWith('--effort ')
    ? args.replace(/--effort\s+\S+/g, '').replace(/\s+/g, ' ').trim()
    : args.trim();
  return `${base} ${flag}`.trim();
}
