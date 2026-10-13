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
  until a location reference is measured. *(Reopened by §9.2: locations as a
  kind, gated by E13 and V-L.)*
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
  *(Reopened by §9.2: a `full_body` reference, gated by E9b and V-B.)*
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
  (E2, E9: every slot tends to become a person). *(§9.4 adds two images per
  person, all or nothing, if E9b passes.)* One person is "the person in
  the image"; two or more are `<image1>…<imageN>` in `cast` order, "from left
  to right", and "each of the N people from the images appears exactly once;
  anyone else the scene describes is a new person". No total: a total with
  no extras erased a waiter the prose described, and without it four cast
  still drew exactly four (E12). With `extras`, a total that counts them.
- **`wearing` and `doing` are required** (E3, E8: a reference supplies its own
  outfit, pose and stare when the scene is silent; stated, they land), except
  that `doing` is left empty when the scene's `together` says what its people
  do: a plain pose beside it drew a person twice (mecha-a3, 2026-10-08), and
  `imagegen::doing_words` says the plain pose only when the scene is silent.
- **References are sent at 512²** (E2: four at 1024² cost 190 s, four at 512²
  79 s; E10: a whole portrait at 512² holds identity). `Request` gains a
  backend-neutral `reference_size`; plain edits keep 1024.
- **At most four cast members** (E3 held four in one pass). Since raised to `MAX_CAST` 5 (`IMAGE-DESIGN.md` §5.2).
- **Anyone else is an `extra`, and is counted** (E11). A waiter described in
  the scene beside a cast of three, with "Exactly three people" compiled in,
  was kept in the picture but pushed into the background; listed as
  "Also in the scene, not from any image: …" with a head count of four — "the
  three from the images, each exactly once, and one new person" — he stood
  where the scene put him. Extras carry no reference, so the four-reference
  ceiling does not bound them; `MAX_EXTRAS` is four, unmeasured beyond. They
  are read by the named-character guard too: "John waving" as an extra is
  John drawn from words.
- **A cast name the library does not hold is drawn as an extra** (owner,
  2026-09-30: a named character not in the library "blocks image generation
  when it shouldn't"). `imagelib::demote_unknown` moves it, with its
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
  read-only and proposing a character should be one request in it.

No tool description changes per turn; the library is never listed in the
system prompt (the cached prefix). An incognito chat may call
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

- **Locations, scene assets, lineage and branching** (draft §6–7, §20). §9 proposes the
  library half of this: typed character references, locations and wardrobe.
- **Tier B checks** (face embedding, detector, VLM) and the repair loop.
- **Cast plus an edit canvas in one call** — needs a per-reference
  resolution (ComfyUI's `resolution = 0` keeps each at its own size), which
  wants its own measurement.
- **A prompt rewriter** (E1b, unmeasured).
- **A crop box** — optional on the character page in phase 2 (E10: a small
  gain).

## 9. Library v2: typed references, locations and wardrobe (ruled 2026-10-13)

**Status:** ruled 2026-10-13 (L1–L7, §9.7); the owner approved the direction the same day (the video design's R6). Nothing in this section is built. Step 1, E9b and V-B, is next. It reopens two of §1's settled rulings with new evidence, and says which, rather than re-arguing them silently.

### 9.1 Why now

- **A second consumer.** The video tool (`video_generate`, MiniMax H3; built outside this repository for now, with its design moving in when it's ported) reads the same library. H3's full-reference mode binds several pictures to one person natively, "`<Subject 1>` … whose appearance comes from `<Picture 1>` and `<Picture 2>`". It has up to nine image slots, three voice slots and three video slots.
- **What the video measurements showed:** V0 in `IMAGE-COMPILER-RESEARCH.md` (2026-10-13). A whole portrait leaked its outfit into clips, a face crop stopped it, and clothing stated per clip landed.
- **What text alone doesn't carry well.** A distinguishing mark (a forearm tattoo came out heavier than "fine-line") and build: E9 never got to test build, because its trench coat hid it.
- **The owner's ask:** several typed images per character (face, full body, details), a location library, and named outfits, shared by pictures and video.

### 9.2 What it reopens

- **§1, "a character is a portrait plus a description … no body reference (E9), no required crop (E10)."**
  - Kept for pictures, until E9b says otherwise.
  - Video takes face crops now, and a body reference becomes a measurement for it too (§9.6).
- **§1, "locations stay free text in the scene until a location reference is measured."** Kept as the rule for **references**, the images. §9.6 is that measurement, for each generator. A text-only `Location` kind is still text in the scene: named owner text resolved into the setting, as a style is. So it ships without waiting for E13 or V-L, which gate only a location's images.

### 9.3 The store

A character entry gains typed references beside its portrait. The portrait stays: it's what the Library tab shows, what `face::anchor_crop` crops, and what every existing manifest names.

```
characters/<name>/entry.toml
  portrait  = "sha256-….jpg"           # unchanged: the canonical picture
  [[reference]]
  kind  = "full_body"                  # face | full_body | detail
  blob  = "sha256-….png"
  label = "standing, front view"                     # optional here (browsing only); required on a detail, e.g. "left-forearm tattoo"
```

- **`kind` is a closed enum in an on-disk store, so it's a wire format.** An unknown `kind` loads, is shown in the library, and is **never sent to a generator**.
  - So is a known kind on the wrong entry kind: a `face` on a location, or a `view` on a character. Each entry kind allows its own kinds, and anything else is read as unknown, so a hand-edited entry can't put a picture where a person's face goes. That's the fail-closed reading: a reference nobody can classify carries nothing into a picture.
- **No `face` entry is needed today.** The face is the portrait's crop, cached by blob.
  - A `face` reference is for the owner who wants a better face than the portrait shows. At most one per character.
  - When attached, it replaces the portrait as the source of video's face crop. Pictures keep sending the whole portrait (E10), since they have no crop to replace; using a `face` reference there would want its own measurement. The portrait stays the picture the library shows.
- **Each reference is a blob,** content-addressed like the portrait, so the existing versioning (`history/v<N>.toml`) and lock apply unchanged.
- **The portrait's two checks extend to every blob any entry names:** references, a location's images, an outfit's image. Stated over the set, not by list.
  - On load, the name check: no path separators, no leading dot, so a hand-edited entry can't point outside `blobs/`.
  - On read, the hash check: bytes swapped under an approved hash are refused, not sent.

  Today both read `entry.portrait` only, so neither carries over by itself.
- **Only the owner writes references** (review of #680): the CLI and the web page. `image_library_propose` can't add one, and **`add-reference` refuses a candidate**: the owner approves the entry first. So a candidate never holds a reference, and:
  - the entry's one `origin` and `status` (§2) still describe everything in it;
  - a reference added to an approved entry is an owner write and a new version, as an owner `update` is (§6). `update`'s "nothing to change" comparison includes the references, and the version bump is what changes `shown_digest`, so a reference write can't slip past a displayed approval;
  - `reject`'s portrait-only clean-up and `MAX_PROPOSED_PORTRAIT_BYTES` stay correct as written. A proposal that tried to carry references would be refused, not trimmed.
- **`label` is at most 100 characters.** Only a `detail`'s label is ever sent; on a `face` or `full_body` it's for browsing. Sent labels count against `MAX_COMPILED_PROMPT`, as the description does.
- **Store limits:** at most 1 `face`, 1 `full_body` and 4 `detail` per character. That's what the owner can attach, not what a call sends. §9.4 says how a call fits the generator's slots.

**Two new kinds**, beside `Character` and `Style`. A location's or outfit's images are `[[reference]]` rows too, with the kind `view`, so every blob an entry names lives in one table, under one set of checks. Each new kind is its own directory (`locations/<name>/`, `outfits/<name>/`), since an entry's kind is derived from its directory (`Entry.kind`).
- **§2's tree gains both.**
  - **Loading** reads the directories this build knows (`Kind::ALL`).
  - **Deleting** a blob is decided by walking every `*.toml` under the store root, whatever directory it sits in (review of #680, pass 5). `blob_referenced` gates a delete, so it must be fail-closed for kinds this build doesn't know: a later build's fifth directory is unseen, not unreadable.
  - That walk also covers `removed/`, which keeps a removed entry's blobs alive for the manifests that name them (§6), and a blob only a location names.

| Kind | Fields | Pictures | Video |
|---|---|---|---|
| `Location` | `text` (≤ 400), 0–3 images (optional until E13 or V-L makes them mean something) | unmeasured as a reference (E13); until then its text goes into the scene | an environment `<Subject>`, "the place in `<Picture k>`" (V-L) |
| `Outfit` | `text` (≤ 300), optional `for` (a character, or none for anyone), optional image | **text only**: pasted into `wearing` | text only: pasted into `wearing` |

- **Outfits stay text.** An image of clothes is the leak §9.1 measured, delivered on purpose. The image is for browsing.
- **An outfit's `for` is a suggestion, not a lock.** A scene that dresses someone else in it resolves anyway.
  - The note says "that outfit was made for someone else", and **never names who**. `for` is never surfaced to a model, since a character name the call didn't supply is the roster leak `imagelib::missing` guards (review of #680, pass 7).
- **An outfit expands only when it fills `wearing` whole.** A `wearing` that only mentions an outfit's name stays the model's text, so the expansion always sits inside `wearing`'s own 300 characters.
- **Who writes:** both new kinds follow §1. The owner creates approved entries; a model's proposal is a candidate, with no images, as above.
- **Not stored yet:** a location's own clip or soundscape. Nothing would read them yet; they arrive with whatever does.

### 9.4 What each generator sends

| Reference | `image_generate` (Qwen-Image) | `video_generate` (H3) |
|---|---|---|
| face | the portrait at 512², as now (E10) | the face crop, as now; from an attached `face` reference when there is one, else from the portrait |
| full_body | **not sent** until E9b passes | sent beside the face, bound as one `<Subject>`, if V-B passes |
| detail | not sent; its `label` is appended to the description | sent with its label ("`<Picture k>` shows her left-forearm tattoo"), if V-D passes |
| location | its text in the scene | its images as an environment `<Subject>`, if V-L passes |
| outfit | its text as `wearing` | its text as `wearing` |

- **Both consumers keep references on this machine** (review of #680, pass 4).
  - `image_generate` is loopback-only, `Egress::None`, enforced by `loopback_url`.
  - The video tool renders on a local ComfyUI over loopback, H3's open weights, not MiniMax's hosted API, and declares no open world.

  That's why sending a portrait, a full body, a labelled mark or a location's images costs nothing against the interlock today. A generator that isn't local gets no library references without the owner's ruling (L6). A hosted backend would send exactly these images off the machine.
- **Every row marked "if" is off until its measurement passes,** and the owner rules on the result. A reference a generator doesn't send still costs no image slot: the description and labels carry the facts in words.
- **Labels have their own budget per person,** at most 300 characters of detail labels appended to one description (review of #680, pass 2).
  - Labels past it are dropped, in the character's own order, and the result says so.
  - **Labels are the first thing dropped when the compiled prompt would pass `MAX_COMPILED_PROMPT`** (review of #680, pass 9). The 9,000 isn't free room: at `MAX_CAST`, each person's description, `wearing` and `doing` (about 1,010 together) plus a style already take about 6,000.
    - So the compiler drops labels, last person first, until the prompt fits, and says which. A label alone never causes a refusal.
    - If the prompt is still over (the location's text is the setting and is kept), the next bullet's message applies.
- **Locations and outfits expand the same way,** from a short name the model wrote into owner text. Each is bounded by its own cap:
  - an outfit fills `wearing` under `wearing`'s own `MAX_CAST_FIELD` (300), so it substitutes for typed text rather than adding to it;
  - a location's 400 characters are added once per scene.

  So the new pressure is about 1,900 characters: labels plus the location.
- **The prompt can already overflow before §9 adds anything** (review of #680, pass 10). The scene's own prose (`PROMPT_CAP` 4,000) and up to four extras (about 900 each) sit beside the cast fields and the style, a ceiling of about 13,650 against 9,000.
  - So an overflow message names **what filled it, by share**: the scene's prose, the extras, or the library text (descriptions, location, labels left after dropping).
  - It's never library text by default, and never "shorten the scene" when the scene didn't grow.
- **Fitting the slots** (review of #680). A call fills the generator's image budget in a fixed order, and says what it left out in the result:
  1. every person's face: the portrait for pictures (E10), the face crop for video;
  2. each person's `full_body`, in cast order;
  3. details, in cast order, then in each character's own order.
  - **Video** has nine slots, less any pinned or other reference images. Two people fit with a body each and up to five details; three fit with a body each and three details.
  - **Bodies are all or nothing in both generators** (review of #680, pass 10). If every person's `full_body` doesn't fit beside every face, none is sent. So a cast of five (5 faces + 5 bodies over 9), or four with two pinned images, sends faces only, never four bound people beside one unbound.
  - Details also bind a second image to one person, so they're the same mixed shape. V-D's arms must include one person with a detail beside one without, before details ship.
  - **Pictures** keep their people budgets (`MAX_CAST`, `EDIT_REFERENCE_BUDGET`, `IMAGE-DESIGN.md` §5.2, "Budgets, as measured") and gain an **image** cap (review of #680, pass 6). `MAX_CAST` counts people, and the measured ceiling is five reference images, with duplicate-free evidence at four (E12).
    - A new picture carries at most five reference images. That's `IMAGE-DESIGN.md` §5.2's measurement: five whole portraits drew all five people, 3/3.
    - Faces come first. **Bodies are all or nothing** (review of #680, pass 8): every person gets their `full_body`, or none does, and only while the total stays at four or fewer. So that's a cast of one or two. A cast of three gets no bodies, never one person bound to two images beside others bound to one, which no wording covers and nothing has measured.
    - What was left out is said.
    - **The four is E9b's to establish, not settled** (review of #680, pass 7). E12 measured four *different* faces with no duplicate, while two people with a body each is E9's B2 arm, which drew one person twice. So that configuration is exactly what E9b must pass.
    - **An edit sends faces only.** Its canvas plus five crops already fill `EDIT_REFERENCE_BUDGET` (6), and E9b's arms are new pictures. An edit gets bodies only with a measurement of its own.
  - A cast that can't fit its faces is refused in the scene's own terms, as today.
- **Pictures with two images per person need new wording.** §3's grammar ("each of the N people from the images appears exactly once") assumes one image per person. If E9b passes, the compiled prompt binds them: "`<image1>` and `<image2>` show one person, Maya: her face and her build". It counts people, not images. E9b's bound arm must run with that wording, with a cast of two (four images).

### 9.5 Surfaces

- **CLI:**
  - `mecha imagelib add-reference <name> --as face|full_body|detail|view <file> [--label …]` and `remove-reference`;
  - `add-location`;
  - `add-outfit [--for <character>]`.

  Each writes a new version. `--kind` keeps its existing meaning, the entry kind that tells apart a character and a style sharing a name; `resolve`'s message then names every kind holding the name, not just the two.
- **Web:** the character page shows its references in a strip with their kinds and labels, and adds or removes one. That's an owner write, through the same door as §7's edit page. The Library tab gains Locations and Outfits beside Characters and Styles.
  - **The image route widens without widening the lock.** `GET /api/library/portrait/{blob}` serves a blob only when some **visible** entry names it, for every blob any entry names, whatever the kind.
  - So a locked entry's images stay unfetchable by hash, the leak the #385 review closed. Every kind carries its own `locked` flag.
- **Blob size:** owner-written blobs of every kind follow the owner portrait's existing rule. `MAX_PROPOSED_PORTRAIT_BYTES` stays a proposal-only cap, since candidates carry no new blobs.
- **Names offered to a model** (review of #680, pass 6). Locations and outfits follow styles, not characters:
  - their **approved, unlocked** names are listed to the model as written, sorted, so the list is stable in the cached prefix. That's as `style_names` is, whose order comes from `Library::load`'s sort by kind and name. A location or outfit name that contains a character's name would expose it, so the owner's doors warn when one does. A warning rather than a refusal is deliberate: the owner is the authority on their own names, where `imagelib::missing`'s roster rule guards against a model;
  - so is the quarantined extraction;
  - a locked one isn't listed, which is how the owner hides a place.
  - A guessed locked name still resolves for generation, as §1 rules for all locked entries, but the result never names it back, and the call reads the same as for a name the library doesn't hold. So the lock hides the place from the model, not only from the list.
  - **The picture's manifest is a result too** (review of #680, pass 12). `write_manifest` writes `images/<stem>.json` inside the chat's folder, beside the picture, and today it names every entry used, by name and version (`Compiled.used`). A model can read it, so it already confirms a guessed locked style.
    - Under §9, the in-folder manifest names only unlocked entries. A locked one is recorded as `{kind, locked: true, version}` with no name.
    - The full record, name and version, goes to a usage log in the library store, outside any chat's folder, where the owner's audit reads it.
    - That applies to every kind, styles and characters included, so it closes today's channel along with the new ones.

  Characters stay unlisted ("their names are the owner's"). A model can only write a location it knows the name of (L5).
- **Where the lists sit:** the location and outfit names are appended after the styles sentence at the end of the description (`imagegen::described`), so a new location moves no earlier byte of the cached prefix.
- **Tools:**
  - `image_library` lists the new kinds;
  - a scene names a location in `setting.location`, and a person's `wearing` may name an outfit.

  The compiler resolves both, and an unknown name stays plain text.
- **How a scene record stores a location** (review of #680). `scene::Setting` is a closed enum read leniently: an unknown variant reads as `Setting::Unknown`, untrusted and drawing nothing. So a location isn't a new variant. It's recorded as today's `Words`, holding **the model's own words** for the place (at least the name it wrote), plus an optional `location` name.
  - **Never the library text** (review of #680, pass 11). `scene::Scene` keeps names, not library text, for characters (`Who::Library`) and styles, and `scene::note` renders `Words` back into a persona's run notes every turn (`NOTE_PLACE_CHARS` 240). Recorded library text would hand a locked location's description back to the model, the very thing L5 says can't happen.
  - The compiler resolves the name to the owner's text at render time, as it does a description and a style. So an edit to the location reaches the next restage.
  - Outfits are the same: `wearing` records what the model wrote, and the outfit's text is resolved at compile time.
  - A build that predates the field reads the model's words and loses only the link.
- **Untrusted text naming a location.** The quarantined extraction (`IMAGE-DESIGN.md` §5.3) would learn the field. A location name from an untrusted-origin record resolves like any name: it selects owner-approved words, and adds none of its own.
  - It's the same rule as a non-library `who` being drawn from words (`IMAGE-DESIGN.md` §8).
  - The record keeps the origin of the write that set it, as every `scene::Field` does; an unreadable setting stays `Setting::Unknown` and untrusted.

### 9.6 Measurements, before anything is built

The arms and pass bars are in `IMAGE-COMPILER-RESEARCH.md` under **Still open** (review of #680: evidence lives there, decisions here). This section only says what each one gates:

| Id | Gates |
|---|---|
| **E9b** | `full_body` for pictures (§9.4), and the two-images-one-person wording |
| **V-B** | `full_body` for video |
| **V-D** | `detail` images for video |
| **E13** | a location reference for pictures |
| **V-L** | a location reference for video |

E9b and V-B use the same reference set, so one set of images answers both generators.

### 9.7 Rulings

**Ruled by the owner, 2026-10-13: L1–L7 as recommended below** ("let's go with your recommendations for now"). E9b and V-B run on the owner's own characters. Only numbers and a generic description of them enter this repository, never their names or images.

- **L1, the store shape:**
  - `[[reference]]` with `kind`, `blob` and `label` (≤ 100 characters), written by the owner only: `face | full_body | detail` on characters, `view` on locations and outfits;
  - the `Location` and `Outfit` kinds as their own directories, with deletes decided by a walk of the whole store root;
  - every blob any entry names getting the portrait's name check on load and hash check on read;
  - `add-reference` refused on a candidate;
  - the image route serving only blobs a visible entry names;
  - unknown kinds, and known kinds on the wrong entry kind, never sent;
  - a location stored in a scene record as words plus its name, the name field `#[serde(default)]` so older records still parse as `Words`.
- **L2, the portrait stays canonical:** no migration. The face is its crop, and existing manifests stay valid.
- **L3, each generator's references gated on its measurement** (§9.4), with the owner ruling on each result, and the slot-filling order of §9.4.
- **L4, outfits are text:** an image is for browsing only.
- **L5, names offered to a model:** location and outfit names are listed like styles, approved and unlocked only; characters stay unlisted, and an outfit's `for` is never named to a model. A locked location or outfit still resolves for generation (§1: generation always has access to locked entries), but **a result never names a locked entry**, neither the tool's text nor the manifest beside the picture, so a model can't tell "locked" from "absent" by calling (review of #680, passes 10 and 12).
- **L6, references stay local:** a generator whose backend isn't on this machine (loopback) is given no library references (face, body, detail, location) without the owner's ruling for that generator.
- **L7, the order:**
  1. E9b and V-B, one session with shared references;
  2. the store and CLI for `full_body` and `detail`;
  3. V-D;
  4. locations: the kind ships **text-only**, like styles; E13 and V-L then gate only its images (review of #680, pass 11);
  5. outfits, last, being text and the cheapest.
