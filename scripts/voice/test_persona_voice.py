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


if __name__ == "__main__":
    unittest.main()
