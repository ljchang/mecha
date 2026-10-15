# Messages between machines on the owner's tailnet

**Status (2026-10-15): designed, not built; Q1 and Q2 ruled (§6).** The owner's ruling that starts
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
is refused. The machine a message came from is the tailnet's word too
(`tailscale whois` of the peer), recorded beside the sender's own claim, so a
message never names its origin on its own say-so.

**D2. Transport: the receiver's `mecha serve`, one owner-gated route.** `POST
/api/messages` on the peer's `mecha serve`, behind the same owner guard as
every other route. The body is a `MailboxMessage` without its id and status:
`to`, `body`, `reply_to`, and the harness-stamped `from`, `from_session` and
`taint`. The route writes it into the receiver's own mailbox store through
`MailboxStore::send` — the same caps, the same duplicate check, the same
recipient lock — with `from` qualified by machine (`chat@laptop`). From there
nothing is new: the receiver's runs claim it at a turn boundary exactly as a
local message. A machine that does not run `mecha serve` cannot receive; one
that runs it with `web` off refuses, as every route does.

**D3. Addressing: `producer@peer`, peers named by the owner.** `[messages]
peers` in the global config — never a project layer, as `[messages]` already
is — maps a short name to a peer's `mecha serve` URL:

```toml
[messages.peers]
spark = "https://spark.tail1234.ts.net"
mbp   = "https://mbp.tail1234.ts.net"
```

`message_send`'s `to` accepts `chat` (this machine) or `chat@spark`. The model
names a peer only from this closed set; an unknown peer is an error that lists
the configured names. Discovery is a helper, never trust: `mecha msg peers
--discover` lists the owner's tailnet nodes answering `mecha serve`, from
`tailscale status --json`, for the owner to copy into the table — a node
appearing on the tailnet is not a peer until the owner writes it down.

**D4. Egress: none, because the boundary moved — and the taint moves with
it.** A send to a configured peer is classified as a local send is: no
`Egress` class, so the interlock does not refuse it. `MessageSendTool`
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
is. A peer on another build may send fields this one does not know: unknown
fields are ignored, never an error — a closed set written to an append-only
store is a wire format. A peer's reply that cannot be parsed is an error to
the sender, never a delivered message.

**D6. No store-and-forward in phase 1.** A send to an unreachable peer fails
at once, to the sender, as a tool error the model can route around ("`spark`
did not answer: its `mecha serve` is not reachable"). Queuing for a peer that
comes back later is phase 3, if the cases above need it.

**D7. Finding the agents: one listing, here and on each peer (proposed).**
Today the model knows producers only by name — `chat`, and the triggers it
has been told — and `mecha msg agents` (live sessions, from their markers) is
the owner's alone. So that discovery works alike on one machine and many: an
owner-gated `GET /api/messages/agents` on each `mecha serve`, answering this
machine's producers (its triggers and `chat`) and which have a live session;
and a read-only `message_agents` tool listing them for this machine and each
configured peer. A peer that does not answer is listed as unreachable, never
as empty. Like `message_send`, it carries no egress class: it reads from the
owner's own machines.

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

1. **Receive.** `POST /api/messages` on `mecha serve`: the owner guard, the
   peer's machine from `tailscale whois`, `MailboxStore::send` with
   `from = <producer>@<machine>`. Tested with a forged header (refused),
   another login (refused), a missing taint (stored untrusted).
2. **Send.** `[messages.peers]`; `message_send` and `mecha msg send` accept
   `producer@peer`; the send stamps taint exactly as a local one and posts to
   the peer, failing at once when it does not answer. The `Egress` class of
   `message_send` is unchanged (none) — the test is that a tainted
   conversation can still send to a peer, and the receiving conversation
   comes out tainted.
3. **Later, if the cases need them:** `mecha msg peers --discover`;
   store-and-forward for an unreachable peer; a reply addressed back to
   `producer@peer` from the receiving run.

## 6. Rulings (the owner, 2026-10-15)

- **Q1. A remote message can wake an idle session.** It follows the
  receiver's own `[messages] inbound` policy exactly as a local message
  does: where a live session starts a turn for a local message, it starts
  one for a remote message too, and nothing remote-only holds it back. A
  machine with no session of that producer running keeps the message until
  one runs, as locally; `mecha serve` starting a run for it is not designed
  here, and is the next question if that case matters.
- **Q2. One mailbox per machine.** A message lands on the machine it is
  addressed to (`chat@laptop`), and only there.

## 7. Open

- **D7 is proposed, not ruled**: whether the model gets `message_agents`, or
  discovery stays the owner's (`mecha msg agents`, extended to peers).
- **Waking a machine with no session running** (Q1's last sentence).
