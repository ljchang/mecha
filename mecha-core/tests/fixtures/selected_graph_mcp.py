#!/usr/bin/env python3
"""Authored MCP graph fixture: no graph, network or private store access."""
import json
import os
import sys

mode = os.environ.get("GRAPH_FIXTURE_MODE", "ok")
marker = os.environ.get("GRAPH_FIXTURE_MARKER")
if marker:
    with open(marker, "a", encoding="utf-8") as stream:
        stream.write("spawn\n")


def tool(name):
    result = {"name": name, "description": "Authored graph snapshot", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}, "annotations": {"readOnlyHint": True}}
    if mode == "schema":
        result["inputSchema"] = []
    if mode == "nested_schema":
        result["inputSchema"]["properties"]["query"] = {"type": 7}
    if mode == "bad_annotations":
        result["annotations"]["openWorldHint"] = "true"
    if mode == "bad_background":
        result["_meta"] = {"mecha-factory.ai/background": "true"}
    if mode == "description":
        result["description"] = "Changed authored description"
    if mode in ("open", "destructive"):
        result["annotations"]["openWorldHint" if mode == "open" else "destructiveHint"] = True
    if mode == "write":
        result["annotations"]["readOnlyHint"] = False
    if mode == "deferred":
        result["_meta"] = {"mecha-factory.ai/background": True}
    return result


for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request["method"]
    if method == "initialize":
        result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "authored", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [tool("kg_search"), tool("kg_entity"), tool("kg_task_list"), tool("kg_upsert")]}
        if mode == "missing":
            result["tools"] = [tool("kg_upsert")]
        if mode == "duplicate":
            result["tools"].append(tool("kg_search"))
        if mode == "duplicate_page":
            result = {"tools": [tool("kg_search")]}
            if "cursor" not in request.get("params", {}):
                result["nextCursor"] = "second"
        if mode == "forever":
            result = {"tools": [], "nextCursor": "again"}
        if mode == "bad_cursor":
            result["nextCursor"] = 4
    elif method == "tools/call":
        action = request.get("params", {}).get("arguments", {}).get("mode", "ok")
        if action == "exit":
            sys.exit(0)
        if action == "wire_error":
            print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": "authored wire error"}}), flush=True)
            continue
        result = {"content": [{"type": "text", "text": "Authored snapshot: project Cedar is active."}]}
        if action == "image":
            result["content"] = [{"type": "image", "data": "AA==", "mimeType": "image/png"}]
        if action == "invalid":
            result["content"] = [{"type": "text", "text": 3}]
        if action == "empty":
            result["content"] = []
        if action == "large":
            result["content"] = [{"type": "text", "text": "x" * 20000}]
        if action == "env":
            result["content"] = [{"type": "text", "text": json.dumps({key: os.environ.get(key) for key in ["GRAPH_TEST_EXPLICIT", "MECHA_TZ", "ANTHROPIC_API_KEY"]})}]
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
