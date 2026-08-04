/**
 * Backlog 27: the last gate between a click and a keystroke in a live agent.
 *
 * `limitMenu` is a SCAN-time verdict: the backend saw a menu in some PTY chunk and raised a flag it
 * only lowers on a recognised permission prompt or 4 KiB of further output. Neither is reliable —
 * the prompt allowlist is hand-authored, and a genuine, differently-worded approval prompt fits in
 * far less than 4 KiB. So between "the button became visible" and "the user clicked it" (plus the
 * confirm dialog's own decision delay, which WIDENS the window) the pane can have moved on to a
 * prompt that a "1" would approve.
 *
 * The close is to stop trusting the flag at the moment of writing: re-read the pane's rendered rows
 * and ask the SAME Rust detector (`menu_signal_in_text` → `limit_signal_in`) what is on screen now.
 * The wording therefore lives in exactly one place — there is no TypeScript copy to drift, and on
 * the rendered buffer the "is the prompt below the menu" ordering is real screen order rather than
 * the byte-order proxy the PTY scanner has to make do with.
 */

export type MenuKind = 'limit' | 'resume';

/**
 * True only if `readTail()` still shows the menu `kind` we are about to answer.
 *
 * Fail-closed on every uncertainty: an empty read (no terminal to look at) and a probe that throws
 * both mean "we cannot see the pane", which is never a licence to press a key.
 */
export async function menuStillUp(
  kind: MenuKind,
  readTail: () => string[],
  probe: (text: string) => Promise<string | null>
): Promise<boolean> {
  const want = kind === 'limit' ? 'limitMenu' : 'resumeMenu';
  try {
    const tail = readTail().join('\n');
    if (!tail.trim()) return false;
    // Exactly the expected menu — a bare `limit` banner (menu already answered, quota still spent)
    // is not something to send a keystroke into either.
    return (await probe(tail)) === want;
  } catch {
    return false;
  }
}
