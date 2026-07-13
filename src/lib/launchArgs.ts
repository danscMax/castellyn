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

const PROFILE_FLAGS = ['--profile', '-p'];
const MODEL_FLAGS = ['--model', '-m'];
const EFFORT_FLAGS = ['--effort'];

// Whole-token match instead of a raw substring regex: split on whitespace and compare each token
// (or its pre-'=' head) against the flag names. Fixes a bare trailing flag (e.g. args ending in
// just "--model", no value/separator) that the old regex's trailing (\s|=) requirement missed.
function hasFlag(a: string, names: string[]): boolean {
  return a
    .split(/\s+/)
    .some((tok) => names.includes(tok.includes('=') ? tok.slice(0, tok.indexOf('=')) : tok));
}

/**
 * Bake the structured launcher selection into the free-text `args`. Each flag is added at most once
 * and ONLY when the user hasn't already typed it by hand (hand-typed wins → no doubling). Structured
 * flags are prepended so they read first. Non-matching flags in `args` are preserved verbatim.
 */
export function composeLaunchArgs(env: string, args: string, sel: LaunchSelection): string {
  let a = args.trim();
  const add = (flag: string, names: string[]) => {
    if (flag && !hasFlag(a, names)) a = a ? `${flag} ${a}` : flag;
  };
  const v = (s?: string) => (s ?? '').trim();
  if (env === 'codex') {
    add(v(sel.codexModel) && `--model ${v(sel.codexModel)}`, MODEL_FLAGS);
    add(v(sel.codexProfile) && `--profile ${v(sel.codexProfile)}`, PROFILE_FLAGS);
  } else if (env === 'opencode') {
    add(v(sel.opencodeModel) && `--model ${v(sel.opencodeModel)}`, MODEL_FLAGS);
  } else if (env === 'claude') {
    add(v(sel.claudeModel) && `--model ${v(sel.claudeModel)}`, MODEL_FLAGS);
    add(v(sel.claudeEffort) && `--effort ${v(sel.claudeEffort)}`, EFFORT_FLAGS);
  }
  return a;
}
