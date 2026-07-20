import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Adoption guard for the confirm gate.
 *
 * The audit found this gate hand-copied into four components plus a fifth degenerate copy inlined
 * in markup, and the copies had already drifted apart — each honoured the global bypass, none
 * carried the type-to-confirm exemption or fired a displaced dialog's onCancel. Consolidating them
 * only helps if a sixth copy cannot quietly appear, so this scans the source rather than trusting
 * review: a component that reads `confirmDestructive` must reach it through $lib/confirmGate.
 */

const SRC = join(process.cwd(), 'src');

function svelteFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    return e.isDirectory() ? svelteFiles(p) : e.name.endsWith('.svelte') ? [p] : [];
  });
}

/**
 * Does this file DECIDE on the toggle, rather than declaring it, forwarding it, or handing it to
 * the gate? The ternary test excludes `?:` — that is the optional-property marker in a prop type.
 *
 * Note the rule is absolute: importing the gate does not buy a component the right to also branch
 * by hand. A first attempt exempted anything that imported `$lib/confirmGate`, and it happily
 * passed when a hand-rolled bypass was put back into a component that also used the gate — which
 * is precisely the hybrid drift this guard exists to catch.
 */
function decidesOnTheToggle(src: string): boolean {
  return /if\s*\(\s*!\s*confirmDestructive\s*\)/.test(src) || /confirmDestructive\s*\?(?!:)/.test(src);
}

describe('confirm gate — adoption', () => {
  const files = svelteFiles(SRC);

  it('no component decides on confirmDestructive by hand — only the gate may', () => {
    const offenders = files
      .filter((f) => decidesOnTheToggle(readFileSync(f, 'utf-8')))
      .map((f) => f.replace(SRC, 'src'));
    expect(offenders).toEqual([]);
  });

  it('every gate-driven dialog binds the full state, so no field is silently dropped', () => {
    const required = ['open', 'title', 'message', 'details', 'confirmLabel', 'requireText', 'danger'];
    const problems: string[] = [];

    for (const f of files) {
      const lines = readFileSync(f, 'utf-8').split('\n');
      for (let i = 0; i < lines.length; i++) {
        if (!lines[i].includes('<ConfirmDialog')) continue;
        let j = i;
        while (j < lines.length && !lines[j].includes('/>')) j++;
        const block = lines.slice(i, j + 1).join('\n');
        // Only dialogs driven by the shared state; bespoke ones are intentional.
        if (!block.includes('confirm.open')) continue;
        const missing = required.filter((k) => !block.includes(`{confirm.${k}}`));
        if (missing.length) {
          problems.push(`${f.replace(SRC, 'src')}:${i + 1} — не привязаны: ${missing.join(', ')}`);
        }
        i = j;
      }
    }
    expect(problems).toEqual([]);
  });

  it('the gate itself is the only place that reads the bypass', () => {
    // If the module ever stops honouring the setting, every caller silently starts asking always
    // (or never) — the one behaviour nobody would notice until it matters.
    const gate = readFileSync(join(SRC, 'lib', 'confirmGate.ts'), 'utf-8');
    expect(gate).toMatch(/if\s*\(\s*!enabled\s*&&\s*!req\.requireText\s*\)/);
  });
});
