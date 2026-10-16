# Messages between machines on the owner's tailnet

**Status (2026-10-16): designed, not built; Q1–Q3 ruled (§6); the wake Q1
asks for is open (§7).** The owner's ruling that starts
it: *"tailnet machines count"* — the owner's own machines on their tailnet are
inside the trust boundary. This document is what follows from that sentence,
and what deliberately does not.

`MESSAGING-RESEARCH.md` designed messages between sessions on one machine
(built: `mailbox.rs`, `mecha msg`, the `message_send` tool), and put
cross-machine transport out of scope because it "changes the trust math
entirely" — a message to another machine was a send to a destination the
model names, an `Egress::Chosen` sink the interlock refuses once a
conversation holds private data and untrusted content. With the owner's
machines inside the boundary, that changes: a message to another of the
owner's machines leaves nothing the owner does not hold. The research's other
decisions all stand — taint travels, text only, receiver-side policy, a
message is never the user.

## 1. What it is for

The owner runs mecha on more than one machine: the GB10 (models, overnight
triggers), a laptop (where the owner is), and now a second Mac. Today an
overnight run on the GB10 can message the `chat` producer on the GB10 and
nowhere else; a session on the laptop cannot ask the GB10's session anything.
The useful cases are the local ones, one hop longer:

- an overnight trigger on the GB10 reports to the owner's chat on the laptop;
- a session on the laptop hands a long job to the GB10, whose run answers back;
- two sessions on two machines working one problem check each other.

## 2. Decisions

**D1. Trust: the owner's machines, by the tailnet's own identity.** A message
is accepted from a peer only when the tailnet says the request comes from the
owner — the same test `mecha serve` already applies to every request:
`Tailscale-User-Login` equal to `[web] owner_login`, the header `tailscale
serve` sets for a request from a node the owner's login owns. A tagged node,
another tailnet user, or a request that did not come through `tailscale serve`
is refused. **The owner is verified; the machine is not.** `tailscale serve`
proxies to `mecha serve` on loopback and passes the *user's* identity, not the
sending node's, so which of the owner's machines sent a message is the
sender's claim. That is acceptable only because the ruling trusts every one of
the owner's machines alike: the machine name is a label for addressing a
reply, recorded as the sender's claim, and never an input to any trust
decision (review of #700).

**D2. Transport: the receiver's `mecha serve`, one owner-gated route.** `POST
/api/messages` on the peer's `mecha serve`, behind the same owner guard as
every other route. The body is a `MailboxMessage` without its id and status:
`to`, `body`, `reply_to`, and the harness-stamped `from`, `from_session` and
`taint`. The route writes it into the receiver's own mailbox store through
`MailboxStore::send` — the same caps, the same duplicate check, the same
recipient lock. `from` stays a producer name (`work::valid_producer` refuses
`@`); the machine goes in a new optional `MailboxMessage::from_machine`,
`None` for a local message and a default for an older build's file, and
surfaces show the pair as `chat@laptop`. "The same owner guard" includes
the rest of a `POST`'s gate: the `x-mecha-request: 1` header every mutating
route requires. The duplicate check's key —
`(from, body, reply_to)` today — gains `from_machine`, or two machines' `chat`
sending one short body to one recipient would coalesce into a single message
(review of #700). From there nothing is new: the receiver's runs claim it at a
turn boundary exactly as a local message. A machine that does not run `mecha
serve` cannot receive; the route refuses where `web` is off, as every route
does, and where the receiver's own `messages` switch is off, since nothing
there would ever claim what it stored.

**D3. Finding the machines: the tailnet is the directory, automatically.**
The owner's ruling (2026-10-15): *"Let's go automatic. No agent means no need
to register."* A machine finds its peers from `tailscale status --json`, which
lists every node with its owner, whether it is online, whether it is tagged,
and its MagicDNS name:

1. keep the nodes the owner's login owns, online and untagged;
2. ask each for its agents (D7) at its MagicDNS name, through `tailscale
   serve` — the door the web app already uses;
3. a node that answers, as the owner (D1), is a peer under its MagicDNS
   short name (`spark` for `spark.tail1234.ts.net`). A node not running
   `mecha serve` has no agents to offer and is no peer: nothing registers a
   machine, and nothing needs to. A node of the owner's that is offline, or
   answered within the last day and does not now, is listed as
   **unreachable** rather than dropped, so a peer that sleeps reads as one
   that is asleep (review of #700).

The `tailscale status --json` fork is the one `system/source.rs` already runs,
with its budget and timeout; D3 reads node names from it, which the system
series deliberately leaves out.

The list is read when a send or a listing needs it, cached for 30 seconds at
most, so a machine that sleeps drops out and one that wakes comes
back without anyone doing anything. `[messages.peers]` in the global config
(never a project layer, as `[messages]` already is) stays optional, for what
the tailnet cannot say:

```toml
[messages.peers]
nicknames = { gpu = "spark" }  # a nickname for a node
exclude = ["old-mbp"]          # a node of the owner's never treated as a peer
```

`message_send`'s `to` accepts `chat` (this machine) or `chat@spark`; an
unknown or unreachable machine is an error that lists the peers found.

**D4. Egress: none, because the boundary moved — and the taint moves with
it.** A send to a peer is classified as a local send is: no `Egress` class,
so the interlock does not refuse it. **What makes that hold is that `to`'s
machine half resolves only against the discovered peer set (D3)**: the model
names a peer, never a host or a URL, so the tool has no destination the model
chooses — the whole difference from `http_fetch`, which is `Chosen` because
the model picks the host (review of #700). `MessageSendTool`
declares none of the four capabilities today because "the payload lands in
`~/.mecha`, owner-only, same uid — no exfiltration channel"; the ruling
extends the same reason to the owner's other machines. What keeps this safe is
the research's rule, unchanged: the sender's conversation taint is stamped by
the harness and merged into the receiving conversation before the text lands.
A tainted run on the GB10 can still report to the laptop; the laptop's session
then treats every external send exactly as if it had read the hostile page
itself. Exfiltration *to* a peer moves data between two of the owner's
machines, which the ruling places inside; exfiltration *from* a peer is the
receiving run's interlock, which now knows what the message carried.

**D5. Fail closed on the wire.** A message whose `taint` is missing or
unreadable is untrusted on arrival, as a local one written by an older build
is. That needs a change below the route: `MailboxStore::send` stamps
`taint_recorded: true` on every message today, so a wire body with no taint
would land recorded *clean*. The route must store a missing taint as
`taint_recorded: false` — the flag the fail-closed rule keys on — through a
send that takes it (review of #700). **A remote message is always
untrusted on arrival**: the receiver takes the sender's taint and adds
`untrusted`, so a peer can widen a message's taint but never narrow it. The
route accepts a `taint` field from anything that presents the owner's header —
including an unconfined `shell` in an injected run on the owner's own machine
— and the research keeps "trust asserted by a sender" deliberately absent;
this is that rule, kept across the wire (review of #700). `from_machine`
keeps a peer's message apart from the owner's own `mecha msg send` here.
A peer on another build may send fields this one does not know: unknown
fields are ignored, never an error — a closed set written to an append-only
store is a wire format. A peer's reply that cannot be parsed is an error to
the sender, never a delivered message.

**D6. No store-and-forward in phase 1.** A send to an unreachable peer fails
at once, to the sender, as a tool error the model can route around ("`spark`
did not answer: its `mecha serve` is not reachable"). Queuing for a peer that
comes back later is phase 3, if the cases above need it.

**D7. Finding the agents: each machine lists its own.** Every live session
already registers itself — a marker naming its producer, session and pid,
which `mecha msg agents` reads and liveness-checks. The listing becomes
reachable from outside as an owner-gated `GET /api/messages/agents` on each
`mecha serve`: this machine's producers (its triggers and `chat`) and which
have a live session. With D3 that is the whole of discovery — the tailnet
lists the machines, each machine lists its agents — and a read-only
`message_agents` tool, and `mecha msg agents --all`, show it as one table:

```
laptop (this machine)   chat ● live        morning-brief
spark                   chat ○             nightly-graph ● live    hud-refresh
mbp                     unreachable
```

The listing is more than the markers: `MailboxStore::agents()` returns only
live sessions, so the producers with none — `chat` between sessions, the
triggers — come from the trigger store and the known producer names, joined
with the live markers. A peer that does not answer is listed as unreachable,
never as empty. A listing's names are checked as the receiver checks its own
(`work::valid_producer`, a bounded count) and anything else is dropped, so a
peer's answer is only ever producer names (review of #700). Like
`message_send`, the listing carries no egress class: it reads from the
owner's own machines.

**D8. The inbound axis.** A delivered message's body carries its sender's
taint, merged as for a local one (D4), so its content is governed whatever
the transport. `message_agents`' listing from a peer is not marked
`.from_outside()`: it is the owner's machines answering with producer names
the owner configured, and it never carries a message body. A peer's error or
reply text is shown to the sender as the tool's own failure, never as
external content it must act on.

## 3. What a message still cannot do

Everything `mailbox.rs` says a message cannot do, across machines too: it is
not the user; it cannot answer an approval prompt, change config, or carry
files or history; a path in it is a path the receiver's jail resolves. A
remote message is labelled as another agent's words *on another machine*,
and the receiver's permissions, hooks, outbox route and interlock govern
everything it provokes.

## 4. Out of scope

- **Anyone but the owner.** Another tailnet user, a tagged node, or a shared
  node is outside the boundary the ruling drew; supporting them is a
  different feature (A2A, `external_send`) with the research's trust math.
- **The internet.** The only door is the tailnet, as for the web app.
- **Sessions of other agents.** Claude Code sessions messaging each other is
  a separate system; mecha's peers are mecha.
- **Waking a machine.** A peer that is asleep is unreachable (D6).

## 5. Build order

1. **Receive.** `POST /api/messages` on `mecha serve`: the owner guard and
   the `messages` switch (D2); `MailboxStore::send` with `from` a producer and
   the sender-claimed machine in `from_machine` (D1, D2), the duplicate key
   widened by it; a missing taint stored unrecorded and every remote taint
   with `untrusted` added (D5); delivery labelled with the machine —
   `render_delivery` says "on this machine" today, and a remote message says
   which (§3), **in the interpolated part only**: `mailbox::DELIVERY_STEM`
   is what `agent::is_harness_voice` matches, and a delivery that falls off
   that list is mined as an owner correction; and `GET /api/messages/agents`
   (D7), built from the live markers and the trigger store. Tested with a
   forged header (refused), another login (refused), a missing
   `x-mecha-request` (refused), the owner's well-formed `POST` (stored — the
   happy path), a missing taint (stored untrusted), a clean asserted taint
   (stored untrusted), two machines' identical bodies (both kept), a remote
   delivery still recognised as the harness's voice, and a listing carrying a
   non-producer name (dropped).
2. **Find and send.** Peers from `tailscale status --json` (D3), with
   `[messages.peers]` for nicknames and exclusions — a field on both
   `MessagesConfig` and `MessagesLayer`, which both deny unknown fields; `message_agents` and
   `mecha msg agents --all` list every machine's agents (D7);
   `message_send` and `mecha msg send` accept `producer@peer`; the send
   stamps taint exactly as a local one and posts to the peer, failing at
   once when it does not answer. The `Egress` class of `message_send` is
   unchanged (none) — the test is that a tainted conversation can still send
   to a peer, and the receiving conversation comes out tainted. Discovery is
   tested over a recorded `tailscale status --json`: another login's node, a
   tagged node, an offline one and one with no `mecha serve` are none of
   them peers.
3. **Later, if the cases need them:** store-and-forward for an unreachable
   peer; a reply addressed back to `producer@peer` from the receiving run.

## 6. Rulings (the owner, 2026-10-15)

- **Q1. A remote message can wake an idle session.** No remote-only rule
  holds a message back: it follows the receiver's own `[messages] inbound`
  policy as a local message does. **But nothing wakes an idle session today,
  for a local message either** (review of #700): `accept` folds messages into
  a run already in flight, at its next turn boundary, and a session waiting
  for the owner to type starts no turn. So the ruling sets the intent, and
  the wake is new work for local and remote alike — §7.
- **Q2. One mailbox per machine.** A message lands on the machine it is
  addressed to (`chat@laptop`), and only there.

- **Q3. Finding peers is automatic** — D3.

## 7. Open, and deferred

- **The wake Q1 rules for.** Two cases, neither built: a live session idle
  at its prompt (the TUI, a web chat), which would start a turn when an
  accepted message arrives; and a machine with no session of that producer,
  where `mecha serve` would start one. Each needs its own design before
  step 2 ships the lead case of §1.
- **Deferred by the owner:** the web app switching between the agents of
  the machines that serve one — the same discovery, offered as a picker.
