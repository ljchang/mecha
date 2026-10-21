# Serving to others: mecha personas as HyperStudy agents

*2026-10-19. Design, not built.* How a mecha persona takes part in a
HyperStudy experiment — reading what the study shows, talking by text and
voice, and acting in the study (a rating, the next screen) — for people who
are not the owner, without a public listener on this box and without anything of
the owner's in reach of the conversation. PERSONA-DESIGN §14 deferred
serving and listed what it would need; this is the design it asked for.

The wire contract is HyperStudy's, not this document's: its
`dev_docs/VOICE_AGENTS_DESIGN.md` defines the HyperStudy Realtime Profile
(HRP) — a pinned subset of OpenAI's Realtime API with a conformance suite —
and the `accept` transport mecha uses. This document decides only what mecha
builds against it, and points there rather than restating it, because two
copies of a protocol drift into disagreeing.

## 0. Rulings

| # | Ruling | By | Date |
|---|---|---|---|
| S1 | **mecha connects to HyperStudy directly.** No `hyperstudy-agent` in the path: it publishes a local server through a public tunnel, which is the inbound door this design exists to avoid, and an outbound mode added there would be one more process between two ends mecha already owns. | owner | 2026-10-19 |
| S2 | **Study policy is the experimenter's.** Disclosure, concealment, consent, age and institutional review belong to whoever runs the study. mecha builds infrastructure: it provides mechanisms (§7) and takes no position on how a study uses them. | owner | 2026-10-19 |
| S3 | **Personas only, for now; the owner's live mecha is never served.** A seat runs a persona from a sealed guest home (§3). A general assistant may come later, and when it does it is a sandboxed instance launched for the purpose — never the owner's running mecha, its stores or its main chat (§1). | owner | 2026-10-19 |
| S4 | **Engine-owned turn-taking for one human and one agent.** The engine decides when a turn ends and when it is interrupted (HRP engine mode); platform mode, for two or more humans, is a later phase on both sides. | owner, via the HyperStudy session | 2026-10-17 |
| S5 | **What a seat does:** reads text and images the study presents, chats by text and by voice, and takes the study's actions — answering on-screen questions and ratings. **No image generation** for now (§4a). | owner | 2026-10-19 |
| S6 | **A feature of its own,** off until the owner turns it on in setup (§6). | owner | 2026-10-19 |
| S7 | **mecha mints the token;** the owner pastes it into HyperStudy (§5.1). | owner | 2026-10-19 |
| S8 | **A seat is recorded in full.** The point of a local model is that what it did can be investigated afterwards: transcripts, directions and the voice journal are kept as an owner's call's are, not silenced (§3). | owner | 2026-10-19 |
| S9 | **`max_seats` caps concurrent seats,** so studies cannot slow every conversation on the box, the owner's included (§5). | owner | 2026-10-19 |
| S10 | **The system prompt is the study's.** A seat is prompted with what HyperStudy sends as its instructions and nothing of the owner's persona text — no `BASE`, no card, no `DISCLOSED`, no `SINCERE`; the persona a seat names contributes its voice (§3). | owner | 2026-10-19 |

## 1. Why no existing door fits

Every door mecha has assumes the speaker is the owner, and each says so in
code:

- **`mecha voice-serve`** binds `LISTEN_HOST` (`127.0.0.1`, pinned by a
  test) and treats the newest user message (`last_user_text`) as the owner
  talking. A spoken yes is matched by `review_policy::parse_answer` and can
  release a staged draft (`voice/confirm.rs`). A stranger's "yes" would be
  the owner's.
- **Persona memory** (`persona::writer`, `persona::recall`) records the human
  side of a persona chat as facts about the owner and recalls the owner's
  past into every turn. A stranger's sentences would become the owner's
  remembered life, and the owner's life would be recalled to the stranger.
- **`mecha serve`** is the owner's page: `owner_guard` admits only the
  tailnet login that is the owner.
- **The approver** now has a second door, `Approver::escalate_access`, which
  asks a person to grant a path (#716). There is no person on the far end of
  a study session entitled to answer it.
- **The persona prompt itself.** `persona::agent::BASE` presents the
  interlocutor as the character's author, with an assistant of their own
  that the chat is separated from. `DISCLOSED` (appended while the
  persona's `disclosure` switch is on, the default) relies on `mecha
  serve`'s AI tag having done the disclosing — a tag no participant sees.
  In a seat both premises are false, and the second steers the model away
  from a disclosure the study may want — which is why a seat uses none of
  it (S10).

**Why not the front door's shape.** The one door that already faces
strangers, `frontdoor.rs`, quarantines a request and hands the privileged
run a typed extraction, never the prose. That fits a request to be acted
on later; a seat's whole job is to converse in prose with the stranger in
real time, so an extraction would remove the conversation itself. A seat
answers the same threat the other way: it holds none of the owner's
stores, so there is nothing for the stranger's prose to reach (§3).

**Why the owner's main chat cannot be served even with every tool off
(S3).** The trifecta interlock guards tools whose destination the model
chooses. In a conversation with a stranger, the *reply itself* is a way out:
spoken to the person who asked, chosen by no tool, seen by no interlock. A
context that holds mail, calendar or the graph leaks by answering "what was
in the last email from…". So the line is drawn around the owner's *stores*,
not around capability — which is why a general assistant can be served
later, as a fresh sandboxed instance holding none of them, and why the
running mecha never can.

## 2. The shape

```
 HyperStudy backend                         this box (dials out only)
 ──────────────────                         ───────────────────────────────────────
 control socket  ◄──────── wss, held open ── mecha-hyperstudy  (transport crate)
   hs.session.offer ─────────────────────►     │ offer: origin pin, typed reject
   hs.session.reject ◄──────────────────────   ▼
                                            mecha-cli/src/hyperstudy/  (wiring)
                                               │ admit? build guest home, start guest facade
                                               ▼
 session socket  ◄──────── wss, per seat ─── mecha-hyperstudy: frames validated here
   (single-use token, 60 s)                    │ text seat: the wiring itself
   text · images · PCM16 24 kHz                │ voice seat: loopback relay to the worker's
                                               │ HRP transport · Silero · SmartTurn · Parakeet · Breeze
                                               ▼
 LiveKit room ◄─► HyperStudy's voice bridge  guest facade (own process, own home, loopback)
                                               the persona's loop, untrusted interlocutor
```

Four pieces, each with one job:

1. **`mecha-hyperstudy`** — a transport crate, the shape `mecha-slack`
   already has: it dials out (Slack's Socket Mode there, HyperStudy's control
   socket here), so no port on this machine is ever exposed, and it has **no
   `mecha-core` dependency**, checkable in `Cargo.toml`. It holds the control
   socket, reconnects with backoff, parses offers, refuses an offer whose
   `sessionUrl` is not on the control socket's origin (HyperStudy §4.1), and
   sends typed rejections. It also dials and holds every **session** socket,
   validating each frame against HRP's schema before anything else sees it,
   so the only code on this box that parses bytes from the internet is this
   crate. It knows nothing about personas or runs.
2. **`mecha-cli/src/hyperstudy/`** — the wiring that does know: admission
   (§5), building the guest home (§3), starting the guest facade, and the
   seat itself — a text seat speaks HRP with text-only output directly, a
   voice seat is handed to the worker.
3. **The worker's HRP transport** — a new transport in
   `mecha-core/units/voice/worker.py`, beside `SmallWebRTCTransport` and
   `UplinkTransport`. Below `run_bot` the pipeline is transport-agnostic;
   above it, both existing transports are WebRTC (`UplinkTransport`
   subclasses `SmallWebRTCTransport`) and are built at one site inside the
   WebRTC runner's entry, so this is the first non-WebRTC transport: pipecat's
   transport surface implemented from scratch, plus its own entrypoint and
   route beside the offer endpoint. It plays the
   *engine's* side of the Realtime protocol over a loopback relay from the
   wiring, never over the internet: the owner's worker is unconfined, holds
   the owner's environment, and so does
   not terminate a stranger's socket or parse its frames — the crate does
   (piece 1). The cost is one loopback hop for audio. Who opens a socket and
   who serves the protocol are separate choices; that separation is what
   lets an engine sit behind NAT with no tunnel.
4. **The guest facade** — the persona's agent loop, served from the guest
   home on loopback to the seat (§3, §4, §4a).

The judgments — is the origin right, may this persona be served, is the box
free — are made in Rust before a seat opens. The worker stays what its
module doc says it is: "deliberately dumb".

## 3. The sealed guest home

The isolated-home pattern of PERSONA-DESIGN §14's third option, kept on
this box rather than a separate one, and built with `trial_env`'s: an
isolated home per seat, assembled from an explicit list, never from the
operator's home.

**The seat directory** (`hyperstudy/seats/<seat>/`) holds four things side
by side, none inside another: `home/` (the guest home, `MECHA_HOME`),
`workspace/` (the jail, which contains no part of `home/` — `setup`'s rule
that a workspace never contains the mecha home), `run/` (the sockets, under
*The agent loop is its own process* below) and the manifest.

**It holds:** the named persona's voice settings (the voice name the
owner's worker is handed, its speed and instructions), copied in,
read-only, and not its reference audio: a voice is a name on the wire,
the owner's worker resolves it through the speech server, nothing in a
seat reads the file, and a seat directory is kept after close (S8). And
nothing of its text: who the agent is
comes from the study (S10). Its config is written fresh by the wiring, not
derived: `[security]` is taken from the owner's field by field — the
owner's value where it narrows a seat, the strictest value where it would
widen one (§4a). "Strictest" is defined only for the three fields that
bear on a seat: `trifecta` (pinned `block`), `mark_untrusted_output`
(always on) and `block_sends_after_private` (the owner's, §4a — the named
departure). Of the other three, which guard the fetch and search tools a
seat does not have: `block_private_ips` is set `true`, its strictest value,
by the rule and moot under `--unshare-net`; the two domain lists have no
strictest value, since an empty list is the *widest*, so they are copied
as the owner's and nothing is inferred from them; `[sandbox]` is not the owner's
at all but written by the wiring as a seat property, as `trifecta` is
enforced (below, after the launch);
`[providers]` names only the loopback router, with no key, and copies
the three fields the seat depends on from the owner's router entry:
`context_window` (it must equal the router's `-c / -np`, as everywhere —
without it compaction never fires and the output budget falls back to a
ceiling too large for the slot), `vision` (without it every study image is
replaced by a text placeholder while still arming `private`),
`structured_output` (it defaults to `Disabled`, and the crisis judge sends
its `response_schema` only when it is on, so without it §7's forced-on
judge runs without its schema; IMAGE-DESIGN §9's 2026-10-08 ruling sets
`llama_json` on every router-served provider). It pins a fourth,
`follow_loaded = false`, as a seat property rather than inherited, as
`trial_env` sets it for every arm — following whatever the owner last
loaded would change a seat's model mid-conversation, the other half of §5's
hold. The agent's model comes from the owner's; its `timezone` is written
as `UTC`, never copied, because the calendar head note renders in the
configured zone (`Agent::calendar_note`) and a participant reading the
owner's zone learns where the owner is — and a participant may be
anywhere, so no zone is theirs either; a study that wants local time says
so in its own instructions. `max_tokens` is copied from the owner's, as
`context_window` is, since both are facts of the one router: it must sit
above the preset's `--reasoning-budget`, or the reply is HTTP 200 with
empty `content`, which in a seat reads as a model-server fault rather
than the misconfiguration it is. `max_turns` is pinned one above §4a's
tool-rounds-per-turn budget, so that budget's `is_error` always answers
first and `max_turns` is only the backstop that ends a runaway run; two
bounds on one quantity with opposite failure modes must not race. Every
other `[agent]` field is its default on purpose. No other
table is written —
no `[[mcp]]`, `[[hook]]`, `[rule]`, `[approval]` or `[search]`. That is
deliberately not `trial_env`'s shape: a trial home inherits the operator's
`[providers]` and keys along with `[security]`, and a seat inheriting them
would hand a stranger's conversation a cloud key and a non-local model with
the seal intact around it.

**It never holds:** the owner's persona memory or chat history with that
persona; mail, calendar, graph, outbox, learning store or triggers; the
owner's `[[hook]]`s, skills, charter or subagents — the list `trial_env` was
rewritten to exclude after a derived home's inherited `session_end` hook put
synthetic sessions into the owner's graph (2026-09-23), and hooks run shell
where a stranger would be firing them; skills and the charter are prompt
content, which S10 rules out; the
owner's `[[mcp]]` servers — including those marked `personas = true`, which
were marked for the owner's chats, not for strangers; provider keys beyond
the loopback model endpoints. What keeps those out of the *directory* is
the explicit source list; what keeps them out of the *process* is the
launch, and the launch's confinement has a stated source and a floor.
`SandboxConfig::default()` is `Backend::None`, under which
`wrap_argv_with_env` returns the bare command — host network, no namespace,
no cleared environment — and the trial-home precedent inherits the
operator's `[sandbox]` among its machine tables. A seat does neither: its
backend is bwrap, pinned by the wiring whatever the owner's `[sandbox]`
says, and a box where that backend cannot run opens no seat. The rule
about a configured sandbox that fails covers the broken case; this one
covers the never-configured one, which would otherwise make "egress no
longer rests on the tool catalogue alone" silently false on a default
install. So: the guest facade is spawned with its environment cleared first and
an explicit allowlist added back, in `Sandbox::child_env`'s shape — the
`mcp.rs` `env_clear()` rule, since `Command::envs()` alone layers on an
inherited environment and hands the child the owner's provider keys. The
load path is decided beside it: the facade loads with `global_config_only`
set, as `voice-serve` sets it a few lines from the `system_extra` this
design inherits, so a project config in whatever directory it starts in
never layers in. Today the start directory would be an empty `workspace/`,
so nothing would load either way, but a layered config is a load-path
decision and is made here rather than left to where the process happens to
start. That
holds whether or not the bwrap question in §9 is settled either way. The guard on the home's *location* is the trial home's,
`experiment::refuse_unsafe_home` — it refuses a home that is or contains
the real one and allows one inside it, which is where §3 keeps seats — not
`trial_env::refuse_operator_home`, which guards an environment's source
data and would refuse the seat directory itself.

**The agent loop is its own process; the audio leg is shared.** The guest
facade runs as a child with `MECHA_HOME` set to the guest home, bound to
loopback on a port chosen per seat, never the owner's `voice-serve`; its
workspace jail is the seat's `workspace/`, a sibling of its home, so the
guarantee is structural rather than resting on a seat having no file tool
(§4a). A voice
seat's audio runs through the owner's running voice worker — the one worker
on the box, its journal the owner's (S8) — which is part of why a live owner
call makes a voice seat `busy` (§5): one worker's state, not only GPU room.
The worker sees a seat only through the wiring's loopback relay of frames
the crate has already validated (§2); it never holds a socket to
HyperStudy.

**Confinement: bwrap with no network, and Unix sockets as the only way
out.** Nothing confines a whole mecha process today — `shell`, MCP servers
and two document helpers are wrapped, and experiment trials, the closest
precedent, get a cleared environment and their own home but no jail. The
guest facade is the first, through `Sandbox::wrap_argv_with_env` (built for
long-lived MCP children). The seat directory is the bind root, because
bwrap's bound directory must include `run/` and its sockets. The path jail
is narrower: `ToolCtx::resolve`'s root is the `workspace/` inside it.
`MECHA_HOME` goes in the environment, since bwrap clears it.

bwrap's network is all or nothing — with the host's network the jail
reaches the LAN and the tailnet, and with `--unshare-net` it reaches not
even the loopback router. The way through is that a *pathname* Unix socket
crosses a network namespace. The wiring, outside the jail, creates
`seat/run/model.sock` (forwarding to the router on `127.0.0.1:8080`) and
`seat/run/parent.sock` (to itself) inside the bound seat directory; the
facade runs under `--unshare-net` and reaches the model and its parent
through those and nothing else. The facade's clients either take a socket
path (reqwest's `ClientBuilder::unix_socket`, present in the locked
0.12.28 under `cfg(unix)` with no feature flag — checked in the crate's
registry source) or keep
their `http://127.0.0.1` URLs through a forwarder on the jail's own `lo`.
The first is cheap on the worker side and not on the facade's: the
provider's HTTP clients are built centrally with no socket parameter
(`provider/mod.rs`), and `base_url` is URL-typed through
`provider::router`, so a socket would mean a new provider field (two
edits, `Config` and `ConfigLayer`) and a placeholder `base_url` such as
`http://seat/v1`. The forwarder needs no core change, since the entry keeps
`http://127.0.0.1:8080`, but it is not free: it runs inside the
`--unshare-net` namespace, and bwrap execs one child, so it is a second
in-jail process with its own lifecycle and its own preflight leg, failing
the seat closed if it is not up. Which one is built is decided on cost measured
in step 2. Either way `follow_loaded = false` leaves
`router::followed_bases` empty, so a confined facade sends no `/props`
probe into a namespace that cannot answer it. The result is a seat with no route to any
host, so egress no longer rests on the tool catalogue alone (§4a).

**The bridge runs both ways, and every seat needs the other way.** Those
two sockets are the facade's way *out*; every seat also needs a way *in*,
because the facade is served, not a client. In a text seat its client is
the wiring, which speaks HRP itself (§2) and drives the facade over HTTP —
reqwest's `unix_socket` again, the same check step 2 makes first. In a
voice seat the worker is a second client, unconfined and the owner's:
completions go to `FACADE_URL` (`MECHA_VOICE_LLM`), and the affect and
direction reads to `AFFECT_URL` and `DIRECT_URL`, derived from it at
import. A facade listening on loopback inside `--unshare-net` listens on
the jail's own `lo`, which the host cannot reach, so without a third leg no
seat's turn has a route — a text seat's no more than a voice seat's reply,
affect read or direction call. The third leg is the same pathname trick
reversed: the facade listens on `seat/run/facade.sock`, created inside the
bound seat directory by the facade itself, and the wiring (and in a voice
seat the worker) connects to it from outside. It lands with the confined
launch in step 2, not with voice in step 4. The worker's side is real work, not a URL change. Its `httpx` (0.28.1 in the worker's
environment) takes a socket through `AsyncHTTPTransport(uds=…)`, which
covers the affect and direction clients. Completions go through Pipecat's
`OpenAILLMService`, whose `create_client` (pipecat-ai 1.7.0,
`BaseOpenAILLMService`) builds its own `DefaultAsyncHttpxClient` and
accepts no client from outside — so a seat uses a subclass that overrides
`create_client` to hand the OpenAI client one on that transport. The
request URL keeps a placeholder host (`http://seat/v1`) that names the
request and routes nothing. `parent.sock`'s closed message set is not
widened to carry any of this: it is the facade's channel to the wiring, and
completions are the worker's channel to the facade, a different direction
with a different peer.

**`parent.sock` is pinned like the catalogue.** It is the one channel from
the process holding a stranger's conversation into the process holding the
owner's home, the router hold and the crisis ledger — and §7 needs it, since
the facade's bind root is its seat directory, so the owner's ledger is not
in its filesystem to write. That holds in §9's filesystem-only interim
too, which drops the network namespace and keeps the jail, so the crisis
record's message on `parent.sock` is needed either way. So
the wiring accepts a closed set of messages on it, checking a ClientTool
call's name and arguments against the catalogue again on arrival, not only
at registration: a ClientTool call (and
returns its result), a crisis trip record, a `CrisisState` change, and seat
close. Anything else is refused and logged, failing the seat closed, never
ignored.

The alternatives, weighed: Landlock (already a backend) can allow TCP
connect by port, but UDP is restrictable at no ABI — which is why
`can_reach_network()` is unconditionally true for it in `sandbox.rs` — so a
landlocked seat remains a route out whatever its TCP rules; and TCP rules
go only by port — `8080` would also allow
`<any LAN host>:8080` and the tailnet's — and it leaves `/proc` readable,
the owner's processes' environments included. Docker's `--network none`
needs the same socket bridge plus an image and a daemon. Filesystem-only
bwrap with the host network is the cheaper first step if the bridge is not
ready; whether step 2 ships the bridge or that interim is owed (§9), and
the preflight of every leg belongs to the bridge. The interim needs no
`facade.sock`: with the host network the facade's loopback is the host's,
so the worker's URLs reach it unchanged — the third leg is a cost of the
bridge, not of the interim.

Three things the build must not miss: the `mecha` binary and any model or
cache file the facade reads live under the owner's home, which bwrap does
not bind, so they are named read-only; `bwrap` is resolved through `PATH`,
and this box's `/usr/bin/bwrap` cannot create user namespaces under its
AppArmor setting (the local `/usr/local/bin` build can), so the launch names
the binary it preflighted; and the preflight proves every leg — the model
answers through `model.sock`, the wiring answers through `parent.sock`, a
voice seat's facade answers the worker through `facade.sock`, and a LAN
address does not answer at all. A confinement that is configured and fails stops the seat, never
falls through (the silently-degrading guard).

**The interlocutor is untrusted — a run mode, not a prompt:**

| Surface | Owner's persona chat | Guest seat |
|---|---|---|
| Who the human side is | the owner | a stranger. Inbound text never arms `untrusted` today, on the premise that the user chose every word (`Taint::arm_for_content`'s comment; it can arm `private` off its stems, §4a), and `external` is a tool result's field, not a message's — so a seat sets `Conversation.taint.untrusted` at construction, and the conversation records it, so the seat is armed from the first word |
| System prompt | `BASE`, the card, plus `DISCLOSED` while `disclosure` is on | HyperStudy's `instructions`, verbatim, and nothing of mecha's persona text (S10), `SINCERE` among it — the persona prompt's one unconditional line (a sincere "am I talking to a person?" gets the truth) is left out because whether an agent answers that truthfully is a disclosure decision, the study's by S2; a study that wants the floor writes it into its own prompt |
| Persona memory | written and recalled | neither: no writer, no recall over owner stores |
| Confirmations, outbox | spoken yes can release | none exist: no outbox route, no `parse_answer` path |
| Approver | the owner's | allows **exactly the ClientTool catalogue** at `approve` — the loop sends every tool that is not `read_only` there, and the answer tools and `send_chat` genuinely act — and refuses every other tool there, and every other door that does not delegate into it (`escalate`, `escalate_access` through it, and `consult` by its own fail-closed default). `permit` delegates to `approve`, so it can allow nothing `approve` would not — and a seat writes no `[approval]` table for it to fire on. Catalogue membership, checked in mecha's code, is what makes a tool runnable, the same place §4a earns `Blind`; no catalogue tool claims `read_only`. What holds a future door is the trait's defaults, which fail closed (`escalate`, `consult` → `Blocked`) or delegate (`escalate_access` → `escalate`, `permit` → `approve`); so the guest approver overrides `approve` and `escalate` and nothing that delegates. Every refusal it returns is `Decision::Blocked`, never `Deny`: `Deny` renders as the owner's correction ("Denied by the user: "), which the learning miner reads from the result string and compaction preserves across a cut, and no human speaks in a seat. A test listing today's doors cannot catch a new one with a default body — exhaustiveness the compiler checks would need a door enum on `Approver`, named in §8 step 2 as a choice, not assumed |
| Grants | `[[grant]]` and `/grant` | off by type, as an incognito chat's are |
| Tools | the persona's eligible set | HyperStudy's declared tools only, as ClientTools — mecha registry tools whose call goes out over the session and whose result HyperStudy sends back (§4a); none of mecha's own |
| Transcript | the chat's session file | kept in full (S8): the seat's session file, with every record a run writes — tool calls, ClientTool results, context items, spoken directions — and the seat's guest home is left in place when it closes |
| Voice worker logging | an ordinary call's journal | the same: a seat is an ordinary call to the worker, never incognito |

**Where a seat is kept.** The guest home is built under the owner's home at
`hyperstudy/seats/<seat>/` and left there when the seat closes — the
experiment store's pattern, a trial's home kept as its record. The seat's
manifest names the study's session, the persona whose voice it used, the
model `served_props` reported, and the HRP settings it ran under, so a seat
can be read back the way a trial is. Seat transcripts stay out of the
owner's run corpus and reflection mining by a recorded fact, not only by
where they live: a seat's session carries its own `SessionKind`, and the
corpus `Scan` excludes it as it excludes `Test` and `Experiment`. The
variant alone is not enough, and the reason is across binary versions: a
reader built before the variant exists loads the unknown kind as `None`
(`de_lenient_kind`), and `Scan::admits` admits `None` like every kind it
does not name — so the variant guards readers built after it, and the
location guards the ones built before — which is why the location stays
the guard every reader honours (PERSONA-DESIGN §3.2), and the kind is the
second check. Their location (outside `$MECHA_HOME/sessions`, which the corpus
reads without recursing) is the one every binary honours, old and new,
and the kind is what still holds against a recursive reader added later,
so neither guard is alone.
The repo's own guard must see them too: `scripts/check-private.py` refuses
a commit that quotes a conversation, but it reads explicit, non-recursive
globs (`personas/*/sessions`, `sessions`), so a seat's
`hyperstudy/seats/*/home/sessions/*.jsonl` would be invisible to it — and
participants' words, held under someone else's consent and kept in full,
are the case the guard exists for. Step 2 adds the seat glob to the script's `whole` list, the one it reads
entire. Its `sessions/` list reads a file whole only when its header's
`kind` is `voice` and otherwise only for spoken turns, and a seat's new
`SessionKind` matches neither, so a text seat's transcript joined there
would contribute nothing while the guard looked applied. The glob follows
the depth a seat's sessions actually land at, and anything else the seat
home writes conversation text into joins it. Retention of these records is the
owner's (S2 hands participants' consent to the study); nothing here deletes
them on a clock. mecha declares to HyperStudy that the engine keeps
transcripts (`hs.retention`), so a study can tell its participants.

**The study's prompt is the whole prompt (S10).** HyperStudy builds the
agent's identity, objective and per-state guidance into `instructions`, and
sends a `session.update` replacing them when the study's state changes.
mecha uses them as the system prompt verbatim, which makes `instructions`
a trusted slot on a socket that also carries participant text (the
study's crisis resources are the other, §7): the
contract with HyperStudy is that `instructions` carry only
experimenter-authored text, and anything a participant wrote — a
free-text answer, typed chat — comes back as a quoted context item, never
interpolated into the next state's guidance (§8). Said plainly: this is
the one guarantee in the design that a remote party's discipline keeps,
not mecha's code — a study that breaks it puts a participant's words in
the system prompt silently. The blast radius is bounded by everything
mecha does enforce (no owner stores, no mecha tools, a crisis sensor no
prompt can turn off), but the contract is HyperStudy's to keep. This is HyperStudy §4.4's
`persona: "hyperstudy"`, the ownership it builds today. Each replacement
re-pays the cached prefix once, which is the study's cost to choose.

**`instructions` is bounded, and an oversized one is refused.** The system
prompt is the one part of a request compaction cannot cut: the loop's only
overflow recovery is summarising `messages`, and `ContextOverflow` never
retries. So a `session.update` whose `instructions` would not fit leaves a
permanently dead seat whose cause reads as a model-server fault. The seat
refuses it at the door with a typed rejection on that `session.update`,
never truncating and never accepting it, under §4's "refused, not ignored"
rule. The bound is a share of the seat's `context_window` and is counted in
bytes: a byte-level tokenizer spends at least one byte per token, so bytes
bound tokens from above with no tokenizer call, and the bound errs toward
refusing. The share is set with the budgets, from a measured seat (§9).
The crisis-resources slot already has its own bound and fallback (§7);
this is the other trusted slot getting the same treatment. The
only harness text a seat adds is what the loop needs mechanically (tool
results, the context items of §4a, and the clock), never a claim about who
the agent is or who it is talking with. The clock needs saying: the
calendar note renders only when the system prompt carries
`date_context::GUIDANCE_HEADING`, so a verbatim study prompt would earn no
date on any request — the "Sunday on a Monday" failure by construction.
The note's vehicle is right and stays: it is attached at wire time, once
per request, never persisted, so it cannot go stale across midnight or
accumulate in history — which a context item folded into the conversation
would do. What changes is the gate: a seat-mode allowance beside the
heading test, so a seat's requests carry the head note, with a one-line
label saying what it is, rendered in the seat's `UTC` (above) rather than
the owner's zone, while the system prompt stays the study's byte for byte.

## 4. The voice transport

What the HRP transport must do, by protocol area (the events themselves are
HyperStudy's to define):

- **Session.** Accept `session.update`; map `turn_detection` (below);
  `instructions` to the system prompt (§3); `tools` to ClientTools;
  `audio.output.voice` accepted only as null (`preset: null`, since the
  named persona supplies the voice) and refused otherwise; `instructions`
  over the seat's bound (§3) refused. A field mecha
  does not support is **refused, not ignored** —
  HyperStudy's conformance asks for exactly that.
- **Silent forced answers.** HyperStudy asks an on-screen question with a
  `response.create` that is text-only and forces one named tool
  (`tool_choice: {type: "function", name}`); its suite's `silent_tool`
  check fails an engine that speaks or skips the call. So that response
  produces no audio, and the forcing must not change the tool list — the
  cached prefix (§4a) — whether through llama-server's `tool_choice` for a
  named function, checked on the installed build rather than assumed, or
  `required` with any other name refused.
- **Audio in.** `input_audio_buffer.append` (PCM16, 24 kHz, mono) into the
  existing Silero VAD → SmartTurn v3 → Parakeet chain.
- **Turns, engine mode (S4).** Two modes, both certified by HyperStudy's
  suite (version 2 runs every turn-dependent check once per mode):
  - `server_vad` — `speech_started` on the VAD edge, the reply in flight
    self-cancelled when `interrupt_response` is set, `speech_stopped` at the
    VAD stop, `committed` at end of turn. `threshold` maps to Silero
    confidence, `silence_duration_ms` to the VAD stop; `prefix_padding_ms`
    is approximate.
  - `semantic_vad` — SmartTurn's hold; `eagerness` maps approximately.
  This differs from the owner's calls on purpose: there, a turn starts on a
  transcript (`TranscriptionUserTurnStartStrategy(use_interim=False)`); here
  the protocol pins the VAD edge.
- **Audio out.** Breeze, streaming, resampled to 24 kHz as
  `output_audio.delta`; the director runs as in any call, and its
  directions are recorded in the seat's transcript (S8).
- **Truncate.** `conversation.item.truncate` cuts the reply's text at the
  heard point proportionally to audio played: grade `approximate`.

## 4a. What a seat can do (S5)

A seat is mecha's agent loop with no tools of its own: everything it reads
and everything it does goes through the session HyperStudy opened, so the
only way out of a seat is the session it answers. The sealed home keeps the
*owner's* data out; it does not keep the private leg from arming. An image
arms `private` wherever it sits in a conversation
(`Taint::arm_for_content`), and with the stranger arming `untrusted` from
the first word, a seat is fully armed from the moment its first study
*image* lands. A text-only study can arm `private` too, since
`arm_for_content` arms it off its stems in user-role text, which a
participant can type; nothing here assumes a text seat stays unarmed, and
it costs nothing while every catalogue tool is `Blind`. The image case needs saying at the fold (below), because an image the loop
did not produce is otherwise counted only from the next run. That is
correct — the interlock cannot know a study image from the owner's — and it
makes what a ClientTool declares the question that decides whether a seat
can act at all.

| Capability | How | Needs from HyperStudy |
|---|---|---|
| Talk by text | a text seat: HRP with text-only output, no worker; typed chat in a voice seat arrives the same way | text seats on the accepted socket |
| Talk by voice | §4 | Phase 3 `accept` |
| Read the study's text | screen text, instructions and typed chat arrive as quoted context items, untrusted like speech | — (sent today as `[context]` items) |
| Read images | a study image or one a participant shares arrives inline as an image content item and reaches the model as `Block::Image`, capped at the door as every image is; mecha never fetches a URL a session names | images on the session, inline |
| Act in the study | **ClientTools**: HyperStudy declares its study tools — `send_chat`, and the answer tools for on-screen questions (`answer_choice`, `answer_choices`, `answer_scale`, `answer_ranking`, `answer_text`, `answer_dimensions`); advancing the screen is not an agent capability in HyperStudy yet — the call goes out over the session, `function_call_output` comes back marked `.from_outside()`, a timeout returns `is_error` — so the agent loop never learns where a tool came from. A timeout is the case §4a's restore cannot see: the run did not fail, and the answer may be in the study's data with only its acknowledgement lost. So a timed-out answer tool's `is_error` says the answer may already be recorded and must not be sent again, and the proxy answers any later answer to that same on-screen question "Not run" until the next screen-change context item, in that run and every later one. It is keyed on the question's identifier, which every answer tool's pinned schema carries (§8), the same key HyperStudy dedupes on, and never on the tool: the answer tools are generic by kind, so a guard keyed on `answer_scale` would leave a second scale question on that screen unanswerable with no unlock coming: the participant's next turn is a new run, and the likeliest moment to answer again. So the guard is conversation-scoped state held by the seat's study tools themselves, not run-scoped like `RunPictures` (which shares only its "Not run" shape). It survives a cut by construction, since compaction rewrites the messages and never the registry; `Tool::carried_state` only restates the answered questions in the rebuilt prompt so the model does not retry blind, and step 2's test asserts the refusal itself after a compaction, with the carried block as the secondary check; the tool list never changes, so the cached prefix holds. A timed-out `send_chat` says its line may have been shown. A second answer is a fresh call with a fresh `call_id`, so HyperStudy's backstop (§8) is keyed on the screen's question, not the call | the study tools, declared per session |

**Where context lands.** Screen changes, typed chat and images arrive on
HyperStudy's clock, including while a ClientTool call is in flight — and
there is no legal slot between a `tool_use` and its result, nor for two user
messages in a row (images are user turns only). So context items queue and
fold at the next legal boundary through the steering path
(`ARCHITECTURE.md` §Interruption and steering) mid-round, into the message
carrying the tool results. The steering path alone is not enough: its one
drain site skips a closing turn by design — a steer taken there could not
be acted on — and leaves what becomes of an unread one to the host. Since
#731 both chat hosts give each a turn of its own, in the order typed
(`chat::next_turn`, `PersonaChats::next_turn`). A seat's host decides too,
by folding at a boundary of its own, and that boundary is in the facade
child, not the wiring. The conversation
lives there, in the seat's `SessionHost`, as voice's facade keeps it
server-side today (`Slot` holds `convo`, and the handler calls `run_in`
and `roll_back_failed_turn` in-process). The wiring only sends items to
it. It is a **deferred** fold: `run_in` holds `&mut Conversation` for the
whole run, so even the host cannot touch the messages while a run is in
flight, and a message already sent must not change — the transcript would
record words the model never received, and the next request would stop
extending the last one's prefix. Four cases:

- **A run is in flight** (a closing turn, or a reply with no tool call):
  the item waits, and cannot be acted on in that turn — the consequence the
  steering drain already accepts. When `run_in` returns it is handled as
  below.
- **The conversation is empty, or ends on the agent's reply:** the item
  (for an empty one, the opening screen's text, the first event in every
  seat) opens a new user
  message of context alone. This case and the next need a facade path that
  *parks* an item in the conversation without starting a run. Today every
  facade POST runs a turn, and a body with no user message is a 400. So
  the seat's facade gains a context endpoint (append, no run), and §8
  step 2 builds it.
- **It ends on an unsent user message** (context with no reply yet, or a
  participant turn not yet sent; unsent means the host parked it since the
  last run returned, a mark the host keeps, since `Conversation` does not
  record what was sent): the item is appended to it, and a
  participant turn arriving on a context-only tail is appended to that
  message too, so two user messages in a row never occur.
- **The run failed, or stopped on a ceiling:** `run_in` does not roll back
  on `Err`, so the tail is a tool-result message or a user message already
  sent — neither case above. A ceiling (`max_turns`, a budget, loop
  detection) is the same tail with `Ok`: it folds `FINAL_ANSWER_NUDGE` into
  the tool-result message and sends it, and if that request errors or is
  interrupted the run returns with no reply after it. §3 pins `max_turns`
  so that this is a runaway seat turn's designed exit, so it is expected,
  and the host tells it from the case above by its mark, not by the tail's
  shape. A cancelled run (a barge-in through `interrupt_response`) also
  returns `Ok`, with one of three tails: a partial reply the loop kept,
  which is the second case; a completed tool round, which is this one; or
  the participant turn just sent with nothing after it, also this one.
  It never takes the voice host's pop: `voice/mod.rs` pops a plain user
  tail because its front end resends one, which a seat's never does, and
  popping a tool-result message would orphan its `tool_use`, the 400 that
  pop once wrote into a slot and every resume of it. §8 step 3 tests a
  barge-in mid-tool-round. The seat restores to the **last complete round**, not to the
  snapshot taken before the run. A round whose ClientTool calls already ran
  has had effects outside mecha: a rating written into the study's data, a
  chat line shown to the participant, each acknowledged by
  `function_call_output` before the failure. Restoring to before the run
  (the first half of `Conversation::roll_back_failed_turn`) would erase
  the record of them, and the next request would ask again: two answers
  for one screen. That is CLAUDE.md's "a retry must never duplicate work"
  one layer up from the provider. So the restore keeps every message up to
  and including the last tool-result message whose results all exist,
  which is legal because every `tool_use` there has its result, and drops
  only what followed it, the partial reply. When no ClientTool ran in the
  failed run, that point *is* the pre-run snapshot. Either way the
  helper's second half (popping a plain user tail because a front end will
  resend it) is never taken. A seat has no front end: HyperStudy sent that
  context and that participant turn once, so popping them would lose them
  with nothing to record their loss. After the restore the tail is either
  an unsent user message (the third case) or a kept tool-result message. A
  later item or participant turn is appended to that message as the
  steering path appends to one, so the next request extends the kept round
  and the model sees what it has already done. Changing a message the
  failed request carried costs that one request's prefix tail, and the
  rewrite records it. Nothing will resend it. The restore is a new helper
  beside `roll_back_failed_turn`, named in step 2. The rolled-back state is
  recorded as a rewrite in the seat's transcript (the helper's contract,
  and the transcript is kept in full, S8), followed by a `Record::Taint`
  with no message between, as every failed-turn arm writes today:
  `TaintTimeline::from_records` clears its checkpoints on a `Rewrite`, so
  without it the record would lose that the seat was armed `untrusted`
  from construction and `private` from its first study image. Whether any of
it asks for a reply is the study's, by its own `response.create`. Nothing is dropped: an item still queued when the
seat closes is recorded as unfolded in the seat's transcript. The
mid-round half is not free either: the steering queue carries text
(`queued_input` is a `VecDeque<String>`), and an image context item needs it
to carry `Block`s. That widening is a core change that touches every
steering caller, the TUI and web chat included, and §8 step 3 names it.

**A folded image arms at the fold, not at the next run.** Taint is
recomputed at run start (`Agent::arm_at_start`, over the messages and the
notes), and the loop's one mid-run arming site is the tool-result image
branch, which exists for exactly this reason: so the pixels "are counted
from the turn they arrive — the same answer, a run sooner". A context
image folded mid-round through the steering path passes neither. Left as
it is, a send later in that same run is judged on the taint from before
the screen arrived, which matters when the owner has
`block_sends_after_private` on and in what the session records. So the
drain that folds steered `Block`s arms them in the same place, through
`arm_for_content` over the folded blocks, beside the tool-result branch.
The host's deferred fold needs nothing new: its folds land between runs,
and the next run's `arm_at_start` counts them before anything is sent.

**Compaction.** A long voice seat reaches the slot's `context_window`, and
compaction applies as it does anywhere: the cut lands on an assistant
message, taint survives it, and the session file records the rewrite. The
study's `instructions` are the system prompt, not a message, so a
`session.update` never falls inside a cut; deferred context items wait for
the run that compacts to return like any other. "Nothing is dropped" means
no context item is lost unrecorded — compaction's summary of earlier turns
is the seat's history, as it is the owner's.

**What a ClientTool declares.** An armed loop refuses every
`Egress::Chosen` tool, so a study tool labelled `Chosen` fails the first
action after the first image; and `mcp.rs` records why a wire-declared tool
can never be labelled anything else — "a remote tool's input schema is the
server's to write", so nothing can prove it holds no destination. The way
through is the one CLAUDE.md already names: `Blind` is earned in code by a
tool this repo can read. So ClientTools are a **closed catalogue in mecha's
code** — `send_chat` and the six answer tools above — each with the
schema HRP pins for it, each `Blind` because its one recipient is the
session the seat answers (the party every reply already reaches) and its
schema has no destination field. Each one's `Capabilities` are pinned whole,
not only on the egress axis: `untrusted_input` (its result is the study's,
marked `.from_outside()`), never `private_data` (it reads none of the
owner's stores; a study image arms `private` by its content, at the fold,
whatever a tool declares), never `destructive`. A tool HyperStudy declares that is not in
the catalogue, or whose schema differs from the pinned one, is refused at
`session.update`, never registered as `Chosen` and never relabelled. A new
study tool is a catalogue entry in a reviewed change, on both sides. What
HyperStudy does past the session with a rating is the study's; what mecha
guarantees is that nothing it sends goes anywhere else. The private leg a
seat carries is the study's own material shown back to the study, but the
send-axis leak guard (`block_sends_after_private`, which refuses `Blind`
too) is a `[security]` switch — a machine fact and the owner's standing
word. The wiring takes `[security]` field by field (§3; §8 step 2 tests
it): the owner's value where it narrows a seat, the strictest value where it
would widen one, with one departure. `block_sends_after_private` is the
one field where the owner's default (off) is the *wider* value, and it is
taken as the owner's rather than as the strictest, until §9's ruling. So it
binds a seat when the owner has it on, while `trifecta` is pinned to `block`, its strictest value — an owner's `allow`,
set for trusted content on their own network, never reaches a stranger's
seat, and neither does `ask`, which a seat would otherwise refuse only
because its approver overrides `escalate`, a second mechanism agreeing
rather than this floor — and `mark_untrusted_output` is always on. A seat's floor is a
property of the seat code, not of the operator's config. With
it off (the default) the catalogue works as above; with it on, a seat
cannot act after its first image, so the engine's hello offers no study
tools and seats are conversation only. Exempting the catalogue from an
owner's guard would be an owner's ruling, not a config value (§9).

**`Blind` reduces risk; it is not safety** (`EGRESS-DESIGN.md` §6). Blind
calls compose and nothing bounds their volume, and here the payload is
model-authored prose (`send_chat`, `answer_text`), not a sixty-byte query.
The bound is the per-seat budget below, in bytes as well as rounds.

**Not in a seat:** image generation (S5); any mecha tool — no `web_search`,
no file reading, no `http_fetch`, no shell, no owner `[[mcp]]` server, no
grants. Adding one later is a ruling, and the bar is that it reaches no
destination a stranger could aim and no content a stranger should read:
`http_fetch` would make this box a fetcher pointed at the owner's LAN and
tailnet. A file tool aims nowhere outside, and by §3's layout its jail is
the seat's empty `workspace/`, which holds no audio and not
the transcript — so it would be dead weight rather than a risk; the bar
for adding one is that the study has material to put there, and what that
material may contain.

**The tool list is declared once.** mecha's tool list is the front of its
cached prefix, so a study that swaps tools per screen re-pays the prefix on
every screen. The cheaper contract, asked of HyperStudy: declare every study
tool at session start, and let each state say which are available; a call to
an unavailable one is refused by HyperStudy with an error the model reads.

**Per-seat budgets** bound what a stranger can spend, and they come in two
kinds. Tool rounds per turn and bytes sent through ClientTools answer with
an `is_error` result the model can talk around, never a dead seat. Turns
per seat and the seat's wall time cannot work that way — a cap whose only
effect is an error the model talks around bounds nothing, since a turn is
the participant's to start — so when either runs out the wiring ends the
seat with a reason sent to HyperStudy, the shape §9 owes for the
owner-call and model-switch collisions.

## 5. Offers and admission

An offer is accepted only when every check passes; each refusal is typed
(`busy | unknown_persona | capacity | policy`):

- **The persona is servable.** A per-persona switch the owner sets, off by
  default. The engine's hello lists only servable personas, as opaque refs;
  an offer naming anything else is `unknown_persona`, so the rest of the
  owner's personas are never confirmed to anyone. Since a seat takes only
  the persona's voice (S10), servable means "this voice may speak in
  studies". That also settles PERSONA-DESIGN §15's line that a romantic
  persona must never reach a minor, "binding when it is designed": no
  persona's character — its template, card or relationship — reaches a
  seat at all, so a romantic one cannot reach a participant of any age by
  construction, not by a check on the switch.
- **The owner comes first.** A live owner call is `busy` for a voice seat —
  and an owner call that starts mid-seat does not preempt the seat
  silently; what happens then is owed (§9).
- **`max_seats`, counted where admission runs (S9).** `[hyperstudy]
  max_seats` caps text and voice seats together; an offer past it is
  `capacity`. The count is kept by the one long-running process that admits
  every seat, in the owner's home. `permit.rs` cannot do this: a seat is
  interactive work, which takes no permit by design, and each seat's guest
  home would give it a pool of its own. A seat can hold two router slots
  while it answers: the forced-on crisis judge (§7) is screened beside the
  reply, not after it (`persona_chat.rs` spawns `judge::screen` while the
  run goes on, which is how a verdict can stop a run in flight). So the
  default is set against the router's `-np` at two slots a seat, with one
  slot left for the owner — measured, not guessed (§9). Counting one would
  queue the owner's turn behind a participant's judge call, and per-request
  rates hide queue wait, so it would show only as a slow owner chat. Within the cap,
  voice seats are further bounded by GPU room for STT, TTS and the model
  beside whatever else holds the card (image and video servers start on
  demand), read from `mecha system`; one at a time until measured.
- **A seat holds the router's model in the owner's home.** The same
  per-home trap breaks `hold.rs`, and there it is a correctness bug:
  `Holds::open_default` roots at `MECHA_HOME`, so a guest facade's hold
  would land where `mecha model use` never reads, and a model switch could
  land mid-seat — one conversation answered by two models, the thing holds
  exist to prevent. So the **wiring** takes the hold in the owner's home
  when a seat opens and releases it when the seat closes; the guest facade
  never writes one. The seal stays literal — a seat writes nothing under
  the owner's home outside its seat directory — and the hold's liveness is
  the seat's lifetime, which the wiring already tracks. That widens
  `hold.rs`'s unit from a run to a seat of many runs, and its ruling has no
  time limit, so the owner's `mecha model use` waits on a participant for
  the whole session — or forces the switch and gets the mid-seat swap the
  hold prevents. Which way it goes is owed (§9), beside the owner-call
  collision.
- **The credential** for the control socket is a token mecha mints and the
  owner registers in HyperStudy (§5.1); an offer carries only a single-use,
  sixty-second session token.

### 5.1 The token, and why there is no URL

**mecha mints it (S7).** `mecha hyperstudy token` — and the same step in
guided setup — generates a connected-agent token: 256 bits from the OS
random source, prefixed `mhs_` so a leaked one is recognisable in a scan.
It is printed once, for the owner to paste into HyperStudy's settings, and
kept in the owner's home with the other credentials, owner-readable only.
mecha presents it in the control socket's handshake each time it dials;
HyperStudy keeps only its hash and checks the presented token against it.
`--rotate` mints a replacement; the old one stops working when the owner
replaces it in HyperStudy, and revoking it there cuts the connection on
mecha's next dial. The token is the familiar `hyperstudy-agent` shape — mint
locally, paste into HyperStudy — and the connection still runs outward:
which side generates a shared secret is independent of which side dials.

There is no public URL for this box, by any provider:

- **A tunnel** — Cloudflare's, which `hyperstudy-agent tunnel` runs, or
  Tailscale Funnel, which gives a public `*.ts.net` HTTPS address without
  putting the caller on the tailnet — publishes a listener on this box to
  the internet. `REMOTE-SURFACE-DESIGN.md` §2 ruled that out for the owner's
  own page ("Funnel is never the answer"); a door for strangers is the
  weaker case for it, not the stronger.
- **What a leaked key costs differs.** Behind a tunnel, a leaked bearer key
  reaches the engine — the box's model and whatever the endpoint can do.
  Over the dial-out, a leaked token lets someone pose as this engine to
  HyperStudy, and HyperStudy revokes it; nothing on this box answers it.
- **HyperStudy never joins the tailnet,** and needs no custom-URL guard for
  this engine: it calls nothing of ours.

## 6. The feature (S6)

`hyperstudy`, a new variant in the closed registry (`feature.rs`,
`FEATURES-DESIGN.md`) that requires `personas`: its own switch in
`[features]`, off by default and
off on every install that predates it, so nothing dials HyperStudy until the
owner says so. Guided setup offers it once personas are on and asks for the
two things it cannot work without — the HyperStudy origin, and a minted
token (§5.1), which setup mints and shows for pasting — and `Unready` names
whichever is missing. Voice
seats additionally need the voice worker installed — `voice`, not
`calls`: `calls` is what buys `mecha serve`'s relay to the worker (and so
requires `web`), and a seat reaches the worker through the wiring's own
loopback relay (§2), so it needs neither the owner's page nor its tailnet
listener. The registry already allows that: `requires` puts `web` on
voice's browser parts only (`dictate`, `calls`, `cloning`), and `voice`
itself stands alone (FEATURES-DESIGN F3: `mecha voice-serve` is its own
loopback surface), so no registry edge changes. `hyperstudy` requires
`personas` only, and a seat checks `voice` at runtime to decide whether its
hello offers voice — a `requires` edge on `voice` would block text seats on
a machine without it. Without `voice` the engine's hello offers text only.

The coupling to `mecha serve` is in code, not in the registry, and the
seat work has to undo it: persona calls are hosted only inside serve
(`VoiceHost` → `PersonaChats`, mounted only when `calls` is on), and the
worker reads its facade URL once, process-wide (`FACADE_URL` from
`MECHA_VOICE_LLM`). So a guest facade is built on `voice`'s `Facade` and
`Mount` with a seat's own `SessionHost`, which means exposing them beyond
serve. Two `Mount` fields are pinned for a seat, not inherited from
`voice-serve`'s mount. `approve_all` is `false`: it is the owner-present
posture, a mounted facade allowing every call, and a seat's approver is the
catalogue's (§4a). `inject_voice_block` takes neither existing value,
because both name the owner, and it is a `bool`, so the third value means
widening the field to an enum. That touches `mecha serve`'s and
`voice-serve`'s mounts too, which keep their two values. `false` would put `VOICE_BLOCK` in the system
prompt, which S10 gives wholly to the study. `true` adds the per-request
voice note (`VOICE_NOTE`), which says the reply is read "to the owner, who
hears it". A seat needs a third spelling, derived from `VOICE_NOTE_STREAMING`, not
`VOICE_NOTE`. The two are chosen per call by engine, a seat streams (§4),
and the streaming note carries no length rules by the owner's 2026-10-03
ruling. So the seat's note is the streaming note's guidance with the
listener left out, for the reason `BASE` is left out (§1), and it
re-imposes nothing that ruling removed. It is a per-request note like the
clock, never in the system prompt. (Strictly, `inject_voice_block: false`
does not write the system prompt; `voice-serve`'s `opts.system_extra =
VOICE_BLOCK` does, and a seat sets no `system_extra`.) The worker takes the facade's address per call — a URL for the
owner's calls, `seat/run/facade.sock` for a confined seat (§3) — and with
it the affect and direction endpoints derived from it at import
(`AFFECT_URL`, `DIRECT_URL`), carried by the wiring's relay as a trusted
field the way the persona voice binding is today; and
the voice binding check that `offer_proxy` does for a browser call moves
into the seat wiring. Building it follows `ARCHITECTURE.md` §Adding a
feature item by item rather than a subset copied here; the one choice the
checklist leaves to the feature is made here: `switchable_from_environment`
is **no**, because the credential lives in the owner's home and an
environment that could switch `hyperstudy` on would be a trial home dialling
HyperStudy.

## 7. Mechanisms, not policy (S2)

mecha provides; the study decides:

- **Crisis handling.** The harness-side crisis sensor (`persona::safety`,
  keyword tier plus model judge) runs on every participant turn — **forced
  on in a seat**, whatever the named persona's `safety.crisis` switch says:
  that switch is the owner's choice about the owner's chat, and it does not
  travel with the voice into a study's. Nothing here is inherited: the
  sensor runs in one place today, interleaved through `persona_chat.rs`'s
  handler — the keyword tier, the judge and its answered state, the
  cooldown, `record_crisis`. (`voice/direct.rs` and `listen.rs` only
  recognise `SAFE_MESSAGE` as harness speech.) So a seat is the second
  surface, and step 2 begins by lifting the sensor out into something both
  can call; it fails closed — a seat whose crisis path is not wired, or
  whose sensor cannot be built, does not open. The ledger follows the
  owner, not the seal: `record_crisis` appends to `<home>/personas/
  safety.jsonl`, which under the guest home would hide every study trip
  from the owner's host report, so the wiring writes a seat's records to the
  owner's store — they carry no words by construction — with the surface
  named `hyperstudy` rather than today's literal `web`. `CrisisRecord` holds
  no persona and no seat, so the record alone would say only "a study,
  somewhere". It gains an optional `seat` field naming the seat directory
  (a closed, append-only record, so absent loads as `None` and today's
  lines still read), set by the wiring, and §8 step 2 tests that a seat's
  trip leads back to its transcript. Its neighbour does
  not travel: `record_dose` and `record_call` — the owner-facing usage
  measures, turns and call minutes, written beside `record_crisis` and by
  `PersonaChats::call_ended` into the owner's store under the persona's
  name — are never written for a seat. A stranger's turns or minutes
  counted as the owner's would be the instrument measuring someone else,
  and §11's call meter exists because voice raises attachment. In a
  seat it keeps the actions it takes today: on a trip the harness steps out
  of character with `SAFE_MESSAGE` and pauses the persona for
  `CRISIS_COOLDOWN`, and mecha raises HRP's `hs.flag`. Two seat-specific
  differences. The crisis resources come from the study when the session
  supplies them, since a participant may be anywhere and `SAFE_MESSAGE`'s
  numbers are US ones. That field is a second trusted slot, and it speaks in
  the one utterance that is deliberately the harness and not the persona,
  so it is held to more than `instructions` is: experimenter-authored only
  under the same contract (§3), plain text within a length bound, and it
  supplies only the resources — mecha's own out-of-character sentence
  frames them. Absent, or failing the bound, the shipped `SAFE_MESSAGE`
  stands. And the handover
  is spoken or typed into the session, so until HyperStudy reads `hs.flag`
  the trip is still visible to the study in its own record of the
  conversation. Whether a study may turn the handover off is owed (§9).
  A seat whose judge becomes unreachable mid-session continues on
  keywords (`Degraded`), at parity with the owner's chats, rather than
  dropping a participant mid-sentence, while a seat whose sensor cannot be
  built never opens: the first is a check that went partial and is
  declared, the second a check that never existed. Whether a study may
  instead end a seat on `Degraded` is owed (§9) beside the handover.
- **The check's state is said, whole.** The seat declares its
  `CrisisState` to HyperStudy at session start and on every change, beside
  `hs.flag` rather than through it — all of it, not one variant, so "on",
  "switched off" and "couldn't check" never pass for each other on the far
  end either (`safety.rs`'s own rule). In a seat `Off` cannot occur (the
  check is forced on); `Degraded` — the judge could not answer, keywords
  only — can, and is said. The declaration needs only the session socket,
  so it lands with the text seat (§8 step 3), not with `hs.flag`; until
  HyperStudy reads it, a dropped declaration still leaves the state in the
  seat's transcript and manifest, and the study, reading only its own
  record, cannot exclude the session on it — which is why HyperStudy reading
  it is on its build list.
- **Retention.** Seats are kept in full (S8, §3), and mecha declares that
  through `hs.retention`, so a study can tell its participants.
- **Disclosure is in the study's prompt.** Whether a participant is told
  the other party is an AI is study design (S2), and the study writes the
  whole system prompt (S10), so it says what the study wants said. mecha
  adds no statement either way; the owner's per-persona `disclosure` switch
  governs the owner's page, where its truth-maker is, and does not reach a
  seat. Said beside §3's: this is the second guarantee a remote party
  keeps rather than mecha's code — and the half of PERSONA-DESIGN §15's
  host baseline that §15 attributes to statute, not to good practice.

## 8. Build order

Each step is testable before the next; only step 5 waits on HyperStudy.

1. **The feature and the token** — every item of `ARCHITECTURE.md`
   §Adding a feature, the `[hyperstudy]` table (origin, `max_seats`) in both
   `Config` and `ConfigLayer`, **stripped from project layers in
   `merge_text`, loudly**, as `[image]`, `[web]` and `[features]` are, and
   for `[image]`'s reason one step worse. `origin` is where mecha presents
   the owner's minted `mhs_` token, and it is the root of this design's
   trust chain: the `sessionUrl` origin pin, verbatim `instructions` and the
   catalogue's `Blind` labels all rest on the owner having chosen that
   endpoint. A `mecha.toml` arriving with a cloned repository must not
   redirect it, nor raise `max_seats` (`[system]`'s reason); a test that a
   project layer's `[hyperstudy]` is dropped with a warning. The same
   reason puts `hyperstudy` on `trial_env::OPERATOR_ONLY_TABLES`, the
   decision `ARCHITECTURE.md` §Adding a feature asks of every new table:
   an experiment environment's `config.toml` also arrives with a checkout,
   and `[documents]` once sat on the project strip list but not this one.
   It is inert today only because `switchable_from_environment` is `no`
   (§6), a second mechanism's accident rather than this table's guard. And `mecha hyperstudy token` with setup's
   step; nothing behind it yet but `Unready` until a token exists. A test
   on the chained `enable_command`, since `hyperstudy` is the first
   switch-bearing feature with a `requires` edge and that branch of
   `plan_enable` has never run. And `hyperstudy`'s row in `TRIFECTA.md`'s
   switch table, with what it costs, including the named
   `block_sends_after_private` departure (§4a).
2. **The guest facade** — `Facade`, `Mount` and a seat `SessionHost` usable
   outside serve; the confined launch (§3 — the socket bridge with its
   preflight of every leg, or the filesystem-only interim, per §9 — and,
   first, a one-line compile check that reqwest's
   `ClientBuilder::unix_socket` exists in the locked version, since the
   bridge's shape rests on it); guest home, untrusted run mode, the §3 table as
   tests (the sensor lifted out of `persona_chat.rs` first, with a seat's
   trip recorded in the owner's ledger as `hyperstudy`; seat transcripts
   absent from the owner's corpus; the approver allows the catalogue and nothing else, every refusal
   rendering `Blocked by policy:` and never the user-denial prefix — and whether `Approver` gains
   a door enum so the compiler holds a future door is decided here; the
   seat is armed untrusted before its first turn; the guest facade process
   writes nothing under the operator's home outside its own seat directory,
   `parent.sock` accepts only its closed message set and fails the seat on
   anything else, and re-checks a ClientTool call against the catalogue on
   arrival; the seat host's fold machine (§4a's four cases) with its
   context endpoint, which parks an item without a run; a run that fails
   after context and a participant turn were folded into its tail restores
   without popping, so both are still in the next request (the test
   asserts the words survive, not only that a rewrite was recorded, and
   that a taint checkpoint follows the rewrite); and
   a run that fails after a forced answer ran keeps that round, so the next
   request carries the `tool_use` and its result and no second answer call
   goes out for the same screen;
   and the wiring's writes there are a closed list of two — the router hold
   (§5) and the crisis record (§7) — among the owner's stores, beside the
   seat directory itself and §5's admission counter, which are the wiring's
   own state and not stores the owner's sessions read; a seat's session carries its own
   `SessionKind` and the corpus scan excludes it, by an arm of its own in
   `Scan::admits` (which admits any kind it does not name), not by the
   variant alone; `check-private.py` reads the seat
   transcripts; no dose or call record is written for a seat; a seat's
   config carries `context_window`, `vision` and `structured_output` (so
   the crisis judge's `response_schema` applies, at parity with the owner's
   chats), the owner's `max_tokens`, and `max_turns` one above the round
   budget and pins its model with
   `follow_loaded = false`; a study's crisis
   resources that fail their bound fall back to `SAFE_MESSAGE`; the `[security]` floor
   holds against an owner config with `trifecta = "allow"` and with
   `trifecta = "ask"`, each refused by the interlock itself; the system
   prompt is the study's `instructions` byte for byte, with no `BASE`, card,
   `SINCERE` or `DISCLOSED` even with the persona's `disclosure` on; a closed seat's
   transcript and manifest are readable afterwards; while a seat is open the
   wiring's hold is visible
   to `mecha model use`), the
   ClientTool catalogue (an armed seat with an image in it still answers a
   question; an uncatalogued tool is refused), context folding at a
   mid-round boundary and on a closing turn, unfolded items recorded, the
   crisis handover and the degraded state, a seat refusing to open without
   its crisis path, the facade's environment holding only the allowlist and
   its config loaded `global_config_only`, its sandbox bwrap under an owner
   config with `[sandbox]` absent or `none` (and no seat opening where bwrap
   cannot run, with the step's bwrap tests run under
   `MECHA_TEST_REQUIRE_BACKENDS=1` in CI, since a skipped confinement test
   reads like a passing one), `facade.sock` answering the wiring
   from a confined facade (a text seat's only route in), `approve_all` pinned
   false with the approver test driven *through* the facade's mount so it
   fails if the mount allows everything, no seat request containing "owner"
   in any note, voice or text, the
   calendar head note present with a verbatim study prompt and rendered
   in `UTC` under an owner config with another zone, the guest
   config holding no `[[hook]]`, skill, charter or subagent, the owner's
   `[security]` read and never written, budgets.
3. **A text seat on loopback** — HRP with text-only output against a local
   stand-in for HyperStudy's session: context items, inline images, declared
   tools, the steering queue widened to carry `Block`s (§4a), and the
   `CrisisState` declaration on the session (§7). Exit: a scripted study (read a screen, chat, answer a rating
   through a silent forced call) runs end to end, including a context item
   with no `response.create` followed by the participant speaking, once
   before any turn and once after one — the
   sequence that would otherwise send two user messages. And a barge-in mid-tool-round, after which the next request still
   carries every `tool_use` with its result. And a context item
   arriving mid-reply in a turn with no tool call, which must wait for the
   run rather than enter a message already sent. And an image folded
   mid-round with `block_sends_after_private` on: the next send in that same
   run is refused, and the session records the arming in that round; plus
   an `instructions` over the bound refused on its `session.update` with the
   seat still open on its previous instructions.
4. **The voice transport on loopback** — the first non-WebRTC transport,
   with its own entrypoint (§2); the worker takes its facade address (and the
   two endpoints derived from it) per call from the relay — over
   `facade.sock` for a confined seat, through an `httpx` socket transport
   for the affect and direction clients and an `OpenAILLMService` subclass
   whose `create_client` hands the OpenAI client the same transport (§3) — and its HRP transport takes
   the wiring's loopback relay, which is its only shape (§2), and
   HyperStudy's conformance suite runs against it through a local stand-in
   for the crate (its runner takes any HRP client; a URL wrapper is all it
   needs). Exit: both turn-detection modes pass, truncate `approximate`.
5. **`mecha-hyperstudy` + wiring + admission** — once HyperStudy's `accept`
   exists; the crate replaces the local stand-in, dialing the control and
   session sockets and validating frames, and nothing behind it changes.
   The same change adds `mecha-hyperstudy` to `release.yml`'s hand-kept
   publish list ahead of `mecha-cli` (it depends on nothing in the
   workspace), since a published crate's missing dependency fails the
   publish after earlier crates uploaded, as `mecha-slack` once did. Its
   no-`mecha-core` property gets a test that reads its `Cargo.toml`,
   rather than resting on review as `mecha-slack`'s does. Once the surface
   exists, `TRIFECTA.md` gains a row for each channel the seat opens: the
   reply itself as a way out no interlock sees (§1), owned by the sealed
   home and the closed `Blind` catalogue; and the `instructions` slot,
   third-party bytes in the system prompt, which `arm_for_content` never
   reads and `mark_untrusted_output` never wraps (§3), owned by the bound
   on its size, the seat being armed `untrusted` from construction, and
   the experimenter-authored-only contract HyperStudy keeps.
   It is the only code on this box that parses bytes from the internet,
   and steps 3 and 4 were proved against a stand-in that replays what
   this design believes HyperStudy sends. Exit: HyperStudy's conformance
   suite passes through the real crate, not the stand-in; and a corpus of
   malformed and hostile frames (truncated, an `instructions` past its
   bound, a `sessionUrl` off the control socket's origin, an uncatalogued
   or mismatched tool, oversized audio, an unknown frame type) each gets a
   typed rejection, with the seat left closed and nothing reaching the
   facade.
6. **`hs.flag`** — once HyperStudy reads it.

**What HyperStudy builds** (its document's Phase 3): connected-agent
credentials, the control socket, the engine hello, offers and rejects,
session tokens, an adapter that gives its voice bridge an HRP client over an
accepted socket, text seats on the same socket, a `connected` engine naming
the registered token and the persona whose voice speaks, a settings page to
register a mecha-minted token (stored hashed, revocable), inline images on the
session, the study tools as a catalogue pinned in HRP, declared once per
session (§4a), each answer tool's schema carrying the on-screen question's
identifier, at most one answer recorded per session and on-screen question, keyed
on the question rather than the `call_id`, since a model's second answer
is a new call, so an answer repeated after a timeout is refused (§4a), and `instructions` kept to experimenter-authored text, with
participant-authored bytes only ever sent as context items (§3). **The frames
are pinned in its §4.1 before either side builds against them** — with a
direct connection, HyperStudy's spec is the contract mecha codes to.

## 9. Out of scope, and owed

**Deliberately out:** the owner's running mecha and main chat (S3); a
general assistant, until it is designed as its own sandboxed instance;
image generation (S5); any public listener, tunnel or Funnel on this box;
mecha joining LiveKit — HyperStudy's voice bridge is the only room presence
and engines never hold room credentials; `hyperstudy-agent` (S1); platform
mode for two or more humans, until HyperStudy builds it — and then the
catalogue's `Blind` labels are re-derived, not inherited, since `send_chat`
would reach participants the seat's own reply does not; HyperStudy's
inbound chat-completions endpoint for text agents — text seats ride the
accepted socket instead.

**Owed:**

- A timed-out answer whose acknowledgement was lost and whose answer never
  landed: §4a's guard then leaves that question unanswerable for the rest
  of its screen, a datum silently missing from the study. Whether the seat
  reports it to HyperStudy (so the study can re-ask, or mark it), and how
  that meets HyperStudy's per-question backstop, is owed before step 3.
- Video perception, owned by another session (2026-10-21), not this design:
  a time-indexed text track of the movie participants watch, released to a
  seat no further than the playback position. It is expected to arrive
  through the paths here, as context items (§4a) or a catalogue tool, and
  any change it needs to HRP events or the catalogue is a reviewed change
  on both sides, coordinated with that session.
- Whether a seat's study tools may run when the owner has `block_sends_after_private` on (§4a); conversation only until ruled.
- What happens when the owner switches the router's model mid-seat (§5):
  wait out the seat, or end it with a reason. `hold.rs` makes the run the
  side that yields, and here the wiring holds for the seat, so the answer
  is the wiring's: refuse to yield until the seat ends (the owner waits,
  with no time limit), or end the seat with a reason and yield.
- What happens to a live voice seat when the owner starts a call: end the
  seat with a reason, or refuse the owner's call while a study runs.
- A reader for `hyperstudy/seats/`: it grows without bound by design
  (S8), so `doctor.rs` should report its count and size, and how the owner
  prunes it is theirs to say.
- The budget numbers and `max_seats`' default, set from a measured seat,
  not guessed.
- Whether a study may turn the crisis handover off (§7); on until ruled.
- What a trip does to a seat shorter than `CRISIS_COOLDOWN`'s fifteen
  minutes: a pause that outlasts the seat ends it, which is not today's
  action.
- Whether the Unix-socket bridge (§3) lands with step 2 or follows a
  filesystem-only first cut — before any real participant either way. The
  bridge is three legs, not two: `model.sock` and `parent.sock` out of the
  facade, and `facade.sock` into it, for the wiring in every seat (step 2)
  and for a voice seat's worker too (Python work in step 4). The interim
  needs none of the three.
- Latency, measured on this box: OpenAI's Realtime models are
  speech-to-speech and mecha is a cascade, so vendor figures are not ours.
  End-of-turn to first audio is recorded per turn-detection mode, with the
  router's resident model read from `served_props`, before a study depends
  on it.
