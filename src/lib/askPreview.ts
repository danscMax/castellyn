/**
 * Backlog 28: distil "what is this agent asking?" into one line, from the pane's OWN terminal buffer.
 *
 * Deliberately frontend-only — no backend hook, no new IPC. The hook that would know the real prompt
 * text is a separate, deferred task; until then the rail row and the attention strip can at least say
 * something more useful than "waits".
 *
 * Agent TUIs draw the question inside a box: border rows, the question, then short option labels
 * ("1. Yes"). The question is the longest line of that group, so that is what this picks.
 * ponytail: longest-line heuristic, not a parser — a prompt whose longest line is a file path inside
 * an option row will preview that instead. Upgrade path is the deferred hook, which knows the text.
 * Nothing acts on this string; it is display-only, so a wrong guess costs a bad label, not a keypress.
 */

// Box-drawing and block glyphs are pure decoration in a TUI prompt — they carry no words, but they
// are what makes a raw buffer line unreadable in a 200px rail row.
const DECOR = /[─-▟]/g;
// An option row: an optional selection marker, then "1." / "2)" etc. Short by nature and identical
// across every prompt, so it never answers "what is being asked".
const OPTION_ROW = /^[>❯»*·•\-\s]*\d+\s*[.)]\s/;

/** One buffer line with the box art removed and whitespace collapsed. */
export function cleanLine(s: string): string {
  return s.replace(DECOR, ' ').replace(/\s+/g, ' ').trim();
}

/**
 * The most question-like line among `lines` (oldest → newest), truncated to `max`.
 * Returns '' when nothing in the window looks like prose.
 */
export function askPreview(lines: string[], max = 90): string {
  let best = '';
  for (const raw of lines) {
    const s = cleanLine(raw);
    // Too short to be a question, or an option row — either way it says nothing.
    if (s.length < 8 || OPTION_ROW.test(s)) continue;
    if (s.length > best.length) best = s;
  }
  return best.length > max ? best.slice(0, max - 1).trimEnd() + '…' : best;
}
