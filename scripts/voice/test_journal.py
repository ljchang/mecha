"""The voice worker's journal keeps measurements, never words (`journal.py`).

Three layers, by what each needs:
- the rules, on a bare python3;
- the sink and the lifespan wrapper, with loguru (CI installs it): a record
  is named by `logger.patch`, so the filter, the level number and the
  composition are measured without pipecat;
- pipecat's real logger names and `worker.install`, in the worker's venv:
  `~/models/voice-worker-venv/bin/python scripts/voice/test_journal.py`.
A layer whose dependency is missing skips, by name."""

import asyncio
import contextlib
import importlib
import importlib.util
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import journal  # noqa: E402

SAID = "a sentence nobody should find in the journal"
DEBUG, INFO, WARNING = 10, 20, 30

HAVE_LOGURU = importlib.util.find_spec("loguru") is not None
HAVE_PIPECAT = HAVE_LOGURU and all(importlib.util.find_spec(m) for m in ("pipecat", "fastapi"))

# A skip reads exactly like a pass, so CI says which layers it expects: with
# MECHA_TEST_REQUIRE_BACKENDS=1 a missing loguru fails the run instead of
# skipping the sink and lifespan layers, as mecha-core/tests/support does for
# the Rust backends. pipecat's real names stay venv-only.
if os.environ.get("MECHA_TEST_REQUIRE_BACKENDS") == "1" and not HAVE_LOGURU:
    raise SystemExit("loguru is unavailable, and MECHA_TEST_REQUIRE_BACKENDS is set")


class TheRules(unittest.TestCase):
    def test_a_line_carries_the_length_of_what_was_said_and_never_the_words(self):
        self.assertEqual(journal.withheld(SAID), f"<{len(SAID)} chars>")
        self.assertNotIn("sentence", journal.withheld(SAID))

    def test_below_info_pipecat_keeps_only_the_families_measured_wordless(self):
        cut = (
            # The three that carried words (measured 2026-10-04).
            "pipecat.services.tts_service",
            "pipecat.services.whisper.base_stt",
            "pipecat.services.openai.base_llm",
            # Unmeasured, so not clean: a family an upgrade adds or renames.
            "pipecat.transcriptions.language",
            "pipecat.llm.context",
            "pipecat",
            "pipecat.transportsx",  # a prefix that is not the family
        )
        for name in cut:
            self.assertFalse(journal.keeps(name, DEBUG), name)
            self.assertTrue(journal.keeps(name, INFO), name)
            self.assertTrue(journal.keeps(name, WARNING), name)
        for name in (
            "pipecat.transports.base_output",
            "pipecat.processors.metrics.frame_processor_metrics",
            "pipecat.pipeline.worker",
            "pipecat.audio.turn.smart_turn.base_smart_turn",
            "pipecat.workers.runner",
            # Outside pipecat, nothing is narrowed here.
            "__main__",
            "pipecatx.something",
            None,
        ):
            self.assertTrue(journal.keeps(name, DEBUG), name)


class Loguru(unittest.TestCase):
    """Captures through loguru, and leaves it as pipecat's runner would."""

    def setUp(self):
        from loguru import logger

        self.logger = logger
        self.lines = []
        self.addCleanup(self._restore)

    def _restore(self):
        self.logger.remove()
        self.logger.add(sys.stderr, level="DEBUG")

    def capture(self, message):
        self.lines.append(str(message))

    def captured(self) -> str:
        return "".join(self.lines)

    def log_named(self, name, level, message):
        """A record named `name`, as pipecat's module would name it."""
        getattr(self.logger.patch(lambda r: r.update(name=name)), level)(message)


@unittest.skipUnless(HAVE_LOGURU, "loguru is not installed")
class TheSink(Loguru):
    def test_the_level_cut_is_loggers_own_info(self):
        self.assertEqual(journal.INFO, self.logger.level("INFO").no)

    def test_the_sink_drops_what_was_said_and_keeps_the_rest(self):
        # An unfiltered handler first, as the runner leaves it: `install`
        # must replace it, not sit beside it.
        self.logger.remove()
        self.logger.add(self.capture, level="DEBUG")
        self.lines.clear()
        journal.install(self.logger, self.capture)
        self.log_named("pipecat.services.tts_service", "debug", f"Generating TTS [{SAID}]")
        self.log_named("pipecat.services.openai.base_llm", "debug", f"Generating chat [{SAID}]")
        self.log_named("pipecat.services.tts_service", "warning", "a service warning")
        self.log_named("pipecat.transports.base_output", "debug", "bot started speaking")
        self.assertNotIn(SAID, self.captured(), "a handler beside the filter still wrote the words")
        self.assertIn("a service warning", self.captured())
        self.assertIn("bot started speaking", self.captured())

    def test_a_traceback_names_the_failing_line_but_not_the_values_on_it(self):
        # loguru's `diagnose` prints the values of the locals on the raising
        # line; pipecat's frame_processor logs exceptions around the frame it
        # was processing, at a name no family cut covers (review of #547).
        journal.install(self.logger, self.capture)
        frame_text = SAID
        try:
            raise ValueError("processing failed for " + frame_text[:0])
        except ValueError:
            self.log_named("pipecat.processors.frame_processor", "exception", "error processing frame")
        self.assertIn("error processing frame", self.captured())
        self.assertIn("ValueError", self.captured())
        self.assertNotIn(SAID, self.captured(), "a diagnosed traceback printed a local's value")


@unittest.skipUnless(HAVE_LOGURU, "loguru is not installed")
class TheLifespan(Loguru):
    def test_the_sink_is_installed_after_the_inner_lifespan_and_its_state_passes_through(self):
        @contextlib.asynccontextmanager
        async def inner(app):
            # An inner lifespan that resets loguru as pipecat's runner does,
            # and yields state as Starlette's lifespan-state form allows.
            self.logger.remove()
            self.logger.add(self.capture, level="DEBUG")
            yield {"started": True}

        wrapped = journal.lifespan(inner, self.logger, self.capture)

        async def serve():
            async with wrapped(object()) as state:
                self.log_named("pipecat.services.tts_service", "debug", f"Generating TTS [{SAID}]")
                self.log_named("pipecat.transports.base_output", "debug", "bot started speaking")
                return state

        state = asyncio.run(serve())
        self.assertEqual(state, {"started": True}, "the inner lifespan's state was dropped")
        self.assertNotIn(SAID, self.captured(), "the inner lifespan's reset won over the filter")
        self.assertIn("bot started speaking", self.captured())


def log_as(module: str, level: str, message: str):
    """Log from inside the real `module`, so the record carries the name
    pipecat gives its own (loguru takes the caller's `__name__`)."""
    mod = importlib.import_module(module)
    exec(f"logger.{level}(message)", vars(mod), {"message": message})


@unittest.skipUnless(HAVE_PIPECAT, "pipecat and fastapi live in the worker's venv")
class TheRealModules(Loguru):
    def test_the_three_that_carried_words_are_named_inside_the_cut(self):
        journal.install(self.logger, self.capture)
        # The three that carried words on this machine (measured 2026-10-04).
        log_as("pipecat.services.tts_service", "debug", f"Generating TTS [{SAID}]")
        log_as("pipecat.services.whisper.base_stt", "debug", f"Transcription: [{SAID}]")
        log_as("pipecat.services.openai.base_llm", "debug", f"Generating chat [{SAID}]")
        log_as("pipecat.transports.base_output", "debug", "bot started speaking")
        self.assertNotIn(SAID, self.captured())
        self.assertIn("bot started speaking", self.captured())

    def test_the_worker_installs_the_sink_at_server_start_after_pipecats_reset(self):
        from fastapi import FastAPI

        import worker

        test = self

        class Capture:
            def write(self, s):
                test.lines.append(s)

            def flush(self):
                pass

        # The worker's sink is the stderr it sees when `install` runs, so the
        # capture stands in for stderr from before that call.
        real_stderr = sys.stderr
        sys.stderr = Capture()
        self.addCleanup(setattr, sys, "stderr", real_stderr)
        app = FastAPI()
        worker.install(app)
        # What pipecat's `main()` does before it serves: replace every handler
        # with an unfiltered DEBUG sink (here, one that captures).
        self.logger.remove()
        self.logger.add(self.capture, level="DEBUG")

        async def serve():
            async with app.router.lifespan_context(app):
                log_as("pipecat.services.tts_service", "debug", f"Generating TTS [{SAID}]")
                log_as("pipecat.transports.base_output", "debug", "bot started speaking")

        asyncio.run(serve())
        self.assertNotIn(SAID, self.captured(), "the lifespan did not replace pipecat's sink")
        self.assertIn("bot started speaking", self.captured())


if __name__ == "__main__":
    unittest.main()
