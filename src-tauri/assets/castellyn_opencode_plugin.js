// castellyn-plugin-version: 1
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
  // opencode runs sub-sessions (subagents) that raise and clear their own busy status. Counting the
  // busy set, rather than watching one session id, means a subagent finishing cannot report the pane
  // as done while the parent is still working — and no parentID lookup is needed.
  const busy = new Set();
  const asking = new Set();
  let last = null;

  const report = () => {
    const state = asking.size ? 'blocked' : busy.size ? 'working' : 'idle';
    if (state === last) return; // dedup: opencode emits status far more often than it changes
    last = state;
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const payload = JSON.stringify({ state, event: 'opencode', ts: Date.now() });
      // Same temp+rename the Rust writer uses: the poll thread must never read a half-written file.
      fs.writeFileSync(`${file}.tmp`, payload, 'utf8');
      fs.renameSync(`${file}.tmp`, file);
    } catch {
      last = null; // a failed write must not suppress the next attempt
    }
  };

  return {
    async event({ event }) {
      try {
        const p = event?.properties ?? {};
        switch (event?.type) {
          case 'session.status':
            // The status union carries other shapes (retry/auth actions); only `busy` means working.
            if (p.status?.type === 'busy') busy.add(p.sessionID);
            else busy.delete(p.sessionID);
            break;
          case 'session.idle':
          case 'session.deleted':
            busy.delete(p.sessionID);
            asking.delete(p.sessionID);
            break;
          case 'permission.asked':
            asking.add(p.sessionID);
            break;
          case 'permission.replied':
            asking.delete(p.sessionID);
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
