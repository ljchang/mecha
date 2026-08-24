"""OpenAI-compatible TTS server for Chatterbox Turbo.

Runs inside the `mecha/chatterbox:serve` image (see docs/VOICE-RESEARCH.md
S7 for why that image exists and what it cost). One route that matters:
POST /v1/audio/speech, the same surface Kokoro-FastAPI serves on :8880,
so the voice worker consumes both voices through identical config.

The `voice` parameter maps to cloning references: "default" (or absent)
uses the model's built-in voice; any other name resolves to
/voices/<name>.wav, a directory mounted from the host. Dropping a 5-second
reference wav there IS the act of adding a voice - no restart needed.

Generation is serialized behind a lock: one GPU, one model instance, and
interleaved generate() calls are not known to be safe. A voice turn sends
one utterance at a time, so the lock never queues in practice.
"""
import io
import os
import threading
import time

import soundfile as sf
from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from pydantic import BaseModel

VOICES_DIR = os.environ.get("VOICES_DIR", "/voices")

app = FastAPI()
model = None
lock = threading.Lock()


class SpeechRequest(BaseModel):
    input: str
    model: str = "chatterbox-turbo"  # accepted, ignored: one model per server
    voice: str = "default"
    response_format: str = "wav"  # wav, or pcm (raw s16le at 24 kHz) for streaming
    # Chatterbox knobs, passed through when a client wants them.
    exaggeration: float = 0.0
    temperature: float = 0.8


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


@app.post("/v1/audio/speech")
def speech(req: SpeechRequest):
    if model is None:
        raise HTTPException(503, "model still loading")
    if req.response_format not in ("wav", "pcm"):
        raise HTTPException(400, "wav or pcm only")
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
            temperature=req.temperature,
        )
    samples = wav.squeeze().cpu().numpy()
    if req.response_format == "pcm":
        # Raw s16le at model.sr (24 kHz) - what the voice worker streams.
        import numpy as np

        pcm = (samples.clip(-1.0, 1.0) * 32767.0).astype(np.int16).tobytes()
        return Response(content=pcm, media_type="audio/pcm")
    buf = io.BytesIO()
    sf.write(buf, samples, model.sr, format="WAV")
    return Response(content=buf.getvalue(), media_type="audio/wav")
