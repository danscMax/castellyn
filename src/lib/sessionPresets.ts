// One-click launch-flag presets, shared by the launch dialog and the panel's default-args bar.
// Keyed by tool; the values are exact CLI flags toggled into the args string.
export const ARG_PRESETS: Record<string, string[]> = {
  claude: ['--dangerously-skip-permissions', '--effort max', '--effort high', '--continue', '--resume'],
  opencode: ['--continue'],
  // --yolo = codex's skip-approvals mode (alias of --dangerously-bypass-approvals-and-sandbox).
  codex: ['--yolo', '--full-auto', '--search']
};

// First-message snippet templates inserted into a pane (no auto-Enter). Common Claude slash
// commands + a couple of nudges. Inserted as-is so the user can review before sending.
export const MSG_SNIPPETS: string[] = ['/clear', '/compact', '/context', 'continue', 'go on'];

// Toggle a flag in a space-separated args string (add if absent, strip if present).
export function toggleFlag(args: string, flag: string): string {
  if (args.includes(flag)) {
    return args.replace(flag, '').replace(/\s+/g, ' ').trim();
  }
  // --effort presets are mutually exclusive (max vs high): strip any existing
  // --effort <level> before adding the newly selected one, so both callers can't
  // produce a malformed "--effort max --effort high" args string.
  const base = flag.startsWith('--effort ')
    ? args.replace(/--effort\s+\S+/g, '').replace(/\s+/g, ' ').trim()
    : args.trim();
  return `${base} ${flag}`.trim();
}
