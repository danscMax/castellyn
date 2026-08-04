# Self-test for the Claude Code status hook (src-tauri/assets/castellyn_status.py).
#
#   py -X utf8 tools/test_status_hook.py     # exit 0 = pass, 1 = a case failed
#
# Plain asserts, no framework: this repo has no Python test harness and one hook file does not
# justify one. The hook is fail-open by construction (main() is wrapped in `except: pass`), which
# means a bug in it is INVISIBLE at runtime — nothing logs, the pane just silently stops reporting.
# That is exactly why the logic needs a check that can fail loudly here.
import importlib.util
import io
import json
import os
import sys
import tempfile

HOOK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "assets", "castellyn_status.py")
_spec = importlib.util.spec_from_file_location("castellyn_status", HOOK)
# assert rather than ignore: a missing spec/loader means the hook file moved, and a confusing
# AttributeError three lines later is a worse way to find that out than saying it here.
assert _spec is not None and _spec.loader is not None, f"cannot load the hook from {HOOK}"
hook = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hook)


def run(payload, sid="abc123", raw=None):
    """Drive main() end to end and return the status file it wrote (or None)."""
    with tempfile.TemporaryDirectory() as tmp:
        old_env, old_stdin = dict(os.environ), sys.stdin
        try:
            os.environ["CASTELLYN_SESSION_ID"] = sid
            os.environ["APPDATA"] = tmp
            sys.stdin = io.StringIO(raw if raw is not None else json.dumps(payload))
            hook.main()
        finally:
            os.environ.clear()
            os.environ.update(old_env)
            sys.stdin = old_stdin
        fp = os.path.join(tmp, "castellyn", "agent-status", sid + ".json")
        if not os.path.exists(fp):
            return None
        with open(fp, encoding="utf-8") as f:
            return json.load(f)


# --- excerpt: the only path payload text takes to disk -------------------------------------------
assert hook.excerpt("hello") == "hello"
assert hook.excerpt("  a\n\tb  ") == "a b", "whitespace collapses to one line"
assert hook.excerpt("\x1b[31mred\x1b[0m") == "[31mred [0m", "ANSI escapes are stripped, not escaped"
assert "\x00" not in hook.excerpt("a\x00b\x7fc") and hook.excerpt("a\x00b\x7fc") == "a b c"
assert len(hook.excerpt("x" * 500)) == hook.ASK_CAP and hook.excerpt("x" * 500).endswith("…")
assert len(hook.excerpt("x" * 500, hook.CWD_CAP)) == hook.CWD_CAP, "the cap is per-call"
for junk in (None, 42, [], {}, True, object()):
    assert hook.excerpt(junk) == "", f"non-string {junk!r} must collapse to empty"

# --- describe: total over every JSON shape -------------------------------------------------------
assert hook.describe({"tool_input": {"questions": [{"question": "Which approach?"}]}}, "PreToolUse", "AskUserQuestion") == ("question", "Which approach?")
# The shapes that used to raise AttributeError and take the whole status write down with them.
for bad in ("a string", 42, ["a", "list"], None, True, {"questions": "not a list"}, {"questions": []}, {"questions": [None]}, {"questions": ["str"]}, {"questions": [{}]}, {"questions": [{"question": 7}]}, {}):
    assert hook.describe({"tool_input": bad}, "PreToolUse", "AskUserQuestion") == ("question", ""), f"tool_input={bad!r}"
assert hook.describe({}, "PreToolUse", "AskUserQuestion") == ("question", ""), "missing tool_input"
assert hook.describe({"tool_input": {"questions": [{"question": "q" * 400}]}}, "PreToolUse", "AskUserQuestion")[1].endswith("…")
assert hook.describe({}, "PermissionRequest", "Bash") == ("permission", "Bash")
assert hook.describe({}, "PermissionRequest", "") == ("permission", "")
assert hook.describe({}, "Notification", "") == ("notification", "")
# The allow-list holds: a command line in tool_input never reaches the excerpt.
leak = hook.describe({"tool_input": {"command": "curl http://x?token=SECRET"}}, "PermissionRequest", "Bash")
assert "SECRET" not in leak[1]

# --- is_subagent: fails SAFE (ambiguous -> main thread -> still notifies) -------------------------
assert hook.is_subagent({"agent_id": "agent_01"}) is True
for ambiguous in ({}, {"agent_id": ""}, {"agent_id": None}, {"agent_id": 0}, {"agent_id": 1}, {"agent_id": True}, {"agent_id": {}}, {"agent_id": []}):
    assert hook.is_subagent(ambiguous) is False, f"{ambiguous!r} must read as the main thread"

# --- the subagent guard covers EVERY blocked-producing path ---------------------------------------
BLOCKING = [
    {"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "tool_input": {"questions": [{"question": "Which?"}]}},
    {"hook_event_name": "Notification"},
    {"hook_event_name": "PermissionRequest", "tool_name": "Bash"},
]
for p in BLOCKING:
    main_out = run(p)
    assert main_out is not None and main_out["state"] == "blocked", f"main thread must block on {p['hook_event_name']}"
    sub = run(dict(p, agent_id="agent_01"))
    assert sub is not None
    assert sub["state"] == "working", f"a subagent must not block the parent pane on {p['hook_event_name']}"
    assert "ask" not in sub and "reason" not in sub, "only a blocked pane carries enrichment"

# --- the state is written even when the enrichment cannot be ---------------------------------------
for broken in ("not json at all", "", "[1,2,3]", '"a bare string"', "null", "123"):
    out = run(None, raw=broken)
    assert out is None, "a payload with no event maps to no state at all"
out = run({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "tool_input": "malformed"})
assert out is not None and out["state"] == "blocked" and out["reason"] == "question" and out["ask"] == ""
out = run({"hook_event_name": ["unhashable"], "tool_name": {"also": "unhashable"}})
assert out is None, "a non-string event is not a state, and must not raise"
out = run({"hook_event_name": "Stop", "tool_name": 42})
assert out is not None and out["state"] == "idle"

# --- cwd is sanitised on the same path as ask ------------------------------------------------------
out = run({"hook_event_name": "Notification", "cwd": "E:\\Scripts\\Cas\x1b[31mtellyn\n"})
assert out is not None and out["cwd"] == "E:\\Scripts\\Cas [31mtellyn", "cwd goes through excerpt()"
assert run({"hook_event_name": "Notification", "cwd": 42})["cwd"] == ""
assert len(run({"hook_event_name": "Notification", "cwd": "C:\\" + "d" * 600})["cwd"]) == hook.CWD_CAP

# --- the env gate: sessions outside Castellyn stay untouched ---------------------------------------
assert run({"hook_event_name": "Stop"}, sid="") is None
assert run({"hook_event_name": "Stop"}, sid="../escape") is None, "sid is path-safe by construction"
assert run({"hook_event_name": "Stop"}, sid="a" * 33) is None

print("ok — status hook self-test passed")
