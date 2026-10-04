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

    def test_a_streamed_piece_is_raw_pcm_as_made_and_logs_no_words(self):
        # Listen streams (docs/VOICE-BREEZE-DESIGN.md S3.1): the chunks pass
        # through as the TTS makes them, odd bytes and all - the page carries
        # a split sample over - at the rate the header names.
        from fastapi import FastAPI

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            from fastapi.testclient import TestClient

        app = FastAPI()
        install(app)
        asked = []

        async def fake_pcm(text, voice, speed=1.0, instructions=None):
            asked.append((text, voice, speed, instructions))
            if text == "refused":
                raise RuntimeError("the TTS said 500")

            async def chunks():
                for c in (b"\x01\x00\x02", b"\x00\x03\x00"):
                    yield c

            return chunks()

        async def no_wav(*a, **k):
            raise AssertionError("a streamed piece asked for a WAV")

        real = (worker.tts_pcm, worker.tts_wav, worker.available_voices)
        worker.tts_pcm, worker.tts_wav = fake_pcm, no_wav
        worker.available_voices = lambda refresh=False: ["ada"]
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                client = TestClient(app)
                r = client.post("/mecha/speak", json={"text": "the secret plan", "voice": "ada", "stream": True})
                refused = client.post("/mecha/speak", json={"text": "refused", "voice": "ada", "stream": True})
        finally:
            worker.tts_pcm, worker.tts_wav, worker.available_voices = real
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers["content-type"], "audio/pcm")
        self.assertEqual(r.headers["x-sample-rate"], str(worker.PCM_RATE))
        self.assertEqual(r.content, b"\x01\x00\x02\x00\x03\x00")
        self.assertEqual(asked[0], ("the secret plan", "ada", 1.0, None))
        # A refusal is said before any audio, as the WAV path says it.
        self.assertEqual(refused.status_code, 502)
        self.assertIn("did not speak", refused.json()["error"])
        self.assertNotIn("secret", out.getvalue(), "the text reached the log")

    def test_only_stream_true_streams(self):
        # A serve older than streaming sends no flag and must get its WAV;
        # a truthy non-boolean is not a request to stream either.
        from fastapi import FastAPI

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            from fastapi.testclient import TestClient

        app = FastAPI()
        install(app)

        async def fake_wav(text, voice, speed=1.0, instructions=None):
            return b"RIFF....WAVE"

        async def no_pcm(*a, **k):
            raise AssertionError("streamed without stream: true")

        real = (worker.tts_pcm, worker.tts_wav, worker.available_voices)
        worker.tts_pcm, worker.tts_wav = no_pcm, fake_wav
        worker.available_voices = lambda refresh=False: ["ada"]
        try:
            client = TestClient(app)
            for stream in (None, False, "true", 1):
                body = {"text": "Hi", "voice": "ada"} | ({} if stream is None else {"stream": stream})
                r = client.post("/mecha/speak", json=body)
                self.assertEqual((r.status_code, r.headers["content-type"]), (200, "audio/wav"), repr(stream))
        finally:
            worker.tts_pcm, worker.tts_wav, worker.available_voices = real

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


class Pcm(unittest.IsolatedAsyncioTestCase):
    """`tts_pcm` against a TTS faked at the transport: what it asks for,
    that a refusal raises before a byte, and that dropping the chunks closes
    the TTS request - a stop on the page stops the synthesis."""

    async def asyncSetUp(self):
        import json

        import httpx

        self.closed = []
        self.bodies = []
        self.reasked = []
        test = self

        class Body(httpx.AsyncByteStream):
            async def __aiter__(self):
                for c in (b"\x01\x00", b"\x02", b"\x00"):
                    yield c

            async def aclose(self):
                test.closed.append(True)

        def handler(request):
            body = json.loads(request.content)
            self.bodies.append(body)
            return httpx.Response(500 if body["input"] == "refused" else 200, stream=Body())

        async def reask(status):
            self.reasked.append(status)

        real_client = httpx.AsyncClient
        self.real = (worker.httpx.AsyncClient, worker.tts_controls, worker.reask_after_refusal)
        worker.httpx.AsyncClient = lambda **kw: real_client(transport=httpx.MockTransport(handler), **kw)
        worker.tts_controls = lambda: frozenset({"instructions"})
        worker.reask_after_refusal = reask

    async def asyncTearDown(self):
        worker.httpx.AsyncClient, worker.tts_controls, worker.reask_after_refusal = self.real

    async def test_chunks_pass_through_and_close_the_request(self):
        chunks = await worker.tts_pcm("Hello there.", "ada", 1.0, "warm")
        self.assertEqual(b"".join([c async for c in chunks]), b"\x01\x00\x02\x00")
        self.assertEqual(self.bodies[0]["response_format"], "pcm")
        self.assertEqual(self.bodies[0]["instructions"], "warm")
        self.assertEqual(self.closed, [True])

    async def test_a_refusal_raises_before_a_byte(self):
        with self.assertRaises(Exception):
            await worker.tts_pcm("refused", "ada")
        self.assertEqual(self.reasked, [500])
        self.assertEqual(self.closed, [True])

    async def test_a_dropped_stream_closes_the_request(self):
        chunks = await worker.tts_pcm("Hello there.", "ada")
        self.assertEqual(await chunks.__anext__(), b"\x01\x00")
        await chunks.aclose()
        self.assertEqual(self.closed, [True])


if __name__ == "__main__":
    unittest.main()
