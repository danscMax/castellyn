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
  return args.includes(flag)
    ? args.replace(flag, '').replace(/\s+/g, ' ').trim()
    : `${args.trim()} ${flag}`.trim();
}
