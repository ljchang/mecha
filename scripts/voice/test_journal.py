"""The voice worker's journal keeps measurements, never words (`journal.py`).

The rules are pure and run on a bare python3, as CI does. The sink itself is
measured against loguru and the real pipecat modules when they are installed
(the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_journal.py`), and
skipped otherwise."""

import asyncio
import importlib
import importlib.util
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import journal  # noqa: E402

SAID = "a sentence nobody should find in the journal"
DEBUG, INFO, WARNING = 10, 20, 30

HAVE_PIPECAT = all(importlib.util.find_spec(m) for m in ("loguru", "pipecat", "fastapi"))


class TheRules(unittest.TestCase):
    def test_a_line_carries_the_length_of_what_was_said_and_never_the_words(self):
        self.assertEqual(journal.withheld(SAID), f"<{len(SAID)} chars>")
        self.assertNotIn("sentence", journal.withheld(SAID))

    def test_pipecats_services_are_cut_below_info_and_nothing_else_is(self):
        for name in (
            "pipecat.services.tts_service",
            "pipecat.services.whisper.base_stt",
            "pipecat.services.openai.base_llm",
            "pipecat.services.some_service_added_later",
        ):
            self.assertFalse(journal.keeps(name, DEBUG), name)
            self.assertTrue(journal.keeps(name, INFO), name)
            self.assertTrue(journal.keeps(name, WARNING), name)
        for name in (
            "pipecat.transports.base_output",
            "pipecat.processors.metrics.frame_processor_metrics",
            "pipecat.pipeline.worker",
            "__main__",
            "pipecat.servicesx",  # a prefix that is not the family
            None,
        ):
            self.assertTrue(journal.keeps(name, DEBUG), name)


def log_as(module: str, level: str, message: str):
    """Log from inside the real `module`, so the record is named the way
    pipecat names its own (loguru takes the caller's `__name__`)."""
    mod = importlib.import_module(module)
    exec(f"logger.{level}(message)", vars(mod), {"message": message})


@unittest.skipUnless(HAVE_PIPECAT, "loguru, pipecat and fastapi live in the worker's venv")
class TheSink(unittest.TestCase):
    def setUp(self):
        from loguru import logger

        self.logger = logger
        self.lines = []
        self.addCleanup(self._restore)

    def _restore(self):
        self.logger.remove()
        self.logger.add(sys.stderr, level="DEBUG")

    def captured(self) -> str:
        return "".join(self.lines)

    def test_the_sink_drops_what_was_said_and_keeps_the_rest(self):
        journal.install(self.logger, lambda m: self.lines.append(str(m)))
        # The three that carried words on this machine (measured 2026-10-04).
        log_as("pipecat.services.tts_service", "debug", f"Generating TTS [{SAID}]")
        log_as("pipecat.services.whisper.base_stt", "debug", f"Transcription: [{SAID}]")
        log_as("pipecat.services.openai.base_llm", "debug", f"Generating chat [{SAID}]")
        # What stays: a service's warning, and every other module's DEBUG.
        log_as("pipecat.services.tts_service", "warning", "a service warning")
        log_as("pipecat.transports.base_output", "debug", "bot started speaking")
        self.assertNotIn(SAID, self.captured())
        self.assertIn("a service warning", self.captured())
        self.assertIn("bot started speaking", self.captured())

    def test_the_worker_installs_the_sink_at_server_start_after_pipecats_reset(self):
        from fastapi import FastAPI

        import worker

        app = FastAPI()
        worker.install(app)
        # What pipecat's `main()` does before it serves: replace every handler
        # with an unfiltered DEBUG sink (here, one that captures).
        self.logger.remove()
        self.logger.add(lambda m: self.lines.append(str(m)), level="DEBUG")

        real_stderr = sys.stderr

        class Capture:
            def write(_, s):
                self.lines.append(s)

            def flush(_):
                pass

        async def serve():
            sys.stderr = Capture()
            try:
                async with app.router.lifespan_context(app):
                    log_as("pipecat.services.tts_service", "debug", f"Generating TTS [{SAID}]")
                    log_as("pipecat.transports.base_output", "debug", "bot started speaking")
            finally:
                sys.stderr = real_stderr

        asyncio.run(serve())
        self.assertNotIn(SAID, self.captured(), "the lifespan did not replace pipecat's sink")
        self.assertIn("bot started speaking", self.captured())


if __name__ == "__main__":
    unittest.main()
