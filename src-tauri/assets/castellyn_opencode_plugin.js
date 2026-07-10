// castellyn-plugin-version: 2
// opencode plugin -> Castellyn agent-status file.
//
// opencode has no per-turn notifier the way codex does, but it does merge an extra config from
// OPENCODE_CONFIG_CONTENT ("inject inline JSON as a final local-scope merge"), and plugin specs from
// that config are APPENDED to the user's own list. Castellyn spawns an opencode pane with that env
// var pointing here, so nothing outside Castellyn's own directory is ever written.
//
// Without this, a pane can only guess a turn ended from PTY silence, and so never dares announce a
// completion. Written by Castellyn; manual edits are overwritten on updates.

import fs from 'node:fs';
import path from 'node:path';

/** Pane ids are `s` + 15 hex (gen_session_id). Anything else means we were not launched by Castellyn. */
function paneId() {
  const id = process.env.CASTELLYN_SESSION_ID;
  return id && /^[A-Za-z0-9]{1,32}$/.test(id) ? id : null;
}

export default async () => {
  const pane = paneId();
  // An opencode started outside Castellyn loads this plugin too (it is merged into the config, not
  // scoped to a session), so registering no hooks at all is the only correct thing to do.
  if (!pane || !process.env.APPDATA) return {};

  const file = path.join(process.env.APPDATA, 'castellyn', 'agent-status', `${pane}.json`);
  // opencode runs sub-sessions (subagents) that raise and clear their own status. Counting the busy
  // set, rather than watching one session id, means a subagent finishing cannot report the pane as
  // done while the parent is still working — and no parentID lookup is needed.
  const busy = new Set();
  // sessionID -> the ids of its unanswered permission requests. A session can have more than one in
  // flight, so a single flag would unblock on the first reply.
  /** @type {Map<string, Set<string>>} */
  const pending = new Map();
  /** @type {string | null} */
  let last = null;

  /** @param {Record<string, any> | undefined} p */
  const key = (p) => p?.sessionID ?? '?';

  const report = () => {
    let state = 'idle';
    for (const reqs of pending.values()) if (reqs.size) state = 'blocked';
    if (state === 'idle' && busy.size) state = 'working';
    if (state === last) return; // opencode emits status far more often than it changes
    last = state;
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const payload = JSON.stringify({ state, event: 'opencode', ts: Date.now() });
      // Same temp+rename the Rust writer uses: the poll thread must never read a half-written file.
      const tmp = `${file}.${process.pid}.tmp`;
      fs.writeFileSync(tmp, payload, 'utf8');
      fs.renameSync(tmp, file);
    } catch {
      last = null; // a failed write must not suppress the next attempt
    }
  };

  return {
    /** @param {{ event?: { type?: string, properties?: Record<string, any> } }} input */
    async event({ event }) {
      try {
        const p = event?.properties ?? {};
        const sid = key(p);
        switch (event?.type) {
          // ANY status means the turn is still running. `busy` is not the only one: a stalled
          // provider produces {type:"retry", attempt, message, next} while the agent keeps trying,
          // and treating that as "not busy" made the pane flip working->idle->working on every
          // retry — which is exactly the false "finished" this feature exists to remove. Only
          // `session.idle` ends a turn, so only it may clear the flag (verified against a live
          // opencode: retry storms fire several statuses per second).
          case 'session.status':
            busy.add(sid);
            break;
          case 'session.idle':
          case 'session.deleted':
            busy.delete(sid);
            pending.delete(sid);
            break;
          case 'permission.asked': {
            const reqs = pending.get(sid) ?? new Set();
            reqs.add(p.id);
            pending.set(sid, reqs);
            break;
          }
          case 'permission.replied':
            pending.get(sid)?.delete(p.requestID);
            break;
          default:
            return;
        }
        report();
      } catch {
        // A broken reporter must never break the agent.
      }
    },
  };
};
