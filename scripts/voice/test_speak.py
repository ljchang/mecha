"""A play button's speech (the owner's ask, 2026-10-01): a piece of a reply
in a listed voice, checked before the TTS is asked, and nothing of the text
in the log. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_speak.py`."""

import contextlib
import io
import sys
import unittest
import warnings

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import worker  # noqa: E402
from worker import MAX_SPEAK_CHARS, install, speak_request  # noqa: E402


class Request(unittest.TestCase):
    def test_a_piece_in_a_listed_voice_is_taken(self):
        self.assertEqual(
            speak_request({"text": " Hello. ", "voice": "ada"}, ["ada"], default_speed=1.0), ("Hello.", "ada", 1.0, None)
        )
        self.assertEqual(speak_request({"text": "Hi", "speed": 1.2}, None, "default"), ("Hi", "default", 1.2, None))

    def test_an_unset_speed_is_the_rate_a_call_speaks_at(self):
        # Not 1.0: a box with MECHA_VOICE_TTS_SPEED=1.3 hears its calls at
        # 1.3, and Listen must match (review of #502).
        self.assertEqual(speak_request({"text": "Hi"}, None, "default", 1.3), ("Hi", "default", 1.3, None))
        self.assertEqual(speak_request({"text": "Hi", "speed": None}, None, "default", 1.3)[2], 1.3)

    def test_a_direction_rides_along_bounded(self):
        # Listen's director pass: serve sends the line it was given.
        line = "In your own natural voice: warm and unhurried."
        self.assertEqual(speak_request({"text": "Hi", "instructions": f" {line} "}, None, "d", 1.0)[3], line)
        self.assertIsNone(speak_request({"text": "Hi", "instructions": "  "}, None, "d", 1.0)[3])
        self.assertIsNone(speak_request({"text": "Hi", "instructions": None}, None, "d", 1.0)[3])
        long = "x" * (worker.MAX_INSTRUCTIONS_CHARS + 1)
        for bad in ({"text": "Hi", "instructions": 7}, {"text": "Hi", "instructions": long}):
            self.assertEqual(speak_request(bad, ["ada"])[:2], (None, 400), repr(bad)[:60])

    def test_refusals_say_why_and_never_default_a_named_voice(self):
        self.assertEqual(speak_request({"text": "Hi", "voice": "nobody"}, ["ada"])[:2], (None, 404))
        self.assertEqual(speak_request({"text": "Hi", "voice": "ada"}, None)[:2], (None, 503))
        for bad in (None, [], {"text": ""}, {"text": "  "}, {"text": 7}, {"text": "x" * (MAX_SPEAK_CHARS + 1)},
                    {"text": "Hi", "voice": 3}, {"text": "Hi", "speed": 9}, {"text": "Hi", "speed": True}):
            self.assertEqual(speak_request(bad, ["ada"])[:2], (None, 400), repr(bad)[:60])


class Route(unittest.TestCase):
    def test_the_route_speaks_and_logs_no_words(self):
        from fastapi import FastAPI

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            from fastapi.testclient import TestClient

        app = FastAPI()
        install(app)
        asked = []

        async def fake_wav(text, voice, speed=1.0, instructions=None):
            asked.append((text, voice, speed, instructions))
            return b"RIFF....WAVE"

        real = (worker.tts_wav, worker.available_voices)
        worker.tts_wav = fake_wav
        worker.available_voices = lambda refresh=False: ["ada"]
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                r = TestClient(app).post("/mecha/speak", json={"text": "the secret plan", "voice": "ada"})
                bad = TestClient(app).post("/mecha/speak", json={"text": "the secret plan", "voice": "nobody"})
                directed = TestClient(app).post(
                    "/mecha/speak",
                    json={"text": "the secret plan", "voice": "ada", "instructions": "hushed, conspiratorial"},
                )
        finally:
            worker.tts_wav, worker.available_voices = real
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers["content-type"], "audio/wav")
        self.assertEqual(directed.status_code, 200)
        self.assertEqual(
            asked,
            [("the secret plan", "ada", 1.0, None), ("the secret plan", "ada", 1.0, "hushed, conspiratorial")],
        )
        self.assertEqual(bad.status_code, 404)
        self.assertNotIn("secret", out.getvalue(), "the text reached the log")
        self.assertNotIn("conspiratorial", out.getvalue(), "the direction reached the log")

    def test_directs_says_whether_the_engine_takes_a_direction(self):
        from fastapi import FastAPI

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            from fastapi.testclient import TestClient

        app = FastAPI()
        install(app)
        real = worker.tts_controls
        try:
            for controls, want in (
                (frozenset({"instructions"}), True),
                (frozenset({"exaggeration", "cfg_weight"}), False),
                (None, False),
            ):
                worker.tts_controls = lambda c=controls: c
                r = TestClient(app).get("/mecha/directs")
                self.assertEqual(r.json(), {"directs": want}, repr(controls))
        finally:
            worker.tts_controls = real


if __name__ == "__main__":
    unittest.main()
