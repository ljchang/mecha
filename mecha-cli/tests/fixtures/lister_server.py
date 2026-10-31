#!/usr/bin/env python3
"""An MCP server that lists the tools in a JSON file and does nothing else.

`argv[1]` names a file holding a `tools/list` array, exactly as a server
would send it (names, descriptions, schemas, annotations). Used by
`effect_class.rs` to put the real first-party definitions, and made-up
third-party ones, through mecha's real MCP client. A call answers an error:
nothing here is ever meant to run. Stdlib only.
"""

import json
import sys


def main():
    with open(sys.argv[1]) as f:
        tools = json.load(f)
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        if "id" not in msg:
            continue
        method = msg.get("method")
        if method == "initialize":
            result = {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "lister", "version": "0"},
            }
        elif method == "tools/list":
            result = {"tools": tools}
        elif method == "tools/call":
            result = {"content": [{"type": "text", "text": "not run"}], "isError": True}
        else:
            result = {}
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
