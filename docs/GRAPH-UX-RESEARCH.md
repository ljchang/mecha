# Graph UX — how a personal knowledge graph should meet its owner

*2026-08-29. The question: what user interfaces work for querying, editing
and feeding a personal knowledge graph — and which features are worth
exposing. Researched by survey: the tools-for-thought generation (Roam,
Obsidian, Tana, Capacities, Notion, Mem, Reflect, Heptabase), the
visualization and HCI literature, Wikidata's editing experience, and the
2024–26 LLM-over-graph work. Claim strengths are marked: **[peer/preprint]**,
**[vendor]**, **[practice]** (accumulated community experience),
**[folklore]** (one practitioner's account). The decisions this produced are
in `NOTES-GRAPH-DESIGN.md`; this file is the evidence.*

## 1. Graph visualization is mostly decoration

The most consistent finding in the survey. Cognitive-load studies put the
usable ceiling of node-link layouts at a link density (edges/nodes) of ~3,
with breakdown already visible in test graphs of 25–175 nodes
**[preprint]** (<https://arxiv.org/pdf/2008.07944>); past that, every layout
produces a hairball needing aggregation or filtering to be usable at all
**[peer]** (Microsoft, "Trimming the Hairball"). A decade of Obsidian
practice converges on the same number independently: the global graph is
informative under ~50 notes and "a tangled web that's more fun to look at
than navigate" past ~200 **[practice]**
(<https://forum.obsidian.md/t/whats-the-point-of-the-graph-view-how-are-you-using-it/71316>).

Two uses survive:

- **The local ego-graph** — 1–2 hops around the current entity — stays
  useful at any corpus size **[practice]**, and node-link is genuinely the
  superior form for connectivity questions ("how are X and Y related")
  **[peer]** (<https://arxiv.org/pdf/2304.01311>).
- **Anomaly spotting** — orphans, duplicates, and sync errors show up
  visually before they show up in search **[folklore]** (Eleanor Konik).

Rule extracted: a graph rendering is a *scoped answer to a question*, never
a homepage.

## 2. The entity page is the workhorse; Wikidata is both model and warning

Wikidata's item model — label + aliases + description, then statements as
property→value rows each carrying qualifiers and references — is the
information architecture that scales linearly where graph views don't, and
it is where editing and provenance naturally live. Its *chrome* is the
cautionary tale: the statement editor feels like database administration,
and its own usability pages document editors getting lost **[vendor]**
(Wikidata UI redesign input; mobile editing discussions). Adopt the
architecture, not the widgets: provenance as a chip on each fact, editable
detail behind a click, editing that reads like completing a sentence.

Faceted browsing is the established middle layer between search and the
entity page — pick a type, narrow by predicate values, counts update
reactively **[peer]**
(<https://www.sciencedirect.com/science/article/abs/pii/S1570826815001432>).
It is what makes typed entities pay rent.

## 3. What the tools-for-thought generation measured with real users

- **Roam**: the daily note as default capture surface (zero decisions about
  where a thought goes) and pervasive backlinks were the durable
  inventions. The failure: a densely linked graph did not by itself produce
  insight — "a garbage dump full of crufty links users hardly revisit"
  **[practice]** (Every, "The Fall of Roam"). Backlinks accumulate; they do
  not organize. Without resurfacing, a link graph is write-only.
- **Tana**: supertags — a tag that turns any bullet into a typed node with
  a small field template, all instances forming a queryable table — are the
  cleanest bridge anyone has built between free notes and a typed graph
  **[vendor/practice]**. The key property: **structure is a retroactive
  promotion, not a prerequisite**. The documented trap is the ontology
  learning cliff for new users **[practice]**.
- **Capacities**: the same idea with training wheels — a handful of
  built-in object types beats both "everything is text" and
  "define your own ontology" **[vendor/practice]**.
- **Notion**: the anti-pattern for capture. Choosing a database and filling
  properties at capture time kills capture; an ecosystem of third-party
  quick-capture apps exists purely to route around Notion's own path
  **[practice]**.
- **Mem.ai**: full auto-organization with no owner verdict is at best a
  conditional success — users split on the loss of control **[practice]**.
  "AI proposes, owner disposes" is the trust-preserving middle.
- **Reflect**: proof that small surface area wins — four sidebar items,
  backlinks with unlinked references, hotkey voice capture, and reviewers
  call it the app that becomes part of daily work *because* navigation is
  trivial **[practice]**.
- **Heptabase**: the spatial canvas is a synthesis workspace, not a
  retrieval surface — orthogonal to this project, skippable.

## 4. Capture

Convergent findings across quick-capture practice **[practice]**:
friction is *decisions*, not keystrokes — the input must open ready to type
with no folder, type, or title choice; everything lands in **one
time-ordered inbox** and classification happens later (the modern twist:
AI does the first filing pass and the human reviews it); sub-five-seconds
requires an omnipresent affordance (global hotkey, persistent bar,
palette) that does not navigate away; and capture must never fail silently
or the habit dies.

## 5. Search and recall

- **Command palette** (cmd-K) as one entry point for find + navigate +
  create + act is now the standard pattern (ARIA combobox; Linear, Notion,
  Raycast) **[practice]**. Putting "capture" and "create entity" in the
  palette makes it the capture surface too.
- **Frecency** — Mozilla's frequency+recency with exponential decay — is
  the proven default ranking for suggestion lists and recency drawers
  **[vendor, battle-tested]**
  (<https://firefox-source-docs.mozilla.org/browser/urlbar/ranking.html>).
  Pure recency evicts still-important items; pure frequency keeps stale ones.
- **Hybrid lexical + semantic retrieval fused with RRF**: representative
  vendor numbers are BM25-only 65% recall@10, dense-only 78%, hybrid 91%
  **[vendor]** (Supermemory, Redis, OpenSearch). Exact-name lookup is the
  dominant personal-notes query, and embeddings alone lose it — hybrid,
  not replacement.
- **Timeline is a first-class recall axis** — "when did I learn this" —
  demonstrated by every daily-note-centric app **[practice]**.

## 6. LLMs over the graph

- **NL-to-graph-query is brittle as a sole interface**: ~85% execution
  accuracy only with per-schema fine-tuning, sharp cross-schema decay
  **[preprint]** (Text2GQL-Bench). What works better is an agent walking
  typed graph tools iteratively — search, entity, neighbors, timeline —
  which is exactly the `kg_*` MCP shape mecha already has **[preprint]**
  (<https://arxiv.org/pdf/2604.02861>).
- **Answers must cite the facts used, clickably** — GraphRAG's failure mode
  is fluent confidence over drifted retrieval **[preprint]**
  (<https://arxiv.org/pdf/2603.14828>).
- **Extraction is mediocre and precision beats recall**: best small-model
  triple extraction from conversation reached F1 ≈ 0.66, semantic errors
  are syntactically well-formed so schema validation cannot catch them, and
  downstream utility tolerates missing triples far better than wrong ones —
  a low-recall extractor retained >75% of downstream performance
  **[preprint]** (<https://arxiv.org/html/2607.00003>). Review effort goes
  to killing false positives; missed facts get hand-added — which is the
  argument for a fact-authoring UI, not a higher-recall extractor.
- **Review-queue UX**: seconds-per-verdict, batch by similarity, and show
  the source snippet beside each candidate — consistent with this repo's
  "the reviewable object is the thing itself" **[practice]**.

## 7. Voice

Two modes, neither substituting for the other **[practice/vendor]**:
**dictation-to-cursor** (Wispr Flow: hotkey, speak, text lands in whatever
field has focus — makes every input voice-capable for free) and
**memo→cleaned note** (AudioPen: ramble with backtracking, get clean prose).
For a graph app the memo mode must keep the **raw transcript beside the
cleaned version** — the cleanup is a model paraphrase, so provenance and
this project's taint discipline both require the original. The winning
correction UX is type-over feeding a personal dictionary; for a graph the
dictionary is better still — bias correction with the graph's own entity
labels, so names the store knows transcribe correctly. Conversation is for
retrieval; dictation is for capture — conversational capture is too slow
and too lossy **[practice]**.

## 8. What this means for mecha specifically

The strongest single pattern in the survey: **every successful blend of
notes and graphs makes structure something added to prose after the fact;
every failed one makes structure something supplied before the prose
exists.** Mecha's pipeline already has the right bones — capture lands as
an episode, linkers attach mentions, the extractor proposes, the review
queue disposes — and the survey says the missing pieces are UI, not
architecture: an entity page that is actually the center of gravity, one
search surface with hybrid retrieval and frecency, capture that is
omnipresent rather than a tab, and provenance rendered on every fact.

Recommending **against**, with the evidence above: a global graph view as
navigation; any required field or type at capture time; NL query as the
only query path; auto-organization without owner verdicts; backlinks
without a resurfacing mechanism; Wikidata's editing chrome; user-authored
ontologies (ship a small fixed starter set instead).

The design that consumed this research: `NOTES-GRAPH-DESIGN.md`.
