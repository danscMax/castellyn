import { describe, it, expect, vi } from 'vitest';
import { menuStillUp } from './menuGuard';

// The detector itself lives in Rust (agent_status.rs) and is tested there — including the positional
// veto. What has to hold HERE is the refusal contract: whatever the probe says, a keystroke is sent
// only on an exact match, and every failure mode ends in "don't send".
const probeReturning = (v: string | null) => vi.fn(async () => v);

describe('menuStillUp', () => {
  it('lets the press through only when the live pane still shows that exact menu', async () => {
    const tail = () => ['What do you want to do?', '1. Stop and wait for limit to reset'];
    await expect(menuStillUp('limit', tail, probeReturning('limitMenu'))).resolves.toBe(true);
    await expect(menuStillUp('resume', tail, probeReturning('resumeMenu'))).resolves.toBe(true);
  });

  it('refuses when the pane has moved on to a permission prompt', async () => {
    // The failure the whole guard exists for: the scan-time flag said "limit menu", the user clicked,
    // the confirm dialog took a beat, and by the time the action ran the agent was asking for
    // approval instead. The backend now reports nothing answerable → nothing is sent.
    const prompt = () => ['Do you want to make this edit to lib.rs?', '❯ 1. Yes', '  2. No'];
    const probe = probeReturning(null);
    await expect(menuStillUp('limit', prompt, probe)).resolves.toBe(false);
    expect(probe).toHaveBeenCalledWith(prompt().join('\n'));
  });

  it('refuses on the wrong menu, a bare limit banner, an empty read and a throwing probe', async () => {
    const tail = () => ['something'];
    // Answering the resume menu's "2" into a limit menu is just as wrong as pressing into a prompt.
    await expect(menuStillUp('resume', tail, probeReturning('limitMenu'))).resolves.toBe(false);
    // A banner is not an answerable menu — the quota is spent, but there is nothing to press.
    await expect(menuStillUp('limit', tail, probeReturning('limit'))).resolves.toBe(false);
    // No terminal to read = we cannot see the pane at all. That is a refusal, not a pass.
    await expect(menuStillUp('limit', () => [], probeReturning('limitMenu'))).resolves.toBe(false);
    await expect(menuStillUp('limit', () => ['  ', ''], probeReturning('limitMenu'))).resolves.toBe(
      false
    );
    await expect(
      menuStillUp('limit', tail, async () => {
        throw new Error('ipc down');
      })
    ).resolves.toBe(false);
    // A pane ref that vanished mid-click throws inside readTail — same answer.
    await expect(
      menuStillUp(
        'limit',
        () => {
          throw new Error('pane gone');
        },
        probeReturning('limitMenu')
      )
    ).resolves.toBe(false);
  });
});
