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
2. **Restage, never chain.** A new pose or camera drawn afresh on the scene's setting beat the edit chain at every step (M2). An edit chain never moves the camera (0/2), and loses identity by step 4.
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
| a new picture's words | 71% | kept, as a typed setting and light (§4) |
| `reference_images` | 55% | replaced by one `picture` (§5) |
| owner photos as the canvas | 44 calls, 13 renders | kept: an owner photo is a setting, or the picture being changed (§5.1) |
| `cast` | 19% | replaced by `scene.people` (§4) |
| `edit.change` | 29% | replaced by `retouch` and scene changes (§5) |
| `edit.keep` | 37% of edits | retired from the model: no measurable effect in use (85% vs 91% near-copy); the tool writes the keep clause itself |
| `size` | 56% | kept |
| `seed` | 48%: 72 on edits, where it is ignored; of 211 on new pictures, 177 copied an earlier result's seed | retired from the model: every seed reuse that helps is the harness's (§5.2); kept for the CLI and evals |
| mask (the Edit modal) | 14 calls | kept |
| `style` | 11 calls, none reached a manifest | kept as a scene field |
| `extras` | 0 | retired: a non-library person is a `people` entry with a description (§4) |
| `negative_prompt` | 0 | retired |
| `edit.face`, `edit.camera` | 0, 0 | retired |
| two or more references | 4 calls | replaced: a person from a photo is a `who` (§4) |
| `scene` (since 10-07 18:29Z) | 0 | becomes the only shape |

What a new picture's free prompt carried beyond its people: light or mood 79%, framing 11%, text to render 1 in 264. So a scene needs a light line where prose lives. Nothing measured needs free prose anywhere else.

The assistant's 26 new pictures included 8 scenes or objects and 5 texts, diagrams or logos. These have no people and often no place, so the scene's first field describes the picture's subject and surroundings, not only a room (§4).

After drawing, the assistant went on in the same run in 43 runs: `image_view` 23 times, `shell` 14, `fs_read` 6. So a run's end cannot be forced on every chat (§5.5).

## 4. The one representation: a scene

A picture is a scene.

```
Scene {
  setting: Words("a narrow kitchen with a window over the sink") | Photo(path, hash)
  light:   "late afternoon sun, warm and hazy"        // optional prose: light and mood
  camera:  "from low by the door, looking up"         // optional prose: viewpoint and framing
  style:   library style name                         // optional
  people:  [ Person { who, wearing, doing, expression } ]   // in left-to-right order
}
Person.who = a library character's name | "self" | { from: picture } | a description of someone not in the library
```

- **`setting` is everything in the picture except its people.** For a picture with people it is the place and its objects, with no looks or poses. For a picture without people (an object, a diagram, a logo) it is the whole subject, including any text to render, in quotes. It is either words, or a photo used as the room.
- **`who`** is resolved against the library, in this order:
  - an approved character's name or alias brings its portrait (new picture) or head crop (edit), and its description verbatim;
  - `self`, or the persona's own name, display name or folder name (as `cast_self` resolves it today), is the persona's linked approved character;
  - `{ from: picture }` is the one face in that picture (an owner photo of a real person), cropped by RetinaFace and budgeted as a crop. This is use case 4, kept;
  - anything else is drawn from its own words, as an extra is today. That needs no separate field, and `extras` was never used.
- **Library names in the prose fields are checked** (review B2): `setting`, `light`, `camera`, a descriptive `who`, `wearing`, `doing`, `expression` and `retouch`. A name in a descriptive `who` resolves to that character. A name in any other field is refused plainly ("John is named in `doing`: add him to people"), because drawing him from words makes a stranger (E1).
- **`wearing`, `doing` and `expression` are prose** (§2.8). `wearing` and `doing` are required for anyone the call introduces. `expression` is optional and separate from `doing`: an expression changes the face and is retouched, while a pose redraws the scene (§5.2).
- **The scene is recorded** for every picture drawn: per chat, outside the jail, in a store the harness writes (§6). That covers the assistant chat as well as personas, so there is one path, not a scene path and a no-scene path. Each field carries its origin (clean or untrusted), as `scene.rs` does now.

## 5. The one operation: draw a scene, or change one

### 5.1 The model-facing call

```
image_generate {
  picture:  path        // the one picture being changed
  scene:    {...}       // a whole scene for a new picture, or the fields that change;
                        // scene.setting may be a photo: { photo: path } ("put her in this room")
  retouch:  "..."       // one small change to the picture itself: an object, a colour, a detail
  mask:     path        // the Edit modal's painted area, for a retouch
  size:     square | landscape | portrait
}
```

- **Retired inputs:** `prompt`, `cast`, `extras`, `edit` (`change`, `keep`, `face`, `camera`), `negative_prompt`, multi-picture `reference_images`, and `seed` (review S1; kept for the CLI and evals, off the chat schemas).
- **A photo's role is said by the call, never guessed.** `picture` is always the picture being changed ("this is her, make her smile"). `scene.setting: { photo }` is a room to put people in ("put her in this room"). Both of today's chats opened with this ambiguity. A `picture` with no record is therefore always the current picture (reviews B3 and N1). Faces cannot decide: RetinaFace found no face in 16 of 106 persona pictures, every one of which shows her from behind, bent over or from the waist down. So an old picture, an assistant picture carried over, or a photo of her made outside mecha (use case 11) is never treated as an empty room.
- **A picture's people can be unknown** (review N2). A no-record picture starts a record whose people are the ones the call declares. If the call declares nobody (a retouch, say), its people are recorded as **unknown**, never as none. A change that would redraw everyone (a restage) on a scene whose people are unknown is drawn as an edit of the picture instead, and the result says the scene's people are not known yet. Once a call declares them, the record knows them.
- **The model never writes the image model's prompt.** The compiler does, in the measured forms (§2.5, E1–E12).
- **Equal is unchanged, and all-equal is a redraw.** On a picture with a record, a scene field equal to its recorded value is not a change, so the model's habit of restating everyone costs nothing. This retires both the `cast` versus `scene.people` overlap and the refusals between them. But the record holds what was asked, not what was drawn. A call that changes nothing at all ("she's still standing", restating `doing: "sitting"`) is therefore a **redraw** of the scene at a new seed, never "no change" and never a refusal (review B4). Era-A panel re-calls were mostly this kind of second attempt: 14 of 41.
- **One retouch, in words.** It is the only free text that addresses the picture itself. The tool writes the #408 keep form around it.

### 5.2 What each call becomes

The planner reads the call against the picture's record and picks one render. Nothing rewrites the call into another shape.

| Call | Render |
|---|---|
| no `picture`; a whole scene | **New picture.** Library portraits, the E1–E12 compile, the call's seed or a new one. |
| `scene.setting` is a photo; a scene with people | **Place on the photo.** An edit of the photo with each person's head crop and description (M1). The photo becomes the scene's setting. |
| a `picture` with no record | **The current picture.** Edited as any recorded picture is, with its record started from the call; its people are unknown until a call declares them, and a restage of unknown people is drawn as an edit (reviews B3, N1, N2). |
| a change to someone's pose, the camera, the setting, or a removal | **Restage.** Everyone drawn afresh on the scene's setting, at the base picture's seed (§2.6): an edit of the setting photo with crops when the setting is a photo, a new picture from the setting's words when it is words. |
| a change to someone's clothes, someone's `expression`, or someone added | **Edit of the picture,** with the crops of the people it changes, in the #408 keep form. Clothes come back unchanged elsewhere 100% (correct for this case). An expression is a face retouch, not a restage (review S2). |
| nothing changed at all | **Redraw** of the scene at a new seed (review B4); the same render as Regenerate (§5.4). |
| `retouch` (with or without `mask`) | **Retouch** of the picture, in the #408 keep form; masked as today. |

**Budgets stay where they were measured:**
- an edit-shaped render carries the canvas plus two face crops (`EDIT_REFERENCE_BUDGET` 3);
- a new picture carries up to `MAX_CAST` 4 portraits;
- a scene keeps up to 8 people.

A change that does not fit falls back before it refuses (review S4).
- A restage that needs more crops than an edit holds, on a words setting, is drawn as a new picture from the words, with up to `MAX_CAST` portraits, at the base seed.
- On a photo setting there is no measured fallback yet. The portrait with the room as material (M2's "RN") was rated "okay, the most unnatural faces". That is a measurement owed before it ships, and until then it is refused in the scene's own terms, naming the people.
- Only a scene with more than `MAX_CAST` people with faces is always refused.
- A person without a library entry costs no budget; they are drawn from words.

**Seeds:** the model does not send seeds (§5.1). An edit-shaped render samples fresh (#306). A restage reuses the base picture's seed. A redraw takes a new one. The seed actually drawn is recorded, so a redraw always differs.

### 5.3 The edit panel: the harness extracts, the persona replies

A panel press is the owner's instruction to the image model, not something said to the persona (`persona/edit.rs`'s own doc). So:

0. **The persona's safety check runs first,** the same `safety::keyword_hit` crisis check a persona turn meets before any model (review S6). A panel press is never a door around it.
1. **The owner's words plus the picture's record go through a one-shot extraction.** It has no tools and no history, and is constrained to the scene-change schema: `scene` fields plus an optional `retouch`, with **no `kind` discriminator** (§2.7: clothes need an obvious home, which is `people[].wearing`).
2. **The extracted call is dispatched through `Agent::dispatch_one`** (#592). It goes through every gate a model call meets, with its own call id, inline, on the chat's job seat, with taint recorded.
3. **The history records one fact,** a `HarnessPicture` record: "Picture X was changed into Y: <the typed change>". The card shows the new version from it.
4. **The persona replies in a line or two,** in its own voice. That reply goes with `tool_choice: "none"`, not with the tools removed: removing them re-sends the whole context (12,277 tokens against 4 measured), while `tool_choice` keeps the cache (review S5). It carries no edit note, and G3's calls-as-text check covers it.
5. **"Try again" is a redraw, not a failure** (review N3). An extraction that comes back empty, or restates the record, on a recorded picture means the owner wants another attempt. It is drawn as a redraw at a new seed (§5.1), and the card says it was drawn again.
6. **An extraction that fails is said, never dropped and never retried in a loop** (review S7). Only these are failures: a 400, unparsable or schema-invalid output, or a name the library does not hold. The card says the edit was not understood and why, in a line, and nothing is drawn. G1 counts it.

This removes the panel loop at its source (§2.9), because the persona never makes the call. It also removes the model's field choice for the turns where it chose wrong. Typed requests in chat ("draw us at the beach") still go through the persona model, with the one-shape schema (§5.1).

### 5.4 Regenerate (R8)

The same scene, a new seed, the same render plan. It goes through `dispatch_one` and is recorded as a `HarnessPicture` with `how: redraw`. The card shows versions ‹ 1/2 ›, and the version showing is the one Edit and Regenerate build on. R8's ruling stands. R8-2's call reconstruction (#593) is not needed, because the scene is the record. #593 is closed unmerged.

### 5.5 A run makes at most one picture

Two rules (review B1).

- **Structural, in every chat.** A deferred job of a tool already started in a run is never started again in that run. The agent loop enforces this on the tool's job kind, not the model, and not a refusal string. The assistant keeps working after drawing (it went on to `image_view`, `shell` or `fs_read` in 43 runs), so its run is not ended.
- **The clean end fires on the first repeat call** to that tool. The repeat is not run, and the next request goes with `tool_choice: "none"`, the turn's note once, and "the picture is on its way; answer in a line". That combination was measured clean 5/6. In a persona chat it fires right after the picture is queued, because nothing after the picture is the persona's job.
- **The backstop:** a reply that is only a tool-call block is replaced, never shown.

**Retired:** `REPEAT_REFUSED` and `REPEAT_IN_FLIGHT`, which never fired in any session.

**LoopGuard stays.** It is generic, not an image guard. Its blind spot is that a harness refusal never counts toward its limit (`denied: out.refusal`). That is fixed in the guard for every tool.

## 6. The record

- **Where:** one scene store per chat, outside the jail, harness-written.
  - **Persona chats** also write the persona's latest and the content-hash index, so a picture carried into another chat is found by its bytes (R3, R7).
  - **Incognito** keeps its copy in the room and never writes back (R9).
- **What:** the scene, with origins per field. The place is the setting only (§4). Plus the picture's hash, its seed, size, render plan and the library versions used.
- **The scene note** (`scene::note` in each persona run's notes) stays, for typed turns, which still have the persona choosing the call. Its stems and origin rules stand. With the setting recorded as the setting only, it no longer carries an old pose, and G3 covers it (review S8).
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
| `cast_self`'s duplicate and possessive refusals (~380 lines) | `self` is a `who` value, resolved from the persona's names as before (§4). Only the name resolution is kept. |
| the name guard's retry skeleton and "split the scene" (~160 lines) | replaced by the plain prose-field check of §4: a name resolves in `who` and is refused elsewhere |
| the foreign-scene placeholders and blanking | the manifest no longer carries a scene (§6) |
| near-copy advice, strikes and library-redraw advice | restage replaces the chains they were warning about. The layout number stays as a fact in an edit-shaped render's result line, stated without advice (review Q1). |
| `same_layout_as`/`original_of` | the one-hop lineage is unneeded under restage |
| `REPEAT_REFUSED`/`REPEAT_IN_FLIGHT` | never fired (0 in all sessions); replaced by §5.5's structural rule |
| model-sent `seed` | 177 of 211 copied an earlier result's seed, the "same picture again" habit (§3) |
| `demote_unknown` | a non-library `who` is drawn from words directly |
| the panel note telling the persona which field to use | §5.3 |

Estimated net: roughly 1,700–2,000 lines of `imagegen.rs` deleted, with the tests that measured the retired seams. The remaining tests are rewritten against the one shape.

## 9. Acceptance gates

mecha-a3 runs these on each implementation branch, on the branch's own tool surface and notes. A failure on any gate blocks the merge.

| Gate | Pass | Proposed threshold |
|---|---|---|
| **G1 panel extraction correctness** (replayed real panel turns plus controls: clothes, expression, camera, object, add someone, owner photo as setting) | the typed change is right; failures said on the card | ≥ 90%, and **0** refusals for shape |
| **G1b typed turns** (replayed typed picture turns through the new schema) | the call the persona makes is right | ≥ 90%, and **0** refusals for shape |
| **G2 the wanted call still happens** | request 1 of every replayed picture turn calls the tool; no reply narrates a picture without one | ≥ 95% |
| **G3 one picture per run** | at most one started, structurally; 0 calls written as text; 0 empty replies | 100% |
| **G4 renders, the owner's eye** (face-sized labelled sheets, ≥ 3 seeds each) | restage pose right, room held across restages, identity holds; words setting and photo setting | owner's verdict |
| **G5 after deploy** (first real chats) | pose and camera changes go through restage; restages pass the owner's eye; edit-shaped renders that should move something (adding a person, placing on a photo) are not near-copies; no runaways | restage share ≥ 90%; edit-shaped near-copy ≤ 30%; runaways 0 |

## 10. Build order

Each PR goes through its review loop, then a3's gates, then the owner's merge word.

1. **The scene schema and the planner, with the retired inputs deleted.** This covers the record per chat (§6), equal-is-unchanged, place as the setting only, seed reuse, and the refusals in scene terms. It is the big one.
2. **One picture per run** (§5.5).
3. **The panel through extraction plus `dispatch_one`,** and the `HarnessPicture` record (§5.3).
4. **Regenerate** on the same record (§5.4).
5. **The web:** versions on the card, and Edit and Regenerate on the version showing.

Steps 1 and 2 are independent and can run in parallel lanes.

## 11. Questions for the owner

1. **Record the scene for the assistant chat too** (§6)? **Ruled 2026-10-07: yes, one path.**
2. **Prose fields: `setting`, `light`, `camera`, and per person `wearing`, `doing`, `expression`** (§4)? `light` covers mood (79% of what prompts carried beyond people), `camera` covers framing (11%), and quoted text goes in `setting`.
3. **Close #593 unmerged** (§5.4)? **Ruled 2026-10-07: closed.**
4. **The thresholds in §9.**
5. **Crop or whole portrait on new pictures** (§2.1)? **Ruled 2026-10-07: the whole portrait until §8.1's crop measurement passes on the owner's sheets**, as for R1.
6. **Seeds off the chat schemas** (§5.1)? **Ruled 2026-10-07: off;** every seed reuse that helps is the harness's.

## 12. Review and how each point is met

mecha-a3's review (local `REVIEW-IMAGE-DESIGN.md`, with new measurements in the evidence file's §F) raised 4 blocking findings, 9 shoulds and 5 questions in round 1, and 1 blocking finding and 2 shoulds in round 2. Each is met above:

| Point | Met in |
|---|---|
| B1: the forced end breaks the assistant; LoopGuard is generic | §5.5, split into two rules; the guard's refusal blind spot fixed for every tool |
| B2: retiring the name guard reopens E1 through prose fields | §4, every prose field checked; `who` resolves names and the persona's names |
| B3, N1: a no-record picture is not an empty room, and faces cannot tell | §5.1 and §5.2: the call says a photo's role; a no-record `picture` is always the current picture |
| N2: no people is not the same as people unknown | §5.1: unknown people are recorded as unknown; a restage of them is drawn as an edit |
| N3: an empty or restated extraction is "try again" | §5.3 step 5: drawn as a redraw |
| B4: equal-is-unchanged makes "try again" a no-op | §5.1 and §5.2: an all-equal call is a redraw at a new seed |
| S1: model-sent seeds copy earlier ones | §5.1 and §8, seeds off the chat schemas |
| S2: `doing` conflates pose and expression | §4 and §5.2: `expression` per person, retouched |
| S3: use case 4 dropped | §4: `who: { from: picture }` |
| S4: budget refusals are round trips | §5.2: fall back before refusing; the photo-setting fallback is owed a measurement |
| S5: `tool_choice` for the reply | §5.3 step 4 |
| S6: the safety check on the panel path | §5.3 step 0 |
| S7: extraction failure | §5.3 step 5 |
| S8: the scene note | §6, kept for typed turns |
| S9: `place` is wrong for a logo | §4: `setting` covers the whole subject without people |
| Q1: near-copy as a fact | §8: the layout number stays in the result line |
| Q2: the assistant record | §11.1, with B3's classification |
| Q3: `light` | §4 and §11.2 |
| Q4: the G5 metric | §9 |
| Q5: a typed-turn arm | §9, G1b |
