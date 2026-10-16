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
every other route. The body carries `to`, `body`, `reply_to`, the
harness-stamped `from` and `from_session`, and the sender's own message id
for idempotency (D6) — and **no `taint`**: D5 adds both legs on arrival
whatever a sender says, so a body naming a taint is refused as malformed. The route writes it into the receiver's own mailbox store through
`MailboxStore::send` — the same caps, the same duplicate check, the same
recipient lock. `from` stays a producer name (`work::valid_producer` refuses
`@`). **Every sender-supplied field is validated at the route**, since on
the wire none is harness-stamped any more: `from` by `valid_producer`,
the message id by the same form, since the store writes it into a
filename, `from_session` by the session-id form (`serve::persona_chat::valid_session_id`,
promoted to one shared check rather than a second copy), `from_machine` by the same character
set as a producer; anything else refuses the message. `render_delivery`
interpolates `from_session` raw into the `<untrusted-content source="…">`
fence today — safe while the harness stamped it, a fence-breaker the moment a
peer can set it (review of #700). So the fence is hardened where it is
built as well as at the route: `render_delivery` escapes whatever it
interpolates into the `source="…"` attribute, since the store is read as
untrustworthy anyway (`effective_taint`'s own argument), and a message
written by any other path must not open the fence either. The machine goes in a new optional
`MailboxMessage::from_machine`,
`None` for a local message and a default for an older build's file, and
surfaces show the pair as `chat@laptop`. "The same owner guard" includes
the rest of a `POST`'s gate — the `x-mecha-request: 1` header every mutating
route requires, and `sec-fetch-site` refused when present and not
`same-origin` (a peer's client sends neither). The duplicate check's key —
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

1. keep the nodes owned by the same Tailscale user as this machine
   (`Self.UserID` in that document — not `[web] owner_login`, which a machine
   that only sends need not set), online and untagged;
2. ask each for its agents (D7) at its MagicDNS name, through `tailscale
   serve` — the door the web app already uses;
3. a node that answers is a peer under its MagicDNS short name (`spark` for
   `spark.tail1234.ts.net`). The *sender's* assurance that a peer is the
   owner's machine is step 1 — tailscaled's node list, filtered by the
   owner's user — reached over MagicDNS and the tailnet's TLS; D1's header
   check protects the *receiver*, and tells the sender nothing. A node not running
   `mecha serve` fronted by `tailscale serve` — the door, not merely the
   process, since `mecha serve` binds loopback — has no agents to offer and
   is no peer: nothing registers a
   machine, and nothing needs to. A node of the owner's that is offline, or
   answered within the last day and does not now, is listed as
   **unreachable** rather than dropped, so a peer that sleeps reads as one
   that is asleep (review of #700). "Within the last day" needs a memory the
   30-second cache is not: each peer's last answer is kept in `.peers.json`
   under the messages root (`$MECHA_MESSAGES_DIR`, `[messages] dir`, or
   `~/.mecha/messages`) — dotted, as `.agents` is, to stay out of the producer
   namespace — written when a peer answers, read and rewritten under the mailbox's own
advisory lock so two senders cannot drop each other's timestamp, through a
temp file named for its writer and a rename,
   since several sessions send at once, so it survives a restart. A file that
   cannot be read is a finding, never an empty map: its peers list as
   unreachable, not as gone (review of #700).

The peer list never enters the cached prefix: `message_send`'s description
names no machine, and the peers reach the model only in a tool *result* —
`message_agents`, or the error for an unknown machine — so a laptop going to
sleep never re-pays the prefix.

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
exclude = ["old-mbp"]          # a node of the owner's never sent to or listed
```

`exclude` is outbound only: no send to it, no listing of it. It cannot keep
messages *from* that node out, because the receiving side cannot tell which
of the owner's machines sent one (D1) — filtering on `from_machine` would be
the trust decision D1 forbids (review of #700).

`message_send`'s `to` accepts `chat` (this machine) or `chat@spark`; an
unknown or unreachable machine is an error that lists the peers found.

**D4. Egress: none, because the boundary moved — and the taint moves with
it.** A send to a peer is classified as a local send is: no `Egress` class,
so the interlock does not refuse it. **But the leak guard still sees it.**
`block_sends_after_private` is the control an operator turns on to stop
private data leaving the box, and its check keys on `can_send()`, which is
false for `Egress::None` — a send to a peer *does* leave the box, so the
remote path is refused by that guard explicitly after private data, as any
send is (review of #700). The interlock and the leak guard answer different
questions, and only the first is answered by the range argument below. The
model *does* name a destination
now — `to`'s machine half is its choice — so what holds the class at none is
not the absence of a choice but its range: **every value `to` can take is one
of the owner's own machines**, found from the tailnet's identity of the owner
(D3) and resolved only against that set — `[messages.peers] nicknames` too,
which may only name a node in it and is never an address book; the model
names a peer, never a host
or a URL. That is the local precedent widened by the ruling, and it is not a
licence for a fixed allowlist of anyone else's hosts — `http_fetch` is
`Chosen` because its range is the internet (review of #700). `MessageSendTool`
declares none of the four capabilities today because "the payload lands in
`~/.mecha`, owner-only, same uid — no exfiltration channel"; the ruling
extends the same reason to the owner's other machines. What keeps this safe is
the research's rule, tightened for the wire: no taint crosses it (D5) — the
*receiver* stamps every remote message with both legs and merges that into
the receiving conversation before the text lands. A run on the GB10 can still
report to the laptop; the laptop's session then treats every external send
as if it had read a hostile page and private data both, whatever the GB10's
run had read. Exfiltration *to* a peer moves data between two of the owner's
machines, which the ruling places inside; exfiltration *from* a peer is the
receiving run's interlock, which now knows what the message carried.

**D5. Fail closed on the wire, and no taint on it.** Locally, a message whose
`taint` is missing or unreadable is untrusted, as one written by an older
build is; across machines there is no taint field at all, and every message
arrives with both legs, as below. That needs a change below the route: `MailboxStore::send` stamps
`taint_recorded: true` on every message today, so a wire body with no taint
would land recorded *clean*. The route must store a missing taint as
`taint_recorded: false` — the flag the fail-closed rule keys on — through a
send that takes it (review of #700). **The wire carries no taint**: with
both legs added on arrival whatever the sender says, a `taint` field on the
wire would have no reader, so it is not sent and a body naming one is
refused as malformed. **A remote message arrives fully tainted**: the receiver takes the sender's taint and adds both `untrusted`
and `private` — what `effective_taint()` already gives an unrecorded one —
so a peer can widen a message's taint but never narrow either leg. Adding
`untrusted` alone was not enough: the interlock arms on both legs, and a
forged "nothing private" would have left `Chosen` egress open over a body
that may be another machine's secrets (review of #700). The
route accepts a `taint` field from anything that presents the owner's header —
including an unconfined `shell` in an injected run on the owner's own machine
— and the research keeps "trust asserted by a sender" deliberately absent;
this is that rule, kept across the wire (review of #700). `from_machine`
keeps a peer's message apart from the owner's own `mecha msg send` here.
**The cost, stated with the rule:** taint lives on the conversation, so the
first remote message a session reads arms the interlock there for good —
both legs at once — and every `Egress::Chosen` tool is refused for the rest
of that session. A laptop chat that reads the GB10's nightly brief
can still reply by message (no egress class) but no longer reach a
destination the model names, where the same brief sent one machine nearer
would have cost nothing. That is the price of not believing a peer's word,
and it is §7's open question whether it is the right one for §1's lead case.
A peer on another build may send fields this one does not know: unknown
fields are ignored, never an error — a closed set written to an append-only
store is a wire format. A peer's reply that cannot be parsed is an error to
the sender, never a delivered message.

**D6. No store-and-forward in phase 1.** A send to an unreachable peer fails
at once, to the sender, as a tool error the model can route around ("`spark`
did not answer: its `mecha serve` is not reachable"). **A send is
idempotent**: the sender names each message with an id of its own, sent on
the wire (D2), and the receiver keeps every id it has stored, per sending
machine, in a ledger beside the mailbox that outlives `prune_resolved` (a
week; a retry arrives within seconds), and refuses a second message with the
same id from the same machine, so a `POST` that timed out after the receiver stored it, and is sent
again, is not delivered twice — the existing duplicate key cannot see that,
since by then the first copy may already be claimed (review of #700). Queuing for a peer that
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

**D8. The inbound axis.** A delivered remote message arrives fully tainted
(D5), so its content is governed whatever the sender says. **`message_agents`'
listing from a peer is marked `.from_outside()`** (review of #700): it is
validated to producer names (D7), but those come from the peer's own
`.agents` markers, which a process on that machine can write, and D5 refuses
to believe a peer — a name is still text a peer chose. The cost: listing the
agents on other machines arms `untrusted` on the conversation that asks, as
reading any outside content does; listing this machine's own does not. A peer's error is
never shown as its text: the sending tool reports a typed reason of its own
— unreachable, refused, full, or malformed — and drops whatever the peer
wrote, so no network text enters the conversation unmarked (review of #700).

**D9. Approval: `read_only` stays, on a reason restated.** `MessageSendTool`
is `read_only()` — no approval, which unattended coordination needs — on the
comment's premise that "nothing leaves the machine". A remote send does
leave it, so the premise is restated in the source: the payload lands in an
owner-only store on one of the owner's own machines (D3), the same reach the
ruling extends, and the receiving side re-imposes every gate. The comment is
edited in step 2, not left asserting what is no longer true (review of
#700).

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
   widened by it; every remote message stored with both taint legs —
   `private` and `untrusted` — whatever it carried (D5); delivery labelled with the machine —
   `render_delivery` says "on this machine" today, and a remote message says
   which (§3) — through a second stem, `REMOTE_DELIVERY_STEM`, added to
   `agent::is_harness_voice`'s closed list beside `DELIVERY_STEM`. That list
   matches by `contains` on *contiguous* fixed text, so the stem holds no
   machine name: the header reads "from `chat@spark` — another mecha agent on
   another of the owner's machines, not the user", and the stem is the fixed
   run "another mecha agent on another of the owner's machines" (review of
   #700). A delivery matching neither would be mined as an owner correction,
   and keeping the local stem would have the harness say "on this machine" of
   a message that is not; and `GET /api/messages/agents`
   (D7), built from the live markers and the trigger store. Tested with an
   absent `Tailscale-User-Login` (refused), another login (refused) — a
   forged header carrying the owner's login is indistinguishable here, and
   the protection is `tailscale serve` stripping client-set headers, which no
   test in this repository can reach — a missing
   `x-mecha-request` (refused), the owner's well-formed `POST` (stored — the
   happy path, stored with both taint legs), a body naming a `taint`
   (refused as malformed), one message id sent twice (delivered once), two
   machines' identical bodies (both kept), a remote
   delivery still recognised as the harness's voice, and a listing carrying a
   non-producer name (dropped).
2. **Find and send.** Peers from `tailscale status --json` (D3), with
   `[messages.peers]` for nicknames and exclusions — a field on both
   `MessagesConfig` and `MessagesLayer`, which both deny unknown fields; `message_agents` and
   `mecha msg agents --all` list every machine's agents (D7);
   `message_send` and `mecha msg send` accept `producer@peer`; the send
   posts to the peer with no taint (D5) and fails at once when it does not
   answer. The `Egress` class of `message_send` is unchanged (none), so
   **the leak guard needs its own check on the remote path**: `leak_risk` is
   `leak_guard_armed && caps.can_send()`, false for `Egress::None`, so
   `block_sends_after_private` would never see a remote send. The route's
   sender refuses explicitly when that switch is on and the conversation
   holds private data (D4). Tested: a clean conversation's remote send
   arrives fully tainted (proving the sender's taint is not what counts); a
   tainted conversation can still send; with `block_sends_after_private` on
   and private data in the conversation, a remote send is refused; a
   listing from a peer arms `untrusted` on the asking conversation. Discovery is
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

- **`mecha forget` is per machine.** It purges this machine's mailbox; a
  message already delivered to a peer's mailbox is that machine's to forget,
  and `purge_mailbox` and its backstop assume a sent message's copy is local.
  Forgetting across peers — a `DELETE` on the route, or forgetting on each —
  is not designed.
- **Whether a remote message should cost a session its `Chosen` egress**
  (D5's price). The alternative to always-untrusted is a narrower mark —
  untrusted for what the message carries, not for the session — which the
  taint model does not have today.
- **The wake Q1 rules for.** Two cases, neither built: a live session idle
  at its prompt (the TUI, a web chat), which would start a turn when an
  accepted message arrives; and a machine with no session of that producer,
  where `mecha serve` would start one. Each needs its own design before
  step 2 ships the lead case of §1.
- **Deferred by the owner:** the web app switching between the agents of
  the machines that serve one — the same discovery, offered as a picker.
