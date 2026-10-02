#!/usr/bin/env python3
"""When scripts/comfyui/comfyui-idle-reset restarts ComfyUI, and when it must not.

`python3 scripts/test_comfyui_idle_reset.py`. Standard library only, like
test_model_idle.py beside it.

**Why this exists.** The script's one act is a restart, and a restart at the
wrong moment kills a picture mid-render; never restarting leaves 13.6 GB held
for hours, which is the failure it was written for (2026-10-02). Every
condition below is one of those two, against stand-ins for systemctl,
nvidia-smi, curl, ss, journalctl and /proc, so the decision is tested without
a GPU or a server.
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "comfyui" / "comfyui-idle-reset"
PID = "4242"
IDLE_QUEUE = '{"queue_running": [], "queue_pending": []}'
BUSY_QUEUE = '{"queue_running": [[0, "x"]], "queue_pending": []}'

# Each stand-in logs its argv to $CALLS and answers from the environment.
FAKES = {
    "systemctl": """#!/bin/bash
echo "systemctl $*" >> "$CALLS"
case "$*" in *show*) echo "${FAKE_PID-4242}";; esac
""",
    "nvidia-smi": """#!/bin/bash
[ -n "${FAKE_GPU_MIB:-}" ] && echo "${FAKE_PID-4242}, $FAKE_GPU_MIB"
echo "1111, 42461"
""",
    "curl": """#!/bin/bash
echo "curl $*" >> "$CALLS"
case "$*" in *POST*) exit 0;; esac
[ -n "${FAKE_QUEUE_DOWN:-}" ] && exit 7
echo "$FAKE_QUEUE"
""",
    "ss": """#!/bin/bash
[ -n "${FAKE_CONNECTED:-}" ] && echo "ESTAB 0 0 127.0.0.1:8188 127.0.0.1:51234"
exit 0
""",
    "journalctl": """#!/bin/bash
[ -n "${FAKE_RECENT:-}" ] && echo "got prompt"
exit 0
""",
}


class IdleReset(unittest.TestCase):
    def run_script(self, *, gpu=12500, rss_mib=1400, queue=IDLE_QUEUE, memfree_mib=30000, **extra):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            bindir = d / "bin"
            bindir.mkdir()
            for name, body in FAKES.items():
                f = bindir / name
                f.write_text(body)
                f.chmod(0o755)
            (d / "proc" / PID).mkdir(parents=True)
            (d / "proc" / PID / "status").write_text(f"Name:\tpython\nVmRSS:\t{rss_mib * 1024} kB\n")
            (d / "meminfo").write_text(f"MemTotal: 127000000 kB\nMemFree: {memfree_mib * 1024} kB\n")
            calls = d / "calls"
            calls.touch()
            env = {
                "PATH": f"{bindir}:{os.environ['PATH']}",
                "CALLS": str(calls),
                "MECHA_PROC": str(d / "proc"),
                "MECHA_MEMINFO": str(d / "meminfo"),
                "FAKE_QUEUE": queue,
            }
            if gpu is not None:
                env["FAKE_GPU_MIB"] = str(gpu)
            env.update({k: v for k, v in extra.items() if v is not None})
            out = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=30)
            self.assertEqual(out.returncode, 0, out.stderr)
            return calls.read_text()

    def restarted(self, calls):
        return "systemctl --user restart comfyui" in calls

    def freed(self, calls):
        return "POST" in calls and "/free" in calls

    def test_a_loaded_idle_server_is_restarted(self):
        calls = self.run_script()
        self.assertTrue(self.restarted(calls), calls)
        self.assertFalse(self.freed(calls))

    def test_a_server_mecha_already_freed_is_still_restarted(self):
        # /free leaves the weights in RSS on unified memory; a GPU-only check
        # would read this as "nothing loaded" and hold 7-9 GB forever.
        calls = self.run_script(gpu=350, rss_mib=7000)
        self.assertTrue(self.restarted(calls), calls)

    def test_a_fresh_server_is_left_alone(self):
        calls = self.run_script(gpu=170, rss_mib=960)
        self.assertFalse(self.restarted(calls))
        self.assertNotIn("curl", calls, "a fresh server was asked anything")

    def test_a_busy_queue_is_never_restarted(self):
        self.assertFalse(self.restarted(self.run_script(queue=BUSY_QUEUE)))

    def test_an_unreadable_queue_is_not_idle(self):
        calls = self.run_script(FAKE_QUEUE_DOWN="1")
        self.assertFalse(self.restarted(calls))
        calls = self.run_script(queue="not json")
        self.assertFalse(self.restarted(calls))

    def test_an_open_connection_is_a_client_mid_call(self):
        self.assertFalse(self.restarted(self.run_script(FAKE_CONNECTED="1")))

    def test_use_within_the_window_is_not_idle(self):
        self.assertFalse(self.restarted(self.run_script(FAKE_RECENT="1")))

    def test_low_free_memory_frees_instead_of_restarting(self):
        calls = self.run_script(memfree_mib=1000)
        self.assertFalse(self.restarted(calls), calls)
        self.assertTrue(self.freed(calls), calls)

    def test_low_free_memory_with_nothing_on_the_gpu_does_nothing(self):
        # /free cannot release RSS, so asking would only add a request.
        calls = self.run_script(gpu=350, rss_mib=7000, memfree_mib=1000)
        self.assertFalse(self.restarted(calls))
        self.assertFalse(self.freed(calls))

    def test_no_running_service_is_left_alone(self):
        calls = self.run_script(FAKE_PID="0")
        self.assertFalse(self.restarted(calls))


if __name__ == "__main__":
    unittest.main()
