"""The Breeze adapter presents chatterbox_server's surface over qwentts.cpp's
tts-server: voices with Parakeet-written transcripts, registration that
survives the engine forgetting, honoured controls listed and the rest refused,
the house voice behind `default`, and speed as a stretch. Against stand-ins for
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
                self._send(200, b'{"status":"ok"}')

            def do_POST(self):
                body = self._body()
                if self.path == "/v1/audio/transcriptions":
                    engine.transcribed += 1
                    return self._send(200, json.dumps({"text": "Please call Maya."}).encode())
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
                        return self._send(400, b'{"error":"unknown voice"}')
                    engine.spoken.append(req)
                    return self._send(200, PCM, "audio/pcm")
                self._send(404, b"{}")

        self.httpd = HTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"


class Adapter(unittest.TestCase):
    def setUp(self):
        self.engine = Engine()
        self.addCleanup(self.engine.httpd.shutdown)
        self.voices = tempfile.mkdtemp()
        for name in ("house", "vctk_p297"):
            wav_file(os.path.join(self.voices, f"{name}.wav"))
        os.environ.update({
            "BREEZE_TTS_URL": self.engine.url,
            "MECHA_VOICE_STT": self.engine.url + "/v1",
            "VOICES_DIR": self.voices,
            "BREEZE_HOUSE_VOICE": "house",
        })
        import breeze_server

        self.mod = importlib.reload(breeze_server)
        from fastapi.testclient import TestClient

        self.client = TestClient(self.mod.app)

    def speak(self, **extra):
        return self.client.post("/v1/audio/speech", json={"input": "Hello.", "response_format": "pcm", **extra})

    def test_voices_list_default_and_the_library_and_the_honoured_controls(self):
        v = self.client.get("/v1/voices").json()
        self.assertEqual(v["voices"], ["default", "vctk_p297"])  # the house voice is `default`
        self.assertEqual(v["controls"], ["temperature", "instructions"])

    def test_default_speaks_as_the_house_voice(self):
        r = self.speak()
        self.assertEqual(r.status_code, 200, r.text)
        self.assertEqual(self.engine.spoken[-1]["voice"], "house")

    def test_a_missing_house_voice_is_said_not_substituted(self):
        os.remove(os.path.join(self.voices, "house.wav"))
        r = self.speak()
        self.assertEqual(r.status_code, 503)
        self.assertIn("house voice", r.json()["detail"])
        self.assertEqual(self.engine.spoken, [])

    def test_an_unknown_voice_is_refused(self):
        self.assertEqual(self.speak(voice="nobody").status_code, 400)

    def test_a_voice_is_transcribed_once_and_registered_with_its_text(self):
        self.speak(voice="vctk_p297")
        self.speak(voice="vctk_p297")
        self.assertEqual(self.engine.transcribed, 1, "transcribed again instead of reading the sidecar")
        txt = os.path.join(self.voices, "vctk_p297.txt")
        self.assertEqual(open(txt).read().strip(), "Please call Maya.")
        reg = self.engine.registered["vctk_p297"]
        self.assertEqual(reg["ref_text"], "Please call Maya.")
        self.assertTrue(base64.b64decode(reg["wav_b64"]).startswith(b"RIFF"))

    def test_an_edited_sidecar_is_what_is_registered(self):
        with open(os.path.join(self.voices, "vctk_p297.txt"), "w") as f:
            f.write("Corrected words.\n")
        self.speak(voice="vctk_p297")
        self.assertEqual(self.engine.transcribed, 0)
        self.assertEqual(self.engine.registered["vctk_p297"]["ref_text"], "Corrected words.")

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
