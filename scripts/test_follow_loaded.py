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
# a directory, the way the real one does, so each script gets past its cd, and
# `config show` answers with the config the case describes (scripts/pin.sh
# reads the default provider's `follow_loaded` off it).
STUB = """#!/bin/sh
printf '%s\\n' "$*" >> "{log}"
if [ "$1" = work ] && [ "$2" = path ]; then
    mkdir -p "{root}/work/$3" && echo "{root}/work/$3"
fi
if [ "$1" = config ] && [ "$2" = show ]; then
    cat "{root}/config.toml"
fi
if [ "$1" = sessions ] && [ "$2" = compare ] && [ -n "$STUB_HANG_COMPARE" ]; then
    sleep 30
fi
exit 0
"""

ROUTER = (
    'default_provider = "local"\n[providers.local]\nkind = "local"\n'
    'base_url = "http://127.0.0.1:8080"\nfollow_loaded = true\n'
)
# The flag on an entry mecha will not follow (`router::follows_here`: local
# kind, loopback URL) is ignored there, so it must not drop the pin here.
OFF_BOX = ROUTER.replace("127.0.0.1", "10.0.0.5")
NO_ROUTER = 'default_provider = "anthropic"\n[providers.anthropic]\n[providers.local]\n'



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

    def run_script(self, name, config=ROUTER, script_dir=None, rc=0, **env):
        """Run `name` with every binary stubbed; return the mecha calls."""
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "config.toml").write_text(config)
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
                ["bash", str((script_dir or HERE) / name)],
                env={**base, **env},
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(done.returncode, rc, done.stderr)
            calls = log.read_text().splitlines() if log.exists() else []
        # Only the calls that reach a model — or, for `rules`, resolve one:
        # `work path`, `work clean` and the listings take no provider.
        return [
            c.split() for c in calls if not c.startswith(("work ", "proposals", "harness list", "config show"))
        ]

    def assert_follows(self, calls, expected):
        self.assertEqual(
            [c[:2] if c[0] in ("frontdoor", "harness", "sessions") else c[:1] for c in calls], expected
        )
        for c in calls:
            self.assertNotIn("-p", c, f"unpinned by default, but: {c}")
            self.assertNotIn("--judge-provider", c, f"the judge follows too, but: {c}")

    def assert_pinned(self, calls, flag, value, where):
        self.assertTrue(calls, "no call reached the stub")
        # Every named stage must be there, or dropping one passes vacuously.
        for w in where:
            self.assertTrue(any(w in c for c in calls), f"no call for {w!r} in {calls}")
        for c in calls:
            if any(w in c for w in where):
                self.assertIn(flag, c, c)
                self.assertEqual(c[c.index(flag) + 1], value, c)

    def test_ruminate_follows_unless_pinned(self):
        self.assert_follows(
            self.run_script("ruminate.sh"),
            # `sessions compare` sits between validate and learn (its Rules arm
            # measures yesterday's rules), and the lesson-source pass runs
            # last; both follow like every other stage (#333 on top of #346 —
            # found when #333's lines still named the removed $PROVIDER, which
            # `set -u` turned into the whole night stopping at compare).
            [
                ["reflect"],
                ["distill"],
                ["validate"],
                # The brake right after validate, ahead of every paid pass
                # (owner, 2026-09-27).
                ["rules"],
                ["sessions", "compare"],
                ["learn"],
                # ...and again after learn, which can re-widen a narrowing.
                ["rules"],
                ["harness", "ruminate"],
                ["learn"],
            ],
        )
        # The compare pass reads a bounded window (owner, 2026-09-27).
        compare = [c for c in self.run_script("ruminate.sh") if c[:2] == ["sessions", "compare"]]
        self.assertEqual(len(compare), 1, compare)
        self.assertIn("--days", compare[0])
        self.assertEqual(compare[0][compare[0].index("--days") + 1], "30", compare[0])
        calls = self.run_script("ruminate.sh", MECHA_RUMINATE_PROVIDER="x", MECHA_RUMINATE_JUDGE="j")
        # `rules propose-retirements` too: it counts only the rows of the
        # model validate measured on, so it must resolve the same one.
        self.assert_pinned(
            calls,
            "-p",
            "x",
            ["reflect", "distill", "validate", "compare", "learn", "propose-retirements", "ruminate", "--compare-sources"],
        )
        self.assert_pinned(calls, "--judge-provider", "j", ["validate"])
        # A pinned night with the judge unset judges on the pinned model, not
        # on the default entry (which would swap the router per judge call).
        calls = self.run_script("ruminate.sh", MECHA_RUMINATE_PROVIDER="x")
        self.assert_pinned(calls, "--judge-provider", "x", ["validate"])
        # No router: `local`, as before it — never the default, which may be
        # a paid API (found on review of #346).
        calls = self.run_script("ruminate.sh", config=NO_ROUTER)
        self.assert_pinned(
            calls,
            "-p",
            "local",
            ["reflect", "distill", "validate", "compare", "learn", "propose-retirements", "ruminate", "--compare-sources"],
        )
        self.assert_pinned(calls, "--judge-provider", "local", ["validate"])
        # An unreadable config is not a router, nor is a flag mecha ignores.
        calls = self.run_script("ruminate.sh", config="not toml [")
        self.assert_pinned(calls, "-p", "local", ["reflect", "learn"])
        calls = self.run_script("ruminate.sh", config=OFF_BOX)
        self.assert_pinned(calls, "-p", "local", ["reflect", "learn"])

    def test_a_hung_compare_is_cut_at_its_cap_and_the_night_goes_on(self):
        # The clock half of the compare bound (review of #355): the stub's
        # compare hangs for 30 s, the cap is 1 s, and every stage after it
        # still runs — promptly. On a line with no `timeout` the night waits
        # the full 30 s.
        import time

        start = time.monotonic()
        calls = self.run_script("ruminate.sh", STUB_HANG_COMPARE="1", MECHA_COMPARE_TIMEOUT="1s")
        elapsed = time.monotonic() - start
        self.assertLess(elapsed, 15, f"the night waited {elapsed:.0f}s on a hung compare")
        after = calls[[c[:2] for c in calls].index(["sessions", "compare"]) + 1 :]
        self.assertEqual(
            [c[:2] if c[0] == "harness" else c[:1] for c in after],
            [["learn"], ["rules"], ["harness", "ruminate"], ["learn"]],
        )

    def test_a_missing_pin_rule_stops_every_script(self):
        # Without pin.sh, PIN would be unset and every stage would run on the
        # default, unpinned — so each script refuses before any model stage.
        # A pin.sh that is there but defines nothing is the same failure.
        for pin in (None, "# emptied\n"):
            with tempfile.TemporaryDirectory() as bare:
                if pin is not None:
                    (Path(bare) / "pin.sh").write_text(pin)
                for name in ("ruminate.sh", "frontdoor.sh", "learn-live.sh"):
                    (Path(bare) / name).write_text((HERE / name).read_text())
                    calls = self.run_script(name, script_dir=Path(bare), rc=1)
                    self.assertEqual(calls, [], f"{name} ran stages without its pin rule ({pin!r})")

    def test_frontdoor_follows_unless_pinned(self):
        self.assert_follows(
            self.run_script("frontdoor.sh"),
            [["frontdoor", "extract"], ["frontdoor", "triage"]],
        )
        calls = self.run_script("frontdoor.sh", MECHA_FRONTDOOR_PROVIDER="x")
        self.assert_pinned(calls, "-p", "x", ["extract", "triage"])
        calls = self.run_script("frontdoor.sh", config=NO_ROUTER)
        self.assert_pinned(calls, "-p", "local", ["extract", "triage"])

    def test_learn_live_follows_unless_pinned(self):
        self.assert_follows(self.run_script("learn-live.sh"), [["reflect"], ["learn"]])
        calls = self.run_script("learn-live.sh", MECHA_LEARN_PROVIDER="x")
        self.assert_pinned(calls, "-p", "x", ["reflect", "learn"])
        calls = self.run_script("learn-live.sh", config=NO_ROUTER)
        self.assert_pinned(calls, "-p", "local", ["reflect", "learn"])


if __name__ == "__main__":
    unittest.main()
