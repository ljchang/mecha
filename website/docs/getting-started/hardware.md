---
title: Choosing hardware
sidebar_position: 2
description: What memory actually buys you, recommended configurations by unified-memory or VRAM tier, and what a dedicated box for this looks like.
---

# Choosing hardware

mecha itself is a thin client — tens of megabytes of host RAM, no GPU, no model.
Everything on this page is about the machine that holds the **weights**.

The whole question is memory. Not compute: a modern MoE model generates at
usable speed on almost anything that can hold it, and the thing that stops you
is running out of room for the weights plus the KV cache. So the useful way to
choose hardware is to work out what fits.

## The arithmetic

Two costs, and only one of them is obvious.

```
total ≈ weights + (KV per token × context) + a little overhead
```

**Weights** are set by the model and the quantisation. A 4-bit quant is
roughly `0.6 GB per billion parameters` — so a 35B model is about 21 GB, a 27B
about 17 GB, a 14B about 9 GB, an 8B about 5 GB. This is the number people
already know.

**KV cache** is the one that surprises people, because it scales with the
context you configure and is *reserved at startup*, not grown on demand. It
depends heavily on the architecture:

| Model shape | KV per token | 128k context |
|---|---|---:|
| Hybrid attention (e.g. Qwen3.6-35B-A3B — only 11 of 41 layers hold a cache) | ~22 KiB | ~2.7 GB |
| Dense attention, same size | ~82 KiB | ~10 GB |

That is a **4x difference for the same parameter count**, and it is why a
hybrid-attention MoE is the shape worth looking for if you want long context on
a fixed memory budget. Quantising the KV cache (`q8_0`, `q4_0`) roughly halves
and quarters it again.

:::tip Long context is cheaper than it looks on the right model
Measured here: generation held **63 tok/s at 108k context against 92 at 1k**,
where a dense model of the same size falls much harder — only the
full-attention layers do work that scales with context, and the rest are
constant-size recurrent updates. The expensive part of long context is
*prefill*, not generation.
:::

## By memory tier

Assume the machine is doing nothing else. If it is your daily driver, subtract
what you actually use — a browser and an IDE are several gigabytes.

There are two kinds of machine, and the tier means something different in
each:

- **Unified memory** — a GB10, a Mac, an AMD Strix Halo: one pool holds the
  operating system, the chat model, and every other model you run beside it.
  The tier is that pool.
- **A separate GPU** — a graphics card with its own memory, and the computer's
  system RAM beside it. The tier is the **card's** memory. The chat model is
  sized to the card, and the smaller models other features use (OCR,
  embeddings, speech to text) can run from system RAM on the CPU instead, so
  a 24 GB card in a 64 GB computer is not a 24 GB machine. A card between two
  tiers takes the row at or below it for the model family; whether it fits
  is the card's actual memory against the sum, never the row's label.

Each figure says where it comes from. **Measured** names the machine and
the date; **Arithmetic** is the formula above, or measured costs added up
for a machine nobody here has run — it transfers, because it is
architectural; **Unmeasured** means there is no number at all. A separate
GPU has two pools, so its cells carry two figures, one per pool, each with
its own marker. Figures from the formula are in GB, like the formula;
measured ones, and sums of them, in GiB, as `nvidia-smi` and `ps` report
them. No cell in the table below is measured on the file it names: the
128 GB chat figure is the recommended file's size plus four slots' cache,
arithmetic until that file is read (an uncensored build of the same base,
read on the GB10, brackets it: 41.5 GiB before its MTP graft, 44.7 after).
What most of the other models cost *is* measured, under [Beside the chat
model](#beside-the-chat-model) — image generation's peak is two readings
added up — and the tiers they are placed into are not.

| Tier | Unified memory | Separate GPU, with system RAM beside it |
|---|---|---|
| 16 GB | An 8B at Q4, or a 14B at Q4 with little else running; 32k context. ~5–9 GB of weights plus the cache, *Arithmetic* | **GPU**: a 14B at Q4 with its context, ~9 GB of weights plus the cache, *Arithmetic*; or a 35B-A3B with its experts in system RAM (`--n-cpu-moe`), *Unmeasured*. **System RAM**: embeddings and OCR on the CPU plus the CPU-side models, ~10 GiB, *Arithmetic* — plus the offloaded experts if you take that path, most of the model's ~21 GB, *Arithmetic* |
| 32 GB | A 14B at Q4–Q6, or a ~27–35B MoE at Q4 with modest context. ~24 GB for the MoE at 128k, *Arithmetic* | **GPU**: a 35B MoE at Q4 with 128k context, ~24 GB, *Arithmetic*. **System RAM**: as at 16 GB, ~10 GiB, *Arithmetic* |
| 64 GB | A 30–35B MoE at Q4–Q5 with 128k–256k context, and an embeddings server. ~30–37 GB, *Arithmetic* | **GPU**: the chat model and its context (the unified cell's figure without the embeddings server), with image generation's ~15 GB and speech's ~4 GB beside it, ~49 GB, *Arithmetic*. **System RAM**: embeddings and OCR on the CPU, the CPU-side models and the GPU servers' host memory, ~12 GiB, *Arithmetic* |
| 128 GB | A 35B-class MoE at Q4 with four slots of 262k: ~44.2 GiB, *Arithmetic* — the recommended file and its cache. Every other feature's model beside it: ~79 GiB in all with everything loaded, up to ~95 with a full prompt cache, *Arithmetic* ([the sum](#beside-the-chat-model)) | **GPU**: the GPU models' card memory, ~48 GiB resident and ~75 with everything loaded — counting image generation's whole peak on the card, since its split is unmeasured — *Arithmetic*. **System RAM**: the CPU-side models and the GPU servers' process memory, ~4 GiB, and up to ~20 with a full prompt cache, *Arithmetic* |

### 16 GB

Enough to run something useful, not enough to stop thinking about it.

- **Model**: an 8B-class model at Q4 (~5 GB), or a 14B at Q4 if little else is running.
- **Context**: 32k comfortably. Quantise the KV cache if you want more.
- **Expect**: good tool-calling on simple errands, more recovery turns on complex ones.
- **Set** `context_window` to what `mecha setup` reads back, and leave
  compaction on — you will hit it.
- **With a separate GPU**: the whole card goes to the chat model, so a 14B
  at Q4 (~9 GB) fits with its context. llama.cpp can also keep an MoE's
  expert weights in system RAM (`--n-cpu-moe`), which puts a 35B-A3B within
  reach of a 16 GB card, with most of its ~21 GB of weights then in system
  RAM — unmeasured here, and slower by an amount set by your system RAM's
  bandwidth.

### 32 GB

The first tier where a mid-size model is comfortable.

- **Model**: a 14B at Q4–Q6, or a ~27–35B MoE at Q4 if you keep context modest.
- **Context**: 64k–128k depending on the model's KV shape.
- **Expect**: this is the point where the assistant workflows in these docs
  start to feel like they are working rather than being demonstrated.
- **With a separate GPU**: a 35B-A3B at Q4 with 128k context is ~24 GB
  (20.7 weights + 2.7 cache + 0.9 projector), which leaves a 32 GiB card
  ~10 GB — room for speech's ~4 GB, not for image generation's ~15 GB.

### 64 GB

Room for the model you want and the context you want at the same time.

- **Model**: a 30–35B MoE at Q4–Q5 with headroom.
- **Context**: 128k–256k on a hybrid model.
- **Also fits**: a second small server for embeddings, which the knowledge
  graph wants — one model per process, so it cannot share the chat port.
- **With a separate GPU**: the same, plus room on the card for image
  generation's ~15 GB peak beside the chat model.

### 128 GB and up

Where a dedicated box stops making you choose.

- **Model**: a 35B-class MoE at Q4 with the full trained context window.
- **Context**: 256k per slot, and multiple slots.
- **Also fits**: embeddings, a vision projector, and a second model for
  comparison, all resident at once.
- **With a separate GPU**: a 128 GB card (or several adding up to it) holds
  everything the unified row does on the card, and the CPU-side models stay
  in system RAM; the prompt cache is host memory either way. No separate
  GPU at this size has been measured here.

**Measured on the machine these docs were written on** — a DGX Spark (GB10,
128 GB unified), Qwen3.6-35B-A3B at Q4_K_M. It runs four slots of 262,144
tokens (`-c 1048576 -np 4`); each extra slot adds another slot's worth of KV
cache on top of the single-slot reservation below:

| | |
|---|---:|
| Weights | ~20.7 GB |
| KV cache, f16 | 22.0 KiB/token |
| Vision projector | ~0.9 GB |
| Total reservation, one 262,144-token slot (`-c 262144 -np 1`) | ~28.5 GB |
| Generation, 1k prompt | ~92 tok/s |
| Generation, 108k prompt | ~63 tok/s |
| Prefill | ~1,570 tok/s |

## Beside the chat model

The chat model is not the only one. Several of mecha's
[optional features](/docs/reference/cli#features) run a model of their
own, each in its own server, and the question is never whether one fits on
its own — it is whether it fits **beside the chat model and everything else
you have switched on**.

What each costs on the GB10 above, read from the running servers —
`nvidia-smi` for GPU memory, `ps` for the rest, added together where both
were read at one moment — in **GiB**, the base both tools report. Layout's
figure was written down as "1.1 GB" without saying which base, and is read
here as GiB; the two differ by 7%. The table is generated from the registry
in `mecha-core/src/recommend.rs` — `mecha features --probe` adds up the same
rows for whatever you have switched on, on your machine — and a test fails
when the two disagree, so change the registry and paste the table it prints.
*How it
holds memory* is the part that decides the sum: **resident** holds it from
start to stop, **on demand** holds nothing until the first request and frees
it after ten idle minutes, **released on idle** holds it until a timer beside
the server gives it back (below), and **per request** holds it only while
working.
It describes how each is installed here: the embeddings and OCR servers sit
behind a systemd socket, and started by hand instead they are resident,
which puts the resident sum near 59 GiB rather than 50.

| Feature | Model | Runs on | How it holds memory | Cost on the GB10 (GiB) | What it counts | Evidence |
|---|---|---|---|---|---|---|
| chat — every feature | Qwen3.6-35B-A3B Q4_K_M, with its vision projector | GPU | Resident | ~44.2 | the pinned files and four 262k slots' cache; an uncensored build of the same base read 41.5 GiB on the GB10 before its MTP graft and 44.7 after; not counted: the chat server's process memory, and the router's prompt cache (`cache-ram`, up to 16 GiB) | Arithmetic |
| embeddings — `graph`, `documents`, `personas` | harrier-oss-v1-0.6b f16, 32k context | GPU | On demand | 5.8 | loaded: GPU and process memory | Measured 2026-10-02 |
| OCR — `ocr` | PaddleOCR-VL 1.6 (GGUF and projector) | GPU | On demand | 3.4 | loaded: GPU and process memory | Measured 2026-10-02 |
| layout — `layout` | PP-DocLayoutV3 (ONNX) | CPU | Per request | 1.1 | peak process memory | Measured 2026-09-29 |
| image generation — `image` | Qwen-Image 2.1 int8 ConvRot, in ComfyUI | GPU | Released on idle | ~20.3 | peak, a picture from cold: ~1.1 idle after the reset and ~19.2 to load; ~15.8 warm, ~14.4 loaded and idle (the Q4's measured figures, carried up by what the int8 file measured above it on 2026-10-04: ~0.7 from cold, ~0.8 warm); not counted: from the resident sum, the ~1.1 GiB ComfyUI holds between pictures after the idle reset | Arithmetic |
| speech to text — `voice` | Parakeet TDT 0.6B v3 int8 | CPU | Resident | 0.7 | process memory | Measured 2026-10-02 |
| speech — `voice` | Breeze TTS 2 Q6_K, in qwentts.cpp | GPU | Resident | 4.1 | peak while speaking: GPU memory and the server's and adapter's process memory | Measured 2026-10-03 |
| turn detection — `voice` | Silero VAD and smart-turn v3, in the voice worker | CPU | Resident | 0.5 | the worker's process memory | Measured 2026-10-02 |

Added up — which is *arithmetic*, since nobody has seen every row loaded at
the same moment — the resident models hold about 50 GiB (51 with image
generation's ~1.1 GiB between pictures), and everything
loaded at once with an image generating from cold about 79, of the GB10's
121.7 GiB (`MemTotal` in `/proc/meminfo`), before the operating system.
**Two things come on top**, and the chat row says so: the chat server's own
process memory, which has not been measured for this configuration, and the
router's prompt cache — `cache-ram` lets it keep up to 16 GiB of saved
prompt prefixes in host memory (`scripts/start-router.sh`), which puts the
total near 95 GiB with a full cache.

**Release the memory beside the server, not from mecha.** mecha's own
ten-minute unload timer lives in the mecha process that drew the picture, so a
one-shot `mecha run`, or a `mecha serve` restarted inside the window, exits
before it fires — on the GB10, 11.9 GiB was still held nine hours after a
picture drawn just before a restart — and on unified memory its `/free` only
moves the weights into ComfyUI's process memory. `scripts/comfyui/install.sh`
installs a one-minute timer that restarts an idle ComfyUI holding a model,
which brings it to ~1.1 GB; the next picture reloads the model in ~15–22 s.
Without it, count image generation's peak if you use it at all.

By default `image_generate` refuses to start with less than 19 GiB available
when ComfyUI holds no model (`[image] min_available_mb`), and ~7 GiB less (12 GiB at the default) once it
has loaded one, since most of that cost is already paid. So on a smaller
unified machine it refuses while the chat model is loaded rather than taking
the machine down.

**On a separate GPU**, the CPU rows stay where they are and cost system RAM.
The two on-demand llama-servers (embeddings and OCR) can run on the CPU
from system RAM with `-ngl 0`, slower by an amount nobody here has measured.
Image generation and the speech server want the card. **None of the separate-GPU
placements has been measured** — the numbers above are what each model costs,
and where it costs it on your machine is arithmetic until you run it.

Nothing on this page has been measured at 16, 32 or 64 GB, so it does not
say which OCR or speech model a 16 GB machine should run — that is genuinely
unknown, and guessing would be worse than the gap.

## Two shapes of machine

### A dedicated always-on box

This is the configuration mecha is built around, and the one that makes
[triggers](/docs/features/automation/triggers) and the [Slack remote
control](/docs/features/interfaces/slack) worth having: the assistant is only useful
overnight if the machine is awake overnight.

A **DGX Spark** or equivalent unified-memory box is what these docs were
measured on. What matters is not the brand but the properties: enough unified
memory to hold the model and its context, and the willingness to leave it
running. A headless Linux box you reach over SSH is the assumed setup — the
TUI is designed for it, and the Slack connector exists because SSH cannot show
you a picture.

### A Mac

A Mac mini or Studio with generous unified memory is a reasonable dedicated
box, and llama.cpp's Metal backend is well supported.

:::note Unmeasured here
No Mac measurements exist in this project — every throughput number on this
page came from the GB10 machine above. The memory arithmetic is
architectural and transfers directly; the tok/s figures do not, and inventing
them would be worse than leaving the gap. Treat the tier table as the guide and
measure your own throughput once running.
:::

Two Mac-specific notes that do transfer:

- **Unified memory has no separate GPU pool**, so the tier table's unified
  column applies directly rather than needing a VRAM-versus-RAM split.
- **Drag-and-drop into the TUI works locally and cannot work over SSH** — the
  path your terminal pastes is your laptop's. If the Mac is the machine mecha
  runs on, dropping a screenshot on the prompt just works; if it is the laptop
  you ssh *from*, use [the Slack conduit](/docs/features/interfaces/images) instead.

## What to do once you have chosen

Do not type the numbers from this page into your config. Start the server, then
let mecha read them back:

```bash
mecha setup --write
```

`context_window` in particular is the **per-slot** figure, not the `-c` you
passed — see [Serving a local model](/docs/features/models/serving) for why that
distinction has bitten more than once.

## Next

- [Installation](/docs/getting-started/installation) — the binaries, and getting a model
- [Setting up](/docs/getting-started/setting-up) — point mecha at it
- [Serving a local model](/docs/features/models/serving) — slots, `-np`, and the four numbers that have to agree
