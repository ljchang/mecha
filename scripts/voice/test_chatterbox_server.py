"""The TTS server says which controls its model honours, and refuses one it
would drop rather than speaking as if it had landed (chatterbox_server.py
`CONTROLS`). The model is a stand-in that records what it was handed, so
this needs no GPU. Run in the server's image, against this checkout:

    docker run --rm --entrypoint python -v "$PWD/scripts/voice:/srv" \\
        mecha/chatterbox:serve /srv/test_chatterbox_server.py
"""

import importlib
import os
import sys
import unittest

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


class Recorded:
    """`wav.squeeze().cpu().numpy()`, as the server reads a generate."""

    def __init__(self, samples):
        self.samples = samples

    def squeeze(self):
        return self

    def cpu(self):
        return self

    def numpy(self):
        return self.samples


class StandIn:
    sr = 24000

    def __init__(self):
        self.calls = []

    def generate(self, text, **kwargs):
        self.calls.append(kwargs)
        return Recorded(np.zeros(2400, dtype=np.float32))


def server(kind):
    """The module as `CHATTERBOX_MODEL=kind` loads it, with a stand-in model.
    Not entered as a context manager, so startup - the real load - never
    runs."""
    os.environ["CHATTERBOX_MODEL"] = kind
    import chatterbox_server

    mod = importlib.reload(chatterbox_server)
    mod.model = StandIn()
    from fastapi.testclient import TestClient

    return mod, TestClient(mod.app)


class Controls(unittest.TestCase):
    def speak(self, client, **extra):
        return client.post("/v1/audio/speech", json={"input": "Hello.", **extra})

    def test_turbo_lists_only_what_it_honours(self):
        _, client = server("turbo")
        listed = client.get("/v1/voices").json()
        self.assertEqual(listed["model"], "chatterbox-turbo")
        self.assertEqual(listed["controls"], ["temperature"])

    def test_turbo_refuses_a_control_it_would_drop(self):
        mod, client = server("turbo")
        for name, value in (("exaggeration", 0.8), ("cfg_weight", 0.3)):
            r = self.speak(client, **{name: value})
            self.assertEqual(r.status_code, 400, name)
            self.assertIn("not honoured", r.json()["detail"])
        self.assertEqual(mod.model.calls, [], "a refused request reached the model")

    def test_turbo_is_never_handed_the_pair(self):
        mod, client = server("turbo")
        self.assertEqual(self.speak(client).status_code, 200)
        self.assertEqual(mod.model.calls, [{"audio_prompt_path": None, "temperature": 0.8}])

    def test_original_honours_the_pair_to_its_own_range(self):
        mod, client = server("original")
        self.assertEqual(
            client.get("/v1/voices").json()["controls"],
            ["temperature", "exaggeration", "cfg_weight"],
        )
        r = self.speak(client, exaggeration=1.5, cfg_weight=0.3)
        self.assertEqual(r.status_code, 200, r.text)
        self.assertEqual(mod.model.calls[-1]["exaggeration"], 1.5)
        self.assertEqual(mod.model.calls[-1]["cfg_weight"], 0.3)
        self.assertEqual(self.speak(client, exaggeration=2.5).status_code, 400)
        self.assertEqual(self.speak(client, cfg_weight=1.5).status_code, 400)

    def test_an_unknown_model_fails_at_import(self):
        with self.assertRaises(RuntimeError):
            server("turbo-ish")

    def test_original_unset_leaves_the_librarys_default(self):
        mod, client = server("original")
        self.assertEqual(self.speak(client).status_code, 200)
        self.assertNotIn("exaggeration", mod.model.calls[-1])
        self.assertNotIn("cfg_weight", mod.model.calls[-1])


if __name__ == "__main__":
    unittest.main()
