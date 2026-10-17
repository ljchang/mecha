# Segmentation for picture edits: design

**Status:** ruled by the owner, 2026-10-16 (§6). S1 and S2 are built, and SAM 3.1 installs with the image slot (`recommend::SAM_SOURCE`, §3), 2026-10-16 (#710). S3 and S4 wait.
**Owner's ask (2026-10-16):** "we also have the SAM segmentation model available to use as well if it is useful to help with the masking", and "whatever will work the best and is the most performant".
**Where it fits:** IMAGE-DESIGN §15.6 lists "in-place per-person edits (arm B): segmentation-backed" as the owner's open decision. This design answers it, and also covers the masking problems found in the 2026-10-16 corpus read.

## 1. What it is for

The 2026-10-16 corpus read covered 145 image calls since 10-08. These are the failures segmentation would fix, ordered by how often they hit:

1. **Words in `mask`.** The model writes a region's name (the shape of "the scarf" or "the upper body") into `mask` on 19 of 19 calls that set it. `picture::parse` drops every one ("not a painted picture's path"), so the edit redraws the whole picture.
2. **Face transfer edges.** A face region is now sampled unmasked and laid back through the owner's paint (#706, ArcFace .79–.85 against the face wanted). The paint is a brush stroke, so the composite's edge follows the stroke, not the hairline or the jaw.
3. **The same kind of thing twice.** "The striped shirt" in a picture of two people in striped shirts. IMAGE-REGION-EDIT-RESEARCH §7.7 found that one painted area with words changed the wrong target 2 of 12 times, and one region per target 0 of 12. Its masks came from SAM 3 text prompts, run outside the tree.
4. **"Did the edit change anything?"** `layout_similarity` is a 32×32 luma correlation. It cannot tell a face swap (1.00) from no change (1.00).
5. **Per-person edits in layers** (IMAGE-DESIGN §15.6, arm B).

## 2. The model: SAM 3 (3.1) only

| | SAM 3 (`facebook/sam3`) | Sapiens2 seg (`facebook/sapiens2-seg-*`) |
|---|---|---|
| Does | text, point or box prompts to per-instance masks | 28 human body-part classes per pixel |
| Licence | SAM License (2025-11-19), "Proprietary Free": a royalty-free grant with no non-commercial limit; prohibits trade-control, military and reverse-engineering uses, and requires the agreement to go with any redistribution | Sapiens2 License: prohibits use "in a pornographic or defamatory manner", "to create deepfakes", and "for biometric processing" |
| In ComfyUI | native: `comfy_extras/nodes_sam3.py` (`SAM3_Detect`, `SAM3_VideoTrack`) in both builds here | no native node |
| Cached here | `models--facebook--sam3` (3.3 GB, F32, transformers layout) | `models--facebook--sapiens2-seg-0.4b` (1.6 GB) |

**SAM 3.1 where it does better.** ComfyUI also loads SAM 3.1 natively (`SAM31` in `comfy/supported_models.py`), and the owner has access approval for it. SAM 3.1 is the checkpoint (§3, the owner's Q2), and §7 checks it on our own edits.

**Sapiens2 is out.** Its licence forbids pornographic use and deepfakes, and persona pictures are in that territory. That makes it unusable for mecha's picture tool, even though it is the better body-part segmenter. SAM 3's text prompts cover the same ground ("face", "hair", "the green jacket", "the left sleeve").

## 3. Where it runs: inside the image server

- **On the GPU, beside the render it serves.** Every masked edit already runs on ComfyUI, and ComfyUI has SAM 3 natively, so running it there adds no stack, no new service and no port. It shares the server's memory admission (`imagegen::memory_need_mb`) and idle reset. A segmentation is a fraction of a second against a 40–60 s render.
- **The detector and ArcFace stay native** (`face.rs`, `face_id.rs`). They are small, CPU-fast, cached, and needed outside the image server: the CLI, the judges, the Mac without ComfyUI.
- **The weights: SAM 3.1, decided.** ComfyUI's own fp16 repackaging (`Comfy-Org/sam3.1`, `checkpoints/sam3.1_multiplex_fp16.safetensors`, 1,745,546,848 bytes, sha256 `9ba99c92…eb03`; the SAM licence). It is what ComfyUI's SAM3 blueprint loads, with `CheckpointLoaderSimple`, and the owner has access approval for it. SAM 3 is not an arm: its only file ComfyUI can load is Meta's original pickle (`sam3.pt`), and the transformers-layout weights cached here (`facebook/sam3`) do not load at all. §7 checks the choice on our edits rather than choosing it.
- **Installed as a second `Source` on both of the image slot's rows** in `recommend::SLOTS`: the int8 ConvRot row and `IMAGE_MAC` each carry their own `sources`, so SAM goes in both, or the Mac never fetches it. `Comfy-Org/sam3.1` is its own repo, beside the slot's `Comfy-Org/Qwen-Image-2.1`. It is fetched into the hub, symlinked where ComfyUI looks, recorded in the manifest, and checked against the server's own listing, as `comfyui_unit.rs` does for the slot's files. Being in the slot's `sources` is what makes `features --probe` and the install plan count its 1.7 GB.
- **The turbo model is deliberately different**, and it is the counter-example rather than the precedent. `TURBO_FILE` sits outside the slot's `sources`, in `comfyui_unit.rs`, fetched on demand by the owner's 2026-10-11 ruling, with its own accounting in `features enable`. SAM goes in `sources` because S1 and S2 serve every masked edit, not an opt-in render mode. (The copy placed straight into `models/checkpoints/` on 2026-10-16 for §7's first read is replaced by this install, not kept beside it.)
- **Memory: segmentation runs only inside an admitted picture job, and every segmentation is followed by that job's render.** The admission gauge cannot tell which model a ComfyUI process holds: `loaded_from_stats` answers "has loaded anything", so SAM loaded alone would make the next picture's ask about 8 GiB too small (`memory_need_mb`, `load_cost_mb`). Inside a picture job, SAM never loads without the image model, so the gauge stays true. That holds *because* no segmentation ends the job without a render: a mask SAM found that `prepare_mask` then refuses is dropped and the edit drawn without it (S1), never a refusal that would leave SAM loaded alone. The job's need gains SAM's measured peak (`imagegen::SEGMENT_NEED_MB`, below) whenever the call may segment (`Call::may_segment`: words in `mask`, or a painted region about a face) and the server can, warm or cold, and the server's idle reset takes both. **A job stopped mid-segmentation is the one gap:** SAM can be left loaded without the image model, the gauge then reads warm, and the next job that does not segment asks the warm figure while the server still has to load the model beside SAM's ~1.9 GiB. The stop is the owner's, and the window is the idle reset's timer, which the stopped job arms. Every segmentation in a job shares one 120 s budget apart from the render's timeout (`SEGMENT_BUDGET`).
- **Measured cost** (2026-10-16, warm server, the #706 photo at 640×1632): the first segmentation in a process took 5.0 s, loading included; each one after took 1.0 s. That is against 40–60 s for the render it serves. **Memory:** SAM 3.1's cold load peaked at 2,528 MB of GPU memory above a freed server (369 → 2,897 MB, ComfyUI's own process by `nvidia-smi`), and keeps 1,920 MB resident after. On the GB10 that is the system's memory, so `SEGMENT_NEED_MB` is 3 GiB. Not measured on a Mac.

## 4. What changes, in order of value

### S1. Words to mask (failure 1)

- `mask` keeps its name in the model's schema, with a new description: "The part of the picture to change, in a few words ('the green jacket', 'the sky'), or a painted mask's path." **Both places that state the rule change together:** the schema's description and the tool's instruction block, which says "With a painted area, also pass its `mask`, and never make one up". Two contradictory sentences about one field in one cached prefix would be worse than either.
- **Words and paths are told apart by `picture::looks_like_a_picture_path`** (a `/`, no whitespace, a picture extension), so "the green jacket" is never read as a path. **Path-shaped wins, by design:** a path-shaped `mask` that does not resolve stays what it is today, left out and said ("The mask inbox/… was left out"), and never falls back to a text prompt. The ordering is deliberate, not an accident of the helper.
- **A SAM-found mask is never a refusal.** `prepare_mask` refuses a mask whose marks do not survive its grow step, which is right for the owner's paint. A mask SAM found that it refuses is dropped and said ("What matched 'the left earring' was too small to edit alone, so the change was made to the whole picture."), as a missing match is. Only the owner's own paint keeps the refusal.
- **Not beside `regions`.** Regions come from the edit panel with their own painted index, and `picture::parse` reads a `mask` beside them as that index. A word `mask` beside `regions` is refused by name, never read as either.
- **The words are capped**, as every other prose field is. Today the path check is the field's only bound (200 characters); words get a `parse` cap of their own.
- **A model-authored mask, and IMAGE-REGION-EDIT-RESEARCH §6.** §6 names "a model-chosen or all-white mask" as the silently-degrading guard. S1 makes the model the mask's author on purpose, and it comes out on the safe side for two reasons:
  - Today a word mask is **dropped**, and the edit redraws the whole picture. A SAM mask strictly narrows what changes, so the case §6 feared is the status quo, not S1.
  - A word mask carries no path, so it cannot name another file in the workspace, which is §6's other half.

  What §6's worry still asks for is that the mask be said ("Masked 'the blue scarf': 1 area"), never silent.
- Words go to `SAM3_Detect` with the picture and threshold 0.5.
  - One instance: that is the mask.
  - Several: the union. Each painted region still settles "which one" (§7.7).
  - None: the mask is left out and said ("Nothing matched 'the lantern', so the change was made to the whole picture."), as a dropped mask is today.
- The result always says what was found: "Masked 'the blue scarf': 1 area." **A mask the model chose is a guess, so it is said, never silent** (IMAGE-REGION-EDIT-RESEARCH §6).
- The found mask goes through `prepare_mask` like a painted one, with the same grow, feather and composite. Nothing outside it moves.

### S2. Face and hair edges for face regions (failure 2)

- When a region takes a face (#706), the composite uses the painted region ∩ SAM 3's "face" mask grown by a few pixels, instead of the paint alone. With "face and hair" in the region's words, the "hair" mask is added.
- **As built** (`imagegen::edge_faces`): the face is found on the canvas inside the job, grown 5 px and feathered (σ 3), and the composite's mask is `max(the no-face regions' own softened paint, min(soft paint, face))`. It is never past the paint's own edge, so nothing outside the paint can change. A face that fills under a tenth of the face regions' paint is someone else's (a neighbour's face, or one at the stroke's edge): the paint is kept, and said.
- **Measured before it ships:** ArcFace against the face wanted (`mecha imagelib face compare`, from #705), and the seam by eye, on the same photo and crops as #706. Paint alone is the control.

### S3. Snap a painted stroke to its object (failures 2 and 3)

- A painted region's bounding box goes to SAM 3 as a box prompt. The mask it returns replaces the stroke only when it covers most of the stroke, and the stroke covers most of it. The thresholds are to be measured.
- If they disagree, the stroke is kept: **the owner's paint is never silently replaced.**
- A switch on the edit modal ("snap to edges"), off by default until measured.

### S4. Change inside the region (failure 4), a consequence of S1

- **Painted edits already have it:** `imagegen::layout_similarity_painted` reads only the cells the mask mostly covers, whenever there is a `mask_plan` (review of #429).
- **The gap is the unmasked edit.** That is where all 19 word-mask calls went, because their masks were dropped. S1 gives them a mask, so they gain the same in-region reading without new code, **above a size floor.**
- **The floor:** `layout_similarity_painted` reads only the 32×32 cells the soft mask covers at 200 or more, and returns `None` under 16 of them ("no reading, not a low one"). A mask for "the left earring" or "the lantern" often sits under it.
- **What the owner is told:**
  - a reading when the area is large enough;
  - "the masked area is too small to compare" when it is not, rather than the near-copy layout fact;
  - "the painted area came back unchanged" when the reading says so.

  A dash is never a zero. This wording is new code: today the landing site's `.ok().flatten()` folds the floor and a failed read into one `None`, and the owner must be told them apart.

### S5. Arm B, per-person edits in layers

Out of scope here. It needs the person split from the scene's record, which is a layers design of its own. This design only makes the mask available.

## 5. Failure is said, never silent

- **SAM 3 not installed, or its load fails:** words in `mask` are left out and said exactly as today, the snap is skipped and said, and the composite falls back to the paint. Never a refusal of an edit that would have drawn without it.
- **A segmentation that times out or errors:** said, and the edit goes on unmasked or with the paint, the same as a missing install.
- **The checkpoint's sha256 mismatches:** treated as not installed (`fetch::Cached::Mismatch`), as the face detector does.

## 6. The owner's rulings (2026-10-16)

- **Q1. What to build: S1 (words to mask) and S2 (face and hair edges) first.** S4 follows from S1 (above). S3 waits until those are measured.
- **Q2. The checkpoint: whichever works best for our edits.** The owner has access approval for SAM 3.1. Settled in §3: ComfyUI's fp16 repackaging of SAM 3.1, pinned by sha256. §7 checks it on our edits.
- **Q3. Part of `image`.** Segmentation serves pictures now. Video may want it later; when it does, it will likely be on pictures taken from the clip.
- **Q4. SAM 3 tracking on video: possibly, later.** Not designed here.

## 7. Measurement plan

- **S1:** replay the 19 word-masks from the corpus through the new path. Record what SAM 3 found per call, then judge by eye whether the right thing was masked. The count goes in the PR. The words are not quoted (no persona text in the repo). **Read (2026-10-16, 22 calls by then):** 16 had their picture on disk, and SAM found something for all 16. By eye, 11 were right, 2 partial and 3 wrong. Every one that missed was a name or a position: SAM 3 finds things by what they are, so it cannot know a name, and a position phrase gave it a noun inside the phrase. Both statements of the rule now ask for what the thing is, never a name or a position. A wrong find redraws a smaller area than was meant, where before the whole picture was redrawn; the result says what was found either way.
- **S2:** two seeds × paint-only versus SAM-edged composite on the #706 photo. ArcFace by `mecha imagelib face compare` (#705), plus the seam by eye in a contact sheet. **First read (2026-10-16):**
  - The "face" mask runs jaw to hairline, and the "hair" mask catches the loose strands.
  - "Face and hair" as one prompt returns the face alone, so the two are asked separately and joined.
  - The same render pasted back through the face mask scores the same identity as the ellipse (.81), keeps the photo's own hairline and earrings, and shows no seam.
  - **Through the built tool** (the real server, one seed): ArcFace .876 against the face wanted (the unedited photo: .196). Every changed pixel is inside the face; the stroke reached about 60 px above it, and the hairline there is the original's.
- **Cost:** time per segmentation and SAM's memory peak, both in §3. The install change moved the image slot's `Peak` (`recommend.rs`, Linux row) from 20,790 to 23,318 MB with the evidence beside it, as the slot's other figures have; the Mac row names SAM as not measured there.
- **S3:** the §7.7 set (two shirts, two cups, a sleeve): stroke-only versus snapped masks, wrong-target and missed-target counts.
- **S4:** the corpus's three edits whose layout similarity was at least 0.97: does the in-region difference tell a real change from none?
