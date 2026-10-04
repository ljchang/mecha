#!/usr/bin/env python3
"""OpenAI-compatible TTS adapter for Breeze TTS 2 on qwentts.cpp.

The voice worker talks to a TTS through one small surface - the one
chatterbox_server.py serves - and this presents the same surface in front of
the Breeze port's `tts-server` (~/models/breeze-qwentts, a qwentts.cpp fork):

POST /v1/audio/speech  -> wav, or raw s16le pcm at 24 kHz, streamed at speed 1
GET  /v1/voices        -> what this server can speak as, and which per-request
                          controls the model honours (chatterbox_server's shape)
GET  /health

Three things the engine does not do, done here:

- **Voices need a transcript.** Breeze clones from a reference clip *and its
  exact text*. A voice is `<VOICES_DIR>/<name>.wav`; its text is the sidecar
  `<name>.txt`, written once by transcribing the clip with Parakeet when it is
  first spoken (owner ruling D3, 2026-10-03) and editable after. The port study
  measured Breeze tolerating small transcript errors.
- **Voices are registered with the engine** (`POST /v1/audio/voices`) on first
  use and again when the clip changes, and once more if the engine has
  restarted and forgotten them - its registry lives in memory.
- **Speed.** The engine parses `speed` and ignores it. At 1.0 the PCM streams
  straight through; at any other speed the sentence is buffered and stretched
  with the same pitch-preserving WSOLA Chatterbox uses (ruling D4), which costs
  that sentence its streaming.

`voice: "default"` is a clip like any other, `default.wav` (ruling D2, as the
owner simplified it on 2026-10-03): Breeze has no built-in voice, so the
default is a file, named for what it is. One that does not exist is a 503 that
says so, never a fallback to another voice.

`instructions` is Breeze's voice direction - how a sentence is delivered - and
is written by the model on a spoken turn (ruling D1, 2026-10-03). It is capped
in length and stripped of control characters here; it never becomes text that
is spoken.

**Text with no letter or digit in it is a pause, never sent to the engine.**
Given nothing to say, Breeze invents something: measured on the real engine
(2026-10-03), "." came back as "Um", "Yeah", or the voice clip's own sentence
read whole; "..." as fifteen seconds of babble or a phrase in Ukrainian; "?!"
as twenty seconds of babble. Any letter at all ("Mm.", "Hmm...") was spoken as
written. The worker's sentence splitter hands over a lone "." when a reply
trails off in "...", so this is an owner hearing a stranger mid-story, with
nothing in the transcript to say why.
"""
import asyncio
import base64
import io
import os
import wave

import httpx
import numpy as np
from fastapi import FastAPI, HTTPException
from fastapi.responses import Response, StreamingResponse
from pydantic import BaseModel

from audio_stretch import stretch
from fragments import speakable_core

BREEZE_URL = os.environ.get("BREEZE_TTS_URL", "http://127.0.0.1:8886").rstrip("/")
VOICES_DIR = os.path.expanduser(os.environ.get("VOICES_DIR", "~/models/voices"))
STT_URL = os.environ.get("MECHA_VOICE_STT", "http://127.0.0.1:8992/v1").rstrip("/")
DEFAULT_VOICE = "default"
MODEL_NAME = os.environ.get("BREEZE_MODEL_NAME", "breeze-tts2-q6_k")
RATE = 24000
MIN_SPEED, MAX_SPEED = 0.5, 2.0
# Long enough for a sentence of direction ("warmly, a little amused, slowing
# down at the end"), short enough that it cannot carry a paragraph.
INSTRUCTIONS_MAX = 300
# What a sentence with nothing speakable in it plays as: a beat, the length
# of the pause punctuation marks.
PAUSE_SECONDS = 0.3
# What Breeze honours per request. Chatterbox's pair is refused, not ignored,
# on the rule chatterbox_server.py states: a control a model drops is refused
# rather than spoken as if it had landed.
CONTROLS = ("temperature", "instructions")
REFUSED = ("exaggeration", "cfg_weight")
# Refused outside, never clamped (chatterbox_server's rule). Zero is greedy
# decoding in the engine, so it is in range.
BOUNDS = {"temperature": (0.0, 2.0)}

app = FastAPI()
# voice name -> the (wav, txt) mtimes it was registered at: editing either
# re-registers on the next request, as ruling D3's "editable" promises.
_registered: dict[str, tuple[float, float]] = {}
_lock = asyncio.Lock()


class SpeechRequest(BaseModel):
    input: str
    model: str = "breeze"  # accepted, ignored: one model per server
    voice: str = "default"
    response_format: str = "wav"
    speed: float = 1.0
    temperature: float | None = None
    instructions: str | None = None
    exaggeration: float | None = None
    cfg_weight: float | None = None


def voice_names() -> list[str]:
    """Breeze has no built-in voice, so an unreadable directory is not an
    empty library: it is said, as a 503, rather than read as "no default voice"."""
    try:
        return sorted(f[:-4] for f in os.listdir(VOICES_DIR) if f.endswith(".wav"))
    except OSError as e:
        raise HTTPException(503, f"cannot read the voices directory {VOICES_DIR}: {e}")


def _stamp(wav_path: str) -> tuple[float, float]:
    txt = wav_path[:-4] + ".txt"
    return (os.path.getmtime(wav_path), os.path.getmtime(txt) if os.path.exists(txt) else 0.0)


def clean_instructions(text: str | None) -> str:
    """The voice direction as sent: control characters out, one line,
    capped. Empty means none."""
    if not text:
        return ""
    flat = "".join(" " if c in "\r\n\t" else c for c in text if c.isprintable() or c in "\r\n\t")
    return " ".join(flat.split())[:INSTRUCTIONS_MAX]


def speakable(text: str) -> bool:
    """Whether there is a word to say: a letter or digit in any script,
    outside a vocal-event tag. Without one the engine hallucinates (module
    docstring); a tag alone, "(laugh)", came back once in three as invented
    words (2026-10-04), so it counts as nothing to say. The worker joins a tag
    to the sentence after it (`fragments.py`); this is the backstop for every
    other route."""
    return any(c.isalnum() for c in speakable_core(text))


async def transcript_for(client: httpx.AsyncClient, name: str, wav_path: str) -> str:
    """The clip's text: the sidecar if there is one and it is not older than
    the clip, else Parakeet's transcript, written as the sidecar so it is done
    once and can be edited. A clip replaced after its sidecar was written (a
    re-cut, a voice deleted and re-added under the same name) would otherwise
    be registered with the old clip's words."""
    txt = wav_path[:-4] + ".txt"
    if os.path.exists(txt) and os.path.getmtime(txt) >= os.path.getmtime(wav_path):
        with open(txt, encoding="utf-8") as f:
            text = f.read().strip()
        if text:
            return text
    with open(wav_path, "rb") as f:
        audio = f.read()
    try:
        r = await client.post(
            f"{STT_URL}/audio/transcriptions",
            files={"file": (os.path.basename(wav_path), audio, "audio/wav")},
            data={"model": "parakeet"},
            timeout=120,
        )
    except httpx.HTTPError as e:
        raise HTTPException(503, f"could not transcribe voice {name}: STT unreachable ({type(e).__name__})")
    if r.status_code != 200:
        raise HTTPException(503, f"could not transcribe voice {name}: STT answered {r.status_code}")
    text = (r.json().get("text") or "").strip()
    if not text:
        raise HTTPException(503, f"could not transcribe voice {name}: empty transcript")
    tmp = txt + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text + "\n")
    os.replace(tmp, txt)
    return text


async def ensure_registered(client: httpx.AsyncClient, name: str, force: bool = False) -> None:
    wav_path = os.path.join(VOICES_DIR, f"{name}.wav")
    # The warm path takes no lock: a cold voice's transcription (~5 s) must
    # not hold up a sentence in a voice that needs nothing done.
    if not force and _registered.get(name) == _stamp(wav_path):
        return
    async with _lock:
        if not force and _registered.get(name) == _stamp(wav_path):
            return
        ref_text = await transcript_for(client, name, wav_path)
        # After the transcript, so a sidecar written just now is in the stamp.
        stamp = _stamp(wav_path)
        with open(wav_path, "rb") as f:
            wav_b64 = base64.b64encode(f.read()).decode()
        r = await client.post(
            f"{BREEZE_URL}/v1/audio/voices",
            json={"name": name, "wav_b64": wav_b64, "ref_text": ref_text},
            timeout=120,
        )
        if r.status_code >= 400:
            raise HTTPException(503, f"the Breeze engine refused voice {name}: {r.text[:200]}")
        _registered[name] = stamp


@app.get("/health")
async def health():
    try:
        async with httpx.AsyncClient() as client:
            r = await client.get(f"{BREEZE_URL}/health", timeout=3)
        return {"status": "ok" if r.status_code == 200 else "loading"}
    except httpx.HTTPError:
        return {"status": "loading"}


@app.get("/v1/voices")
def voices():
    """Read off the directory, as chatterbox_server does: adding a voice is
    dropping a wav in. `default` (default.wav) leads the list when it exists."""
    on_disk = voice_names()
    names = [n for n in on_disk if n != DEFAULT_VOICE]
    # `default` is a file, not a built-in: listed only when it can be spoken,
    # so the picker never offers a voice that 503s every sentence.
    first = [DEFAULT_VOICE] if DEFAULT_VOICE in on_disk else []
    return {
        "default": "default",
        "voices": first + names,
        "speed": {"min": MIN_SPEED, "max": MAX_SPEED, "default": 1.0},
        "model": MODEL_NAME,
        "controls": list(CONTROLS),
        # The first audio of a sentence arrives while the rest is still being
        # synthesised, so a long sentence costs no silence up front. The
        # worker passes this on, and the spoken-turn prompt drops its
        # brevity rules, which existed for whole-sentence engines (owner
        # ruling, 2026-10-03). Absent, as on Chatterbox, means it does not.
        "streams": True,
    }


def wav_bytes(pcm: bytes) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm)
    return buf.getvalue()


@app.post("/v1/audio/speech")
async def speech(req: SpeechRequest):
    if req.response_format not in ("wav", "pcm"):
        raise HTTPException(400, "wav or pcm only")
    if not (MIN_SPEED <= req.speed <= MAX_SPEED):
        raise HTTPException(400, f"speed must be in [{MIN_SPEED}, {MAX_SPEED}]")
    for name in REFUSED:
        if getattr(req, name) is not None:
            raise HTTPException(400, f"{name} is not honoured by {MODEL_NAME}; refused, not ignored")
    for name, (lo, hi) in BOUNDS.items():
        value = getattr(req, name)
        if value is not None and not (lo <= value <= hi):
            raise HTTPException(400, f"{name} must be in [{lo}, {hi}]")
    voice = req.voice or DEFAULT_VOICE
    if voice not in voice_names():
        if req.voice in ("default", ""):
            raise HTTPException(503, f"the default voice is missing: no default.wav in {VOICES_DIR}")
        raise HTTPException(400, f"unknown voice: {req.voice}")
    if not speakable(req.input):
        # Speed applies here too: a beat at 1.5x is shorter, as speech is.
        pause = b"\x00\x00" * int(RATE * PAUSE_SECONDS / req.speed)
        if req.response_format == "pcm":
            return Response(content=pause, media_type="audio/pcm")
        return Response(content=wav_bytes(pause), media_type="audio/wav")

    body = {"input": req.input, "voice": voice, "response_format": "pcm"}
    if req.temperature is not None:
        body["temperature"] = req.temperature
    direction = clean_instructions(req.instructions)
    if direction:
        body["instructions"] = direction

    client = httpx.AsyncClient(timeout=httpx.Timeout(120, connect=5))
    try:
        await ensure_registered(client, voice)
        upstream = await _open(client, body)
        if upstream.status_code != 200:
            detail = (await upstream.aread()).decode(errors="replace")[:200]
            await upstream.aclose()
            # The engine keeps its registry in memory, so a restart forgets
            # every voice. Ask its registry rather than parse the error's
            # wording: absent means teach it again and ask once more; present
            # means the refusal was about something else, said as it was.
            if await _engine_knows(client, voice) is True:
                raise HTTPException(502, f"the Breeze engine answered {upstream.status_code}: {detail}")
            await ensure_registered(client, voice, force=True)
            upstream = await _open(client, body)
            if upstream.status_code != 200:
                detail = (await upstream.aread()).decode(errors="replace")[:200]
                await upstream.aclose()
                raise HTTPException(502, f"the Breeze engine answered {upstream.status_code}: {detail}")
    except httpx.HTTPError as e:
        # Down or still loading: the engine answers its own `/health` only
        # once it is listening (measured on the real fork, 2026-10-03) and
        # gives systemd no readiness notice, so the adapter can be up first.
        # Said by name, as /health says it, not a bare 500 with the reason
        # only in the journal.
        await client.aclose()
        raise HTTPException(503, f"the Breeze engine is unreachable ({type(e).__name__}); loading or down")
    except BaseException:
        await client.aclose()
        raise

    if req.response_format == "pcm" and abs(req.speed - 1.0) < 0.01:
        # The first chunk is read before the headers are committed, so a 200
        # with no audio is still a 502 on the path calls use, not an empty
        # stream that plays as silence.
        chunks = upstream.aiter_bytes()
        first = b""
        try:
            async for chunk in chunks:
                if chunk:
                    first = chunk
                    break
        except BaseException:
            await upstream.aclose()
            await client.aclose()
            raise
        if not first:
            await upstream.aclose()
            await client.aclose()
            raise HTTPException(502, "the Breeze engine answered 200 with no audio")

        async def relay():
            try:
                yield first
                async for chunk in chunks:
                    if chunk:
                        yield chunk
            finally:
                await upstream.aclose()
                await client.aclose()

        return StreamingResponse(relay(), media_type="audio/pcm")

    try:
        pcm = await upstream.aread()
    finally:
        await upstream.aclose()
        await client.aclose()
    if not pcm:
        # The envelope before the content: llama.cpp-family servers answer 200
        # with nothing in it, and a silent RIFF would pass that on as speech.
        raise HTTPException(502, "the Breeze engine answered 200 with no audio")
    if abs(req.speed - 1.0) >= 0.01:
        samples = np.frombuffer(pcm[: len(pcm) // 2 * 2], dtype=np.int16).astype(np.float32) / 32768.0
        samples = stretch(samples, req.speed, sr=RATE)
        pcm = (np.clip(samples, -1.0, 1.0) * 32767.0).astype(np.int16).tobytes()
    if req.response_format == "pcm":
        return Response(content=pcm, media_type="audio/pcm")
    return Response(content=wav_bytes(pcm), media_type="audio/wav")


async def _engine_knows(client: httpx.AsyncClient, name: str) -> bool | None:
    """Whether the engine's registry holds `name`: None when the registry
    cannot be read, which the caller treats as "teach it again" - one
    re-upload and one retry, bounded, rather than a voice that 502s until the
    adapter restarts. The shape is the fork's `tts_handle_voices`, measured
    against the real engine on 2026-10-03:
    `{"voices": [{"name": "vctk_p297", "kind": "registered"}]}`."""
    try:
        r = await client.get(f"{BREEZE_URL}/v1/audio/voices", timeout=5)
        voices = r.json().get("voices") if r.status_code == 200 else None
    except (httpx.HTTPError, ValueError, AttributeError):
        voices = None
    if not isinstance(voices, list):
        return None
    return any(v.get("name") == name for v in voices if isinstance(v, dict))


async def _open(client: httpx.AsyncClient, body: dict) -> httpx.Response:
    request = client.build_request("POST", f"{BREEZE_URL}/v1/audio/speech", json=body)
    return await client.send(request, stream=True)


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host=os.environ.get("BREEZE_HOST", "127.0.0.1"),
                port=int(os.environ.get("BREEZE_PORT", "8887")))
