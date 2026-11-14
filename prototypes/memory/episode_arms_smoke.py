#!/usr/bin/env python3
"""End to end: the episode arms of a `mecha exp` lifetime, through the real
binary and a real fixture server.

    episode_arms_smoke.py <mecha binary> <empty scratch directory> <checkout>

The opaque ledger lifetime (`eval/fixtures/opaque_*`), four tasks in one home
per arm, against a scripted provider that knows the service's rules and, in
every task, first makes one export call the service refuses. So every run
has a refusal and then the call that works, which is what an episode is.

Asserted, for `episodes`, `episodes_appraised` and `episodes_store`:

- every task's first request carries the episode delivery after the owner's
  words, and from the second task on it holds the refusal and the call that
  worked, kept from the task before;
- the recorded tool surface names `episode_recall`: the tool was registered
  before the config record was written;
- every task after the first is handed its predecessor's outcome;
- the file arms keep `episodes/episodes.jsonl`, owner-only, and the
  appraised arm has one verdict a task; the store arm keeps no such file and
  has a store, and its grades were recorded by the driver's child without
  error;
- the `none` arm's requests carry no delivery.

Needs a binary built with the `memory-prototype` feature.
"""
import glob
import http.server
import json
import os
import re
import stat
import subprocess
import sys
import threading
from pathlib import Path

BINARY, ROOT, CHECKOUT = sys.argv[1], Path(sys.argv[2]).resolve(), sys.argv[3]
sys.path.insert(0, CHECKOUT + "/eval/fixtures")
import opaque_world as w  # noqa: E402

ROOT.mkdir(parents=True)
WORLD = "ledger"
world = w.WORLDS[WORLD]
T = world["tools"]
ORDINAL = {"fifth": 5, "ninth": 9, "eleventh": 11}
STEM = "The harness looked up what your earlier runs found out"
ARMS = ("episodes", "episodes_appraised", "episodes_store")
REQUESTS = []


def call(name, args, n):
    return {"role": "assistant", "tool_calls": [{"index": 0, "id": f"c{n}", "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}]}


def owner_words(request):
    """The owner's part of the first user message: what precedes a delivery."""
    first = next(m for m in request["messages"] if m["role"] == "user")
    content = first["content"]
    text = content if isinstance(content, str) else "\n".join(part.get("text", "") for part in content)
    return text.split(STEM)[0], text


class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        REQUESTS.append(body)
        prompt, _ = owner_words(body)
        name = re.search(r"named '([^']+)'", prompt).group(1)
        period = ORDINAL[re.search(r"(fifth|ninth|eleventh)", prompt).group(1)]
        ident, _ = world["accounts"][name]
        token = w.token(WORLD, name, 2026, period)
        if "number of rows" in prompt:
            answer = {"rows": len(w.rows(WORLD, name, 2026, period))}
        else:
            answer = {"total": w.total_major(WORLD, name, 2026, period)}
        script = [
            call(T["export"], {"id": ident, "token": "bogus"}, 9),
            call(T["lookup"], {"name": name}, 1),
            call(T["export"], {"id": ident, "token": token}, 2),
            call("fs_write", {"path": "answer.json", "content": json.dumps(answer)}, 3),
            {"role": "assistant", "content": f"Done: wrote {json.dumps(answer)} for {name}."},
        ]
        step = sum(1 for m in body["messages"] if m["role"] == "assistant")
        msg = script[min(step, len(script) - 1)]
        finish = "tool_calls" if "tool_calls" in msg else "stop"
        usage = {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}
        if body.get("stream"):
            chunk = {"id": "f", "model": body["model"], "choices": [{"index": 0, "delta": msg, "finish_reason": finish}], "usage": usage}
            data = ("data: " + json.dumps(chunk) + "\n\ndata: [DONE]\n\n").encode()
            kind = "text/event-stream"
        else:
            data = json.dumps({"model": body["model"], "choices": [{"message": msg, "finish_reason": finish}], "usage": usage}).encode()
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
enabled = ["goal_context", "fs_read", "fs_write", "fs_list"]
permission_mode = "allow"
"""
)
# The tracked manifest, cut to one seed and the arms under test.
source = Path(CHECKOUT, "eval", f"memory-opaque-{WORLD}.toml").read_text()
head, *arms = re.split(r"\n(?=\[arms\.\w+\]\n)", source)
kept = [a for a in arms if re.match(r"\[arms\.(none|" + "|".join(ARMS) + r")\]", a)]
assert len(kept) == 1 + len(ARMS), [a.split("\n")[0] for a in arms]
manifest = ROOT / "episode-arms.toml"
manifest.write_text(
    head.replace(f'name = "memory-opaque-{WORLD}"', 'name = "episode-arms-smoke"').replace("seeds = [1, 2, 3]", "seeds = [1]")
    + "\n"
    + "\n".join(kept)
)
env = {"HOME": str(home), "MECHA_HOME": str(mecha), "PATH": os.environ["PATH"], "LANG": "C.UTF-8", "TERM": "dumb"}


def run(*args):
    p = subprocess.run([BINARY, *args], cwd=CHECKOUT, env=env, capture_output=True, text=True, timeout=600)
    assert p.returncode == 0, f"mecha {' '.join(args)} -> {p.returncode}\n{p.stdout[-1500:]}\n{p.stderr[-2500:]}"
    return p.stdout + p.stderr


run("exp", "new", str(manifest))
log = run("exp", "run", "episode-arms-smoke")
experiment = mecha / "experiments" / "episode-arms-smoke"
rows = [json.load(open(p)) for p in glob.glob(str(experiment / "trials" / "*.json"))]
assert len(rows) == 4 * (1 + len(ARMS)) and all(r["status"] == "done" and r["passed"] for r in rows), log[-3000:]

for arm in ARMS + ("none",):
    arm_home = experiment / "homes" / f"{arm}__s1__r1"
    trials = sorted((r for r in rows if r["arm"] == arm), key=lambda r: r["started_at"])
    deliveries = []
    for trial in trials:
        records = [json.loads(line) for line in open(arm_home / "sessions" / f"{trial['session_id']}.jsonl")]
        text = json.dumps(records)
        delivered = [s for s in re.findall(r'"text": "((?:[^"\\]|\\.)*)"', text) if s.startswith(STEM)]
        if arm == "none":
            assert not delivered and "episode_recall" not in text, trial["id"]
            continue
        # One delivery, in the owner's message, after the owner's words.
        first_user = next(r for r in records if isinstance(r.get("role"), str) and r["role"] == "user" or r.get("message", {}).get("role") == "user")
        blocks = first_user.get("content") or first_user["message"]["content"]
        texts = [b["text"] for b in blocks if b.get("type") == "text"]
        assert len(texts) == 2 and not texts[0].startswith(STEM) and texts[1].startswith(STEM), (trial["id"], texts)
        assert texts[0].startswith("Use the ledger tools"), texts[0][:80]
        # The surface as recorded names the tool.
        assert '"episode_recall"' in text, trial["id"]
        deliveries.append(texts[1])
    if arm == "none":
        continue
    assert len(deliveries) == 4, (arm, len(deliveries))
    assert "Nothing recorded about these tools." in deliveries[0], deliveries[0][:400]
    for later in deliveries[1:]:
        assert f"{T['export']} was refused" in later and "E17: request rejected" in later, later[:800]
        assert "It worked when called with" in later and "26:05#K" in later, later[:800]
        assert "An earlier task asked:" in later and "The run concluded: Done: wrote" in later, later[:800]
        # A delivery is never kept as something found out.
        assert later.count(STEM) == 1 and "episode_recall was" not in later
    file = arm_home / "episodes" / "episodes.jsonl"
    if arm == "episodes_store":
        assert not file.exists() and (arm_home / "memory" / "store").is_dir(), arm
        # The recorder did not run beside the episodes: nothing was sealed.
        assert not glob.glob(str(arm_home / "memory" / "authority" / "*.seal")), arm
    else:
        assert stat.S_IMODE(file.stat().st_mode) == 0o600, oct(file.stat().st_mode)
        lines = [json.loads(line) for line in open(file)]
        verdicts = [line for line in lines if "verdict" in line]
        assert len(verdicts) == (4 if arm == "episodes_appraised" else 0), (arm, len(verdicts))
        # One finding however many runs met it, and every run's outcome.
        assert sum(1 for line in lines if line.get("concluded")) == 4, arm

# Every request of the `none` arm went out with no delivery.
assert sum(STEM in owner_words(r)[1] for r in REQUESTS) == sum(1 for r in REQUESTS if STEM in json.dumps(r["messages"][:2]))
print("episode arms smoke: ok —", len(rows), "trials;", ", ".join(ARMS), "each handed the refusal, the fix and the outcome")
