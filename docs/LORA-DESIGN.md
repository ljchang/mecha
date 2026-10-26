# LoRAs for pictures and clips — one registry, one picker

Status: **design, not built** (2026-10-26). §8 holds the decisions the
build waits on.

mecha ships no LoRA and names none (the owner's ruling, 2026-10-23). This
document is the mechanism only. Every name, file and trigger word in it is
made up, and the one LoRA it names by kind is MiniMax's own turbo
distillation, which the video graph already carries. Which LoRAs exist on a
machine is that owner's files and that owner's registry, and neither is in
any repository.

## 1. Where it stands

**Clips have a registry and no picker.** `videogen::loras` reads a TOML
file the owner writes (`[video] loras`). Each entry has a `name`, a one-line
`about` shown to the model, a file per H3 mode (`frames`, `references`), a
`strength`, an optional `trigger`, and `always`. The model picks up to
`MAX_PER_CLIP` entries per clip through `motion`. An `always` entry rides
every clip in a mode it has a file for and is never offered. Every chat with
video on is offered every entry: v1 has no per-chat list, which the 10-25
ruling accepted as a first step ("registry file, everywhere").

**Pictures have no LoRAs at all.** The image graph is `UNETLoader` →
`KSampler` with no LoRA node. Picture turbo is a separate distilled
checkpoint (`sampling::Model::Turbo`), not a LoRA. The 10-23 ruling says
picture LoRAs are a picture setting, applied to every picture in that chat
or persona, never named by the model. That was never built.

**Nothing is editable from the web.** The registry is a hand-edited file,
and nothing on any page says which LoRAs exist or which are on.

**The registry assumes its base model rather than recording it.** It is
video-only, so "made for H3" is implied by which file it is in. The mode is
checked: a pick without a file for the clip's mode is dropped with a note.
Whether a file was trained for H3 at all is not checked. A LoRA trained for
another model loads without an error and changes little or nothing, because
the loader skips weights whose keys the model does not have, and that
silence is the failure this design closes first.

## 2. What a file says about itself

A `.safetensors` LoRA starts with a JSON header of tensor names, which can be
read without loading a tensor. Read across every LoRA file on the owner's two
servers (2026-10-26, 41 files, names not recorded), the key shapes split
cleanly by model family:

| Family | Key shape |
|---|---|
| Qwen-Image | `transformer_blocks.*` (under a `diffusion_model.` or `transformer.` prefix) |
| H3 | `blocks.*` beside `token_refiner.*` or `audio_patch_proj.*`, or the kohya spelling `lora_unet_blocks_*` |

The `__metadata__` block is not evidence. Ten of the 41 had no base-model
field at all, and the field names differ by trainer. So:

- **Family is read from the keys.** A file whose keys match neither family
  is *unknown*, and unknown is refused, never assumed.
- **Mode is declared, never read.** H3's references and frames checkpoints
  share one architecture, so their LoRAs have the same keys. The owner's
  tests on 10-24 showed a LoRA on the other checkpoint breaks clips, so the
  registry keeps a file per mode, as it does now.

## 3. One registry

One file, for both pictures and clips:

```toml
[[lora]]
name = "watercolor"
about = "loose watercolor washes"
model = "qwen-image"                   # read from the file's keys and checked
file = "watercolor_qi.safetensors"     # in the image server's models/loras
strength = 0.7
trigger = "watercolor painting"

[[lora]]
name = "wave"
about = "a friendly wave"
model = "h3"
references = "wave_ref.safetensors"    # a file per H3 mode, as today
frames = "wave_fl.safetensors"
strength = 0.8
```

- **`model` is written by the owner and checked against the file.** A file
  whose keys say otherwise costs that entry, with the reason on the page and
  on stderr, never the whole registry. That is the rule `loras.rs` already
  follows for an entry that does not read. A file mecha cannot read, for
  example a ComfyUI install it was not told the path of, is kept and marked
  *unchecked* on the page, never shown as checked.
- **Each graph gets only entries of its family.** The picture graph sees
  `qwen-image` entries and the clip graph sees `h3` entries. Today the clip
  path drops a pick with no file for the clip's mode; the family check adds
  the step before that.
- **`always` stays, and means the same thing for both.** It applies to every
  picture or clip in its family, outside any per-chat choice, and is never
  offered to the model.
- **The old file is read for one release.** `[video] loras` keeps working
  and is reported by `mecha doctor` as moved. Its entries become `model =
  "h3"` with no other change. §8, L2 decides where the new one lives.

## 4. Who chooses which LoRAs run

Picture settings already have two layers: Settings → Images is the chat
layer, and each persona's `state.toml` overrides it field by field
(`Tuning::over`). LoRAs ride the same layers.

| | Pictures | Clips |
|---|---|---|
| Who picks | the owner, per layer | the model, per clip, from the owner's list |
| Chat layer | the LoRAs on every picture | the LoRAs the model may pick |
| Persona layer | overrides the chat's list, or inherits it | the same |
| The model sees | nothing (the ruling) | each allowed entry's `name` and `about` in the `motion` enum |
| Cap | `MAX_PER_PICTURE`, set by measurement M3 | `MAX_PER_CLIP` (4), unchanged |

- **"Inherit" and "none" are different states.** A persona that inherits
  follows the chat layer as it changes. A persona set to an empty list stays
  empty. `Tuning` already writes an explicit choice and leaves an unset one
  out, for the same reason.
- **A setting changes the tool list, so it changes the cached prefix.** The
  `motion` enum is part of `video_generate`'s schema, at the front of the
  prefix. A change in settings applies from the next turn and re-pays the
  prefix once. Nothing changes per turn, and the enum is sorted so the same
  choice always produces the same bytes.
- **Settings are written only by the owner's settings route**, as
  `set_tuning` is. No tool writes them, and the model never authors a
  registry entry. An `about` line is owner-written prompt text, so it is
  trusted the way a persona's own text is.
- **What a result says.** A picture result gives the number of LoRAs from
  settings, not their names, because the model has no say in them. A clip
  result keeps naming what the model picked, as it does today, so it can say
  what it chose.

## 5. The picture graph

One `LoraLoaderModelOnly` per entry between `UNETLoader` and `KSampler`, in
registry order. The clip graph builds its chain the same way
(`videogen::graph::lora_chain`). A trigger is added to the prompt when the
prompt does not already contain it, with the same rule as
`loras::with_triggers`. Two things are unknown, so the build waits on them:

- **M1. Does a LoRA patch the int8 checkpoint?** The standard picture model
  is `int8_convrot`. LoRA patching on quantized weights is not given. Test:
  one same-seed pair with and without one LoRA, plus render time and peak
  memory, against the bf16 checkpoint for the same pair.
- **M2. Do LoRAs trained on the base model work on picture turbo?** Turbo
  is a distillation, not the base. Test: the same pair on turbo. If they
  fail, entries do not apply to turbo pictures, and the result says which
  were left out.
- **M3. How many at once?** Two strong action LoRAs broke a clip that each
  passed alone (2026-10-24). Test: one, two and three stacked on one scene;
  the cap is the largest count that held.

## 6. The web

- **Settings → LoRAs**, shown when the image or video feature is on, with
  the pane's `PANE_FEATURE` entry (ARCHITECTURE §Features, *Adding a
  feature*):
  - One row per registry entry: name, family, the mode files for H3, about,
    strength, trigger, `always`, and checked, unchecked or refused with the
    reason.
  - **Add** lists the files in the server's `models/loras` that no entry
    uses yet. The family is read from the file and shown. The owner fills
    in name, about, strength and trigger, and for H3 says which mode the
    file is for. A second H3 file is attached to an existing entry the same
    way.
  - Edit and remove. Removing an entry also removes it from every layer's
    list, and the page says how many layers held it.
- **Settings → Images** gains a LoRA list (the chat layer's picture LoRAs).
  **Settings → Video** is a new pane holding the chat layer's clip
  allowlist.
- **Each persona's picture settings** gain both lists, each with an
  "Inherit from chat" state.
- **Locking.** The library lock hides the Personas and Library tabs while
  locked. §8, L3 decides whether it hides this pane too. It would hide the
  pane from browsing only; what the model is offered never depends on the
  lock (the library ruling, 2026-10-13).

Server routes: `GET /api/loras` (entries, their check, the unused files),
`POST/PUT/DELETE /api/loras/{name}`, and the layers through the existing
settings routes, the way `Tuning` already travels. Each write goes through
the same parser as the file, so the page cannot write an entry the file
would refuse.

## 7. Build order

1. **Registry v2 in core.** The shared file, `model`, the key check, the old
   file read for one release, the clip path filtered by family. No change in
   behaviour for a registry that only holds H3 entries.
2. **Layers for clips.** The allowlist on the chat and persona layers, the
   enum built from it, results unchanged.
3. **Pictures**, gated on M1–M3. `Tuning` gains `loras`, the graph gains the
   chain, triggers, and the result's count.
4. **The web.** The LoRAs pane, the lists in Images, Video and persona
   settings, and the lock if L3 says so.
5. **The real-person guard**, if L5 says build it.

Each step is its own PR. The tests and docs use made-up names only.
`scripts/check-private.py` guards conversation text, not LoRA names, so each
push is also checked by hand for names from the owner's registry.

## 8. Decisions owed

- **L1. What a new chat or persona starts with.** The 10-23 ruling said
  empty. v1 gives every chat every entry.
  *Recommendation:* migrate the chat layer to today's set, so nothing the
  owner uses changes. Personas inherit. An entry added later starts off
  everywhere until it is turned on.
- **L2. Where the registry lives.** A fixed `~/.mecha/loras.toml`, or a
  config path as now.
  *Recommendation:* fixed, like triggers and skills. A layered config can
  come from a cloned repository, and a repository should not be able to add
  a LoRA to a trusted session.
- **L3. Does the library lock hide the LoRAs pane?**
  *Recommendation:* yes. The pane lists explicit descriptions, and the lock
  exists so a glance at the screen does not show them.
- **L4. Picture LoRAs on turbo:** decided by M2 rather than in advance.
  *Recommendation:* follow what M2 shows.
- **L5. The real-person guard.** The 10-23 ruling asked for this to be
  raised again before the picker ships. One shape: library characters gain
  a `real` mark, registry entries an `explicit` mark, and an explicit entry
  never applies to a picture or clip with a real character in its cast,
  with the result saying which entries were left out.
  *Options:* build it as step 5, or defer it again.
