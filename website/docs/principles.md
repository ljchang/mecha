---
title: Design principles
sidebar_position: 2
description: The rules mecha's code keeps, why each one exists, and the things deliberately left out.
---

# Design principles

mecha is opinionated, and the opinions are load-bearing. This page states them
once, at the level of the whole system, so that the feature pages can get on
with explaining mechanism. Most of them were learned by getting something wrong
first.

## 1. Structural beats prompted

If a rule matters, it belongs in the type system or the loop, not in the system
prompt. A prompt is a request; a model under adversarial pressure can be argued
out of a request, and a model having a bad day can simply forget one.

The trifecta interlock is the clearest case. mecha does not tell the model "do
not exfiltrate data" — the model is never in a position to. Tools declare their
capabilities, the conversation tracks which have entered it, and a sending tool
is refused before it runs. Similarly, the path jail is not a rule tools are
asked to follow but a function every tool has to call, and containment is proven
after canonicalization rather than checked before it, because a symlink inside
the workspace is otherwise a way out of it.

The documents surface is the same move made outside mecha's own code. Google
sells several ways to reach a Doc; mecha asks for the one that covers *only
files it created or you handed it*, so a document nobody gave it is not
reachable — not because a rule forbids it, but because no token in the process
can name it. The choosing happens in Google's file chooser, outside the
model's context window, so nothing a document says can widen what the next run
can touch. A boundary you verify by reading a scope string does not have to
keep being true in every future diff.

## 2. Fail closed, and fail loudly

A security control that stops working must stop the run, not quietly stop
protecting. The recurring bug this prevents is the **silently degrading
sandbox**: something that still appears in `mecha tools`, still reads as
configured, and no longer does anything.

So a configured sandbox that cannot actually confine a command fails at startup
with instructions, rather than falling back to running unconfined — which would
be worse than having no sandbox at all, since `shell` declares narrower
capabilities when confined and the interlock believes it. A `pre_tool` hook that
crashes, times out, or returns an undefined exit code **denies**. A tool call
that could not be staged to the outbox returns an error rather than executing; a
full disk must not be the way around a review.

## 3. The loop knows nothing

The agent loop never learns which provider is behind it or where a tool came
from. Both are trait objects. A built-in `fs_read` and a tool from a third-party
MCP server are the same type to the loop; Anthropic and a local llama-server are
the same type to the loop. If code in the loop ever matches on a provider name,
the abstraction has leaked and the next backend will cost twice what it should.

The same reasoning runs one level up. `mecha-core` cannot reach into the CLI, so
a feature that only works when a human is watching a terminal cannot quietly
become load-bearing for a scheduled run. `mecha-slack` has no dependency on
`mecha-core` at all — which is why it is a separate crate rather than a module.
An invariant you can check by reading `Cargo.toml` is cheaper to keep than one
you have to enforce by reviewing diffs.

## 4. Policy is declared by configuration, never by the thing being governed

What counts as a "send", what is publishable, how far to trust a server: those
are the operator's decisions, and they are recorded where the operator can see
them. A tool does not get to nominate itself as safe, and a third-party MCP
server certainly does not.

This is why an MCP server's declared capabilities can be **widened** by config
but never narrowed by the server's own claim, why the outbox learns which tools
are publishes from `[outbox] publish_tools` rather than from the tool, and why
`/review now|later|auto` in the TUI is set by a slash command and never inferred
from a prompt. Anything that shares a context window with third-party text must
not be able to decide release policy.

## 5. Narrowing is always allowed; widening almost never is

Every layer that can affect what a run may do is ordered so that it can only
restrict. The dispatch order is **interlock → hook → approver**: a hook can
tighten policy and can never loosen security, and a hook's denial never reaches
the human, because mechanical policy is cheaper than an interruption and a hook
cannot be talked into clicking yes.

The exceptions are explicit and few — `trifecta = "allow"`, a subagent's
`trusted_output = true` — and each is documented as a risk decision rather than
a setting. Where widening would be genuinely dangerous there is deliberately **no
knob at all**: nothing lets untrusted-origin reflections into the learned rule
set, because a switch that admits third-party text into every future prompt is
the silently-degrading sandbox in a new costume.

## 6. A boundary is a function, not a discipline

"Remember not to pass the raw text through" holds until the first person in a
hurry. So the boundaries in mecha are things you have to call, and they have no
argument that turns them off.

The front door is the example worth studying. A stranger's free text goes to an
extractor with **no tools and no conversation history** — not one told not to use
tools, one issued a request with an empty tool list — and the privileged run
receives the typed extraction through a function that has no parameter capable of
returning the prose. Even the extractor's own summary stays behind, because a
paraphrase of an injection is the injection rearranged.

## 7. Staging is not sending

Anything outbound can be intercepted before it happens and left for a person to
release. Because that interception lives in the loop rather than in each tool,
"draft-only, never send" covers tools that have never heard of the outbox,
including third-party ones.

The useful consequence is that the safe configuration and the useful
configuration turn out to be the same one: an unattended overnight run that
drafts nine replies needs no write permission at all, because staging executes
nothing. A staged item also records the workspace it was drafted in and the taint
that was present when it was drafted — a deferred tool call means nothing apart
from the jail it was written against, and a reviewer approving one file while
reading another is exactly the failure the review exists to prevent.

## 8. Measure it, or it is not real

Anything a model says about its own work is hearsay, including a model's opinion
about whether mecha is working. So the parts of the system that could drift are
attached to something that can be counted.

[Eval cases](/docs/features/evaluation) are graded on the tool-call trace before
the prose, and end in a `verify` command's exit status where one applies. Repeat
runs report **pass^k** beside pass@k, because reliability decays faster than mean
success and a single-run scorecard cannot distinguish a flaky case from a solid
one. [Learned rules](/docs/features/learning) are kept because they flip a
counterfactual replay — the recorded prefix driven again with and without the
rule — not because a model liked them. And the checks that grade the *harness*
rather than the model (did the interlock fire, was a budget what stopped the
run, was a summary ever taken) exist because none of that is visible in the
answer text.

Runs themselves are counted the same way. Every finished run records **how it
went** as well as what it cost — stop cause, calls attempted against errors,
whether it stopped of its own accord with its last call failed — and
[that corpus](/docs/features/run-quality) is what lets a harness problem be
noticed at all. Before it, the only signal that something was wrong was a human
stepping in, so a run that quietly failed a third of its tool calls produced no
intervention and nothing downstream ever heard about it.

And a proposed change to the harness has to beat the original to survive: a
falsifiable prediction made before the measurement, arms paired by episode,
the winner confirmed on a holdout it was never selected on, and a work guardrail
that rejects a gain bought by attempting less. "Fewer errors" is trivially
achieved by doing nothing.

A corollary: never edit a recorded measurement to match a later result. A
retracted measurement is evidence about how much to trust the next one.

And the counting has a **sign**, which took a while to notice was missing. Every
metric above is phrased as a cost, so the system could rank two runs that went
badly and could not rank two that went well — and every signal that started a
loop needed the world to act first, because nothing represented what a run was
*for*. A [charter and a signed goal error](/docs/features/appraisal) are the
other half: what mecha is for, in your own words, and how far a run landed from
it. The honest first finding was that almost every run comes back with no label
at all, and that is published rather than tuned away — inventing precedence until
every run gets an interesting word manufactures the signal the measurement exists
to test for.

## 9. Evidence is kept; belief is gated

The two are stored differently and on purpose. Evidence is append-only and
cheap to keep. Belief — anything that will ride in a future prompt or be acted
on — has to pass a gate, and is revocable afterwards.

So a reflection whose origin is untrusted stays in the archive and is simply
never a candidate. A retired rule is flagged, never deleted, and the learner is
shown it as "measured harmful — never re-derive". A rejected draft returns the
request to `extracted` rather than to `closed`, because "not this reply" is not
"not this request". And distillation deliberately takes the *opposite* provenance
rule from learning: a tainted session still becomes an episode, with its taint
recorded on the episode, because losing the record of a real afternoon because a
web page was open would gut the memory — and an episode re-enters a future prompt
as untrusted evidence, never as trusted instruction.

## 10. State that is only correct after someone runs a command is state nobody trusts

Reconciliation happens on read, retention is a policy rather than an intention,
and defaults are resolved and **written down** rather than left implicit.

`mecha trigger add` records the workspace it resolved instead of leaving the
field empty, and `trigger show` prints the resolved default, because "where is
this jailed" must not be answered by an omitted line. `mecha work clean` keeps
the last *n* entries per producer and says exactly what it removed. The front
door's `reconcile` runs on its own rather than on a verb you have to remember.

## 11. The default has to be the safe thing, because it is what runs

Triggers are read-only unless the file says otherwise. Triggers read the global
config only, never a project's `mecha.toml`, because a cloned repository must
not be able to shape a scheduled agent run on your machine. Provider fallback is
empty by default — answering with a different model than the one you named is
worse than failing. Compaction is off by default, because paraphrasing someone's
conversation because it got long is their decision to make. Unfurling is off on
everything a model authors, with no parameter to enable it.

## What is deliberately not here

The omissions are decisions too, and stating them saves the same argument being
had twice:

- **No decay, TTLs, or usage-based eviction on learned rules.** The rule that
  fires rarely may be the one that must never expire. Only measured harm argues
  for retirement, and a human accepts the argument.
- **No policy built on model-rated confidence.** A model's certainty about its
  own output is not evidence.
- **No scheduled shell commands.** A trigger's action is a prompt. Scheduled
  commands are what cron is for, and giving them a home here would mean
  re-answering how they are confined and what environment they see.
- **No completion-gating on an LLM judge.** Summary validation is a grounded
  comparison of two texts both present in the same request, and an unusable
  verdict installs the summary with a warning — a run that must compact to
  survive still has to be able to compact.
- **No way for a subagent to launder untrusted content into trusted content.** A
  summary of a hostile page is still derived from hostile text. What delegation
  buys is that the raw content never enters the parent's context and the two
  halves of the trifecta can live in separate agents.
- **No escape hatch on the front door's extraction boundary**, no argument that
  returns a stranger's prose to a privileged run, and no fallback to passing the
  prose through when extraction fails — a failed extraction waits for a human.
- **No model in the gate, and nothing outside a closed set that applies itself.**
  `mecha diagnose` is the one place a model authors a change, and it prints the
  command that would falsify its own proposal rather than running it. A model is
  safe there precisely because being wrong costs one measurement — automated
  failure attribution is right about which step failed roughly one time in
  seven — and that property does not hold at the gate, which is why there is no
  model in the gate. A **config** change that clears the whole gate does apply
  itself, into a revertible override layer, and the keys it may move are exactly
  the ones a run can be launched with; architecture reaches a person however well
  it scored, and a `security`-class proposal is never measured at all, because a
  loop that can argue for widening its own confinement will eventually argue well.
- **No model-authored charter line, and no model-reported mood.** The charter is
  edited by a person with a text editor — a model that could edit its own
  standing priorities could edit its way around every other guardrail — and the
  affect label is a pure function of the record rather than something a model
  announces about itself. A self-report is unfalsifiable, drifts, and is exactly
  what a page saying *"you have failed your owner"* is aiming at.
- **No significance test on a candidate.** With a few dozen episodes the noise
  is the model's sampling rather than the measurement, and the answer to that is
  repetition, not a p-value over one sample. The raw win/loss/tie counts ride on
  the verdict so a person sees what it was decided from.

## Where these came from

Most of the above is the generalisation of a specific bug. The unabridged
version, with the incident behind each rule, is [`CLAUDE.md`] and its
companion [`ARCHITECTURE.md`] in the repository;
this page is the part that survives being restated without the scar tissue.

[`CLAUDE.md`]: https://github.com/ljchang/mecha/blob/main/CLAUDE.md
[`ARCHITECTURE.md`]: https://github.com/ljchang/mecha/blob/main/docs/ARCHITECTURE.md
