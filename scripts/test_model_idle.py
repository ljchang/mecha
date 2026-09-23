#!/usr/bin/env python3
"""The exit-code contract of scripts/model-idle.sh, against a stub server.

`python3 scripts/test_model_idle.py`. Standard library only, like
test_appraisal_report.py beside it.

**Why this exists.** The script's whole job is its exit code: 0 lets the
daytime mail sweep run, 1 skips a tick quietly, 255 fails the unit so `mecha
doctor` sees it. Which answer gets which code is exactly what decides whether
a broken model server is noticed or silently stands the sweep down forever,
and every case below was a review finding before it was a line of the
script. A stub llama-server stands in for the real one, so a change to the
script that turns a loud failure quiet fails here.
"""
import http.server
import json
import os
import socket
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("model-idle.sh")

ROUTES = {
    "/idle": (200, [{"id": 0, "is_processing": False}, {"id": 1, "is_processing": False}]),
    "/busy": (200, [{"id": 0, "is_processing": True}, {"id": 1, "is_processing": False}]),
    "/loading": (503, {"error": {"message": "Loading model"}}),
    "/noslots": (501, {"error": {"message": "This server does not support slots endpoint."}}),
    "/empty": (200, []),
    "/renamed": (200, [{"id": 0, "busy": False}]),
    "/notalist": (200, {"status": "ok"}),
}


class Stub(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/hang":
            threading.Event().wait(8)  # longer than the script's 5 s budget
            return
        code, body = ROUTES.get(self.path, (404, {}))
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class ModelIdle(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def setUp(self):
        self.state = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.state.cleanup()

    def run_check(self, url, stuck_max=3, gpu_busy="100", path=None):
        env = dict(
            os.environ,
            PATH=path or os.environ["PATH"],
            MECHA_SLOTS_URL=url,
            MECHA_IDLE_STUCK_MAX=str(stuck_max),
            MECHA_GPU_BUSY=gpu_busy,  # never skip on the real GPU under test
            XDG_STATE_HOME=self.state.name,
        )
        out = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=30)
        return out.returncode, out.stdout

    def test_an_idle_server_lets_the_sweep_run(self):
        self.assertEqual(self.run_check(f"{self.base}/idle")[0], 0)

    def test_a_busy_slot_skips(self):
        code, said = self.run_check(f"{self.base}/busy")
        self.assertEqual(code, 1)
        self.assertIn("1 model slot(s) in use", said)

    def test_a_server_answering_wrongly_fails_at_once(self):
        # --no-slots, and every slot shape this script does not recognise:
        # each must be loud, never a confident "0 in use".
        for route in ["/noslots", "/empty", "/renamed", "/notalist", "/missing"]:
            with self.subTest(route=route):
                self.assertEqual(self.run_check(f"{self.base}{route}")[0], 255)

    def test_loading_skips_then_fails_once_it_has_lasted(self):
        codes = [self.run_check(f"{self.base}/loading", stuck_max=3)[0] for _ in range(3)]
        self.assertEqual(codes, [1, 1, 255])

    def test_a_refused_port_is_a_restart_until_it_has_lasted(self):
        dead = f"http://127.0.0.1:{free_port()}/slots"
        codes = [self.run_check(dead, stuck_max=2)[0] for _ in range(2)]
        self.assertEqual(codes, [1, 255])

    def test_a_timeout_counts_toward_the_same_limit(self):
        code, said = self.run_check(f"{self.base}/hang", stuck_max=5)
        self.assertEqual(code, 1)
        self.assertIn("too busy to answer", said)

    def test_a_live_answer_clears_the_count(self):
        self.run_check(f"{self.base}/loading", stuck_max=3)
        self.run_check(f"{self.base}/loading", stuck_max=3)
        self.assertEqual(self.run_check(f"{self.base}/busy", stuck_max=3)[0], 1)
        # Had the count survived, this third stuck tick would fail.
        self.assertEqual(self.run_check(f"{self.base}/loading", stuck_max=3)[0], 1)

    def gpu_path(self, util):
        # A stub `nvidia-smi` first on PATH; util None means no nvidia-smi at
        # all, which needs a PATH without the real one on it.
        bindir = Path(self.state.name) / "bin"
        bindir.mkdir()
        for tool in ["bash", "curl", "python3", "cat", "mkdir", "dirname", "rm", "head", "tr"]:
            real = subprocess.run(["bash", "-c", f"command -v {tool}"], capture_output=True, text=True).stdout.strip()
            if real:
                (bindir / tool).symlink_to(real)
        if util is not None:
            smi = bindir / "nvidia-smi"
            smi.write_text(f"#!/bin/sh\necho {util}\n")
            smi.chmod(0o755)
        return str(bindir)

    def test_a_busy_gpu_skips(self):
        code, said = self.run_check(f"{self.base}/idle", gpu_busy="30", path=self.gpu_path(85))
        self.assertEqual(code, 1)
        self.assertIn("GPU at 85%", said)

    def test_a_quiet_gpu_runs(self):
        self.assertEqual(self.run_check(f"{self.base}/idle", gpu_busy="30", path=self.gpu_path(12))[0], 0)

    def test_no_gpu_query_fails_open(self):
        # A box without a usable nvidia-smi still sorts its mail.
        self.assertEqual(self.run_check(f"{self.base}/idle", gpu_busy="30", path=self.gpu_path(None))[0], 0)

    def test_a_count_that_cannot_be_kept_fails_rather_than_never_escalating(self):
        blocker = Path(self.state.name) / "mecha"
        blocker.write_text("a file where the state directory should be")
        self.assertEqual(self.run_check(f"{self.base}/loading")[0], 255)


if __name__ == "__main__":
    unittest.main()
