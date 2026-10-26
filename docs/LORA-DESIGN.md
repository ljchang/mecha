# LoRAs for pictures and clips — one registry, one picker

Status: **design, not built** (2026-10-26). §8 holds the decisions: L1,
L2, L3, L5 and L6 ruled the same day; L4 is decided by M2.

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

- **The read is a bounded prefix read.** The format allows reading the
  header without loading a tensor, but the only header parse in the repo
  (`face::read_safetensors`) takes the whole file as `&[u8]`, and a LoRA is
  100 MB to several GB. The family read takes the 8-byte little-endian
  length, refuses a length over 8 MiB before using it as a size, and reads
  that many bytes only. The largest header among the owner's 41 files is
  75 KB. The length is the file's own claim, so a corrupt or absurd one is
  refused, never allocated.
- **Family is read from the keys.** A file whose keys match neither family
  is *unknown*, and unknown is refused, never assumed. A file whose keys
  match both is refused too, as ambiguous. None of the 41 does, but a
  merged or hand-built file could.
- **Nothing read from a LoRA file is ever shown or prompted as text.** The
  file is third-party content. Its keys decide one value out of a closed set
  (`qwen-image`, `h3`, unknown) and `__metadata__` is not read at all, so
  nothing in a file can reach a prompt or a page in its own words.
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
  has a `models/` path mecha knows. So each server's LoRA folders become a
  global config key, `[image] loras_dir` and `[video] loras_dir`. The
  owner sets them. `comfyui_unit::write_image_table` adds `[image]
  loras_dir` only when it writes the whole `[image]` table from nothing,
  since it never edits a table the owner wrote, and nothing writes
  `[video]`. So on every machine that already generates, step 1's upgrade
  needs the keys set by hand, or its LoRAs are refused until they are:
  the release note says so first, and `doctor` and the page name the key. Each is a list, because ComfyUI's `extra_model_paths.yaml`
  can add LoRA folders beyond `models/loras`, and the owner's video server
  has one. A file is looked up in each folder in turn. Today a registry
  name is a bare file name (`plain`); step 1 extends it to a relative path
  under the folder, since a server lists files in subfolders that way. The
  guard is lexical, on the relative path: every `/`-separated part passes
  `plain`, which already rules out `..`, a leading dot and an absolute
  part. It is not a canonicalize-inside check, because a LoRA linked in
  from the hub or another disk is mecha's own layout (`comfyui_unit` links
  every model file it fetches), and the link must still read. With the key
  unset,
  every entry for that server is refused, and `mecha doctor` and the page
  name the key. A registry entry whose file is in none of the folders is a
  named `doctor` finding, not a silent refusal: that is a store-and-stat
  read, which doctor's no-network rule allows. The other half, a file
  ComfyUI can load that mecha cannot find, needs `/object_info`, so it is
  shown on the page (§6), which already asks the server for its Add list. On this machine step
  1's deploy sets both keys first, or every clip LoRA would drop. The clip
  turbo LoRA is not one of them: it is a `[video]` config key
  (`frames_turbo_lora`, `references_turbo_lora`), not a registry entry, so
  it keeps working, and it also escapes the family check. That gap is out
  of scope here and noted for a later change.
- **The check runs when a file changes, not on every call.** Its result is
  kept per file by (path, length, mtime): the registry's load checks each
  entry's file once, and a later read re-checks only a file whose length or
  mtime moved. An unchanged file keeps its result, so a transient read
  failure on one cannot change the `motion` enum or re-pay the cached
  prefix. The reason shown keeps *unreadable* apart from *keys matched
  neither family*, though both refuse: the first can be a file mid-copy, the
  second never changes. So an unreadable file is not dropped from the
  registry. Its entry stays, marked unreadable, applied nowhere and not
  offered in the `motion` enum, and the
  stat triple it was refused under is kept, so the next read whose stat has
  moved, or one a minute later when the stat could not be taken at all,
  retries it. A file caught mid-copy recovers once the copy finishes,
  without waiting for a restart, and joins the enum then. That moves the
  enum once, when the file's state really changed, never on a transient
  read, since an unchanged stat keeps the last result. The stat triple is
  a trigger for a re-read, not a
  claim about the content: a same-length swap that keeps the mtime would
  keep its old result. That window is accepted, since the owner's own
  files are the only writers, and every process start reads every header
  afresh.
- **Each graph gets only entries of its family.** The picture graph sees
  `qwen-image` entries and the clip graph sees `h3` entries. Today the clip
  path drops a pick with no file for the clip's mode; the family check adds
  the step before that.
- **`always` is for clips only.** For clips it overrides the model's pick,
  which is its purpose. For pictures the owner is the picker, so an `always`
  entry would override the owner's own list: a persona set to an empty list
  would still get it. The parser refuses `always` on a `qwen-image` entry,
  and a LoRA wanted on every picture goes in the chat layer's list. It also
  refuses `always` together with `character` (L6): an always-on clip entry
  never sits on a layer's list, so the switch that turns a character LoRA on
  could never reach it.
- **The registry reaches the running process when it changes.** Today it is
  read once, in `VideoGenerate::new`, and a restart picks up an edit
  (`VideoConfig::loras`). Once built, it is re-read when its contents
  change: for pictures on each call (step 3), as `Tuning` is now, and for
  clips where the narrowed `video_generate` is built per run (step 2, §4);
  until step 2, clips keep today's read-once. An edit from the page
  applies from the next turn. A change to a clip entry moves the enum and
  re-pays the cached prefix once.
- **A registry file that does not read keeps the last good registry.**
  Today `Registry::load` answers an unreadable file with an empty registry
  and a note, which is safe while it is read once at startup. Re-read per
  call, the same answer would turn one transient read error into a call
  with no LoRAs, and collapse the `motion` enum and re-pay the prefix
  twice. So a re-read that fails keeps the registry the process last read,
  with a `doctor` finding, and only a file that read and parsed replaces
  it. At startup, with no last good registry, it is today's loud empty
  registry. The page's writes are atomic, a temp file renamed over the
  old one, so a read never sees half a write from the new second writer.
  This runs opposite to §4's damaged layer list, which reads as empty, and
  on purpose. The registry is the set of LoRAs that exist, and a transient
  read of it says nothing about the owner's wishes. A layer list is the
  owner's choice, and a damaged one could otherwise widen into a character
  LoRA the owner left off.
- **The old file is read for one release.** `[video] loras` keeps working
  and is reported by `mecha doctor` as moved. Its entries become `model =
  "h3"` with no other change. L2 put the new one at `~/.mecha/loras.toml`.
  The two are never merged. When the new file exists it is the registry, and
  an old one still configured is ignored, with a `doctor` finding naming it,
  so an entry is never read twice, never half from each, and never
  shadowed by a name in the other. When only the old one exists, it is
  read. `mecha` never writes or moves either file: the owner moves the
  entries, and the page's first write creates the new file only after
  saying the old one will then stop being read.

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
| Cap | `MAX_PER_PICTURE`, set by M3; the settings route refuses a longer list | `MAX_PER_CLIP` (4), unchanged |

- **An absent chat layer is not a damaged one.** The chat layer has no
  layer above it, so absent cannot mean inherit. For pictures, absent means
  empty (L1). For clips, absent means every entry the registry offers,
  which is v1's behaviour, never empty: a fresh install, or a machine where
  step 2's migration has not run, keeps today's clips rather than refusing
  every pick. Damaged still reads as empty, with the finding, because
  damaged means a list was written and cannot be read.
- **"Inherit" and "none" are different states.** A persona that inherits
  follows the chat layer as it changes. A persona set to an empty list stays
  empty. `Tuning` already writes an explicit choice and leaves an unset one
  out, for the same reason. Only the LoRA route writes them (§6), so a save
  of the other picture settings can never turn one into the other.
- **The LoRA list is not a field of `Tuning`.** It sits beside it in the
  same stored state (`state.image.loras` for a persona, the chat settings
  file for the chat layer), with its own type, its own lenient read and its
  own route (§6). Two reasons. `ImageSettingsBody::tuning` builds a whole
  `Tuning` and `set_tuning` replaces it, so a field inside would be reset by
  every cfg save, and the natural `loras: None` in that literal compiles.
  And `Tuning::over` drops the whole chat layer when the persona's model
  differs from the chat's, because cfg and steps are tuned per checkpoint; a
  list of names is not, so it resolves on its own, and a persona whose only
  choice is turbo still inherits the chat's list. Whether an entry then
  applies on turbo is M2's answer, filtered at render time (§5). A damaged
  list reads as **empty**, not as inherit, and the page and `doctor` say it
  could not be read. Unknown is never clean, and a LoRA list is a set of
  things applied: inherit is the direction that applies more, and could
  apply a character-tied entry (L6) the owner left off this persona. The
  cost is one re-save. Unlike `Tuning::lenient`, whose damaged numbers
  inherit because a number has no such risk. A test covers a cfg save
  leaving an explicit empty list explicit.
- **A list can name an entry that no longer exists.** The registry is
  hand-edited (L2), so an entry can be deleted or renamed outside the page,
  and the layers' lists are not rewritten when that happens. For clips a
  dangling name just leaves the enum. For pictures, the result names it in
  a count ("one LoRA in settings is no longer in the registry"), never by
  name, beside the count of those that applied, and `doctor` names the
  layer and the name to the owner. Otherwise the applied count would read
  as correct. A character-tied entry whose library character was removed is
  refused at parse time, like any entry that does not read (§3).
- **The allowlist is enforced in `pick`, not only in the schema.**
  `Registry::pick` resolves a name against the whole registry, so once a
  layer narrows the enum, a name it did not offer would still load. `pick`
  takes the resolved layer's list and refuses a name outside it. The enum is
  the hint, `pick` is the gate, and a test names an off-list entry and
  expects the refusal: that is the case the enum cannot cover.
- **A refusal names only what the run was offered.** Today's refusal for an
  unknown name lists `offered()`, the whole registry but its `always`
  entries. After §3
  merges pictures and clips, that list would put every picture entry, every
  clip entry the layer left off, and every character-tied entry (L6) into
  the model's context, in the refusal for the gate that withheld them. So
  the names a refusal lists are the resolved layer's list, filtered to the
  clip's family and mode, and less any character-tied entry whose character
  is not in this clip's cast (L6). A test asserts an off-list name appears
  in no refusal.
- **The enum is built per run, from the resolved layer.** Today the schema
  is built once, in `VideoGenerate::new` from `prepare_tools`, and in
  `mecha serve` one instance serves every chat and persona. A run's tools are
  already chosen from that pool by name (a persona's `[tools] allow`). Step 2
  adds, at that point, a `video_generate` narrowed to the run's list, kept
  per distinct rendered `motion` property, never per list of names:
  `Registry::schema` renders each entry's `about` and its modes into the
  enum's description, so an edit that leaves the list alone must still reach
  the next turn. The same rendering always gives the same bytes. A change
  in settings applies from the next turn and re-pays the cached prefix once.
  Nothing changes per turn. The enum keeps registry file order, as
  `Registry::schema` renders it today, which is stable; sorting it would
  re-pay every existing prefix once for nothing.
- **Settings are written only by the owner's settings route**, as
  `set_tuning` is. No tool writes them, and the model never authors a
  registry entry. An `about` line is owner-written prompt text, so it is
  trusted the way a persona's own text is.
- **What a result says.** A picture result gives the number of LoRAs from
  settings, not their names, because the model has no say in them. When
  settings added trigger words to the prompt, the result says that too,
  without the words, so a picture that differs from what the model wrote
  has a recorded reason. A clip result keeps naming what the model picked,
  as it does today, so it can say what it chose.
- **Names are one namespace across both families.** The registry dedupes
  by name, so a picture entry and a clip entry cannot share one; the
  second is refused with that reason.

## 5. The picture graph

One `LoraLoaderModelOnly` per entry between the diffusion model's loader
(`imagegen::unet_loader`'s choice) and `KSampler`, in registry order. The
clip graph builds its chain the same way (`videogen::graph::lora_chain`). A trigger is added to the prompt when the
prompt does not already contain it, with the same rule as
`loras::with_triggers`. Two things are unknown, so the build waits on them:

- **M1. Does a LoRA patch Qwen-Image's int8 file?** Clips already chain
  `LoraLoaderModelOnly` onto H3's int8 ConvRot checkpoints through
  `UNETLoader`, so int8 patching through ComfyUI's own loader has working
  evidence. What is unmeasured is Qwen-Image's int8 file specifically. Test:
  one same-seed pair with and without one LoRA, plus render time and peak
  memory, against the bf16 file for the same pair. A `.gguf` file loads
  through `UnetLoaderGGUF`, a second quantization with its own patching path.
  Until that is measured the same way, LoRA entries do not apply to a
  picture whose loaded file is a `.gguf`, and the result says they were left
  out. The check is per picture, on the file the picture actually loads
  (`ImageConfig::model_file(sampling.model)`, which is the turbo file for a
  turbo picture), never on one config key.
- **M2. Do LoRAs trained on the base model work on picture turbo?** Turbo
  is a distillation, not the base. Test: the same pair on turbo. If they
  fail, entries do not apply to turbo pictures, and the result says which
  were left out.
- **M3. How many at once, on Qwen-Image?** Two strong action LoRAs broke a
  clip that each passed alone (2026-10-24), but that was H3, a different
  architecture, through a different loader path. The clip cap stays
  `MAX_PER_CLIP` (4) from that test, and needs nothing new. The picture cap
  is measured on pictures: one, two and three Qwen-Image LoRAs stacked on one
  scene, same seed, on the int8 file M1 cleared; the cap is the largest
  count that held. Pictures have no `always` entries,
  so the cap is the length of the resolved list, and the settings route
  refuses a list longer than it, so the cap never drops an entry at render
  time. (M1 and M2 can, and the result says so.)

## 6. The web

- **Settings → LoRAs**, shown when the image or video feature is on.
  `PANE_FEATURE` maps a pane to exactly one feature, so step 4 extends the
  map's value to a list, and this pane is its first caller (ARCHITECTURE
  §Features, *Adding a feature*). Three functions in
  `web/src/lib/features.js` read the map: `featureOf`, `opens` and
  `bounceFor`. Everything downstream takes one id: `isShown`, `refuses`,
  `banner` and `hiddenLine`, through `App.svelte`. Each of those does
  `rows.get(id)` and reads a miss as shown, not refusing, no banner and no
  line, so an array handed through would fail open in all four with no
  error. So only the readers learn lists, and each hands one id on.
  Ranking by which feature is on needs `rows`, which `featureOf` does not
  take: it is a pure function of the two maps, and `web/test/features.mjs`
  asserts against it with no rows fixture. So `featureOf(view, sub)` stays
  pure and returns the first listed id, and a new `featureFor(rows, view,
  sub)` returns the first listed feature that is on, or the first listed
  when none is. `App.svelte`'s `banner` and `hiddenLine` calls and `opens`
  move to `featureFor`, so `isShown` means **any on** and `banner` speaks
  for the feature the pane is showing under. `bounceFor` returns an id only when
  **all** listed features refuse, the first of them, so `hiddenLine` still
  says why. `opens` follows `featureFor`, and its second clause, which hands
  the raw map value to `isShown` for a pane in `OPENS_ANYWAY`, resolves it
  the same way first: `library/voices` is in both maps today, so the first
  such pane to take a list would otherwise fail open there. Otherwise an owner with video on and
  image off sees the tab and is bounced from it, the #654 shape. A web test
  covers both directions, and asserts that the pane's banner and the Home
  line survive a list value. One more site sits outside the map:
  `Settings.svelte` builds the settings index with hard-coded
  `isShown(features.rows, …)` calls, so the LoRAs entry there is shown on
  `image` or `video`, the way the same list already shows one entry on
  `personas` or `library`.
  The routes follow the same rule: `/api/loras` answers when either feature
  is on, and each entry's write is refused when its own family's feature is
  off. Each section shows only when its own feature is on: picture entries
  with `image`, clip entries with `video`.
  - One row per registry entry: name, family, the mode files for H3, about,
    strength, trigger, `always` (clips), the character an entry is tied to
    (L6), and checked or refused with the reason. A picture entry before
    step 3 shows as checked but not applied yet, since the image graph has
    no LoRA node until then.
  - **Add** lists the files no entry uses yet, labelled by server. The list
    comes from each server's `/object_info` (`lora_name`'s options), as
    `comfyui_unit` already asks a server what it has, so listing needs no
    path. `loras_dir` is for §2's key check only. The family is read from
    the file and shown; with `loras_dir` unset the list still shows every
    file, its family as unknown, and names the key as the reason, so an
    empty pane never hides the cause. The owner fills in name, about, strength and
    trigger, and for H3 says which mode the file is for. A second H3 file
    is attached to an existing entry the same way. ComfyUI lists files in
    subfolders of `models/loras` by their relative path, so `file`, `frames`
    and `references` accept one, under §3's lexical rule. The Add list
    never offers a file the parser would refuse.
  - Edit and remove. Removing an entry also removes it from every layer's
    list, and the page says how many layers held it.
- **Settings → Images** gains a LoRA list (the chat layer's picture LoRAs).
  **Settings → Video** is a new pane holding the chat layer's clip
  allowlist.
- **Each persona's picture settings** gain both lists, each with an
  "Inherit from chat" state.
- **Locking (L3).** Today the library lock is a per-entry browse filter:
  `serve/library.rs` leaves locked entries out of the list and answers 404
  for their portraits without a live unlock token, enforced at the route,
  not the page. Both tabs stay in the nav. L3 adds a pane-level scope that
  does not exist yet, and enforces it the same way. `GET /api/loras` and
  every write under it require the live unlock token. Without one, the route
  answers *locked* with no names, abouts or triggers, and the pane shows
  only that it is locked. Hiding the pane on the page alone would still ship
  the text to the browser, which `library.rs` refuses to do for the same
  reason. The layer lists in Images, Video and persona settings show their
  LoRA rows only under the same token. What the model is offered never
  depends on the lock (the library ruling of 2026-09-28, reaffirmed
  2026-10-13).

Server routes: `GET /api/loras` (entries, their check, the unused files),
`POST/PUT/DELETE /api/loras/{name}`, and each layer's LoRA lists through
routes of their own, `GET` and `PUT /api/loras/layer/{chat|persona/<name>}`,
both behind the same unlock token. The read needs its own route as much as
the write: `GET /api/settings/images` takes no token today, since the
chat's other picture settings are behind no lock, so the LoRA rows never
ride in its body. A locked page renders the Images settings as now, with
the LoRA list shown as locked. The lists live beside `Tuning` in the same stored
state, but they are never written by the existing picture-settings route.
That route replaces `Tuning` whole (`ImageSettingsBody::tuning`, then
`persona::set_tuning`), so an omitted field reads as unset. A cfg change
saved while the pane is locked, from a page that cannot hold the LoRA rows,
would otherwise reset a persona's explicit empty list to inherit. Each
write goes through the same parser as the file, so the page cannot write
an entry the file would refuse.

## 7. Build order

1. **Registry v2 in core.** The shared file, `model`, the key check,
   `[image] loras_dir` and `[video] loras_dir`, the old file read for one
   release, the clip path filtered by family. No change in behaviour for a
   registry that only holds H3 entries, once `[video] loras_dir` is set.
   Both keys need `ConfigLayer::expand_home` arms: today it never opens
   `[image]`, and its `[video]` arm covers only `guide_addendum` and
   `loras`. Without them, a hand-typed `~/…` is read literally and every
   entry for that server is refused. `every_path_field_a_layer_can_read_is_expanded`
   does not catch this, because it walks only the `*Layer` structs in
   `config.rs`. So step 1 extends `a_leading_tilde_in_a_config_path_is_the_home_directory`,
   which already covers `[video]`'s two paths for the same reason.
   `scripts/check-private.py`'s `corpus()` also reads the registry's
   `name`, `file` and `trigger` and refuses them in added text, as it already
   does for persona names. While the old file is still read, `corpus()`
   also reads it, at the path `[video] loras` names in the global config,
   since it is not at a fixed path.
2. **Layers for clips.** The allowlist on the chat and persona layers,
   enforced in `pick`, and the narrowed `video_generate` per run. This is
   plumbing through where a run's tools are chosen, not a change inside
   `loras.rs`. It also writes L1's migration: on the first start of the new
   build, a chat settings file with no clip list gets one naming every
   `h3` entry the registry offers (picture entries, which step 1 already
   allows, never seed a clip list), so a later entry starts off as L1
   rules. The write is the one irreversible step here, so it runs only
   from a clean read: the registry file read and parsed, `[video]
   loras_dir` set, and no `h3` entry refused for an unreadable or unfound
   file. A picture file's state never blocks it. Otherwise the list stays
   absent, which §4 reads as every entry offered, `doctor` says why, and a
   later start tries again. A degraded first start never bakes "off" into
   the list. mecha
   writes the chat settings file through the same code as the owner's
   settings route; L2's "never writes or moves" is about the registry
   files, not this. Results are unchanged.
3. **Pictures**, gated on M1–M3. The LoRA list beside `Tuning` (§4), the
   graph's chain, triggers, and the result's count. Entries tied to a character
   (L6) apply only with that character in the cast, for pictures here and
   for clips in step 2's `pick`.
4. **The web.** The LoRAs pane and the `PANE_FEATURE` list, the lists in
   Images, Video and persona settings, and the lock (L3). A list value in
   `PANE_FEATURE` fails `every_feature_the_web_app_names_is_a_registry_id`,
   which reads one id per line, so that test grows with the map rather than
   reading as a regression.
5. ~~**The real-person guard**~~ deferred (L5).

Each step is its own PR. The tests and docs use made-up names only. Until
step 1 puts the registry in `check-private.py`'s corpus, each push is also
checked by hand for names from the owner's registry.

## 8. Decisions

- **L1. What a new chat or persona starts with. Ruled 2026-10-26:**
  pictures start empty. Clips keep today's set: the chat layer is migrated
  to every entry v1 offers, so nothing the owner uses changes, and personas
  inherit. An entry added later starts off everywhere until it is turned on.
  An `always` clip entry is on no layer's list, so it rides every clip
  whatever a list says, and no layer can turn it off, even one set to an
  empty list. A LoRA meant per chat is an ordinary entry on the lists.
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
- **L6. LoRAs trained on one character. Ruled 2026-10-26: tied to the
  library entry, and off unless enabled.** Some LoRAs are trained on one
  person's likeness. On a layer's list, one would put that face on everyone
  in every picture. So an entry may name a library character
  (`character = "<name>"`, checked against the library like a cast name).
  It starts off everywhere and applies only where the owner has turned it on
  in a chat or persona layer, and even then only to pictures and clips with
  that character in the cast. For a picture, with the character absent,
  it is left out without a note: the owner picked the list, and the setting
  did not ask for it there. For a clip the model picks, the enum is built
  per run while the cast is per clip, and the model cannot see the tie. So
  a tied pick for a clip without its character is dropped **with a note**,
  as a wrong-mode pick is (2026-10-25: a model that could not see why its
  pick failed spent three calls on it). The note says it applies only with
  its own character in the clip, without naming the character. Location
  and outfit names do appear in `video_generate`'s schema today; character
  names never do, since the model types a cast name and is never handed a
  list of the owner's people. L6 keeps it that way in the tie, the note
  and the `about`. The one channel left is the entry's own `name`, which
  the enum carries, and an owner naming a likeness LoRA after the person is
  the first thing to expect. So the parser refuses a character-tied entry
  whose `name` contains that character's name, or any word of four or more
  letters from its display words: the same set `check-private.py`'s
  `corpus()` already reads from each persona. The
  enum's `about` for a tied entry says the same, in the same words. Off by
  default because the owner is still testing this kind of LoRA. A tied
  entry still counts against the cap of a picture it applies to.
