#!/usr/bin/env python3
"""Read-only frozen retrieval; executes actual MCP searches, never replays calls."""
import argparse
import collections
import json
import math
import re
import sys
from pathlib import Path

STOP = set("what should i know about the a an and or to of in on for is are was were with from my your sources evidence show does did as it its this that at has have had".split())


def words(text):
    return set(re.findall(r"[a-z0-9]+", text.lower())) - STOP


def search(case, args):
    if (args.get("scope") != "evidence_only" or args.get("probe") is not True
            or args.get("include_private") is not True
            or args.get("sources") not in (["slack"], ["bee"])
            or args.get("since") != "2026-01-01" or args.get("until") != "2026-09-10"
            or not args.get("query", "").startswith(case["entity"]["name"] + ": ")):
        raise ValueError("source, target, probe or window boundary violated")
    # A two-hit retrieval cap forces selection; no call history, gold, arm or
    # question generator affects ranking. Query terms drive actual retrieval.
    query = words(args["query"]) - words(case["entity"]["name"])
    docs = [e for e in case["episodes"] if e["source"] in args["sources"]]
    counts = collections.Counter(w for e in docs for w in words(e["text"]))
    def score(e):
        return sum(math.log(1 + len(docs) / counts[w]) for w in sorted(query & words(e["text"])))
    ranked = sorted(docs, key=lambda e: -score(e))
    k = max(1, min(2, int(args.get("k", 10))))
    return {"items": ranked[:k], "fixture_result_cap": 2}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--cases", required=True)
    p.add_argument("--case", required=True)
    p.add_argument("--log", required=True)
    args = p.parse_args()
    case = next(c for c in json.loads(Path(args.cases).read_text()) if c["id"] == args.case)
    for line in sys.stdin:
        req = json.loads(line)
        if "id" not in req:
            continue
        error = False
        if req["method"] == "initialize":
            result = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "gossip-fixture", "version": "1"}}
        elif req["method"] == "tools/list":
            result = {"tools": [{"name": "kg_search", "description": "Search source evidence",
                                "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": True}}]}
        elif req["method"] == "tools/call":
            try:
                if req["params"]["name"] != "kg_search":
                    raise ValueError("fixture permits only kg_search")
                body = search(case, req["params"]["arguments"])
            except (ValueError, KeyError, TypeError) as exc:
                error, body = True, {"error": str(exc)}
            with open(args.log, "a") as log:
                log.write(json.dumps({"request": req["params"], "result": body, "error": error}) + "\n")
            result = {"content": [{"type": "text", "text": json.dumps(body)}], "isError": error}
        else:
            print(json.dumps({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "unknown method"}}), flush=True)
            continue
        print(json.dumps({"jsonrpc": "2.0", "id": req["id"], "result": result}), flush=True)


if __name__ == "__main__":
    main()
