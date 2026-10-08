# Image scenes — design

**Superseded 2026-10-07 by [`IMAGE-DESIGN.md`](IMAGE-DESIGN.md)** for the mechanism (the call, the routing and the record). The measurements here (§4), the owner's rulings R1–R9 (§9), and the open crop-on-new-pictures measurement (§8.1) stand, and `IMAGE-DESIGN.md` cites them.

**Status:** accepted 2026-10-07: the owner ruled R1–R9 (§9). Steps 1–4 were built (#584–#591,
2026-10-07) and their mechanism then superseded by `IMAGE-DESIGN.md` (see the note above).
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
production. It is met only in that one shape. `comfy_graph` takes one `reference_size` per call;
the `cast` path replaces `req.references` and forces `imagelib::REFERENCE_SIZE` (512²); and
`MAX_REFERENCES` (4) counts the pictures the model passes. §5.2 states the resolution and slot rule
that lifting rule 1 needs.

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

Step 3, the back view, has no face to score; all four paths drew it. Chains changed pose but
never the camera. No path produced a clearly high angle from the
wording used. Step 4 is that high-angle step, with the face tilted against the table. R's 0.20
there is read as the wording failing (§8.2), not the restage path: the owner rated R pretty good
across its steps.

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
  path (place 0.39 → 0.31, two people 0.48 → 0.47). The portraits themselves looked like A to the
  owner, but the scenes drawn from them were no closer to her, so the lever is dropped.
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
**The assistant's own chats have no scene store.** Scenes are per persona. In the assistant
chat, §5.3's canvas rule and §5.7's redraw read the picture's own manifest within its chat,
under the same fail-closed reading, and nothing crosses chats.

**The lookup, in one rule: a picture's bytes are looked up only in a harness-written index in the
persona store, and nowhere else.** The harness writes an entry there when a render in one of the
owner's own sessions lands, keyed by the picture's content hash and pointing at the scene it
wrote. The bytes are only the key; the security is in where the index lives:
- **The model cannot write it.** A workspace manifest sits in the chat's jail, where a run can
  write one with `fs_write` (`repair_orphan`'s comment grants this), so cross-chat lookup never
  reads workspace manifests at all.
- **A manifest's stated origin is never trusted:** within its own chat it is read only as
  fail-closed evidence, never as a clean label.
- **The front door never looks anything up:** an outside sender's bytes must not pull a scene out
  of the persona store.

**A scene carries its origin.** Scene text is model-written, so it crosses from one chat's run
into another chat's notes. Each write records the taint of the run that made it, classified
from the transcript's recorded taint the way a library candidate's `imagelib::Origin` is, and
failing closed: unknown classifies untrusted. **Origin is kept per field, and a scene's origin is
the union over its fields.** Each field records the origin of the write that last set it. §5.4
keeps unchanged fields, so a clean write over a scene with one untrusted field leaves that field,
and the scene, untrusted. Only a field the clean write replaced takes the clean origin. A single
origin per write would satisfy the words and launder the field. A chat that reads a scene written under untrusted
input takes that taint on, as a `mailbox.rs` message carries its sender's.

The carrier into the notes is a stem. Notes arm taint only by stem match (`Taint::arm_for_notes`),
and a stem is text, so one stem would arm every scene alike. **There are two scene stems, one
for a clean origin and one for an untrusted one**, as memory has two (`persona::recall::stem_of`;
the comment on `arm_for_content` says why). The note opens with the stem its record's origin
picks. **Both stems arm `private`, as memory's do.** A scene can carry an owner photo, or a person
out of one, into a chat that never saw it, which is exactly the case `private` exists for. So a
clean scene and one page from outside still meet the interlock. A scene record with no origin, older than
this rule or hand-made, reads as untrusted, never as the default; this is the second half of
mailbox's `taint_recorded`.

**A picture whose bytes match no record names nobody,** and the call has to declare its people
(§5.2). The manifest records that no scene was found for it. It never falls through silently:
that silence is §1's `face_anchor: null` again.

**An incognito chat writes no scene outside its own folder.** This follows
`INCOGNITO-DESIGN.md` R3 (no writes outside the chat's own folder) and R6 (images deleted when
the session closes).

- It reads the persona's latest scene when it starts. That is a read, which R3 allows.
- It keeps its copy in its own folder, where it is deleted on close.
- It never writes back, and its pictures are not indexed by content hash outside the chat.

This is ruling R9 (owner, 2026-10-07).

### 5.2 People are declared on every call

- **Each library person in a render enters as their detector head crop plus their library
  description, on every call, whatever the canvas, except a masked edit** (§7). Where no crop
  can be had (no detector, no face in the portrait, past the budget), a declared person still
  enters as their description, clothes and pose in words; only the face reference is missing,
  and the result line says so. A masked edit
  still runs the name guard; declaring the person satisfies it, and their crop is simply not
  sent. The crop is the anchor's existing crop. The
  description is the entry's text, verbatim: build, and features such as a tattoo, worded so
  they cannot leak (§4, M4).
- The library-name guard runs on every call, edits included. **`"cast": []` is the one waiver,
  and it is explicit and recorded:** it means someone else by that name, it skips the self-cast
  and the guard, and the manifest's `identity` says "`cast` was empty". It is the model's
  deliberate statement, not a shape the call happens to have.
- A person from an owner photo who is not in the library is declared as "the person in picture
  X", which uses their crop from that picture. Picture X itself is not sent to the generator;
  only the crop is. Picture X passes the same test as an edit's canvas (`PERSONA-CONTEXT-DESIGN.md`
  §5.5): the owner pointed at it, by a typed reference in the turn being answered. It also goes
  through `ToolCtx::resolve` before any crop is read. The model can propose them for the library
  through the existing candidate path.
- The persona's own character comes in when something names her. **The picture's record**
  carries her over. **Her name in the edit's words**, on a picture that does not record her,
  brings her in when those words say what she wears and does. Otherwise the call is refused and
  asks for her in `cast`, because she is new to that picture and stand-in clothes would be
  invented ones; that costs use case 2 one round trip.
- **A call's `cast` adds to a picture's record and never erases it.** Removing one person from
  the record waits for the scene record (§10 step 2). Until then `"cast": []` resets the whole
  record when the recorded people have left the picture.
  An owner photo with no record and no mention of her names nobody (§6, case 12).

**Resolution and slots.** One call has one reference size.

- **With a canvas,** every crop is encoded at the canvas's size (`EDIT_REFERENCE_SIZE`, 1024),
  as the anchor does today. That is how M1, M2's R and M3's add-a-person were measured. Times:
  canvas plus one crop, 52–54 s; canvas plus two crops, 66 s, against 40 s with none.
- **With no canvas,** references go at 512², as `cast` does today.
- **One budget for the whole call, computed from decoded sizes**, never from a role the call
  declares (`IMAGE-COMPILER-RESEARCH.md` §10, item 3). Everything that goes in, model-passed
  pictures and harness crops alike, is counted at the size it will be encoded at. The measured
  ceiling is **three references at 1024², canvas included**: canvas plus one crop took 52–54 s;
  canvas plus two crops took 66 s. Four full-size references are the research's cliff (+120 s).
  **Crops never take a call past the budget. Pictures the model passes itself keep
  `MAX_REFERENCES` (4), as today:** four plain references with nobody declared draw, slowly, as
  they always did, and the budget only governs what the harness adds. **In short, what is
  refused: people the call names whose crops would not fit. What is only slow: four pictures the
  model passed itself.** People carried over from a picture's record are trimmed to the budget,
  never refused. `identity` and the result line the model reads both say who went without a crop
  (as step 1 builds it). **A call whose crops would exceed it is refused
  before the GPU and says why, naming the ways out: fewer people, fewer pictures, or the place in
  words.** The model cannot reach those by sending the same call again. **Nothing
  falls back to another shape silently.** Drawing the scene without the place's picture is a different call, one the
  model makes on purpose, and its result says the room was redrawn: R2's canvas is what keeps an
  owner photo's room (M1, layout 0.84–0.97). That call goes at 512²; E3's four people held there
  as whole portraits standing apart, and touching is measured for two (M3). A third crop beside a
  canvas is a measurement (§8). The cap is therefore a function of the encode size, not a larger constant
  beside `MAX_REFERENCES`.

**New pictures keep the whole portrait until §8.1 passes.** R1's crop is measured only on paths
with a canvas: M1, M2's R, and M3's add-a-person. New pictures still send the whole portrait at
512² (`IMAGE-COMPILER-DESIGN.md` §1, E10), and M3 drew its touching pairs that way, rated good.
Each path switches to the crop when §8.1 passes on the owner's sheets for that path, and not
before.

### 5.3 The canvas follows the change, and nothing chains

| Change | Canvas (slot 1) | Then |
|---|---|---|
| Clothing, expression, hair, an object, a painted region | the current picture | crops and descriptions follow it |
| Pose, camera, removing a person | **the place's picture**, when the place is a picture | the people are drawn fresh from crops and descriptions |
| Adding a person | the current picture, everyone in it carried over, the new person's crop beside it (M3's add-a-person, mixed at two seeds) | the place's picture instead is unmeasured (§8) |
| The same, when the place is words | none | a new picture at 512², the M3 shape (the whole portrait until §8.1 passes) |
| A new scene | none | the place as material, or words |

The code chooses the canvas from **which scene fields changed**, never from a flag the model sets.
A restage always starts from the place, never from the last output, so identity is one step from
the library on every render. Words are the usual place until locations are built (§8.5), so the
canvas-less row is the common restage today. It is also the shape M3 measured and the owner rated
good.

A retouch builds on the current picture, and **whether a chain of retouches holds identity is
unmeasured** (§8). The nearest data point goes the other way: M2's EA, a chain with the crop
riding along, fell to −0.01. EA moved the layout and a retouch does not, so the result may not
carry over, but until it is measured a long retouch chain is not assumed safe. Each manifest
records its **retouch depth** (the picture it built on, plus one; zero for a render from the
place), so §8.7 can be read from real use as well as from a fresh sheet. Before step 3 (the canvas rule) ships,
the depth also reaches the result line ("the third retouch of …"). Whether a cap holds until
§8.7 is measured is decided then; §1's failure was a depth-7 chain.

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

The **chat's own copy** of the scene (R7), never the persona's latest, goes into the run's notes
(`PERSONA-CONTEXT-DESIGN.md` §5.1): what A is
wearing and where. A recalled day's outfit or blocking then cannot stand in for the present one.

### 5.6 Scenes and background jobs

This is built on #583 (pictures as jobs, `PERSONA-CONTEXT-DESIGN.md` §5.4). The notes below were
checked against #583's branch on 2026-10-07; check them again against the tree once it merges.

- **The scene advances when a render lands, never when it is asked for.** The write happens
  inside the job, or in the host's `late::land`, so a cancelled or failed render leaves the scene
  where it was. **Identity is not in the job:** crops are references, and a reference has to be
  in the request before `backend.generate`. So §10 step 1 runs in `call` before the split, where
  #569's anchor ran. Only the **scene write** belongs after the split, inside #583's
  `let job = async move { … }` beside `write_manifest`. Not in `late::land`, which returns early
  on a rolled-back run and would leave a picture that really landed one scene behind (review of
  #584, final pass).
- **Two chats with one persona can render at once.** Jobs are one in flight per chat, and the
  scene is kept per persona (§5.1). Each chat works on its own copy of the scene, taken
  from the persona's latest when the chat starts. A landing writes back to the persona's latest,
  and the last render to land wins. That is ruling R7, and an incognito chat never writes back
  (R9).
- **The scene note goes in the run notes' tail** (`cx.notes`, #577), beside #583's note about a
  picture that arrived late. It never goes in the cached head: the scene changes from turn to
  turn, and the head is the cached prefix.
- **Manifests keep `tool_use_id`.** `imagegen::repair_orphan` finds a render that landed after a
  restart by that key. A manifest that gains a scene keeps the key, or `repair_orphan` changes
  with it.

### 5.7 Regenerate

There is a **Regenerate** button beside Edit on every picture card: in the assistant chat, in the
persona chat, and on a call. It is the owner's answer to seed variance (§4: up to 0.32 between
two seeds of one call). A second draw is paid for only when the first misses, never on every
picture.

- **The harness redraws the picture's recorded scene with a new seed.** No model turn is
  involved: no loop, no reasoning, one render. A retouch regenerates as itself, which is the same
  change on the same canvas with a new seed. **The redraw is recompiled from the scene through
  the compiler, never replayed from the manifest's compiled prompt.** That prompt sits in a file
  a run can write (§5.1). Recompiling runs the pre-GPU checks again: `cast_self` (now
  `picture::parse`'s `who` resolution), the name guard,
  unknown names, `MAX_CAST` and the budget. The library re-check covers only the approval half.
- **Identity follows R1**, because the scene names its people. The live library is checked
  again on every redraw: each entry has to be approved now, as on every other identity path, not
  merely when the picture was first drawn. A redraw whose person is no longer approved says so
  and does not draw.
- **The chat history records the redraw as a fact.** Picture X was redrawn as Y; nothing tells
  the model what to do about it. The persona then knows which picture is current, because the
  history is what happened (`PERSONA-CONTEXT-DESIGN.md` §5.3).
- **On a call it runs as a job** (#583). It meets #583's one-job-per-chat rule like any picture:
  while another is being made, it is refused with `BUSY`. **A redraw has its own call id, never
  the source picture's `tool_use_id`:** `repair_orphan` accepts a picture as an orphaned call's by
  that key, and a reused id would let it take a redraw for the original.
- **A redraw passes the same gates as any tool call.** The interlock, the `pre_tool` hooks and
  the approver live in the agent loop's dispatch, not in the `Registry`, which only resolves
  tools. That dispatch is private to `Agent::run_tools`, and it takes its calls from the model's
  `tool_use` blocks, so a button press with no model turn cannot reach it. **The build factors the
  gate sequence (interlock, then `pre_tool`, then the approver) out of `run_tools` into one
  function that `run_tools` and the redraw both call.** It is not copied (two copies drift), and
  no assistant turn is forged to carry the call. Calling the tool directly would skip all three. The interlock is moot
  (`Capabilities::default()`), and the library re-check below is in addition to those gates, not
  instead of them.
- **The new version shows on the same card** (‹ 1/2 ›), and the version showing is the current
  one (owner, 2026-10-07). The other versions stay on the card, one swipe away. Edit, a retouch,
  works on the version showing, as does Regenerate. **A restage takes the showing version's
  scene** (who is in it, what they wear) and starts from the place, never from its pixels
  (§5.3).
- **It can ship before the rest of this design.** Today's manifests already record the compiled
  prompt, the references, the size and the steps, so a redraw with a new seed works now. Identity
  improves when R1 lands.
- **What a redraw needs, and when it cannot run.**
  - A new picture records no reference paths (`reference_images` is null). Its redraw rebuilds
    the references from the manifest's `cast` block: the name, the entry version and the
    portrait. Portraits are content-addressed, so a redraw at the recorded version finds the same
    portrait while the blob is kept.
  - An edit records workspace-relative paths, in a file a run can write (§5.1). **A redraw
    re-validates every recorded path before reading it**, as every manifest reader in
    `imagegen.rs` already does (`original_of`): a plain path of safe characters that resolves
    through `ToolCtx::resolve` in this chat's jail, or the redraw is refused. Once `work.rs`
    retention has collected the chat's workspace, there is nothing to redraw from.
  - Either way, the button says so and does not draw something else in its place.

## 6. Use cases

| # | Use | Canvas | People |
|---|---|---|---|
| 1 | New picture of the persona | none | A's whole portrait at 512², as today; the crop only after §8.1 passes |
| 2 | The persona in an owner photo | the photo | A (M1) |
| 3 | With another library character | place or none | with a canvas, both crops + descriptions; with none, whole portraits at 512² (M3 measured touching for two that way) until §8.1 |
| 4 | With someone from an owner photo | place | A + "the person in picture X" (their crop only) |
| 5 | Clothing, expression or hair | current picture | crops ride along |
| 6 | Pose | place | restage (M2, R) |
| 7 | Camera move | place | restage; wording from §5.4 |
| 8 | A painted region (the Edit modal) | current picture + mask | no crops, as today (§7) |
| 9 | One small change, the rest held | as the change requires | the unchanged fields are kept by the scene |
| 10 | A picture from another chat | resolved by content hash | from its scene |
| 11 | A picture of the persona made outside mecha | the photo | declared by the owner's words |
| 12 | An owner photo with no library people | the photo | none: a plain edit |
| 13 | A persona with no linked character | as above | words, as today; linking a character fixes it |
| 14 | A library style | any | any |
| 15 | A picture during a call (`PERSONA-CONTEXT-DESIGN.md` §5.4 jobs) | any | the scene is data a job can carry |
| 16 | Variations | the same scene | new seeds |

Five or more people, or physical interaction beyond M3's two people, may need the research's
tier C, which `IMAGE-COMPILER-RESEARCH.md` keeps for five or more people and for interaction. E3's
four were whole portraits standing apart. Tier C is switched on per case.

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

**Kept:** a masked edit carries no crops, as `ImageGenerate` decides today ("the masked graph has
one canvas"). Outside the mask nothing moves, so the people there keep their pixels. And the
10-05 drift study behind #569 found that a masked redraw with a face crop did not move identity.

## 8. Open, measured during the build

Each is judged by the owner on face-sized, labelled sheets. ArcFace only flags gross drift.

1. Two people in one new picture **with head crops** in place of full portraits. M3 used full
   portraits.
2. Wording that produces a high camera angle.
3. Why the seated, facing-the-camera placement in P1 failed for crop alone and for words, but
   held with the description.
4. How often the model waives with `"cast": []` (the one-token opt-out of declared identity),
   counted from manifests' `identity`. The tool description gives it two uses: someone else who
   shares a library character's name, and resetting a record whose people have left the picture,
   until the scene record can remove one person. Which use it is put to is the measurement.
5. A location library entry beside owner photos (`IMAGE-COMPILER-DESIGN.md` §1 left locations as
   free text until measured).
6. A third crop beside a canvas (§5.2, budget), and adding a person by restaging from the place
   rather than onto the current picture (§5.3).
7. A chain of retouches, three or more deep, each carrying the crops (§5.3).

## 9. Rulings (owner, 2026-10-07)

R1–R7 and R9 were accepted as written, and R8 is the owner's own proposal. They are settled, and the
build follows them without asking again.

- **R1.** Identity is declared: every library person enters every render as head crop plus
  description (§5.2).
- **R2.** Pose, camera and people changes restage from the place, and nothing chains (§5.3).
- **R3.** The current scene lives with the persona across chats, and pictures resolve by content
  hash (§5.1).
- **R4.** §5.5 is amended as in §7.
- **R5.** #569's lineage path is retired rather than kept beside this.
- **R6.** The owner's eye on face-sized sheets is the gate for every step. ArcFace is a backstop.
- **R7.** With two chats rendering for one persona (incognito chats never write back, R9 and
  `INCOGNITO-DESIGN.md` R3), each chat works on its own copy of the scene,
  and the last render to land updates the persona's latest (§5.6).
- **R8.** Regenerate beside Edit on every picture card: the same scene with a new seed, no model
  turn (§5.7). This replaces drawing two variants of every picture. Versions stay on one card, and the version showing
  is the one that is built on.

- **R9.** An incognito chat reads the persona's scene and keeps its own copy, deleted on close.
  It never writes back (§5.1, from `INCOGNITO-DESIGN.md` R3 and R6).

## 10. Build order

It builds on #583 (jobs) and #577 (run notes in a head and a tail), both merged 2026-10-07. The
scene record comes before the canvas rule, because the rule compares scene fields and restages
from the scene's place.

1. Declared identity on the measured paths: lift the `cast` + references refusal, crops and
   descriptions beside a canvas under §5.2's resolution and slot rule, the name guard on edits,
   and `face_anchor` recording why it was not applied. New pictures keep the whole portrait.
   They switch after §8.1, and only on the owner's sheets.
2. *(Built 2026-10-07, `scene.rs`.)* The scene record per persona, manifests carrying scenes,
   and lookup by content hash through a harness-written index in the persona store, never
   through workspace manifests (§5.1). An incognito chat's write-back path does not exist
   rather than being switched off, and every scene carries its origin (§5.1).
3. *(Built 2026-10-07, the `scene` field; owner's ruling: option A.)* The canvas rule and
   restage, with the scene change as the call's type, replacing the typed
   edit.
4. *(Built 2026-10-07.)* The scene in run notes.
5. §8's measurements, each before the step that depends on it.
