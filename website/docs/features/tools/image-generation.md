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

## Editing a picture

The same tool edits. Hand it a picture and say what to change:

- **One you generated:** tap **Edit** under it in web chat. The input fills
  with `Edit images/….png: ` — finish the sentence ("put the fox in a yellow
  raincoat") and send.
- **One of your own:** attach it with the paperclip, then say what to change.

Say what to keep as well as what to change — "keep everything else the same"
works. The edit is saved as a new file; the original is never touched. Up to
four pictures can go in one request, which is how "put the jacket from the
second picture on the person in the first" works. The result keeps the first
picture's shape unless you ask for a size.

Your photo is handed to the image server on this machine only, into a
temporary folder it clears when it restarts.

## Recurring characters: the image library

Describe a person in words and the image model draws *someone* who fits the
words — a different someone every time. To draw the same people across many
pictures, keep them in the **image library**: a portrait and a short
description per character, plus any styles you reuse.

**Adding a character.** In the web app's **Library** tab, tap **Add
character**, choose a front-facing portrait, and give it a name (`maya`) and a
short description that includes build and height. The picture is resized
before it is sent, and the photo's metadata, location included, is left
behind. Or, for a picture already in a chat, tap **Save to library** under
it. **Add style** on the Styles pane takes a name and a few lines on how the
pictures should look, and **Edit** on any character or style changes its
description or portrait. Pictures already made keep the version they used.
From a terminal:

```bash
mecha imagelib add-character maya --portrait maya.png \
  --description "a woman in her mid-30s with short curly black hair; slim, average height"
mecha imagelib add-style watercolour --text "loose watercolour, soft washes, paper texture"
```

**Using them.** Name them in a chat, and say what each is wearing and doing:
"Maya and John sharing a picnic by a lake, in the watercolour style — Maya in
a straw hat pouring lemonade, John in a linen shirt, laughing." The model
passes them to the tool by name; their portraits keep their faces. Up to four
people fit in one picture. If the model forgets and describes a library
character in words, the tool refuses before drawing and tells it how to ask.

**The model can propose characters.** Ask it to invent one — "create a
recurring character called Sam, make his portrait, and add him to my
library" — and it stages him as a candidate. Nothing uses a candidate until
you approve it: in the **Library** tab under **Waiting**, or with
`mecha imagelib list --all` and `mecha imagelib approve sam`. You approve
exactly the text you were shown.

**Locking.** Locking a character hides the whole entry — card, portrait and
description — while you browse the Library tab; it still works in any chat,
and pictures already in your chats are untouched. The lock button at the top
of the Library tab shows locked entries until you lock again, reload, leave
the tab, or leave the page untouched for the autolock — 15 minutes unless
you change it under **Settings → Lock** (or `mecha imagelib set-autolock
<minutes>`). The Personas tab shares the same lock and the same autolock.
With no password it is a plain toggle; to require one, set it once with
`mecha imagelib set-lock-password`. A character saved from a
picture made with a locked character starts locked; untick the box to save
it unlocked. Locking hides a character from then on; a portrait your browser
already showed may stay in its cache until you clear the site's data.

**People who aren't in the library.** Mention them in the scene — "a waiter
pouring coffee" — and the model adds them as `extras`: drawn as new faces
each time, and counted with your characters so they take part in the scene
instead of fading into the background.

Every generated picture also gets a small `.json` file beside it recording
how it was made — the prompt, the seed, and which library entries (and which
versions of them) it used.

## What it will refuse, and why

- **A server that is not on this machine.** `url` must be `127.0.0.1`, `::1`
  or `localhost`. Your prompts can contain anything the conversation holds,
  including private mail, and the tool declares that nothing leaves the
  machine — so it refuses to start otherwise rather than make that untrue.
  Because nothing leaves, image generation keeps working in a conversation
  that has read your mail, where tools that send are refused.
- **Starting without memory to spare.** On machines where the GPU shares
  system memory, a generation competes with everything else running, and
  running out takes more than the image down. Before each picture the tool
  asks the image server whether it already holds the model. If it does,
  12 GB free is enough, since most of the cost is already paid; if it
  doesn't, the tool needs `min_available_mb` (19 GB). Below that it declines
  and says why; try again once a large build or another model has finished.
  After ten idle minutes it asks the server to unload its models. On a
  machine running the `comfyui-idle-reset` timer (`scripts/comfyui/install.sh`),
  the server is also restarted after ten idle minutes, which returns nearly
  all of its memory, and a picture asked for during a restart waits for it
  instead of failing.
- **Leaving copies on the image server.** The server's record of each job
  (the prompt, the file names) is deleted when the job ends, however it ends.
  Set `server_temp_dir` and the uploaded pictures and the server's preview
  are deleted too; without it they stay in the server's temp folder until it
  restarts, and an [incognito chat](/docs/features/interfaces/incognito)
  will not generate images at all.
- **Anything but a prompt, a size and a seed.** The workflow the server runs
  is fixed in mecha's code. The model fills in values; it never writes the
  workflow, because an image server will run whatever workflow it is handed.
