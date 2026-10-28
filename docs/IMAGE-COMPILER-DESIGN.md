# Image compiler — design

> **Addendum 2026-09-29:** phase 2, the web surface, shipped as #385 (the
> Library tab, Save to library, and the browse lock, with approval as an
> in-process HMAC) and #394 (adding and editing entries from the page).
> Installed 01:07Z and 04:06Z. See `HISTORY.md` under 2026-09-29; the
> status line below is left as written on 2026-09-28.

**2026-09-28. Phase 1 merged and deployed (#383); phase 2, the web surface, on `feat/image-library-web`.** What a character
library for `image_generate` is, how a scene compiles against it, and what
the first build deliberately leaves out. The evidence is
`IMAGE-COMPILER-RESEARCH.md` (E1–E10 there, cited below by number); this
document holds the decisions and does not re-argue them.

## 1. The owner's rulings (2026-09-28)

Settled; the build follows them and does not re-ask.

- **v1 is characters and styles.** Locations stay free text in the scene
  until a location reference is measured. *(Reopened by §9.2: a `Location`
  kind, text-only now; its images gated by E13 and V-L.)*
- **The model proposes, the owner approves — and the owner may also create
  directly.** An entry the owner makes is approved on creation; one the
  model makes is a candidate until the owner approves it.
- **Its own store in mecha core, not the graph.** The graph MCP server is
  labelled untrusted (`untrusted_input = true` in the operator's config):
  reading a character from it would arm taint on every image. The graph's
  review queue is borrowed as a *shape*, not as storage.
- **A character is a portrait plus a description** that carries build and
  height — no body reference (E9), no required crop (E10: the whole portrait
  at 512² held identity within a few hundredths of a tight crop).
  *(Ruled 2026-10-13 by §9.2: pictures send a `full_body` reference, since
  E9b passed, built in step 2b. Video sends one too, unbound, since V-B's
  retest passed; owner, 2026-10-14.)*
- **The web surface ships second**, minimal: save to library, browse, lock.
- **The lock is a browsing filter, not an access control.** Generation
  always has access to locked entries. Things made from a locked entry start
  locked, and any item can be unlocked individually. It hides the whole
  entry — card, portrait, description — from the library only; pictures
  already in chats are untouched.
- **The password is optional** (a later ruling the same day). With none set,
  the Library tab's lock is a plain show/hide toggle; with one set
  (`mecha imagelib set-lock-password`), showing locked entries asks for it.
  Locking works either way.
- **No likeness rule in the compiler** — declined; the chat model's
  guardrails are the control (research §5 states the cost).

## 2. The store

`~/.mecha/imagelib/`, **global only** — nothing in a project's `mecha.toml`
can add to it, for the `[[trigger]]` and skills reason: a cloned repository
must not bring a character into a trusted session. There is no config key
for it in v1.

```
imagelib/
  characters/<name>/entry.toml
  characters/<name>/history/v<N>.toml
  styles/<name>/entry.toml
  styles/<name>/history/v<N>.toml
  blobs/sha256-<hex>.<png|jpg|webp>
```

- **The name is the directory**, `[a-z0-9][a-z0-9-]{0,63}`, pinned on load
  exactly as a skill's is — two directories cannot produce one name.
- **Pictures are content-addressed.** A portrait is copied into `blobs/`
  under its SHA-256; an entry names the hash, never a path. A canonical
  reference cannot change because a workspace file was edited.
- **Versions are append-only.** An update writes the old `entry.toml` to
  `history/v<N>.toml` and bumps `version`; every manifest names the version
  it used.
- **Writes are temp-then-rename**, so a reader never sees half an entry.
- **Loading is best-effort per entry**, like `SkillStore::load`: one
  unparseable entry is a reported finding, never a crash, and a missing
  directory is an empty library — an agent given no library creates no state
  by starting.

An entry:

| Field | Character | Style | Notes |
|---|---|---|---|
| `name` | ✓ | ✓ | pinned to the directory |
| `text` | a description, ≤ 400 chars | ≤ 1000 chars | pasted verbatim into prompts |
| `portrait` | ✓ | — | `sha256-<hex>.<ext>` |
| `source_seed` | optional | — | the seed that drew the portrait, when known |
| `status` | ✓ | ✓ | `approved` \| `candidate` |
| `origin` | ✓ | ✓ | `owner` \| `model_clean` \| `model_untrusted` |
| `locked` | ✓ | ✓ | browse filter only |
| `version`, `created`, `updated` | ✓ | ✓ | |

`status` and `origin` are closed enums in an on-disk store, so they are wire
formats: **an unknown `status` loads as `candidate` and an unknown `origin`
as `model_untrusted`** — never approved, never clean.

## 3. Compiling a scene

> **Superseded 2026-10-07 for the call (`IMAGE-DESIGN.md` §5.1):** `cast`, `extras` and
> `reference_images` are retired. The model sends a `scene` whose people name library characters
> in `who`, and the compiler below is what a new picture still runs through. The store (§2), the
> `image_library` tools (§4), the owner's door (§6) and the web surface (§7) stand as written; §4's
> `image_generate` bullet and §5's manifest contents are superseded, each with its own note.

`image_generate` gains two optional fields. The model writes only what
varies; code writes what persists.

```json
{
  "prompt": "a diner booth at night, warm light, candid",
  "cast": [
    {"name": "maya", "wearing": "a mustard-yellow raincoat", "doing": "laughing, head tilted back"},
    {"name": "john", "wearing": "a red plaid flannel shirt", "doing": "turned toward her, smiling"}
  ],
  "style": "film-noir"
}
```

The rules, each a measurement:

- **Identity is the reference plus the description** (E1: description alone
  0.33, pointer 0.74, both 0.78). Each cast member's portrait becomes a
  reference, and their description rides beside the pointer.
- **One reference per character, left to right, each appearing exactly once**
  (E2, E9: every slot tends to become a person). *(Ruled 2026-10-13, to be
  built: §9.4 adds two images per person, all or nothing, since E9b
  passed.)* One person is "the person in the image"; two or more are
  `<image1>…<imageN>` in `cast` order, "from left to right". Without
  `extras`, every cast is counted closed in the foreground, by name:
  "Exactly two people are in the foreground, Maya and Jo, each exactly
  once; anyone in the background is a stranger, not from any image" (ruled
  2026-10-19, E14). With `extras`, a total that counts them (E11).
  *(Before 2026-10-19 the cast was counted open, "each of the N people from
  the images appears exactly once; anyone else the scene describes is a new
  person", with no total: a total with no extras erased a waiter the prose
  described (E12). The open wording drew a touching pair three or four
  times (E14), and "no one else is in it" would empty a busy setting as it
  erased E12's waiter.)*
- **`wearing` and `doing` are required** (E3, E8: a reference supplies its own
  outfit, pose and stare when the scene is silent; stated, they land), except
  that `doing` is left empty when the scene's `together` says what its people
  do: a plain pose beside it drew a person twice (mecha-a3, 2026-10-08), and
  `imagegen::doing_words` says the plain pose only when the scene is silent.
- **References are sent at 512²** (E2: four at 1024² cost 190 s, four at 512²
  79 s; E10: a whole portrait at 512² holds identity). `Request` gains a
  backend-neutral `reference_size`; plain edits keep 1024.
- **At most five cast members** (`MAX_CAST`; the owner's **provisional**
  ceiling C5, `IMAGE-DESIGN.md` §11 and §5.2: five whole portraits drew all
  five, 3/3, up from E3's measured four; to be revisited after a closer
  look).
- **Anyone else is an `extra`, and is counted** (E11). A waiter described in
  the scene beside a cast of three, with "Exactly three people" compiled in,
  was kept in the picture but pushed into the background; listed as
  "Also in the scene, not from any image: …" with a head count of four — "the
  three from the images, each exactly once, and one new person" — he stood
  where the scene put him. Extras carry no reference, so the reference
  ceiling (five, above) does not bound them; `MAX_EXTRAS` is four,
  unmeasured beyond. They are read by the named-character guard too:
  "John waving" as an extra is John drawn from words.
- **A cast name the library does not hold is drawn as an extra** (owner,
  2026-09-30: a named character not in the library "blocks image generation
  when it shouldn't"). `imagegen`'s `compile_from` moves it, with its
  `wearing` and `doing`, into the extras before compiling, and the result
  says so. Only a name with no trace moves: a candidate, or an entry that did
  not load, stays in the cast and gets its own refusal, because drawing a
  stranger in its place is the substitution the cast exists to prevent. A
  persona chat's form of the tool keeps refusing an unknown name.
- **The style's text is appended verbatim**, never paraphrased.
- *(Lifted 2026-10-07; see the amendment at the head of this section.)*
  **`cast` and `reference_images` are exclusive in v1.** ComfyUI's encoder
  takes one reference resolution per call, so an edit canvas at 1024² and
  portraits at 512² cannot share one; editing a cast image works already by
  passing the image, whose people carry their own identity. `extras` are
  words, not references, so an edit may take them: they are appended as
  "Also in the scene: …", and the named-character guard, which an edit
  skips, does not read them there.
- **A cast generation defaults to square**, not to its first reference's
  shape — a portrait is not a canvas.
- **The same-seed rule, narrowed and kept.** An edit still always samples at
  a fresh seed. A cast generation *keeps* the model's seed — that is how a
  scene is revised with its composition — unless it equals a cast member's
  `source_seed`, which is replaced and said.

## 4. Tools

> **Superseded 2026-10-07 for `image_generate`'s call (`IMAGE-DESIGN.md` §5.1):** the first
> bullet's `cast` and name guard became `scene.people[].who` and the planner's prose check. The
> `image_library` tools stand as written.

- **`image_generate`** — `cast` and `style` as above. The result names each
  entry and its version. **A prompt naming an approved character who is
  not in `cast` is refused** before any GPU time: the first real run (2026-09-28)
  copied the looked-up descriptions into the prompt, left `cast` out, and drew
  two strangers. `"cast": []` means "someone else by that name".
- **`image_library`** — list or search the library, with a line saying how
  entries are used (names in `cast`, descriptions never copied into the
  prompt). **Returns approved entries only**, so it never declares
  `untrusted_input`: every approved entry's text crossed the owner, and
  candidates never reach the model (research §5: the loop taints on
  `caps.untrusted_input && out.external`, per tool, so returning them would
  arm every lookup). It **does** declare `private_data` — an entry describes
  a person, and with no likeness field nothing says which are real
  (`goal_context`'s footing). Locked entries are listed — the lock is for
  browsing.
- **`image_library_propose`** — stage a candidate character (a name, a
  description, and a workspace image as the portrait, read through the path
  jail) or style. Records `origin` from the conversation's taint
  (`ToolCtx::taint`; `None` is `model_untrusted`). Refuses a name already in
  use in any state, beyond 50 pending candidates, and a portrait over 4 MB —
  the cap bounds bytes, not only entries. It creates only
  a candidate, which changes nothing the owner uses until approved — so it is
  `read_only` on the footing `image_generate` has: a web chat starts
  read-only and proposing a character should be one request in it. For a
  character already in the library it points at the next door.
- **`image_library_reference`** — attach or remove a character's
  `full_body`, `face` or `detail`, directly from a clean conversation and
  staged for the owner otherwise (§9.3, the owner's ruling of 2026-10-14).

No tool description changes per turn; no character is ever listed in the
system prompt (the cached prefix). Approved, unlocked style names are, in
`image_generate`'s description since 2026-10-08, and §9.5 adds location and
outfit names on the same footing. An incognito chat may call
`image_library` and generate with a cast — both only read — and
`image_library_propose` stages there **locked** (`ToolCtx::stage_locked`;
the owner's ruling, 2026-10-12, on `persona_propose`'s footing): the write
outside the room is a candidate the owner keeps on purpose, hidden behind the
lock until unlocked. *Until that ruling it was withheld there.*

## 5. Manifests

> **Narrowed 2026-10-07 (`IMAGE-DESIGN.md` §6):** the manifest keeps only what the owner-facing
> doors read (the image, the call id, the route, the seed, the cast names and versions, the crops,
> the layout reading, a pointer to the scene by hash). No prompt; the scene record outside the jail
> is the record.

Every `image_generate` result writes `images/<stem>.json` beside its PNG,
`create_new` like the PNG: the scene prompt as the model wrote it, the
compiled prompt, the seed, steps and size, the model files, and — for a cast
generation — each entry's name, version and portrait hash. It is what makes an
image reproducible, and what phase 2's lineage and "save to library" read.

## 6. The owner's door (phase 1: the CLI)

`mecha imagelib`:

- `list [--all] [--json]` — approved entries; `--all` adds candidates.
- `show <name>` — the entry, its text, its portrait's path in `blobs/`.
- `add-character <name> --portrait <file> --description <text> [--seed N] [--locked]`
  and `add-style <name> --text <text> [--locked]` — owner-made, approved.
- `approve <name>` — prints the entry's text and asks; `--yes` skips the
  question **only for `model_clean`**. An untrusted candidate's text is read
  before it can ride into prompts. The web approves in `mecha serve`'s own
  process against a signature only it can make (§7); there is no CLI flag
  that stands in for having read the text.
- `set-lock-password` — the browse lock's password, read without echo.
- `reject <name>` deletes a candidate outright, with its portrait unless another
  entry names the same blob — nothing was generated from a candidate, and a
  kept portrait would let propose-reject-propose fill the mecha home.
  `remove <name>` moves any entry aside under `removed/`, portrait kept,
  because manifests may name it.
- `lock <name>`, `unlock <name>`, `update <name> …` — an update approves
  nothing it did not rewrite: only new text makes an entry the owner's.

## 7. Phase 2: the web surface (built 2026-09-28)

The **Library** tab (`#library`, panes `characters`, `styles`, `candidates`),
**Save to library** on the chat image card, and the lock — from §1's rulings.
The decisions that are this design's rather than the owner's:

- **Reads direct, writes by the CLI — except approval** (`serve/library.rs`).
  The list and the portraits are read from the store; reject, lock, unlock,
  remove and save are `mecha imagelib` children, the house rule of every
  write on the server. Approval is made in the server's process (below).
- **Writes honour the lock as reads do.** An action on a locked entry needs a
  live unlock token, and a hidden entry and a missing one answer the same
  404 before any child runs — without that, `unlock` revealed a hidden entry
  with no password, and the 200/409 split named which exist (review of #385).
- **The server does the hiding.** Locked entries are left out of
  `GET /api/library`, and `GET /api/library/portrait/{blob}` answers 404 for a
  blob no visible entry names — a page that blurred a thumbnail would still
  receive its bytes. The list and every locked portrait carry `no-store`; an
  open portrait is content-addressed and cached `immutable`.
- **Nothing says how much is hidden** (owner ruling, 2026-09-30). No
  `hidden · N` tile, no "N locked entries hidden" line, and no count in
  `GET /api/library` or `GET /api/personas`: a count tells whoever holds the
  phone there is something to find, which "sort of defeats the point". A
  locked page reads exactly like one with nothing locked — the list-level
  form of hidden answering as missing. #425 removed the `hidden_locked` field
  that had carried it. The same ruling covers a *name*: a visible persona
  linked to a locked character must not name it to a locked page, though the
  link itself is kept. #425 covers the persona list (`character` is `null`)
  and the problems the list and a save return, failing closed on an entry
  that will not load; the Edit screen's settings file is #430's.
- **The unlock is a token in the page's memory.** `POST /api/library/unlock`
  returns a token the page keeps in a variable — no cookie, no storage,
  which `web/test/no-storage.mjs` forbids — and sends as `?unlock=`; it lapses
  after 30 idle minutes and a reload drops it. With no password set it is
  granted for the asking: the lock is then a plain toggle (§1). With one,
  it is checked against `lock.toml` (argon2id, 0600, set only by
  `mecha imagelib set-lock-password`, read without echo); five wrong
  passwords in five minutes answer 429. The file's presence decides which,
  so a damaged lock file is verified, errors, and never opens as if absent.
- **Approval is of the text shown, vouched for by the server.** The list
  carries, per entry, an HMAC of its `shown_digest` (kind, name, version,
  text) under a key the server draws at start and never stores; the approve
  button sends it back, and the server approves in process only if the
  entry as re-read still signs the same (`imagelib::approve_as_shown`
  re-reads once more at the write). A bare digest was the first cut, passed
  to `mecha imagelib approve --shown` — but anyone who can read the store can
  compute one, so any shell could approve a model's proposal unread (review
  of #385). The flag is gone: the CLI's only door for an untrusted candidate
  is the interactive question, and the web's is a page only this process
  could have signed:
  a client of this server, as every approve route on it is. An
  owner-authenticated client can fetch a signature and send it back; the gap
  closed was a shell with no server at all.
- **Save copies, never points.** A chat's files are served only while the
  chat is open, so `POST /api/library/save` reads the picture through the
  jail now, stages it in a 0700 scratch directory, reads the seed from its
  manifest, and runs `add-character`. The lock box starts checked when the
  picture's manifest names a locked character (`GET /api/library/source`).
  From an incognito chat the save is always locked — the box shows checked
  and fixed, and `save_locked` locks it whatever the box sends (the owner's
  ruling, 2026-10-12; until then both routes refused an incognito key). An
  incognito save stages its scratch copy in the chat's RAM room, not the host
  temp directory, and refuses where the room is not tmpfs.
- **Add and edit from the tab (2026-09-29, the owner's ask).** An *Add
  character* tile takes an uploaded portrait, a name and a description; *Add
  style* takes a name and text; *Edit* on an approved entry takes a new
  description or portrait. Each is `add-character`, `add-style` or `update`
  as a CLI child, like every write here. The portrait is scaled to 1536 px on
  its long edge and re-encoded as JPEG in the page, which drops the photo's
  metadata before it leaves the device and costs nothing, since generation
  sends it at 512² (E10). It travels base64 in the JSON body so the owner's
  words never sit in a URL. Edit is refused on candidates: a proposal is
  approved or rejected as the model wrote it, because rewriting a text is
  what approves it (`update`), and an edit screen is not the place to read it.
- **Candidates are a review-queue row, not a backlog field.** `mecha review
  queues` (and so the Home cards) gains `image candidates`, opening
  `#library/candidates`. It is not added to `backlog::Backlog`, which is
  recorded on every run — a new field there moves what every older row is
  compared against, the reason `requests_on_owner` sits beside it — and
  candidates are owed to nobody outside, like the harness's own queues.
- **Deferred:** a character page's "appears in" (a walk over every chat's
  manifests), and the crop box (E10: a small gain).

## 8. Not yet built

- **Locations, scene assets, lineage and branching** (draft §6–7, §20).
  §9 proposes the library half of this: typed character references,
  locations and wardrobe.
- **Tier B checks** (face embedding, detector, VLM) and the repair loop.
- **Cast plus an edit canvas in one call** — needs a per-reference
  resolution (ComfyUI's `resolution = 0` keeps each at its own size), which
  wants its own measurement.
- **A prompt rewriter** (E1b, unmeasured).
- **A crop box** — optional on the character page in phase 2 (E10: a small
  gain).

## 9. Library v2: typed references, locations and wardrobe (ruled 2026-10-13)

**Status:** ruled 2026-10-13 (L1–L7, §9.7); the owner approved the direction
the same day (the video design's R6). **V-L ruled passed by the owner,
2026-10-25** (a room photo carried into four of four clips from the private
H3 server), and the `Location` kind built with video sending its first
`view` picture (L7 step 4). Still to come: pictures resolving a scene's
location to its text (§9.5; its images wait on E13), and the web Library
tab's Locations section. Built: the reference store and CLI
(step 2a, #684), pictures sending `full_body` (step 2b, #685), the web
strip (#688) and models' reference writes (#692). Video's bodies are ruled,
to be built in the video tool.
**Step 1 is measured and ruled (owner, 2026-10-13):**
  - E9b passed, so pictures send `full_body`, in a neutral, square-on
    stance;
  - V-B failed, with an extra person in one seed of two, so video stayed
    on face crops. Its retest passed (owner, 2026-10-14): four pictures
    without the face/figure binding drew no extra person in 4 of 4 seeds,
    so video sends bodies too, unbound: both pictures named as one
    subject, with no face-from/build-from split (§9.4).

  Step 2 (the store and CLI, then pictures sending bodies) reopened two of
  §1's settled rulings with new evidence, and §9.2 says which, rather than
  re-arguing them silently.

### 9.1 Why now

- **A second consumer.** The video tool (`video_generate`, MiniMax H3; built
  outside this repository for now, with its design moving in when it's
  ported) reads the same library. H3's full-reference mode binds several
  pictures to one person natively, "`<Subject 1>` … whose appearance comes
  from `<Picture 1>` and `<Picture 2>`". It has up to nine image slots,
  three voice slots and three video slots.
- **What the video measurements showed:** V0 in `IMAGE-COMPILER-RESEARCH.md`
  (2026-10-13). A whole portrait leaked its outfit into clips, a face crop
  stopped it, and clothing stated per clip landed.
- **What text alone doesn't carry well.** A distinguishing mark (a forearm
  tattoo came out heavier than "fine-line") and build: E9 never got to test
  build, because its trench coat hid it.
- **The owner's ask:** several typed images per character (face, full body,
  details), a location library, and named outfits, shared by pictures and
  video.

### 9.2 What it reopens

- **§1, "a character is a portrait plus a description … no body reference
  (E9), no required crop (E10)."**
  - **Changed for pictures by E9b** (passed; owner, 2026-10-13): a
    `full_body` reference is sent, bound to its portrait (§9.4), and
    narrowed by E14 (owner, 2026-10-19) to a picture of that person alone.
  - **Changed for video by V-B's retest** (passed; owner, 2026-10-14): a
    `full_body` is sent beside the face crop, unbound, all or nothing
    (§9.4).
- **§1, "locations stay free text in the scene until a location reference is
  measured."** Kept as the rule for **references**, the images. §9.6 is that
  measurement, for each generator. A text-only `Location` kind is still text
  in the scene: named owner text resolved into the setting, as a style is.
  So it ships without waiting for E13 or V-L, which gate only a location's
  images.

### 9.3 The store

A character entry gains typed references beside its portrait. The portrait
stays: it's what the Library tab shows, what `face::anchor_crop` crops, and
what every existing manifest names.

```
characters/<name>/entry.toml
  portrait  = "sha256-….jpg"           # unchanged: the canonical picture
  [[reference]]
  kind  = "full_body"                  # face | full_body | detail
  blob  = "sha256-….png"
  label = "standing, front view"       # optional here (browsing only)
                                       # required on a detail, e.g.
                                       # "left-forearm tattoo"
```

- **`kind` is a closed enum in an on-disk store, so it's a wire format.** An
  unknown `kind` loads, is shown in the library, and is **never sent to a
  generator**.
  - So is a known kind on the wrong entry kind: a `face` on a location, or a
    `view` on a character. Each entry kind allows its own kinds, and
    anything else is read as unknown, so a hand-edited entry can't put a
    picture where a person's face goes. That's the fail-closed reading: a
    reference nobody can classify carries nothing into a picture.
- **No `face` entry is needed today.** The face is the portrait's crop,
  cached by blob.
  - A `face` reference is for the owner who wants a better face than the
    portrait shows. At most one per character.
  - **Stored, and read by mecha only once something is measured** (review
    of #680, pass 27). Pictures *do* have a crop: every edit-shaped render
    sends one (`face::cached`, reading `entry.portrait` through
    `read_portrait`; "an edit sends faces only", §9.4). Re-sourcing that
    read would change every edit with no measurement behind it. So a
    `face` reference is the video tool's crop source, read at that tool's
    call site (outside this repository until the port); mecha's own crop
    stays the portrait's. Pictures keep sending the whole portrait for a
    new picture (E10), and the portrait's crop for an edit, until a
    measurement or the owner's ruling says a `face` reference should
    replace it. The portrait stays the picture the library shows.
  - **When it is read, the crop's source moves, not its cache** (passes 24
    and 25). `face::cache_paths` already keys a crop and its "no face"
    marker by the blob's stem; what is hardwired is the read. So the face
    reference is read through `read_reference`, under its own hash check,
    and cropped under its own key.
  - **A `face` reference with no face in it is said, never silently passed
    over.** `add-reference --as face` runs the detector when it is
    installed and refuses a picture it finds no face in. At generation, a
    reference the detector finds no face in falls back to the portrait's
    crop, and the result says so: the owner asked for that face, so
    dropping it without a word is the silently degrading guard.
- **Each reference is a blob,** content-addressed like the portrait, so the
  existing versioning (`history/v<N>.toml`) and lock apply unchanged.
- **The portrait's two checks extend to every blob any entry names:**
  references, a location's images, an outfit's image. Stated over the set,
  not by list.
  - On load, the name check: no path separators, no leading dot, so a
    hand-edited entry can't point outside `blobs/`.
  - On read, the hash check: bytes swapped under an approved hash are
    refused, not sent.

  Today both read `entry.portrait` only, so neither carries over by itself.
- **A candidate never holds a reference** (review of #680). Who may write
  one is the provenance bullet below (amended 2026-10-14): the owner, or a
  model from a clean conversation directly and otherwise staged.
  `image_library_propose` can't add one, and **`add-reference` refuses a
  candidate**: the owner approves the entry first. So:
  - the entry's `status` (§2) still describes everything in it, and its
    `origin` describes its text; each reference carries its own `origin`
    when a model attached it (absent is the owner's own);
  - a reference added to an approved entry is a new version, as an owner
    `update` is (§6). `update`'s "nothing to change"
    comparison includes the references, and the version bump is what changes
    `shown_digest`, so a reference write can't slip past a displayed
    approval;
  - `reject`'s portrait-only clean-up and `MAX_PROPOSED_PORTRAIT_BYTES` stay
    correct as written. A proposal that tried to carry references would be
    refused, not trimmed.
- **`label` is at most 100 characters.** Only a `detail`'s label is ever
  sent; on a `face` or `full_body` it's for browsing. Sent labels count
  against `MAX_COMPILED_PROMPT`, as the description does.
- **Store limits:** at most 1 `face`, 1 `full_body` and 4 `detail` per
  character. That's what the owner can attach, not what a call sends. §9.4
  says how a call fits the generator's slots.

**Two new kinds**, beside `Character` and `Style`. A location's or outfit's
images are `[[reference]]` rows too, with the kind `view`, so every blob an
entry names lives in one table, under one set of checks. Each new kind is
its own directory (`locations/<name>/`, `outfits/<name>/`), since an entry's
kind is derived from its directory (`Entry.kind`).
- **§2's tree gains both.**
  - **Loading** reads the directories this build knows (`Kind::ALL`).
  - **Deleting** a blob is decided by walking every `*.toml` under the store
    root, whatever directory it sits in (review of #680, pass 5).
    `blob_referenced` gates a delete, so it must be fail-closed for kinds
    this build doesn't know: a later build's fifth directory is unseen, not
    unreadable. That walk also reads `lock.toml` and `autolock.toml`; a blob
    name never matches in them, so it's harmless and the safe direction.
  - That walk also covers `removed/`, which keeps a removed entry's blobs
    alive for the manifests that name them (§6), and a blob only a location
    names.

| Kind | Fields | Pictures | Video |
|---|---|---|---|
| `Location` | `text` (≤ 400), 0–3 images (optional until E13 or V-L makes them mean something) | unmeasured as a reference (E13); until then its text goes into the scene | an environment `<Subject>`, "the place in `<Picture k>`" (V-L) |
| `Outfit` | `text` (≤ 300), optional `for` (a character, or none for anyone), optional image | **text only**: pasted into `wearing` | text only: pasted into `wearing` |

- **Outfits stay text.** An image of clothes is the leak §9.1 measured,
  delivered on purpose. The image is for browsing.
- **An outfit's `for` is a suggestion, not a lock.** A scene that dresses
  someone else in it resolves anyway.
  - The note says "that outfit was made for someone else", and **never names
    who**. `for` is never surfaced to a model, since a character name the
    call didn't supply is the roster leak `imagelib::missing` guards (review
    of #680, pass 7).
  - **`for` is owner-written only, like a reference** (pass 26). A model's
    proposed outfit carries no `for`: validating one against the roster
    would answer which names exist, the same leak from the write side.
- **An outfit expands only when it fills `wearing` whole.** A `wearing` that
  only mentions an outfit's name stays the model's text, so the expansion
  always sits inside `wearing`'s own 300 characters.
- **Who writes:** both new kinds follow §1. The owner creates approved
  entries; a model's proposal is a candidate, with no images, as above.
- **A character's references, by provenance** (the owner's ruling of
  2026-10-14, amending L1's "written by the owner only"; asked in a chat to
  save a tattoo picture to a character, the model had no door and retried a
  refused proposal four times). `image_library_reference` attaches or removes
  a `full_body`, `face` or `detail`:
  - from a **clean** conversation, directly: a new version recording the
    model's origin, shown "from a chat" on the character's page, where the
    owner can remove it;
  - from one that has read outside content, or whose taint is unknown, as a
    **staged change** under `pending/` at the store root, applied only when
    the owner approves it in the Waiting pane (`mecha imagelib
    approve-reference`). A reference rides into every later picture of that
    character, so this is learning's provenance split (`docs/ARCHITECTURE.md`,
    learning), not a new rule;
  - from an **incognito** chat, staged locked, on `image_library_propose`'s
    footing (`docs/INCOGNITO-DESIGN.md` R3's third exception).

  A staged picture is held to the proposal cap (`MAX_PROPOSED_PORTRAIT_BYTES`)
  and the pending queue to `MAX_PENDING`; its file protects its blob, a
  rejection deletes the blob once nothing names it, and removing the
  character drops its staged changes.
- **The load-bearing choice: the tool declares `read_only`** (review of
  #692). So no approver is asked: from a clean conversation it writes a new
  approved version of a character, which rides into every later picture,
  with no human in the loop — in a read-only web chat too, so a default chat
  can do what the owner asked in it. `image_library_propose`'s `read_only`
  rests on a candidate reaching no prompt; this one does not, and provenance
  is what bounds it instead. Its brakes: the store's per-kind limits (one
  face, one full body, four details), so a clean conversation can make at
  most six changes before one must be removed; every write is a new version
  with the old one kept in `history/`; the character page's remove; and the
  `[tools]` off switch. A `research` subagent never keeps the tool (it
  declares `destructive`, which `ToolProfile::narrow` reads), and the
  planning phase refuses it in `call`, as `message_send` is refused, since
  the phase gate keys on `read_only`. A direct picture is held to the owner's own cap
  (`MAX_PORTRAIT_BYTES`), a staged one to the proposal cap. A tool cannot see the
  chat's permission mode, so "a read-only chat stages instead" would need
  that plumbed first.
- **Not stored yet:** a location's own clip or soundscape. Nothing would
  read them yet; they arrive with whatever does.

### 9.4 What each generator sends

| Reference | `image_generate` (Qwen-Image) | `video_generate` (H3) |
|---|---|---|
| face | the whole portrait at 512² for a new picture (E10), its crop for an edit, as now; not sent when the camera sees the person from behind and they have a full body (below) | the face crop, as now; from an attached `face` reference when there is one, else from the portrait |
| full_body | **sent** to a new picture of one person, bound to the portrait (E9b; owner, 2026-10-13); faces only for two or more (E14; owner, 2026-10-19), and on edits | **sent** beside the face crop to a cast of **one or two** when everyone has one, **unbound** (both pictures named as one subject, with no face-from/build-from split): "the person in `<Picture 1>` and `<Picture 2>`", not the face-from/build-from wording, the likely cause of V-B's extra person (V-B's retest passed for a cast of two; owner, 2026-10-14) |
| detail | **sent** with its label after every face and body, while five images last (`MAX_PICTURE_IMAGES`): "<Name>'s <label> is shown close up in `<imageK>`" inside the description, the identity images then named by number, and "`<imageK>` shows only <Name>'s <label>, not a person: it is kept exactly where it is, and nowhere else" (V-D for pictures passed; owner, 2026-10-26) | **sent** with its label, "`<Subject n>`'s <label> is shown close up in `<Picture k>`", kept "exactly where it is, and nowhere else" (V-D passed; owner, 2026-10-26) |
| location | its text in the scene | its images as an environment `<Subject>`, if V-L passes |
| outfit | its text as `wearing` | its text as `wearing` |

- **Both consumers keep references on this machine** (review of #680, pass
  4).
  - `image_generate` is loopback-only, `Egress::None`, enforced by
    `loopback_url`.
  - The video tool renders on a local ComfyUI over loopback, H3's open
    weights, not MiniMax's hosted API, and declares no open world.

  That's why sending a portrait, a full body, a labelled mark or a
  location's images costs nothing against the interlock today. A generator
  that isn't local gets no library references without the owner's ruling
  (L6). A hosted backend would send exactly these images off the machine.
- **Every row marked "if" is off until its measurement passes,** and the
  owner rules on the result. A reference a generator doesn't send still
  costs no image slot: the description and labels carry the facts in words.
- **Seen from behind, the full figure stands in for the portrait** (owner,
  2026-10-28). When the camera reads as from behind (`picture::faces_away`:
  behind, rear, a back view, a face turned away; never with a front, a
  shoulder looked over, or a face toward the camera), a person with a
  `full_body` is drawn from it in both identity slots, with no portrait,
  and the prompt says their face is turned away. The person's expression
  and the style's clauses about eyes are left out for that picture.
  Measured on a rear-view Continue:
  - the portrait sent: the face turned to the camera under every wording
    tried, at the base seed and fresh ones, 0 of 12;
  - the full figure alone in one slot: 2 of 5 away, and a second head
    drawn 2 of 5;
  - in both slots: 4 of 5; with the expression and the eye clause gone
    too: 5 of 5, no second head, base seed included.

  Without a `full_body` the portrait still goes and the sentence carries the
  turn. The manifest's `cast` row then has no `portrait`, since none went.
- **Labels have their own budget per person,** at most 300 characters of
  detail labels appended to one description (review of #680, pass 2).
  *Never built for pictures, and superseded there on 2026-10-26:* a label
  now rides only beside its own close-up, so the five-image cap bounds it
  (at most four, of `MAX_LABEL` each), and past `MAX_COMPILED_PROMPT` the
  close-ups go with the bodies, said in counts. The counting rules below
  still hold.
  - Labels past it are dropped, in the character's own order, and the result
    says how many, **never which** (review of #680, pass 20). A label
    describes a person, and `image_generate` declares
    `Capabilities::default()` because nothing private comes back, only a
    path and a seed. A result naming labels would arm `private_data` on
    every picture-drawing conversation. Counts keep the declaration true.
  - **The manifest keeps the same rule** (review of #680, pass 25). It is
    written inside the jail, so `fs_read` reaches it. Its `cast` rows gain
    the hash of each reference image actually sent (a `full_body` beside
    its `portrait`), and never a label: a detail's label is in the prompt
    and nowhere a model can read back, as `clothes_in_act` already carries
    library names only and described people by count.
  - **Labels are the first thing dropped when the compiled prompt would pass
    `MAX_COMPILED_PROMPT`** (review of #680, pass 9). The 9,000 isn't free
    room: at `MAX_CAST`, each person's description, `wearing` and `doing`
    (about 1,010 together) plus a style already take about 6,000.
    - So the compiler drops labels, last person first, until the prompt
      fits, and says how many, never which (the same capability reason). A
      label alone never causes a refusal.
    - If the prompt is still over (the location's text is the setting and is
      kept), the next bullet's message applies.
- **Locations and outfits expand the same way,** from a short name the model
  wrote into owner text. Each is bounded by its own cap:
  - an outfit fills `wearing` under `wearing`'s own `MAX_CAST_FIELD` (300),
    so it substitutes for typed text rather than adding to it;
  - a location's 400 characters are added once per scene.

  So the new pressure is about 1,900 characters: labels plus the location.
- **The prompt can already overflow before §9 adds anything** (review of
  #680, pass 10). The scene's own prose (`PROMPT_CAP` 4,000) and up to four
  extras (about 900 each) sit beside the cast fields and the style, a
  ceiling of about 13,650 against 9,000.
  - So an overflow message names **what filled it, by share**: the scene's
    prose, the extras, or the library text (descriptions, location, labels
    left after dropping).
  - It's never library text by default, and never "shorten the scene" when
    the scene didn't grow.
- **Fitting the slots** (review of #680). A call fills the generator's image
  budget in a fixed order, and says what it left out in the result:
  1. every person's face: the portrait for pictures (E10), the face crop for
     video;
  2. each person's `full_body`, in cast order;
  3. details, in cast order, then in each character's own order, in both
     generators. That's greedy, so it's the mixed shape the next bullets
     describe; V-D measured it and passed for video, then for pictures
     (owner, 2026-10-26). **Built** (`VideoGenerate::extras_for`, and
     `imagelib::compile_with` for pictures, 2026-10-26). A picture planned in
     layers sends none, being drawn from portraits, and says how many. A
     close-up with no label (only a hand edit makes one) is not sent to a
     picture, since a pointer with nothing to bind it by is worse than none;
     video binds it as "mark" (`extras_for`).
  - **Video sends bodies, unbound, to a cast of one or two** (V-B's retest;
    owner, 2026-10-14; the table above), as pictures do: the retest
    measured two people, so three with a body each needs its own
    measurement first (review of #693). **Built** (2026-10-26). Video has
    nine reference slots, less any other reference images (a location, an
    outfit); a pinned opening or closing frame is not one of them, since it
    goes through `MiniMaxH3AddGuide` beside the slots (`videogen::graph`).
    Two people with a body each leave up to five for details. A clip seen through someone's
    eyes (`pov`) sends no bodies: that person is in the cast, with their own
    picture and never a body, so bodies for the others would be the mixed
    shape (review of #744). Three with a body each would
    leave three, once measured; that is not a rule in force.
  - **Bodies are all or nothing in both generators**: in video from V-B's
    retest (owner, 2026-10-14); in pictures from L7's step 2b (review of
    #680, pass
    10). Bodies are sent only when **every person in the cast has one
    attached and they all fit** beside every face; otherwise none is (review
    of #680, pass 13). So a cast where one character has a body and another
    doesn't sends faces only. So a cast of five (5 faces + 5 bodies over 9),
    or four with two pinned images, sends faces only, never four bound
    people beside one unbound.
  - Details also bind a second image to one person, so they're the same
    mixed shape. V-D's arms included one person with a detail beside one
    without, and no one was drawn twice (2026-10-26).
  - **Pictures** keep their people budgets (`MAX_CAST`,
    `EDIT_REFERENCE_BUDGET`, `IMAGE-DESIGN.md` §5.2, "Budgets, as measured")
    and gain an **image** cap (review of #680, pass 6). `MAX_CAST` counts
    people, and the measured ceiling is five reference images, with
    duplicate-free evidence at four (E12).
    - A new picture carries at most five reference images. That's
      `IMAGE-DESIGN.md` §5.2's measurement: five whole portraits drew all
      five people, 3/3. It is the provisional C5 (§3), so if C5 goes back
      to four this cap moves with it (review of #680, pass 22).
    - Faces come first. **Bodies go to a cast of one** (owner, 2026-10-19,
      on E14): two images for one person. It was all or nothing within four
      images, a cast of one or two (review of #680, pass 8), until a cast of
      two with a body each drew someone twice in 6 of 13 live pictures and
      19 of 21 replayed ones, against 2 of 21 from portraits with a closed
      count. A cast of two or more gets no bodies, so a person bound to two
      images beside others bound to one, which no wording covers, still
      never happens.
    - What was left out is said.
    - **The four was established by E9b** (review of #680, passes 7 and
      22), and E14 retired it for pictures; video keeps it. E12 measured
      four *different* faces with no duplicate, while two people with a
      body each is E9's B2 arm, which drew one person twice.
      E9b ran that configuration, a cast of two with a body each, and drew
      two people in both seeds with no grey from the references; pose
      transfer was its one side effect (next bullet). It was judged by
      eye, not ArcFace-scored (`IMAGE-COMPILER-RESEARCH.md`).
    - **An edit sends faces only.** Its canvas plus five crops already fill
      `EDIT_REFERENCE_BUDGET` (6), and E9b's arms are new pictures. An edit
      gets bodies only with a measurement of its own.
  - A cast that can't fit its faces is refused in the scene's own terms, as
    today.
- **A body reference carries its pose too** (E9b; review of #680, pass 19).
  One character's hip-shifted reference stance came through in both seeds of
  the bound arm.
  - So a `full_body` reference should be a neutral, square-on stance, arms
    relaxed. It's guidance the CLI and web page show when one is added,
    since no check can read a stance.
  - The scene's `doing` still states each person's pose, as
    `IMAGE-DESIGN.md` §2.8 requires. A pose the scene states is what the
    picture should follow, with the reference's stance only a fallback.
- **As built in step 2b** (2026-10-13), three choices beside E9b's arm:
  - the binding names the person where E9b's said "her" ("`<image1>` is
    Maya's portrait, `<image2>` Maya's full figure"), since the library
    holds anyone and a description settles no pronoun;
  - the head count stays E12's (no total without extras), not arm B's
    "Exactly two people": a total erased a person the prose described
    (superseded 2026-10-19: E14 closes it, by name, in the foreground, once
    the pictures no longer outnumber the people);
  - a picture the layers switch may build compiles without bodies, so a
    layered build's fallback single pass draws what its cutouts would
    have, and the result says so. If layers then decline (three people, a
    described person, a place they can't split), the single pass is the
    picture and is compiled again with the bodies (review of #685).
- **Pictures with two images per person need new wording.** §3's grammar
  of the time ("each of the N people from the images appears exactly
  once", open until 2026-10-19) assumes one image per person. The compiled
  prompt binds them, naming what each image is. For pictures the first is
  the whole portrait (§9.3), so: "`<image1>` and `<image2>` show one person, Maya: `<image1>` is her
  portrait, `<image2>` her full figure". Naming how each person is shown is
  what E9 found prevents duplicates (review of #680, pass 17). It counts
  people, not images. E9b's bound arm ran with exactly that wording, on a
  cast of two (four images), and passed (review of #680, pass 22).

### 9.5 Surfaces

- **CLI:**
  - `mecha imagelib add-reference <name> --as face|full_body|detail|view
    <file> [--label …]` and `remove-reference`, which keeps the picture in
    `blobs/`: the history still names it, as an `update` keeps an old
    portrait;
  - `add-location`;
  - `add-outfit [--for <character>]`.

  Each writes a new version.
  - **The web routes parse an entry kind once, into `Kind`** (review of
    #680, pass 14). Today `parse_kind` allowlists a string and `visible`
    re-matches it with a Character fallback, so a new kind added to one and
    not the other would resolve a location against a same-named character,
    with every test green. One typed parse lets the compiler find every
    site, as `Config` and `ConfigLayer` rely on.
  - **Every kind-matching site, not only the web's** (review of #680, pass
    20). `tool/image_library.rs`'s `parse_kind` and its schema enum,
    `image_library_propose`'s match and schema enum, the CLI's `KindArg`,
    `serve/library.rs`'s `parse_kind` and `visible`, and `load_entry`'s
    character-only portrait rule all spell kinds by hand today. Each becomes
    an exhaustive match on `Kind`, so a new variant fails the build at every
    one, where today `kind: "location"` would be refused by the listing tool
    that §9.5 says lists it.
  - **Two sites are declarations, not matches, and no match can guard
    them** (review of #680, pass 24). `Kind::ALL` is a hand-written array,
    and it's what `Library::load` walks: a `Location` wired through every
    match but missing there compiles, and every location loads as absent
    with the typed tests green. The CLI's `KindArg` converts *into* `Kind`,
    so a new `Kind` can't make it fail either. Both take
    `charter::SensorKind::ALL`'s fix, the same incident one module over:
    one declaration the list is derived from, plus a test that walks
    `Kind::ALL` and asserts every other list agrees with it (for `KindArg`,
    a `From<Kind>` the compiler checks).
  - **Four more of that shape in `imagelib` itself** (passes 26 and 29):
    `create`'s `_ => {}` wildcard, which a new kind falls through silently,
    and these three. `update`'s two `kind != Kind::Character` guards ("a style
    has no portrait") let a `Location` through with a style's message, since
    `!=` can't fail a build; they become matches. And `default_kind()`'s
    Character fallback types any `Entry` read on a path that forgets to stamp
    its kind from the directory (a `history/v<N>.toml` read, say) as a
    character: `visible`'s defect one module over. Every such read stamps the
    kind, and a test reads each kind's history back.
  - `--kind` keeps its existing meaning, the entry kind that tells apart a
    character and a style sharing a name; `resolve`'s message then names
    every kind holding the name, not just the two.
- **Web:** the character page shows its references in a strip with their
  kinds and labels, and adds or removes one. That's an owner write, through
  the same door as §7's edit page. The Library tab gains Locations and
  Outfits beside Characters and Styles.
  - **The image route widens without widening the lock.** `GET
    /api/library/portrait/{blob}` serves a blob only when some **visible**
    entry names it, for every blob any entry names, whatever the kind.
  - So a locked entry's images stay unfetchable by hash, the leak the #385
    review closed. Every kind carries its own `locked` flag.
  - **Both halves of #385's answer widen together** (review of #680, pass
    30). `serve::library::portrait` closes it twice: the visible-entry
    lookup that answers 404, and the `open` fold that sends
    `Cache-Control: no-store` for a blob only a locked entry names instead
    of a year's `immutable`. Each widens from `entry.portrait` to every
    blob any entry names, with the `!e.locked` fold kept, or a locked
    entry's `full_body` lands in the browser cache for a year.
- **Blob size:** owner-written blobs of every kind follow the owner
  portrait's existing rule. `MAX_PROPOSED_PORTRAIT_BYTES` stays a
  proposal-only cap, since candidates carry no new blobs.
- **Names offered to a model** (review of #680, pass 6). Locations and
  outfits follow styles, not characters:
  - their **approved, unlocked** names are listed to the model as written,
    sorted, so the list is stable in the cached prefix. That's as
    `style_names` is, whose order comes from `Library::load`'s sort by kind
    and name. A location or outfit name that contains a character's name
    would expose it, so the owner's doors warn when one does, **in both
    orders** (review of #680, pass 23): adding **or approving** a location
    or outfit whose name contains an existing character's (pass 25:
    approve is the only door a model-proposed name passes where the check
    can fire, since saying so at propose would name the roster to the
    model, as `imagelib::missing` refuses to), and adding or approving a
    character whose name an existing location or outfit contains. The
    second is the likelier order, and the one a one-way check misses. A
    warning
    rather than a refusal is deliberate: the owner is the authority on their
    own names, where `imagelib::missing`'s roster rule guards against a
    model;
  - so is the quarantined extraction;
  - a locked one isn't in that passive list, as a locked style isn't
    (`style_names`). That's the existing behaviour, not a new hiding.
  - **The lock is for browsing** (§1, §4; the owner reaffirmed it
    2026-10-13, after passes 10–17 of this review drifted to treating it as
    model-facing).
    - A locked location or outfit resolves for generation, as every locked
      entry does.
    - `image_library` lists it, and the manifest beside a picture names it,
      as both do for characters and styles today. Pictures already in chats
      are untouched by a lock (§1).
    - What the lock hides is the owner's browsing without the unlock: the
      Library tab, the image route (below), and the save dialog's `source`,
      which returns a yes or no and never names (#385).
- **Where the lists sit:** the location and outfit names are appended after
  the styles sentence at the end of the description (`imagegen::described`).
  What keeps the cached prefix stable is that the description is built when
  the tool is built, not per turn: in `ImageGenerate::new` (and
  `with_library_dir`), and again in `persona_form` whenever a persona's
  agent is rebuilt (on a persona edit, or a new binding).
  - So a new location reaches a running chat on a restart or a persona
    rebuild. Each of those already re-pays the prefix.
  - A rebuild per turn would re-pay it wherever the lists sit (review of
    #680, passes 16 and 19).
- **Tools:**
  - `image_library` lists the new kinds as it lists characters and styles:
    approved entries, locked ones included (§4: "the lock is for browsing");
  - a scene names a location in `setting.location`, and a person's `wearing`
    may name an outfit.

  The compiler resolves both, and an unknown name stays plain text.
- **How a scene record stores a location** (review of #680).
  `scene::Setting` is a closed enum read leniently: an unknown variant reads
  as `Setting::Unknown`, untrusted and drawing nothing. So a location isn't
  a new variant. It's recorded as today's `Words`, holding **the model's own
  words** for the place (at least the name it wrote), plus an optional
  `location` name.
  - **Never the library text** (review of #680, pass 11). `scene::Scene`
    keeps names, not library text, for characters (`Who::Library`) and
    styles, so a restage picks up the owner's later edits instead of a
    snapshot. `scene::note` renders `Words` back into a persona's run notes
    every turn (`NOTE_PLACE_CHARS` 240), so a location's 400 characters
    there would also crowd the note.
  - The compiler resolves the name to the owner's text at render time, as it
    does a description and a style. So an edit to the location reaches the
    next restage.
  - Outfits are the same: `wearing` records what the model wrote, and the
    outfit's text is resolved at compile time.
  - `scene::note` renders the words, never the `location` field, so the note
    stays the model's own words.
  - A build that predates the field reads the model's words and loses only
    the link.
- **Untrusted text naming a location.** The quarantined extraction
  (`IMAGE-DESIGN.md` §5.3) would learn the field. A location name from an
  untrusted-origin record resolves like any name: it selects owner-approved
  words, and adds none of its own.
  - It's the same rule as a non-library `who` being drawn from words
    (`IMAGE-DESIGN.md` §8).
  - The record keeps the origin of the write that set it, as every
    `scene::Field` does; an unreadable setting stays `Setting::Unknown` and
    untrusted.

### 9.6 Measurements, and what each gates

The arms and pass bars are in `IMAGE-COMPILER-RESEARCH.md` §8: measured
ones above its **Still open** list, open ones in it (review of #680, pass
26: evidence lives there, decisions here). This section
only says what each one gates:

| Id | Gates |
|---|---|
| **E9b** | `full_body` for pictures (§9.4), and the two-images-one-person wording |
| **E14** | `full_body` for pictures of one person only, and the closed foreground count without extras (§9.4; passed 2026-10-18 for the strict wording, 2/21 drawn twice; the foreground one measured 2026-10-19, 3/21, crowds unmeasured) |
| **V-B's retest** | `full_body` for video: **passed** 2026-10-14, unbound (V-B itself failed, 2026-10-13) |
| **V-D** | `detail` images for video: **passed**, owner 2026-10-26 (one same-seed pair, a cast of one person with a detail beside one without: the tattoo on both forearms without the picture, on the left forearm only with it; no one drawn twice) |
| **V-D, pictures** | `detail` images for pictures: **passed**, owner 2026-10-26. Two recorded pictures, a cast of two with a close-up for one, each re-rendered at its own seed: a ribcage tattoo given in words alone came out as a different design on both sides, 2/2; with the close-up it was the right design on the right side only, 2/2, first with hand-written wording and then with the compiler's own (`compile_with`'s prompt checked byte for byte against the recorded one with close-ups off). No one drawn twice |
| **E13** | a location reference for pictures |
| **V-L** | a location reference for video |

E9b and V-B use the same reference set, so one set of images answers both
generators.

### 9.7 Rulings

**Ruled by the owner, 2026-10-13: L1–L7 as recommended below** ("let's go
with your recommendations for now"). E9b and V-B run on the owner's own
characters. Only numbers and a generic description of them enter this
repository, never their names or images.

- **L1, the store shape:**
  - `[[reference]]` with `kind`, `blob` and `label` (≤ 100 characters),
    written by the owner, or by a model as §9.3's provenance split allows
    (amended 2026-10-14): `face | full_body | detail` on characters,
    `view` on locations and outfits;
  - the `Location` and `Outfit` kinds as their own directories, with deletes
    decided by a walk of the whole store root;
  - every blob any entry names getting the portrait's name check on load and
    hash check on read;
  - `add-reference` refused on a candidate;
  - the image route serving only blobs a visible entry names, with the entry
    kind parsed once into `Kind`;
  - `create`'s wildcard and `update`'s two `kind != Kind::Character` guards
    made exhaustive matches over `Kind` (`validate_text` already is), for the
    same reason; `default_kind()`'s Character fallback held by every read
    stamping the kind from its directory, since a wrong kind defeats the
    "wrong entry kind is unknown" rule (§9.5; review of #680, pass 29);
    `Kind::ALL` and `KindArg`, which no match can guard, held to `Kind` by a
    test (§9.5);
  - unknown kinds, and known kinds on the wrong entry kind, never sent;
  - a location stored in a scene record as words plus its name, the name
    field `#[serde(default)]` so older records still parse as `Words`.
- **L2, the portrait stays canonical:** no migration. The face is its crop,
  and existing manifests stay valid.
- **L3, each generator's references gated on its measurement** (§9.4), with
  the owner ruling on each result, and the slot-filling order of §9.4.
- **L4, outfits are text:** an image is for browsing only. **Reopened for
  video by the owner, 2026-10-25**: a picture of the garment alone (never
  a person wearing it) is sent as the wearer's clothes, after one measured
  pair — the same seed and scene, words only against words plus the
  garment picture — held the garment across a cut where the words let it
  drift into two other designs. Pictures keep outfits as text until a
  measurement of their own.
- **L5, names offered to a model:** location and outfit names appear in
  `image_generate`'s description as styles do (approved, unlocked);
  `image_library` lists them, locked included, as it lists everything (§4);
  characters stay unlisted in the description, and an outfit's `for` is
  never named to a model. The lock is for browsing (§1), reaffirmed
  2026-10-13.
  - **Its cost** (review of #680, passes 26 and 29). `docs/TRIFECTA.md`
    doesn't yet mention the library, styles' 2026-10-08 ruling included, so
    L7's step 4 adds the row there, in its switches-and-costs table, for style
    and location names together. The 2026-10-08 ruling it inherits was about
    style names, a rendering idiom; a location name may be a place the owner
    goes. The description sits in every chat that can draw, persona and
    incognito included, with nothing armed, since `image_generate` declares
    `Capabilities::default()`. So an unarmed conversation can read every
    approved, unlocked location name from its own prefix and hand it to a
    `Chosen` sender. Locking a location is the owner's lever for a name they
    don't want listed.
- **L6, references stay local:** a generator whose backend isn't on this
  machine (loopback) is given no library references (face, body, detail,
  location) without the owner's ruling for that generator. **The two
  generators differ** (review of #680, passes 18, 21 and 30). For
  `image_generate` it's already held at the URL door: `Client::for_config`
  refuses a non-loopback server through `imagegen::loopback_url` before a
  client exists, so no reference can reach one. A reference-read check
  would be a branch that cannot be taken; it becomes one only if a
  non-loopback picture backend is ever ruled in. For the video tool it's a
  convention until the port: that tool is built outside this repository and
  calls none of this read path, so the check is on its port checklist, and
  until then video is held to L6 by the owner's private server alone, not by
  mecha.
- **L7, the order:**
  1. E9b and V-B, one session with shared references;
  2. the references, in two halves (review of #680, pass 23):
     - **2a,** the store and CLI for `face`, `full_body` and `detail`.
       mecha reads no `face` reference: it is the video tool's crop source,
       and pictures' edit crop stays the portrait's until measured (§9.3;
       review of #680, passes 17 and 27);
     - **2b,** the picture compiler sending `full_body` (§9.4): the
       two-images-one-person binding, all-or-nothing bodies, the image cap,
       edits on faces only. 2a alone would leave an attached body stored
       and unused, so the two ship back to back;
  3. V-D;
  4. locations: the kind ships **text-only**, like styles; E13 and V-L then
     gate only its images (review of #680, pass 11). The `docs/TRIFECTA.md`
     row for listed library names (L5's cost) ships with it (pass 29);
  5. outfits, last, being text and the cheapest.

**Ruled by the owner, 2026-10-19: E14's D ships** ("let's go with d for
now"). A new picture of two or more library characters sends their
portraits only, and without extras the cast is counted closed, by name, in
the foreground, with anyone in the background a stranger (§3, §9.4): the
count is whoever the scene lists, never a cap of two. The strict "no one
else is in it" was measured; the owner chose the foreground wording so a
busy setting keeps its crowd. The layers pipeline (`IMAGE-DESIGN.md` §15) stays the
per-persona switch for when no doubling is acceptable at its cost in time.
MOSS-Image is explored outside this repository.
