#!/usr/bin/env python3
"""Stub `vibe-app-server` for the CI e2e run.

CI has no Mistral Vibe CLI and no API key, so the real GUI would sit on the
setup/login gate forever and the shell specs could not run. This stub speaks
just enough NDJSON JSON-RPC 2.0 (docs/appserver-protocol.md) for the client
to reach the normal shell: the initialize handshake + `initialized`
notification, `setup/status` reporting a stored key, an empty
`session/list` and a generic `{state: null, lastEventId: 0}` for anything
else. Unknown methods answer `-32601 method_not_found`, which is exactly how
a real CLI without an optional feature behaves (the UI degrades, it does not
break).
"""

import json
import sys

SERVER_INFO = {"name": "vibe-app-server-e2e-stub", "version": "0.0.0-e2e"}

SETUP_STATUS = {
    "provider": None,
    "consoleBaseUrl": "",
    "vibeBaseUrl": "",
    "supportsBrowserSignIn": False,
    "hasApiKey": True,
    "pinnedConsoleUrl": None,
}

# Methods the stub pretends not to know (feature detection paths).
UNKNOWN = {"account/read", "identity/read", "runtime/read", "agents/list"}


def respond(rid, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": rid}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result if result is not None else {}
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        rid = msg.get("id")
        method = msg.get("method")
        if rid is None:
            # notification (e.g. `initialized`) — nothing to answer
            continue
        if method == "initialize":
            respond(
                rid,
                {
                    "serverInfo": SERVER_INFO,
                    "capabilities": {},
                    "protocolVersion": 1,
                },
            )
        elif method == "setup/status":
            respond(rid, SETUP_STATUS)
        elif method == "session/list":
            respond(rid, {"sessions": []})
        elif method in UNKNOWN:
            respond(rid, error={"code": -32601, "message": f"{method}: not found"})
        else:
            # session/start, session/resume, turn/*, config/*, setup/* …
            respond(rid, {"state": None, "lastEventId": 0})


if __name__ == "__main__":
    main()
