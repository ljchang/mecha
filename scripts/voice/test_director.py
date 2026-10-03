"""The worker asks the facade's director how each sentence should sound, and
sends it as `instructions` - only when the TTS lists that control, once per
sentence with its index in the answer, speaking undirected on any failure,
and never writing a sentence or a direction to the journal (worker.py
`LocalTTS.direction_for`, `run_tts`). Against stand-ins for the TTS and the
facade, through the real `run_tts`. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_director.py`."""

import asyncio
import json
import sys
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from loguru import logger  # noqa: E402
from pipecat.services.openai.tts import OpenAITTSService  # noqa: E402

import worker  # noqa: E402
from echo_filter import BotSpeech  # noqa: E402
from worker import LocalTTS, available_voices  # noqa: E402

SECRET = "the rendezvous is at the old mill"
DIRECTION = "hushed and conspiratorial, slowing at the end"


def serve(handler):
    httpd = HTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, f"http://127.0.0.1:{httpd.server_address[1]}"


class Stand:
    """The TTS (`/v1/voices`, `/v1/audio/speech`) and the facade's director
    (`/v1/mecha-direct`), each request kept."""

    def __init__(self, controls, direct_status=200, direct_delay=0.0):
        self.spoken, self.asked = [], []
        stand = self

        class Tts(BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802
                body = json.dumps({"voices": ["default"], "controls": controls}).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):  # noqa: N802
                n = int(self.headers.get("content-length", 0))
                stand.spoken.append(json.loads(self.rfile.read(n)))
                self.send_response(200)
                self.send_header("content-type", "audio/pcm")
                self.end_headers()
                self.wfile.write(b"\x00\x00" * 2400)

            def log_message(self, *a):
                pass

        class Facade(BaseHTTPRequestHandler):
            def do_POST(self):  # noqa: N802
                n = int(self.headers.get("content-length", 0))
                stand.asked.append(json.loads(self.rfile.read(n)))
                time.sleep(direct_delay)
                body = json.dumps({"direction": DIRECTION, "outcome": "ok"}).encode()
                self.send_response(direct_status)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.tts, tts_url = serve(Tts)
        self.facade, facade_url = serve(Facade)
        self.tts_url = tts_url + "/v1"
        self.direct_url = facade_url + "/v1/mecha-direct"

    def close(self):
        self.tts.shutdown()
        self.facade.shutdown()


class Director(unittest.TestCase):
    def setUp(self):
        self.lines = []
        logger.remove()
        self._sink = logger.add(lambda m: self.lines.append(str(m)), level="DEBUG")

    def tearDown(self):
        logger.remove(self._sink)
        logger.add(sys.stderr, level="WARNING")

    def stand(self, controls, **kw):
        s = Stand(controls, **kw)
        self.addCleanup(s.close)
        for name, value in (("TTS_URL", s.tts_url), ("DIRECT_URL", s.direct_url)):
            old = getattr(worker, name)
            setattr(worker, name, value)
            self.addCleanup(setattr, worker, name, old)
        available_voices(refresh=True)
        return s

    def speak(self, sentences, key="chat:main", context="ctx-1"):
        """Speak `sentences` through one LocalTTS; a sentence given as
        (text, context) moves to that context, as a new answer does."""
        tts = LocalTTS(
            api_key="unused",
            base_url=worker.TTS_URL,
            settings=OpenAITTSService.Settings(voice="default", model="tts"),
            echo_window=BotSpeech(),
        )
        tts.set_affect_key(key)

        async def run():
            for item in sentences:
                text, ctx = item if isinstance(item, tuple) else (item, context)
                async for _ in tts.run_tts(text, ctx):
                    pass

        asyncio.run(run())

    def test_no_direction_is_asked_for_a_tts_that_drops_it(self):
        s = self.stand(["temperature"])
        self.speak(["Hello there."])
        self.assertEqual(s.asked, [])
        self.assertNotIn("instructions", s.spoken[0])

    def test_each_sentence_is_directed_once_with_its_index(self):
        s = self.stand(["temperature", "instructions"])
        self.speak(["Oh, you did it! ", "Go rest.", "Promise me."])
        self.assertEqual([a["index"] for a in s.asked], [0, 1, 2])
        self.assertEqual(s.asked[0]["session"], "chat:main")
        self.assertEqual(s.asked[0]["sentence"], "Oh, you did it!")
        self.assertEqual([b.get("instructions") for b in s.spoken], [DIRECTION] * 3)

    def test_a_new_answer_counts_from_zero(self):
        s = self.stand(["instructions"])
        self.speak([("One.", "ctx-1"), ("Two.", "ctx-1"), ("Three.", "ctx-2")])
        self.assertEqual([a["index"] for a in s.asked], [0, 1, 0])
        self.assertEqual([a["context"] for a in s.asked], ["ctx-1", "ctx-1", "ctx-2"])

    def test_a_failed_direction_still_speaks_undirected(self):
        s = self.stand(["instructions"], direct_status=500)
        self.speak(["Hello."])
        self.assertEqual(len(s.spoken), 1)
        self.assertNotIn("instructions", s.spoken[0])

    def test_a_slow_director_is_not_waited_for(self):
        s = self.stand(["instructions"], direct_delay=0.4)
        old = worker.DIRECT_FIRST_TIMEOUT_SECONDS
        worker.DIRECT_FIRST_TIMEOUT_SECONDS = 0.1
        self.addCleanup(setattr, worker, "DIRECT_FIRST_TIMEOUT_SECONDS", old)
        started = time.monotonic()
        self.speak(["Hello."])
        self.assertLess(time.monotonic() - started, 0.4)
        self.assertNotIn("instructions", s.spoken[0])

    def test_the_journal_never_carries_the_words_or_the_direction(self):
        self.stand(["instructions"])
        self.speak([f"Remember, {SECRET}."])
        text = "".join(self.lines)
        self.assertIn("voice direction: index=0 outcome=ok", text)  # non-vacuous
        self.assertNotIn(SECRET, text)
        self.assertNotIn(DIRECTION, text)

    def test_an_incognito_call_is_directed_and_leaves_no_line(self):
        s = self.stand(["instructions"])
        key = "chat:incognito-0123456789abcdef012345"
        self.speak([f"Remember, {SECRET}."], key=key)
        self.assertEqual(len(s.asked), 1, "the director runs in incognito")
        self.assertEqual(s.spoken[0].get("instructions"), DIRECTION)
        text = "".join(self.lines)
        self.assertNotIn("voice direction", text)
        self.assertNotIn("incognito-0123", text)


class Streams(unittest.TestCase):
    """`"streams": true` in the TTS's `/v1/voices` is what sends
    `X-Voice-TTS-Streams: 1`; absent or anything else sends nothing."""

    def listed(self, listing):
        class Tts(BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802
                body = json.dumps(listing).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        httpd, url = serve(Tts)
        self.addCleanup(httpd.shutdown)
        old = worker.TTS_URL
        worker.TTS_URL = url + "/v1"
        self.addCleanup(setattr, worker, "TTS_URL", old)
        available_voices(refresh=True)
        return worker.tts_streams(), worker.tts_controls()

    def test_a_streaming_tts_is_said_to_the_facade(self):
        streams, controls = self.listed(
            {"voices": ["default"], "controls": ["instructions"], "streams": True}
        )
        self.assertTrue(streams)
        self.assertEqual(
            worker.tts_headers(controls, streams),
            {"X-Voice-Directed": "1", "X-Voice-TTS-Streams": "1"},
        )

    def test_chatterbox_lists_no_stream_and_sends_neither(self):
        streams, controls = self.listed({"voices": ["default"], "controls": ["temperature"]})
        self.assertFalse(streams)
        self.assertEqual(worker.tts_headers(controls, streams), {})

    def test_only_an_explicit_true_streams(self):
        for value in ("true", 1, "yes", None):
            streams, _ = self.listed({"voices": ["default"], "streams": value})
            self.assertFalse(streams, value)


if __name__ == "__main__":
    unittest.main()
