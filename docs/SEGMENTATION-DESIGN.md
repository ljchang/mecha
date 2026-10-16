# Segmentation for picture edits: design

**Status:** ruled by the owner, 2026-10-16 (§6). Nothing is built yet; S1 and S2 come first.
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

**SAM 3.1 where it does better.** ComfyUI also loads SAM 3.1 natively (`SAM31` in `comfy/supported_models.py`), and the owner has access approval for it. The checkpoint is chosen by measurement on our own edits (§7), not by version number.

**Sapiens2 is out.** Its licence forbids pornographic use and deepfakes, and persona pictures are in that territory. That makes it unusable for mecha's picture tool, even though it is the better body-part segmenter. SAM 3's text prompts cover the same ground ("face", "hair", "the green jacket", "the left sleeve").

## 3. Where it runs: inside the image server

- **On the GPU, beside the render it serves.** Every masked edit already runs on ComfyUI, and ComfyUI has SAM 3 natively, so running it there adds no stack, no new service and no port. It shares the server's memory admission (`imagegen::memory_need_mb`) and idle reset. A segmentation is a fraction of a second against a 40–60 s render.
- **The detector and ArcFace stay native** (`face.rs`, `face_id.rs`). They are small, CPU-fast, cached, and needed outside the image server: the CLI, the judges, the Mac without ComfyUI.
- **The weights need a pinned, original-layout checkpoint.** ComfyUI's `SAM3` model reads Meta's original key layout (`detector.*` / `tracker.*`, `comfy/model_detection.py`), not the transformers layout cached here (`detector_model.*`). The install fetches a pinned file through `fetch.rs` (sha256-checked, resumable) into ComfyUI's checkpoints. Which file to pin is open question Q2.
- **Memory:** about 3.3 GB at F32 as cached, and about half at fp16. It loads with the first segmentation and goes with the server's idle reset.

## 4. What changes, in order of value

### S1. Words to mask (failure 1)

- `mask` keeps its name in the model's schema, with a new description: "The part of the picture to change, in a few words ('the green jacket', 'the sky'), or a painted mask's path."
- Words go to `SAM3_Detect` with the picture and threshold 0.5.
  - One instance: that is the mask.
  - Several: the union. Each painted region still settles "which one" (§7.7).
  - None: the mask is left out and said ("Nothing matched 'the lantern', so the change was made to the whole picture."), as a dropped mask is today.
- The result always says what was found: "Masked 'the blue scarf': 1 area." **A mask the model chose is a guess, so it is said, never silent** (IMAGE-REGION-EDIT-RESEARCH §6).
- The found mask goes through `prepare_mask` like a painted one, with the same grow, feather and composite. Nothing outside it moves.

### S2. Face and hair edges for face regions (failure 2)

- When a region takes a face (#706), the composite uses the painted region ∩ SAM 3's "face" mask grown by a few pixels, instead of the paint alone. With "face and hair" in the region's words, the "hair" mask is added.
- **Measured before it ships:** ArcFace against the face wanted (`mecha imagelib face compare`, from #705), and the seam by eye, on the same photo and crops as #706. Paint alone is the control.

### S3. Snap a painted stroke to its object (failures 2 and 3)

- A painted region's bounding box goes to SAM 3 as a box prompt. The mask it returns replaces the stroke only when it covers most of the stroke, and the stroke covers most of it. The thresholds are to be measured.
- If they disagree, the stroke is kept: **the owner's paint is never silently replaced.**
- A switch on the edit modal ("snap to edges"), off by default until measured.

### S4. Change inside the region (failure 4), a consequence of S1

- **Painted edits already have it:** `imagegen::layout_similarity_painted` reads only the cells the mask mostly covers, whenever there is a `mask_plan` (review of #429).
- **The gap is the unmasked edit.** That is where all 19 word-mask calls went, because their masks were dropped. S1 gives them a mask, so they gain the same in-region reading without new code. What is left is the wording: say "the painted area came back unchanged" when the reading says so, rather than the near-copy layout fact alone.

### S5. Arm B, per-person edits in layers

Out of scope here. It needs the person split from the scene's record, which is a layers design of its own. This design only makes the mask available.

## 5. Failure is said, never silent

- **SAM 3 not installed, or its load fails:** words in `mask` are left out and said exactly as today, the snap is skipped and said, and the composite falls back to the paint. Never a refusal of an edit that would have drawn without it.
- **A segmentation that times out or errors:** said, and the edit goes on unmasked or with the paint, the same as a missing install.
- **The checkpoint's sha256 mismatches:** treated as not installed (`fetch::Cached::Mismatch`), as the face detector does.

## 6. The owner's rulings (2026-10-16)

- **Q1. What to build: S1 (words to mask) and S2 (face and hair edges) first.** S4 follows from S1 (above). S3 waits until those are measured.
- **Q2. The checkpoint: whichever works best for our edits.** The owner has access approval for SAM 3.1. Both are tried on the §7 sets and the better one pinned by sha256 in `fetch.rs`. The cached transformers-layout weights do not load in ComfyUI, so an original-layout checkpoint is fetched either way.
- **Q3. Part of `image`.** Segmentation serves pictures now. Video may want it later; when it does, it will likely be on pictures taken from the clip.
- **Q4. SAM 3 tracking on video: possibly, later.** Not designed here.

## 7. Measurement plan

- **S1:** replay the 19 word-masks from the corpus through the new path. Record what SAM 3 found per call, then judge by eye whether the right thing was masked. The count goes in the PR. The words are not quoted (no persona text in the repo).
- **S2:** two seeds × paint-only versus SAM-edged composite on the #706 photo. ArcFace by `mecha imagelib face compare` (#705), plus the seam by eye in a contact sheet.
- **S3:** the §7.7 set (two shirts, two cups, a sleeve): stroke-only versus snapped masks, wrong-target and missed-target counts.
- **S4:** the corpus's three edits whose layout similarity was at least 0.97: does the in-region difference tell a real change from none?
