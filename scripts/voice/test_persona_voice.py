"""A persona call speaks in the persona's voice or not at all
(docs/PERSONA-DESIGN.md §11): the `persona_voice` serve puts in the offer, as
the worker reads it, and the route serve checks a voice against first. Run in
the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_persona_voice.py`."""

import sys
import unittest
import warnings

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import worker  # noqa: E402
from worker import install, persona_voice  # noqa: E402


class PersonaVoice(unittest.TestCase):
    def test_no_persona_voice_is_the_workers_own(self):
        self.assertIsNone(persona_voice({"session": "persona-abc"}))
        self.assertIsNone(persona_voice(None))

    def test_a_bound_voice_carries_what_it_names(self):
        got = persona_voice({"persona_voice": {"voice": "ada", "speed": 1, "cfg_weight": 0.4}})
        self.assertEqual(got, {"voice": "ada", "speed": 1.0, "cfg_weight": 0.4})

    def test_a_malformed_one_is_refused_never_defaulted(self):
        for bad in (
            {"persona_voice": "ada"},
            {"persona_voice": {}},
            {"persona_voice": {"voice": 7}},
            {"persona_voice": {"voice": "ada", "speed": 9.0}},
            {"persona_voice": {"voice": "ada", "speed": True}},
            {"persona_voice": {"voice": "ada", "exaggeration": "loud"}},
            {"persona_voice": {"voice": "ada", "cfg_weight": 1.5}},
        ):
            with self.assertRaises(ValueError, msg=repr(bad)):
                persona_voice(bad)


class VoicesRoute(unittest.TestCase):
    def client(self):
        from fastapi import FastAPI

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            from fastapi.testclient import TestClient

        app = FastAPI()
        install(app)
        return TestClient(app)

    def test_the_route_asks_the_tts_fresh(self):
        asked = []

        def fake(refresh=False):
            asked.append(refresh)
            return ["ada", "default"]

        real, worker.available_voices = worker.available_voices, fake
        try:
            r = self.client().get("/mecha/voices")
        finally:
            worker.available_voices = real
        self.assertEqual(r.json(), {"voices": ["ada", "default"]})
        self.assertEqual(asked, [True], "a voice cloned since the last ask must count")

    def test_an_unreadable_list_says_unknown_not_empty(self):
        real, worker.available_voices = worker.available_voices, lambda refresh=False: None
        try:
            r = self.client().get("/mecha/voices")
        finally:
            worker.available_voices = real
        self.assertEqual(r.json(), {"voices": None})


class SampleRoute(unittest.TestCase):
    """The library's preview speaks a fixed line in a listed voice, and
    refuses before asking the TTS for one it does not list."""

    def setUp(self):
        self.real = (worker.available_voices, worker.tts_sample)
        self.asked = []

        async def fake_sample(voice):
            self.asked.append(voice)
            return b"RIFF....WAVE"

        worker.tts_sample = fake_sample
        self.client = VoicesRoute.client(self)

    def tearDown(self):
        worker.available_voices, worker.tts_sample = self.real

    def listing(self, voices):
        worker.available_voices = lambda refresh=False: voices

    def test_a_listed_voice_is_spoken(self):
        self.listing(["ada"])
        r = self.client.get("/mecha/sample", params={"voice": "ada"})
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers["content-type"], "audio/wav")
        self.assertEqual(r.content, b"RIFF....WAVE")
        self.assertEqual(self.asked, ["ada"])

    def test_an_unlisted_voice_never_reaches_the_tts(self):
        self.listing(["ada"])
        self.assertEqual(self.client.get("/mecha/sample", params={"voice": "nobody"}).status_code, 404)
        self.listing(None)
        self.assertEqual(self.client.get("/mecha/sample", params={"voice": "ada"}).status_code, 503)
        self.assertEqual(self.asked, [])


if __name__ == "__main__":
    unittest.main()
