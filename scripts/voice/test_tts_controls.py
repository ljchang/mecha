"""The worker sends only the controls the TTS's model honours, as the server
lists them (worker.py `tts_controls`, `optional_controls`); unknown sends
nothing optional. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_tts_controls.py`."""

import asyncio
import json
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import worker  # noqa: E402
from worker import available_voices, optional_controls, tts_controls  # noqa: E402


def tts_server(listing, spoken=None, asked=None):
    """A stand-in TTS whose `/v1/voices` answers `listing`, counting each ask
    in `asked`, and whose speech route keeps each body in `spoken`."""

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802 - the stdlib's name
            if asked is not None:
                asked.append(self.path)
            body = json.dumps(listing).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):  # noqa: N802
            length = int(self.headers.get("content-length", 0))
            if spoken is not None:
                spoken.append(json.loads(self.rfile.read(length)))
            self.send_response(200)
            self.send_header("content-type", "audio/wav")
            self.end_headers()
            self.wfile.write(b"RIFF")

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

    def test_a_failed_ask_is_retried_only_after_the_interval(self):
        old = worker.TTS_URL
        worker.TTS_URL = "http://127.0.0.1:9/v1"
        self.addCleanup(setattr, worker, "TTS_URL", old)
        available_voices(refresh=True)
        self.assertIsNone(tts_controls())
        asked = []
        httpd, url = tts_server({"voices": ["default"], "controls": ["temperature"]}, asked=asked)
        self.addCleanup(httpd.shutdown)
        worker.TTS_URL = url
        # Within the interval a dead TTS is not asked again per sentence ...
        self.assertIsNone(tts_controls())
        self.assertEqual(asked, [])
        # ... and after it, "could not ask" has not latched as "honours
        # nothing": the TTS that came back is asked and believed.
        worker._controls_asked_at -= worker.CONTROLS_RETRY_SECS
        self.assertEqual(tts_controls(), frozenset({"temperature"}))
        self.assertEqual(len(asked), 1)

    def test_a_worker_restarted_first_learns_the_new_servers_controls(self):
        # The restart order: the worker comes up against the old server,
        # which lists voices and no controls ...
        listing = {"voices": ["default"]}
        asked = []
        httpd, url = tts_server(listing, asked=asked)
        self.addCleanup(httpd.shutdown)
        old = worker.TTS_URL
        worker.TTS_URL = url
        self.addCleanup(setattr, worker, "TTS_URL", old)
        available_voices(refresh=True)
        self.assertIsNone(tts_controls())
        # ... then the container restarts on the new build. Listing voices
        # must not have latched "unknown": after the interval it is asked.
        listing["controls"] = ["temperature", "exaggeration", "cfg_weight"]
        worker._controls_asked_at -= worker.CONTROLS_RETRY_SECS
        self.assertEqual(
            tts_controls(), frozenset({"temperature", "exaggeration", "cfg_weight"})
        )

    def test_only_a_refusal_is_asked_again(self):
        asked = []
        httpd, url = tts_server({"voices": ["default"], "controls": ["temperature"]}, asked=asked)
        self.addCleanup(httpd.shutdown)
        old = worker.TTS_URL
        worker.TTS_URL = url
        self.addCleanup(setattr, worker, "TTS_URL", old)
        for status in (None, 500, 503):
            asyncio.run(worker.reask_after_refusal(status))
        self.assertEqual(asked, [], "a dead or failing TTS was asked on the error path")
        asyncio.run(worker.reask_after_refusal(400))
        self.assertEqual(len(asked), 1)

    def test_unknown_is_labelled_unknown_never_false(self):
        self.assertEqual(worker.controls_label(None, "cfg_weight"), "unknown")
        self.assertEqual(worker.controls_label(frozenset({"temperature"}), "cfg_weight"), "False")
        self.assertEqual(worker.controls_label(frozenset({"cfg_weight"}), "cfg_weight"), "True")

    def wav_body(self, listing):
        """The body `tts_wav` (the library preview, the play button) sends
        to a TTS listing `listing`."""
        spoken = []
        httpd, url = tts_server(listing, spoken=spoken)
        self.addCleanup(httpd.shutdown)
        old = worker.TTS_URL
        worker.TTS_URL = url
        self.addCleanup(setattr, worker, "TTS_URL", old)
        available_voices(refresh=True)
        asyncio.run(worker.tts_wav("Hello.", "default"))
        return spoken[-1]

    def test_the_preview_sends_turbo_neither_of_the_pair(self):
        body = self.wav_body({"voices": ["default"], "controls": ["temperature"]})
        self.assertNotIn("exaggeration", body)
        self.assertNotIn("cfg_weight", body)

    def test_the_preview_sends_the_original_both(self):
        body = self.wav_body(
            {"voices": ["default"], "controls": ["temperature", "exaggeration", "cfg_weight"]}
        )
        self.assertEqual(body["exaggeration"], worker.TTS_EXAGGERATION)
        self.assertEqual(body["cfg_weight"], worker.TTS_CFG_WEIGHT)


if __name__ == "__main__":
    unittest.main()
