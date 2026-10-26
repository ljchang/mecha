# LoRAs for pictures and clips — one registry, one picker

Status: **design, not built** (2026-10-26). §8 holds the decisions: L2,
L3 and L5 ruled the same day, L1 and L6 owed, L4 decided by M2.

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
  follows for an entry that does not read.
- **A file mecha cannot read is refused too.** It is in the same state as
  a file whose keys matched nothing, and applying it anyway is the silence
  §1 says this design closes first. Today mecha has no path to a server's
  `models/loras`: the registry holds bare file names, the servers are spoken
  to over loopback HTTP, and only a ComfyUI that `comfyui_unit` installed
  has a `models/` path mecha knows. So each server's LoRA folder becomes a
  global config key, `[image] loras_dir` and `[video] loras_dir`, which
  `comfyui_unit` fills for a server it installs and the owner sets for a
  provided one. With the key unset, every entry for that server is refused,
  and `mecha doctor` and the page name the key. On this machine step 1's
  deploy sets both keys first, or every clip LoRA would drop.
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
| Cap | `MAX_PER_PICTURE`, set by M3, counting `always` entries | `MAX_PER_CLIP` (4), unchanged |

- **"Inherit" and "none" are different states.** A persona that inherits
  follows the chat layer as it changes. A persona set to an empty list stays
  empty. `Tuning` already writes an explicit choice and leaves an unset one
  out, for the same reason.
- **The LoRA list inherits across a model change.** `Tuning::over` drops
  the whole chat layer when the persona's model differs from the chat's,
  because cfg and steps are tuned per checkpoint. A list of names is not, so
  `loras` resolves on its own: a persona whose only choice is turbo still
  inherits the chat's list. Whether an entry then applies on turbo is M2's
  answer, filtered at render time (§5). `Tuning::lenient` gains a branch so a
  damaged list reads as inherit, never as an empty override.
- **The allowlist is enforced in `pick`, not only in the schema.** Today
  `Registry::pick` resolves a name against the whole registry, so a name the
  enum did not offer still loads. `pick` takes the resolved layer's list and
  refuses a name outside it with the same refusal as an unknown name. The
  enum is the hint, `pick` is the gate, and a test names an off-list entry
  and expects the refusal: that is the case the enum cannot cover.
- **The enum is built per run, from the resolved layer.** Today the schema
  is built once, in `VideoGenerate::new` from `prepare_tools`, and in
  `mecha serve` one instance serves every chat and persona. A run's tools are
  already chosen from that pool by name (a persona's `[tools] allow`). Step 2
  adds, at that point, a `video_generate` narrowed to the run's list, kept
  per distinct list so the same list always gives the same bytes. A change
  in settings applies from the next turn and re-pays the cached prefix once.
  Nothing changes per turn, and the enum is sorted.
- **Settings are written only by the owner's settings route**, as
  `set_tuning` is. No tool writes them, and the model never authors a
  registry entry. An `about` line is owner-written prompt text, so it is
  trusted the way a persona's own text is.
- **What a result says.** A picture result gives the number of LoRAs from
  settings, not their names, because the model has no say in them. A clip
  result keeps naming what the model picked, as it does today, so it can say
  what it chose.

## 5. The picture graph

One `LoraLoaderModelOnly` per entry between the diffusion model's loader
(`imagegen::unet_loader`'s choice) and `KSampler`, in registry order. The clip graph builds its chain the same way
(`videogen::graph::lora_chain`). A trigger is added to the prompt when the
prompt does not already contain it, with the same rule as
`loras::with_triggers`. Two things are unknown, so the build waits on them:

- **M1. Does a LoRA patch the int8 checkpoint?** The standard picture model
  is `int8_convrot`. LoRA patching on quantized weights is not given. Test:
  one same-seed pair with and without one LoRA, plus render time and peak
  memory, against the bf16 checkpoint for the same pair. A `.gguf`
  diffusion model loads through `UnetLoaderGGUF`, a second quantization with
  its own patching path. Until it is measured the same way, LoRA entries do
  not apply when `[image] diffusion_model` is a `.gguf`, and the result says
  they were left out.
- **M2. Do LoRAs trained on the base model work on picture turbo?** Turbo
  is a distillation, not the base. Test: the same pair on turbo. If they
  fail, entries do not apply to turbo pictures, and the result says which
  were left out.
- **M3. How many at once?** Two strong action LoRAs broke a clip that each
  passed alone (2026-10-24). Test: one, two and three stacked on one scene;
  the cap is the largest count that held. It is a limit on what the model
  carries, so unlike `MAX_PER_CLIP` the picture cap counts `always` entries.

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

1. **Registry v2 in core.** The shared file, `model`, the key check,
   `[image] loras_dir` and `[video] loras_dir`, the old file read for one
   release, the clip path filtered by family. No change in behaviour for a
   registry that only holds H3 entries, once `[video] loras_dir` is set.
   `scripts/check-private.py`'s `corpus()` also reads the registry's
   `name`, `file` and `trigger` and refuses them in added text, as it already
   does for persona names.
2. **Layers for clips.** The allowlist on the chat and persona layers,
   enforced in `pick`, and the narrowed `video_generate` per run. This is
   plumbing through where a run's tools are chosen, not a change inside
   `loras.rs`. Results are unchanged.
3. **Pictures**, gated on M1–M3. `Tuning` gains `loras`, the graph gains the
   chain, triggers, and the result's count.
4. **The web.** The LoRAs pane, the lists in Images, Video and persona
   settings, and the lock if L3 says so.
5. ~~**The real-person guard**~~ deferred (L5).

Each step is its own PR. The tests and docs use made-up names only. Until
step 1 puts the registry in `check-private.py`'s corpus, each push is also
checked by hand for names from the owner's registry.

## 8. Decisions owed

- **L1. What a new chat or persona starts with.** The 10-23 ruling said
  empty. v1 gives every chat every entry.
  *Recommendation:* migrate the chat layer to today's set, so nothing the
  owner uses changes. Personas inherit. An entry added later starts off
  everywhere until it is turned on.
- **L2. Where the registry lives. Ruled 2026-10-26: a fixed
  `~/.mecha/loras.toml`**, like triggers and skills, not a config path.
  The reason is consistency with the other owner-written stores, and one
  path per machine. It is not the cloned repository: `Config::merge_file`
  already strips `[video]` and `[image]` from a project layer, so a
  repository cannot point the registry anywhere today.
- **L3. The library lock hides the LoRAs pane. Ruled 2026-10-26.** The
  pane lists explicit descriptions, and the lock exists so a glance at the
  screen does not show them. It hides the pane from browsing only.
- **L4. Picture LoRAs on turbo:** decided by M2 rather than in advance.
  *Recommendation:* follow what M2 shows.
- **L5. The real-person guard.** The 10-23 ruling asked for this to be
  raised again before the picker ships. One shape: library characters gain
  a `real` mark, registry entries an `explicit` mark, and an explicit entry
  never applies to a picture or clip with a real character in its cast,
  with the result saying which entries were left out.
  **Ruled 2026-10-26: deferred again.** Step 5 is out of the build until
  it is raised again.
- **L6. LoRAs trained on one character.** Some picture LoRAs are trained on
  one person's likeness. On a chat or persona layer, one would put that face
  on everyone in every picture. One shape: an entry may name a library
  character (`character = "<name>"`), and then applies only to pictures and
  clips with that character in the cast, never through a layer's list.
