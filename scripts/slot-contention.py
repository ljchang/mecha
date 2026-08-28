#!/usr/bin/env python3
"""Do N long-lived conversations keep their slots, or evict each other?

`affinity-test.py`'s question at the axis R1 actually needs: not "does one
conversation keep its slot" but "how many conversations can share four slots
before they start re-prefilling each other". A throughput benchmark cannot
see this — it sends independent prompts, the one workload with no prefix to
lose — and neither can the single-conversation test, which never contends.

The metric is the server's own `prompt eval time = ... / N tokens`: small N
means the prefix was reused and only the new turn was paid for; N in the tens
of thousands means the whole transcript was re-prefilled.
"""
import json, sys, threading, time, urllib.request

BIG = open("/home/ljchang/Github/mecha/docs/ARCHITECTURE.md").read()[:40000]
K = int(sys.argv[1])
TURNS = int(sys.argv[2]) if len(sys.argv) > 2 else 6

def convo(i, out):
    msgs = [{"role": "user", "content":
             f"Conversation {i}. Here is a document:\n\n{BIG}\n\nReply with one short sentence."}]
    for turn in range(1, TURNS + 1):
        req = urllib.request.Request(
            "http://127.0.0.1:8080/v1/chat/completions",
            data=json.dumps({"model": "qwen3.6-35b-a3b", "messages": msgs,
                             "max_tokens": 512}).encode(),
            headers={"Content-Type": "application/json"})
        t0 = time.time()
        try:
            with urllib.request.urlopen(req, timeout=900) as r:
                b = json.load(r)
        except Exception as e:
            out.append((i, turn, -1, str(e)[:40])); return
        dt = time.time() - t0
        u = b.get("usage", {})
        out.append((i, turn, dt, u.get("prompt_tokens")))
        msgs.append({"role": "assistant",
                     "content": b["choices"][0]["message"].get("content") or "(ok)"})
        msgs.append({"role": "user",
                     "content": f"Question {turn}: name one design decision and why, in one sentence."})

out, threads = [], []
t0 = time.time()
for i in range(K):
    t = threading.Thread(target=convo, args=(i, out)); t.start(); threads.append(t)
for t in threads: t.join()
wall = time.time() - t0
print(f"K={K} turns={TURNS} wall={wall:.1f}s requests={len(out)}")
