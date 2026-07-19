# castellyn-status-version: 3
# Claude Code lifecycle hook -> Castellyn agent-status file.
#
# Castellyn spawns each Sessions pane with CASTELLYN_SESSION_ID in the env; this hook
# (wired into the lifecycle events of every profile) reports the semantic state of THAT
# pane by writing a tiny JSON file the app watches. Sessions started outside Castellyn
# have no env id -> instant no-op exit, so regular Claude Code use is unaffected.
# Fail-open: never block or break the session.
#
# v2: PreToolUse/PostToolUse(+Failure) refresh `working` DURING a long turn, so a quiet
# tool call (a long Bash run with no PTY output) no longer self-heals to a false idle;
# PermissionRequest / an AskUserQuestion tool-call report `blocked` the moment the agent
# is actually waiting on the human. Event mapping follows the hook contract Orca's
# integration verified in production (working/waiting/done classes).
#
# Managed by Castellyn (Sessions settings); manual edits are overwritten on updates.
import json
import os
import sys
import time

STATE = {
    "SessionStart": "idle",        # agent is up, waiting for the first prompt
    "UserPromptSubmit": "working",
    "PreToolUse": "working",       # heartbeat during the turn (see AskUserQuestion below)
    "PostToolUse": "working",
    "PostToolUseFailure": "working",  # a failed tool doesn't end the turn
    "Notification": "blocked",     # permission request / waiting for input
    "PermissionRequest": "blocked",
    "Stop": "idle",                # turn finished ("done" is derived by the UI until seen)
    "StopFailure": "idle",
    "SessionEnd": "ended",         # agent gone, pane is back to a plain shell
}


def main():
    sid = os.environ.get("CASTELLYN_SESSION_ID", "")
    if not sid or not sid.isalnum() or len(sid) > 32:
        return
    try:
        data = json.load(sys.stdin)
    except Exception:
        data = {}
    event = data.get("hook_event_name", "")
    state = STATE.get(event)
    # An AskUserQuestion tool-call is the agent waiting on the human, not working.
    if event == "PreToolUse" and data.get("tool_name") == "AskUserQuestion":
        state = "blocked"
    if not state:
        return
    base = os.environ.get("APPDATA")
    if not base:
        return
    out_dir = os.path.join(base, "castellyn", "agent-status")
    os.makedirs(out_dir, exist_ok=True)
    fp = os.path.join(out_dir, sid + ".json")
    payload = {
        "state": state,
        "event": data.get("hook_event_name", ""),
        "claudeSessionId": data.get("session_id", ""),
        "ts": int(time.time() * 1000),
    }
    # pid-unique temp name: parallel tool calls in ONE session fire these hooks concurrently, so a
    # shared "<sid>.json.tmp" would let two processes truncate/write the same path and os.replace over
    # each other — the poller then reads interleaved JSON. Mirrors plugin_sync.py / _opencode_plugin.js.
    tmp = fp + f".{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(payload, f)
    os.replace(tmp, fp)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        pass  # fail-open
    sys.exit(0)
