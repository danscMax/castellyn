import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// The opencode reporter is a state machine, and its first version got the states wrong in a way no
// screenshot or type-check could show: a stalled provider emits `session.status {type:"retry"}` while
// the agent keeps working, and treating anything-but-busy as done made the pane flap
// working -> idle -> working on every retry. Each flap is a false "the agent finished" toast.

// The asset is the shipped artifact — imported, not copied, so the test cannot drift from it.
import plugin from '../../src-tauri/assets/castellyn_opencode_plugin.js';

const PANE = 's0123456789abcde';
let dir: string;

// One module, but each call to the factory closes over its own busy/pending state, and it reads the
// environment at call time — so no module cache games are needed.
const load = async () => plugin;

const statusFile = () => join(dir, 'castellyn', 'agent-status', `${PANE}.json`);
const stateOf = () => JSON.parse(readFileSync(statusFile(), 'utf8')).state as string;

/** Feed one bus event, exactly as opencode's `event` hook delivers it. */
const send = (hooks: any, type: string, properties: Record<string, unknown>) =>
  hooks.event({ event: { type, properties } });

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'castellyn-oc-'));
  process.env.APPDATA = dir;
  process.env.CASTELLYN_SESSION_ID = PANE;
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe('opencode status plugin', () => {
  it('registers nothing when opencode was not launched by Castellyn', async () => {
    delete process.env.CASTELLYN_SESSION_ID;
    const hooks = await (await load())();
    expect(hooks).toEqual({});
  });

  it('stays working through a retry storm', async () => {
    const hooks = await plugin();
    await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'busy' } });
    expect(stateOf()).toBe('working');

    // A provider that cannot be reached alternates retry and busy several times a second. The defect
    // lived in the GAP, not the end state, so assert after every single retry — checking only once
    // the storm is over passes even with the bug.
    for (let i = 1; i <= 4; i++) {
      await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'retry', attempt: i } });
      expect(stateOf(), `retry #${i} must not read as a finished turn`).toBe('working');
      await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'busy' } });
      expect(stateOf()).toBe('working');
    }

    await send(hooks, 'session.idle', { sessionID: 'a' });
    expect(stateOf()).toBe('idle'); // only session.idle ends a turn
  });

  it('a subagent finishing does not report the parent as done', async () => {
    const hooks = await plugin();
    await send(hooks, 'session.status', { sessionID: 'parent', status: { type: 'busy' } });
    await send(hooks, 'session.status', { sessionID: 'child', status: { type: 'busy' } });
    await send(hooks, 'session.idle', { sessionID: 'child' });
    expect(stateOf()).toBe('working');
    await send(hooks, 'session.idle', { sessionID: 'parent' });
    expect(stateOf()).toBe('idle');
  });

  it('stays blocked until every pending permission is answered', async () => {
    const hooks = await plugin();
    await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'busy' } });
    await send(hooks, 'permission.asked', { sessionID: 'a', id: 'r1' });
    await send(hooks, 'permission.asked', { sessionID: 'a', id: 'r2' });
    expect(stateOf()).toBe('blocked');
    await send(hooks, 'permission.replied', { sessionID: 'a', requestID: 'r1' });
    expect(stateOf()).toBe('blocked'); // r2 is still unanswered
    await send(hooks, 'permission.replied', { sessionID: 'a', requestID: 'r2' });
    expect(stateOf()).toBe('working');
  });

  it('writes only on a change, and leaves no temp file behind', async () => {
    const hooks = await plugin();
    await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'busy' } });
    const first = readFileSync(statusFile(), 'utf8');
    await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'busy' } });
    await send(hooks, 'session.status', { sessionID: 'a', status: { type: 'busy' } });
    expect(readFileSync(statusFile(), 'utf8')).toBe(first); // identical bytes: no rewrite, no new ts
    expect(existsSync(`${statusFile()}.${process.pid}.tmp`)).toBe(false);
  });

  it('ignores events it does not model', async () => {
    const hooks = await plugin();
    await send(hooks, 'file.edited', { file: 'x.ts' });
    expect(existsSync(statusFile())).toBe(false); // no state, so nothing to report
  });
});
