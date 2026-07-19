/**
 * Mapping between a string index in `line.translateToString(true)` and a buffer COLUMN.
 *
 * They are not the same coordinate: a wide cell (CJK, emoji) is one string char but occupies two
 * columns — its trailing half reports width 0 and is skipped by translateToString — and a single
 * cell can carry several chars (combining marks, surrogate pairs). Treating the string index as a
 * column makes xterm link ranges drift right of the text they underline, by one column per wide
 * char before them.
 *
 * Same walk as xterm's own `_mapStrIdx`, minus the wrapped-line expansion we don't do (our matches
 * never span a wrapped row). Lives here rather than inside TerminalPane so it can be tested without
 * standing up a Terminal.
 */

/** The slice of xterm's `IBufferCell` this walk needs — structural, so the real thing satisfies it. */
export interface CellLike {
  getWidth(): number;
  getChars(): string;
}

/** The slice of xterm's `IBufferLine` this walk needs. */
export interface BufferLineLike {
  readonly length: number;
  getCell(col: number): CellLike | undefined;
}

/**
 * Buffer column holding the character at `strIdx` of the line's string form.
 * Returns `strIdx` unchanged when there is no line (nothing better to say), and the line length
 * when `strIdx` runs past the end — both keep callers inside the buffer.
 */
export function colAt(line: BufferLineLike | undefined, strIdx: number): number {
  if (!line) return strIdx;
  let i = 0;
  for (let col = 0; col < line.length; col++) {
    const cell = line.getCell(col);
    if (!cell) break;
    if (cell.getWidth() === 0) continue; // trailing half of a wide char — contributes no string char
    if (i >= strIdx) return col;
    i += cell.getChars().length || 1; // an empty cell renders as one space
  }
  return line.length;
}
