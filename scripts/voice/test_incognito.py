"""A voice call into an incognito chat keeps no words (docs/INCOGNITO-DESIGN.md
§3.4): the log silence `Unlogged` holds, the session name that decides it, and
the worker's own lines that keep their measurements and lose the words. Run
in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_incognito.py`."""

import asyncio
import importlib
import sys
import unittest
import warnings

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from loguru import logger  # noqa: E402

from echo_filter import BotSpeech  # noqa: E402
from worker import (  # noqa: E402
    LocalTTS,
    OpenAITTSService,
    Unlogged,
    is_incognito,
    named_chat_session,
    names_incognito,
    session_line,
    spoken_words,
    UNLOGGED,
    install,
)

# The shape `incognito::new_key` mints: the prefix and 22 hex digits, 32
# characters — exactly the length limit, so a test that it passes is a test
# that nobody tightens the limit under it.
INCOGNITO_KEY = "incognito-0123456789abcdef012345"
SECRET = "the words nobody is meant to keep"


def log_as(module: str, message: str):
    """Log from inside the real `module`: its own globals and its own
    `logger`, so the record is named the way pipecat's are rather than by a
    string this test chose. loguru names a record by the caller's
    `__name__`, and that name is what `logger.disable` matches."""
    mod = importlib.import_module(module)
    exec("logger.debug(message)", vars(mod), {"message": message})


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


class TheChatsName(unittest.TestCase):
    INCOGNITO_AFFECT_KEY = f"chat:{INCOGNITO_KEY}"

    @staticmethod
    def latch_lines(affect_key: str) -> list[str]:
        """The affect latch, driven on the real class once, as the pipeline
        does per answer, against a facade that is not there."""
        tts = LocalTTS(
            api_key="unused",
            base_url="http://127.0.0.1:9/v1",
            settings=OpenAITTSService.Settings(voice="x", model="tts"),
            echo_window=BotSpeech(),
        )
        tts.set_affect_key(affect_key)
        with Captured() as cap:
            asyncio.run(tts.on_turn_context_created("ctx-1"))
        return [line for line in cap.lines if "voice affect latch" in line]

    def test_the_affect_latch_says_nothing_of_an_incognito_chat(self):
        # `LocalTTS` lives in the worker, so its records are not `pipecat`'s
        # and the silence does not reach them. Redacting the key would still
        # say an incognito chat was spoken into, so the line is not written.
        lines = self.latch_lines(self.INCOGNITO_AFFECT_KEY)
        self.assertEqual(lines, [])

    def test_the_affect_latch_still_logs_every_other_call(self):
        # The non-vacuous half: the latch line is how a hook that stopped
        # firing is caught, so an ordinary call must still write it.
        lines = self.latch_lines("chat:main")
        self.assertEqual(len(lines), 1)
        self.assertIn("key=chat:main", lines[0])

    def test_a_call_into_an_incognito_chat_starts_as_one_that_named_none(self):
        line = session_line("webrtc-1a2b", INCOGNITO_KEY)
        self.assertEqual(line, session_line("webrtc-1a2b", None))
        self.assertNotIn("incognito", line)
        self.assertIn("'main'", session_line("webrtc-1a2b", "main"))

    def test_only_an_incognito_chat_key_is_recognised(self):
        self.assertTrue(names_incognito(self.INCOGNITO_AFFECT_KEY))
        self.assertFalse(names_incognito("chat:main"))
        self.assertFalse(names_incognito("voice:webrtc-1a2b"))
        self.assertFalse(names_incognito(None))


class TheRunnersDoor(unittest.TestCase):
    """What `install` adds to the runner's app: the vouch `mecha serve` asks
    for, and the silence over the offer itself. On a fresh app with a stand-
    in `/api/offer`, since the runner's own needs a peer connection."""

    def client(self):
        from fastapi import FastAPI, Request

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            from fastapi.testclient import TestClient

        app = FastAPI()
        self.during = []

        @app.post("/api/offer")
        async def offer(request: Request):
            # What the runner's handler would see: the body, whole, and the
            # silence, held or not, while it runs.
            self.during.append((UNLOGGED.active, await request.json()))
            return {"sdp": "v=0", "type": "answer"}

        install(app)
        return TestClient(app)

    def test_the_worker_vouches_for_itself(self):
        r = self.client().get("/mecha/unlogged")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.json(), {"unlogged": True})

    def test_an_incognito_offer_is_handled_inside_the_silence(self):
        body = {"sdp": "x", "type": "offer", "request_data": {"session": INCOGNITO_KEY}}
        r = self.client().post("/api/offer", json=body)
        self.assertEqual(r.status_code, 200)
        self.assertEqual(self.during, [(True, body)], "held, and the body replayed intact")
        self.assertFalse(UNLOGGED.active, "the offer's silence outlived it")

    def test_an_ordinary_offer_is_not(self):
        body = {"sdp": "x", "type": "offer", "request_data": {"session": "main"}}
        self.client().post("/api/offer", json=body)
        self.assertEqual(self.during, [(False, body)])


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
