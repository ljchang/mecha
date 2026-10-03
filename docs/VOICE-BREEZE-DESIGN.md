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
  - `default` as a clip like any other (`default.wav`);
  - speed by stretch.

  The engine's registry lives in memory, so a restarted engine has forgotten every voice. Measured against the real engine (2026-10-03): `GET /v1/audio/voices` answers `{"voices": [{"name": …, "kind": "registered"}]}`, and speaking an unregistered voice is HTTP 400 `unknown voice '<name>'`. The adapter asks the registry before retrying, and re-registers when the registry cannot be read.

  Unit: `scripts/voice/mecha-breeze-adapter.service`, which runs from the shared checkout as the worker does. Tests: `scripts/voice/test_breeze_server.py`, with stand-ins for the engine and the STT.
- **The switch.** Breeze is the worker's default TTS: `MECHA_VOICE_TTS` defaults to `http://127.0.0.1:8887/v1`. **Rolling back** to Chatterbox means setting `MECHA_VOICE_TTS=http://127.0.0.1:8881/v1` in a drop-in on `mecha-voice-worker.service`, starting the `chatterbox` container (its restart policy is `no` since 2026-10-03, so it is down after a reboot), and restarting the worker.

## 3. Delivery instructions (the "dual stream")

Breeze takes, beside the text of a sentence, a free-text **voice direction**: `instructions`, rendered as `<ins_bos>…<ins_eos>` in its prompt.

**Ruling D1:** the chat model writes it, on spoken turns only. One stream leaves the model; the worker splits it:

1. **The syntax.** A direction in double square brackets, placed before the words it shapes: `[[softly, with a smile]] I'm sorry to hear that.`
   - It holds until the next one, within a turn.
   - It is never spoken.
   - An unclosed `[[` at a sentence cut is held until it closes.
2. **The worker.**
   - It takes the latest direction in each sentence and strips every `[[…]]` from the text.
   - It sends `instructions` only when the TTS lists that control. On Chatterbox the directions are stripped and dropped, so the same prompt works on either engine.
   - The echo filter is given the stripped text, since that is what the room hears.
3. **The prompt.** The spoken-turn block (`VOICE_BLOCK`, and `persona::call::note`) gains one sentence offering the syntax, **only when the TTS honours `instructions`.** The worker tells the facade so with a request header.
4. **The display.** The chat transcript keeps the model's text as written (the record is append-only). The page and the derived readers hide `[[…]]`: titles, memory, appraisal and the echo text.

**Mitigations that leave D1 as ruled:**
- the adapter caps a direction at 300 characters and flattens it to one printable line;
- a direction can change only how a sentence sounds, never what is said or sent.

The injection concern stays recorded: a reply that quotes third-party text could carry a `[[…]]`. The fixed menu of cues is the named fallback if this proves a problem.

## 4. Voices

- **Transcripts (ruling D3).** Each `<name>.wav` gets `<name>.txt`, written by Parakeet the first time the voice is spoken (about 5 s once) and editable afterwards. The port study measured Breeze tolerating small transcript errors.
- **The default voice (ruling D2).** `default` is `default.wav` (plus its `.txt`) in the voices directory: a clip like any other, named for what it is, because Breeze has no built-in voice. A missing one is a 503 that says so, never another voice. `make-voices.py` writes one from a Kokoro reference when there is none, and `default` is a reserved name in the library, so no recording, upload or delete can replace it by accident. On the owner's machine it is `vctk_p297` trimmed to one reading (2026-10-03).
- **VCTK clips say their sentences twice** (the dataset carries each utterance once per microphone, `mic1` and `mic2`, with identical text), and Breeze copies that delivery. `add-vctk-voices.py` now skips a row whose text repeats the one before and writes the corpus text as the clip's `.txt`. Clips added before that still say everything twice: delete `vctk_<id>.wav` and its `.txt` and run the script again to re-cut one.
- **Speed (ruling D4).** At 1.0 the PCM streams through. At any other speed the adapter buffers the sentence and applies the WSOLA stretch shared with Chatterbox (`scripts/voice/audio_stretch.py`), so that sentence waits for its synthesis.

## 5. Before the switch (gates)

1. The default voice (`default.wav`) exists and the owner has chosen it.
2. A call measured while ComfyUI renders a picture. Q6_K runs at 0.76 under steady chat load and image generation is a heavier co-tenant. If it is too tight: a larger pre-buffer when the GPU is busy, Q5_K_M, or the fork's fused depth layer.
3. The VCTK clips re-cut.
4. **The engine starts on a busy box.** On 2026-10-03 at 02:43Z and 02:45Z `tts-server` failed at CUDA init with `NV_ERR_NO_MEMORY` from `kgrctxAllocMainCtxBuffer` (kernel log), while `MemAvailable` read 42 GB and `MemFree` 20 GB: the driver could not allocate a context although the page cache was reclaimable. A unit that restarts into that loops. Find what the driver needs free, and whether starting at boot (before the cache fills) or a start-time retry is the answer.
5. The `tts` slot's measured peak in `recommend.rs` (Chatterbox Turbo, 8,018 MiB) re-measured for Breeze, or `mecha features --probe` sums a model the box no longer holds.
6. The units installed and `mecha doctor` clean. Then the worker drop-in, restarted by the update skill's order: worker first.

## 6. Rulings (owner, 2026-10-03)

- **D1.** The model writes delivery instructions (free text), sent as `instructions`. It may change to cues plus a menu.
- **D2.** `default` is a Breeze voice clip the owner chooses. Simplified the same day: it is named `default.wav`, a clip like any other, rather than a separate "house" voice behind the name.
- **D3.** Parakeet auto-transcribes each reference clip once, into an editable `.txt` sidecar.
- **D4.** Speed other than 1.0 is applied by stretching the buffered sentence.
- **Q6_K is the default; Q5_K_M is kept as the fallback.**

## 7. Phases

1. **This PR:** the adapter, the shared stretch module, the units (not installed) and this document.
2. **The worker:** split out the `[[…]]` directions, send `instructions` when it is listed, and give the echo filter the stripped text. Python tests.
3. **mecha:** the voice block and call note gain the direction sentence when the worker's header says the TTS honours it; the page and the derived readers hide `[[…]]`.
4. **The gates in §5, then the switch.**
