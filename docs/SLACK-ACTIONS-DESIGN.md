# Executable actions from Slack — design

**2026-08-14.** Tapping a button on a phone to run a fix: a doctor remedy that
restarts a failed unit, a draft released or rejected, a trigger probed or
silenced. This document decides which actions may be tap-executable, under what
trust, confirmation and idempotence rules, and what ships first.

The Slack surface already executes from taps — the outbox Send/Reject cards
spawn `mecha outbox send -y` as a child process
(`mecha-cli/src/slack/connector.rs:1081`), tainted drafts get a two-step red
confirm (`connector.rs:1041`), approval cards resolve a waiting run
(`connector.rs:1301`). And the doctor rendering explicitly refused to grow
buttons until this pass existed: *"tap-to-run-a-remedy needs its own design
pass (approval semantics, which tier may tap, idempotence of a re-tapped
restart) and must not ride in as a footnote on a rendering change"*
(`mecha-cli/src/slack/doctor.rs:85`). This is that pass.

One sentence to carry through: **a tappable argv is authored by deterministic
code from typed store state, and the tap is a gated human; no model output and
no message text is ever between a finding and a command line.** Every decision
below either enforces that sentence structurally or names what happens where it
cannot hold.

---

## 1. The action set is a closed enum, not an allowlist of strings

**Decision: a Rust enum, `Action`, in `mecha-cli/src/slack/actions.rs`, whose
variants carry typed fields — and the only way to execute anything from a tap
is to hold a value of it.**

An allowlist of command strings is the tempting shape and the wrong one: a
string list is data, and data gets appended to. An enum is code — adding an
action is a diff someone reviews, the compiler forces the executor's `match` to
handle it, and there is no runtime state whose corruption widens the set. This
is the same move as `Decision::Allow | Deny | Blocked`
(`mecha-cli/src/slack/approve.rs`, and the hooks section of `CLAUDE.md`): the
split lives in the type, so no wording a caller chooses can escape it.

It lives in `mecha-cli/src/slack/`, beside `approve.rs`, and **never in
`mecha-slack`** — the crate boundary is the whole point of the fourth crate. A
transport that knows what a remedy or an outbox item is has learned about the
agent's world, and the invariant "checkable by reading `Cargo.toml`" dies. The
Block Kit builders keep taking strings; the meaning stays on this side.

It is also **not in `mecha-core`**. `doctor::Remedy` stays what it is —
`{description, argv, needs_terminal}` (`mecha-core/src/doctor.rs:59`), a display
and terminal-execution shape. Which remedies are *tappable* is a property of
the surface holding the human, not of the examination: the terminal runs any
remedy because the human typed `y` at a real screen showing the whole finding
(`mecha-cli/src/commands/doctor.rs:174`); a phone gets the narrower set this
document defines. Encoding tappability in core would let every future surface
inherit a decision made for this one.

### The inventory, classified

| Action | Class | Tappable | Phase |
|---|---|---|---|
| `outbox send` (untainted) | outward-facing | one-tap | shipped |
| `outbox send` (tainted) | outward-facing | two-step red confirm | shipped; §8 tightens it |
| `outbox reject` | terminal but local — nothing leaves | one-tap | shipped |
| `systemctl --user restart mecha-*` (doctor remedy) | local, reversible | one-tap, re-examined (§5) | **1** |
| `trigger run <name>` (doctor's manual probe) | evidence-not-fire; read-only unless the trigger file says otherwise | one-tap | **1** |
| `trigger cancel <name>` | reversible — the run stops at a safe point, partial turn kept | one-tap | **1** |
| `trigger disable` / `enable <name>` | reversible flag (`commands/trigger.rs:604`) | one-tap | 2 |
| `frontdoor needs-info <seq>` | reversible, needs free text | modal | 2 |
| `frontdoor close <seq>` | terminal for a stranger's request, requires a reason | modal, two-step | 2 |
| `mecha-mail import <legacy>` (doctor remedy) | additive credential move | one-tap | 2 |
| `mecha-mail auth <account>` | `needs_terminal` — an OAuth flow | **never a button** (§below) | — |
| `mecha outbox review` / `frontdoor list` (doctor remedies) | terminal surfaces | translated, not spawned (§6) | 2 |
| `trigger delete` | destructive — the schedule is gone | **never** | — |
| `work clean` | destructive — directories removed | **never** | — |
| `proposals accept` (rule changes) | prompt-tenure — rides in every future cached prefix | **never** | — |

**The `needs_terminal` remedies get no phone execution path, and the phone UX
is the one already shipped**: the command rendered as copyable code with
*"needs a terminal — run it where there is one"* (`slack/doctor.rs:125`). The
honest answer to "my mail auth died and I'm on a train" is a command you paste
when you reach a keyboard. The tempting alternative — relaying Microsoft's
device-code flow into the DM, since it needs no local browser — is deliberately
not built: an agent surface that sometimes says "go to microsoft.com/devicelogin
and enter this code" has taught its owner to follow sign-in instructions
arriving in a chat window, which is the reflex every phishing campaign in the
world is trying to install. The one place mecha must never optimise for
convenience is the shape of a credential prompt.

**`work clean` and `trigger delete` do not belong on a phone in any phase.**
Not because a button couldn't confirm — because both already have better
answers. Retention is *a policy, not an intention*: the nightly runs
`work clean`, so a phone button would be a manual override of a policy that
needs no manual anything. And a trigger you want gone from a train is a trigger
you want `disable`d — reversible, sufficient, and exactly what phase 2 ships.
Deleting is a decision for a screen that can show you the trigger file and its
ledger. `proposals accept` is the same reasoning at higher stakes: an accepted
rule rides in every future run's cached prefix, the longest half-life in the
project, and its review is a diff — a reading task, not a tap.

---

## 2. Provenance: where the rule lives

The load-bearing invariant: **a tappable argv is assembled by a total function
from an `Action` value, and an `Action` value is only constructible from typed
store state.** Never from model output, never from message text, never from a
string a Slack payload carried. Otherwise a prompt injection in a thread — a
fetched page that gets the model to print a convincing card-shaped message, or
a poisoned finding detail — composes a button that a tired human taps at 2am.
The rule cannot be a discipline ("remember not to interpolate"); it has to be a
function, which is this codebase's recurring idiom
(`Record::for_privileged_run` has no argument that returns the prose; the
`Decision` prefix is chosen by the loop from the variant, never by the
approver).

Three functions carry it:

- **`Action::argv(&self) -> Vec<String>`** — a total `match` from variant to
  command line. The verb, the flags, the subcommand: all literals in the match
  arms. The only non-literal parts are the typed fields — an outbox id, a
  trigger name, a unit name — and each is validated by the arm that uses it
  (below). There is no `Action::Raw(Vec<String>)` variant and never will be;
  the executor takes `Action`, not argv, so nothing can hand it a command line
  it didn't derive.

- **`Action::from_remedy(&Remedy) -> Option<Action>`** — the recogniser that
  turns a doctor finding's remedy into a button. It matches the argv *shape*:
  `["systemctl", "--user", "restart", unit]` where the unit matches
  `mecha-[a-z-]+.service` becomes `Action::RestartUnit`;
  `["mecha", "trigger", "run", name]` becomes `Action::TriggerRun`. Anything
  unrecognised — including every `needs_terminal: true` remedy — renders as
  copyable code exactly as today, so **an unrecognised remedy degrades to
  display, never to execution**. This is why core needs no change: recognition
  is the surface's narrowing of the terminal's wider trust, and a new remedy
  shape in core is display-only on Slack until someone deliberately adds a
  variant here. Fail-closed by construction.

- **`Action::from_payload(action_id, value) -> Option<Action>`** — the parser
  for the button press coming back. The `action_id` is the verb, fixed at
  compose time from a closed set of literals (`slack_action_restart_unit`,
  `slack_outbox_send`, …); the `value` is **the object id only** — an outbox
  item id, a trigger name — never a command fragment. This is the pattern the
  shipped buttons already follow (`connector.rs:1007`: action id + item id,
  and the comment at `connector.rs:1240`: *"never on the button's value, which
  is a correlation id chosen by whatever composed the message"*). The object
  id is then resolved against its store before anything runs: an outbox id
  through `OutboxStore::item` (which errors on no match and on ambiguity,
  `mecha-core/src/outbox.rs:313`), a trigger name through `store.get(name)`
  (`commands/trigger.rs:194`), a unit name by re-appearing in `systemctl`'s
  own failed-units listing at tap time (§5). **Fixed verb, store-resolved
  object** — the only bytes that travel through Slack and back are an id whose
  meaning the store, not the payload, supplies.

Note what this makes true end to end: the card is composed by deterministic
code reading a store (doctor's examination is pure functions over store roots,
`doctor.rs:110`; the outbox cards read `OutboxStore`); the payload carries a
verb from a closed set and an id; the executor re-derives the argv from the
enum. A model's output can appear *inside* a card only as quoted content (a
draft's summary), never as its structure — the model cannot mint an
`action_id`, and a hostile string in a store field is inert because no store
field is ever interpolated into argv. The one store field that does reach an
argv is the trigger name and the unit name, both of which are filenames/unit
names validated by shape and by existence, not free text.

---

## 3. Trust tiers: owner-only, with no exceptions to design

**Decision: every action is owner-tier, and no non-owner tier ever sees a
button.**

The connector's gate already makes this nearly free: `binding::check` runs on
every interaction before any button is honoured (`connector.rs:1243`), on
`payload.user.id` — the field Slack signed — and a non-owner's press is dropped
with a warning log. The question this section exists to answer is whether any
action is safe enough to widen, and the answer is no, for two reasons that are
not the same:

- **Every finding is the private surface.** A doctor report names mail
  accounts, stuck drafts, trigger names, unit names — the shape of the user's
  machine and life (`slack/doctor.rs:10` already states this for the report;
  it holds a fortiori for buttons on it). There is no action whose *card* a
  stranger should see, so there is no action whose button they could press.
- **The two-tier rule is settled and this is not the feature that reopens
  it.** SLACK-RESEARCH §3: two tiers is a boundary; three is a policy, and a
  policy needs evidence this project does not have. The "trusted colleague who
  may restart a unit" tier is the same unmeasurable middle tier as "trusted
  colleague who may run read-only commands", one notch dressed up as ops.

And the standing rule from the memory notes applies unchanged: **the factory is
never an owner channel.** No binding on the factory box, therefore no actions
from it, therefore nothing to decide.

---

## 4. The confirmation ladder

Three rungs, and the rung is decided by the action's class, not per card:

**One tap** — local, reversible, or self-limiting actions: unit restart,
trigger run, trigger cancel, trigger enable/disable, outbox reject, and
untainted outbox send. Untainted send stays one-tap deliberately, matching the
terminal: `mecha outbox send <id>` for a single untainted item does not confirm
either (`commands/outbox.rs:707` — "a single untainted item is not [confirmed],
which keeps `send <id>` exactly as direct as it was"). The parity matters: a
surface that is more ceremonious than the terminal for the same act teaches the
owner that ceremony is noise.

**Two-step** — anything where the first tap cannot have shown enough to decide.
Today that is exactly one case: the tainted draft, whose first tap rewrites the
card into the full arguments in red with **Send anyway** (`connector.rs:1041`).
Phase 2 adds the modal actions — `frontdoor close` requires a reason
(`commands/frontdoor.rs:89`), `needs-info` requires the question — where the
second step is Slack's modal, opened within the three-second `trigger_id`
window (SLACK-DESIGN §5.2). A required free-text field is itself a confirmation:
nobody types a reason by accident.

**Never on this surface** — the destructive and prompt-tenure rows of §1's
table. The ladder has no rung for them because no amount of confirming fixes
the real problem, which is that the phone cannot show what the decision needs:
the trigger file and its ledger, the work entries and which bundle sources pin
them, the rule diff against the prompt it will ride in.

**No `Always`, no blankets across runs, no remembered approvals** — the same
decision `approve.rs` already made and for the same reason: a connector runs
for months, and a widening made once on a phone is a blast radius nobody
re-reviews. Every tap authorises exactly one execution of exactly one action.

---

## 5. Idempotence and replay, per action

Slack's redelivery semantics across a dropped socket are undocumented
(`connector.rs:38` assumes redelivery happens), the event dedup ring covers
events but not interactions, and humans double-tap. The defense is decided per
action, because the right one differs — and in every case it is **the store,
not the payload**, that makes the second delivery harmless. A nonce in the
payload would protect only against replay; store-state guards protect against
replay, double-tap, *and* a stale card pressed hours later, which is the case
that actually bites (the 2am approval card pressed in the morning was a real
bug, `connector.rs:246`).

| Action | What makes a second delivery safe |
|---|---|
| `outbox send` | The store flock is held across execution (`commands/outbox.rs:697`), and `resolve` refuses a non-pending item (`outbox.rs:348`). The second child exits "is sent, not pending"; the card was already rewritten into a terminal record. Two layers, both needed: the card rewrite is UX, the store guard is correctness. |
| `outbox reject` | Same pair — `resolve` guards, card rewrites. |
| `restart unit` | **Re-examination before execution.** At tap time the executor re-runs the failed-units listing (`commands/doctor.rs:198`); if the unit is no longer failed, nothing is restarted and the card says "already recovered — nothing run". This is the doctor-specific rule: a restart is naturally idempotent against a *failed* unit but disruptive against a *running* one — `mecha-triggers` restarted mid-run cancels whatever it was doing — so the finding must still be true when the tap lands, not just when the card was posted. The card is also rewritten to "restarting…" on dispatch, so the button is gone before the child runs. |
| `trigger run` | The trigger's own non-blocking flock: a second run while the first is in flight is a recorded overlap-skip, not a second fire (the store already answers this, and a skip is written to the ledger). A manual run records a row with no slot, so even N replays never advance the schedule. The cheapest possible answer: the primitive was already safe. |
| `trigger cancel` | A sentinel file the runner polls (`commands/trigger.rs:930`) — writing it twice is writing it once. `cancel` of a non-running trigger reports "not running" (`trigger.rs:198`). Idempotent by construction. |
| `trigger enable/disable` | Setting a flag to its current value is a no-op (`trigger.rs:604`). Idempotent by construction. |
| `frontdoor close` / `needs-info` (phase 2) | The state machine refuses transitions from terminal states, same shape as the outbox's pending check; the modal's submit carries the seq, re-resolved against the store. |

**Buttons are retired eagerly everywhere**, extending the rules the connector
already enforces: controls rewritten at completion because "a Stop button for a
run that already ended is a lie the reader has to test" (`connector.rs:727`),
approval cards retired on expiry (`connector.rs:1125`), orphaned threads'
controls rewritten before anything else on restart (`connector.rs:148`). An
action card follows the same law: rewritten on dispatch, rewritten again with
the outcome, and a press that races the rewrite is caught by the store guard
behind it. The card rewrite is never the *only* defense.

---

## 6. Outcome reporting: the store answers, not the child

**Decision: every action's outcome is reported from a re-read of the state it
was supposed to change, rendered as an update of the card that launched it —
and the child's exit code alone is never the answer.**

This is the TUI's watch pattern (SLACK-DESIGN §5.4: *"the result is collected
by polling the store, not the process — a child that died without writing is
otherwise indistinguishable from one still working"*), and phase 1 brings the
shipped outbox path into line with it: `resolve_draft` today reports from the
child's exit status and first stderr line (`connector.rs:1094`), which is
almost always right and wrong in exactly the case that matters — a child killed
after the send but before exiting reports "failed" over a mail that left. The
truth is `item.status` and `item.error`, both durable on the item
(`outbox.rs:364` — a failed release records the error and stays pending), so
the card update reads them.

Per action, the store that answers:

- **outbox send/reject** — the item, re-read: `sent` / `rejected` / still
  `pending` with `error` set. A pending-with-error card says so and keeps the
  draft offered, because the draft is still good; the delivery was not.
- **restart unit** — `systemctl --user is-failed <unit>` after the child
  returns: the unit's state is the outcome, the restart command's exit is not.
  "Restarted, and it is running" and "restarted, and it failed again — the
  fix is upstream" are different findings, and the second one matters more
  (it is why `unit_finding` orders the dead-auth fix first,
  `commands/doctor.rs:237`).
- **trigger run** — the ledger: the new row's `status` and `error`
  (`doctor.rs`'s trigger check reads the same rows). A trigger run can take
  twenty minutes, so the card goes to "running — started HH:MM" on dispatch
  and the watch polls the ledger with the same caps that retire a wedged watch
  in the TUI. The row is the record; a second copy could disagree with it.
- **trigger cancel/enable/disable** — the trigger file and the running marker,
  re-read.

Where the outcome lands: **the card, updated in place**, exactly as approval
and draft cards already resolve into "`x` sent by @who" terminal records. A
thread reply is added only when the outcome arrived long after the tap (a
trigger run finishing), because a card edit fires no notification and a person
who tapped twenty minutes ago has stopped watching.

**The terminal-surface remedies translate rather than spawn.** Doctor's remedy
for stuck drafts is `mecha outbox review` — deliberately, "doctor never
releases a draft" (`doctor.rs:336`) — and for the frontdoor queue it is
`frontdoor list`. Spawning either from Slack is meaningless (there is no
terminal), so in phase 2 the finding's button is **Review here**, which posts
the pending items as the draft cards the connector already knows how to make
(`offer_drafts`, `connector.rs:961`, generalised to take an item set rather
than a session scope). The remedy's *intent* — put the stuck thing in front of
the human — is honoured; the argv is not executed, because it was never an
action, only a doorway. Doctor's own rule survives intact: the button that
appears is still per-item send/reject with the taint ladder, never "release
what is stuck".

---

## 7. Audit: a tap ledger, because "who" exists nowhere else

**Decision: an append-only `~/.mecha/slack/actions.jsonl`, one row at dispatch
and one at resolution, carrying `{at, user_id, action, outcome}` with `action`
as the serialized typed enum.**

The stores already record most of *what happened*: the outbox item is its own
audit record and nothing moves to an archive (`outbox.rs:346`), the trigger
ledger records manual runs as evidence, the thread keeps the rewritten card. What
none of them record is **who asked and from where**: `mecha outbox send` has no
actor field — the same command serves the terminal, the TUI and this connector
— and the card's "sent by @who" lives in Slack, which is a rendering, not a
store this system can read back. A security surface whose "who pressed it" is
only reconstructible by scraping a chat history has no audit at all.

Two rows rather than one, because a crash between dispatch and outcome is
exactly when the record matters: a dispatch row with no outcome row is the
durable evidence that a tap launched something whose result was lost, which is
the same reasoning as the trigger ledger writing skips ("why did I not get my
briefing" has to be answerable — here, "did my tap do anything" is). The rows
share a tap id; the resolution row also carries the store-read outcome from §6,
so the ledger agrees with the stores by construction rather than by parallel
bookkeeping.

Deliberately *not* built: a second copy of the action's effect. The ledger
records the tap and points at the store; it never restates the item, the
ledger row, or the unit state, because a second source of truth is the disease
this project keeps naming.

---

## 8. The interlock, and the one place the argument is subtle

The trifecta interlock refuses `external_send` tools once private data and
untrusted content share a conversation, because a model holding both plus a
send is the standing emergency. **A tap is outside that geometry, and the
argument deserves to be written out rather than waved at:**

The interlock guards *model-initiated* action inside a context window that may
contain an attacker's words. A tap has no context window. The card was composed
by deterministic code from typed store state (§2); the argv is derived from a
closed enum by a total function; the human pressing it is proven by
`payload.user.id` against a binding written only by `mecha slack link`. There
is no point in the path where third-party text chooses anything — not the
verb, not the object, not the timing. The one channel an attacker has into a
card is quoted *content* (a draft summary, a finding detail), and content is
inert here: it can try social engineering on the human, which is what review
surfaces exist to survive, but it cannot compose structure. This is CaMeL's
split again at button scale: the prose can be hostile; the typed thing that
executes was never touched by it.

**The subtle case is releasing a tainted draft**, because there the tap is not
executing machine-authored intent — it is *ratifying model-authored output from
a conversation the attacker may have steered*. The interlock deliberately does
not fire at staging ("staging skips the interlock… nothing leaves the machine
at stage time"), which makes the release the moment of consequence, and the
whole design leans on the release being *informed*. The existing rules already
encode "approval predates what armed the taint": tainted drafts never
auto-release in the TUI, and `/review auto`'s scope excludes them, because a
yes given before the hostile page was fetched authorises nothing about what
came after.

**Decision: tainted releases stay two-step forever on this surface — a phone
tap is presence enough only when the phone could show everything.** Which
yields the one tightening phase 1 makes to shipped behaviour:
`ask_to_confirm_tainted` shows the full arguments through
`truncate_for_slack`, which cuts at ~2,600 characters (`connector.rs:1053`,
`:1460`). For a draft whose arguments fit, the red card *is* the TUI's
full-arguments-in-red review and the confirm tap is as informed as the
terminal's `y`. For a draft whose arguments were cut, the reviewer is
approving bytes the surface could not show — the exact "reading one file while
approving another" failure the outbox's workspace-resolution rule exists to
prevent, in miniature. So **when the arguments are truncated, the second step
carries no Send anyway button**: the card shows what fits, says what was cut,
and names the terminal (`mecha outbox show <id>`) as the place this draft is
released. Reject stays available — declining needs no completeness. The rule
in one line: *you may reject what you cannot fully read; you may not send it.*

---

## 9. Phase plan

**Phase 0 — shipped.** Outbox send/reject cards with the tainted two-step,
approval cards, Stop/Mode. The precedents this design generalises.

**Phase 1 — the typed layer, and doctor's two buttons.**

1. `Action` enum, `argv`/`from_remedy`/`from_payload`, with tests that
   `from_remedy` refuses every `needs_terminal` remedy and every unrecognised
   argv shape (fails on the old display-only behaviour only by *adding*
   buttons, never by executing more than the two recognised shapes).
2. The tap ledger (§7), and the shipped outbox buttons retrofitted through
   `Action` and the ledger — one execution path, not two generations of one.
3. Doctor cards grow buttons for exactly two remedy shapes: **restart unit**
   (with re-examination, §5) and **trigger run** (the manual probe). Both are
   the remedies doctor most often proposes for the failures that actually
   happened (the revoked-token incident's unit failures; a trigger whose last
   run failed).
4. **trigger cancel** as a button on the doctor card when a run is in flight —
   the one action a phone most plausibly needs at an inconvenient hour.
5. Outcome-from-store reporting (§6) for everything above, including the
   existing draft cards' move off exit-code reporting.
6. The truncated-tainted rule (§8).

**Phase 2 — the modal actions and the translations.**

- `trigger enable/disable` buttons (on the doctor card's trigger findings, and
  on a `triggers` command word listing, same shape as `doctor`).
- `frontdoor close` / `needs-info` via modals with required text fields.
- **Review here** — the terminal-surface remedies translated into item cards
  (§6), which closes the loop on doctor's outbox and frontdoor findings.
- `mecha-mail import` for the legacy-store finding.

**Never built** — named the way POLL-DESIGN names the Keynote no-build, so the
absence reads as a decision rather than a gap:

- **A generic "run this command" button, or any `Action::Raw`.** The closed
  enum is the design; a raw variant is its deletion.
- **A button on any `needs_terminal` remedy**, including every OAuth flow, and
  any relay of a device-code sign-in through Slack (§1).
- **`trigger delete`, `work clean`, `proposals accept` from a tap** (§1).
- **Any non-owner action, any factory-hosted action surface** (§3).
- **Auto-release of tainted drafts, an `Always`/blanket approval for actions,
  or a `/review auto` equivalent** — release policy set from a surface other
  people can post into is not a policy (SLACK-DESIGN §5.4), and a standing
  approval made on a phone outlives the attention that made it.
- **A parallel executor.** Every action spawns the CLI as a child process,
  exactly as `resolve_draft` and the TUI's modals do: one implementation of
  each verb, no way for this surface to do something the command line cannot,
  and every store guard (the send flock, the trigger flock, the pending
  checks) inherited rather than reimplemented.
