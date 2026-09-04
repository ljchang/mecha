# The spoken override — design

Designed 2026-09-04, not built. The question it answers: **when a staged draft
carries a parameter the harness chose rather than the drafter — which account,
reply-all or not — how does the owner change it by ear?**

Its origin is the owner's, on 2026-09-04: *"can't we have this turn into a
verbal ask that requires the user to decide in the moment?"* — asked about
`account`, after #144 made the pinned value merely *visible* on every review
surface. Visibility was the smaller half. This is the other one.

The prerequisite landed first: `docs/VOICE-RESEARCH.md` §the fourth door, and
#158, which closed the spoken confirmation against the room's own echo. This
file assumes that mechanism and reuses it; read that section before this one,
because every hazard here is a consequence of something recorded there.

## 1. The two moments, and why this file is about the later one

An ambiguous parameter can be settled at **draft time** or at **review time**,
and they are different features.

*Draft time* already exists and is not this. `mail_send` fails when several
accounts are configured and none is default — `unified.rs`'s description says
*"the call fails and you should ask the user which account to send from"* — so
the model asks, in whatever surface the run is on, and drafts afterwards. It
costs a model turn, and the answer can change what gets *written* (a work
letter and a personal one are not the same letter). That path is right when the
choice shapes the prose.

*Review time* is this file. The draft exists, the owner is hearing it, and the
question is whether the value the harness supplied is the one they want. It
costs no model turn, it happens where the owner already is, and it composes
with the outbox's existing `update_args`. It is the right moment for a value
that changes the *envelope* rather than the letter.

**Both should exist.** Nothing here removes the draft-time path.

## 2. Why the confirmation mechanism does not transplant

The spoken confirmation rests on a **closed, compile-time** answer vocabulary:
`SEND_PHRASES`, `LATER_PHRASES`, `READ_PHRASES` in `review_policy.rs`, matched
against the whole normalised utterance. A parameter ask has no such thing —
account names are configuration, reaching the facade through a tool schema.
Three consequences, and the second is the one to design against.

**2.1 The one-word band inverts from protection to hole.** `MIN_SPAN_WORDS = 2`
exists so that a real `"yes"` survives the echo gate: one word is where an echo
and the plainest possible answer are the same string, so the gate declines to
judge there. `"personal"` is also one word. So an echo of the question's own
tail — *"…dartmouth or personal?"* — is immune **by the same rule that makes
the confirmation usable**, and would be taken as a choice. With yes/no this is
a stated residual (`VOICE-RESEARCH`, the timing layer's justification); with a
parameter ask it is the ordinary case rather than the edge.

**2.2 The answer vocabulary becomes server-declared.** Account names arrive
from `mecha-mail`'s schema `enum`. #144 established the rule this violates: a
schema is *the judged party's own declaration* and may never buy anything —
which is why `agent.rs`'s policy `max` narrows only, and why `publish_tools`
is config's to declare rather than the tool's (`config.rs`: *"a third-party
MCP server cannot be trusted to say"*). An unfiltered override vocabulary lets
a server declare an account named `"yes"` and put it in the spoken answer set.
This is not hypothetical in shape: **every outbox-routed tool today is already
an MCP tool** (`mail__*`, `docs__*`); the only thing making them safe is that
we wrote both servers, and the mechanism cannot tell.

**2.3 Rewording does not help.** #158 moved the accept menu off the offer's
tail, because the tail is what echoes. A parameter question **must** name its
options — that is what makes it answerable — so the same move is unavailable.

## 3. What makes it tractable: the consequence asymmetry

A wrong parameter **sends nothing**. The draft stays staged, still requires a
confirmation, and `identity_tail` (#158) already speaks the account as the
*last words of every offer* — the one fact a listener cannot re-read.

So the design does not need to distinguish a spoken override from its own echo.
It needs the *result* to be audible before anything acts on it.

> **Do not ask a question. State the default, and accept an override.**

The offer already ends *"That one is from your dartmouth account."* An override
switches the value, rewrites the staged args, and **re-offers** — and the
re-offer's tail names the new account. A spurious switch therefore costs one
repetition and is **heard**. A silent wrong send is the failure this avoids,
and it is avoided by readback rather than by parsing.

## 4. The design

### 4.1 Shape

1. The offer states the value in its tail, as today.
2. An utterance matching one of the *alternatives* for an overridable field is
   an override: `OutboxStore::update_args` rewrites that key, and the draft is
   re-offered from the top.
3. The re-offer's tail states the new value. `unedited_defaults` (#154) drops
   the key from the "Defaults:" clause once it is rewritten, so the value stops
   being narrated as a harness choice the moment it becomes the owner's — that
   behaviour already exists and this depends on it.
4. Everything else is unchanged: the confirmation still gates the send, the
   echo window still slides, `MAX_REASKS` still bounds the re-ask.

**The cycle needs its own bound, and `MAX_REASKS` is not it** — that counts
`NotConvinced` re-asks, and a re-offer is a fresh offer. The loop is not
hypothetical: the re-offer's *tail speaks the new value*, which is exactly the
word most likely to come back off the speaker, and an override token is what
it would come back as. Two rules, and the first is the one that matters:

- **The value already set is never an alternative.** Overriding `account` to
  `personal` when it is already `personal` is not a change, and an echo of the
  tail is precisely that no-op. Removing it from the offered set makes the
  dominant loop unrepresentable rather than bounded — the same move as taking
  the accept menu off the tail in #158, one layer up. Reaching A→B→A then
  needs the room to echo a name the tail never said.
- **And a small per-draft cap regardless**, terminating the way everything
  else on this surface does: state the value, stop offering alternatives, and
  leave it for the page. A bound whose argument is "the other rule makes this
  unreachable" is the kind that stops being true quietly.

### 4.2 Where the alternatives come from

**The live tool schema, read at offer time — not a new field on `OutboxItem`.**
The facade holds `Arc<Agent>`, so `shared.agent.registry().get(&item.tool)`
gives the `Arc<dyn Tool>` and `input_schema()` gives `properties.<key>.enum`.

(Not `agent.context().tools`, which is the first thing to reach for and is
wrong: `ToolCtx` is the path jail and the output budget, not the registry.
`Agent::registry()` is the accessor, and the facade's own module docs already
note that the registry belongs to the one shared agent.)

Two reasons for the live read over a stored copy. A field on `OutboxItem` is an
append-only wire format — it would need `#[serde(default)]`, and it would go
stale against a reconfigured account list while looking authoritative. And the
registry is where the schema already is; copying it is the duplication that
`docs/README.md` warns about, one layer down.

**If the tool is not in the registry, there is no override.** A staged draft
outlives the run that made it, and an MCP server can be removed from config
between staging and review. Fail closed and say nothing about alternatives:
the offer still states the value, and the page still works.

The loop's invariant is not touched: this reads a schema through the `Tool`
trait, in the facade, and learns nothing about where the tool came from.

**The match is whole-utterance, never substring** — the same rule
`parse_answer` follows and for the same reason. `"personal"` overrides;
*"actually that one should go from personal"* does not, and falls through to
the model as the ordinary correction it is. A substring match would make every
mention of an account name an override, which is the bag-of-words looseness
six rounds of the worker's text filter already rejected.

### 4.3 The filter, which is the load-bearing part

**A candidate override token is rejected if it parses as an answer, *or if it
normalises to nothing*.** Not "if it looks odd" — run it through
`review_policy::parse_answer` and drop any option that returns anything but
`NotAnAnswer`. An account named `"yes"`, `"ok"`, `"later"`, `"read"` or `"no"`
must never enter the spoken vocabulary.

The second half is not belt-and-braces, and `parse_answer` alone does not give
it: it **returns `NotAnAnswer` for the empty string**, so an option that
normalises away passes the filter and then matches every utterance that also
normalises away. That is reachable — `normalise` keeps only alphabetic
characters and whitespace, so an all-digit account name empties, as does a
bare filler like `"so"` or `"thanks"`. §3's readback bounds the damage, but
this is §2.2's hazard exactly, in the clause this section calls load-bearing.
Require a **non-empty** normalised form as well. Found on review.

Fail closed, and **over the draft's whole spoken vocabulary rather than each
field against the answer lists**. `mail_reply` has two overridable fields, so
collisions are pairwise as well as against `parse_answer`: a server-declared
account named `reply all` passes `parse_answer` cleanly and collides with
§4.4's boolean pair, and neither field checked alone would see it. If any two
tokens in a draft's spoken vocabulary collide — or any one of them parses as
an answer — that **draft** gets no spoken override and the owner uses the page.

Per draft rather than per option, for the same reason: dropping only the
colliding token leaves a set whose safety depends on which names a server
happened to choose. Found on review, which had this as a per-field rule.

**Checked once, over the full `enum` including the current value.** §4.1's
"the value already set is never an alternative" makes the *offered* set change
after every override — A→B turns `{B, C}` into `{A, C}` — so a collision
between `A` and `C` is invisible to a check that ran against `{B, C}`. The
decision has to be stable across re-offers, which means computing it over
everything the field could ever offer, not over what it offers right now.
Same reason as the paragraph above: safety must not depend on which state the
draft happens to be in either.

This wants the test shape `no_single_word_of_the_reask_is_an_answer` already
uses — checked against the real phrase lists, not against a comment, because a
phrase list can grow and the collision is silent when it does.

### 4.4 Which parameters qualify

Three conditions, all required:

- the harness supplied the value (`filled_defaults` names it),
- the choice is genuinely the owner's, not a constant,
- there is a **closed set** to speak — from the schema's `enum`, or, for a
  boolean, from a pair this repo authors.

| field | qualifies | why |
|---|---|---|
| `account` | yes | the archetype; the model cannot know work from personal. `enum` in the schema |
| `reply_all` | yes, but not from the schema | `{"type": "boolean", "default": false}` — **no `enum` at all**, so §4.2's live read yields nothing and the pair has to be ours |
| `calendar_id` | no | `"primary"` is a harness constant |
| `all_day` | no | same |

**A boolean's pair cannot be yes/no**, which is the first thing to reach for
and is unusable: both are in `SEND_PHRASES`/`LATER_PHRASES`, so §4.3's filter
disables the field. It has to be named — *"reply all"* against *"just the
sender"* — and neither of those collides. That is a small mercy but it is also
the better design: a harness-authored pair is compile-time closed, exactly like
`SEND_PHRASES`, so §2.2's server-declared hazard does not arise for booleans at
all. Found on review of this doc, which had `reply_all` qualifying under a
condition it does not meet.

A field with no `enum` and no boolean type is not overridable by ear. That is a
deliberate ceiling: an open-ended spoken value is a transcription bet on a
string nobody can re-read, which is the opposite of what the outbox is for.

### 4.5 What is deliberately not in scope

**`questions.rs` is not this.** `Question` carries `options` and looks like a
fit, but it is the *model's* question — session-bound, `ask_user`-authored,
answered on the page, and part of the draft-time path in §1. The review-time
override reuses `Pending`/`react` and `update_args` and persists nothing new.
Two mechanisms, two moments; a pointer between them, not a merge.

**"Reuses `react`" is true of the machinery and not of the signature.** `react`
is pure over `(utterance, pending, head, next)`; a schema-derived vocabulary is
computed at *offer* time (§4.2) and needed at *answer* time (§4.1.2), so it has
to reach `react` as an argument or ride on `Pending`. `Pending` is in-memory
and already carries the echo window, so it is the natural home — but that is a
real signature change, and this doc should not imply otherwise while being
precise about access paths everywhere else.

**No open-ended spoken values**, per §4.4.

**No new config knob.** The overridable set is derived from the schema and the
`filled_defaults` list, both of which already exist.

## 5. The residual, stated in the direction that matters

A single-word override is below `MIN_SPAN_WORDS`, invisible to the echo gate,
exactly as a bare `"yes"` is — §2.1. This design does not close that; it makes
it survivable, because nothing sends without a further confirmation and the
readback names the new value.

**The timing layer is still the real fix**, and it is still blocked on the same
measurement: `VOICE-RESEARCH` records why the playback constant cannot be
derived from the existing journal (TTS generation runs ahead of playback, so
`Generating TTS` intervals measure buffering — ~33 chars/s, ≈400 wpm, plainly
wrong) and that it wants a real call made after the 2026-09-03 mic-meter
repair. That call is owed regardless; it is the same call that re-derives
`ECHO_SEGMENT_RMS`.

## 6. Sequencing

This does not have to wait for the timing layer, because nothing here can send.
But it adds one-word answers to a surface whose one-word band is unguarded, so
**the measurement call should come first** — otherwise the timing work starts
behind a second feature that depends on it.

Suggested order:

1. The voice call, staging a draft on purpose. It is the first end-to-end
   exercise of the confirmation path — thirty days of journal contain no
   `"Say yes to send it"`, so none of #158 has run in anger.
2. The timing layer, from that call's numbers.
3. This.

## 7. Open, for the owner

- **Does `reply_all` belong in the first cut**, or is `account` alone the
  right scope? `account` is the one that prompted this; `reply_all` is the one
  whose wrong value is most embarrassing.
- **An override must not be mined as a writing correction — and this is a
  decision the doc should carry rather than leave to whoever builds it.**
  `mineable_as_writing()` is `writing_outcome() == SentEdited`, and an
  override makes `edited()` true, so an account switch would enter
  `mecha reflect`'s corpus as *"the owner rewrote what I wrote"*. It is not:
  the prose is untouched and only the envelope moved. That matters more than
  the appraisal consequence below, because a learned rule rides every future
  prompt's cached prefix — the longest-half-life path in the system.

  The principled line is available and generalises: `DraftView` already knows
  `body_field`, so **a diff that touches no prose key is not a writing
  correction**. That also fixes the pre-existing case — editing only a subject
  line in `mecha outbox edit` is mined today — which is the argument for
  fixing it in the miner rather than special-casing the override.

- **Should an override without a following confirmation expire?** A switched
  account on a draft nobody then sends is harmless, but the staged item now
  reads as edited by the owner (`edited()` is true, and `writing_outcome()`
  will count it `SentEdited` if it later goes out unchanged from there). That
  is arguably correct — a person did choose it — but it is a change to what
  the appraisal corpus means, and it should be a ruling rather than a side
  effect.
