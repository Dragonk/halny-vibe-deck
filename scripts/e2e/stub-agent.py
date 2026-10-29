#!/usr/bin/env python3
"""Scripted `vibe-app-server` stand-in for the CI e2e suite.

Speaks NDJSON JSON-RPC 2.0 as documented in docs/appserver-protocol.md, at
the level the GUI actually consumes:

  initialize / initialized      handshake
  setup/status                  hasApiKey=true (skips the login wizard)
  session/list                  session summaries
  session/start                 a new session + full PublicSessionState
  session/resume               full PublicSessionState (backfill source!)
  session/turn/enqueue         begins a scripted turn
  session/turn/interrupt       cancels the running turn
  callback/result              approval decision from the UI

A turn is scripted by the prompt text:

  "@gap"        emit the first assistant chunk, then keep the rest ONLY in
                the session history (simulates a client that missed the
                stream). A `session/resume` returns everything, so a UI that
                backfills on visibility-resync shows the missing text without
                a page reload — the stable-edition bug we must not repeat.
  "@approval"   ask for a tool approval mid-turn (callback/call) and continue
                only after callback/result.
  anything else stream 4 chunks with 250 ms gaps, then complete the turn.
"""

import json
import os
import sys
import threading
import time

LOCK = threading.Lock()
TRACE = os.environ.get("VD_STUB_TRACE", "")


def trace(line):
    if not TRACE:
        return
    try:
        with open(TRACE, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")
    except OSError:
        pass
SESSIONS = {}
STATE = {"counter": 0}


def now_ms():
    return int(time.time() * 1000)


def send(msg):
    trace("out " + (msg.get("method") or ("response " + str(msg.get("id")))))
    with LOCK:
        sys.stdout.write(json.dumps(msg) + "\n")
        sys.stdout.flush()


def respond(rid, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": rid}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result if result is not None else {}
    send(msg)


def notify(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})


def new_session(cwd):
    STATE["counter"] += 1
    sid = f"s{STATE['counter']}"
    SESSIONS[sid] = {
        "session": {
            "id": sid,
            "title": f"e2e session {STATE['counter']}",
            "status": "idle",
            "cwd": cwd or "/tmp",
            "model": "mistral-medium-3",
            "reasoningEffort": "high",
            "agent": "default",
        },
        "history": [],
        "turns": [],
        "eventId": 0,
        "current_turn": None,
    }
    return sid


def state_of(sid):
    s = SESSIONS[sid]
    return {
        "eventId": s["eventId"],
        "session": s["session"],
        "history": s["history"],
        "turns": s["turns"],
        "isQuiescent": s["current_turn"] is None,
        "activeCallbacks": [],
    }


def bump(sid):
    SESSIONS[sid]["eventId"] += 1
    return SESSIONS[sid]["eventId"]


def push_entry(sid, entry):
    entry.setdefault("id", f"e{bump(sid)}")
    SESSIONS[sid]["history"].append(entry)
    notify("history/entryAdded", {"sessionId": sid, "entry": entry})
    return entry


def patch_entry_text(sid, entry_id, text):
    """Replace an entry's text (json-patch on the absolute history path)."""
    s = SESSIONS[sid]
    idx = next(i for i, e in enumerate(s["history"]) if e["id"] == entry_id)
    s["history"][idx]["content"] = [{"type": "text", "text": text}]
    notify(
        "history/entryUpdated",
        {
            "sessionId": sid,
            "entryId": entry_id,
            "patch": [
                {
                    "op": "replace",
                    "path": f"/history/{idx}/content",
                    "value": [{"type": "text", "text": text}],
                }
            ],
        },
    )


def run_turn(sid, text, approval_gate):
    s = SESSIONS[sid]
    turn = {"id": f"t{bump(sid)}", "status": "running", "stopReason": None}
    s["current_turn"] = turn["id"]
    s["turns"].append(turn)
    s["session"]["status"] = "running"
    notify("turn/started", {"sessionId": sid, "turn": turn})

    entry = push_entry(
        sid,
        {
            "type": "message",
            "role": "assistant",
            "turnId": turn["id"],
            "content": [{"type": "text", "text": ""}],
        },
    )
    eid = entry["id"]

    chunks = ["Working on it. ", "Step 1 done. ", "Step 2 done. ", "All done."]
    if "@gap" in text:
        # First chunk only; the rest stays in the history so only a
        # state refetch (session/resume) can recover it.
        patch_entry_text(sid, eid, chunks[0])
        final = "".join(chunks)
        SESSIONS[sid]["history"][-1]["content"] = [{"type": "text", "text": final}]
        SESSIONS[sid]["session"]["status"] = "idle"
        SESSIONS[sid]["current_turn"] = None
        turn["status"] = "completed"
        turn["stopReason"] = "completed"
        notify("turn/completed", {"sessionId": sid, "turn": turn})
        return

    acc = ""
    for i, chunk in enumerate(chunks):
        acc += chunk
        patch_entry_text(sid, eid, acc)
        if i == 1 and "@approval" in text:
            gate = threading.Event()
            approval_gate["event"] = gate
            approval_gate["sid"] = sid
            send(
                {
                    "jsonrpc": "2.0",
                    "id": 900,
                    "method": "callback/call",
                    "params": {
                        "sessionId": sid,
                        "callback": {
                            "callbackId": "cb1",
                            "kind": "approval",
                            "detail": {
                                "type": "approval",
                                "toolName": "shell",
                                "command": "echo e2e-approval",
                            },
                        },
                    },
                }
            )
            gate.wait(timeout=30)
        time.sleep(0.25)

    SESSIONS[sid]["session"]["status"] = "idle"
    SESSIONS[sid]["current_turn"] = None
    turn["status"] = "completed"
    turn["stopReason"] = "completed"
    notify("turn/completed", {"sessionId": sid, "turn": turn})


def handle(msg):
    rid = msg.get("id")
    method = msg.get("method")
    trace("in " + str(method))
    params = msg.get("params") or {}
    if rid is None:
        return  # notification
    if method == "initialize":
        respond(
            rid,
            {
                "serverInfo": {"name": "vibe-app-server-e2e", "version": "0.0.0-e2e"},
                "capabilities": {},
                "protocolVersion": 1,
            },
        )
    elif method == "setup/status":
        respond(
            rid,
            {
                "provider": None,
                "consoleBaseUrl": "",
                "vibeBaseUrl": "",
                "supportsBrowserSignIn": False,
                "hasApiKey": True,
                "pinnedConsoleUrl": None,
            },
        )
    elif method == "session/list":
        respond(
            rid,
            {
                "sessions": [
                    {
                        "id": sid,
                        "title": s["session"]["title"],
                        "cwd": s["session"]["cwd"],
                        "updatedAt": now_ms(),
                    }
                    for sid, s in SESSIONS.items()
                ]
            },
        )
    elif method == "session/start":
        cfg = params.get("agentConfig") or {}
        sid = new_session(cfg.get("cwd"))
        respond(rid, {"state": state_of(sid), "lastEventId": SESSIONS[sid]["eventId"]})
    elif method == "session/resume":
        sid = params.get("sessionId")
        if sid in SESSIONS:
            respond(rid, {"state": state_of(sid), "lastEventId": SESSIONS[sid]["eventId"]})
        else:
            respond(rid, error={"code": -32602, "message": f"unknown session {sid}"})
    elif method == "session/turn/enqueue":
        sid = params.get("sessionId")
        entries = params.get("entries") or []
        text = ""
        for e in entries:
            for block in e.get("content") or []:
                if block.get("type") == "text":
                    text += block.get("text", "")
        respond(rid, {})
        if sid in SESSIONS:
            threading.Thread(
                target=run_turn,
                args=(sid, text, APPROVAL),
                daemon=True,
            ).start()
    elif method == "session/turn/interrupt":
        sid = params.get("sessionId")
        s = SESSIONS.get(sid)
        if s and s["turns"]:
            turn = s["turns"][-1]
            turn["status"] = "cancelled"
            turn["stopReason"] = "cancelled"
            s["current_turn"] = None
            s["session"]["status"] = "idle"
            respond(rid, {})
            notify("turn/completed", {"sessionId": sid, "turn": turn})
        else:
            respond(rid, {})
    elif method == "callback/result":
        gate = APPROVAL.get("event")
        if gate:
            gate.set()
        respond(rid, {})
    elif method in ("account/read", "identity/read", "runtime/read", "agents/list"):
        respond(rid, error={"code": -32601, "message": f"{method}: not found"})
    else:
        respond(rid, {"state": None, "lastEventId": 0})


APPROVAL = {}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        try:
            handle(msg)
        except Exception as exc:  # never die on a malformed request
            rid = msg.get("id")
            if rid is not None:
                respond(rid, error={"code": -32603, "message": str(exc)})


if __name__ == "__main__":
    main()
