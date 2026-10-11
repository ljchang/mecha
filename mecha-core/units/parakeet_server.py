#!/usr/bin/env python3
"""OpenAI-compatible transcription server for Parakeet TDT (sherpa-onnx).

POST /v1/audio/transcriptions (multipart, whisper-style) -> {"text": ...}
GET  /health

The STT seat's structural occupant: a transducer CANNOT answer, obey, or
improvise - it can only emit the tokens it heard. The chat-model
transcriber it replaces obeyed spoken instructions ("say the word
banana" -> "banana") and answered question-shaped speech instead of
transcribing it; no prompt fixes what a model *is*, so the fix is a
model that has no prompt (docs/VOICE-RESEARCH.md S7, 2026-08-24).

CPU on purpose: int8 Parakeet on the Grace cores is faster than
real-time, cannot be starved by llama-server saturating the GPU, and
the GB10's aarch64 GPU onnxruntime builds cannot target sm_121 anyway.
"""
import io
import os
import time
import wave

import numpy as np
import sherpa_onnx
from fastapi import FastAPI, File, Form, UploadFile, HTTPException

MODEL_DIR = os.environ.get(
    "PARAKEET_DIR",
    os.path.expanduser("~/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"),
)

app = FastAPI()
recognizer = None


@app.on_event("startup")
def load():
    global recognizer
    t0 = time.time()
    recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
        encoder=f"{MODEL_DIR}/encoder.int8.onnx",
        decoder=f"{MODEL_DIR}/decoder.int8.onnx",
        joiner=f"{MODEL_DIR}/joiner.int8.onnx",
        tokens=f"{MODEL_DIR}/tokens.txt",
        model_type="nemo_transducer",
        num_threads=8,
    )
    print(f"parakeet ready in {time.time() - t0:.1f}s", flush=True)


@app.get("/health")
def health():
    return {"status": "ok" if recognizer is not None else "loading"}


@app.post("/v1/audio/transcriptions")
async def transcribe(file: UploadFile = File(...), model: str = Form("parakeet"),
                     language: str = Form(None), prompt: str = Form(None),
                     temperature: float = Form(0.0)):
    raw = await file.read()
    try:
        with wave.open(io.BytesIO(raw)) as w:
            rate = w.getframerate()
            frames = w.readframes(w.getnframes())
    except (wave.Error, EOFError) as e:
        raise HTTPException(status_code=400, detail=f"not a WAV clip: {e}")
    samples = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
    if samples.size == 0:
        # A clip with a header and no samples. The dictate button used to
        # send one whenever the page's audio graph never ran (2 of 8 clips
        # on 2026-09-12), and the encoder crashed on the empty tensor
        # ("Invalid input shape: {0,128}") as a 500 — which the page showed
        # as a gateway error rather than as "nothing was recorded". A
        # request with nothing in it is the caller's mistake, said so.
        raise HTTPException(status_code=400, detail="empty audio: the clip has no samples")
    stream = recognizer.create_stream()
    stream.accept_waveform(rate, samples)
    recognizer.decode_stream(stream)
    return {"text": stream.result.text.strip()}
