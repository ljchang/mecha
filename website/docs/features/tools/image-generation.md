---
title: Image generation
sidebar_position: 5
description: The image_generate tool — ask for a picture in chat and a local image model draws it, saved into the conversation's workspace and shown inline in web chat.
---

# Image generation

`image_generate` lets a run draw a picture with an image model on your own
machine. Ask for one in any conversation — "make me a poster for Friday's lab
meeting that says 'Journal Club'" — and the model writes a prompt, the image
server renders it, and the PNG is saved under `images/` in the conversation's
workspace. Web chat shows it under the call; the terminal shows the path.

It is registered only when `[image]` is configured. With none, the tool does
not exist.

## Setting it up

mecha does not run the image model itself; it talks to a local server. Today
that server is [ComfyUI](https://github.com/comfyanonymous/ComfyUI) with the
[ComfyUI-GGUF](https://github.com/city96/ComfyUI-GGUF) node and Qwen-Image 2.1:

1. Put the three model files where ComfyUI looks for them — the diffusion
   model under `models/diffusion_models/`, the text encoder under
   `models/text_encoders/`, the VAE under `models/vae/`.
2. Run ComfyUI on this machine only:
   `python main.py --listen 127.0.0.1 --port 8188`.
3. Add the section to `~/.mecha/config.toml` (the global file — a project's
   `mecha.toml` cannot configure this):

```toml
[image]
url = "http://127.0.0.1:8188"
diffusion_model = "Qwen-Image-2.1-Q4.gguf"
text_encoder = "qwen3vl_8b_w4a8.safetensors"
vae = "qwen_image_2.1_vae_bf16.safetensors"
```

`mecha tools` lists `image_generate` once it is configured. Every key and its
default is in the [configuration reference](/docs/reference/configuration#image).

## Using it

Ask in plain words. The model chooses the prompt, a size — `square`
(1024×1024), `landscape` (1344×768) or `portrait` (768×1344) — and, when you
are revising, a seed.

- **Text in images works well.** Say the exact words; the model quotes them in
  the prompt.
- **Revise by talking.** "Same thing, but warmer light" — the model edits its
  prompt and reuses the seed, which keeps the composition.
- **The model cannot see the picture.** Images reach a conversation only when
  you attach them, so it knows what it asked for, not what came out. You are
  the judge; tell it what to change.
- **It works in any chat, read-only ones included.** It only ever adds new
  files under `images/` in the conversation's own folder, so it needs no
  approval and no mode switch.
- **It takes about a minute** for a 1024×1024 image at the default 40 steps.
  Cancelling the run stops the generation on the server too.

## What it will refuse, and why

- **A server that is not on this machine.** `url` must be `127.0.0.1`, `::1`
  or `localhost`. Your prompts can contain anything the conversation holds,
  including private mail, and the tool declares that nothing leaves the
  machine — so it refuses to start otherwise rather than make that untrue.
  Because nothing leaves, image generation keeps working in a conversation
  that has read your mail, where tools that send are refused.
- **Starting without memory to spare.** On machines where the GPU shares
  system memory, a generation competes with everything else running, and
  running out takes more than the image down. Below `min_available_mb`
  (16 GB) the tool declines and says why; try again once a large build or
  another model has finished. After ten idle minutes it asks the server to
  unload its models.
- **Anything but a prompt, a size and a seed.** The workflow the server runs
  is fixed in mecha's code. The model fills in values; it never writes the
  workflow, because an image server will run whatever workflow it is handed.
