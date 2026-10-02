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
        self.assertEqual(speak_request({"text": " Hello. ", "voice": "ada"}, ["ada"]), ("Hello.", "ada", 1.0))
        self.assertEqual(speak_request({"text": "Hi", "speed": 1.2}, None, "default"), ("Hi", "default", 1.2))

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

        async def fake_wav(text, voice, speed=1.0):
            asked.append((text, voice, speed))
            return b"RIFF....WAVE"

        real = (worker.tts_wav, worker.available_voices)
        worker.tts_wav = fake_wav
        worker.available_voices = lambda refresh=False: ["ada"]
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                r = TestClient(app).post("/mecha/speak", json={"text": "the secret plan", "voice": "ada"})
                bad = TestClient(app).post("/mecha/speak", json={"text": "the secret plan", "voice": "nobody"})
        finally:
            worker.tts_wav, worker.available_voices = real
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers["content-type"], "audio/wav")
        self.assertEqual(asked, [("the secret plan", "ada", 1.0)])
        self.assertEqual(bad.status_code, 404)
        self.assertNotIn("secret", out.getvalue(), "the text reached the log")


if __name__ == "__main__":
    unittest.main()
