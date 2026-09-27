#!/usr/bin/env python3
"""The scheduled scripts run on whatever the router has loaded, unless pinned.

`python3 scripts/test_follow_loaded.py`. Standard library only, like
test_model_idle.py beside it.

**Why this exists.** On the llama-server router an explicit `-p <entry>` is a
pin, and a pin is a *load*: the request names a model and the router swaps it
in. The owner's ruling is that background work runs on the resident model
(REMOTE-SURFACE-DESIGN §14), and the router install removed `-p local` from
the distill hook and the morning trigger — but three scripts defaulted their
provider variable to `local`. On 2026-09-27 ruminate.sh's first stage pulled
production over the comparison arm at 03:30:14; learn-live.sh, a
`session_end` hook, would have done it at every chat's close, and
frontdoor.sh hourly. Each case below runs a script against a stub `mecha`
that records its arguments, and reads what it was asked.
"""
import http.server
import os
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

HERE = Path(__file__).parent

# Every stub records its argv, one line per call; `work path X` answers with
# a directory, the way the real one does, so each script gets past its cd.
STUB = """#!/bin/sh
printf '%s\\n' "$*" >> "{log}"
if [ "$1" = work ] && [ "$2" = path ]; then
    mkdir -p "{root}/work/$3" && echo "{root}/work/$3"
fi
exit 0
"""


class Health(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()

    def log_message(self, *a):
        pass


def serve_health():
    srv = http.server.HTTPServer(("127.0.0.1", 0), Health)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


class Scripts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.health = serve_health()
        cls.health_url = f"http://127.0.0.1:{cls.health.server_port}/health"

    @classmethod
    def tearDownClass(cls):
        cls.health.shutdown()

    def run_script(self, name, **env):
        """Run `name` with every binary stubbed; return the mecha calls."""
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "calls"
            stub = Path(root) / "mecha"
            stub.write_text(STUB.format(log=log, root=root))
            stub.chmod(0o755)
            other = Path(root) / "other"
            other.write_text("#!/bin/sh\nexit 0\n")
            other.chmod(0o755)
            base = {
                "PATH": os.environ["PATH"],
                "HOME": root,
                "MECHA_BIN": str(stub),
                "FACTORY_PUBLISH_BIN": str(other),
                "MECHA_MAIL_BIN": str(other),
                "MECHA_LEARNING_DIR": str(Path(root) / "learning"),
                "MECHA_RUMINATE_HEALTH": self.health_url,
                "MECHA_FRONTDOOR_HEALTH": self.health_url,
            }
            (Path(root) / "learning").mkdir()
            done = subprocess.run(
                ["bash", str(HERE / name)],
                env={**base, **env},
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(done.returncode, 0, done.stderr)
            calls = log.read_text().splitlines() if log.exists() else []
        # Only the calls that reach a model — or, for `rules`, resolve one:
        # `work path`, `work clean` and the listings take no provider.
        return [c.split() for c in calls if not c.startswith(("work ", "proposals", "harness list"))]

    def assert_follows(self, calls, expected):
        self.assertEqual([c[:2] if c[0] in ("frontdoor", "harness") else c[:1] for c in calls], expected)
        for c in calls:
            self.assertNotIn("-p", c, f"unpinned by default, but: {c}")
            self.assertNotIn("--judge-provider", c, f"the judge follows too, but: {c}")

    def assert_pinned(self, calls, flag, value, where):
        self.assertTrue(calls, "no call reached the stub")
        for c in calls:
            if any(w in c for w in where):
                self.assertIn(flag, c, c)
                self.assertEqual(c[c.index(flag) + 1], value, c)

    def test_ruminate_follows_unless_pinned(self):
        self.assert_follows(
            self.run_script("ruminate.sh"),
            [["reflect"], ["distill"], ["validate"], ["learn"], ["rules"], ["harness", "ruminate"]],
        )
        calls = self.run_script("ruminate.sh", MECHA_RUMINATE_PROVIDER="x", MECHA_RUMINATE_JUDGE="j")
        # `rules propose-retirements` too: it counts only the rows of the
        # model validate measured on, so it must resolve the same one.
        self.assert_pinned(
            calls, "-p", "x", ["reflect", "distill", "validate", "learn", "propose-retirements", "ruminate"]
        )
        self.assert_pinned(calls, "--judge-provider", "j", ["validate"])

    def test_frontdoor_follows_unless_pinned(self):
        self.assert_follows(
            self.run_script("frontdoor.sh"),
            [["frontdoor", "extract"], ["frontdoor", "triage"]],
        )
        calls = self.run_script("frontdoor.sh", MECHA_FRONTDOOR_PROVIDER="x")
        self.assert_pinned(calls, "-p", "x", ["extract", "triage"])

    def test_learn_live_follows_unless_pinned(self):
        self.assert_follows(self.run_script("learn-live.sh"), [["reflect"], ["learn"]])
        calls = self.run_script("learn-live.sh", MECHA_LEARN_PROVIDER="x")
        self.assert_pinned(calls, "-p", "x", ["reflect", "learn"])


if __name__ == "__main__":
    unittest.main()
