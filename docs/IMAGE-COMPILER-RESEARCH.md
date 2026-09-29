# Image compiler — research

**2026-09-28.** One question: *how should mecha build a persistent library of
characters, places and styles that a short narrative intent compiles against —
so that generation is faster, carries the owner's intent, and keeps the cast
and the world consistent across many images — and which of the possible
designs are fast, which are slow but better, and which survive a move from
ComfyUI to stable-diffusion.cpp?*

Inputs: the owner's draft spec (*MECA Image Compiler — Design Specification*,
v0.1, reviewed section by section in §7); `imagegen.rs` and the ComfyUI and
stable-diffusion.cpp checkouts on this box, read at ComfyUI `88ab4a06` and
sd.cpp `19bbbca`; the recorded `image_generate` calls in `~/.mecha/sessions/`;
two web passes on this date (Qwen-Image 2.1's consistency levers; the prior
art in products and research). Labels: **[official]** is the vendor's own
docs, card or code; **[measured]** is a number someone published from a run;
**[reported]** is a forum post, blog or issue; **[local]** was read or checked
on this machine today; **[E1]**–**[E3]**, **[E8]** and **[E9]** are this document's own
experiments, run on this box the same day (§8). Qwen-Image 2.1 was released
eight days before this document and its ecosystem is moving daily — re-check
§2 before building.

**The finding that shapes everything below: identity lives in the
reference, not in the words.** Measured here [E1]: a character drawn from
their stored description alone is a *different person* — ArcFace similarity
0.33 to the portrait, inside the 0.10–0.34 band two different characters
score — while the same scene pointing at the portrait scores 0.74, higher in
8 of 8 paired scenes. Qwen's own edit rewriter says two things about this in
its published system prompt, verified verbatim [official, local copy of
`Qwen/Qwen-Image-2.1-PE-I2I/system_prompt.txt`]; the first half of the first
held here and the second half did not:

> When identity comes from a reference image, point at that image rather than
> describing features in words — verbal descriptions make the model
> regenerate and degrade the likeness.

> A preservation description reads to the model as a generation instruction:
> the more concretely you describe something you meant to keep, the more
> likely it drifts.

Adding the description *beside* the pointer did not degrade the likeness:
it raised it slightly, 0.78 vs 0.74, in 8 of 8 pairs [E1]. So the draft's
principle 1 — *structured state is canonical; prompts are compiled
artifacts* — is exactly right, and what identity compiles *to* on Qwen is
**a pointer (`<image2>`), with the stored description as a short
companion** — never the description alone. The preservation rule was not
tested (no edits were run), so a preserve list still compiles to **a mask
and a validator check, plus one blanket clause**, on Qwen's word. On another
model (OpenAI's guidance is the opposite: restate the preserve list every
call) the adapter compiles them differently, which is the draft's
principle 10 earning its keep.

> **Addendum, 2026-09-29 (#408; HISTORY, 2026-09-29):** the preservation
> rule has now been tested on edits, and on this model it falls on
> OpenAI's side, not Qwen's word. On 12 fixed seeds of one picnic edit
> ("stand Maya up"): a caption of the scene stood her up 0 times; the bare
> instruction 8; the kept parts named ("Keep the watercolor style, the
> lake, willow tree, and red checkered blanket unchanged.") then the
> instruction, 12. The blanket form, "Keep <image1> unchanged except: …",
> gave 0 clean edits in 2 tries (1 near-copy, 1 partial). So the preserve
> list compiles to named parts restated in the edit prompt, not one blanket
> clause. The mask and the validator check are untested.

**Three more things the measurements found** (§8). A reference **fills in
whatever the scene leaves unsaid** — wardrobe, pose, expression — so "a
candid photograph of friends" came back as a line-up of the four portraits;
stating them per person released all of it, in every reference format tried
[E8]. **Every reference slot tends to become a person**: two unnamed
references of one character drew that character twice [E2], and binding a
face and a full-body reference as one person still failed once in four
[E9] — naming each person and stating the head count is what held. And the cost curve has a
cliff: **three full-size references are cheap, four double the time**, while
512² face crops keep identity at a fraction of the cost [E2, E3]. The storage
format that follows [E8]: a character is **an approved front portrait (the
canonical source), a tight face crop derived from it (what generation
uses), and a short description**.

---

## 1. What exists today

`image_generate` (`docs/ARCHITECTURE.md` §Image generation) is stateless:
prompt, negative prompt, a size from a closed set, an optional seed, and up
to four `reference_images` (workspace paths) in; one PNG out, into the run's
own workspace. The ComfyUI graph is fixed in code (`imagegen::comfy_graph`);
the model supplies typed values, never nodes. Measured on 2026-09-25: 42 s warm
at 25 steps, ~65–70 s at the shipped 40, ~15 GB peak.

What the recorded calls show [local, `~/.mecha/sessions/`, 20 calls, content
not reproduced here]:

- **The persistence problem is already visible.** One session re-described a
  single subject from scratch across 11 calls, each paraphrase slightly
  different. Every one of those paragraphs is a fresh chance to drift.
- **Edits have converged on a template** the model re-invents each time:
  *"Keep everything exactly the same as `<image1>` but …"*, with people named
  by position ("second from the left"). That is a compile rule living in the
  model's head.
- **No call has passed a seed.** Nothing generated so far is reproducible
  from its record.
- **Prompts run ~100 words.** Qwen's text-to-image rewriter targets one
  paragraph of ~400–500 words "whether the brief was three words or three
  hundred" [official, PE-T2I system prompt]. The gap is the part of the frame
  the model is currently leaving to chance.

There is no library, no manifest of what went into an image, and no check of
what came out except `image_view` on request.

## 2. What the model and the two backends can actually do

### Qwen-Image 2.1

- **Architecture [official].** 7B generator, 32 single-stream DiT layers with
  block-causal attention; Qwen3-VL 8B text encoder; 64-channel RGBA VAE at
  16× compression. Not the MMDiT of Qwen-Image v1 / Edit-2509 / 2511 — which
  is why nothing built for those (LoRAs, adapters) loads on it.
- **References [official, local].** Up to 10 per the model card. ComfyUI's
  `TextEncodeQwenImage21` exposes 16 slots (`image_1..16`) and prepends
  `<image1><vision> <image2><vision> …` in slot order, so `<imageN>` in the
  prompt binds to slot N. The canvas follows the **first** reference's size —
  "any other size shifts the edit" (the node's tooltip) — so the canvas goes
  in slot 1. mecha caps at four (`MAX_REFERENCES`).
- **Reference cost [official + derivation].** Each reference is resized to
  about `resolution`² (default 1024) and becomes, at 16× compression,
  64 × 64 = **4,096 tokens — as many as the 1024² canvas itself**. The
  text-and-reference prefix is KV-cached across steps, on by default in
  ComfyUI (`QwenImage21Cache` defaults to `auto` and is only needed to move
  or quantise the cache) [local]. sd.cpp documents the cache's size: ~2 GiB
  per 4,096-token prefix at f16, ~1 GiB at q8_0 [official, sd.cpp
  `docs/qwen_image_2.1.md`]. So four full-size references cost on the order
  of 8 GiB of cache on a pool whose floor mecha already guards at 16 GB.
  **Measured [E2]:** at 40 steps, 1024² references cost +13 s, +23 s and
  **+120 s** for one, two and four (70 s → 190 s); E3 reproduced the same
  cliff at 25 steps (73 s for three, 151 s for four). 512² face crops cost
  +2, +5, +9 and +22 s for one, two, four and **eight**, and held identity
  as well as full size in a four-person frame (0.77–0.91 vs 0.80–0.90). The
  cliff's mechanism is not established — the pool readings are shared with
  `llama-server` and too noisy to attribute — but its position is.
- **RGBA output [official].** Transparency is selected by the prompt
  (*"This is an RGBA image with transparency. … the background is
  transparent."*). This is what makes compose-then-harmonize (§4, tier C)
  possible without a separate matting model.
- **Official prompt rewriters [official].** Two fine-tuned Qwen3.5-VL 9B
  checkpoints, `PE-T2I` and `PE-I2I`, ~20 GB each in bf16, each with its own
  system prompt; the stock base model "does not reliably emit the answer
  JSON". Beyond the two rules quoted above, the edit prompt requires
  `<imageN>` tags whenever N ≥ 2, the role of every image stated ("which one
  is the canvas … which supply material"), and — for a group scene with no
  canvas — "all images serve as identity sources". It names the two edit
  failures: **leakage** (touching what was not named) and **under-editing**.
- **Known failure modes.**
  - *Same-seed trap* [reported, GH #9; independently measured here on
    2026-09-25]: editing at the seed that drew the reference returns a
    near-copy. mecha already forces a fresh seed on every edit — and a
    library that reuses its own outputs as references is exactly where this
    bites, so the rule must hold for compiled calls too.
    *Addendum, 2026-09-29 (#408):* a fresh seed is not enough. With a fresh
    seed, an edit whose prompt was a caption of the scene still came back a
    near-copy on 11 of 12 seeds. The seed only decides which way an
    ambiguous prompt tips, and the prompt form decides how ambiguous it is
    (the addendum under the summary above).
  - *Face preservation "still below expectations"* [reported, HF #11].
  - *Degradation with many mixed references* [measured, on Edit-2511, not
    2.1: DyRef, arXiv 2606.26947].
  - *VAE moiré on light skin at high resolution; GGUF banding* [reported,
    HF #12, GH #14]. Worth checking our Q4 against the int8 file already on
    disk (E4).
  - *Text encoder dominates edit time* [reported, GH #7: ~30 s encoder vs
    ~10 s DiT]. If true here, prompt length and reference count cost more
    than steps do.
- **No identity benchmark for 2.1 exists.** The nearest measured
  predecessor: Qwen-Image-Edit-2509 on ViStoryBench, CIDS cross 0.475 / self
  0.574 vs OmniGen2 0.548 / 0.647 [measured, arXiv 2505.24862].
- **Licence [reported].** Qwen Research License, non-commercial — a step back
  from Edit-2509/2511's Apache 2.0. Fine for a personal assistant; it matters
  if any of this ever feeds the factory.

### The identity levers, and whether each survives the move

| Lever | Works on Qwen-Image 2.1? | ComfyUI | sd.cpp | Survives a model change? |
|---|---|---|---|---|
| In-context references (`<imageN>`) | yes, native | `TextEncodeQwenImage21` [local] | `-r` / `ref_images` [local] | yes — every current edit model takes references |
| Character LoRA | yes; ai-toolkit (`qwen_image_2`) and DiffSynth train it; musubi-tuner does not yet [official] | `comfy/lora.py` maps 2.1's fused `img_mlp` keys; GGUF patches on load [local] | `lora` field, from a server-listed directory [local] | **no** — tied to the exact base weights; retrain per model |
| Mask inpaint / local edit | via crop-edit-paste (below) | generic nodes; not verified for 2.1 | `--mask` / `mask_image` [local] | yes if done as crop-edit-paste in mecha's code |
| Detect-then-repair (ADetailer) | yes, as crop-edit-paste | custom nodes | native, YOLOv8 [local] | yes |
| PuLID / InfiniteYou / UNO / USO | **no port** | FLUX only | PuLID FLUX only [local] | n/a |
| PhotoMaker / IP-Adapter | **no port** | SDXL / SD1.5 | SDXL / SD1.5 only [local] | n/a |
| StoryDiffusion / ConsiStory (shared attention) | **no port** | SDXL | — | n/a |
| Image-to-LoRA (i2L) | v1 base only [official] | — | — | n/a |

**Two levers are real on this model: references and LoRA.** Everything in the
adapter literature is for a different base, and building on it means a
rewrite at the next model change. Local repair is available on both backends
*if mecha does the masking itself* — crop the region, edit the crop with the
relevant references, paste it back through a feathered mask in code. That
keeps every pixel outside the mask byte-identical (the draft's principle 7,
structurally) and needs nothing from the backend but "edit this image".

### stable-diffusion.cpp is closer than it was on 2026-09-25

Its native `/sdcpp/v1` API [local, `examples/server/api.md`] has async jobs
with cancel, a capabilities endpoint listing available LoRAs, and structured
`ref_images`, `mask_image`, `lora` and `seed` fields — and it **refuses
`<lora:…>` tags inside prompt text** on every API, the same instinct as
mecha's fixed graph: what is loaded into the model is a structured field
checked against a list, never text a model wrote. `Request` was already
shaped after sd.cpp's API; a compiler that targets the contract in §6 needs
nothing from ComfyUI that sd.cpp lacks. What held the migration back was
speed and text rendering (the engine comparison in §Image generation), not features.

## 3. What the field has converged on

From the prior-art pass (sources in §11):

1. **A named entity invoked by tag, resolved against reference images.**
   `@name` (Runway, LTX Studio), slots (Midjourney), per-character prompt
   boxes (NovelAI). Almost no shipped product stores a structured text
   definition beside the image; research systems (DreamStory's "multimodal
   anchors", TheaterGen's prompt book) store both.
2. **Entity types kept apart**: character, location, prop, style. LTX
   Studio's Elements is the closest shipped analogue to this whole project —
   a storyboard generated from a script "automatically identifies characters,
   objects, and locations and pulls in matching Elements".
3. **A compile stage run by a language model**, emitting per-shot records:
   who is present, a caption each, a region, the background, the camera
   (LMD, RPG, TheaterGen, MovieAgent, FIBO, and Qwen's own rewriter).
4. **References with explicit roles** ("identity from image 1, setting from
   image 2") — universal in OpenAI's, FLUX.2's and Qwen's guidance.
5. **Graduating an identity from reference to LoRA once the design is
   locked** (Krea, Leonardo, Scenario, Firefly).
6. **Compose-then-edit for more than two people.** Scenario composes then
   masks and refines each character; Dashtoon inpaints each with its own
   reference and an ArcFace embedding; TheaterGen "rehearses" each character
   alone first. Concept bleed between characters in one frame is the failure
   everyone names, and plain regional prompting "is highly likely to produce
   images with some subjects missing or having mixed identities" (MuDI).

Open disagreements worth knowing: JSON vs prose at the image model (FIBO
trains on JSON; BFL says FLUX.2 reads both equally; Qwen's rewriter emits
prose — so structure stays inside the compiler here); references vs LoRAs;
and how many characters per frame is realistic (claims run 3–22, benchmarks
exist because nobody has measured the curve). Vendor mechanisms churn —
Midjourney retired Omni Reference for an Edit Model within about a year —
which is the argument for a model-neutral library with model-specific
compilation.

## 4. The solutions, from fastest to best

Four tiers. Each is a superset of the one before in machinery, not in cost
per image: tier D is *faster* per image than B once the LoRA exists. Times
are arithmetic on the measured single pass (~45–70 s), not measurements.

### Tier A — the cast sheet (fast; build first)

The library holds entries; the model names them; code compiles.

- **The model writes only what varies**: which entries appear, pose, gaze,
  action, camera, light, time of day — a small `SceneSpec` with a closed
  schema, in the tool call.
- **Code writes what persists**: it resolves each named entry, chooses
  references by role (a portrait takes the face crop; a full-body shot takes
  the full-body sheet and the outfit; an edit takes the parent image as the
  canvas in slot 1), assigns `<imageN>` in that order, and assembles the
  prompt from a per-model template — identity as a pointer plus the entry's
  short description [E1], style and location as the entry's stored text
  pasted verbatim, never paraphrased.
- **Three rules the measurements added** (§8): identity references go at
  **512²** unless a full-size one is the canvas [E2]; **one reference per
  character per call, each appearing exactly once** (a stated total held
  duplicates off in E9, and E12 found "each appears exactly once" alone does
  too),
  because every slot tends to become a person [E2, E9];
  and **wardrobe, pose and expression are stated per person in every
  scene**, because a reference supplies its own when the text is silent
  [E3] — and stated, they land [E8].
- **One pass, up to four people** [E3]. Cost ≈ today's single call plus
  ~2 s per 512² reference [E2].
- **Every output carries a manifest** (§5): entries and their versions,
  references and slots, seed, model file, template version. It is the thing
  that makes the image reproducible and promotable.

What it fixes: the 11-paraphrase drift, the re-invented edit template, the
missing seed. What it does not: a failed likeness is only caught by the owner
looking. This is draft-spec phases 1 and 2 with the planner reduced to
"choose references by role" — which the evidence says is most of the value.

**The rewrite stage is a switch inside tier A**, with three settings:

| Setting | Cost | What it buys |
|---|---|---|
| R0 — template only | nothing | deterministic; short prompts; the frame left to chance |
| R1 — the chat model with Qwen's published PE system prompt | one LLM call on a seat (seconds to tens of seconds) | the 400–500-word frame; unmeasured whether our model follows the JSON contract |
| R2 — the PE-T2I / PE-I2I checkpoints as a router preset | a model switch on :8080 (evicts the chat model), ~20 GB each bf16 | the rewriter the model was trained against |

The rewriter reads the library's text and the scene, and the compiler hands
it the `<imageN>` bindings, their roles and each entry's short description
— the pointer carries identity; the description rides beside it, which E1
measured as harmless-to-helpful, not the degradation the PE prompt warns
of. Whether R1 is worth its seconds is still open:
the arm was built (E1b) and dropped from the 2026-09-28 run to save time.

### Tier B — checked (2–3× time; the owner looks less)

Tier A plus measurement, and a bounded retry.

- **Identity is measured, not judged**: a face embedding (ArcFace-family)
  against the character's canonical references, reported as ViStoryBench
  does — *cross* (vs the reference), *self* (vs the character's other
  approved images), and *copy-paste* (too close to a reference is a failure,
  not a success: a copy-paste baseline scores 0.929 cross). Thresholds are
  calibrated on the library's own same-character vs different-character
  pairs; there is no standard number (DeepFace's default is ≈0.32
  similarity, which is a verification threshold, not a likeness bar).
- **Counts are detected, not judged**: "exactly one coffee cup" goes to an
  open-vocabulary detector, because counting is a known weakness of VLM
  judges and a detector's box is also the mask for a repair.
- **Relationships go to the vision model**: gaze, holding, handing — the
  things only a VLM can read — each requirement answered separately against
  the `SceneSpec`, with its answer treated as evidence, not a score (§7,
  draft §14).
- **Retry by the cheapest fix**: an identity failure re-samples at a fresh
  seed (no drift, no compounding); a relationship failure becomes one edit;
  a count failure becomes a crop-edit-paste on the detector's box. Bounded
  (the draft's three), and on exhaustion the best candidate is kept with its
  failures listed.

Cost: each retry is a full pass; the checks themselves are seconds (the
embedding and detector are small; the VLM call takes a seat on
`llama-server`, `permit.rs`'s domain). This is draft phases 3 and 4.

### Tier C — composed (N + 2 passes or more; multi-character quality)

For two or more characters, where concept bleed is the named failure.

1. **The location plate**: the approved scene or the location's canonical
   view, as the canvas.
2. **Each character alone**, posed as the shot needs, rendered as an RGBA
   cutout against their own references — TheaterGen's "rehearsal", using
   2.1's native transparency.
3. **Composite in code** at the `SceneSpec`'s regions — deterministic, and
   the one place layout boxes (draft §9) earn their keep.
4. **One harmonize edit**, canvas in slot 1, characters' identity
   references after it: match lighting, contact shadows, occlusion.
5. **Per-character repair** by crop-edit-paste with that character's own
   references only, so no other identity is in the prefix to bleed from.

Cost ≈ (characters + 2) × a pass before any repair: two people is ~4–5
minutes. **E3 removed most of its premise**: native multi-reference held
four people in one pass — all present, counted, in the requested
left-to-right order, identity 0.72–0.92, no bleed (the highest similarity
of any face to a *wrong* cast member was 0.40 — at or just above the
different-character band's 0.34 ceiling, and far below every same-person
score, 0.72 or higher). C is kept for what E3 did not test: five or more
people, physical interaction between them, and pose freedom (below).

### Tier D — trained cast (hours per character once; then fastest per image)

A per-character LoRA, trained from the character's approved images.

- **The library already holds the training set**: tiers A–C's promoted
  images are the dataset (the field's typical 20–30 clean images). The LoRA
  is a *derived artifact*, rebuilt from the entry, never the entry itself —
  so a model change costs a retrain, not the character.
- **Cost, unmeasured here**: no one has published a Qwen-Image 2.1 LoRA run
  on a GB10. The nearest figures: 2.1 on a 12 GB RTX 4070, 31 images, 2000
  steps, 3.5 h, identity effect judged "weak" for anime, better for realistic
  subjects [reported]; Z-Image on a DGX Spark, 3000 steps in 4.5 h
  [reported]. Training holds the unified pool for hours — it competes with
  `llama-server` the way the first generation did on 2026-09-25, and needs a
  permit-style reservation, not a hope (E6).
- **Per image it is the cheapest identity lever**: a LoRA adds a load-time
  patch, not per-step tokens, freeing reference slots for location and
  props. The prefix cache should stay on with a LoRA applied: the model
  skips the cache only when "hooked" — a block replacement or a
  `post_input` / `single_block` / `attn1_patch` patch, because a cached step
  runs target rows only — and a LoRA is a weight patch, none of those [local,
  read from `comfy/ldm/qwen_image21/model.py`, not run].

### Side by side

| | A — cast sheet | B — checked | C — composed | D — trained cast |
|---|---|---|---|---|
| Per image | 1 pass | 1–4 passes | N + 2 passes, then repairs | 1 pass |
| Setup per character | a few approved images | + calibration pairs | + a pose-able full-body sheet | + hours of training |
| Identity | reference strength | reference, verified | reference, per character, verified | strongest (field consensus; unmeasured on 2.1) |
| 3+ characters in frame | held 4, no bleed, one seed [E3] | bleed caught, not prevented | for 5+ and interaction | LoRAs blend (Scenario, NovelAI warn) — combine with C |
| Needs from the backend | refs, seed | + edit | + edit, RGBA | + LoRA |
| On sd.cpp | yes | yes | yes | yes, retrained per base |
| Draft-spec phases | 1, 2 | 3, 4 | 4, 5 | not in the draft |

They compose rather than compete: the recommended path (§10) is A, then B,
with C switched on by character count and D by a character's use count.

## 5. mecha's invariants, applied

These are not preferences; each is a rule the codebase already enforces
elsewhere, and a compiler that breaks one reopens a closed bug.

- **The model never authors the workflow.** The draft's operation
  vocabulary (§11) becomes a **closed enum**, each variant compiled into a
  graph fixed in code — ComfyUI runs any graph it is handed, custom nodes
  included. `RELIGHT`, `OUTPAINT`, `UPSCALE` are variants to add when a fixed
  graph for each has been measured, not names to accept.
- **Names in, never paths.** The model names library entries; the tool
  resolves them. The library lives outside every run's workspace, so it is
  served by the tool the way `skill(name, file:)` serves level-3 files —
  containment proved against the entry's own directory — never through
  `fs_read` or a `reference_images` path.
- **The library is global only** (`~/.mecha/imagelib/`, or wherever the
  design lands), and nothing in a project's `mecha.toml` can add to it — the
  `[[trigger]]` and skills rule: a cloned repository must not bring a
  character into a trusted session.
- **A lane must not promote itself.** The model may *stage* a candidate
  entry or a candidate canonical image (the outbox's and the graph review
  queue's shape); promotion to approved or canonical is an owner act in a
  surface — the web chat's image card is the obvious one. The draft's
  "or a deliberately configured policy" (§18) is the part to strike: the two
  standing exceptions in `CLAUDE.md` are the owner's written rulings, not
  precedents.
- **Provenance gates what rides forward.** An entry's text rides into every
  future session that uses the character — as tool results, and into the
  rewriter's context. An entry drafted in a conversation holding third-party
  content is an injection path with a long half-life, the same shape as a
  learned rule. So an entry records its `Origin` from the transcript's
  recorded taint, and a non-clean draft cannot be promoted without the owner
  reading its text. Reference *pixels* reach only the loopback server, so
  they do not change the tool's capabilities.
- **Canonical bytes are content-addressed.** Promotion copies the bytes into
  the library under their hash; a canonical reference cannot change because a
  workspace file was edited, and every manifest names hashes, not paths.
  Versions are append-only records pointing at a parent (draft §20), the
  session store's discipline.
- **The cached prefix is sacred.** No cast list in the system prompt — the
  library changes on every promotion, and a toggling block re-pays the
  prefix. Lookup is a tool call (`image_library` list/search), or at most a
  sorted snapshot taken at session start, as skills are.
- **What the lookup declares follows from what it returns.** `skill`
  declares `Capabilities::default()` because a skill body is the owner's own
  words. `image_library` earns the same only if it returns **approved
  entries only**: every approved entry's text crossed the owner at
  promotion (and a non-clean draft's text was read, above). A staged
  candidate is model-written text that may come from a tainted
  conversation, so candidates are never returned to the model — they are
  shown only on the owner's surfaces. If the lookup ever returns
  candidates, the **tool** declares `Capabilities::untrusted_input` *and*
  the output carrying them is marked `.from_outside()` — the loop's rule is
  `caps.untrusted_input && out.external`, with `caps` read per tool, so the
  per-result marking alone taints nothing. That is the cost of returning
  candidates at all: the capability is static, so it would arm every
  lookup, approved-only included — which is why candidates never reach the
  model and `image_library` keeps `Capabilities::default()`.
- **Incognito may read the library and may not write it.** A promotion from
  an incognito chat is a trace by definition; whether the owner wants that
  door at all is a ruling for `INCOGNITO-DESIGN.md`, not this document.
- **Real people's likenesses: no rule in the compiler — the owner's
  ruling, 2026-09-28.** Proposed here: entries marked `invented | person`,
  a `person` entry recording consent, and a structural refusal of sexual or
  nude depictions of a `person` entry. Declined; the chat model's own
  guardrails are the control. The cost, stated: the router's presets include
  uncensored and abliterated chat models and the local image model has no
  filter of its own, so that control is whatever the selected chat model
  does.

## 6. The backend contract

What the compiler emits, per operation, is the only thing a backend must
accept. `Request` already has the first six fields; the last two are the
additions, and both exist on both backends:

| Field | ComfyUI today | sd.cpp `/sdcpp/v1` |
|---|---|---|
| prompt, negative | `TextEncodeQwenImage21` | `prompt`, `negative_prompt` |
| size or follow-canvas | `EmptyLatentImage` or the encoder's latent | `width`, `height` |
| steps, seed | `KSampler` | `sample_params`, `seed` |
| references, ordered | `images.image_N` via temp upload | `ref_images` |
| **mask** | crop-edit-paste in mecha (no backend mask needed) | same; `mask_image` if ever preferred |
| **LoRAs, by name** | `LoraLoaderModelOnly`, name from `object_info`'s list | `lora`, name from `capabilities` |

The rewriter, the checks, the compositing and the crop-paste all live in
mecha — so a migration is one adapter, and the library, the manifests and
the tiers do not notice.

## 7. The draft spec, section by section

| § | Draft | Verdict | Why |
|---|---|---|---|
| 2 | Ten principles | **keep all**, with 9's "configured policy" struck | 1, 2, 7, 9, 10 each match an invariant mecha already enforces |
| 4–7 | Library: characters, locations, objects, wardrobe, styles, scenes | **keep**; start with characters, locations, styles | the field's converged types; objects and wardrobe when a scene needs one |
| 5 | `CharacterSpec` identity fields | **keep, retarget** | on their own they draw a different person (0.33, E1); beside the reference pointer they help slightly (+0.04, 8/8) — so they compile as the pointer's companion, and serve the planner, validator and disambiguation |
| 8 | `SceneSpec` | **keep**; the model writes it, closed schema | this is "what varies", exactly the split in tier A |
| 9 | Scene graph | **defer** to tier C | relationships compile to sentences a Qwen3-VL encoder reads; boxes only matter when compositing |
| 10 | Reference roles and selection | **keep — the core** | the field's most consistent guidance, and Qwen's rewriter demands roles |
| 11 | Planner, operation vocabulary | **keep as a closed enum**; `INPAINT` as crop-edit-paste | fixed graphs; pixel preservation without backend masks |
| 12–13 | Model adapter, ComfyUI executor | **keep**; executor picks among fixed graphs | already `Request`'s shape |
| 14 | Vision validator with `confidence: 0.94` | **split three ways** | identity by embedding, counts by detector, relationships by VLM; a model's self-reported confidence is hearsay here, as appraisal labels are |
| 15 | Repair planner | **keep**, re-seed before edit for identity | an edit compounds drift; a fresh seed does not |
| 16 | Preserve constraints in the generation instruction | **adjust for Qwen** | one blanket clause by type/position/role; the list itself becomes the mask and the validator's checks. *2026-09-29: the blanket clause measured against: name the kept parts in the edit prompt (addendum under the summary)* |
| 17 | Bounded loop, best candidate | **keep**; add a wall-clock budget | three repairs at ~1 min each is the owner waiting |
| 18 | `generated → candidate → approved → canonical` | **keep**; promotion is an owner act | a lane must not promote itself |
| 19 | Provenance record | **keep**, add model-file hash, entry versions, template and rewriter versions, `Origin` | "ask the artifact" |
| 20 | Scene versioning and branching | **keep**, content-addressed and append-only | the session store's discipline |
| 21 | Ten agent actions | **collapse** to two or three tools | each tool costs cached-prefix bytes; `image_generate` gains `cast`/`location`/`scene` fields, plus a library lookup |
| 23 | Roadmap | **reorder** | phases 1+2 thin (tier A), then the measurements in §8, then 3+4 |
| 25 | Semantic and visual memory both | **keep** | the manifest and the canonical bytes are the two halves |

## 8. Measured on 2026-09-28, and what is still open

**Setup.** Four invented characters (Maya, John, Priya, Theo), each drawn
once from a stored description as a front-facing studio portrait — the
library's canonical reference. Every generation ran through a copy of
mecha's fixed graph (`comfy_graph`, Q4 GGUF, euler/simple, cfg 1) against
the loopback ComfyUI, 1024² unless stated, at seeds distinct from the
references'. Identity is ArcFace cosine (InsightFace `buffalo_l`) between the
largest detected face and the character's portrait. **The bar:** two
*different* characters' portraits score 0.10–0.34; a portrait against its own
512² face crop, 0.88–0.96. 59 images (E1–E3 41, E8 10, E9 8), no failures;
each run checked for 16 GB free before every job and would have
interrupted itself below 10 GB.

**E1 — pointer vs description** (2 characters × 4 scenes: a rainy diner, a
farmers market in full body, a library close-up, a beach at sunset; one seed
per scene, shared across conditions; 40 steps):

| Identity from | Similarity to the portrait | Consistency across the 4 scenes |
|---|---|---|
| the stored description only | **0.33** (min 0.10) | 0.31 / 0.22 |
| the reference only (pointer) | **0.74** (min 0.59) | 0.59 / 0.60 |
| reference + description | **0.78** (min 0.64) | 0.70 / 0.64 |

- **The description alone draws someone else**: inside the
  different-character band. Pointer beat it in 8/8 pairs, by +0.41 on
  average. Library text is not an identity mechanism on this model.
- **Adding the description to the pointer helped**: +0.04, 8/8 pairs, and
  more consistent across scenes. Qwen's "verbal descriptions degrade the
  likeness" did not hold on our quantisation; the composition was no more
  copied (thumbnail correlation with the portrait 0.20 vs 0.18).
- **A detail can be lost at the first step**: Maya's scar was in her
  description and did not render in her own portrait. A canonical reference
  is what was drawn, not what was meant; approving it is the owner's check.

**E2 — the price of a reference** (one prompt, one seed, 40 steps, warm;
exec time as ComfyUI reports it; two no-reference runs measured 70.0 and
68.6 s):

| References | 1024² (full portrait) | 512² (face crop) |
|---|---|---|
| 0 | 70.0 s | — |
| 1 | 83.1 s | 71.9 s |
| 2 | 93.3 s | 74.6 s |
| 4 | **189.7 s** | 79.2 s |
| 8 | not run (memory) | 92.2 s |

- **Four full-size references is a cliff**, not a slope, reproduced by E3 at
  25 steps (51 → 62 → 73 → **151 s** for one to four). Its cause is not
  established: the free-memory readings are shared with `llama-server` and
  too noisy to attribute.
- **Crops keep identity at a fraction of the cost**: in the four-person
  frame, 512² crops scored 0.77–0.91 against full-size 0.80–0.90.
- **Every reference slot tends to become a person.** The eight-reference run
  (each character's crop *and* portrait, unbound in the prompt) drew six
  people — Maya and Priya twice each. Two references of one character need
  binding in the prompt ("`<image1>` and `<image5>` are the same person"),
  which is untested; until then, one per character.

**E3 — characters per frame** (1–4 characters, full-size portraits in
left-to-right slot order, "a candid photograph of friends in a diner booth",
one seed, 25 steps, 1344×768):

| People | Found | Identity per character | Order | Highest similarity to a wrong cast member |
|---|---|---|---|---|
| 1 | 1 | 0.91 | ok | 0.32 |
| 2 | 2 | 0.91, 0.88 | ok | 0.40 |
| 3 | 3 | 0.92, 0.88, 0.89 | ok | 0.34 |
| 4 | 4 | 0.83, 0.86, 0.87, 0.72 | ok | 0.35 |

- **No bleed and no loss up to four people**, in order, in one pass. The
  concept-bleed failure the prior art centres on did not appear at this
  size; one seed, so this is an existence result, not a rate.
- **But every portrait arrived with its outfit, pose and stare.** "Candid
  friends" came back as a line-up of the four studio portraits — the same
  black or grey tops, frontal, neutral. The ~0.9 scores are partly this
  copying. 512² crops did not release it (the crop includes the collar, and
  no scene named clothes or expressions). E8 is the fix.

**E8 — releasing wardrobe, pose and expression, and the storage format.**
E3's four-person booth, now with each person's clothing, pose and
expression stated (a mustard-yellow raincoat, laughing, head tilted back;
a red plaid flannel shirt, turned toward her, smiling; an emerald-green
sweater, chin on hand, eyes closed; a white hoodie, looking out of the
window — colours chosen against the portraits' black and grey), across four
reference formats; 25 steps, 1344×768. Outfit is scored automatically from
the median colour under each face (calibrated on E3, where all four read
black/grey; plaid defeats it once); expression and pose were judged by eye.

| Reference format | Seeds | Identity (mean of 4) | Outfits | Time |
|---|---|---|---|---|
| A — full portrait, 1024² | 1 | 0.63 | 4/4 | 173 s |
| B — tight face crop (1.2× the face box, no collar), 512² | 2 | 0.62, 0.56 | 4/4, 4/4* | 53, 51 s |
| C — tight crop + the stored description | 2 | **0.67, 0.71** | 4/4, 4/4 | 52, 57 s |
| D — generated 2×2 angle-and-expression sheet, 1024² | 1 | 0.53 | 4/4 | 164 s |

\* one red plaid shirt read as "other" by the colour check; visibly red plaid.

- **Stating it releases it — in every format.** All six images put all four
  people in the requested clothes, in order; the laugh, the turn, the
  chin-on-hand and the look out of the window landed in every image, the
  closed eyes in five of six. E3's line-up was a prompt problem, not a
  storage problem.
- **Identity drops once the face moves** — 0.46–0.78 here against
  0.72–0.87 for the same four-person scene in E3, lowest for the head-back
  laugh — and every face stays above the
  different-character band. Part of E3's height was the copying; how much of
  the drop is the model and how much is ArcFace on a laughing, tilted face
  is not separable from these runs.
- **Crop + description is best, and as cheap as the bare crop**,
  consistent with E1. The
  angle sheet was the weakest generation reference (each face is a quarter
  of a 1024² panel) and hit the four-full-size-reference cliff; sheets are
  for browsing and for the validator's comparisons, not for generation.
- The black undershirt under two of the new outfits appears with tight crops
  that show no clothing at all, so it is the model's styling, not leakage.

**E9 — binding a face and a full body as one person.** Full-body
references for Maya and John with deliberately opposite builds (petite and
very slender; very tall and heavyset, with a large belly), in plain grey
clothes; then two friends standing on a sidewalk, head to toe, each in
stated clothing, the build never mentioned in the scene text. 512²
references, 25 steps, 1024².

| Condition | People drawn | Identity | Build carried over | Reference's grey clothing leaked |
|---|---|---|---|---|
| F — face crops only | 2 | 0.81, 0.88 | — (generic builds) | — |
| U — faces + bodies, prompt names only the faces | 2 | 0.83, 0.87 | not visibly | no |
| B1 — bound, references grouped by kind (2 seeds) | 2, 2 | 0.84/0.86, 0.71/0.79 | at most slightly (John) | yes, both |
| B2 — bound, grouped by person (2 seeds) | 2, **3** | 0.84/0.88, Maya twice | not visibly | yes, both |

- **A body reference does not earn its slot yet.** Build did not visibly
  transfer — partly this design's fault: the stated trench coat hid exactly
  the build it was testing — while the reference's clothing leaked in every
  bound image, and one bound image drew Maya twice, the copy dressed in the
  reference's grey. A solo full-body shot also carries no scale: each
  person fills their own frame, so it cannot say *tall* or *short*.
- **Naming each person and stating the head count is what prevented
  duplicates**: the unbound condition drew exactly two, where E2's
  unnamed eight did not.
- Face identity was unaffected by the extra references.

**E11 — a person who is no library character** (2026-09-29; three cast
portraits at 512², a waiter described in the scene, 25 steps, two seeds per
arm). With the compiled head count counting only the cast ("Exactly three
people"), the waiter was drawn in both images but moved to the background,
behind the bar rather than at the table, and one image grew a fifth figure.
With the waiter listed as "not from any image" and counted ("Exactly four
people … and one new person"), he stood at the table pouring coffee in both.
The cast's identity held either way (0.58–0.82), and the waiter's face
matched none of them (0.03–0.13): a new person each time, as an extra should
be. This is the evidence behind `image_generate`'s `extras`.

**E12 — a count that forbids no one** (2026-09-29; 25 steps, two seeds per
arm). A live run through the real model named Maya and John in `cast` and
wrote the waiter into the prose instead of `extras`; the compiled "Exactly
two people" erased him outright. The same shape with the cast's count but no
total — "each of the three people from the images appears exactly once;
anyone else the scene describes is a new person" — drew the waiter in both
images (in the background, as E11's uncounted arm did), and four cast with no
one else came back as exactly four faces, each the right person (0.63–0.80),
no duplicate. So the compiler states no total unless `extras` are given, and
counts them when they are. The one-reference case lost "Exactly one person
in the image" on the same reasoning with no arm of its own: it is carried by
inference from the three- and four-person arms, not measured, so a
duplicate there is not a regression E12 covers.

**Still open**, in the order they would change the design:

- **E9b — build, re-run with fitted clothing** so build is visible, and
  with height stated in text; decides whether any body reference joins the
  format before tier D's LoRAs are the answer.
- **Identity under expression**: more seeds per condition, and a second
  canonical view (a smiling three-quarter crop) as the *measurement*
  reference, to separate model drift from ArcFace's penalty on a laugh.
- **E1b — the R1 rewriter**: built, not run. Identity cost or gain of the
  PE-I2I rewrite, and whether our chat model keeps its JSON contract.
- **E4 — Q4 vs int8.** Banding and skin moiré on the same seeds; the int8
  file is already on disk.
- **E5 — calibration.** Same-character vs different-character distributions
  from the first real library entries; sets tier B's thresholds and the
  copy-paste ceiling. The bar above is four characters' worth.
- **E6 — a LoRA on the GB10.** One character, ai-toolkit, time and peak
  memory beside a resident `llama-server`; decides whether tier D is a
  nightly job or a weekend one.
- **E7 — the validator's agreement with the owner.** Twenty images, the
  VLM's per-requirement answers against the owner's; the ~80% agreement
  DreamBench++ reports is for GPT-4o, not our model.
- **The cliff's mechanism**, measured with `llama-server` idle, and whether
  `QwenImage21Cache` at int8 moves it.

Every generation checks memory headroom first (`min_available_mb`); a batch
of these beside a cargo link is how the box went down on 2026-09-25.

## 9. The surface: browsing, promoting, locking

The design needs a web surface, not only a tool: promotion is an owner act
(§5) made *somewhere*, and a canonical reference is only checked by being
looked at — Maya's scar was lost in her own portrait (E1). The manifests
every image carries make the pages nearly free. A sketch for the design doc
to settle (Svelte 5, the web surface's stack):

- **Library tab** — character, location and style cards: counts and a
  thumbnail, not lists (the home page's ruling of 2026-09-28).
- **Character page** — the canonical portrait, its derived crop, an angle
  sheet for looking at (E8), the description, version history, and the
  scenes the character appears in.
- **Scene page** — the image, its lineage (parent → edits), the
  `SceneSpec` that produced it, "make a variation", "branch from here".
- **Promotion** — "Save to library…" beside the web chat image card's
  existing Edit button; staged candidates as a row in `/queues`.

**Owner's rulings, 2026-09-28 — the lock.**

- **The lock is a browsing filter, not an access control.** One "Show
  locked" toggle, unlocked by a password for the browser session with an
  expiry, shows everything; off, locked characters, scenes and images are
  hidden. Generation is unaffected: a locked entry resolves by name in any
  chat, with no password asked mid-conversation.
- **Lock state is per item and always the owner's.** An image or scene made
  from a locked entry *starts* locked — the save dialog's lock box arrives
  checked, with the reason shown ("features a locked character") — and the
  owner may uncheck it, or unlock any single item later.

What follows for the build: the **server** withholds locked items from
browse pages and thumbnail requests while the toggle is off (a blurred
thumbnail still ships its bytes to the page and the cache); the password is
typed only in the lock dialog, never in chat, where it would land in a
transcript; and the lock changes nothing at generation.

## 10. Recommendation

1. **Build tier A**, with the measured rules: identity as a pointer plus
   the entry's short description; a tight 512² face crop as the reference;
   one reference per character per call; wardrobe, pose and expression
   stated per person in every scene; each person exactly once, with a
   total only when `extras` are counted [E11, E12]; up to four people
   natively. A character is stored as an approved front portrait,
   the crop derived from it, and the description — build and height in the
   description, not a body reference [E8, E9]. The library store,
   `SceneSpec`-lite in the tool call, the per-model template, manifests,
   and owner-only promotion from the web image card. None of it is Qwen- or
   ComfyUI-specific except one template.
2. **Design the surface** (§9) as its own `*-DESIGN.md`, from the owner's
   rulings recorded there.
3. **Raise `MAX_REFERENCES` only for crops**: eight 512² references cost
   less than two full-size ones [E2]; keep full-size references to three
   until the cliff is understood. The cap has to be computed from the
   decoded dimensions: today `imagegen.rs` bounds references by bytes
   (`MAX_REFERENCE_BYTES`) and a magic-number sniff, which cannot tell a
   512² crop from a 1024² portrait — and it should never come from a role
   the tool call declares.
4. **Tier B next** — embedding and detector checks are cheap and turn "the
   owner looks" into "the owner looks at failures"; E5 calibrates it from
   the first real entries.
5. **Tier C only for five or more people or physical interaction**, and
   **tier D for the few characters used often enough to amortise hours of
   training** — each switched on per case, not globally.
6. **Keep the contract in §6 the only backend surface**, so the sd.cpp
   migration stays one adapter whenever its speed and text rendering close
   the gap.

## 11. Sources

Qwen-Image 2.1: [model card](https://huggingface.co/Qwen/Qwen-Image-2.1) ·
[README](https://github.com/QwenLM/Qwen-Image-2.1) ·
[prompt_rewrite](https://github.com/QwenLM/Qwen-Image-2.1/tree/main/prompt_rewrite) ·
[PE-T2I](https://huggingface.co/Qwen/Qwen-Image-2.1-PE-T2I) ·
[PE-I2I](https://huggingface.co/Qwen/Qwen-Image-2.1-PE-I2I) ·
[same-seed issue #9](https://github.com/QwenLM/Qwen-Image-2.1/issues/9) ·
[face preservation HF #11](https://huggingface.co/Qwen/Qwen-Image-2.1/discussions/11) ·
[VAE moiré HF #12](https://huggingface.co/Qwen/Qwen-Image-2.1/discussions/12) ·
[GGUF banding #14](https://github.com/QwenLM/Qwen-Image-2.1/issues/14) ·
[encoder time #7](https://github.com/QwenLM/Qwen-Image-2.1/issues/7) ·
[Comfy tutorial](https://docs.comfy.org/tutorials/image/qwen/qwen-image-2-1) ·
[Edit-2509 card](https://huggingface.co/Qwen/Qwen-Image-Edit-2509) ·
[Edit-2511 card](https://huggingface.co/Qwen/Qwen-Image-Edit-2511) ·
[licence review](https://blog.buildfastwithai.com/qwen-image-2-1-review).

Training and adapters: [ai-toolkit on DGX Spark](https://forums.developer.nvidia.com/t/ostris-ai-toolkit-on-dgx-spark/355277) ·
[DiffSynth 2.1 docs](https://diffsynth-studio-doc.readthedocs.io/en/latest/Model_Details/Qwen-Image-2.1.html) ·
[musubi-tuner Qwen docs](https://github.com/kohya-ss/musubi-tuner/blob/main/docs/qwen_image.md) ·
[2.1 LoRA run on a 4070](https://note.com/sepiablue/n/n9372886b9e5d?hl=en) ·
[ComfyUI-GGUF](https://github.com/city96/ComfyUI-GGUF) ·
[Qwen-Image-i2L](https://huggingface.co/DiffSynth-Studio/Qwen-Image-i2L) ·
[InfiniteYou](https://arxiv.org/abs/2503.16418) · [USO](https://arxiv.org/abs/2508.18966) ·
[DyRef / OmniRef-Bench](https://arxiv.org/abs/2606.26947).

Measurement: [ViStoryBench](https://arxiv.org/abs/2505.24862)
([code](https://github.com/ViStoryBench/vistorybench)) ·
[DreamBench++](https://arxiv.org/abs/2406.16855) ·
[MultiHuman-Testbench](https://arxiv.org/abs/2506.20879) ·
[deepface thresholds](https://github.com/serengil/deepface/issues/359).

Products: [Midjourney V8 Edit Model](https://updates.midjourney.com/edit-model-for-v8/) ·
[NovelAI multiple characters](https://docs.novelai.net/en/image/multiplecharacters/) ·
[NovelAI precise reference](https://docs.novelai.net/en/image/precisereference/) ·
[Runway Gen-4 References](https://replicate.com/runwayml/gen4-image) ·
[LTX storyboard + Elements](https://ltx.studio/blog/ltx-storyboard-generator-update) ·
[Leonardo image guidance](https://intercom.help/leonardo-ai/en/articles/8497988-image-guidance) ·
[Scenario multi-character](https://help.scenario.com/articles/8459982289-generate-multi-character-scenes) ·
[Krea character design](https://www.krea.ai/blog/character-design-with-krea-2) ·
[OpenAI image prompting guide](https://developers.openai.com/cookbook/examples/multimodal/image-gen-models-prompting-guide) ·
[Nano Banana Pro](https://blog.google/innovation-and-ai/products/nano-banana-pro/) ·
[Dashtoon ID inpainting](https://insiders.dashtoon.com/a-road-towards-tuning-free-id-consistent-character-inpainting/) ·
[FLUX.2 prompting](https://docs.bfl.ml/guides/prompting_guide_flux2) ·
[Bria FIBO](https://github.com/Bria-AI/FIBO).

Research pipelines: [LMD](https://arxiv.org/abs/2305.13655) ·
[LayoutGPT](https://arxiv.org/abs/2305.15393) · [RPG](https://arxiv.org/abs/2401.11708) ·
[TheaterGen](https://arxiv.org/abs/2404.18919) · [AutoStudio](https://arxiv.org/abs/2406.01388) ·
[DreamStory](https://arxiv.org/abs/2407.12899) · [StoryDiffusion](https://arxiv.org/abs/2405.01434) ·
[Story-Iter](https://arxiv.org/abs/2410.06244) · [Anim-Director](https://arxiv.org/abs/2408.09787) ·
[MovieAgent](https://arxiv.org/abs/2503.07314) · [DiffSensei](https://arxiv.org/abs/2412.07589) ·
[CANVAS](https://arxiv.org/abs/2604.13452) · [Bounded Attention](https://arxiv.org/abs/2403.16990) ·
[MuDI](https://arxiv.org/abs/2404.04243).

Local: `mecha-core/src/imagegen.rs` (`comfy_graph`, `Request`,
`MAX_REFERENCES`); ComfyUI `comfy_extras/nodes_qwen.py`
(`TextEncodeQwenImage21`, `QwenImage21Cache`), `comfy/lora.py`,
`comfy/ldm/qwen_image21/model.py`; stable-diffusion.cpp
`docs/qwen_image_2.1.md`, `docs/lora.md`, `docs/adetailer.md`,
`docs/pulid.md`, `docs/photo_maker.md`, `docs/ip_adapter.md`,
`examples/server/api.md`.
