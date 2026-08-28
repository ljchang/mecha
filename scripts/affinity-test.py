#!/usr/bin/env python3
"""Does one long-lived conversation keep its slot?

Reproduces the failure the cache lens caught: a multi-turn conversation with a
large, stable prefix. Slot affinity is invisible in a throughput benchmark —
bench-slots.sh sends independent prompts, which is exactly the workload that
CANNOT show this — so it needs its own test.
"""
import json, sys, urllib.request, time

BIG = open("/home/ljchang/Github/mecha/docs/ARCHITECTURE.md").read()[:60000]
msgs = [{"role": "user", "content":
         "Here is a document I want to discuss:\n\n" + BIG +
         "\n\nReply with one short sentence acknowledging it."}]
for turn in range(1, 13):
    req = urllib.request.Request(
        "http://127.0.0.1:8080/v1/chat/completions",
        data=json.dumps({"model": "qwen3.6-35b-a3b", "messages": msgs,
                         "max_tokens": 8192}).encode(),
        headers={"Content-Type": "application/json"})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=900) as r:
        b = json.load(r)
    dt = time.time() - t0
    m = b["choices"][0]["message"]
    u = b.get("usage", {})
    print(f"  turn {turn:2d}: {dt:6.1f}s  prompt_tokens={u.get('prompt_tokens')}")
    msgs.append({"role": "assistant", "content": m.get("content") or "(ok)"})
    msgs.append({"role": "user", "content": f"Question {turn}: name one design decision and why."})
