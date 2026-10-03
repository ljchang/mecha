#!/usr/bin/env python3
"""OpenAI-compatible TTS server for Chatterbox (Turbo by default).

POST /v1/audio/speech  -> wav, or raw s16le pcm at 24 kHz for streaming
GET  /v1/voices        -> what this server can actually speak as, and which
                          per-request controls the loaded model honours
GET  /health

`CHATTERBOX_MODEL` picks the model: `turbo` (the default, and the live
voice) or `original`, the 500M model Turbo was distilled from - slower,
and the only one of the two that honours `exaggeration` and `cfg_weight`.

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

from audio_stretch import stretch

VOICES_DIR = os.environ.get("VOICES_DIR", "/voices")

# Speed is applied here rather than on the client because the browser's
# only cheap knob is playbackRate, which resamples - it moves pitch with
# tempo and turns the assistant into a chipmunk. `stretch` below keeps
# pitch fixed. The bounds are taste, not safety: past 2x any time-domain
# method smears consonants, and below 0.5x it sounds drugged.
MIN_SPEED, MAX_SPEED = 0.5, 2.0

MODEL_KIND = os.environ.get("CHATTERBOX_MODEL", "turbo")

# What each model actually does with a request's controls, read from the
# installed library (chatterbox-tts 0.1.7) rather than from its signature -
# and the difference is the point. Turbo's `generate` accepts
# `exaggeration` and `cfg_weight` and then drops both: its config sets
# `emotion_adv = False`, so the emotion input is never built, and CFG is
# never passed to `inference_turbo`. It logs a warning saying so, once per
# request, inside this container and nowhere a caller looks - which is how
# the worker sent 0.8 / 0.3 for five weeks to a model that ignored them.
# So a control the loaded model would ignore is refused, and `/v1/voices`
# says which ones it honours, so a client can send only those.
CONTROLS = {
    "turbo": ("temperature",),
    "original": ("temperature", "exaggeration", "cfg_weight"),
}
# Checked here, at import, rather than at load: every lookup of
# CONTROLS[MODEL_KIND] is then safe for anything that imports this module,
# startup or not, and an unknown kind still fails the start - never a quiet
# fallback to Turbo advertising controls its model drops.
if MODEL_KIND not in CONTROLS:
    raise RuntimeError(f"CHATTERBOX_MODEL must be turbo or original, not {MODEL_KIND!r}")

# The original model's own documented ranges: exaggeration past 1.0 is
# Resemble's expressive end, which the worker and a persona may ask for.
BOUNDS = {"exaggeration": (0.0, 2.0), "cfg_weight": (0.0, 1.0)}

app = FastAPI()
model = None
lock = threading.Lock()


class SpeechRequest(BaseModel):
    input: str
    model: str = "chatterbox-turbo"  # accepted, ignored: one model per server
    voice: str = "default"
    response_format: str = "wav"  # wav, or pcm (raw s16le at 24 kHz) for streaming
    # The original model's expressiveness pair: unset leaves the library's
    # own default (0.5 / 0.5 - zero is not neutral, it is the monotone
    # end). Resemble's expressive recipe is a high exaggeration against a
    # *low* cfg_weight; the two interact. Set on a model that ignores them
    # (`CONTROLS`), a request is refused rather than spoken as if they
    # had landed.
    exaggeration: float | None = None
    cfg_weight: float | None = None
    temperature: float = 0.8
    # OpenAI's own speech API spells speed this way, so a generic client
    # gets it for free. Chatterbox itself has no speed parameter - see
    # `stretch` below for what actually happens.
    speed: float = 1.0


@app.on_event("startup")
def load():
    global model
    t0 = time.time()
    if MODEL_KIND == "turbo":
        from chatterbox.tts_turbo import ChatterboxTurboTTS

        model = ChatterboxTurboTTS.from_pretrained(device="cuda")
    else:  # "original", the only other kind CONTROLS admits
        from chatterbox.tts import ChatterboxTTS

        model = ChatterboxTTS.from_pretrained(device="cuda")
    # Warm pass: the first generate pays kernel compilation; pay it at
    # boot, not on the first thing the owner says.
    model.generate("Warm up.")
    print(f"chatterbox-{MODEL_KIND} ready in {time.time() - t0:.1f}s", flush=True)


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
        "model": f"chatterbox-{MODEL_KIND}",
        "controls": list(CONTROLS[MODEL_KIND]),
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
    # And refuse-not-ignore for a control this model drops, for the same
    # reason one level down.
    controls = {}
    for name, (lo, hi) in BOUNDS.items():
        value = getattr(req, name)
        if value is None:
            continue
        if name not in CONTROLS[MODEL_KIND]:
            raise HTTPException(
                400, f"{name} is not honoured by chatterbox-{MODEL_KIND}; refused, not ignored")
        if not (lo <= value <= hi):
            raise HTTPException(400, f"{name} must be in [{lo}, {hi}]")
        controls[name] = value
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
            temperature=req.temperature,
            **controls,
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
