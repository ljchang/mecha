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
  tiers takes the row below it.

Each cell says where its numbers come from. **Measured** means on a real
machine, named; **Arithmetic** means from the formula above, which is
architectural and transfers; **Unmeasured** means nobody here has tried it.
Only one cell is measured.

| Tier | Unified memory | Separate GPU, with system RAM beside it |
|---|---|---|
| 16 GB | An 8B at Q4, or a 14B at Q4 with little else running; 32k context. *Arithmetic* | A 14B at Q4 on the card with room for its context; OCR, embeddings and speech to text in system RAM. *Arithmetic* |
| 32 GB | A 14B at Q4–Q6, or a ~27–35B MoE at Q4 with modest context. *Arithmetic* | A 35B MoE at Q4 with 128k context (~24 GB) on the card. *Arithmetic* |
| 64 GB | A 30–35B MoE at Q4–Q5 with 128k–256k context, and an embeddings server. *Arithmetic* | The same model and context, with image generation's ~15 GB beside it on the card. *Arithmetic* |
| 128 GB | A 35B-class MoE at Q4 with four slots of 262k, *Measured — DGX Spark (GB10)*; every other feature's model beside it, *Arithmetic* (the sum under [Beside the chat model](#beside-the-chat-model)) | The 128 GB unified row, with the smaller models moved to system RAM. *Arithmetic* |

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
  reach of a 16 GB card — unmeasured here, and slower by an amount set by
  your system RAM's bandwidth.

### 32 GB

The first tier where a mid-size model is comfortable.

- **Model**: a 14B at Q4–Q6, or a ~27–35B MoE at Q4 if you keep context modest.
- **Context**: 64k–128k depending on the model's KV shape.
- **Expect**: this is the point where the assistant workflows in these docs
  start to feel like they are working rather than being demonstrated.
- **With a separate GPU**: a 35B-A3B at Q4 with 128k context is ~24 GB
  (20.7 weights + 2.7 cache + 0.9 projector), which leaves the card ~8 GB
  and nothing else needs it.

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
`nvidia-smi` for GPU memory, `ps` for the rest — in **GiB**, the base both
tools report. The three figures carried from earlier measurements (OCR,
layout, and image generation's peak) were written down in GB without saying
which; the two bases differ by 7%. *How it
holds memory* is the part that decides the sum: **resident** holds it from
start to stop, **on demand** holds nothing until the first request and frees
it after ten idle minutes, and **per request** holds it only while working.

| Feature | Model | Runs on | How it holds memory | Cost on the GB10 (GiB) | Evidence |
|---|---|---|---|---|---|
| Chat — every feature | Qwen3.6-35B-A3B Q4_K_M, with its vision projector | GPU | Resident | 41.5 at four 262k slots — the server process's GPU memory; the router's prompt cache is host memory on top (below) | Measured 2026-10-02 |
| `graph` — embeddings; persona file search | harrier-oss-v1-0.6b f16, 32k context | GPU | On demand | 5.1 loaded | Measured 2026-10-02 |
| `documents` — OCR | PaddleOCR-VL 1.6 (GGUF and projector) | GPU | On demand | 2.6 loaded | Measured 2026-09-29 |
| `documents` — layout | PP-DocLayoutV3 (ONNX) | CPU | Per request | 1.1 peak | Measured 2026-09-29 |
| `image` | Qwen-Image 2.1 Q4, in ComfyUI | GPU | Resident — ComfyUI keeps its models between pictures | ~15 peak at 1024²; 11.9 held while idle | Measured 2026-09-25 (peak), 2026-10-02 (idle) |
| `voice` — speech to text | Parakeet TDT 0.6B v3 int8 | CPU | Resident | 0.7 | Measured 2026-10-02 |
| `voice` — speech | Chatterbox Turbo | GPU | Resident | 5.4, and 2.5 of system memory | Measured 2026-10-02 |
| `voice` — turn detection | Silero VAD and smart-turn v3, in the voice worker | CPU | Resident | 0.5 | Measured 2026-10-02 |

Added up — which is *arithmetic*, since nobody has seen every row loaded at
the same moment — that is about 67 GiB of GPU memory with image generation
idle (70 at its peak), and about 6 GiB of host memory for the processes
around them. **The router's prompt cache comes on top**: `cache-ram` lets it
keep up to 16 GiB of saved prompt prefixes in host memory
(`scripts/start-router.sh`). On a unified pool all of it is the same memory —
73 GiB, and up to 89 GiB with a full prompt cache, of the GB10's 121 GiB
usable, before the operating system.

Image generation's peak sits ~3 GiB above its idle figure, and by default
`image_generate` refuses to start with less than 16 GiB available (`[image]
min_available_mb`), so on a smaller
unified machine it refuses while the chat model is loaded rather than taking
the machine down.

**On a separate GPU**, the CPU rows stay where they are and cost system RAM.
The two on-demand llama-servers (embeddings and OCR) can run on the CPU
from system RAM with `-ngl 0`, slower by an amount nobody here has measured.
Image generation and Chatterbox want the card. **None of the separate-GPU
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
