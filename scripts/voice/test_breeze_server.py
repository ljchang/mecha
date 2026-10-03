"""The Breeze adapter presents chatterbox_server's surface over qwentts.cpp's
tts-server: voices with Parakeet-written transcripts, registration that
survives the engine forgetting, honoured controls listed and the rest refused,
`default` as a clip like any other (default.wav), and speed as a stretch. Against stand-ins for
the engine and the STT, so no GPU. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_breeze_server.py`."""

import base64
import importlib
import json
import os
import sys
import tempfile
import threading
import unittest
import wave
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

RATE = 24000
PCM = (b"\x10\x00\x20\x00" * 6000)  # 0.5 s of s16le


def wav_file(path):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(PCM)


class Engine:
    """tts-server and Parakeet, recorded."""

    def __init__(self):
        self.registered = {}
        self.spoken = []
        self.transcribed = 0
        self.forget_once = False
        self.silent = False
        self.refused = 0
        self.registry_readable = True
        engine = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _body(self):
                n = int(self.headers.get("content-length", 0))
                return self.rfile.read(n)

            def _send(self, code, data, ctype="application/json"):
                self.send_response(code)
                self.send_header("content-type", ctype)
                self.send_header("content-length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                if self.path == "/v1/audio/voices":
                    if not engine.registry_readable:
                        return self._send(404, b"{}")
                    names = [{"name": n, "kind": "registered"} for n in engine.registered]
                    return self._send(200, json.dumps({"voices": names}).encode())
                self._send(200, b'{"status":"ok"}')

            def do_POST(self):
                body = self._body()
                if self.path == "/v1/audio/transcriptions":
                    engine.transcribed += 1
                    return self._send(200, json.dumps({"text": "Please call Stella."}).encode())
                if self.path == "/v1/audio/voices":
                    v = json.loads(body)
                    engine.registered[v["name"]] = v
                    return self._send(200, b"{}")
                if self.path == "/v1/audio/speech":
                    req = json.loads(body)
                    if engine.forget_once:
                        engine.forget_once = False
                        engine.registered.clear()
                    if req["voice"] not in engine.registered:
                        # Worded unlike the real engine on purpose: the
                        # adapter must ask the registry, not read the text.
                        return self._send(400, b'{"error":{"message":"synthesis refused"}}')
                    if req["input"] == "refuse me":
                        engine.refused += 1
                        return self._send(400, b'{"error":"empty text after normalisation"}')
                    engine.spoken.append(req)
                    return self._send(200, b"" if engine.silent else PCM, "audio/pcm")
                self._send(404, b"{}")

        self.httpd = HTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"


class Adapter(unittest.TestCase):
    def setUp(self):
        self.engine = Engine()
        self.addCleanup(self.engine.httpd.shutdown)
        self.voices = tempfile.mkdtemp()
        for name in ("default", "vctk_p297"):
            wav_file(os.path.join(self.voices, f"{name}.wav"))
        saved = dict(os.environ)
        self.addCleanup(lambda: (os.environ.clear(), os.environ.update(saved)))
        os.environ.update({
            "BREEZE_TTS_URL": self.engine.url,
            "MECHA_VOICE_STT": self.engine.url + "/v1",
            "VOICES_DIR": self.voices,
        })
        import breeze_server

        self.mod = importlib.reload(breeze_server)
        from fastapi.testclient import TestClient

        self.client = TestClient(self.mod.app)

    def speak(self, **extra):
        return self.client.post("/v1/audio/speech", json={"input": "Hello.", "response_format": "pcm", **extra})

    def test_voices_list_default_and_the_library_and_the_honoured_controls(self):
        v = self.client.get("/v1/voices").json()
        self.assertEqual(v["voices"], ["default", "vctk_p297"])  # default.wav leads, once
        self.assertEqual(v["controls"], ["temperature", "instructions"])
        self.assertIs(v["streams"], True)

    def test_default_is_not_offered_without_its_clip(self):
        os.remove(os.path.join(self.voices, "default.wav"))
        self.assertEqual(self.client.get("/v1/voices").json()["voices"], ["vctk_p297"])

    def test_temperature_out_of_range_is_refused_not_clamped(self):
        self.assertEqual(self.speak(temperature=2.5).status_code, 400)
        self.assertEqual(self.speak(temperature=0.0).status_code, 200)
        self.assertEqual(self.engine.spoken[-1]["temperature"], 0.0)

    def test_an_engine_refusal_is_said_not_retried_as_amnesia(self):
        self.speak(voice="vctk_p297")
        r = self.client.post("/v1/audio/speech", json={"input": "refuse me", "voice": "vctk_p297", "response_format": "pcm"})
        self.assertEqual(r.status_code, 502)
        self.assertIn("empty text", r.json()["detail"])
        self.assertEqual(self.engine.refused, 1, "a deterministic refusal was asked twice")

    def test_default_speaks_as_its_own_clip(self):
        r = self.speak()
        self.assertEqual(r.status_code, 200, r.text)
        self.assertEqual(self.engine.spoken[-1]["voice"], "default")

    def test_a_missing_default_voice_is_said_not_substituted(self):
        os.remove(os.path.join(self.voices, "default.wav"))
        r = self.speak()
        self.assertEqual(r.status_code, 503)
        self.assertIn("default voice is missing", r.json()["detail"])
        self.assertEqual(self.engine.spoken, [])

    def test_an_unknown_voice_is_refused(self):
        self.assertEqual(self.speak(voice="nobody").status_code, 400)

    def test_a_voice_is_transcribed_once_and_registered_with_its_text(self):
        self.speak(voice="vctk_p297")
        self.speak(voice="vctk_p297")
        self.assertEqual(self.engine.transcribed, 1, "transcribed again instead of reading the sidecar")
        txt = os.path.join(self.voices, "vctk_p297.txt")
        self.assertEqual(open(txt).read().strip(), "Please call Stella.")
        reg = self.engine.registered["vctk_p297"]
        self.assertEqual(reg["ref_text"], "Please call Stella.")
        self.assertTrue(base64.b64decode(reg["wav_b64"]).startswith(b"RIFF"))

    def test_an_edited_sidecar_is_what_is_registered(self):
        with open(os.path.join(self.voices, "vctk_p297.txt"), "w") as f:
            f.write("Corrected words.\n")
        self.speak(voice="vctk_p297")
        self.assertEqual(self.engine.transcribed, 0)
        self.assertEqual(self.engine.registered["vctk_p297"]["ref_text"], "Corrected words.")

    def test_a_sidecar_edited_after_registration_is_registered_again(self):
        self.speak(voice="vctk_p297")
        txt = os.path.join(self.voices, "vctk_p297.txt")
        with open(txt, "w") as f:
            f.write("Corrected words.\n")
        st = os.stat(txt)
        os.utime(txt, (st.st_atime, st.st_mtime + 5))  # a later edit, not the same tick
        self.speak(voice="vctk_p297")
        self.assertEqual(self.engine.registered["vctk_p297"]["ref_text"], "Corrected words.")

    def test_a_warm_voice_does_not_wait_on_a_cold_ones_lock(self):
        self.speak(voice="vctk_p297")
        import asyncio

        async def warm_while_locked():
            async with self.mod._lock:
                import httpx
                async with httpx.AsyncClient() as c:
                    await asyncio.wait_for(self.mod.ensure_registered(c, "vctk_p297"), 1)

        asyncio.run(warm_while_locked())

    def test_an_empty_answer_is_an_error_not_silence(self):
        self.engine.silent = True
        for r in (self.speak(), self.speak(speed=1.5),
                  self.client.post("/v1/audio/speech", json={"input": "Hi.", "response_format": "wav"})):
            self.assertEqual(r.status_code, 502)
            self.assertIn("no audio", r.json()["detail"])

    def test_nothing_speakable_is_a_pause_and_never_reaches_the_engine(self):
        pause = int(RATE * self.mod.PAUSE_SECONDS) * 2
        for text in (".", "...", "—", "*", "?!", "  "):
            r = self.client.post("/v1/audio/speech", json={"input": text, "response_format": "pcm"})
            self.assertEqual(r.status_code, 200, text)
            self.assertEqual(r.content, b"\x00" * pause, text)
        r = self.client.post("/v1/audio/speech", json={"input": ".", "response_format": "pcm", "speed": 1.5})
        self.assertEqual(len(r.content), int(RATE * self.mod.PAUSE_SECONDS / 1.5) * 2, "speed was ignored")
        r = self.client.post("/v1/audio/speech", json={"input": ".", "response_format": "wav"})
        self.assertTrue(r.content.startswith(b"RIFF"))
        self.assertEqual(self.engine.spoken, [], "the engine was given nothing to say, and it invents")
        for text in ("Mm.", "Hmm...", "3.", "好。"):
            self.speak(input=text)
        self.assertEqual([s["input"] for s in self.engine.spoken], ["Mm.", "Hmm...", "3.", "好。"])
        self.assertEqual(self.speak(input=".", voice="nobody").status_code, 400, "a pause skipped the voice check")

    def test_an_unreadable_voices_directory_is_said(self):
        os.environ["VOICES_DIR"] = os.path.join(self.voices, "missing")
        self.mod = importlib.reload(self.mod)
        from fastapi.testclient import TestClient

        r = TestClient(self.mod.app).post("/v1/audio/speech", json={"input": "Hi."})
        self.assertEqual(r.status_code, 503)
        self.assertIn("cannot read the voices directory", r.json()["detail"])

    def test_an_unreadable_registry_still_recovers_a_forgotten_voice(self):
        self.speak(voice="vctk_p297")
        self.engine.registry_readable = False
        self.engine.forget_once = True
        r = self.speak(voice="vctk_p297")
        self.assertEqual(r.status_code, 200, r.text)

    def test_an_unreachable_stt_is_named(self):
        os.environ["MECHA_VOICE_STT"] = "http://127.0.0.1:9/v1"
        self.mod = importlib.reload(self.mod)
        from fastapi.testclient import TestClient

        r = TestClient(self.mod.app).post("/v1/audio/speech", json={"input": "Hi.", "voice": "vctk_p297"})
        self.assertEqual(r.status_code, 503)
        self.assertIn("STT unreachable", r.json()["detail"])

    def test_health_reads_the_engine(self):
        self.assertEqual(self.client.get("/health").json(), {"status": "ok"})
        os.environ["BREEZE_TTS_URL"] = "http://127.0.0.1:9"
        self.mod = importlib.reload(self.mod)
        from fastapi.testclient import TestClient

        self.assertEqual(TestClient(self.mod.app).get("/health").json(), {"status": "loading"})

    def test_an_unreachable_engine_is_named(self):
        with open(os.path.join(self.voices, "default.txt"), "w") as f:
            f.write("Words.\n")
        os.environ["BREEZE_TTS_URL"] = "http://127.0.0.1:9"
        self.mod = importlib.reload(self.mod)
        from fastapi.testclient import TestClient

        r = TestClient(self.mod.app).post("/v1/audio/speech", json={"input": "Hi."})
        self.assertEqual(r.status_code, 503)
        self.assertIn("unreachable", r.json()["detail"])

    def test_a_clip_replaced_after_its_sidecar_is_transcribed_again(self):
        txt = os.path.join(self.voices, "vctk_p297.txt")
        with open(txt, "w") as f:
            f.write("The old clip's words.\n")
        wav = os.path.join(self.voices, "vctk_p297.wav")
        st = os.stat(txt)
        os.utime(wav, (st.st_atime, st.st_mtime + 5))  # the clip is newer
        self.speak(voice="vctk_p297")
        self.assertEqual(self.engine.transcribed, 1)
        self.assertEqual(self.engine.registered["vctk_p297"]["ref_text"], "Please call Stella.")

    def test_an_engine_that_forgot_its_voices_is_taught_again(self):
        self.speak(voice="vctk_p297")
        self.engine.forget_once = True
        r = self.speak(voice="vctk_p297")
        self.assertEqual(r.status_code, 200, r.text)

    def test_instructions_are_forwarded_flat_and_capped(self):
        self.speak(instructions="  warmly,\nwith a smile\x07  " + "x" * 400)
        sent = self.engine.spoken[-1]["instructions"]
        self.assertTrue(sent.startswith("warmly, with a smile"), sent)
        self.assertNotIn("\x07", sent)
        self.assertLessEqual(len(sent), self.mod.INSTRUCTIONS_MAX)
        self.speak()
        self.assertNotIn("instructions", self.engine.spoken[-1], "an absent direction was sent")

    def test_chatterboxs_pair_is_refused_not_ignored(self):
        for name in ("exaggeration", "cfg_weight"):
            r = self.speak(**{name: 0.5})
            self.assertEqual(r.status_code, 400, name)
            self.assertIn("not honoured", r.json()["detail"])

    def test_speed_one_streams_the_engines_bytes_and_other_speeds_stretch(self):
        self.assertEqual(self.speak().content, PCM)
        faster = self.speak(speed=1.5).content
        self.assertLess(len(faster), len(PCM))
        self.assertEqual(self.speak(speed=3.0).status_code, 400)

    def test_wav_is_wrapped(self):
        r = self.client.post("/v1/audio/speech", json={"input": "Hi.", "response_format": "wav"})
        self.assertTrue(r.content.startswith(b"RIFF"))


if __name__ == "__main__":
    unittest.main()
