import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// The wide tables have to fit the default window, and until now that was an assertion in a comment:
// "1020px against the 1015px a default window gives". Comments do not fail a build. A column added
// later, or a smaller default window, would silently put a column back behind a horizontal scrollbar
// — which is exactly how the MCP table broke when the liveness-probe column was added.
//
// This parses the widths straight out of the components and out of tauri.conf.json, in the spirit of
// the STREAM_IDS parity test in ipc.test.ts: no codegen, no duplicated constant, it just breaks when
// the two drift apart.

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');

// Everything the window spends before the table gets any: sidebar, page padding, card padding and
// the table's own borders. MEASURED on the running app (window 1320 → .dt-scroll clientWidth 1015).
// If the sidebar is ever resized this is the one number to re-measure.
const CHROME_PX = 305;

// DataTable's own defaults — mirrored from colWidth() in DataTable.svelte.
const GROW_DEFAULT = 260;
const PLAIN_DEFAULT = 160;

function columnBudget(source: string): number {
  const block = source.match(/const COLS: DTColumn\[\] = \$derived\(\[([\s\S]*?)\n {2}\]\)/);
  if (!block) throw new Error('COLS array not found — did the declaration change shape?');
  // One entry per `{ key: … }` object literal.
  const entries = block[1].match(/\{[^{}]*key:[^{}]*\}/g) ?? [];
  expect(entries.length, 'columns parsed').toBeGreaterThan(3);
  return entries.reduce((sum, e) => {
    const explicit = e.match(/width:\s*'(\d+)px'/);
    if (explicit) return sum + Number(explicit[1]);
    return sum + (/grow:\s*true/.test(e) ? GROW_DEFAULT : PLAIN_DEFAULT);
  }, 0);
}

describe('wide tables fit the default window', () => {
  const conf = JSON.parse(read('../../src-tauri/tauri.conf.json'));
  const defaultWidth: number = conf.app.windows[0].width;
  const available = defaultWidth - CHROME_PX;

  it.each([
    // `extra` = fixed columns DataTable adds that are not in COLS (the expand chevron).
    { name: 'MCP', file: '../lib/components/McpTab.svelte', extra: 0 },
    { name: 'Profiles', file: '../lib/components/ProfilesTab.svelte', extra: 28 },
  ])('$name columns fit', ({ file, extra }) => {
    const total = columnBudget(read(file)) + extra;
    expect(
      total,
      `columns need ${total}px but a ${defaultWidth}px window leaves ~${available}px — either widen ` +
        `the default window or take the difference out of the grow column, NOT out of the fixed ` +
        `columns (their sizes are load-bearing; see the notes beside them)`
    ).toBeLessThanOrEqual(available);
  });

  it('the parsed defaults still match DataTable', () => {
    // If colWidth's fallbacks change, the sums above are computed from stale numbers and this test
    // would keep passing while the real layout drifts.
    const dt = read('../lib/components/DataTable.svelte');
    expect(dt).toContain(`c.grow ? '${GROW_DEFAULT}px' : '${PLAIN_DEFAULT}px'`);
  });
});
