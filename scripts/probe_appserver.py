#!/usr/bin/env python3
"""F0 protocol probe for vibe-app-server.

Spawn vibe-app-server, initialize, session/start, turn/enqueue, listen
for notifications, ack server callbacks. Prints the full lifecycle:
turn/started -> history/entryAdded -> entryUpdated -> turn/completed.

Usage: python3 scripts/probe_appserver.py [prompt]
Reference: docs/appserver-protocol.md
"""
import json
import subprocess
import sys
import tempfile
import threading
import time
import os

PROMPT = sys.argv[1] if len(sys.argv) > 1 else "Say exactly: ok"
TIMEOUT = float(os.environ.get("PROBE_TIMEOUT", "120"))

proc = subprocess.Popen(
    ["vibe-app-server"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
    bufsize=1,
)

seen = []
state = {"session_id": None, "initialized": False, "completed": False}


def send(msg):
    line = json.dumps(msg, separators=(",", ":"))
    try:
        proc.stdin.write(line + "\n")
        proc.stdin.flush()
    except BrokenPipeError:
        pass


def ack_server_request(m):
    params = m.get("params") or {}
    cb = params.get("callback") or {}
    send({
        "jsonrpc": "2.0",
        "id": m["id"],
        "result": {"callbackId": cb.get("callbackId"), "accepted": True},
    })


def reader():
    for line in proc.stdout:
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if "method" in m:
            seen.append(m["method"])
            params = m.get("params") or {}
            print("<<", m["method"], json.dumps(params)[:280], flush=True)
            if m["method"] == "session/snapshot":
                st = params.get("state") or {}
                state["session_id"] = (st.get("session") or {}).get("id")
            if "id" in m:
                ack_server_request(m)
        else:
            rid = m.get("id")
            if rid == "c1":
                state["initialized"] = True
                print("<< initialize:", json.dumps(m.get("result"))[:200], flush=True)
            elif rid == "c2":
                res = m.get("result") or {}
                st = res.get("state") or {}
                state["session_id"] = (st.get("session") or {}).get("id")
                print("<< session/start:", state["session_id"], flush=True)
            elif "error" in m:
                print("<< ERROR:", json.dumps(m["error"])[:300], flush=True)


threading.Thread(target=reader, daemon=True).start()

send({
    "jsonrpc": "2.0", "id": "c1", "method": "initialize",
    "params": {
        "clientInfo": {"name": "probe", "version": "0.1"},
        "capabilities": {"callbackKinds": ["approval", "user_input"]},
    },
})
time.sleep(1.0)
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
time.sleep(0.3)
cwd = tempfile.mkdtemp(prefix="probe-appsrv-")
send({
    "jsonrpc": "2.0", "id": "c2", "method": "session/start",
    "params": {"agentConfig": {"cwd": cwd}, "kind": "normal"},
})

deadline = time.time() + TIMEOUT
enqueued = False
while time.time() < deadline:
    if state["session_id"] and state["initialized"] and not enqueued:
        enqueued = True
        send({
            "jsonrpc": "2.0", "id": "c3", "method": "session/turn/enqueue",
            "params": {
                "sessionId": state["session_id"],
                "entries": [{"role": "user",
                             "content": [{"type": "text", "text": PROMPT}]}],
            },
        })
    if "turn/completed" in seen:
        state["completed"] = True
        break
    time.sleep(0.2)

try:
    proc.terminate()
    proc.wait(timeout=5)
except Exception:
    proc.kill()

print("methods seen:", sorted(set(seen)))
ok = state["initialized"] and state["session_id"] and "turn/completed" in seen
print("PROBE", "OK" if ok else "INCOMPLETE")
sys.exit(0 if ok else 1)
