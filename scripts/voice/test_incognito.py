"""A voice call into an incognito chat keeps no words (docs/INCOGNITO-DESIGN.md
§3.4): the log silence `Unlogged` holds, the session name that decides it, and
the worker's own lines that keep their measurements and lose the words. Run
in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_incognito.py`."""

import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from loguru import logger  # noqa: E402

from worker import (  # noqa: E402
    Unlogged,
    is_incognito,
    named_chat_session,
    spoken_words,
    UNLOGGED,
)

# The shape `incognito::new_key` mints: the prefix and 22 hex digits, 32
# characters — exactly the length limit, so a test that it passes is a test
# that nobody tightens the limit under it.
INCOGNITO_KEY = "incognito-0123456789abcdef012345"
SECRET = "the words nobody is meant to keep"


def log_as(module: str, message: str):
    """Log from `module`'s namespace. loguru names a record by the caller's
    `__name__`, which is what `logger.disable` matches, so this is the line
    pipecat's STT service writes as far as the silence can tell."""
    exec("logger.debug(message)", {"__name__": module, "logger": logger, "message": message})


class Captured:
    """The runner's own move (`pipecat.runner.run`: `logger.remove()`, then a
    fresh DEBUG sink), with the sink capturing — so the silence is measured
    against the sink that is actually live in the worker, not one we added."""

    def __enter__(self):
        self.lines = []
        logger.remove()
        self._id = logger.add(lambda m: self.lines.append(str(m)), level="DEBUG")
        return self

    def __exit__(self, *exc):
        logger.remove(self._id)
        logger.add(sys.stderr, level="DEBUG")

    def text(self) -> str:
        return "".join(self.lines)


class TheSilence(unittest.TestCase):
    def test_an_ordinary_call_logs_as_it_always_has(self):
        # The non-vacuous half: without this, "nothing reached the sink"
        # below could be a sink that never received anything.
        with Captured() as cap:
            log_as("pipecat.services.whisper.base_stt", f"Transcription: {SECRET}")
        self.assertIn(SECRET, cap.text())

    def test_pipecat_says_nothing_while_an_incognito_call_is_live(self):
        u = Unlogged()
        with Captured() as cap:
            with u.held(True):
                log_as("pipecat.services.whisper.base_stt", f"Transcription: {SECRET}")
                log_as("pipecat.services.tts_service", f"LocalTTS#1: [{SECRET}]")
            log_as("pipecat.services.whisper.base_stt", "after the call")
        self.assertNotIn(SECRET, cap.text())
        self.assertIn("after the call", cap.text(), "the silence outlived the call")

    def test_the_silence_survives_the_runner_replacing_its_sink(self):
        # Why it is `logger.disable` and not a sink filter: the runner swaps
        # sinks after this module is imported, and a filter would go with it.
        u = Unlogged()
        with u.held(True):
            with Captured() as cap:
                log_as("pipecat.services.whisper.base_stt", f"Transcription: {SECRET}")
        self.assertNotIn(SECRET, cap.text())

    def test_the_first_call_to_end_does_not_lift_the_second_calls_silence(self):
        u = Unlogged()
        with Captured() as cap:
            u.enter()
            u.enter()
            u.exit()
            log_as("pipecat.services.whisper.base_stt", f"Transcription: {SECRET}")
            self.assertTrue(u.active)
            u.exit()
            self.assertFalse(u.active)
        self.assertNotIn(SECRET, cap.text())

    def test_a_call_that_fails_still_releases_the_silence(self):
        u = Unlogged()
        with self.assertRaises(RuntimeError):
            with u.held(True):
                raise RuntimeError("the call died")
        self.assertFalse(u.active)
        with Captured() as cap:
            log_as("pipecat.services.whisper.base_stt", "logging again")
        self.assertIn("logging again", cap.text())

    def test_an_ordinary_call_holds_nothing(self):
        u = Unlogged()
        with u.held(False):
            self.assertFalse(u.active)


class TheWorkersOwnLines(unittest.TestCase):
    def test_words_are_withheld_while_an_incognito_call_is_live(self):
        self.assertEqual(spoken_words(SECRET, 80), repr(SECRET))
        with UNLOGGED.held(True):
            self.assertNotIn(SECRET, spoken_words(SECRET, 80))
        self.assertEqual(spoken_words(SECRET, 8), repr(SECRET[:8]))


class TheSessionName(unittest.TestCase):
    def test_an_incognito_key_passes_and_is_recognised(self):
        self.assertEqual(len(INCOGNITO_KEY), 32)
        self.assertEqual(named_chat_session({"session": INCOGNITO_KEY}), INCOGNITO_KEY)
        self.assertTrue(is_incognito(INCOGNITO_KEY))

    def test_an_ordinary_chat_is_not_incognito(self):
        self.assertEqual(named_chat_session({"session": "main"}), "main")
        self.assertFalse(is_incognito("main"))
        self.assertFalse(is_incognito(None))

    def test_malformed_names_are_refused(self):
        for bad in ["", "-x", "Main", "a/b", "x" * 33, "incognito-ÿ", 7]:
            self.assertIsNone(named_chat_session({"session": bad}), bad)
        self.assertIsNone(named_chat_session({}))
        self.assertIsNone(named_chat_session(None))


if __name__ == "__main__":
    unittest.main()
