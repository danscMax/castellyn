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
 * review.
 *
 * What it can and cannot do: it is a source scan, not a type system. It catches a component that
 * *reads* `confirmDestructive` for anything other than declaring, forwarding, or handing it to the
 * gate, and it catches a gate-driven dialog that drops part of the state or stubs its handlers.
 * It does not understand indirection — a component that laundered the flag through an imported
 * helper would pass. Widen it if that ever happens rather than assuming it cannot.
 */

const SRC = join(process.cwd(), 'src');

function svelteFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    return e.isDirectory() ? svelteFiles(p) : e.name.endsWith('.svelte') ? [p] : [];
  });
}

/**
 * Lines that legitimately mention the flag: the prop's type declaration, its default, forwarding it
 * to a child, and passing it to the gate. Anything else is the component making its own decision —
 * including `if (confirmDestructive) {…} else {…}` and `const skip = !confirmDestructive`, which an
 * earlier version of this guard missed because it only looked for the negated-if and the ternary.
 */
function offendingLines(src: string): string[] {
  return src.split('\n').filter((raw) => {
    // Strip quoted text first: `t('settings.confirmDestructive')` is a translation KEY that merely
    // contains the word, not a use of the flag.
    const l = raw
      .replace(/'[^']*'/g, "''")
      .replace(/"[^"]*"/g, '""')
      .trim();
    if (!l.includes('confirmDestructive')) return false;
    if (/^\/\/|^\*|^\/\*/.test(l)) return false; // comment
    if (/confirmDestructive\?\s*:/.test(l)) return false; // optional-prop type marker
    if (/^(let\s+)?confirmDestructive\s*=/.test(l)) return false; // owner's declaration / assignment
    if (/^\{confirmDestructive\}$/.test(l)) return false; // shorthand forward to a child
    if (/\{confirmDestructive\}/.test(l) && l.startsWith('<')) return false; // forward in markup
    if (/gateAsk\(\s*\w+\s*,\s*confirmDestructive\s*,/.test(l)) return false; // handed to the gate
    if (/^confirmDestructive,?$/.test(l)) return false; // destructuring entry
    return true;
  });
}

/** Extract each `<ConfirmDialog … />` block with its start line. */
function confirmDialogs(src: string): Array<{ line: number; block: string }> {
  const lines = src.split('\n');
  const out: Array<{ line: number; block: string }> = [];
  for (let i = 0; i < lines.length; i++) {
    if (!lines[i].includes('<ConfirmDialog')) continue;
    let j = i;
    while (j < lines.length && !lines[j].includes('/>')) j++;
    out.push({ line: i + 1, block: lines.slice(i, j + 1).join('\n') });
    i = j;
  }
  return out;
}

describe('confirm gate — adoption', () => {
  const files = svelteFiles(SRC);

  it('only the gate decides on confirmDestructive', () => {
    const offenders = files.flatMap((f) => {
      const src = readFileSync(f, 'utf-8');
      return offendingLines(src).map((l) => `${f.replace(SRC, 'src')}: ${l.trim().slice(0, 90)}`);
    });
    expect(offenders).toEqual([]);
  });

  it('every ConfirmDialog is fully wired — no dropped field, no stubbed handler', () => {
    // Keyed on the component rather than on a variable NAME: an earlier version only inspected
    // blocks containing the literal `confirm.open`, so renaming the state would have silently
    // exempted the dialog from every check below.
    const fields = ['open', 'title', 'message', 'details', 'confirmLabel', 'requireText', 'danger'];
    const problems: string[] = [];

    for (const f of files) {
      const src = readFileSync(f, 'utf-8');
      for (const { line, block } of confirmDialogs(src)) {
        const where = `${f.replace(SRC, 'src')}:${line}`;
        // Gate-driven is decided by what the dialog CALLS, not by what its state variable is named:
        // an earlier version keyed on the literal `confirm.open`, so a rename would have exempted
        // the dialog silently. A bespoke dialog (e.g. Sessions' send-to-all) is legitimate and only
        // has to satisfy the handler checks below.
        if (/onConfirm=\{[^}]*(gateDo|doConfirm)/.test(block)) {
          const missing = fields.filter((k) => !new RegExp(`\\b${k}=\\{`).test(block));
          if (missing.length) problems.push(`${where} — не привязаны поля: ${missing.join(', ')}`);
        }
        // A dialog whose handlers are stubs is worse than one missing a field: it renders, it looks
        // right, and it confirms nothing. This check was in the throwaway scanner and got lost on
        // the way into this file — the blind reviewer caught it.
        for (const handler of ['onConfirm', 'onCancel']) {
          const m = block.match(new RegExp(`${handler}=\\{([^]*?)\\}\\s*(?=\\w+=|/>)`));
          if (!m) {
            problems.push(`${where} — нет ${handler}`);
          } else if (/^\(\)\s*=>\s*\{\s*\}$/.test(m[1].trim())) {
            problems.push(`${where} — ${handler} — пустая заглушка`);
          }
        }
      }
    }
    expect(problems).toEqual([]);
  });

  it('the gate itself still honours the bypass and its type-to-confirm exemption', () => {
    // If the module stops reading the setting, every caller silently starts asking always (or
    // never) — the one behaviour nobody notices until it matters.
    const gate = readFileSync(join(SRC, 'lib', 'confirmGate.ts'), 'utf-8');
    expect(gate).toMatch(/if\s*\(\s*!enabled\s*&&\s*!req\.requireText\s*\)/);
    expect(gate).toMatch(/if\s*\(state\.open\)\s*state\.onCancel\?\.\(\)/);
  });
});
