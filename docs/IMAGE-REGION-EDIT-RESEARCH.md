# Region-targeted image edits — research

**2026-09-29, measured 2026-09-30.** One question: *can Qwen-Image 2.1 be told where to edit — a
painted area, a box, a mask — and if so, which way of telling it keeps the
rest of the picture, lands the change, and costs least, so the web chat's
Edit button can become a modal where the owner paints or boxes the part to
change?*

It follows #408 (HISTORY, 2026-09-29), which found that an edit comes back
unchanged when its prompt describes the scene, and that naming the parts to
keep fixes that. Naming them in words is a request. A region can make it a
guarantee.

## 1. What the model supports

- **Qwen-Image 2.1 accepts local edits "via circles, painted annotations, or
  separate masks"** [official, README]. Its showcase is "circle-guided
  multi-region editing: remove watch, change hair color, replace clothing".
  The README documents no prompt format for any of the three, and no
  coordinates in text.
- **These are location hints, not boundaries.** A walkthrough of 2.1 in
  ComfyUI: "This works very differently from inpainting. Inpainting has a
  mechanism that prevents edits outside the mask, while this is only a
  location guide. The edit may therefore spill outside the specified area"
  [reported, nomadoor].
- **Qwen's edit models have long followed boxes drawn on the picture.** The
  Qwen-Image-Edit release used drawn boxes to mark what to correct [official,
  Qwen blog].
- **Masks are an extra reference, not a model input.** 2.1's encoder
  (`TextEncodeQwenImage21`) takes a prompt and up to sixteen images; it has
  no mask input [local, `comfy_extras/nodes_qwen.py`]. A mask reaches the
  model only as a picture (`<image2>`) or not at all.
- **A known failure: generated content does not fit the mask.** The model
  "intends" an object bigger than the masked area, and the result is clipped
  at the mask's edge; no workaround is recorded [reported, Qwen-Image #137].

## 2. What this box can run

Every node below is in the installed ComfyUI, core or `comfy_extras`, with no
custom node [local, `object_info`]:

- `SetLatentNoiseMask (samples, mask)`: the sampler resamples only where the
  mask is white, holding the rest to the source latent at every step.
- `VAEEncode (pixels, vae)`: the source as the starting latent, where an
  edit today starts from an empty one sized to the reference.
- `ImageToMask`, `GrowMask`, `ImageCompositeMasked`, `DifferentialDiffusion`,
  `InpaintModelConditioning`.

A published 2.1 inpainting workflow is exactly the 2.1 encoder, a
VAE-encoded source, `SetLatentNoiseMask` on a grown and feathered mask, and a
composite in pixel space afterwards, "guaranteeing everything outside the
mask is bit-identical" [reported, RunComfy]. LanPaint 2.2 adds a sampler for
the same job on 2.1's edit model, at 2–10 extra reasoning steps per
denoising step [reported, comfyui-wiki]; it is a custom node and was not
tried.

## 3. Four ways to target a region

| | How | Outside the region | Cost to build |
|---|---|---|---|
| **A. Mark the picture** | a red box drawn on the picture; "inside the red box: …" | a hint; may spill (the mark itself never survived, §4) | none in the graph; the page draws the mark |
| **B. Mask as a second image** | the original as `<image1>`, a black-and-white mask as `<image2>` | a hint | none in the graph; one more reference |
| **C. Latent noise mask** | the encoded original as the canvas, resampled only under the mask, then composited in code | identical by construction | a second fixed graph, and a composite in `imagegen` |
| **D. Crop, edit, paste** | mecha crops the region plus a margin, edits the crop at full size, pastes it back through a feathered mask | identical by construction outside the crop, not intact: its paste can damage what the crop's edge crosses (§4) | code only; any backend. `IMAGE-COMPILER-RESEARCH.md` §2 and §6 already recommend it |

A and B only say where. C and D are the two that keep the rest of the
picture. They differ in what the model sees: C sees the whole picture and
redraws part of it, while D sees only the crop, so a change that depends on
the rest of the scene (lighting, where someone stands relative to someone
else) is out of its reach.

## 4. The measurement

**Design.** One picture: the fictional picnic from #408, 1344×768, the Q4
GGUF with the w4a8 encoder, 40 steps, cfg 1. Three edits, each with a box:

- **T1, swap an object:** the pitcher → "a small wicker basket full of
  lemons", box `(340, 490, 570, 720)`.
- **T2, a pose, where the man must not move:** "have the woman stand up",
  box `(0, 0, 650, 768)`, her side of the frame.
- **T3, a detail:** "make the man's white shirt blue and white striped", box
  `(650, 220, 1260, 740)`.

Each edit goes through five arms on the same four seeds: P, a plain edit,
plus A–D above, for 60 images. Every prompt names the kept parts and then
the change, as #408 found works. The T2 prompt was "Keep the watercolour
style, the background, the blanket and the man unchanged. Have the woman
stand up at the edge of the blanket, holding the glass pitcher." That is
#408's form, but not #408's prompt: #408 named her ("Have Maya stand up"),
kept the lake, the tree and the blanket rather than the man, and ran other
seeds. So P here is this run's baseline, not #408's measured one (next
paragraph). For C and D the mask is the box grown by 24 px
and feathered by 16 px, and the composite runs in Python, as it would in
mecha.

**What is measured:**
- whether the change landed, judged by eye;
- how far the outside moved: the mean absolute pixel difference from the
  original, outside the box plus its feather (0 means untouched);
- for A, whether the red box survived into the result;
- seams, judged by eye;
- the seconds per image.

**Results** (2026-09-30, 60 of 60 images, 0 errors; each image also judged
by eye):

| | Swap (T1) | Pose (T2) | Detail (T3) | Outside moved, mean (T1 / T2 / T3) | Seconds |
|---|---|---|---|---|---|
| **P** plain | 4/4 | 2 clear, 1 partial, 1 left sitting; both clear ones redrew the man | 4/4 | 13.6 / 32.2 / 11.4 | 80 |
| **A** red box | 4/4 | 0 clear, 2 partial, 2 left sitting | 4/4 | 13.3 / 16.6 / 10.9 | 80 |
| **B** mask as `<image2>` | 4/4 | 1 clear, 1 partial, 2 left sitting | 4/4 | 11.9 / 14.6 / 9.8 | 91 |
| **C** noise mask + composite | 4/4 | 1 clear, 1 partial, 2 left sitting | 4/4 | 0 / 0 / 0 by construction* | 80 |
| **D** crop, edit, paste | 4/4, basket visibly smaller | **4/4 clear** | 4/4, stripes paler in 2 | 0 / 0 / 0 by construction* | 81 |

The outside figure is the mean absolute pixel difference on a 0–255 scale,
measured outside the box plus its feather. \*For C and D that area is
copied from the original, so the zeros check the composite rather than
measure the edit. D's damage lies inside the band the figure leaves out,
and is judged by eye below. The zeros are also this picture's size: 1344×768
is already the edit canvas the graph samples at, so the composite lined up
without a resize. A picture of another size is edited at its canvas, and
"identical" then holds at that size, not the file's (§5, and #429).

**P is not #408's baseline.** #408 stood Maya up in 12 of 12 seeds with
the named form. Here P managed 2 of 4 with "the woman" and a keep list that
names the man; where it did, it recomposed the scene, and its pose edits
moved the outside by up to 49.6. Against #408's rate, every arm here
under-edits the pose. So "C stood her up less often than P" is a comparison
within this run's wording, at n = 4, not a claim about C against the tested
form. It was not re-run with the named subject.

- **A and B are hints, and they keep nothing.** On the swap and the detail,
  their outside moved as much as a plain edit's, within 2 points. On the
  pose their drift was half of P's, but that is because they mostly failed
  to stand her up (P landed 2, A 0, B 1). Less change left less to spill,
  so a lower number there is evidence of failure, not of keeping anything.
  The red box never survived into a result (at most 0.5% of its outline
  stayed red).
- **C is seamless and exact; on the pose it did no better than this run's
  weak baseline.** Nothing outside
  the mask moved, and at the boundary the new pixels continue the old ones.
  In T2 the grass, the blanket and the man's arm run straight through. It
  landed every swap and every detail, but it stood the woman up less often
  than this run's plain edit did: 1 clear against 2, with the weaker wording
  above. The reference and the source latent still show her sitting.
- **D lands the pose, and leaves seams where its edge crosses something.**
  With the man cropped out, nothing anchored her, and all four seeds stood
  her up in full. But T2's crop edge ran through the man's forearm, and in
  every seed the feathered paste left it half transparent or pasted grass
  over the blanket. T3's edge ran through open background and came out
  clean. The swap's basket came out smaller in all four seeds: in a
  zoomed-in crop, the model has no sense of scale.
- **The cost is the same for all of them.** A mask is not a slower edit
  (C: 80 s). B's second reference costs about 10 s.

**What generalises, and what does not.** Every image is the one picnic
from #408, at four seeds. D's exact outside is by construction and holds
for any picture, since it pastes into the original. C's holds at the edit
canvas, the picture's own size for every picture the tool made (§4's size
note). The landing rates, D's scale and seams, and C's
under-editing of a pose are this scene's, and need a second scene before
they are general (HANDOFF lists "any scene but the one picnic" as untested
since #408).

**What this says.** For the local edits the modal will mostly carry (swap
this, recolour that, change a detail), C does what the owner asked for: the
change, and nothing else, at no extra cost. A pose or a move is a different
kind of edit. In these four seeds, C held the rest of the picture but
stood her up once where a plain edit did so twice. That is n = 4, and it
was the reason to measure C′, below. D makes the change but cannot blend it
when the region's edge crosses a person.

**C′, measured the same day** (2026-09-30, 17 images, 0 errors). C′ is C
with the region hidden from the reference, so the model is not shown the
pose it is meant to replace. It was tried three ways, on the pose edit and
the swap, at the same four seeds:

| | n | Pose (T2) | Still her? | Swap (T1) |
|---|---|---|---|---|
| **grey fill** | 1 (T2, seed 1) | the fill copied into the result, a grey wall with a faint figure | n/a | not run |
| **blur** | 8 | stood up in 3 of 4 | **no: a different woman in 4 of 4**, and seed 3 copied the blur into the result | 4 of 4 |
| **grey + the original as `<image2>`** | 8 | 1 clear, 1 partial, 2 left sitting; seed 3 copied its grey fill across the frame | yes, 4 of 4 | 4 of 4 |

Grey fill was stopped after its first image, which was conclusive. The
grey-plus-original variant also copied its grey fill across the frame in
seed 3. The model treats its reference as the picture, so a placeholder in
it is content to reproduce, not a blank to fill. That is the same lesson
#408 found for a caption. Hiding the region takes her identity along with
her pose, and giving the identity back with a second reference brings the
pose back too. **C′ does not fix the pose, and it adds a catastrophic
failure on large regions.** It was dropped.

Not yet run: **D's edge placed in background,** by growing the crop until
its border clears people, which is hard to do from a box the owner drew;
and **D then a thin C pass**, C over just the seam band of D's result, to
blend what D's paste leaves (two generations, about 160 s).

## 5. The edit modal

The owner's direction (2026-09-29): the Edit button opens a modal where
areas can be painted and annotated. What the measurement has to settle
first is which graph a painted area drives. What the modal needs is the same
whichever graph wins:

- **The picture, full width, with two tools:** a box (drag) and a brush
  (paint). On a phone a box is one gesture and a brush is fiddly, so the box
  comes first, both on pointer events. "Clear" and "whole picture" keep the
  plain edit one tap away.
- **Annotations are regions with their own instruction.** Each painted
  region carries a short note ("make this blue"), numbered on the canvas.
  That maps onto Qwen's multi-region showcase: numbered marks for the hint
  (A), and the union of the regions as the mask that keeps the rest (C or
  D).
- **The page renders the mask; the model passes its path.** A
  black-and-white PNG at the picture's own size, uploaded into the chat's
  jail. `image_generate` gains a `mask` path that it reads through
  `ToolCtx::resolve`, like `reference_images`. The core then sizes picture
  and mask together to the edit canvas, and composites at that size (as
  built in #429). The model never writes coordinates, which keeps the graph
  fixed in code and the tool typed values. It is still a path the model
  passes, though, not a registered object. `ToolCtx::resolve` proves the
  file is in the jail, not that the page made it, and the model could name
  or write another mask. So the region is the owner's request, carried by
  the model, not a guarantee (§6).
- **A mask is not an image for the model to look at.** Web uploads reach the
  model as pixels today (#366), and an attached image arms `private_data`.
  A mask would spend context and taint the chat for nothing, so its upload
  must be a kind the page stores without attaching.
- **The note is the prompt, and the kept parts come free.** With the mask
  the owner drew, what stays is enforced by the composite (C, D) rather than
  listed in words.

## 6. Open, for the owner

Set, or narrowed, by the results:
- **Which graph backs a painted area. The owner chose C on 2026-09-30, and
  it is built in #429.** C for the local edits (measured above). C′ was measured and dropped. A pose or a move has no region-graph
  answer yet, and two alternatives outside it: a plain edit in #408's form,
  which moved people but redrew the rest; and a library redraw, which
  #408's probes found moves people reliably. D followed by a thin C pass is
  the untested candidate inside a region.
- **What C costs if the backend changes.** `Request` is shaped like
  stable-diffusion.cpp's API so that a second backend can meet it
  (`ARCHITECTURE.md`, image generation). D needs nothing from any backend
  but "edit this image". C needs the backend to resample under a mask. On
  ComfyUI that is a second fixed graph. On stable-diffusion.cpp it would be
  its native `mask_image` field (`IMAGE-COMPILER-RESEARCH.md` §2), unverified
  in combination with 2.1's reference-image edits. The composite is mecha's
  code on either. Choosing C now means a mask field in `Request` and a
  measurement owed at any backend move. D avoids both, at the cost of its
  seams.
- **A mask the page registers, not a path the model passes.** As built in
  #429, the mask is a workspace path, so an injected instruction could name
  another mask while the modal shows a small painted area. The page could
  register the mask it uploaded (for example, the descriptor-relative I/O in
  `workspace_files.rs`), and `image_generate` accept only a registered mask
  for that picture. Not built. The live run found the model passes the path
  faithfully once `size` is not refused. (The first live run's tool
  refused a `size` passed beside a mask, and the model retried without the
  mask; #429 now sets the size aside instead.) Said in CLAUDE.md's terms,
  a model-chosen or all-white mask is the silently-degrading guard: C
  falls back to the unguarded plain edit while the modal still shows a
  small painted area.
- **A and B are not worth building on their own.** A mark in the picture
  could still ride along with C as a hint for multi-region notes, but nothing
  here shows it helps.
- Whether a region without a note falls back to the chat message as the
  instruction.

Set later by §7.6–7.7 (2026-10-09):
- Whether annotations on several regions run as one edit or one per region:
  one edit, with each region's words in a colour legend (§7.7).

## 7. Several regions, each with its own instruction (2026-10-09)

The owner's question (2026-10-09): today's modal paints one white region with
one instruction. Can the owner draw several regions in different colours,
each carrying its own instruction, and what should that interface look like?
§6 left open "whether annotations on several regions run as one edit or one
per region". This section answers what the sources say and proposes how to
measure the rest. **§7.1–7.5 were written before any run; §7.6 and §7.7
report the results.**

### 7.1 What the model supports

- **Qwen-Image 2.1 does this in one pass, and says so** [official, README and
  HF card].
  - The README says local edits can be specified "via circles, painted
    annotations, or separate masks".
  - Its "Local Editing" figure is captioned "Circle-guided multi-region
    editing: remove watch, change hair color, replace clothing".
  - The input photo carries thin freehand outlines in red, blue and green,
    drawn on the photo itself. The output has all three edits and no
    outlines.
  - Its prompt is quoted, by a third-party guide rather than by Qwen, as
    "Remove the metal watch in the blue circle, change the hair in the red
    circle to black, and replace the area in the green circle with gray
    short-sleeved linen pajamas".
- **Regions are named by colour.** Qwen's Pro guide does the same ("Replace
  the object inside the red circle … Remove the red circle") and asks for "a
  color that appears nowhere else in the frame" [reported, a third-party
  host's guide to 2.1 Pro].
  - Coordinates in the prompt do not work: one user reported they "could not
    edit the region" [reported, Qwen-Image #289].
  - Every source asks the model to remove the marks.
  - No limit on the number of regions is published. The demo uses three, and
    Krita's region system recommends five or fewer [official, Krita's docs].
- **Marks are hints, not walls** (§1 holds). No source compares marks
  against true inpainting. Every source agrees that only a composite
  guarantees the rest of the picture. The same agreement is what chose C in
  §6.
- **Marks the output is never built from.** A community node for
  Edit-2509/2511 (`comfyui_qwen_edit_pixel_perfect`, read from its source)
  does the following:
  - It tints the mask red only on the small copy the vision encoder sees.
  - It VAE-encodes the reference from the clean original.
  - It tells the model the red region is the only area to change, then
    composites byte-exactly.
  - So the mark cannot bleed into the result. Whether 2.1's encoder
    (`TextEncodeQwenImage21`: prompt plus a list of images, no mask input,
    §1) can be given a marked copy as one image and the clean original as
    another, and still edit the clean one, is untested.

### 7.2 How other tools put it in front of a person

| Tool | Pattern |
|---|---|
| Gemini (Nano Banana), Photoshop web | circles, arrows and written words on an annotation layer; prompt "follow the red prompts in the image, remove them after" |
| ChatGPT images | one brush selection, one chat instruction; "edits may extend beyond the area you selected" |
| Krita AI | a layer per region, each with its own prompt, plus a root prompt for the whole scene added to each |

Krita's split is the useful one here: per-region notes plus one note for the
whole picture.

### 7.3 Three ways to run several regions

| | How | Outside every region | Inside each region | Cost |
|---|---|---|---|---|
| **M1. One pass, marks on the canvas** | the picture with each region's outline drawn on it in its colour as the canvas; the union of the regions as C's latent noise mask; the prompt from the legend, "In the red outline: …; in the blue outline: …; remove the outlines". The composite restores from the **clean** picture, not the marked canvas | identical, provided the composite restores from the clean picture (from the marked canvas it would restore every outline pixel outside the mask, by construction) | an outline the model leaves inside a region survives the composite, which keeps everything under the mask | one render |
| **M2. One pass, marks on a second image** | the clean picture as `<image1>` and canvas, a copy with the outlines as `<image2>`; the same union mask and legend, "…the coloured outlines in the second image mark…" | identical | no mark on the canvas, so none can survive; whether 2.1 follows a mark it sees only in a second image is the question | one render, one more reference |
| **M3. One pass per region** | C as built (#429), once per region with that region's mask and words, each composited onto the last | identical | exact per region; a later pass sees the earlier result | n renders. Each pass resamples only under its own region and composites the rest byte-exactly, so quality loss can accumulate only where two regions' grown masks overlap (about 21 px past each painted edge; adjacent regions do overlap). The grain one guide measured over six whole-image rounds does not apply. |

M3 needs no new graph and is the fallback that cannot confuse two regions.
M1 is the officially demonstrated form. M2 is M1 with the one failure M1
cannot rule out designed away.

### 7.4 The interface

- **Regions, not one mask.** A row of colour chips above the picture, at
  most four. Tapping a chip makes it the active region. Brush and box paint
  in that colour, and the eraser erases only the active region.
  - Painted regions show as **thin outlines** on the canvas, with a dim fill
    while drawing. The official demo uses outlines, and a fill hides what the
    model must see.
  - Each region gets a number badge where it was first drawn, so a phone user
    can tell regions apart without relying on colour alone.
- **A note per region, and one for the whole picture.** Under the picture,
  one line per region: its chip, its number, and a text field ("make this
  blue"). An optional "whole picture" line carries anything that is not a
  region (Krita's root prompt).
  - Send is enabled when every painted region has a note.
  - A region with no note is the thing §6 left open. Proposed: it may not be
    sent empty. The owner is asked, never guessed for.
- **Colours chosen against the picture.** The palette is **eight**
  saturated hues, and at load the page picks the four least present in the
  picture, so there is always a choice: Qwen asks for colours "that appear
  nowhere else in the frame". With four hues for four regions there would
  be nothing to pick.
- **What the page sends.** It sends one colour-indexed **region index** PNG
  at the picture's size: each region's pixels in its palette colour, and
  every unpainted pixel RGB black (not merely transparent, since
  `to_luma8()` drops alpha). It is uploaded into the jail like today's mask.
  The panel's turn then carries `regions: [{colour, words}]` beside it.
  - The server derives everything from that one file: the union mask for C,
    each region's own mask (M3), and the outline image (M1 and M2).
  - **It is an index, not a mask.** It is binarised per colour (a pixel of
    colour *k* is region *k*) before anything reaches `prepare_mask`, which
    takes luma and thresholds it at 16 after a resize. Saturated hues have
    very different luma (white 255, green ~182, red ~54, blue ~18), so a
    blue stroke sent through as-is would need about fifteen times white's
    coverage to register. A region could then be dropped silently, the
    silently-degrading guard of §6. Likewise the graph's `ImageToMask`
    reads the red channel only, so only the derived greyscale mask ever
    reaches it.
  - The legend is built in the harness from the typed `regions`, in fixed
    words, so the model never writes coordinates and the prompt is typed
    values as before.
  - One region in one colour is today's edit exactly.
- **Not changed:** the composite, the mask never attached as an image (§5),
  and the panel's harness draw (`dispatch_one`; IMAGE-DESIGN.md §5.3).
  §6's "a mask the page registers, not a path the model passes" applies
  unchanged, and more so with several regions.

### 7.5 The measurement owed

On this box, a3's way: paired seeds, n ≥ 4 per arm, judged by eye with the
outside difference computed.

- **Cases:** two regions with different, non-overlapping jobs (recolour a
  shirt, remove a cup); three regions, including one on a face (hair colour);
  and two adjacent regions (a jacket and the shirt under it) to catch
  swapped instructions.
- **Arms:** M1, M2, M3, and a control of one region with the instructions
  joined in words ("make the shirt blue and remove the cup").
- **Per region:** whether the change landed; whether a region did another's
  job; whether a mark survived (M1); identity on the face case; and
  seconds.

**Decision rule**, as n = 4 can read it: take M2 if it lands as often as M1
with no visible mark left in any edit; else M1 if no visible mark is left in
any of its edits; else M3, whose cost is known. (A rate such as "under 5%"
needs at least 20 edits per arm before one failure reads under it, and
three cases at n = 4 make 12.) Each graded image records the legend the
harness built, so a swap is told apart from both regions doing both jobs.
Until it is measured, a build can ship M3 behind the same interface, since
the interface does not depend on which arm wins.

### 7.6 First results: distinct targets (2026-10-09)

mecha-a3 ran the arms of §7.5 on the real C path: `prepare_mask`,
`SetLatentNoiseMask` and `composite_masked`, with paired seeds and n = 4.
The pictures were three made-up scenes:

- (a) a shirt recoloured, a cup removed;
- (b) hair, a sweater, and a plant removed;
- (c) a jacket and the shirt under it, which are adjacent.

The outline colours were the hues least present in each picture.

| | Landed | Marks left | Face (ArcFace, b) | Seconds per edit |
|---|---|---|---|---|
| control (one region, words joined) | 12/12 | 0 | .90–.92 | ~44 |
| M1 marks on the canvas | 12/12 | 1–40 px of cyan in b, not visible | .88–.90 (its outline ran round the face) | ~44–48 |
| M2 marks on `<image2>` | 12/12 | 0 | .91–.93 | ~55–60 |
| M3 a pass per region | 12/12 | 0 | .90–.92 | ~88 (2 regions), ~130 (3) |

- **No region did another's job,** including on the adjacent pair (c).
- **Outside the regions, nothing moved in any of the 48 edits** (mean
  difference 0.000).
- **By the rule in §7.5, M2 wins:** it lands as often as M1 and leaves no
  marks, for about 12 s more than M1. It holds the face at least as well as
  the rest; M1 does not.
- The outside figure is a sanity check of the composite, which guarantees
  it, not a finding about the arms.

**This set is a ceiling, though, not a test of regions.** The control landed
everything too, because each instruction named a different kind of thing
that words alone can find. What regions exist for is the **same-class**
target, where only the region says which one:

- two people's shirts in different colours (the persona picture's usual
  ask);
- one of two identical cups;
- the left sleeve, not the right.

That set was approved by the owner the same day and is running. The choice
of arm waits on it.

### 7.7 Same-class targets: what regions are for (2026-10-09)

The set approved in §7.6, run the same way (n = 4, paired seeds). The masks
came from SAM 3 text prompts, two instances each, taken left and right. The
pictures were made-up scenes:

- (p) a woman and a man in identical grey t-shirts: hers red, his green;
- (q) two identical white cups: remove the left one;
- (r) a man in a grey sweater: the left sleeve red.

The control painted **every** candidate (both shirts, both cups, both
sleeves) and used the words as the owner would type them ("make her top red
and his shirt green", "remove the cup on the left"). Otherwise the mask
alone would answer "which one", and the control would test nothing.

| | p | q | r | Wrong target | Marks left | Seconds |
|---|---|---|---|---|---|---|
| control (one brush over all, words) | 3/4 (his stayed grey once) | 4/4 | 2/4 (both sleeves red twice) | **3/12** | none | ~64 |
| M1 marks on the canvas | 4/4 | 4/4 | 4/4 | 0/12 | 0–12 px, not visible | ~65 |
| M2 marks on `<image2>` | 4/4 | 4/4 | 4/4 | 0/12 | none | ~82–90 |
| M3 a pass per region | 4/4 | 4/4 | 4/4 | 0/12 | none | ~64 per region (~128 for p) |

- **Regions settle what words cannot.** With one painted area and words,
  the wrong target changed 3 times in 12. With a region per target it never
  did, in any arm.
- **Faces held** on (p) for every arm.
- The GPU was shared during this set, so every arm ran slower than in
  §7.6. The gaps between arms hold.
- **By the §7.5 rule M2 wins; M1 also passes.** Neither left a visible mark,
  and M1 is about 20 s cheaper per edit. The choice between them is the
  owner's.
- **A side finding, not about the arms:** removing a cup left a faint
  rectangular seam at the mask's edge, on the backsplash and the table, in
  every arm. That is the composite's feathered edge on a smooth background,
  a question for `prepare_mask`'s grow and feather, not for regions.

## Sources

- [QwenLM/Qwen-Image-2.1](https://github.com/QwenLM/Qwen-Image-2.1): "specify
  local edits via circles, painted annotations, or separate masks".
- [Qwen-Image-2.1 in ComfyUI](https://comfyui.nomadoor.net/en/basic-workflows/qwen-image-2-1/):
  circles, masks, and "only a location guide".
- [Qwen Image 2.1 Inpainting in ComfyUI](https://www.runcomfy.com/comfyui-workflows/qwen-image-2-1-inpainting-in-comfyui-no-offset-edits):
  the encoder, `SetLatentNoiseMask` and a composite.
- [LanPaint 2.2 for Qwen-Image 2.1](https://comfyui-wiki.com/en/news/2026-09-28-lanpaint-2-2-qwen-image-2-1).
- [Qwen-Image issue #137](https://github.com/QwenLM/Qwen-Image/issues/137):
  generated content does not fit the mask.
- [Qwen-Image-Edit](https://qwenlm.github.io/blog/qwen-image-edit/): boxes
  drawn on the picture.
- [Qwen Image Edit Inpaint](https://stable-diffusion-art.com/qwen-image-edit-inpaint/).
- [Qwen-Image-2.1 on Hugging Face](https://huggingface.co/Qwen/Qwen-Image-2.1):
  "specify local edits via circles, painted annotations, or separate masks";
  the "Circle-guided multi-region editing" figure.
- [Qwen-Image 2.1 prompt guide (third party)](https://themindstudio.cc/mindcraft/docs/tutorial/prompt-guide-qwen-image-2-1?lang=en):
  the multi-region demo's prompt, quoted.
- [Qwen-Image 2.1 Pro, editing images](https://runware.ai/docs/models/alibaba-qwen-image-2-1-pro/guides/editing-images):
  colour naming, "a color that appears nowhere else in the frame", and grain
  over repeated rounds.
- [Qwen-Image issue #289](https://github.com/QwenLM/Qwen-Image/issues/289):
  box coordinates in the prompt did not edit the region.
- [comfyui_qwen_edit_pixel_perfect](https://github.com/oron1208/comfyui_qwen_edit_pixel_perfect):
  the mask tinted only for the vision encoder, the reference from the clean
  original, a byte-exact composite.
- [Pixel-perfect Qwen-Image-Edit with ReferenceLatent](https://lilting.ch/en/articles/qwen-image-edit-pixel-perfect-referencelatent):
  output shift and drift outside the edit, fixed by compositing.
- [Photoshop web: Nano Banana generative edits](https://www.adobe.com/learn/photoshop/web/nano-banana-generative-edits):
  annotation-layer prompting.
- [Krita AI Diffusion: regions](https://docs.interstice.cloud/regions/):
  a prompt per region plus a root prompt.
- [ChatGPT images: editing](https://help.openai.com/en/articles/9055440-editing-your-images-with-chatgpt-images):
  one selection; "edits may extend beyond the area you selected".
