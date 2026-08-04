# castellyn-status-version: 4
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
# v4: say WHAT the pane is waiting for, not just that it waits — `reason` (which kind of prompt),
# `ask` (a short excerpt of it) and `cwd` (the project, so a notification can name it before the
# frontend pushes a label). A SUBAGENT's tool calls arrive on the parent's session_id AND inherit
# the parent's CASTELLYN_SESSION_ID (measured), so without the `agent_id` discriminator a
# subagent's prompt would report the pane as waiting on the human when nobody is asked — the guard
# therefore sits on the `blocked` STATE, covering all three events that produce it.
# Self-test: `py -X utf8 tools/test_status_hook.py` (asserts only, no framework).
#
# Managed by Castellyn (Sessions settings); manual edits are overwritten on updates.
import json
import os
import re
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

ASK_CAP = 120  # one rail row's worth; askPreview caps at 90, a question deserves a little more
CWD_CAP = 260  # MAX_PATH: long enough that the basename (all the backend keeps) survives the cap


def excerpt(value, cap=ASK_CAP):
    """One short, printable line — the ONLY way text from the payload reaches the status file.

    Control characters are dropped rather than escaped (a TUI question can carry ANSI), whitespace
    is collapsed so a multi-line prompt still fits one row, and the length is hard-capped: this
    string is displayed, never parsed, so a truncated one costs nothing. Anything that is not a
    string collapses to "" — the caller never has to know the payload's shape.
    """
    if not isinstance(value, str):
        return ""
    s = re.sub(r"\s+", " ", re.sub(r"[\x00-\x1f\x7f]", " ", value)).strip()
    return s[: cap - 1] + "…" if len(s) > cap else s


def describe(data, event, tool):
    """(reason, ask) for a blocked pane. TOTAL: never raises, whatever JSON shape arrives.

    Every field is type-checked before it is walked — a hook payload is attacker-adjacent input
    (it is whatever the agent's tool call happened to contain), and an AttributeError here used to
    abort the whole status write, i.e. lose the `blocked` state this feature exists to report.

    ALLOW-LIST, never `tool_input` wholesale: `tool_input.command` carries raw shell command lines
    (that is what the user's own rtk_guard hook reads), and the status file is written to disk and
    shown in OS notifications. Only two sources are read: the tool NAME (non-secret by
    construction) and the AskUserQuestion question text (written to be shown to the human anyway).
    """
    if tool == "AskUserQuestion":
        tool_input = data.get("tool_input")
        questions = tool_input.get("questions") if isinstance(tool_input, dict) else None
        first = questions[0] if isinstance(questions, list) and questions else None
        return "question", excerpt(first.get("question") if isinstance(first, dict) else None)
    if event == "PermissionRequest":
        return "permission", excerpt(tool)
    return "notification", ""


def is_subagent(data):
    """True only when the payload UNAMBIGUOUSLY belongs to a subagent.

    Measured over 154 live hook events (two sessions, one of them NOT started by Castellyn):
    `agent_id` is absent from every main-thread payload and is a non-empty string on subagent
    calls. The check is written to fail SAFE around that observation — a missing key, null, an
    empty string or any future non-string shape all read as MAIN thread, so an ambiguous payload
    still reports `blocked`. A spurious "agent is waiting" toast costs a glance; a swallowed one
    costs the entire feature.
    """
    agent_id = data.get("agent_id")
    return isinstance(agent_id, str) and agent_id != ""


def main():
    sid = os.environ.get("CASTELLYN_SESSION_ID", "")
    if not sid or not sid.isalnum() or len(sid) > 32:
        return
    try:
        data = json.load(sys.stdin)
    except Exception:
        data = {}
    if not isinstance(data, dict):
        data = {}  # valid JSON is not necessarily an object; everything below indexes it
    event = data.get("hook_event_name")
    tool = data.get("tool_name")
    # Non-string keys would blow up the STATE lookup (an unhashable one raises outright).
    event = event if isinstance(event, str) else ""
    tool = tool if isinstance(tool, str) else ""
    state = STATE.get(event)
    # An AskUserQuestion tool-call is the agent waiting on the human, not working.
    if event == "PreToolUse" and tool == "AskUserQuestion":
        state = "blocked"
    # A subagent's tool calls are reported on the PARENT's session_id and inherit the parent's
    # CASTELLYN_SESSION_ID, so they land on the parent's pane. Their `working` heartbeat is honest
    # (the turn really is running), but "waiting on the human" is not: a subagent's prompt reaches
    # its spawner, not the terminal, and `blocked` sticks for ~20s once set. Guard the STATE, not
    # one event: Notification and PermissionRequest map to `blocked` just as AskUserQuestion does,
    # and a subagent produces those on the parent's pane by the same mechanism.
    if state == "blocked" and is_subagent(data):
        state = "working"
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
        "event": event,
        "claudeSessionId": data.get("session_id", ""),
        # The pane's project directory. Its basename becomes a notification label, so it goes
        # through the same sanitiser as `ask` — a path is still payload-controlled text.
        "cwd": excerpt(data.get("cwd"), CWD_CAP),
        "ts": int(time.time() * 1000),
    }
    # Only a blocked pane has something to say; anything else would ride stale into the next turn.
    # The enrichment is a bonus, the STATE is the point: a payload shape describe() has never seen
    # must not cost the pane its "waiting for you" notification.
    if state == "blocked":
        try:
            payload["reason"], payload["ask"] = describe(data, event, tool)
        except Exception:
            payload["reason"], payload["ask"] = "notification", ""
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
