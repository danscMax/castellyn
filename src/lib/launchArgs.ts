// The single place a launcher's structured pickers turn into free-text CLI args. Pure & tested so
// the "emit each flag exactly once, never double what the user typed" rule can't silently regress.

export type LaunchSelection = {
  codexProfile?: string;
  codexModel?: string;
  opencodeModel?: string;
  // Claude structured controls (Task 4): a reasoning effort and an optional model override.
  claudeEffort?: string;
  claudeModel?: string;
};

const PROFILE_RE = /(^|\s)(--profile|-p)(\s|=)/;
const MODEL_RE = /(^|\s)(--model|-m)(\s|=)/;
const EFFORT_RE = /(^|\s)--effort(\s|=)/;

/**
 * Bake the structured launcher selection into the free-text `args`. Each flag is added at most once
 * and ONLY when the user hasn't already typed it by hand (hand-typed wins → no doubling). Structured
 * flags are prepended so they read first. Non-matching flags in `args` are preserved verbatim.
 */
export function composeLaunchArgs(env: string, args: string, sel: LaunchSelection): string {
  let a = args.trim();
  const add = (flag: string, re: RegExp) => {
    if (flag && !re.test(a)) a = a ? `${flag} ${a}` : flag;
  };
  const v = (s?: string) => (s ?? '').trim();
  if (env === 'codex') {
    add(v(sel.codexModel) && `--model ${v(sel.codexModel)}`, MODEL_RE);
    add(v(sel.codexProfile) && `--profile ${v(sel.codexProfile)}`, PROFILE_RE);
  } else if (env === 'opencode') {
    add(v(sel.opencodeModel) && `--model ${v(sel.opencodeModel)}`, MODEL_RE);
  } else if (env === 'claude') {
    add(v(sel.claudeModel) && `--model ${v(sel.claudeModel)}`, MODEL_RE);
    add(v(sel.claudeEffort) && `--effort ${v(sel.claudeEffort)}`, EFFORT_RE);
  }
  return a;
}
