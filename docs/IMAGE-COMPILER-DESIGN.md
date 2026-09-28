# Image compiler — design

**2026-09-28. Phase 1 building on `feat/image-library`.** What a character
library for `image_generate` is, how a scene compiles against it, and what
the first build deliberately leaves out. The evidence is
`IMAGE-COMPILER-RESEARCH.md` (E1–E10 there, cited below by number); this
document holds the decisions and does not re-argue them.

## 1. The owner's rulings (2026-09-28)

Settled; the build follows them and does not re-ask.

- **v1 is characters and styles.** Locations stay free text in the scene
  until a location reference is measured.
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
- **The web surface ships second**, minimal: save to library, browse, lock.
- **The lock is a browsing filter, not an access control.** Generation
  always has access to locked entries. Things made from a locked entry start
  locked, and any item can be unlocked individually.
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
- **One reference per character, left to right, and the head count stated**
  (E2, E9: every slot tends to become a person). One person is "the person in
  the image"; two or more are `<image1>…<imageN>` in `cast` order, "from left
  to right", with "Exactly N people in the image."
- **`wearing` and `doing` are required** (E3, E8: a reference supplies its own
  outfit, pose and stare when the scene is silent; stated, they land).
- **References are sent at 512²** (E2: four at 1024² cost 190 s, four at 512²
  79 s; E10: a whole portrait at 512² holds identity). `Request` gains a
  backend-neutral `reference_size`; plain edits keep 1024.
- **At most four cast members** (E3 held four in one pass).
- **The style's text is appended verbatim**, never paraphrased.
- **`cast` and `reference_images` are exclusive in v1.** ComfyUI's encoder
  takes one reference resolution per call, so an edit canvas at 1024² and
  portraits at 512² cannot share one; editing a cast image works already by
  passing the image, whose people carry their own identity.
- **A cast generation defaults to square**, not to its first reference's
  shape — a portrait is not a canvas.
- **The same-seed rule, narrowed and kept.** An edit still always samples at
  a fresh seed. A cast generation *keeps* the model's seed — that is how a
  scene is revised with its composition — unless it equals a cast member's
  `source_seed`, which is replaced and said.

## 4. Tools

- **`image_generate`** — `cast` and `style` as above. The result names each
  entry and its version. **A prompt naming an approved character with no
  `cast` key is refused** before any GPU time: the first real run (2026-09-28)
  copied the looked-up descriptions into the prompt, left `cast` out, and drew
  two strangers. `"cast": []` means "someone else by that name".
- **`image_library`** — list or search the library, with a line saying how
  entries are used (names in `cast`, descriptions never copied into the
  prompt). **Returns approved entries only**, and so declares `Capabilities::default()`, on `skill`'s
  footing: every approved entry's text crossed the owner. Candidates never
  reach the model (research §5: the loop taints on `caps.untrusted_input &&
  out.external`, per tool, so returning them would arm every lookup).
  Locked entries are listed — the lock is for browsing.
- **`image_library_propose`** — stage a candidate character (a name, a
  description, and a workspace image as the portrait, read through the path
  jail) or style. Records `origin` from the conversation's taint
  (`ToolCtx::taint`; `None` is `model_untrusted`). Refuses a name already in
  use in any state, and refuses beyond 50 pending candidates. It creates only
  a candidate, which changes nothing the owner uses until approved — so it is
  `read_only` on the footing `image_generate` has: a web chat starts
  read-only and proposing a character should be one request in it.

No tool description changes per turn; the library is never listed in the
system prompt (the cached prefix). An incognito chat may call
`image_library` and generate with a cast — both only read — and
`image_library_propose` is withheld there, because it writes outside the
room.

## 5. Manifests

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
  before it can ride into prompts.
- `reject <name>`, `lock <name>`, `unlock <name>`, `update <name> …`.

## 7. Not in phase 1

- **The web surface** — Save to library, the Library tab, the character page,
  the "show locked" toggle, candidates in `/queues`. Phase 2, from §1's
  rulings.
- **Locations, scene assets, lineage and branching** (draft §6–7, §20).
- **Tier B checks** (face embedding, detector, VLM) and the repair loop.
- **Cast plus an edit canvas in one call** — needs a per-reference
  resolution (ComfyUI's `resolution = 0` keeps each at its own size), which
  wants its own measurement.
- **A prompt rewriter** (E1b, unmeasured).
- **A crop box** — optional on the character page in phase 2 (E10: a small
  gain).
