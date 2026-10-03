# Breeze TTS 2 as the voice (design, 2026-10-03)

Replace Chatterbox Turbo with **Breeze TTS 2**, running on a fork of qwentts.cpp,
as the speech engine for calls, Listen and the voice library. This was the
owner's pick by ear across four listening rounds (2026-10-02/03): the most
expressive of the models tried. This document holds the decisions; the
measurements are summarised in §1 and the rulings in §6.

## 1. What was measured, and why this engine

On the GB10, RTF is synthesis time divided by audio time; a live call needs it
well under 1 *while the chat model is generating*. Error is the teacher-forced
KL of the depth decoder's logits against bf16: lower is closer to full precision.

| Breeze configuration | RTF idle / under steady chat load | first audio | error | GPU memory |
|---|---|---|---|---|
| **qwentts.cpp fork, Q6_K (chosen)** | **0.44 / 0.76** | 0.07 s | 0.0042 | ~4.2 GB |
| qwentts.cpp fork, Q5_K_M (fallback) | 0.40 / 0.70 | 0.07 s | 0.011 | ~4.0 GB |
| PyTorch bf16, Breeze's own fast mode | 1.10 / 1.94 | 0.15 s | 0 | 16.4 GiB |
| PyTorch int4 HQQ-g32 | 0.44 / 0.67 | 0.14 s | 0.045 | ~11 GiB |
| audio.cpp Q5_K | 0.60 / 1.95 | 0.93 s | 0.011 | — |
| Chatterbox Turbo (today, for reference) | 0.26 / 0.45 | 1.34 s (whole sentence) | — | 5.5 GB + 2.6 GB |

**In a call,** simulated with the router writing the reply and sentences sent as they complete:
- Q6_K needed **0.12–0.29 s** of pre-buffer for zero underruns, even on 100-second replies. The buffer does not grow with reply length.
- PyTorch bf16 needed 18.6 s.

**Why this engine holds up under load where audio.cpp does not:**
- The fork runs the 15 depth passes *and their sampling* as one CUDA graph per frame, with 2 host syncs.
- audio.cpp samples on the CPU, about 45 round trips per frame, each queued behind llama-server.

**Faithfulness to the original:**
- token and prompt ids are identical to the original;
- prefill logits cosine 0.99999, same argmax;
- the codec is byte-identical to Qwen3-TTS's 12 Hz codec.

Also tried and dropped: TensorRT-LLM (only FP8 and NVFP4 are fused on sm_121, and both are slower than torch int8 at batch 1) and ExLlamaV3 (best quality per bit, but 6–10 µs fixed cost per kernel).

**Not yet measured:**
- a call while ComfyUI renders a picture (§5);
- a call during the nightly jobs.

## 2. The pieces

```
voice worker ──/v1/audio/speech, /v1/voices──▶ breeze_server.py (:8887) ──▶ tts-server (:8886)
  (MECHA_VOICE_TTS)                               adapter, this repo          ~/models/breeze-qwentts
                                                  │
                                                  └─ Parakeet (:8992): a voice clip's transcript, once
```

- **The engine.** `tts-server` from the fork, in `~/models/breeze-qwentts`:
  - `gguf/`: Q6_K, Q5_K_M and the codec;
  - `src/`: the fork, about 1,100 new lines: a T5Gemma2 text encoder, a Gemma tokenizer and the Breeze prompt builder;
  - `build/`: self-contained, carrying its own ggml shared libraries;
  - `README.md`.

  The engine lives outside the repo because the repo holds no third-party engines or weights, as with llama.cpp. Unit: `scripts/voice/mecha-breeze-tts.service`; the quant is set by `BREEZE_GGUF`.
- **The adapter.** `scripts/voice/breeze_server.py` presents exactly the surface `chatterbox_server.py` does, so the worker changes only its URL:
  - `/v1/voices` with `controls`: `temperature` and `instructions`;
  - `/v1/audio/speech` streaming PCM;
  - `/health`.

  What it adds to the engine:
  - a transcript sidecar per voice;
  - registration with the engine, renewed if the engine has forgotten it;
  - the house voice behind `default`;
  - speed by stretch.

  The engine's registry lives in memory, so a restarted engine has forgotten every voice. Measured against the real engine (2026-10-03): `GET /v1/audio/voices` answers `{"voices": [{"name": …, "kind": "registered"}]}`, and speaking an unregistered voice is HTTP 400 `unknown voice '<name>'`. The adapter asks the registry before retrying, and re-registers when the registry cannot be read.

  Unit: `scripts/voice/mecha-breeze-adapter.service`, which runs from the shared checkout as the worker does. Tests: `scripts/voice/test_breeze_server.py`, with stand-ins for the engine and the STT.
- **The switch.** A drop-in on `mecha-voice-worker.service` sets `MECHA_VOICE_TTS=http://127.0.0.1:8887/v1`. Chatterbox keeps running, so rolling back means removing the drop-in and restarting the worker.

## 3. Delivery: the director, and the prompt on a streaming engine

Breeze takes, beside the text of a sentence, a free-text **voice direction**: `instructions`, rendered as `<ins_bos>…<ins_eos>` in its prompt.

**Ruling D1, as refined (2026-10-03):** a separate model call, the **director**, writes a detailed direction for every spoken sentence. The chat model writes only the words. The first plan, where the chat model wrote inline `[[…]]` directions that the worker split out, was set aside after a pros-and-cons comparison: the director gives every sentence its own direction at a consistent level of detail, and nothing reaches the transcript that the page and derived readers would then have to hide. The code is `mecha_core::voice_direction` (prompt, one-shot call, record) and `mecha-cli`'s `voice::direct` (`POST /v1/mecha-direct`); `docs/ARCHITECTURE.md` § The voice director holds the invariants.

1. **When it runs.** The worker asks the facade once per sentence, only when the TTS lists `instructions` in its `controls`. On Chatterbox no call is made.
2. **On which model.** Whichever the session already has loaded (owner: "so we don't incur a model switching cost or run out of RAM"). The router is held without waiting and followed per call, never named from a cached binding; a model switch in flight skips the direction rather than waiting. Thinking is off for the call (`CompletionRequest::think`), since a reasoning pass would spend seconds per sentence.
3. **Latency.** Measured on the loaded `qwen3.6-35b-a3b-uncensored` with the real prompt (the director's frame, a scene and the growing list of sentences already directed), thinking off, 3 rounds × 6 sentences: a 12–25-word direction took a median 1.61 s, p90 1.77 s, max 1.83 s; a 6–12-word one a median 1.11 s, max 1.32 s. The detailed length stays, as the owner wants. So:
   - sentences after the first are directed while the one before plays, under a 2.5 s deadline. The slack is that sentence's remaining playback, about (1 − RTF) × its length, so a short sentence before a long direction leaves a gap; the follow-up is prefetch, where the facade directs each sentence as its text streams, before the worker asks;
   - the first sentence's direction (the *opening*) starts from the owner's words as the turn begins, overlapping the chat model's own time to its first sentence, and the first sentence waits at most 2.5 s for it;
   - directions are asked for at 12–25 words;
   - any failure, refusal or timeout speaks the sentence undirected.
4. **Harness speech is not directed.** Offers, draft read-backs, failure lines and the persona safe message are skipped with reason `harness`.
5. **Every direction is recorded** in the conversation's transcript as a `spoken_direction` record: the sentence, the direction, the model, the latency, the outcome and a prompt digest, joined by turn and sentence rather than by file position. In an incognito chat the director runs and nothing is kept, in the file or in any journal (owner ruling, 2026-10-03).

**Mitigations:** the adapter caps a direction at 300 characters and flattens it to one printable line, and a direction can change only how a sentence sounds, never what is said or sent.

**The prompt on a streaming engine (owner ruling, 2026-10-03: "breeze streams… now we don't need things short").** The spoken-turn rules about length ("short sentences", "the first one short", "keep replies brief") were a latency control for an engine that speaks a sentence only once all of it is synthesised. The adapter lists `"streams": true`, the worker passes `X-Voice-TTS-Streams: 1`, and the facade opens the turn with `VOICE_BLOCK_STREAMING` and the persona call note without its length rule. Every other voice rule is kept. Without the header, the prompt is byte-for-byte what it was.

## 4. Voices

- **Transcripts (ruling D3).** Each `<name>.wav` gets `<name>.txt`, written by Parakeet the first time the voice is spoken (about 5 s once) and editable afterwards. The port study measured Breeze tolerating small transcript errors.
- **The house voice (ruling D2).** `default` speaks as `BREEZE_HOUSE_VOICE` (`house.wav` plus its `.txt`). Breeze has no built-in voice. The candidates are expressive clips rendered by Breeze from a library voice with a direction; the owner picks one.
- **VCTK clips say their sentences twice** (the dataset rows are duplicated), and Breeze copies that delivery. Re-cut them with the duplicates removed before they serve Breeze; this belongs in `add-vctk-voices.py`.
- **Speed (ruling D4).** At 1.0 the PCM streams through. At any other speed the adapter buffers the sentence and applies the WSOLA stretch shared with Chatterbox (`scripts/voice/audio_stretch.py`), so that sentence waits for its synthesis.

## 5. Before the switch (gates)

1. The house voice exists and the owner has chosen it.
2. A call measured while ComfyUI renders a picture. Q6_K runs at 0.76 under steady chat load and image generation is a heavier co-tenant. If it is too tight: a larger pre-buffer when the GPU is busy, Q5_K_M, or the fork's fused depth layer.
3. The VCTK clips re-cut.
4. **The engine starts on a busy box.** On 2026-10-03 at 02:43Z and 02:45Z `tts-server` failed at CUDA init with `NV_ERR_NO_MEMORY` from `kgrctxAllocMainCtxBuffer` (kernel log), while `MemAvailable` read 42 GB and `MemFree` 20 GB: the driver could not allocate a context although the page cache was reclaimable. A unit that restarts into that loops. Find what the driver needs free, and whether starting at boot (before the cache fills) or a start-time retry is the answer.
5. The `tts` slot's measured peak in `recommend.rs` (Chatterbox Turbo, 8,018 MiB) re-measured for Breeze, or `mecha features --probe` sums a model the box no longer holds.
6. The units installed and `mecha doctor` clean. Then the worker drop-in, restarted by the update skill's order: worker first.

## 6. Rulings (owner, 2026-10-03)

- **D1.** The model writes delivery instructions (free text), sent as `instructions`. It may change to cues plus a menu. **Refined:** a director pass writes them per sentence, on the loaded model, recorded in the session (§3); in incognito it runs and nothing is kept.
- **Brevity.** On a streaming engine the spoken-turn prompt drops its length rules (§3).
- **D2.** `default` is a Breeze house voice.
- **D3.** Parakeet auto-transcribes each reference clip once, into an editable `.txt` sidecar.
- **D4.** Speed other than 1.0 is applied by stretching the buffered sentence.
- **Q6_K is the default; Q5_K_M is kept as the fallback.**

## 7. Phases

1. **The adapter** (#523, merged): the adapter, the shared stretch module, the units and this document.
2. **The director and the streaming prompt** (the PR after #523, replacing the `[[…]]` plan): §3.
3. **The gates in §5, then the switch.** The owner switched the worker to Breeze for testing on 2026-10-03 at 03:30Z, ahead of gates 1–5, with `vctk_p297` trimmed to one reading as the stand-in house voice; Chatterbox kept running for rollback.
