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
  - speed by stretch;
  - a short pause for text with no letter or digit outside a vocal-event tag ("(laugh)" alone counts as nothing), never sent to the engine, which invents speech when given nothing to say (`breeze_server.py` module docstring, measured). The worker joins such pieces, and phrases that trail off in an ellipsis, to the sentence after them (`scripts/voice/fragments.py`), so the pause is a backstop.

  The engine's registry lives in memory, so a restarted engine has forgotten every voice. Measured against the real engine (2026-10-03): `GET /v1/audio/voices` answers `{"voices": [{"name": …, "kind": "registered"}]}`, and speaking an unregistered voice is HTTP 400 `unknown voice '<name>'`. The adapter asks the registry before retrying, and re-registers when the registry cannot be read.

  Unit: `scripts/voice/mecha-breeze-adapter.service`, which runs from the shared checkout as the worker does. Tests: `scripts/voice/test_breeze_server.py`, with stand-ins for the engine and the STT.
- **The switch.** Breeze is the worker's default TTS: `MECHA_VOICE_TTS` defaults to `http://127.0.0.1:8887/v1`. **Rolling back** to Chatterbox means setting `MECHA_VOICE_TTS=http://127.0.0.1:8881/v1` in a drop-in on `mecha-voice-worker.service`, starting the `chatterbox` container (its restart policy is `no` since 2026-10-03, so it is down after a reboot), and restarting the worker. Chatterbox ignores `default.wav`: its `default` is its own built-in voice, so on the rollback the library's `default` is Chatterbox's, not the clip.

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
4. **What a direction may name (owner, 2026-10-03: texture stays; pitch removed or constrained).** The owner heard the voice "sound like a different person" line to line. Breeze's direction mode is defined as steering tone, emotion, pace and delivery while keeping the reference speaker's identity; its own benchmark grades pitch *movement*, whispering and phonation, never a pitch level. Measured on the engine over one logged turn's directions (8 sentences × 3, texture kept throughout), the mean pitch move between adjacent lines was 5.5 semitones with "low/high/medium pitch" in the directions, up to 14.5; 2.2 with those words removed; 1.5 with an identity anchor in front. So the director directs emotion, energy, pace, emphasis and texture, may move pitch within a line, never names a level, and directs only the voice (no gestures, gaze or scene); code adds the anchor; and each reply carries the last direction of the one before. Its texture examples are balanced (plain beside expressive, "or none"): a list of only breathy and husky textures was copied as a menu, and no list at all wandered into gestures and vocal events. With the real director over three logged replies, the new frame named no pitch level in 24 directions and moved 1.6 semitones on average, 5.3 at most, against 9 of 24, 2.8 and 18.0 for the frame before it; latency was unchanged (median 0.7 s). The reasoning and each variant's numbers sit on `voice_direction::SYSTEM`. Upstream's own identity control, dual CFG weighting the reference against the instruction, is not ported and costs three passes a frame, which this machine cannot stream.
5. **Harness speech is not directed.** Offers, draft read-backs, failure lines and the persona safe message are skipped with reason `harness`.
6. **Every direction is recorded** in the conversation's transcript as a `spoken_direction` record: the sentence, the direction, the model, the latency, the outcome and a prompt digest, joined by turn and sentence rather than by file position. In an incognito chat the director runs and nothing is kept, in the file or in any journal (owner ruling, 2026-10-03).

### 3.1 Listen is directed too, once per reply

A reply read aloud with Listen gets one direction for all of it (owner, 2026-10-03: "a director pass before reading", then "just do it for the entire turn … let's start simple"). Calls are unchanged: every sentence there is still directed.

- **How.** The page sends the reply's key, the piece's position, the reply's text, the owner's words it answered and the speaker's previous reply with each piece. On the first piece, serve asks the worker whether its engine takes directions (`GET /mecha/directs`), asks the director once with the whole reply (`Cue::Reply`, the same frame and anchor as a call), and sends that line as `instructions` with every piece of the reply. The §3 rules carry over: the loaded model, the 2.5 s deadline, the single-slot and switching skips, and any miss speaks the reply undirected.
- **Why once.** A per-sentence direction must be ready before the sentence ahead of it finishes playing, so a short sentence followed by a slow direction leaves a silence. One direction costs a single wait before speech starts and none after. It also cannot drift between lines (§3, item 4); the price is that a reply which changes mood is spoken in one delivery.
- **Pieces, streamed (owner, 2026-10-04: "stream it, like calls").** Whole sentences grouped up to ~250 characters, each streamed: the page asks with `stream: true`, the worker answers raw 24 kHz PCM as the engine makes it (`tts_pcm`, `x-sample-rate`), serve passes the body on unbuffered, and the page plays it through one Web Audio clock (`pcm.js`), asking for the next piece as soon as the last has *arrived*, up to 20 s ahead of the listener. A worker older than streaming answers one WAV, decoded and played through the same clock.
  - *Why.* The first design cut the first sentence alone and grouped the rest up to ~400 characters, on the belief that each piece is made while the previous plays. It is not, for the second: a whole WAV is heard only once it is all made, and the first piece covers the second's synthesis only for its own length. Measured 2026-10-04 on 159 of a persona chat's replies (median first sentence 18 characters; a 400-character piece is ~26 s of audio, 11 s to make at idle): a median 8 s silence after the first sentence at idle, 16 s while a chat generated. Streamed, through the real player against the live engine: first sound 0.27–0.45 s after the tap and no gap in 35–38 s of speech at idle; one 0.25 s gap in 36 s while a chat generated.
  - *Not the first sentence alone any more.* A piece's length no longer decides how soon it is heard, and a lone opener ("Oh.") is the short input the engine invents speech around. The 250 cap keeps a piece inside what the engine renders whole (a long passage can stop a sentence early, `breeze-qwentts` README).
  - *A stop stops the synthesis.* The page aborts its request; serve drops the body, which closes the worker's request and the engine's (`tts_pcm`'s iterator closes the TTS request when dropped).
- **Heard again, spoken the same (owner, 2026-10-03: "if the directions were saved it should just reuse them").** An earlier tap's direction, recorded under the reply's key, is reused with no model call. A reply that was spoken in a call reuses that call's first direction: the call turn that spoke at least half the reply's sentences, if exactly one did, and the reply has at least four words. A text-chat reply has nothing saved the first time, so its first tap is directed and recorded.
- **Recorded** as one `spoken_direction` per reply under the turn `listen:<key>`, with `sentence` holding the reply it covers. In an incognito chat the director runs and nothing is kept.
- **Not yet measured:** how the whole-reply cue sounds against per-sentence directions, and the added wait before the first sentence (one director call, median 0.7–1.6 s on the call measurements).

**Mitigations:** the adapter caps a direction at 300 characters and flattens it to one printable line, and a direction can change only how a sentence sounds, never what is said or sent.

**The prompt on a streaming engine (owner ruling, 2026-10-03: "breeze streams… now we don't need things short").** The spoken-turn rules about length ("short sentences", "the first one short", "keep replies brief") were a latency control for an engine that speaks a sentence only once all of it is synthesised. The adapter lists `"streams": true`, the worker passes `X-Voice-TTS-Streams: 1`, and the facade opens the turn with `VOICE_BLOCK_STREAMING` and the persona call note without its length rule. Every other voice rule is kept. Without the header, the prompt is byte-for-byte what it was.

## 4. Voices

- **Transcripts (ruling D3).** Each `<name>.wav` gets `<name>.txt`, written by Parakeet the first time the voice is spoken (about 5 s once) and editable afterwards. The port study measured Breeze tolerating small transcript errors.
- **The default voice (ruling D2).** `default` is `default.wav` (plus its `.txt`) in the voices directory: a clip like any other, named for what it is, because Breeze has no built-in voice. A missing one is a 503 that says so, never another voice. `make-voices.py` writes one from a Kokoro reference when there is none, and `default` is a reserved name in the library, so no recording, upload or delete can replace it by accident. On the owner's machine it is `vctk_p297` trimmed to one reading (2026-10-03).
- **VCTK clips say their sentences twice** (the dataset carries each utterance once per microphone, `mic1` and `mic2`, with identical text), and Breeze copies that delivery. `add-vctk-voices.py` now skips a row whose text repeats the one before and writes the corpus text as the clip's `.txt`. Clips added before that still say everything twice: delete `vctk_<id>.wav` and its `.txt` and run the script again to re-cut one.
- **Speed (ruling D4).** At 1.0 the PCM streams through. At any other speed the adapter buffers the sentence and applies the WSOLA stretch shared with Chatterbox (`scripts/voice/audio_stretch.py`), so that sentence waits for its synthesis.

## 5. Gates

The switch was made on 2026-10-03 (§7) with these not all met. As of that day: **1** met (the owner chose `vctk_p297`, trimmed to one reading, as `default.wav`); **3** met for clips added from then on (`add-vctk-voices.py` dedupes), older VCTK clips still say everything twice until re-cut; **5** measured by mecha-a3 (4,185 MiB peak, #526); **2**, **4** and **6** open. Gate 4 is the operational risk: the Chatterbox container no longer restarts at boot, so a boot where `tts-server` loses the CUDA-init race leaves voice with no TTS until a person starts one (its unit retries every 5 s).


1. The default voice (`default.wav`) exists and the owner has chosen it.
2. A call measured while ComfyUI renders a picture. Q6_K runs at 0.76 under steady chat load and image generation is a heavier co-tenant. If it is too tight: a larger pre-buffer when the GPU is busy, Q5_K_M, or the fork's fused depth layer.
3. The VCTK clips re-cut.
4. **The engine starts on a busy box.** On 2026-10-03 at 02:43Z and 02:45Z `tts-server` failed at CUDA init with `NV_ERR_NO_MEMORY` from `kgrctxAllocMainCtxBuffer` (kernel log), while `MemAvailable` read 42 GB and `MemFree` 20 GB: the driver could not allocate a context although the page cache was reclaimable. A unit that restarts into that loops. Find what the driver needs free, and whether starting at boot (before the cache fills) or a start-time retry is the answer.
5. The `tts` slot's measured peak in `recommend.rs` (Chatterbox Turbo, 8,018 MiB) re-measured for Breeze, or `mecha features --probe` sums a model the box no longer holds.
6. The units installed and `mecha doctor` clean. Then the worker drop-in, restarted by the update skill's order: worker first.

## 6. Rulings (owner, 2026-10-03)

- **D1.** The model writes delivery instructions (free text), sent as `instructions`. It may change to cues plus a menu. **Refined:** a director pass writes them per sentence, on the loaded model, recorded in the session (§3); in incognito it runs and nothing is kept.
- **Brevity.** On a streaming engine the spoken-turn prompt drops its length rules (§3).
- **D2.** `default` is a Breeze voice clip the owner chooses. Simplified the same day: it is named `default.wav`, a clip like any other, rather than a separate "house" voice behind the name.
- **D3.** Parakeet auto-transcribes each reference clip once, into an editable `.txt` sidecar.
- **D4.** Speed other than 1.0 is applied by stretching the buffered sentence.
- **Q6_K is the default; Q5_K_M is kept as the fallback.**

## 7. Phases

1. **The adapter** (#523, merged): the adapter, the shared stretch module, the units and this document.
2. **The director and the streaming prompt** (#527, merged, replacing the `[[…]]` plan): §3.
3. **The switch, made 2026-10-03.** The owner switched the worker to Breeze at 03:30Z, ahead of the gates in §5, and then made it the repo default (#528): `worker.py` defaults to the adapter on `:8887`, and `default` is the clip `default.wav` (on this machine `vctk_p297` trimmed to one reading). Both Breeze units start at boot. The Chatterbox container no longer restarts at boot and was stopped the same morning; §2 has the rollback.
4. **The gates in §5** that the switch left open.
