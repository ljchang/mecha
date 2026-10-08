# Image generation, redesigned

**Status: accepted 2026-10-07; rulings through 2026-10-08** (recorded in §11, §13 and §14). Step 2 is built (#596, merged 2026-10-08).

- **Lead:** mecha-7e.
- **Evidence and review:** mecha-a3. a3 owns the evidence base and the acceptance gates, and reviews this draft adversarially.
- **Supersedes:** `IMAGE-SCENE-DESIGN.md`'s mechanism, the call shape of `IMAGE-COMPILER-DESIGN.md` §3 (`cast` and `extras`; the compiler itself and its templates stand), §4's `image_generate` bullet and §5's manifest contents, and `PERSONA-CONTEXT-DESIGN.md` §5.5's mechanism.
- **Keeps:** the library store and its doors (`IMAGE-COMPILER-DESIGN.md` §2, §4's `image_library` tools, §6 and §7), background jobs (`BACKGROUND-JOBS-DESIGN.md`) and incognito (`INCOGNITO-DESIGN.md`) as they are.

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

1. **Identity comes from the library.** A head crop plus the library's own description works on every edit (M1, by the owner's eye). New pictures use the whole portrait at 512², which the owner rated good (M3) until the crop-on-new-pictures measurement passes (`IMAGE-SCENE-DESIGN.md` §8.1, which survives that document's supersession; §9 carries it as an open measurement). A model-drawn portrait does not help (M4), and neither does 1536² resolution (M4).
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
  setting:  Words("a narrow kitchen with a window over the sink") | Photo(path, hash)
  light:    "late afternoon sun, warm and hazy"       // optional prose: light, mood, time of day, colour tone
  camera:   "a wide shot from low by the door"        // optional prose: shot size, angle, framing, lens
  style:    library style name                        // optional
  people:   [ Person { who, where, wearing, doing, expression } ]
  together: "Maya and John hold hands, looking at each other"   // optional prose: what people do with each other
  text:     [ { words, where, look } ]                // optional: words to render exactly, in quotes
}
Person.where = left | centre | right | background     // optional; defaults to the list's left-to-right order
Person.who = a library character's name | "self" | a description of someone not in the library
```

- **`setting` is everything in the picture except its people and its words.** For a picture with people it is the place and its objects, with no looks or poses. For a picture without people (an object, a diagram, a logo) it is the whole subject. Era and period go here too. It is either words, or a photo used as the room.
- **`together`** is what people do with each other, stated once (§13): a shared act, gaze between people, who holds whom. Each person's `doing` stays their own pose. Without it a shared act is written twice, once per person, and can contradict itself. Relations between people are the measured weak point of image models (T2I-CompBench, LAION-SG).
  - **Only between people in the picture** (review T1). With one person drawn, a relation is with someone not drawn ("reaching toward the viewer"), so the compiler folds `together` into that person's `doing`. In the corpus, 96 of 116 relation phrases were with someone not drawn, and only 20 were between two drawn people.
  - **It does not outlive the act** (review T2). "Equal is unchanged" carries most fields forward, but a relation depends on the people's own acts. Any change to someone's `doing`, a removal, or an addition clears the recorded `together` unless the call restates it. Otherwise "John sits down" would keep a hug the record no longer has bodies for.
- **`where`** places a person: left, centre, right or background. It also anchors which portrait or crop goes to which body ("the woman from <image2>, on the left"). Without it the list's order is left to right, as the compiler does today (E1–E12).
- **`text`** is words to render, quoted exactly, never paraphrased, with optional placement and look ("on the shop sign, in gold serif"). Qwen-Image's text rendering is its strongest skill, and every vendor's guide treats text apart. Rare in chat (1 in 264), but a field makes "the owner's words, unaltered" checkable.
- **`who`** is resolved against the library, in this order:
  - an approved character's name or alias brings its portrait (new picture) or head crop (edit), and its description verbatim;
  - `self`, or the persona's own name, display name or folder name (as `cast_self` resolves it today), is the persona's linked approved character;
  - a real person (a friend, the owner) is a library character like any other, added to the library from photos the owner attaches (§4.1; owner, 2026-10-07: real people come in through the library, not straight from a photo into a scene);
  - anything else is drawn from its own words, as an extra is today. That needs no separate field, and `extras` was never used.
- **Library names in the prose fields are checked** (review B2): `setting`, `light`, `camera`, `together`, a descriptive `who`, `wearing`, `doing`, `expression` and `retouch`. In `together`, a name of someone in `people` is expected and kept as written; a name of someone not in `people` follows the rule below. A name in a descriptive `who` resolves to that character. **Someone is drawn only when listed in `people`** (owner, 2026-10-08, §11 item 11): a library name in any other field, for someone not in the picture, is neither drawn nor refused. The image model reads "the viewer" in its place, since drawing them from words makes a stranger (E1), and the result says so. On a picture with no record the names are left as written, its people being unknown. A character whose entry did not load, named anywhere in the call, is refused.
- **`wearing`, `doing` and `expression` are prose** (§2.8). `wearing` and `doing` are required for anyone the call introduces. `expression` is optional and separate from `doing`: an expression changes the face and is an edit of the picture, while a pose redraws the scene (§5.2).
- **The scene is recorded** for every picture drawn: per chat, outside the jail, in a store the harness writes (§6). That covers the assistant chat as well as personas, so there is one path, not a scene path and a no-scene path. Each field carries its origin (clean or untrusted), as `scene.rs` does now.

### 4.1 Real people, through the library

The owner can put a real person (a friend, the owner) in scenes. **They come in through the image library, not straight from a photo into a scene** (owner, 2026-10-07). The library's approval step is where the owner says who this is and that they may be drawn, once, instead of a face appearing in a scene on a run's say-so.

So far this has been used once: 41 attached images held 22 other real people's faces, 5 picture calls referenced them, and 1 added a person to another picture.

- **Adding them.** The owner attaches one or more photos of the person and asks for a library entry. `image_library_propose` stages a candidate from those photos. On the library page the owner approves it, gives it a name, and confirms the person has agreed to be in pictures. The entry is marked `real`. Nothing reaches a scene before that approval.
- **Several pictures per entry, one default** (owner, 2026-10-07). Any library entry can hold more than one portrait, real people and drawn characters alike: a front view, a profile, another day. One is the default, set on the library page, and the default is what every render uses today. Adding a photo to an existing entry is a candidate the owner approves, like any library change. Choosing a portrait to match a scene's angle is a later step; it needs its own measurement.
- **Picking the face in a photo.** When the owner adds a photo, RetinaFace finds the faces. A photo with one face uses it. In a group photo the library page shows numbered boxes and the owner picks one. A face under a minimum size (proposed 96 px, set by G4b) is refused there and then, before it becomes a portrait. This replaces in-scene `which` (reviews R3, R4).
- **In a scene they are an ordinary `who`.** The prose check and the budget apply as for any library character (§4). Their name, being a library name, can't collide with another.
- **How real people may be drawn is deferred** (owner, 2026-10-07: get the functionality working first, and think usage rules through carefully as a separate decision). See §14. The `real` mark is still recorded on approval, so that decision has something to act on, and so is a `minor` mark (§14): recording a mark is a fact, not a usage rule, and an entry with no mark cannot be told from one the owner affirmed as an adult, so a check §14 adds could not fail closed over entries approved before it.
- **Existing entries.** Entries already in the library that are real people need marking `real`. The owner marks them on the library page; nothing guesses.
- **Privacy.** The photos arm `private_data` when attached, and the portraits stay in the library store on this machine. Removing an entry removes its portraits, as `imagelib::remove` does now.
- **Measured before it ships** (§9, G4b). Identity from photos the owner adds has not been tested the way the existing portraits were (M1–M4). The render set is mecha-a3's design:
  - the owner's own photo, chosen by the owner, with the face at least 150 px, added as a library entry;
  - drawn as a new picture, beside a drawn library character, placed on a photo, and restaged, plus a 64 px small-face arm that sets the minimum size;
  - 3 seeds each, 15 renders, all neutral and clothed, judged by the owner on face-sized sheets.

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
| no `picture`; a whole scene | **New picture.** Library portraits, the E1–E12 compile, a new seed (or, on the CLI and in evals only, the call's). |
| `scene.setting` is a photo; a scene with people | **Place on the photo.** An edit of the photo with each person's head crop and description (M1). The photo becomes the scene's setting. |
| a `picture` with no record | **The current picture.** Edited as any recorded picture is, with its record started from the call; its people are unknown until a call declares them, and a restage of unknown people is drawn as an edit (reviews B3, N1, N2). |
| a change to someone's pose or place in the frame (`where`, a swap of positions included), the camera, the setting, the light, `together`, the style, or a removal | **Restage.** Everyone drawn afresh on the scene's setting, at the base picture's seed (§2.6): an edit of the setting photo with crops when the setting is a photo, a new picture from the setting's words when it is words. |
| a change to someone's clothes, someone's `expression`, someone added, or `text` | **Edit of the picture,** with the crops of the people it changes, in the #408 keep form. Clothes come back unchanged elsewhere 100% (correct for this case). An expression is an edit of the face, not a restage (review S2). |
| `size` alone, on a recorded picture | **Redraw** at the asked shape, at a new seed, as the nothing-changed row (it is that row with a shape): `size` is not a scene field, so it changes nothing in the record, and a render at another shape is a new latent. A restage or redraw with no `size` keeps the picture's shape (§2.6: the base seed holds the room only at the same latent size). A restage that also names a `size` is drawn at a fresh seed, since at another latent size the base seed cannot hold the room. |
| nothing changed at all | **Redraw** of the scene at a new seed (review B4); the same render as Regenerate (§5.4). |
| `retouch` (with or without `mask`) | **Retouch** of the picture, in the #408 keep form; masked as today. |

**Budgets, as measured:**
- **An edit-shaped render carries the canvas plus up to five face crops** (`EDIT_REFERENCE_BUDGET` 6, up from 3). **Provisional (owner, 2026-10-07): start with C5 and revisit after a closer look at the sheets.** The owner wants up to five people, though usually one to three. Because the budget is a cap, one to three people cost what they did. Five people on a photo, in one pass (C5), kept the room, with faces "mostly really good, a few of Luke's drift"; about 240 s. Two passes (S5) shifted the first three. Before it, mecha-a3's three-person set put three library characters on an owner photo, 3 seeds per approach (evidence §F):
  - **C3, canvas plus three crops:** kept the owner's actual room. All three approaches placed the three people correctly, and face identity was close across them.
  - **RN3, portraits with the photo as material:** recomposed the room.
  - **W3, the room in words:** drew a different room every time.

  The owner's verdict (2026-10-07): C3 keeps the scene, RN3 is close with some distortions, and W3 is clearly not the same room. The cost is time: about 150 s against about 46 s at three people, and about 240 s at five. The owner chose scene fidelity, so one budget serves every edit-shaped render: placing on a photo, restaging on a photo setting, adding someone, and clothes.
- **A new picture carries up to `MAX_CAST` 5 portraits** (up from 4; five whole portraits with the room in words drew all five correctly, 3/3).
- **A scene keeps up to 10 people** (`MAX_PEOPLE`, twice `MAX_CAST`); people without library entries count here, not against the face budget.

A change that does not fit falls back before it refuses (review S4):
- **On a words setting,** a restage is drawn as a new picture from the words at the base seed, with up to `MAX_CAST` portraits. Under C5 an edit and a new picture both hold five faces, so there is no case where a words restage fits one and not the other; the fallback that mattered at the old budget of 3 (a restage needing more faces than an edit held) cannot arise, and more than five faces are refused on either setting.
- **On a photo setting,** up to five people with faces are drawn in one pass (C5). More than five are refused in the scene's own terms, naming them.
- **A person without a library entry** costs no budget; they are drawn from words.

**Seeds:** the model does not send seeds (§5.1). An edit-shaped render samples fresh (#306). A restage reuses the base picture's seed. A redraw takes a new one. The seed actually drawn is recorded, so a redraw always differs.

### 5.3 The edit panel: the harness extracts, the persona replies

A panel press is the owner's instruction to the image model, not something said to the persona (`persona/edit.rs`'s own doc). So:

0. **The persona's safety check runs first,** the same `safety::keyword_hit` crisis check a persona turn meets before any model, gated the same way on the persona's crisis switch (review S6). A panel press is never a door around it.
1. **The owner's words plus the picture's record go through a one-shot extraction,** a `quarantine::QuarantinedPass`: the record's fields may carry untrusted origin, and that type holds "no tools, no history" structurally rather than by convention. It has no tools and no history, and is constrained to the scene-change schema: `scene` fields plus an optional `retouch`, with **no `kind` discriminator** (§2.7: clothes need an obvious home, which is `people[].wearing`). A change of look is `look`, in the owner's own words, and code makes it a change: a look the library holds is that `style` (spelled as names are, a trailing "style" or "look" dropped), and any other look joins the setting in words, except over the owner's photo, where words would replace it and the change is refused naming the way on. The reader copies a look faithfully, but given `style` it picked the nearest name for a look the library lacks, and a setting-restating clause leaked into unrelated asks (mecha-a3's G605, 2026-10-08, #605). The request carries the approved, unlocked style names (not private, the owner's ruling of 2026-10-08).
2. **The extracted call is dispatched through `Agent::dispatch_one`** (#592). It goes through every gate a model call meets, with its own call id, inline. `dispatch_one` takes no job seat itself; the seat is the chat's live run, and the panel handler marks the turn live (`ps.live`) before the extraction starts, since no model call does it on this path (step 3's obligation): while the panel's turn is live, another message to the chat is a steer, not a turn, so no model call of that chat can start a picture beside it, and a picture still out from an earlier turn refuses the dispatch as busy. The caller records the taint (`Record::Taint`), since `dispatch_one` writes no session file.
   - **The extraction's output takes the record's origin.** It read the record, so a field it re-words is not the run's own words. When the record's origin is untrusted, the harness arms the conversation untrusted before the dispatch, so every field the call sets is stamped untrusted (`scene::Origin::of`), and a re-wording never launders an untrusted setting clean. Equal-is-unchanged alone protects only a verbatim restatement. A unit test pins it: an untrusted record's panel edit lands untrusted.
3. **The history records one fact,** a `HarnessPicture` record: "Picture X was changed into Y: <the typed change>". The card shows the new version from it.
4. **The persona replies in a line or two,** in its own voice. That reply goes with `tool_choice: "none"`, not with the tools removed: on llama-server, removing them re-sent the whole context (12,277 tokens against 4 measured), while `tool_choice` kept the cache (review S5). That measurement is the local server's; Anthropic's caching rules list a `tool_choice` change as invalidating cached message blocks, so on that path the closing request may re-read the history, and the cache lens reports it as a surface change, not an unexplained drop. It carries no edit note, and G3's calls-as-text check covers it.
5. **"Try again" is a redraw, not a failure** (review N3). An extraction that comes back empty, or restates the record, on a recorded picture means the owner wants another attempt. It is drawn as a redraw at a new seed (§5.1), and the card says it was drawn again.
6. **An extraction that fails is said, never dropped and never retried in a loop** (review S7). Only these are failures: a 400, unparsable or schema-invalid output, or a name the library does not hold. The card says the edit was not understood and why, in a line, and nothing is drawn. G1 counts it.
7. **A dispatch that the tool refuses is said the same way.** A schema-valid extraction can still meet the tool's own refusals: a library entry that did not load, named anywhere in the call (§4), more than five faces (§5.2), or the chat's picture seat busy (§5.5). A library name in prose for someone not in the picture is not one of them: it is "the viewer" (§4). With no model left on this path to read the refusal, the card shows the tool's sentence as the edit's result, and the `HarnessPicture` records it as not drawn. The persona then replies in a line under its own closing note, which says the change was not drawn and points at the reason in the turn; replying keeps the turn's shape (the owner's message answered), and a failed edit is still said by the card. G1 counts a refusal for shape against the extraction.

This removes the panel loop at its source (§2.9), because the persona never makes the call. It also removes the model's field choice for the turns where it chose wrong. Typed requests in chat ("draw us at the beach") still go through the persona model, with the one-shape schema (§5.1).

### 5.4 Regenerate (R8)

The same scene, a new seed, the same render plan. It goes through `dispatch_one` and is recorded as a `HarnessPicture` with `how: redraw`. The card shows versions ‹ 1/2 ›, and the version showing is the one Edit and Regenerate build on. R8's ruling stands. R8-2's call reconstruction (#593) is not needed, because the scene is the record. #593 is closed unmerged.

### 5.5 A run makes at most one picture

Two rules (review B1).

- **Structural, in every chat.** A deferred job of a tool already started in a run is never started again in that run. The agent loop enforces this on the deferred job's tool name, not the model, and not a refusal string. The closing lines are the picture's own today, because `image_generate` is the only tool that defers; a second deferred tool brings its lines through its job (`DeferredJob`, as its busy text already does), never as new text in `agent.rs`. The assistant keeps working after drawing (it went on to `image_view`, `shell` or `fs_read` in 43 runs), so its run is not ended.
- **The clean end fires on the first repeat call** to that tool. The repeat is not run: its `tool_use` gets a refusing `tool_result` ("Not run: … already started in this run"), so every call is still answered. The next request then goes with `tool_choice: "none"`, the turn's note once, and "the picture is on its way; answer in a line" as its last note. That combination was measured clean 5/6. In a persona chat it fires right after the picture is queued, because nothing after the picture is the persona's job.
- **A busy refusal** (an earlier turn's picture still drawing) closes a persona run, with a line that says this turn's picture was not started. Any other run keeps working after one and closes on a retry of the refused call. A picture this run started is always the fact said first.
- **The backstop:** a reply that is only a tool-call block is replaced, never shown.

**Retired:** `REPEAT_REFUSED` and `REPEAT_IN_FLIGHT`, which never fired in any session.

**LoopGuard stays as it is.** It is generic, not an image guard, and by design it never counts a call the harness refused — the approver, a hook, a policy, the interlock (the `refused_by_harness` filter on the trace's `denied` flag; review of #448: "that is the harness working"). The queue's busy refusal is one of those, which is why it never stopped a picture loop; the rules above close that loop for pictures without touching the guard. Whether the guard should also count harness refusals, for every tool, reverses #448's rule and is the owner's question (§11 item 8), not part of this build.

## 6. The record

- **Where:** one scene store per chat, outside the jail, harness-written. Records written before the redesign still read: the old `place` as the setting (its `picture` kind as a photo) and a person's `name` as a library `who`. Their people are unknown, since the mark postdates them, so a pose change on one is an edit, never a restage with nobody.
  - **Persona chats** also write the persona's latest and the content-hash index, so a picture carried into another chat is found by its bytes (`IMAGE-SCENE-DESIGN.md` rulings R3 and R7).
  - **Incognito** keeps its copy in the room and never writes back (R9).
- **What:** the scene, with origins per field. The place is the setting only (§4). Plus the picture's hash, its seed, size, render plan and the library versions used.
- **The scene note** (`scene::note` in each persona run's notes) stays, for typed turns, which still have the persona choosing the call. Its stems and origin rules stand. With the setting recorded as the setting only, it no longer carries an old pose, and G3 covers it (review S8).
- **The manifest** in the jail keeps only what the owner-facing doors read: the image path, the call id (orphan repair), the seed and the cast names and versions (save-to-library), the route, the crops and the layout reading, plus a pointer to the scene by hash. A run can write the jail, so nothing reads a scene back out of a manifest, and where the manifest and the record disagree the record wins. The cast names there only prefill save-to-library's form, whose save crosses the owner.

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
| the name guard's retry skeleton and "split the scene" (~160 lines) | replaced by the rule of §4: a name resolves in `who`; elsewhere, for someone not in the picture, it is "the viewer", never drawn and never refused |
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
| **G1 panel extraction correctness** (replayed real panel turns plus controls: clothes, expression, camera, object, add someone, owner photo as setting, a two-person relation, a swap of positions, and an off-camera relation) | the typed change is right; failures said on the card. A relation may land in `together` or in a `doing`; only a contradiction or a lost relation fails (review T3) | ≥ 90%, and **0** refusals for shape |
| **G1b typed turns** (replayed typed picture turns through the new schema) | the call the persona makes is right | ≥ 90%, and **0** refusals for shape |
| **G2 the wanted call still happens** | request 1 of every replayed picture turn calls the tool; no reply narrates a picture without one | ≥ 95% |
| **G3 one picture per run** | 0 calls written as text shown; 0 empty replies (at most one started is a unit test, below, not a gate) | 100% |
| **G4 renders, the owner's eye** (face-sized labelled sheets, ≥ 3 seeds each) | restage pose right, room held across restages, identity holds; words setting and photo setting; and a five-person arm on a photo (a canvas plus five crops), where Luke-style drift is watched | owner's verdict |
| **G4b a real person through the library** (§4.1) | the same real person, added to the library from the owner's photo, drawn new, beside a drawn library character, placed on a photo, and restaged, plus a small-face arm; ≥ 3 seeds each | owner's verdict |
| **G5 after deploy** (first real chats) | pose and camera changes go through restage; restages pass the owner's eye; edit-shaped renders that should move something (adding a person, placing on a photo) are not near-copies; no runaways | restage share ≥ 90%; edit-shaped near-copy ≤ 30%; runaways 0 |

**The harness's own claims are unit tests, not gates.** One job per run, taint through `dispatch_one`, origin under equal-is-unchanged, incognito's scene kept in its room and never in the mecha home (`only_the_served_chats_stamp_a_scene_slot`; R9 holds by that test now that the assistant has a store), every field counted in a scene's origin (`Scene::origin` names each field, so a new one is a compile error), and an extraction failing closed on a name the library does not hold are deterministic: each step names the tests that failed on the behaviour before it (step 2's, for example, in #596: `two_calls_in_one_turn_say_the_picture_is_on_its_way`, `a_steer_on_the_closing_turn_stays_queued`). The gates above measure what only the model and the owner's eye can.

**Open measurement, not a merge gate:** the head crop on new pictures (`IMAGE-SCENE-DESIGN.md` §8.1). New pictures keep the whole portrait at 512² until it passes on the owner's sheets for each path (§11 item 5).

## 10. Build order

Each PR goes through its review loop, then a3's gates, then the owner's merge word.

1. **The scene schema and the planner, with the retired inputs deleted.** This covers the record per chat (§6), equal-is-unchanged, place as the setting only, seed reuse, and the refusals in scene terms. It is the big one.
2. **One picture per run** (§5.5). **Built: #596, merged 2026-10-08.** It added `ToolChoice` to `CompletionRequest`, rendered by both providers (`"tool_choice": "none"` and `{"type": "none"}`). A provider added later must render it or refuse the request, never drop it: a dropped `tool_choice` costs the clean end (the structural rule still refuses a second picture), and a guard that silently stops working is the shape this repo refuses.
3. **The panel through extraction plus `dispatch_one`,** and the `HarnessPicture` record (§5.3).
4. **Regenerate** on the same record (§5.4).
5. **The web:** versions on the card, and Edit and Regenerate on the version showing.
6. **Real people through the library, and several portraits per entry** (§4.1), after G4b passes. This step covers adding people from photos on the library page, picking a face in a group photo, the minimum face size, the `real` and `minor` marks and the default portrait. Usage rules are not part of this build (§14). **Ruled 2026-10-07: get it working first, with no restrictions; step 6 is not held for §14.**

Steps 1 and 2 are independent and can run in parallel lanes.

## 11. Questions for the owner

1. **Record the scene for the assistant chat too** (§6)? **Ruled 2026-10-07: yes, one path.**
2. **The scene's fields** (§4, after the survey in §13): `setting`, `light`, `camera`, `style`, `together`, `text`, and per person `who`, `where`, `wearing`, `doing`, `expression`? *Owner, 2026-10-07: research how other image tools structure their inputs before ruling.* The survey added `together`, `where` and `text`. It confirmed `camera` absorbing shot size and angle, `light` absorbing mood and colour, and the compiler (not the model) writing the keep list on edits. Rendered words go in `text`, never in `setting`. **Ruled 2026-10-07: this set.** `light` covers mood (79% of what prompts carried beyond people), `camera` covers framing (11%).
3. **Close #593 unmerged** (§5.4)? **Ruled 2026-10-07: closed.**
4. **The thresholds in §9.** **Ruled 2026-10-07: accepted as proposed.**
5. **Crop or whole portrait on new pictures** (§2.1)? **Ruled 2026-10-07: the whole portrait until `IMAGE-SCENE-DESIGN.md` §8.1's crop measurement passes on the owner's sheets**, as for R1.
6. **Seeds off the chat schemas** (§5.1)? **Ruled 2026-10-07: off;** every seed reuse that helps is the harness's.
7. **Up to five people** (§5.2)? **Ruled provisionally 2026-10-07: C5,** a canvas plus five crops in one pass; the owner may revisit after a closer look. The budget is one constant.
8. **Should LoopGuard count harness refusals** (§5.5)? **Ruled 2026-10-08: no change.** The owner's aim is a design that needs no loop-blocking function for pictures, and this one has it: one picture per run is structural (§5.5), so the picture loop is closed where it starts. LoopGuard stays as it is, a generic backstop for every tool, under #448's rule that a call the harness refused never counts.
9. **Structured output for the extraction** (§5.3 step 1)? **Ruled 2026-10-08: on.** `structured_output = "llama_json"` on every provider served by the router. mecha-a3's baseline on the first extraction build measured it right on 63 of 83 labelled real cases with the schema, and 35 without; after the reader's fixes, 67 of 82 (82%) and 65 of 83 (78%). That is below §9's 90% bar; mecha-a3 passed G1 for merge on its reading that the remaining misses are label ambiguity between clothes and pose (an action like undressing reasonably lands in `doing`), and the owner may weigh those cases by eye. The other one-shot passes on those providers (the crisis judge, triage) send their schemas too.
10. **Rewrite old-shape image calls in existing chats' history** (mecha-a3's send-time view)? **Deferred 2026-10-08.** With the planner absorbing over-filled fields, edits in chats holding old calls already scored 33 of 35 (G1b, H0), and the owner ruled out backwards compatibility. See §14.
11. **A name the persona uses for the owner is also a library character** (mecha-a3's G1b: every refusal was the persona writing "…with Luke watching", and the library holds a `luke`)? **Ruled 2026-10-08: someone is drawn only when listed in `people`.** A library name in the other words is "the viewer" to the image model (§4). The owner and the `luke` entry may be treated as the same person.
12. **Queue a picture asked for while another is drawing, instead of refusing it** (mecha-a3)? **Ruled 2026-10-08: no;** it is refused, as §5.5 has it.

## 12. Review and how each point is met

mecha-a3's review (local `REVIEW-IMAGE-DESIGN.md`, with new measurements in the evidence file's §F) raised 4 blocking findings, 9 shoulds and 5 questions in round 1, 1 blocking finding and 2 shoulds in round 2, 2 blocking findings and 3 shoulds in round 3 (§4.1), and 3 shoulds and a doc point in round 4 (§13), with nothing blocking. Each is met above:

| Point | Met in |
|---|---|
| B1: the forced end breaks the assistant; LoopGuard is generic | §5.5, split into two rules; LoopGuard unchanged, and counting harness refusals is an open question (§11 item 8) |
| B2: retiring the name guard reopens E1 through prose fields | §4, every prose field checked; `who` resolves names and the persona's names |
| B3, N1: a no-record picture is not an empty room, and faces cannot tell | §5.1 and §5.2: the call says a photo's role; a no-record `picture` is always the current picture |
| N2: no people is not the same as people unknown | §5.1: unknown people are recorded as unknown; a restage of them is drawn as an edit |
| N3: an empty or restated extraction is "try again" | §5.3 step 5: drawn as a redraw |
| R1: consent; a real face in sexual pictures | §4.1: real people come in only through the library, where approval confirms consent. How they may be drawn is deferred to §14 (owner's ruling) |
| R2: labels colliding with library names | §4.1: moot: a real person's name is a library name |
| R3: small faces | §4.1: a minimum face size, proposed 96 px, set by G4b's small-face arm |
| R4: group photos | §4.1: the owner picks a numbered face on the library page |
| R5: per chat, and deletion | §4.1: moot: a real person is a library entry, removed with its portraits |
| T1: `together` with one person drawn | §4: folded into that person's `doing`; an off-camera control in G1 |
| T2: a stale relation | §4: a change to the acts clears `together` unless restated |
| T3: G1 and relations in either field | §9, G1 |
| T4: `where` (positions in 38% of calls with two or more people) | §4: `where` per person, also anchoring which portrait or crop goes to which body |
| T5: text in `setting` and in `text` | §4: `text` holds rendered words; `setting` excludes them |
| B4: equal-is-unchanged makes "try again" a no-op | §5.1 and §5.2: an all-equal call is a redraw at a new seed |
| S1: model-sent seeds copy earlier ones | §5.1 and §8, seeds off the chat schemas |
| S2: `doing` conflates pose and expression | §4 and §5.2: `expression` per person, an edit of the face |
| S3: use case 4 dropped | §4.1: a real person, first-class, through the library (approval is consent), its portraits in the library store, and its own gate (G4b) |
| S4: budget refusals are round trips | §5.2: fall back before refusing. Measured: a canvas plus three crops keeps the scene, and five held in one pass (C5, the owner's provisional ruling), so `EDIT_REFERENCE_BUDGET` is 6 references; more than five faces on a photo are refused |
| S5: `tool_choice` for the reply | §5.3 step 4 |
| S6: the safety check on the panel path | §5.3 step 0 |
| S7: extraction failure | §5.3 step 6 |
| S8: the scene note | §6, kept for typed turns |
| S9: `place` is wrong for a logo | §4: `setting` covers the whole subject without people |
| Q1: near-copy as a fact | §8: the layout number stays in the result line |
| Q2: the assistant record | §11 item 1, with B3's classification |
| Q3: `light` | §4 and §11.2 |
| Q4: the G5 metric | §9 |
| Q5: a typed-turn arm | §9, G1b |

## 13. How other image tools structure their inputs (survey, 2026-10-07)

The owner asked for a survey before ruling on the fields. It covered the official guides of FLUX.2 (whose model is trained on JSON prompts), Google's Gemini image and Veo, OpenAI's image API, Midjourney's parameters, and Qwen-Image's own prompt rewriter, README and edit model cards. It also covered the research on relations between people (InteractDiffusion, LAION-SG, T2I-CompBench) and ComfyUI's regional prompting.

- **What every guide has, and this design already has:** a subject and what they do, a setting, light and mood, the camera, a style, and the aspect. Qwen-Image's own example for people runs in nearly this design's order: who, wearing, pose, setting, light.
- **Added from the survey:**
  - **`together`, for relations.** Research represents interaction as a subject–action–object triple, and the benchmarks single out relations as a weak point. Products put relations in prose. One prose line naming the people is the portable form.
  - **`where`, for placement.** FLUX.2 has a position per subject, and Qwen's rewriter adds a position when one is missing. It fights attributes bleeding between people, and anchors each reference to a body.
  - **`text`, its own field.** Every vendor treats rendered text apart: quoted, exact, with placement and look.
- **Consolidated:**
  - **`camera`** carries shot size, angle, framing and lens. Veo's "composition" and Gemini's "shot type" are both framing, so there is no separate composition field.
  - **`light`** carries mood, time of day and colour tone, which is Veo's "ambiance". FLUX.2's hex colour palette is for brand work and is left out.
- **Edits: change, preserve, and the role of each reference.** Every vendor's edit guidance converges on three parts:
  - what changes ("change only X");
  - what stays, restated each time;
  - which reference plays which role.

  Here the model sends only the change. The compiler derives the keep list from the typed fields the change did not touch (identity, clothes, setting, light, framing), which is the structural form of "repeat the preserve list". It names each reference's role (canvas, crop of whom), as it does now.
- **Left out of the model's schema:** negative prompts (FLUX.2 doesn't support them; Qwen-Edit recommends a blank), seed, steps, guidance and quality knobs, exact lens numbers (OpenAI calls them "cues, not a guarantee"), per-person boxes (they need trained adapters Qwen-Image does not ship), and fine aspect ratios.
- **Noted for later, not in this build:**
  - outpainting, re-framing a picture to another shape, which Qwen has a template for and would be a separate operation;
  - pose-from-photo and garment-from-photo reference roles (Qwen-Image-Edit-2509);
  - Qwen-Edit works best with 1–3 input images. The edit budget allows a canvas plus five crops (six), past that range and slower (about 150 s at three people, 240 s at five). The owner chose it on the three- and five-person measurements because only it kept the scene (§5.2). Two people or fewer stay inside the range.
- **Not measured anywhere found:** whether JSON-shaped prompts help Qwen-Image. Qwen's own tooling compiles everything to prose of 200 words or fewer. That supports this design's typed fields compiled to prose by the tool.

Sources: docs.bfl.ai (FLUX.2 prompting, JSON prompting); developers.googleblog.com and ai.google.dev (Gemini image); developers.openai.com (image prompting) and the OpenAI cookbook (input fidelity); docs.midjourney.com (parameters); github.com/QwenLM/Qwen-Image (`prompt_utils.py`, README); huggingface.co/Qwen/Qwen-Image-Edit-2509; arXiv 2312.05849 (InteractDiffusion), 2412.08580 (LAION-SG), 2307.06350 (T2I-CompBench).

## 14. Deferred: usage rules

The owner deferred decisions about how pictures may be used (2026-10-07). The functionality comes first, and usage rules are to be thought through carefully as their own decision. Recorded here so they are not lost:

- **Real people.** Whether, and how, a library entry marked `real` may appear in nude or sexual pictures. mecha-a3's review R1 and the lead both recommended never, held in code. The proposed shape: a prose check before render, the compiler stating them clothed, and a local image-safety classifier on the output. No classifier is in the tree yet.
- **Minors.** The library holds an entry described as 16. The proposed rule: a `minor` mark on library entries, set by the owner, and set automatically for any proposed entry with an age under 18. A marked entry is never drawn nude or sexually, under the same three checks, and possibly never in a persona chat whose chats are sexual. The image model here is local with no guard of its own. **Ruled 2026-10-07: that entry is left out of all tests.** mecha-7e and mecha-a3 also will not generate such content.
- **Existing entries.** Which current library entries are real people or minors; the owner marks them.
- **Old-shape calls in existing chats** (deferred 2026-10-08, §11 item 10). Chats from before the redesign hold `image_generate` calls in the retired shapes, and the model imitates its history. mecha-a3's measurement: edit turns 19 of 35 right before the planner absorbed over-filled fields, 33 of 35 after, against 35 of 35 in a fresh history. A send-time view that rewrites those calls into the new shape would close the rest; it is a compatibility layer, so it waits on a ruling.
