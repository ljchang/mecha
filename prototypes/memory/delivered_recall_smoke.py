#!/usr/bin/env python3
"""End to end: a `mecha exp` lifetime whose harness recalls for each task.

    delivered_recall_smoke.py <mecha binary> <empty scratch directory> <checkout>

Two tasks in one home per arm, against a scripted provider that answers in
one word and calls no tool. Asserted, for the `delivered` arm (the store) and
the `transcripts` arm (the home's session files):

- the first request of every task carries a delivery after the owner's words;
- the second task's delivery holds the first task's prompt, so what was
  delivered was found in what the first task left behind;
- in the `delivered` arm both runs are retained (two seals): the recorder
  reconciled the harness's recall with its tracked attempt. Before
  `Walk::delivery` a delivered run was refused as incomplete and left none;
- a delivery is never stored as a passage: the second task's packet does not
  hold the first task's delivery line;
- the `none` arm's requests carry no delivery.

Needs a binary built with the `memory-prototype` feature.
"""
import glob
import http.server
import json
import os
import subprocess
import sys
import threading
from pathlib import Path

BINARY, ROOT, CHECKOUT = sys.argv[1], Path(sys.argv[2]).resolve(), sys.argv[3]
ROOT.mkdir(parents=True)
REQUESTS = []
MEMORY_STEM = "The harness searched your memory of earlier work"
TRANSCRIPT_STEM = "The harness searched the transcripts of your earlier sessions"


class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        REQUESTS.append(body)
        msg = {"role": "assistant", "content": "goodbye" if "goodbye" in json.dumps(body["messages"][0:2]) else "hello"}
        usage = {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}
        if body.get("stream"):
            chunk = {"id": "f", "model": body["model"], "choices": [{"index": 0, "delta": msg, "finish_reason": "stop"}], "usage": usage}
            data = ("data: " + json.dumps(chunk) + "\n\ndata: [DONE]\n\n").encode()
            kind = "text/event-stream"
        else:
            data = json.dumps({"model": body["model"], "choices": [{"message": msg, "finish_reason": "stop"}], "usage": usage}).encode()
            kind = "application/json"
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
home, mecha = ROOT / "home", ROOT / "mecha"
home.mkdir()
mecha.mkdir(mode=0o700)
(mecha / "config.toml").write_text(
    f"""default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
model = "fixture"
base_url = "http://127.0.0.1:{server.server_port}"
max_retries = 0
[tools]
enabled = ["goal_context", "fs_read"]
permission_mode = "read-only"
"""
)
arm = """[arms.{name}]
preset = "bare"
memory = "{name}"
[arms.{name}.prediction]
metric = "failure"
rationale = "Plumbing check, not a measurement."
"""
manifest = ROOT / "delivered-smoke.toml"
manifest.write_text(
    f"""name = "delivered-smoke"
kind = "lifetime"
description = "Two tasks; the harness recalls for each."
control = "none"
split_seed = 17
seeds = [1]
repetitions = 1
[schedule]
reflect = 0
validate = 0
learn = 0
retire = 0
ruminate = 0
[tasks]
source = ["python3", "{CHECKOUT}/eval/fixtures/source_stub.py"]
fixture = "{CHECKOUT}/eval/workspace"
ids = ["say-hello", "say-goodbye"]
[tasks.confirmed_goals]
say-hello = "task:say-hello"
say-goodbye = "task:say-goodbye"
[arms.none]
preset = "bare"
"""
    + arm.format(name="delivered")
    + arm.format(name="transcripts")
)
env = {"HOME": str(home), "MECHA_HOME": str(mecha), "PATH": os.environ["PATH"], "LANG": "C.UTF-8", "TERM": "dumb"}


def run(*args):
    p = subprocess.run([BINARY, *args], cwd=CHECKOUT, env=env, capture_output=True, text=True, timeout=300)
    assert p.returncode == 0, f"mecha {' '.join(args)} -> {p.returncode}\n{p.stdout[-1500:]}\n{p.stderr[-2500:]}"
    return p.stdout


def owner_message(request):
    """The first user message's text parts, in order."""
    first = next(m for m in request["messages"] if m["role"] == "user")
    content = first["content"]
    return [content] if isinstance(content, str) else [part.get("text", "") for part in content]


def deliveries(stem):
    found = []
    for request in REQUESTS:
        parts = owner_message(request)
        whole = "\n".join(parts)
        if stem in whole:
            # After the owner's words, never before them.
            assert not parts[0].startswith(stem), parts[0][:200]
            found.append(whole[whole.index(stem):])
    return found


run("exp", "new", str(manifest))
run("exp", "run", "delivered-smoke")
status = run("exp", "status", "delivered-smoke")
experiment = mecha / "experiments" / "delivered-smoke"
rows = [json.load(open(p)) for p in glob.glob(str(experiment / "trials" / "*.json"))]
assert len(rows) == 6 and all(r["status"] == "done" for r in rows), status

from_store, from_files = deliveries(MEMORY_STEM), deliveries(TRANSCRIPT_STEM)
assert len(from_store) == 2 and len(from_files) == 2, (len(from_store), len(from_files))
assert len(REQUESTS) == 6, len(REQUESTS)

# What the first task left is what the second is handed.
# (A provider that takes one string per message gets the blocks joined, so
# the owner's words are what precedes the delivery line.)
first = "\n".join(owner_message(next(r for r in REQUESTS if MEMORY_STEM in json.dumps(r))))
first_prompt = first[: first.index(MEMORY_STEM)].strip()



def recalled(delivery):
    """The texts of the records in a store delivery's packet. The packet also
    echoes the task's own words as its situation, which is not a record."""
    packet = json.loads(delivery[delivery.index("{") : delivery.rindex("}") + 1])
    return [hit["record"]["data"].get("text", "") for hit in packet["memory"]["records"]]


assert first_prompt and first_prompt in recalled(from_store[1]), from_store[1][:800]
assert recalled(from_store[0]) == [], from_store[0][:800]
assert not any(MEMORY_STEM in text for text in recalled(from_store[1]))
assert first_prompt in from_files[1], from_files[1][:800]
assert "no passage in 0 earlier session" in from_files[0], from_files[0][:400]

# Both delivered runs retained, and a delivery is not a passage.
seals = glob.glob(str(experiment / "homes" / "delivered__s1__r1" / "memory" / "authority" / "*.seal"))
assert len(seals) == 2, seals
assert from_files[1].count(TRANSCRIPT_STEM) == 1
assert not glob.glob(str(experiment / "homes" / "transcripts__s1__r1" / "memory"))
print("delivered recall smoke: ok —", len(rows), "trials, 2 store deliveries, 2 transcript deliveries, 2 seals")
