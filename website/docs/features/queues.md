---
title: The queues
sidebar_position: 9.5
description: Every store waiting on you, in one list — and the knowledge graph's merge queue reviewable from mecha, by mechanism, by class, and by random sample.
---

# The queues

Five stores accumulate work for you, and each has its own verb: the
[outbox](./outbox) holds drafts, the [front door](./frontdoor) holds
strangers' requests, [learning](./learning) stages rule changes,
[run quality](./run-quality) stages harness changes, and — in another
repository entirely — the [knowledge graph](../graph/overview) holds a merge
queue of proposed facts. Knowing what was waiting meant remembering five
commands, which is how a queue can grow to thousands of items without anybody
deciding to let it.

`mecha review` is the aggregator, and `/queues` in the TUI is its modal:

```
$ mecha review
4,520 item(s) waiting on you

  4,461  graph candidates       3 proposer(s); 12 from mechanisms you have never judged
     10  graph shadow           2,456 unreviewed facts live, 0 ever served
     32  graph entities         2 detector(s) with something to say
     10  outbox drafts          10 drafted with the trifecta armed
      1  front-door requests    5 closed
      0  rule proposals         0 decided
      0  harness changes        0 resolved
```

The `graph shadow` row is the graph's *surfaced-verdict* queue
(review-on-use): extracted facts no longer wait in the candidate queue —
they go live as **shadow facts**, retrievable but rank-discounted and
labeled `unreviewed`, and earn a human verdict only when they are about to
matter: served in a context pack, contradicting a reviewed fact, or
spot-checked by a sampled class. Its depth is how many are surfaced right
now (at most ten — review arrives a handful at a time), and
`mecha review shadow` lists and decides them: `--confirm <uid>` stands
behind one, `--refute <uid> --reason '…'` retracts it as never true.
The verdict verbs are deliberately absent from the graph's MCP tool
surface — the model can *show* the queue (`kg_shadow_queue`, read-only),
but only a hand on an owner surface can settle it.

It holds nothing of its own — like [the doctor](../reference/cli#doctor), it
reads what the other stores own and adds no sixth store that could disagree
with them. Four of the rows hand off to the surface that already owns them,
because `/outbox` and `/frontdoor` carry the confirmations and taint warnings
that make their approvals safe, and a second copy of those would be a second
thing to keep correct. The graph queue is the exception: it is reviewed in
place, because before this nothing in mecha could reach it at all.

:::note Why it is not called `/review`
`/review now|later|auto` already exists — it is the outbox's release policy.
Two things called *review*, one word apart, is a trap, so the modal is named
for the stores rather than for the act.
:::

## An unreadable store is a dash, never a zero

"Nothing waiting" and "could not look" are opposite findings. If the graph
binary is missing or too old, its row reports `—` with the reason beside it,
the other four rows are unaffected, and a footer says the total is a floor.
A reader that rendered its own failure as an empty queue would reproduce
exactly the bug this surface exists to catch.

## The graph queue, three levels deep

```
queues ──Enter──▸ proposers ──Enter──▸ classes ──Enter──▸ items
                    t │                  t │               a / r / n
              evidence filter      evidence filter       one at a time
                      │                    │
                      s                    s
                      ▼                    ▼
            similar across          similar within
            every class             this class
                      └──── a / r verdicts a whole group ────┘
```

**Proposers** is the level decisions are actually made at. A proposing
mechanism — the LLM extractor, a linker, a wearable's suggestions, a rule —
spreads across many predicates, so its own hit rate is invisible in a list of
hundreds of `(proposer, predicate)` classes. The rollup shows each mechanism's
pending count, its **human** accept rate, and how much evidence that rate
rests on:

```
  4,841 in 726  llm              59% of 1984  solid     1167 auto-dropped
  1,084 in 1    bee:suggested      —  none    unjudged    16 auto-dropped
     56 in 1    linker:knn       16% of 57    solid       54 auto-dropped
```

Two rules keep those numbers honest, both learned the expensive way:

- **Machine rejections are never counted as yours.** The graph's own precheck
  rejects duplicates and ephemerals by the hundreds; folding those into the
  accept rate made good classes look terrible — one measured 49 points worse
  than the owner's actual record — in exactly the view a person reads before
  verdicting a whole class. They are shown beside the rate as *auto-dropped*,
  never inside it: a mechanism that mostly repeats itself is a different
  problem from one that is mostly wrong.
- **An unjudged mechanism has no rate, not a rate of zero.** A dash and the
  word `unjudged`, because "never reviewed" and "always rejected" are opposite
  findings, and rendering them alike makes an untouched mechanism read as a
  rejected one.

**Classes** are the queue grouped by `(proposer, predicate)` — one decision
per class rather than per fact. `a`/`r` verdict the whole class (driving the
graph's own bulk accept, with its cap and its dry-run), and `t` cycles the
evidence filter — `all → unjudged → thin → some → solid` — so the classes
that need evidence are one keystroke from the top rather than scattered
through a list ordered by size.

## Item review is a random sample, on purpose

`Enter` on a class does not show you the head of its queue. It draws a dozen
candidates **uniformly at random** (`mecha review sample`), seeded, with the
seed in the title:

```
 bee:suggested · related_to — random sample of 12 · seed 1787433025547322892
```

The queue has an order, and every order it could have is correlated with
something — age, id, confidence. Judging the first dozen and reading the
result as the class's accept rate measures the ordering, not the class. A
random draw is the only selection that turns a sitting's verdicts into
evidence about the class, and the printed seed is what makes the sample
checkable: anyone can redraw it.

Two details protect that property:

- **A verdict does not resample.** `a`/`r` decide one item and drop it from
  the list locally; the other eleven stay exactly the eleven that were drawn,
  so a sitting's verdicts describe *one* sample. `n` asks for a fresh draw,
  explicitly.
- **`mecha review items` is the queue-order alternative**, for a class you
  have already decided to clear — and it says outright that verdicts
  collected that way are not a rate.
- **`Enter` opens the whole item** — full statement, the payload the graph
  holds, confidence, and when it was proposed — and `j`/`k` flip through the
  sample without leaving the view. The list truncates to one line; a verdict
  on text you could not read is the approving-unread failure the
  [outbox](./outbox) exists to prevent, one store over.

## Repetition is reviewable as repetition

The queue's bulk is the same fact said many ways. An extractor proposes
*"Luke plays with his children"* a hundred slightly different ways, and the
graph's own dedup only removes the near-identical — everything between
*similar* and *duplicate* queues for review one item at a time. That is how
a queue reaches seven thousand items.

`s` groups a class by semantic similarity, largest group first:

```
  x25   #12636   Dana Whitfield is family of Mara.
           ~ Dana Whitfield is the partner of Mara.
           ~ Mara is Dana Whitfield's wife.
```

`a` or `r` on that row is **one verdict covering twenty-five candidates** —
and, crucially, **one human verdict**. The item you see is accepted or
rejected as yours; the rest ride through as a labelled machine cascade that
the [autonomy ladder](/docs/graph/architecture) never counts. A keystroke
that manufactured twenty-five human verdicts would promote a class on its
own volume, which is the one thing the ladder exists to prevent.

Three rules make the fan-out safe to press:

- **The group's face is a real member's own words**, never a model-written
  summary. A verdict lands on these rows, and approving a paraphrase is
  approving unread.
- **The cascade acts on the ids that were on screen.** It does not re-derive
  similarity at verdict time, so what you read is what you decided about —
  and no embedding runs, which is why the keystroke answers at store speed.
- **`[` and `]` step the threshold**, from the value the last grouping
  actually ran at rather than from a constant the interface holds.

### Across every class, when the backlog is the problem

Within a class, sharing a proposer and a predicate already vouches for
kinship. **`s` at the proposer level does the same thing over the whole
pending queue**, regardless of class — the fast way through a backlog that
one class at a time will never clear:

```bash
mecha review groups --all
```

Because the class no longer vouches for anything out there, the floor is
stricter (0.90 rather than 0.83), and **every group names the classes it
spans**, because the blast radius is part of what you are approving:

```
  x25   #12636   Dana Whitfield is family of Mara.
           spans: llm . family_of ×18, llm . is ×4, llm . knows_of ×2, …
```

A cascade still never crosses a class *uninvited* — you ask for it by flag,
the listing shows you what it reaches, and the verdict rides
`--across-classes` so the graph vets every id against a rule that knows the
crossing was intended. Singletons stay in their class listings; the view
reports how much of the queue it covered rather than quietly showing less.

## The one place mecha shells out to the graph

Everything else mecha does with the graph goes through the MCP tool surface
([tasks](../reference/cli#tasks), [distillation](./distillation), memory
reads). Review deliberately does not, and the reason is a boundary rather
than a convenience: the tool surface has `kg_pending` (read) and `kg_verdict`
(an opinion that decides nothing) and **no `kg_accept`** — because every MCP
tool lands in the model's registry, and a model that can accept fact
candidates can accept the ones its own extractor proposed.

So the decision runs the way a person runs it: the `mecha-graph` binary as a
child process, found on `PATH` or via `$MECHA_GRAPH_BIN` — resolved from the
environment and never from `mecha.toml`, since a project file arrives with a
cloned repository, and a project that could name a binary mecha executes has
been handed arbitrary execution. The dependency is runtime and optional:
every verb degrades to a named error, and the summary still covers the four
mecha-owned stores without it.

## The commands

```
mecha review                  # the summary (also: mecha review queues)
mecha review proposers        # the queue by proposing mechanism
mecha review shadow           # the surfaced-verdict queue  [--confirm U | --refute U --reason S]
mecha review list             # pending classes  [--proposer X]
mecha review sample           # a random draw    [--proposer X --predicate Y -n 12 --seed S]
mecha review items            # queue order — not a rate
mecha review accept <ids…>    # or --proposer X --predicate Y [--limit N] [--dry-run]
mecha review reject <ids…>    # same, plus --reason
```

Every one of them takes `--json`, and the modal drives exactly these — there
is nothing `/queues` can do that a script cannot.
