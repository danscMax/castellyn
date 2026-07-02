# castellyn-status-version: 1
# Claude Code lifecycle hook -> Castellyn agent-status file.
#
# Castellyn spawns each Sessions pane with CASTELLYN_SESSION_ID in the env; this hook
# (wired into SessionStart / UserPromptSubmit / Notification / Stop / SessionEnd of every
# profile) reports the semantic state of THAT pane by writing a tiny JSON file the app
# watches. Sessions started outside Castellyn have no env id -> instant no-op exit, so
# regular Claude Code use is unaffected. Fail-open: never block or break the session.
#
# Managed by Castellyn (Sessions settings); manual edits are overwritten on updates.
import json
import os
import sys
import time

STATE = {
    "SessionStart": "idle",        # agent is up, waiting for the first prompt
    "UserPromptSubmit": "working",
    "Notification": "blocked",     # permission request / waiting for input
    "Stop": "idle",                # turn finished ("done" is derived by the UI until seen)
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
    state = STATE.get(data.get("hook_event_name", ""))
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
    tmp = fp + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(payload, f)
    os.replace(tmp, fp)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        pass  # fail-open
    sys.exit(0)
