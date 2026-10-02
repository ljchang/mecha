"""The worker sends only the controls the TTS's model honours, as the server
lists them (worker.py `tts_controls`, `optional_controls`); unknown sends
nothing optional. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_tts_controls.py`."""

import json
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import worker  # noqa: E402
from worker import available_voices, optional_controls, tts_controls  # noqa: E402


def tts_server(listing):
    """A stand-in TTS whose `/v1/voices` answers `listing`."""

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802 - the stdlib's name
            body = json.dumps(listing).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    httpd = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, f"http://127.0.0.1:{httpd.server_address[1]}/v1"


class Controls(unittest.TestCase):
    def listed_by(self, listing):
        httpd, url = tts_server(listing)
        self.addCleanup(httpd.shutdown)
        old = worker.TTS_URL
        worker.TTS_URL = url
        self.addCleanup(setattr, worker, "TTS_URL", old)
        available_voices(refresh=True)
        return tts_controls()

    def test_turbo_is_sent_neither_of_the_pair(self):
        controls = self.listed_by({"voices": ["default"], "controls": ["temperature"]})
        self.assertEqual(controls, frozenset({"temperature"}))
        self.assertEqual(optional_controls(controls, exaggeration=0.8, cfg_weight=0.3), {})

    def test_the_original_is_sent_both(self):
        controls = self.listed_by(
            {"voices": ["default"], "controls": ["temperature", "exaggeration", "cfg_weight"]}
        )
        self.assertEqual(
            optional_controls(controls, exaggeration=0.8, cfg_weight=0.3),
            {"exaggeration": 0.8, "cfg_weight": 0.3},
        )

    def test_a_server_that_lists_no_controls_is_unknown_and_sent_nothing(self):
        controls = self.listed_by({"voices": ["default"]})
        self.assertIsNone(controls)
        self.assertEqual(optional_controls(controls, exaggeration=0.8), {})

    def test_an_unreachable_server_is_unknown(self):
        old = worker.TTS_URL
        worker.TTS_URL = "http://127.0.0.1:9/v1"
        self.addCleanup(setattr, worker, "TTS_URL", old)
        available_voices(refresh=True)
        self.assertIsNone(tts_controls())


if __name__ == "__main__":
    unittest.main()
