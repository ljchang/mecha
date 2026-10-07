# Image scenes — design

**Status:** proposed 2026-10-07, waiting on the owner's rulings (§9). Nothing here is built.
It amends `IMAGE-COMPILER-DESIGN.md` §3, where `cast` and `reference_images` are exclusive,
and `PERSONA-CONTEXT-DESIGN.md` §5.5, where edits happen only on the owner's initiative. It
replaces the lineage rule of #569's face anchor. The measurements are in §4. The owner judged
them by eye on face-sized sheets; ArcFace is recorded beside each verdict as a backstop.

**The question:** a persona's pictures drift into a different person, a second library character
is drawn as a stranger, and camera moves are ignored. Why, and what design holds every use the
owner makes of pictures?

**The answer in one line:** whether a picture gets a library identity is decided by **the shape
of the call**, not by **who is in the picture**. Any call that passes a picture is an edit, and
an edit can never carry a library identity. The fix is that every call declares its people, and
every render goes back to the place rather than to the last picture.

Privacy: this document carries no conversation text, no persona names and no session ids. The
persona's linked character is **A**, and **B** is an invented library test entry. Figures are
counts. The renders and scripts live outside the repository.

---

## 1. Symptoms (2026-10-06)

Two persona chats on one afternoon, 57 `image_generate` calls between them:

- **0 of 57 used `cast`**, though the persona has a linked character.
- Edit chains reached depth 7. Earlier measurement put each edit at about 0.77 of its parent's
  identity, so depth 7 is a stranger.
- A second library character was asked for by name and drawn from words.
- 11 calls asked for a camera move in `edit.change`, and none of them set `edit.camera`.
- **#569's face anchor applied to 6 of the 69 persona edits** since it was deployed. The other
  63 record `face_anchor: null` with no reason. All 6 were in chains that began from a `cast`
  picture.
- Every chain that afternoon traced back to one picture from an earlier day. In it, character A
  was drawn into an attached photo from a written description, with no `cast` (layout
  similarity to the photo 0.22, so effectively a new scene). The face the owner later pointed
  at as "her" had never come from her portrait.

## 2. Root cause

**Any call with `reference_images` is an edit, and an edit is outside the compiler.** Two rules
in `ImageGenerate` follow from that:

1. **`cast` and `reference_images` are refused together** ("cannot be combined yet"). The
   reason given is that one reference size per call is all the encoder takes.
2. **The library-name guard (`imagelib::named_in`) skips edits**, with the comment "an edit's
   people carry their own identity".

That is true when the request is "change her pose in this picture of her". It is false for
everything else the owner passes a picture for:

- a background photo;
- a photo from outside;
- a picture carried over from another chat (each chat has its own workspace, so the picture
  arrives as a bare inbox file with no manifest);
- a picture that someone is being added to.

The fixes since then each treated one case:

- **#408** added near-copy notices.
- **#429** added masks.
- **#569** added the face anchor. It is the closest of the three, but because a call cannot
  say who is in it, `ImageGenerate::traces_to` has to infer the character from the manifest
  chain. That inference fails:
  - at a chat boundary;
  - at any edit with two references, which is ruled out on purpose;
  - at a chain that started from words.

  The anchor also drops its crop when `edit.camera` is set, a field the model never set.
- **#579** typed the edit fields.
- **`PERSONA-CONTEXT-DESIGN.md` §5.5** keeps the assumption: the picture the owner points at
  carries identity. On 10-06 it did not.

**Why the research missed it.** Every chain in the 10-05 drift study began from one scene drawn
from A's portrait. That is the one case `traces_to` can follow. No experiment started from a
background, a carried-over picture or a two-person composition.

**What the research had planned.** `IMAGE-COMPILER-RESEARCH.md` tier A chose references by role,
with "an edit takes the parent image as the canvas in slot 1" beside the identity references. It
also kept `SceneSpec` and a location library. The build shipped `cast` for new pictures and left
edits as raw changes to pixels. This design finishes what tier A described.

## 3. The model, briefly

From `IMAGE-COMPILER-RESEARCH.md` §2 and §8:

- Qwen-Image 2.1 takes a canvas in slot 1 and references after it, each bound by `<imageN>`.
- A 512² face crop costs 2–9 s.
- Four people held in one pass, with no bleed between them.
- Stated wardrobe, pose and expression override what a reference would otherwise copy.

The face anchor already sends a detector head crop (`face::anchor_crop`, 1.12 × the face box)
beside a full-size canvas in one call, so the encoder constraint behind rule 1 is already met in
production.

## 4. Evidence (2026-10-07)

Measured with invented prompts on the production graph (int8 UNet, w4a8 encoder, 40 steps): 87
renders. Each render was gated on no live voice call. Character A is drawn in a red sundress
throughout. The places were an owner photo of an empty room with furniture (P1) and a generated
living room (P2). **The owner's verdict decides; ArcFace (buffalo_l, against A's portrait) is
reported only as a backstop.** It disagreed with the owner more than once (below).

**Calibration.** A's real portrait shrunk to 50 px still scores 0.88 against itself, so low
scores at small sizes are real loss of identity, not measurement error. A's 35 single-person
production `cast` pictures average 0.67; 34 of them have faces over 160 px.

### M1: a person added to a place (the place as canvas)

| Identity source | ArcFace, 8 pairs | Owner, P1 |
|---|---|---|
| Words only (today) | 0.18 | 1 of 4 |
| Head crop | 0.39, higher than words in 8/8 | 1 of 4 |
| **Head crop + library description** | 0.39 | **all ok** |

The room, camera and framing were kept in every case (layout 0.84–0.97). ArcFace could not see
the description's effect; the owner could.

### M2: four changes that move the layout (pose, low camera, back view, high camera)

| Path | ArcFace at steps 1 / 2 / 4 | Camera moved | Owner |
|---|---|---|---|
| E: edit chain (today) | 0.12 / 0.17 / −0.03 | 0 of 2 | not her |
| EA: edit chain + anchor | 0.33 / 0.20 / −0.01 | 0 of 2 | drifts like E |
| **R: restage on the place + head crop** | 0.43 / 0.60 / 0.20 | low angle yes | **pretty good** |
| RN: new picture, full portrait, place as material | 0.22 / 0.69 / 0.41 | low angle yes | okay, the most unnatural faces |

Chains changed pose but never the camera. No path produced a clearly high angle from the
wording used.

### M3: two library people

- **Touching, in one new picture with `cast`** (slow dance, hug, hand in hand, head on a
  shoulder; 2 seeds each): 8 of 8 drawn correctly, both people exactly once, no bleed (the
  highest wrong-person score was 0.18). B scored 0.83–0.90; A 0.24–0.82, lowest with her head
  turned. Owner: good.
- **Adding B to an existing picture of A** (A's picture as the canvas, both head crops): B
  lands and A is kept. B's identity was mixed (0.55 and 0.29 with crops, against 0.15 and 0.28
  from words). Two seeds only.

### M4: three levers

- **A portrait drawn by the model itself** (best of 6, 0.78 to the original): no gain on any
  path (place 0.39 → 0.31, two people 0.48 → 0.47). Dropped. The owner judged the drawn
  portraits to look like A.
- **1536 instead of 1024**: ArcFace went from 0.28 / 0.60 to 0.73 / 0.68. To the owner, the
  seed mattered more: the best and the worst both include a 1536. It costs 256 s against 54 s.
  Dropped.
- **A feature stated in text** (a forearm tattoo): present 4 of 4. But "floral" leaked into the
  dress in 4 of 4. Wording of this kind belongs in the library description, written once.

**Two other findings:**

- **The seed is the largest single source of variance** after the path itself: up to 0.32
  between two seeds of the same call.
- **The full-body portrait leaks.** It carries her identity well in new pictures, but it brought
  its outfit and pose into 4 of 6 scenes on 10-05, and a covering outfit hid that here.

## 5. Design

### 5.1 The scene is the record

A **scene** is a typed record:

- **place:** an owner photo, a library location, or words;
- **people:** each a library character, "the person in picture X", or an extra described in
  words, with what they are wearing, what they are doing, and where they are;
- **camera:** framing, height, angle, facing;
- **light and style.**

The harness keeps the **current scene per persona**, in the persona store, not in a chat's
workspace. Every manifest records its scene and the content hash of its picture. A picture
attached in any chat is matched by its bytes, so a carried-over picture resolves to its scene.

### 5.2 People are declared on every call

- **Each library person in a render enters as their detector head crop plus their library
  description, on every call, whatever the canvas.** The crop is the anchor's existing crop. The
  description is the entry's text, verbatim: build, and features such as a tattoo, worded so
  they cannot leak (§4, M4).
- The library-name guard runs on every call, edits included.
- A person from an owner photo who is not in the library is declared as "the person in picture
  X", which uses their crop from that picture. The model can propose them for the library
  through the existing candidate path.
- The persona's own character is added by default, as `cast_self` does today for new pictures.

### 5.3 The canvas follows the change, and nothing chains

| Change | Canvas (slot 1) | Then |
|---|---|---|
| Clothing, expression, hair, an object, a painted region | the current picture | crops and descriptions follow it |
| Pose, camera, adding or removing a person | **the place** | the people are drawn fresh from crops and descriptions |
| A new scene | none | the place as material, or words |

The code chooses the canvas from **which scene fields changed**, never from a flag the model sets.
A restage always starts from the place, never from the last output, so identity is one step from
the library on every render. A retouch of a retouch is allowed, because each one carries the
crops.

### 5.4 The model writes scene changes, the compiler writes the prompt

- The model's call is a change to the scene: who, wearing, doing, place, camera.
- Everything not changed is kept, and the compiler words it, so keep and change cannot
  contradict each other.
- Camera wording comes from a closed vocabulary that the research measured working, for example
  the physical wording for a low angle. High angles are still open (§8).
- The owner's Edit panel sends its words as the change. The model turns them into fields.
- **The types follow the design** (owner, 2026-10-07: "we can update types with our new design
  if needed"). #579's typed edit (`edit.change`, `keep`, `face`, `camera`) becomes the scene
  change itself, not a second shape beside it. The manifest gains the scene.

### 5.5 The scene is the story's present

The current scene goes into the run's notes (`PERSONA-CONTEXT-DESIGN.md` §5.1): what A is
wearing and where. A recalled day's outfit or blocking then cannot stand in for the present one.

### 5.6 Scenes and background jobs

This is built on #583 (pictures as jobs, `PERSONA-CONTEXT-DESIGN.md` §5.4). The notes below were
checked against #583's branch on 2026-10-07; check them again against the tree once it merges.

- **The scene advances when a render lands, never when it is asked for.** The write happens
  inside the job, or in the host's `late::land`, so a cancelled or failed render leaves the scene
  where it was. Steps 1–3 of §10 sit inside #583's `let job = async move { … }`, after
  `backend.generate`, where the face anchor and `write_manifest` already run.
- **Two chats with one persona can render at once.** Jobs are one in flight per chat, and the
  scene is kept per persona (§5.1). Proposed: each chat works on its own copy of the scene, taken
  from the persona's latest when the chat starts. A landing writes back to the persona's latest,
  and the last render to land wins. That is ruling R7.
- **The scene note goes in the run notes' tail** (`cx.notes`, #577), beside #583's note about a
  picture that arrived late. It never goes in the cached head: the scene changes from turn to
  turn, and the head is the cached prefix.
- **Manifests keep `tool_use_id`.** `imagegen::repair_orphan` finds a render that landed after a
  restart by that key. A manifest that gains a scene keeps the key, or `repair_orphan` changes
  with it.

## 6. Use cases

| # | Use | Canvas | People |
|---|---|---|---|
| 1 | New picture of the persona | none | A's crop + description |
| 2 | The persona in an owner photo | the photo | A (M1) |
| 3 | With another library character | place or none | both crops + descriptions; touching per M3 |
| 4 | With someone from an owner photo | place | A + "the person in picture X" |
| 5 | Clothing, expression or hair | current picture | crops ride along |
| 6 | Pose | place | restage (M2, R) |
| 7 | Camera move | place | restage; wording from §5.4 |
| 8 | A painted region (the Edit modal) | current picture + mask | crops ride along |
| 9 | One small change, the rest held | as the change requires | the unchanged fields are kept by the scene |
| 10 | A picture from another chat | resolved by content hash | from its scene |
| 11 | A picture of the persona made outside mecha | the photo | declared by the owner's words |
| 12 | An owner photo with no library people | the photo | none: a plain edit |
| 13 | A persona with no linked character | as above | words, as today; linking a character fixes it |
| 14 | A library style | any | any |
| 15 | A picture during a call (§5.4 jobs) | any | the scene is data a job can carry |
| 16 | Variations | the same scene | new seeds |

Five or more people, or close physical interaction beyond M3's four, may need the research's tier
C. That is switched on per case.

## 7. What this replaces

- `ImageGenerate::traces_to`, and `face_anchor`'s silent null. The anchor's crop becomes how
  every library person enters every call.
- `edit.camera` as the switch for the anchor.
- The refusal of `cast` beside `reference_images`.
- The near-copy recovery advice, already reduced by #581. A restage is not an edit of the last
  picture.
- `PERSONA-CONTEXT-DESIGN.md` §5.5 keeps its rule (edit only what the owner points at) and loses
  its assumption: what the owner points at chooses the scene and the retouch canvas, and the
  crops always come along.

## 8. Open, measured during the build

Each is judged by the owner on face-sized, labelled sheets. ArcFace only flags gross drift.

1. Two people in one new picture **with head crops** in place of full portraits. M3 used full
   portraits.
2. Wording that produces a high camera angle.
3. Why the seated, facing-the-camera placement in P1 failed for crop alone and for words, but
   held with the description.
4. Seed variance: whether to draw two and let the owner keep one, at twice the time.
5. A location library entry beside owner photos (`IMAGE-COMPILER-DESIGN.md` §1 left locations as
   free text until measured).

## 9. Rulings asked of the owner

- **R1.** Identity is declared: every library person enters every render as head crop plus
  description (§5.2).
- **R2.** Pose, camera and people changes restage from the place, and nothing chains (§5.3).
- **R3.** The current scene lives with the persona across chats, and pictures resolve by content
  hash (§5.1).
- **R4.** §5.5 is amended as in §7.
- **R5.** #569's lineage path is retired rather than kept beside this.
- **R6.** The owner's eye on face-sized sheets is the gate for every step. ArcFace is a backstop.
- **R7.** With two chats rendering for one persona, each chat works on its own copy of the scene,
  and the last render to land updates the persona's latest (§5.6).

## 10. Build order

All of it after #583 (jobs) and #577 (run notes in a head and a tail) merge, since §5.6 builds
inside both.

1. Declared identity: lift the `cast` + references refusal, crops and descriptions on every
   call, the name guard on edits, and `face_anchor` recording why it was not applied.
2. The canvas rule and restage, with the scene change as the call's type, replacing the typed
   edit.
3. The scene record per persona, manifests carrying scenes, and lookup by content hash.
4. The scene in run notes.
5. §8's measurements, each before the step that depends on it.
