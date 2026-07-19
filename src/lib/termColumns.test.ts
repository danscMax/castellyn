import { describe, it, expect } from 'vitest';
import { colAt, type BufferLineLike, type CellLike } from './termColumns';

/**
 * Build a fake buffer line from cell specs. A wide char is written as TWO cells the way xterm stores
 * it: the visible one with width 2, then a spacer with width 0 and no chars.
 */
function line(cells: Array<{ chars: string; width: number }>): BufferLineLike {
  const cellObjs: CellLike[] = cells.map((c) => ({
    getWidth: () => c.width,
    getChars: () => c.chars
  }));
  return { length: cellObjs.length, getCell: (col) => cellObjs[col] };
}

const ascii = (s: string) => s.split('').map((ch) => ({ chars: ch, width: 1 }));
const wide = (ch: string) => [
  { chars: ch, width: 2 },
  { chars: '', width: 0 }
];

describe('colAt', () => {
  it('is the identity on plain ASCII', () => {
    const l = line(ascii('C:\\x.rs:12'));
    expect(colAt(l, 0)).toBe(0);
    expect(colAt(l, 5)).toBe(5);
  });

  it('shifts by one column per wide char to the left of the index', () => {
    // String "日x" is 2 chars; columns are [日][spacer][x] — so 'x' is string index 1, column 2.
    const l = line([...wide('日'), ...ascii('x')]);
    expect(colAt(l, 0)).toBe(0);
    expect(colAt(l, 1)).toBe(2);
  });

  it('accumulates the shift across several wide chars', () => {
    // "日本x": columns [日][sp][本][sp][x]; 'x' is string index 2, column 4.
    const l = line([...wide('日'), ...wide('本'), ...ascii('x')]);
    expect(colAt(l, 2)).toBe(4);
  });

  it('counts a multi-char cell (combining mark / surrogate pair) as its char length', () => {
    // One cell carrying "é" is 2 string chars but 1 column, so the next char lands at column 1.
    const l = line([{ chars: 'é', width: 1 }, ...ascii('x')]);
    expect(colAt(l, 2)).toBe(1);
  });

  it('treats an empty cell as one space rather than consuming nothing', () => {
    // A blank cell that advanced no string index would make the walk stall and return early.
    const l = line([{ chars: '', width: 1 }, ...ascii('ab')]);
    expect(colAt(l, 1)).toBe(1);
    expect(colAt(l, 2)).toBe(2);
  });

  it('clamps past-the-end indices to the line length instead of running off the buffer', () => {
    const l = line(ascii('abc'));
    expect(colAt(l, 99)).toBe(3);
  });

  it('falls back to the string index when there is no line', () => {
    expect(colAt(undefined, 7)).toBe(7);
  });
});
