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
[ -n "${FAKE_SMI_BROKEN:-}" ] && exit 9
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
[ -n "${FAKE_SS_BROKEN:-}" ] && exit 1
[ -n "${FAKE_CONNECTED:-}" ] && echo "ESTAB 0 0 127.0.0.1:8188 127.0.0.1:51234"
exit 0
""",
    "journalctl": """#!/bin/bash
[ -n "${FAKE_JOURNAL_BROKEN:-}" ] && exit 1
[ -n "${FAKE_RECENT:-}" ] && echo "got prompt"
exit 0
""",
}


class IdleReset(unittest.TestCase):
    def run_script(self, *, gpu=12500, rss_mib=1400, queue=IDLE_QUEUE, memfree_mib=30000, rc=0, **extra):
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
            self.assertEqual(out.returncode, rc, out.stdout + out.stderr)
            # Every tick says what it decided: a silent decline reads the same
            # as a timer that never ran.
            self.assertIn("comfyui-idle-reset:", out.stdout, "the script decided silently")
            self.last_said = out.stdout
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
        # Parses, but is not a queue: unknown, so busy.
        calls = self.run_script(queue="{}")
        self.assertFalse(self.restarted(calls))

    def test_an_open_connection_is_a_client_mid_call(self):
        self.assertFalse(self.restarted(self.run_script(FAKE_CONNECTED="1")))

    def test_a_connection_check_that_cannot_run_is_busy_and_fails_the_unit(self):
        # The only guard while references upload, before anything is queued;
        # a failed unit is what `mecha doctor` can see.
        self.assertFalse(self.restarted(self.run_script(FAKE_SS_BROKEN="1", rc=1)))

    def test_a_journal_that_cannot_be_read_is_not_unused_and_fails_the_unit(self):
        self.assertFalse(self.restarted(self.run_script(FAKE_JOURNAL_BROKEN="1", rc=1)))

    def test_an_unreadable_gpu_figure_is_unknown_not_zero(self):
        # Read as zero, a loaded server (12.2 GPU + 1.4 RSS) would fall under
        # the floor on its RSS alone and never be restarted.
        calls = self.run_script(gpu="[N/A]", rss_mib=1400, rc=1)
        self.assertFalse(self.restarted(calls))
        self.assertIn("unknown", self.last_said)
        calls = self.run_script(FAKE_SMI_BROKEN="1", rc=1)
        self.assertFalse(self.restarted(calls))

    def test_use_within_the_window_is_not_idle(self):
        self.assertFalse(self.restarted(self.run_script(FAKE_RECENT="1")))

    def test_low_free_memory_is_judged_after_the_restart_returns_what_is_held(self):
        # 1 GB free now, but the restart gives back ~13.9 GB first: restart.
        calls = self.run_script(memfree_mib=1000)
        self.assertTrue(self.restarted(calls), calls)
        self.assertFalse(self.freed(calls))

    def test_too_little_even_after_the_restart_frees_instead(self):
        # 3.1 GB held + 0.5 GB free < 4 GB: a new CUDA context may not come up.
        calls = self.run_script(gpu=1100, rss_mib=2000, memfree_mib=500)
        self.assertFalse(self.restarted(calls), calls)
        self.assertTrue(self.freed(calls), calls)

    def test_too_little_with_nothing_on_the_gpu_does_nothing(self):
        # /free cannot release RSS, so asking would only add a request.
        calls = self.run_script(gpu=100, rss_mib=3000, memfree_mib=500)
        self.assertFalse(self.restarted(calls))
        self.assertFalse(self.freed(calls))

    def test_no_running_service_is_left_alone(self):
        calls = self.run_script(FAKE_PID="0")
        self.assertFalse(self.restarted(calls))


if __name__ == "__main__":
    unittest.main()
