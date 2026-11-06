#!/usr/bin/env python3
"""Fictional mailbox: exercises classify completion without a model/socket."""
import json
import sys

for line in sys.stdin:
    message = json.loads(line)
    if "id" not in message:
        continue
    method = message.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                  "serverInfo": {"name": "fictional-mail", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "mail_recent", "description": "Fictional empty inbox",
                             "inputSchema": {"type": "object", "properties": {}}}]}
    elif method == "tools/call":
        rows = []
        if "--bulk" in sys.argv:
            rows = [{"thread_id": "fictional-thread", "account": "fictional",
                     "from": "robot@example.com", "subject": "Fictional notice",
                     "date": "2026-11-06T12:00:00Z", "snippet": "Fictional bulk notice", "bulk": True}]
        result = {"content": [{"type": "text", "text": json.dumps(rows)}]}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}), flush=True)
