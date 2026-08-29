# Notes + graph — one surface over one store

*2026-08-29. Designing the consolidation of the web UI's `notes` and `graph`
tabs into a single surface for capturing into, finding in, and editing the
knowledge graph. The evidence behind each choice is in
`GRAPH-UX-RESEARCH.md`; the current-state audit below was verified against
source the same day. All five owner decisions are ruled in §8.*

> **Status 2026-08-29: tier 1 built on `feat/graph-tab` (PR #120)** —
> `Graph.svelte` replaces both tabs, `/api/related` and `/api/timeline`
> land behind the owner guard, `#notes` redirects. Still unbuilt: the
> entity half of the drawer interleave and per-fact provenance chips
> (both need small mecha-graph-side reads, named in §2.1/§2.2), all of
> tier 2, and tier 3.

## 1. The problem, verified

The two tabs are not redundant — they are two disjoint halves of one
missing surface, split along a widget seam instead of an intent seam:

- A note **is** a graph record: `POST /api/notes` → `board::note` →
  `mecha kg note` → MCP `kg_upsert` writing an `episode` row with
  `source='note'` into the same SQLite file as every entity and fact. On
  capture the linkers already write `mention` rows for every entity the
  note names, and the nightly extractor mines it into `fact_candidate`
  rows. The stores were never separate.
- `Notes.svelte` has capture (typing + the `Dictate` mic) and a search box
  that is actually **whole-graph** search (`board::find` → `kg_search`,
  facts and episodes) — but its result cards are inert: a hit naming an
  entity cannot open that entity.
- `Entity.svelte` (the `graph` tab) is a single-entity lookup with
  per-fact confirm/refute — but has no search fallback on a missed name,
  no browse, no autocomplete, no mic, and nothing anywhere in `web/src`
  links to it.
- `kg_entity`'s envelope already returns `aliases`, `sources`, and
  `context`, all silently dropped by `Entity.svelte`. `kg_related`
  (neighborhood) and `kg_timeline` (bi-temporal history) have no web route
  at all. Nor do fact authoring (`kg_upsert{kind:"fact"}`, `assert`,
  `retract`, `link`), the proposals/merge/alias/retype family, predicates,
  or tags — the entire curation surface is CLI-only while `Home.svelte`
  shows its pending count and dead-ends.
- "Graph" already names three different things in the UI: the nav tab, the
  Review pane (`Queue.svelte`), and a Home stat card.

A structural simplification the design leans on: **`edges` is a SQL view
over `fact_current`** — there is no separate edge record, so "edit how a
node connects" and "edit a fact" are the same operation and need one UI,
not two.

## 2. The shape

One tab, three surfaces, one store underneath:

> Capture like Roam (one stream, zero decisions), structure like Tana
> (type as retroactive promotion), browse like Wikidata minus its chrome
> (entity pages, provenance chips, a small ego-graph), recall like Firefox
> (frecency) under one palette — and the model organizes like Mem, gated
> by the review queue Mem lacked.

### 2.1 Capture bar + recency drawer

A persistent capture input — textarea plus the existing `Dictate` mic,
zero decisions, no title, no type — landing in the episode stream exactly
as `board::note` does today. Beside it, the drawer: recent notes and
recently touched entities **interleaved**, frecency-ordered (the
`access_count` column on `nodes` already records the signal; episodes rank
by `occurred_at`). Tapping a note expands and edits it in place
(`board::note_edit` semantics are already right: timestamp preserved,
enrichment invalidated, honest `status`). Each note row shows the entities
it linked (`entities_linked` comes back from `kg_upsert` today and is
discarded) — capture becomes visibly graph-feeding, which is the trust
loop the research says makes the habit stick.

### 2.2 The entity page as center of gravity

`Entity.svelte` grows from a lookup card into the app's hub:

- Render what the envelope already carries: `aliases`, `sources` coverage,
  scope `context`.
- Every fact gets a **provenance chip** — `fact.episode_id` exists on
  every fact today and is never linked; one tap opens the source episode
  (note, mail, calendar) beside the fact it produced.
- A **local ego-graph** (1–2 hops, `kg_related`) in a corner — the one
  visualization the evidence supports. Never a global view.
- A **timeline section** (`kg_timeline`): superseded facts, `valid_to`,
  what changed when. Today only `valid_from` is shown.
- **Inline fact authoring**: add a fact as a sentence-shaped row
  (predicate autocomplete from the `predicate` table, object autocomplete
  over entities, literal fallback), landing via the same write path as
  every other mutation (§3). Retract likewise.

One build-time obligation on every new route: main's docs demo transport
(`302fb02`) embeds the real app with fixture data, and each endpoint a
component reaches must be registered in the `ROUTES` table in
`web/src/demo/index.js` — with a fixture if a page reads it, in the 501
group if it is a mutation the demo declines. Do not trust a green
`check-demo.mjs` for mutations: its `CALL` regex reads only literal paths
at the call site, so a fetch through a helper with the path in a variable
passes the guard and then renders "no fixture" in the live demo. Add
mutation routes to the 501 group by hand.
- Confirm/refute on unreviewed facts stays — the entity page is
  review-on-use, per `board::entity`'s own doc comment.
- A missed lookup falls back to `kg_search` instead of the current
  `found: false` dead end.

### 2.3 One palette

cmd-K from anywhere in the graph tab (D3): hybrid search (`kg_search` BM25 side plus
the :8081 embeddings, RRF-fused — hybrid, never embeddings-only, because
exact-name lookup dominates), frecency-ranked, with actions in the same
list: open entity, capture note, create entity, jump to queue. `[[` / `@`
autocomplete in every text field, create-on-miss. Search hits navigate —
an episode hit opens the note, a fact hit opens its subject's page.

## 3. Writes go through the CLI, structurally

Every mutating route shells out to `mecha` as a child process and relays
the CLI's own refusal verbatim (the `review::verb` pattern: the child's
last stderr line comes back as a 409). The web layer never becomes a
second editor implementation — validation, refusal wording, and the
review-on-use gates live in the CLI/MCP path that agents already use, so
the two surfaces cannot drift into accepting different inputs. New routes
needed: fact assert/retract, alias add, entity create; the backend verbs
all exist. A `the_graph_routes_sit_behind_the_owner_guard` test mirrors
the settings one (its own test, appended — not rows added to the existing
array).

## 4. The review boundary

Verdict work stays in Review. The entity page keeps its per-fact
confirm/refute (review-on-use is the point of it), but the deck/queue
belongs to `Queue.svelte` — and pkg's 2026-08-29 direction (`378ab8d`:
shadow queue answering in the generic `/queues` shape) means the bespoke
shadow rendering there should migrate to the generic surface rather than
be rebuilt inside the new tab. The consolidation removes the triplicated
verdict-card markup (`Entity.svelte::factVerdict`,
`Queue.svelte::shadowVerdict`, `Queue.svelte` accept/reject) down to one
shared component posting to the existing two routes.

## 5. Voice

- `Dictate` lands in every text field the new surface owns: capture,
  palette, entity lookup, fact authoring, refute reasons. This is
  component reuse, not new plumbing — audio stays on the box
  (`serve::dictate` → local Parakeet).
- Memo mode (ramble → cleaned note) is tier 2: the cleaned text is
  model-authored, so the raw transcript is stored beside it (an episode
  `meta` field), never replaced — provenance discipline and the taint
  model both require the original.
- Correction dictionary biased by the graph's own entity labels is tier 3.

## 6. Build order

- **Tier 1 (the spine):** capture bar + frecency drawer · entity page
  upgrades (dropped fields, provenance chips, ego-graph, timeline, search
  fallback, links *to* the page from every hit and mention) · palette with
  hybrid search · `[[`/`@` autocomplete.
- **Tier 2 (multipliers):** inline fact authoring + retract · type-as-
  promotion tags with faceted tables · memo voice mode · verdict-card
  unification.
- **Tier 3:** grounded chat panel citing clickable facts · entity-biased
  transcription dictionary · filtered local graph as a
  question-answering/diagnostic view.

Nearly all of tier 1 is UI over existing, tested verbs — `kg_related`,
`kg_timeline`, the dropped envelope fields, and provenance need no
graph-side work. The genuinely new code is small: frecency ranking, RRF
fusion, the palette component.

## 7. Deliberately out of scope

- **A global graph visualization.** Decoration past ~200 nodes; the
  ego-graph and the diagnostic view cover the two real uses.
- **Any required field, type, or title at capture time.** The Notion
  failure. Structure is always a promotion after the fact.
- **NL query as the only query path.** Palette, facets and entity pages
  are primary; chat is complementary and always cites.
- **User-authored ontologies.** The node-type set stays closed; free
  predicates remain allowed, as today.
- **Delete from the web.** `note_edit` refusing an empty body stands;
  tombstone/redact stay CLI-only. An erasure surface is a different
  design with different stakes.
- **Auto-accepting extraction.** The queue's human verdict is
  load-bearing (Mem's lesson, and this repo's "a lane must not promote
  itself").
- **Wikidata's statement-editing chrome.** Its data model, not its widgets.

## 8. Decisions

Ruled by the owner, 2026-08-29:

- **D1 — the surviving tab is `graph`.** One tab replaces both; the
  `notes` nav entry goes away. `#notes` should route to `#graph` so old
  habits and bookmarks keep working.
- **D2 — the drawer lives inside the graph tab**, not app-global. The
  omnipresent-capture reading loses to layout containment.
- **D3 — the palette is scoped to the graph tab**, not the whole web app.
  Widening it later is additive if it earns it.
- **D4 — promotion tags may mint predicates when needed.** The node-type
  set stays closed (§7 stands), but a tag's field template is not limited
  to existing predicate names — a new predicate rides the same write path
  as any fact and lands in the `predicate` table like one minted from the
  CLI. Prefer autocomplete toward existing predicates so minting is the
  exception, not the default.
- **D5 — memo-mode cleanup runs the local model only.** The cleaned-note
  pass never rides a cloud provider.

No decisions remain open; the design is buildable through tier 2.
