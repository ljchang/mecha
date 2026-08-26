#!/usr/bin/env python3
"""OpenAI-compatible TTS server for Chatterbox Turbo.

POST /v1/audio/speech  -> wav, or raw s16le pcm at 24 kHz for streaming
GET  /v1/voices        -> what this server can actually speak as
GET  /health

`voice` names a cloning reference in VOICES_DIR: "default" (or "")
uses the model's built-in voice; any other name resolves to
<VOICES_DIR>/<name>.wav, and an unknown name is a 400 rather than a
fallback - the assistant speaking as the wrong person is worse than
not speaking.

Generation is serialized behind a lock: one GPU, one model instance, and
concurrent generates would interleave.
"""
import io
import os
import threading
import time

import numpy as np
import soundfile as sf
from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from pydantic import BaseModel

VOICES_DIR = os.environ.get("VOICES_DIR", "/voices")

# Speed is applied here rather than on the client because the browser's
# only cheap knob is playbackRate, which resamples - it moves pitch with
# tempo and turns the assistant into a chipmunk. librosa's phase vocoder
# keeps pitch fixed. The bounds are taste, not safety: past 2x the
# vocoder smears consonants, and below 0.5x it sounds drugged.
MIN_SPEED, MAX_SPEED = 0.5, 2.0

app = FastAPI()
model = None
lock = threading.Lock()


class SpeechRequest(BaseModel):
    input: str
    model: str = "chatterbox-turbo"  # accepted, ignored: one model per server
    voice: str = "default"
    response_format: str = "wav"  # wav, or pcm (raw s16le at 24 kHz) for streaming
    # Chatterbox knobs. These are the *library's* own defaults, and the
    # distinction cost something: 0.0 looks like "no opinion" and is in
    # fact the most monotone setting on a 0-1 scale, so a wrapper picking
    # it as a neutral-looking placeholder ships a flat voice that nothing
    # errors about. Zero is not neutral. Resemble's expressive recipe is
    # a high exaggeration against a *low* cfg_weight - the two interact,
    # and cranking exaggeration alone rushes the cadence.
    exaggeration: float = 0.5
    cfg_weight: float = 0.5
    temperature: float = 0.8
    # OpenAI's own speech API spells speed this way, so a generic client
    # gets it for free. Chatterbox itself has no speed parameter - see
    # `stretch` below for what actually happens.
    speed: float = 1.0


def stretch(samples: np.ndarray, speed: float) -> np.ndarray:
    """Pitch-preserving time stretch. speed > 1 is faster (shorter)."""
    if abs(speed - 1.0) < 0.01:
        return samples
    import librosa

    return librosa.effects.time_stretch(samples, rate=speed)


@app.on_event("startup")
def load():
    global model
    from chatterbox.tts_turbo import ChatterboxTurboTTS

    t0 = time.time()
    model = ChatterboxTurboTTS.from_pretrained(device="cuda")
    # Warm pass: the first generate pays kernel compilation; pay it at
    # boot, not on the first thing the owner says.
    model.generate("Warm up.")
    print(f"chatterbox ready in {time.time() - t0:.1f}s", flush=True)


@app.get("/health")
def health():
    return {"status": "ok" if model is not None else "loading"}


@app.get("/v1/voices")
def voices():
    """What this server can speak as, right now.

    Read off the directory rather than a list in code: adding a voice is
    dropping a wav in, so a hardcoded list would be a second source of
    truth that goes stale the moment someone does the documented thing.
    A UI that offers a name this does not return would 400 on selection.
    """
    names = []
    try:
        names = sorted(
            f[:-4] for f in os.listdir(VOICES_DIR) if f.endswith(".wav")
        )
    except OSError:
        # An unreadable voices dir means only the built-in voice works -
        # which is a smaller story than failing the request, and the
        # caller can still speak.
        pass
    return {
        "default": "default",
        "voices": ["default"] + names,
        "speed": {"min": MIN_SPEED, "max": MAX_SPEED, "default": 1.0},
    }


@app.post("/v1/audio/speech")
def speech(req: SpeechRequest):
    if model is None:
        raise HTTPException(503, "model still loading")
    if req.response_format not in ("wav", "pcm"):
        raise HTTPException(400, "wav or pcm only")
    if not (MIN_SPEED <= req.speed <= MAX_SPEED):
        raise HTTPException(400, f"speed must be in [{MIN_SPEED}, {MAX_SPEED}]")
    # Clamp-and-refuse, never clamp-and-accept: the caller's UI would
    # otherwise show a value the voice is not using (worker.py set_speed).
    for name, value in (("exaggeration", req.exaggeration), ("cfg_weight", req.cfg_weight)):
        if not (0.0 <= value <= 1.0):
            raise HTTPException(400, f"{name} must be in [0.0, 1.0]")
    prompt_path = None
    if req.voice not in ("default", ""):
        prompt_path = os.path.join(VOICES_DIR, f"{req.voice}.wav")
        if not os.path.isfile(prompt_path):
            # A missing voice must fail loudly: falling back to the default
            # voice would have the assistant speak as the wrong person.
            raise HTTPException(400, f"unknown voice: {req.voice}")
    with lock:
        wav = model.generate(
            req.input,
            audio_prompt_path=prompt_path,
            exaggeration=req.exaggeration,
            cfg_weight=req.cfg_weight,
            temperature=req.temperature,
        )
    samples = wav.squeeze().cpu().numpy()
    samples = stretch(samples, req.speed)
    if req.response_format == "pcm":
        # Raw s16le at model.sr (24 kHz) - what the voice worker streams.
        pcm = (samples.clip(-1.0, 1.0) * 32767.0).astype(np.int16).tobytes()
        return Response(content=pcm, media_type="audio/pcm")
    buf = io.BytesIO()
    sf.write(buf, samples, model.sr, format="WAV")
    return Response(content=buf.getvalue(), media_type="audio/wav")
