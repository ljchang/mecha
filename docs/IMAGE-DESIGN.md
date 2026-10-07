# Image generation, redesigned

**Status: DRAFT for the owner's rulings** (2026-10-07).

- **Lead:** mecha-7e.
- **Evidence and review:** mecha-a3. a3 owns the evidence base and the acceptance gates, and reviews this draft adversarially.
- **Supersedes:** `IMAGE-SCENE-DESIGN.md`, plus the compile and edit halves of `IMAGE-COMPILER-DESIGN.md` (§3) and `PERSONA-CONTEXT-DESIGN.md` §5.5.
- **Keeps:** the library store (`IMAGE-COMPILER-DESIGN.md` §2, §4–§7), background jobs (`BACKGROUND-JOBS-DESIGN.md`) and incognito (`INCOGNITO-DESIGN.md`) as they are.

This is a from-scratch design for how mecha draws and changes pictures. It is written with what the measurements of the last two weeks showed, and with what the models actually did with the current interface. It does not keep anything for backwards compatibility. It keeps every feature the owner uses and that works. It retires every input and code path that exists only to referee between overlapping ways of saying the same thing.

The numbers below are counts and rates only. Their source is mecha-a3's evidence file (`EVIDENCE.md`, local, never committed) and the measurement docs it cites.

## 1. Why redesign

Three ways to say "who is in it" (`prompt`, `cast`, `scene.people`) and three ways to say "change it" (`prompt` + references, `edit`, `scene`) were added over four refactors. Each was right when it landed. Together they make the model choose between overlapping shapes, and make the code referee every combination. The result, from the corpus of 592 `image_generate` calls between 09-25 and 10-07:

- **Half of all calls drew nothing** (291 of 592). Almost every refusal was inside a loop: 162 name-guard refusals on one day, 81 busy refusals.
- **Edits mostly did not change what they were asked to.** 84% came back as near-copies of their canvas: pose 84%, camera 70%. Half of all edits sit two or more deep in a chain, where identity drifts about 0.77 per step.
- **The new `scene` field was never sent from the edit panel** (0/20 replayed). Three of the four steering texts point the model at `edit`. With the best rewording, 12 of the 13 `scene` calls were refused for also carrying `cast`.

The code shows the same split. Of about 5,400 non-test lines in `imagegen.rs`, roughly 1,700 referee between input shapes or patch around them:

- the scene-change router (~530);
- the edit identity and cast-merge block (~660);
- `cast_self` (~380);
- the name guard (~160).

The parts that work, the renderer, the jail, masks, face crops, the library compile and jobs, are about 2,000 lines and stay.

## 2. What we know works

These are measured, mostly with the owner's eye as ground truth (M1–M4, #408, E1–E12, the replay of 10-07).

1. **Identity comes from the library.** A head crop plus the library's own description works on every edit (M1, by the owner's eye). New pictures use the whole portrait at 512², which the owner rated good (M3) until §8.1's crop-on-new-pictures measurement passes. A model-drawn portrait does not help (M4), and neither does 1536² resolution (M4).
2. **Restage, never chain.** A new pose or camera drawn afresh on the scene's place beat the edit chain at every step (M2). An edit chain never moves the camera (0/2), and loses identity by step 4.
3. **A place photo works as the canvas** (24/24 placed, M1).
4. **Two people touching, cast in one pass:** 8/8 interactions with no identity bleed (M3).
5. **An edit prompt must not caption the picture it already has.** "Keep X unchanged. Have her stand up" stood a sitting person up 12/12; a caption did it 0/12 (#408). The tool, not the model, writes that form.
6. **The place must be the setting only.** A restage whose place was the model's whole first prompt drew the asked pose 0/3, with the old pose back every time. With a room-only place it drew 3/3. Reusing the base picture's seed kept the room (1/1); without it, every restage drew a different room (6/6).
7. **Typed extraction is reliable where the model's own choice is not.** The owner's words plus the picture's record, through a one-shot schema-constrained call with no tools and no history, gave the right typed change 32/32 with thinking and 28/32 fast. All 4 fast misses filed a clothes change as a retouch; that is a schema problem (§5.3).
8. **Prose, not structure, for what someone does.** Action-unit and pose JSON were ignored by the image model; prose worked (10-05).
9. **Loops are made by the panel's note sitting beside a "being made" result.**
   - Panel turns ran away 5 times in 13, typed turns 0 in 19.
   - A loop already in the history cannot be repaired from context.
   - A clean end needs `tool_choice: "none"` plus the note once plus a "picture is on its way" line: 5/6 clean, against 1/6 for today's LoopGuard exit.

## 3. What is used, and what is not

From the corpus (592 calls, 111 manifests):

| Field or feature | Used | Decision |
|---|---|---|
| a new picture's words | 71% | kept, as typed place and light (§4) |
| `reference_images` | 55% | replaced by one `picture` (§5) |
| owner photos as the canvas | 44 calls, 13 renders | kept: an owner photo is a place (§5.2) |
| `cast` | 19% | replaced by `scene.people` (§4) |
| `edit.change` | 29% | replaced by `retouch` and scene changes (§5) |
| `edit.keep` | 37% of edits | retired from the model: no measurable effect in use (85% vs 91% near-copy); the tool writes the keep clause itself |
| `size` | 56% | kept |
| `seed` | 48% (72 on edits, where it is ignored) | kept on new pictures only |
| mask (the Edit modal) | 14 calls | kept |
| `style` | 11 calls, none reached a manifest | kept as a scene field |
| `extras` | 0 | retired: a non-library person is a `people` entry with a description (§4) |
| `negative_prompt` | 0 | retired |
| `edit.face`, `edit.camera` | 0, 0 | retired |
| two or more references | 4 calls | retired |
| `scene` (since 10-07 18:29Z) | 0 | becomes the only shape |

What a new picture's free prompt carried beyond its people: light or mood 79%, framing 11%, text to render 1 in 264. So a scene needs a light line where prose lives. Nothing measured needs free prose anywhere else.

## 4. The one representation: a scene

A picture is a scene.

```
Scene {
  place:  Words("a narrow kitchen with a window over the sink") | Picture(path, hash)
  light:  "late afternoon sun, warm and hazy"         // optional prose: light, mood, framing, quoted text
  camera: "from low by the door, looking up"          // optional prose
  style:  library style name                          // optional
  people: [ Person { who, wearing, doing } ]          // in left-to-right order
}
Person.who = a library character's name | "self" | a description of someone not in the library
```

- **`place` is the setting only.** It has no people, looks or poses. It is either words, or a picture (an owner photo, or an earlier picture used as a room).
- **`who`** is resolved against the library:
  - an approved character brings its portrait (new picture) or head crop (edit), and its description verbatim;
  - `self` is the persona's linked approved character;
  - anything else is drawn from its own words, as an extra is today. That needs no separate field, and `extras` was never used.
- **`wearing` and `doing` are prose** (§2.8). They are required for anyone the call introduces.
- **The scene is recorded** for every picture drawn: per chat, outside the jail, in a store the harness writes (§6). That covers the assistant chat as well as personas, so there is one path, not a scene path and a no-scene path. Each field carries its origin (clean or untrusted), as `scene.rs` does now.

## 5. The one operation: draw a scene, or change one

### 5.1 The model-facing call

```
image_generate {
  picture:  path        // the one picture to change, or an owner photo to use as the place
  scene:    {...}       // a whole scene for a new picture, or the fields that change
  retouch:  "..."       // one small change to the picture itself: an object, a colour, a detail
  mask:     path        // the Edit modal's painted area, for a retouch
  size:     square | landscape | portrait
  seed:     integer     // a new picture only
}
```

- **Retired inputs:** `prompt`, `cast`, `extras`, `edit` (`change`, `keep`, `face`, `camera`), `negative_prompt`, multi-picture `reference_images`.
- **The model never writes the image model's prompt.** The compiler does, in the measured forms (§2.5, E1–E12).
- **Equal is unchanged.** On a picture with a record, a scene field equal to its recorded value is not a change. The model's habit of restating everyone then costs nothing. This is what retires both the `cast` versus `scene.people` overlap and the refusals between them.
- **One retouch, in words.** It is the only free text that addresses the picture itself. The tool writes the #408 keep form around it.

### 5.2 What each call becomes

The planner reads the call against the picture's record and picks one render. Nothing rewrites the call into another shape.

| Call | Render |
|---|---|
| no `picture`; a whole scene | **New picture.** Library portraits, the E1–E12 compile, the call's seed or a new one. |
| `picture` is an owner photo with no record; a scene with people | **Place on the photo.** An edit of the photo with each person's head crop and description (M1). The photo becomes the scene's place. |
| a change to someone's pose, the camera, the place, or a removal | **Restage.** Everyone drawn afresh on the scene's place, at the base picture's seed (§2.6): an edit of the place picture with crops when the place is a picture, a new picture from the place words when it is words. |
| a change to someone's clothes, or someone added | **Edit of the picture,** with the crops of the people it changes, in the #408 keep form. Clothes come back unchanged elsewhere 100% (correct for this case). |
| `retouch` (with or without `mask`) | **Retouch** of the picture, in the #408 keep form; masked as today. |

**Budgets stay where they were measured:**
- an edit-shaped render carries the canvas plus two face crops (`EDIT_REFERENCE_BUDGET` 3);
- a new picture carries up to `MAX_CAST` 4 portraits;
- a scene keeps up to 8 people.

A change that cannot fit is refused in the scene's own terms, naming the people. A person without a library entry costs no budget; they are drawn from words.

**Seeds:** an edit-shaped render always samples fresh (#306); a restage reuses the base picture's seed unless the call names one.

### 5.3 The edit panel: the harness extracts, the persona replies

A panel press is the owner's instruction to the image model, not something said to the persona (`persona/edit.rs`'s own doc). So:

1. **The owner's words plus the picture's record go through a one-shot extraction.** It has no tools and no history, and is constrained to the scene-change schema: `scene` fields plus an optional `retouch`, with **no `kind` discriminator** (§2.7: clothes need an obvious home, which is `people[].wearing`).
2. **The extracted call is dispatched through `Agent::dispatch_one`** (#592). It goes through every gate a model call meets, with its own call id, inline, on the chat's job seat, with taint recorded.
3. **The history records one fact,** a `HarnessPicture` record: "Picture X was changed into Y: <the typed change>". The card shows the new version from it.
4. **The persona replies in a line or two,** in its own voice, with no tools offered for that reply.

This removes the panel loop at its source (§2.9), because the persona never makes the call. It also removes the model's field choice for the turns where it chose wrong. Typed requests in chat ("draw us at the beach") still go through the persona model, with the one-shape schema (§5.1).

### 5.4 Regenerate (R8)

The same scene, a new seed, the same render plan. It goes through `dispatch_one` and is recorded as a `HarnessPicture` with `how: redraw`. The card shows versions ‹ 1/2 ›, and the version showing is the one Edit and Regenerate build on. R8's ruling stands. R8-2's call reconstruction (#593) is not needed, because the scene is the record. #593 is closed unmerged.

### 5.5 A run makes at most one picture

Whatever the path, a deferred picture already started in a run is never started again in that run. That is enforced in the agent loop on the tool's job kind, not by the model or by a refusal string. A run that has queued its picture ends cleanly:
- the next request goes with `tool_choice: "none"`, the turn's note once, and "the picture is on its way; answer in a line";
- a reply that is only a tool-call block is replaced and never shown.

This replaces `REPEAT_REFUSED`, `REPEAT_IN_FLIGHT` and the image half of the LoopGuard.

## 6. The record

- **Where:** one scene store per chat, outside the jail, harness-written.
  - **Persona chats** also write the persona's latest and the content-hash index, so a picture carried into another chat is found by its bytes (R3, R7).
  - **Incognito** keeps its copy in the room and never writes back (R9).
- **What:** the scene, with origins per field. The place is the setting only (§4). Plus the picture's hash, its seed, size, render plan and the library versions used.
- **The manifest** in the jail keeps only what the owner-facing doors read: the image path, the call id (orphan repair), the seed and the cast names (save-to-library), plus a pointer to the scene by hash. A run can write the jail, so nothing reads a scene back out of a manifest.

## 7. What stays

- **Renderer:** the ComfyUI backend, the fixed graph with values only, loopback only, preflight, trail and temp deletion, interrupt-this-job, idle unload, and the memory guard.
- **Jail:** `read_references` (O_NOFOLLOW, regular files only, sniffed types, size caps), `fit_reference` (upright, EXIF stripped), and `save` (create-new, never overwrite).
- **Masks:** `prepare_mask`, compositing, and the Edit modal.
- **Faces:** face crops (`face.rs`, RetinaFace, the crop cache) and the library's compile templates (E1–E12), driven by typed people instead of `cast`.
- **Jobs:** the deferred job, one per chat, late landing, Stop semantics, orphan repair, and the picture clock.
- **Library:** the library, its approval and lock, `image_library`, `image_library_propose`, `persona_propose`'s character link, and the library page and save-to-library.
- **Other tools and pages:** `image_view`, incognito image handling, the card, Download, and the call screen's picture slot.

## 8. What is retired

| Retired | Why |
|---|---|
| `prompt`, `cast`, `extras`, `edit`, `negative_prompt`, multi `reference_images` | §3: unused, or replaced by the one shape |
| the router that rewrites a `scene` call into an edit or prompt call (~530 lines) | the planner draws directly (§5.2) |
| the edit cast-merge block: record merge, `left_out`, the six-way `people_from`, the waiver (~660 lines) | equal-is-unchanged plus one record (§5.1, §6) |
| `cast_self`'s duplicate and possessive refusals (~380 lines) | `self` is a `who` value; no prose names the persona to be guarded |
| the name guard on free prompts (~160 lines) | no free prompt; `place` and `light` are checked for a library name, and refused plainly if one is found |
| the foreign-scene placeholders and blanking | the manifest no longer carries a scene (§6) |
| near-copy advice, strikes and library-redraw advice | restage replaces the chains they were warning about; the layout measurement stays in the record as data |
| `same_layout_as`/`original_of` | the one-hop lineage is unneeded under restage |
| `REPEAT_REFUSED`/`REPEAT_IN_FLIGHT`; the image half of the LoopGuard | §5.5's structural rule |
| `demote_unknown` | a non-library `who` is drawn from words directly |
| the panel note telling the persona which field to use | §5.3 |

Estimated net: roughly 1,700–2,000 lines of `imagegen.rs` deleted, with the tests that measured the retired seams. The remaining tests are rewritten against the one shape.

## 9. Acceptance gates

mecha-a3 runs these on each implementation branch, on the branch's own tool surface and notes. A failure on any gate blocks the merge.

| Gate | Pass | Proposed threshold |
|---|---|---|
| **G1 typed-change correctness** (replayed real turns plus controls: clothes, camera, object, add someone, owner photo as place) | the typed change is right | ≥ 90%, and **0** refusals for shape |
| **G2 the wanted call still happens** | request 1 of every replayed picture turn calls the tool; no reply narrates a picture without one | ≥ 95% |
| **G3 one picture per run** | at most one started, structurally; 0 calls written as text; 0 empty replies | 100% |
| **G4 renders, the owner's eye** (face-sized labelled sheets, ≥ 3 seeds each) | restage pose right, room held across restages, identity holds; words place and picture place | owner's verdict |
| **G5 after deploy** | runaway rate and pose/camera near-copy rate on the first real chats | runaways 0; near-copy on pose/camera well below 84% |

## 10. Build order

Each PR goes through its review loop, then a3's gates, then the owner's merge word.

1. **The scene schema and the planner, with the retired inputs deleted.** This covers the record per chat (§6), equal-is-unchanged, place as the setting only, seed reuse, and the refusals in scene terms. It is the big one.
2. **One picture per run** (§5.5).
3. **The panel through extraction plus `dispatch_one`,** and the `HarnessPicture` record (§5.3).
4. **Regenerate** on the same record (§5.4).
5. **The web:** versions on the card, and Edit and Regenerate on the version showing.

Steps 1 and 2 are independent and can run in parallel lanes.

## 11. Questions for the owner

1. **Is the scene recorded for the assistant chat too** (§6)? Recommended: yes, one path.
2. **`light` as the one prose field beside the place** (§4)? It covers mood, framing and quoted text.
3. **Close #593 unmerged** (§5.4)?
4. **The thresholds in §9**, especially G5's near-copy target.
5. **Crop or whole portrait on new pictures** (§2.1): keep the whole portrait until §8.1's crop measurement passes on the owner's sheets, as ruled for R1?
