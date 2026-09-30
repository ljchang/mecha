# Region-targeted image edits — research

**2026-09-29.** One question: *can Qwen-Image 2.1 be told where to edit — a
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
| **A. Mark the picture** | a red box drawn on the picture; "inside the red box: …" | a hint; may spill, and the mark may survive | none in the graph; the page draws the mark |
| **B. Mask as a second image** | the original as `<image1>`, a black-and-white mask as `<image2>` | a hint | none in the graph; one more reference |
| **C. Latent noise mask** | the encoded original as the canvas, resampled only under the mask, then composited in code | identical by construction | a second fixed graph, and a composite in `imagegen` |
| **D. Crop, edit, paste** | mecha crops the region plus a margin, edits the crop at full size, pastes it back through a feathered mask | identical by construction | code only; any backend. `IMAGE-COMPILER-RESEARCH.md` §2 and §6 already recommend it |

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

Each edit goes through five arms on the same four seeds: P, a plain edit in
#408's form, plus A–D above, for 60 images. Every prompt names the kept
parts and then the change. For C and D the mask is the box grown by 24 px
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
| **C** noise mask + composite | 4/4 | 1 clear, 1 partial, 2 left sitting | 4/4 | **0 / 0 / 0** | 80 |
| **D** crop, edit, paste | 4/4, basket visibly smaller | **4/4 clear** | 4/4, stripes paler in 2 | **0 / 0 / 0** | 81 |

The outside figure is the mean absolute pixel difference on a 0–255 scale.
P's pose edits reached 49.6 where the scene was recomposed.

- **A and B are hints, and they keep nothing.** Their outside moved as much
  as a plain edit's, within a few points. They helped the pose edit no more
  than P did. The red box never survived into a result (at most 0.5% of its
  outline stayed red).
- **C is seamless and exact, and it under-edits a pose.** Nothing outside
  the mask moved, and at the boundary the new pixels continue the old ones.
  In T2 the grass, the blanket and the man's arm run straight through. It
  landed every swap and every detail, but it stood the woman up less often
  than a plain edit did: 1 clear against 2. The reference and the source latent still
  show her sitting.
- **D lands the pose, and leaves seams where its edge crosses something.**
  With the man cropped out, nothing anchored her, and all four seeds stood
  her up in full. But T2's crop edge ran through the man's forearm, and in
  every seed the feathered paste left it half transparent or pasted grass
  over the blanket. T3's edge ran through open background and came out
  clean. The swap's basket came out smaller in all four seeds: in a
  zoomed-in crop, the model has no sense of scale.
- **The cost is the same for all of them.** A mask is not a slower edit
  (C: 80 s). B's second reference costs about 10 s.

**What this says.** For the local edits the modal will mostly carry (swap
this, recolour that, change a detail), C does what the owner asked for: the
change, and nothing else, at no extra cost. A pose or a move is a different
kind of edit. C holds the rest of the picture but not the change. D makes
the change but cannot blend it when the region's edge crosses a person. Not
yet run:
- **C′**, C with the region greyed out of the reference, so the model is not
  shown what it is meant to redraw. It keeps C's composite. It targets C's
  one weakness and needs one more graph input, not a new approach.
- **D's edge placed in background,** by growing the crop until its border
  clears people. That is harder to do from a box the owner drew.

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
- **The page renders the mask, and the owner is its only author.** A
  black-and-white PNG at the picture's own size, uploaded into the chat's
  jail. `image_generate` gains a `mask` path that it reads through
  `ToolCtx::resolve`, like `reference_images`, and checks against the
  picture's shape. The model passes the path along and never writes
  coordinates, which keeps the graph fixed in code and the tool typed
  values.
- **A mask is not an image for the model to look at.** Web uploads reach the
  model as pixels today (#366), and an attached image arms `private_data`.
  A mask would spend context and taint the chat for nothing, so its upload
  must be a kind the page stores without attaching.
- **The note is the prompt, and the kept parts come free.** With a region,
  what stays is enforced by the mask (C, D) rather than listed in words.

## 6. Open, for the owner

Set, or narrowed, by the results:
- **Which graph backs a painted area.** C for the local edits (measured
  above). For a pose or a move, C′ is the next measurement, about 12 images
  on T2 and a swap check.
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
- **A and B are not worth building on their own.** A mark in the picture
  could still ride along with C as a hint for multi-region notes, but nothing
  here shows it helps.
- Whether a region without a note falls back to the chat message as the
  instruction.

Not set by any measurement:
- Whether annotations on several regions run as one edit or one per region.

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
