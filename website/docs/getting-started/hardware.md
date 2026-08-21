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

### 16 GB

Enough to run something useful, not enough to stop thinking about it.

- **Model**: an 8B-class model at Q4 (~5 GB), or a 14B at Q4 if little else is running.
- **Context**: 32k comfortably. Quantise the KV cache if you want more.
- **Expect**: good tool-calling on simple errands, more recovery turns on complex ones.
- **Set** `context_window` to what `mecha setup` reads back, and leave
  compaction on — you will hit it.

### 32 GB

The first tier where a mid-size model is comfortable.

- **Model**: a 14B at Q4–Q6, or a ~27–35B MoE at Q4 if you keep context modest.
- **Context**: 64k–128k depending on the model's KV shape.
- **Expect**: this is the point where the assistant workflows in these docs
  start to feel like they are working rather than being demonstrated.

### 64 GB

Room for the model you want and the context you want at the same time.

- **Model**: a 30–35B MoE at Q4–Q5 with headroom.
- **Context**: 128k–256k on a hybrid model.
- **Also fits**: a second small server for embeddings, which the knowledge
  graph wants — one model per process, so it cannot share the chat port.

### 128 GB and up

Where a dedicated box stops making you choose.

- **Model**: a 35B-class MoE at Q4 with the full trained context window.
- **Context**: 256k per slot, and multiple slots.
- **Also fits**: embeddings, a vision projector, and a second model for
  comparison, all resident at once.

**Measured on the machine these docs were written on** — a DGX Spark (GB10,
128 GB unified), Qwen3.6-35B-A3B at Q4_K_M, four slots of 262,144 tokens:

| | |
|---|---:|
| Weights | ~20.7 GB |
| KV cache, f16 | 22.0 KiB/token |
| Vision projector | ~0.9 GB |
| Total reservation at `-c 262144` | ~28.5 GB |
| Generation, 1k prompt | ~92 tok/s |
| Generation, 108k prompt | ~63 tok/s |
| Prefill | ~1,570 tok/s |

## Two shapes of machine

### A dedicated always-on box

This is the configuration mecha is built around, and the one that makes
[triggers](/docs/features/triggers) and the [Slack remote
control](/docs/features/slack) worth having: the assistant is only useful
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

- **Unified memory has no separate GPU pool**, so the tier table applies
  directly rather than needing a VRAM-versus-RAM split.
- **Drag-and-drop into the TUI works locally and cannot work over SSH** — the
  path your terminal pastes is your laptop's. If the Mac is the machine mecha
  runs on, dropping a screenshot on the prompt just works; if it is the laptop
  you ssh *from*, use [the Slack conduit](/docs/features/images) instead.

## What to do once you have chosen

Do not type the numbers from this page into your config. Start the server, then
let mecha read them back:

```bash
mecha setup --write
```

`context_window` in particular is the **per-slot** figure, not the `-c` you
passed — see [Serving a local model](/docs/features/serving) for why that
distinction has bitten more than once.

## Next

- [Installation](/docs/getting-started/installation) — the binaries, and getting a model
- [Setting up](/docs/getting-started/setting-up) — point mecha at it
- [Serving a local model](/docs/features/serving) — slots, `-np`, and the four numbers that have to agree
