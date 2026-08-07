# The mecha-factory documentation section — a plan

The docs site already lives at `docs.mecha-factory.ai` and documents only the
harness. This is the missing half: what the factory is, what a request type can
say, and what it renders as.

Status: the **gallery generator ships** (`mecha-factory`,
`mecha-manifest/examples/gallery.rs`). Every page below that says "embeds the
gallery" has real bytes to embed, served from `/factory/gallery/…`. The prose
is what remains.

## Where things live, and why

| | where | why |
|---|---|---|
| The rendered gallery | generated in `mecha-factory`, committed at `gallery/` | the renderer lives there, so the drift check can fail the build that broke it |
| The prose | `mecha/website/docs/factory/` | the site is already on the factory's domain and already deployed |
| The design record | `mecha-factory/docs/*.md`, `mecha/docs/PUBLIC-SURFACE-DESIGN.md` | unchanged — the site gets the reader-facing version, not a second copy of the reasoning |

`website/scripts/sync-gallery.mjs` copies the committed gallery into
`static/factory/gallery/` at build time, from a sibling checkout when there is
one and from the public tarball otherwise. The copy is gitignored: two
repositories owning the same bytes means one of them holds the stale set.

## The pages

Ordered as the sidebar should read. `sidebar_position` in brackets.

### `overview.md` [1] — What the factory is

The two directions of one boundary, which is the frame everything else hangs
off: **publish what mecha makes** (durable, versioned, permissioned URLs) and
**build interfaces back into mecha** (one request type emits the HTML form, the
JSON Schema, the MCP tool and the agent-to-agent skill, so a human with a
browser, an agent with a browser, an agent with MCP and an agent doing
discovery all arrive at the same typed object).

Then the three crates as a table — `mecha-manifest` (pure contract, no I/O),
`mecha-factory-publish` (the home side, holds the key), `mecha-factory` (the
box, holds no credential that reaches home) — and the sentence that explains
the split: *the box holds nothing that can reach home.*

Must say: a form is the **default rendering, not the point**. Adding a modality
is a renderer, not a parallel system. Otherwise every reader files this under
"form builder" and stops.

### `request-types.md` [2] — Writing one

The anatomy of a manifest, in the order the file is written: `id`/`version`/
`title`/`description`/`retain_days`, then `[verification]`, then `[[fields]]`,
then `[[steps]]`, then `[[acknowledgments]]`, then `[confirmation]`.

Claims that have to land, each of which is a real rule in `check()`:

- **`version` is bumped deliberately.** Being made to think about it is the
  point, not friction.
- **`[verification]` names its field, never guesses.** A form can hold two
  email addresses — yours and your advisor's — and picking the first sends a
  stranger's verification link to somebody who never asked for it. That is not
  a wrong default, it is unsolicited mail sent in the user's name. The named
  field must be an `email` and must be `required`.
- **Absent `[verification]` means the type cannot be served as a form.** It is
  refused, not served unverified.
- **A `{placeholder}` in `[confirmation] body` must name a real field**,
  checked at load, so a typo is a startup error rather than `{advisor_nmae}` on
  a stranger's screen.
- **Fields are declared once and referenced by name from a step.** The system
  this borrows from keeps `fields`, `requiredFields`, `hiddenFields` and
  `conditionalFields` as four parallel lists that can disagree; a field owning
  both facts leaves nowhere for the contradiction to live.
- **A field on no step is a manifest error** once steps exist.

Embeds `source/stepped.toml` and links its rendered steps.

### `field-kinds.md` [3] — The reference page

The four-column view, one row per kind. This is the page that currently only
exists inside `request.rs`, and the one a second client needs.

| the TOML | the JSON Schema | renders as | what the server enforces |

Ten kinds: `text`, `long_text`, `email`, `url`, `date`, `integer`, `select`,
`multi_select`, `bool`, `file`. Per-kind notes worth writing out:

- `text`/`long_text` — **`max_length` is required.** These forms sit on an
  unauthenticated endpoint; an uncapped field is an unbounded write. `0` is
  refused too, as the degenerate spelling of the same mistake.
- `text` `pattern` — **client-side only.** The browser enforces it; the server
  enforces the cap and the type and never the regex. Say this plainly or
  somebody will rely on it as a control.
- `url` — data to show, never a thing to fetch. **Nothing resolves one**, and
  nothing downstream should: a form field is not a reason to make an outbound
  request to an address a stranger chose.
- `date` — bounds are literal dates, not offsets. An offset resolves against a
  clock, and the browser's clock is not the server's.
- `select` — the Action-Selector shape doing the real work: **nothing a
  stranger types can change what kind of thing their request is.**
- `file` — the value is never the bytes. It is `FileMeta`: what the box
  measured, from sniffed magic rather than a claimed mime or an extension. The
  stranger's filename rides inside the object until `take_attachments` lifts it
  out. Caps: 16 MB per field, 32 MB per type, because the box's disk is shared
  and a tenant-supplied manifest cannot set its own ceiling.

Then the derived property, in its own short section: **`is_free_text` is
derived from the kind, never declared.** A knob that let a manifest mark a text
field trusted is precisely the switch that must not exist — the same reasoning
that gives the learning system's provenance gate no override. This is the hinge
between this page and `/docs/features/frontdoor`, and it should link there.

### `gallery.mdx` [4] — See it

Live iframes of `/factory/gallery/<theme>/<page>.html`, with a theme picker.
Sections: every kind · rejected with errors · conditional fields · a form in
steps · the upload page.

Two implementation notes for whoever builds the MDX component:

- The iframe is same-origin, so the page can set
  `frame.contentDocument.documentElement.dataset.theme` to follow Docusaurus's
  own light/dark toggle. Without that, the form follows the OS's
  `prefers-color-scheme` and disagrees with the docs around it the moment a
  reader toggles.
- Say on the page that the forms **submit nowhere** — clicking Submit
  demonstrates the HTML5 constraint layer and nothing else. A reader who thinks
  it is broken has learned the wrong thing.

The conditional page renders almost empty at rest, which is correct: the
conditions have not been met yet. That needs one line of caption, or it reads
as a bug.

### `theming.md` [5] — Palettes

The thesis first: **a theme is tokens, never rules.** A schema-driven form
should look designed without anyone designing it and certainly without an
*agent* designing it — a model writing CSS per request produces a different
form every time, which is the opposite of a surface people learn to trust. So
layout is fixed and shared and a theme supplies colours, radii and type.

- The nine role-named tokens, as a table: `ground`, `surface`, `text`, `muted`,
  `line`, `accent`, `on_accent`, `ring`, `signal`. Roles rather than hues — a
  token called `purple` is one nobody can recolour. `ring` is distinct from
  `accent` because a focused button has both. `on_accent` is authored, not
  derived, because contrast is a judgement about a specific pair.
- **Both schemes, always.** A theme that shipped one forces every reader onto
  whichever the author happened to use.
- **Type is a stack, not a download.** An `@import` of a hosted stylesheet is
  an external reference, which the publish gate fails a bundle for and
  `style-src 'self'` blocks outright. Vendoring a `woff2` is the supported
  route, and a separate decision from picking a palette.
- Why there are two built-ins: **one built-in theme is a stylesheet with extra
  steps.** `paper` exists to prove the structural sheet hardcodes nothing.

Embeds the same page in both themes, side by side. That comparison *is* the
argument, and it is why the gallery iterates `BUILT_IN` rather than listing
palettes by hand — a third theme appears here for free.

### `bundles.md` [6] — Publishing what an agent made

`bundle_render` → `bundle_publish` → `bundle_alias`, and the two directories
that mean opposite things (`~/.mecha/work/<producer>/` mutable and cleanable
vs. `~/.mecha/bundles/<id>/<ver>/` immutable and never deleted). Content
classes, visibility, the content digest the server recomputes over what arrived
on the wire.

The vendoring gate gets its own section: **a published bundle must be
self-contained**, and the gate is a real check rather than a grep, because a
grep cannot tell a link a reader clicks from a resource the page fetches. Note
that the gallery itself passes this gate in CI — the docs are evidence for the
rule they describe.

### `publishing.md` [7] — Staging a publish

The existing `features/publishing.md`, linked from here rather than moved:
it is about the outbox, which is a harness feature. This page is the factory
half — what a publish *is* — and it should hand off at the sentence where the
outbox takes over.

### `self-serve.md` [8] — Getting an account

From `mecha-factory/docs/SELF-SERVE.md`: invites, handle claims, pairing,
per-user certificates, the tenant page. Reader-facing only; the arc stays in
the design doc.

### `deploying.md` [9] — Running a box

From `mecha-factory/docs/DEPLOY.md`: three origins, three policies, SQLite,
ACME. Plus the one sentence that governs the whole design — **the box holds no
credential that reaches home** — restated here because this is the page
somebody reads while pointing DNS at a VPS.

### `second-client.md` [10] — Not mecha

From `mecha-factory/docs/SECOND-CLIENT.md`. Depends on `field-kinds.md` being
complete, since the four-column table is the contract this page assumes. Last
on purpose.

## What this section should not do

- **No screenshots of the components.** A live iframe cannot drift and a
  screenshot always does. Screenshots earn their place only for what cannot be
  embedded: `mecha outbox show` on a publish, the account page, the operator
  surface, a request's lifecycle in a terminal.
- **No copy of the design reasoning.** `PUBLIC-SURFACE-DESIGN.md` and the
  factory's own `docs/` stay the record. A page here that restates an argument
  will be the copy that goes stale.
- **No serving the gallery from the box.** Tempting, since the box already
  serves themed pages under CSP, but it would make the docs need a running
  server and a tenant. Somebody deciding whether to install anything has to be
  able to read them cold.

## Open questions

1. **Does `features/publishing.md` move or stay?** Staying costs a
   cross-reference; moving costs a redirect and breaks any link already out
   there. Leaning stay.
2. **Does the sidebar get a top-level "Factory" category, or does the factory
   get its own docs *instance*?** A category is one `_category_.json`. A second
   instance gives it its own version tree, which matters only once the manifest
   format versions independently of mecha — which it already does (`version` on
   every request type). Not yet, but the day `manifest v2` lands, revisit.
3. **Should the gallery carry a third theme purely as documentation** — one
   deliberately garish — to make "tokens, never rules" undeniable? Cheap
   (`BUILT_IN` is an array) but it ships a palette somebody will use.
