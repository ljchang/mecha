#!/usr/bin/env python3
"""A deliberately nosy MCP server, used as a test fixture.

It reports what it can see — its whole environment, its uid, whether it can
reach the network, whether a home directory is in front of it — so that mecha's
claims about confining a server can be *measured through a real handshake*
rather than asserted about an argv it never spawned.

Speaks the newline-delimited JSON-RPC dialect mecha's client uses. Never import
this into anything: seeing everything is the point.
"""

import json
import os
import socket
import sys

TOOLS = [
    {
        "name": "environ",
        "description": "Every environment variable this server can see.",
        "inputSchema": {"type": "object", "properties": {}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "probe",
        "description": "What this server can see of the machine it runs on.",
        "inputSchema": {"type": "object", "properties": {}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "touch",
        "description": "Write a file into the working directory.",
        "inputSchema": {
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
        },
        "annotations": {"destructiveHint": True},
    },
]


def environ_text():
    return "\n".join(f"{k}={v}" for k, v in sorted(os.environ.items()))


def probe_text():
    home = os.environ.get("HOME", "")
    # A TCP connect to a literal address rather than a DNS lookup: getaddrinfo
    # ignores the socket timeout and can hang for the resolver's own, which
    # would make "no network" look like a wedged test.
    try:
        socket.create_connection(("1.1.1.1", 53), timeout=2).close()
        network = True
    except OSError:
        network = False

    return json.dumps(
        {
            "uid": os.getuid(),
            "cwd": os.getcwd(),
            "hostname": socket.gethostname(),
            "home": home,
            "home_ssh_exists": bool(home) and os.path.isdir(os.path.join(home, ".ssh")),
            "network": network,
        }
    )


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def reply(request_id, result):
    # The id is echoed back as a *string* on purpose: it is a dialect real
    # servers speak, and answering this way makes every test in the suite
    # prove the client routes it. A client that only matches numeric ids
    # times out against this fixture on the very first handshake.
    send({"jsonrpc": "2.0", "id": str(request_id), "result": result})


def fail(request_id, message):
    send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": message}})


def call_tool(name, arguments):
    if name == "environ":
        return environ_text()
    if name == "probe":
        return probe_text()
    if name == "touch":
        path = os.path.join(os.getcwd(), arguments.get("name", "probe.txt"))
        with open(path, "w") as handle:
            handle.write("written by the MCP server\n")
        return path
    return None


def main():
    while True:
        line = sys.stdin.readline()
        if not line:  # stdin closed: the client went away.
            return
        line = line.strip()
        if not line:
            continue

        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue

        request_id = message.get("id")
        method = message.get("method")
        if request_id is None:
            continue  # A notification; nothing to answer.

        if method == "initialize":
            reply(
                request_id,
                {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "nosy", "version": "0"},
                },
            )
        elif method == "tools/list":
            # One tool per page: a client that does not follow nextCursor
            # sees a third of the surface, and the handshake test's
            # assertions on all three tools catch it.
            params = message.get("params") or {}
            start = int(params.get("cursor") or 0)
            page = {"tools": TOOLS[start : start + 1]}
            if start + 1 < len(TOOLS):
                page["nextCursor"] = str(start + 1)
            reply(request_id, page)
        elif method == "tools/call":
            params = message.get("params") or {}
            text = call_tool(params.get("name"), params.get("arguments") or {})
            if text is None:
                fail(request_id, "no such tool: {}".format(params.get("name")))
            else:
                reply(request_id, {"content": [{"type": "text", "text": text}]})
        else:
            fail(request_id, "unsupported method: {}".format(method))


if __name__ == "__main__":
    main()
