# One session engine: principals, profiles and doors

*2026-10-20. Design, not built.* The design for one conversation engine in
`mecha-core` that every surface drives. In it, who is speaking is fixed by
the connection rather than assumed, what a session can do is a closed
profile, and the web page, the terminal, Slack and HyperStudy become doors
onto it. The document also says how the move is made without losing a
behaviour or breaking setup, because that is the risk the owner named (E3).

PERSONA-SERVING-DESIGN.md (PR #724) is the first new door built on this
engine, and this document depends on it: S1–S10 and several sections are
cited from there, so #724 lands first and this one after it: a HyperStudy seat is the `Seat` profile behind the HyperStudy
door. That document stays the door's specification, and this one does not
restate it.

## 0. Rulings and constraints

| # | Ruling | By | Date |
|---|---|---|---|
| E1 | **The speaker is decided by the connection.** No surface assumes the human side is the owner. Identity comes from how the connection authenticated, and nothing said in the conversation changes it. | owner | 2026-10-19 |
| E2 | **One engine, the cleanest and most modular design.** The owner is open to it in principle. This document is what the owner reviews before deciding to build. | owner | 2026-10-19 |
| E3 | **Nothing is lost, and setup does not break.** A refactor this size is how behaviours quietly disappear. The migration (§9) is built around making each one visible before anything moves. | owner | 2026-10-20 |
| E4 | **General text and voice chat without the web page.** A connection can supply the prompt and the session's setup. | owner | 2026-10-19 |

PERSONA-SERVING-DESIGN's S1–S10 hold for seats unchanged. Four rulings
are still open (§11).

## 1. What is duplicated, and what is different

Measured at `dc00a90b2` (`git show dc00a90b2:<file> | wc -l`, and
`#[test]`/`#[tokio::test]` attributes counted with `grep -c`):

| File | Lines | Test attributes |
|---|---|---|
| `mecha-cli/src/commands/serve/persona_chat.rs` | 13,705 | 121 |
| `mecha-cli/src/commands/serve/chat.rs` | 7,012 | 52 |
| `mecha-cli/src/tui/mod.rs` | 11,287 | 62 |
| `mecha-cli/src/voice/mod.rs` | 3,535 | 44 |
| `mecha-cli/src/slack/connector.rs` | 3,378 | 20 |
| `mecha-cli/src/commands/serve/incognito.rs` | 795 | 10 |

Both web chats live inside the CLI's web server, which already breaks
CLAUDE.md's rule that the CLI is thin and the logic lives in core. They
are the same engine written twice. Each copy has:

- a session map (`WebSession`, `PersonaSession`) and a live-run struct;
- a turn body: fold or push the user text, refuse to run if recording
  fails, run `run_in`, `record_run`, `roll_back_failed_turn` on error,
  hand late jobs back;
- steer, cancel and job reordering;
- a background-job queue (`late::Jobs` `marked("chat")` and
  `marked("persona")`) with its own delivery task;
- a barge-in loop (`BARGE_IN_TRIES` is defined in both files);
- a read-aloud seat (`listen_seat` exists in both);
- a transcript builder, an agent builder and cache, and a key scheme;
- a mirrored route set: `/api/chat/{key}/…` and
  `/api/persona-chat/{key}/…` each have `send`, `cancel`, `events`,
  `jobs/order`, `upload` and `file`.

Standalone voice (its `Slot` map in `voice/mod.rs`), `mecha chat`, the TUI and Slack
each run their own assistant session loop as well. The appraisal readout
(`appraisal::live_readout`) is wired in three of them: `serve/chat.rs`,
`tui/mod.rs` and `slack/connector.rs`.

**What is really different** fits in five lines, and every one survives
the refactor:

- **Prompt.** The assistant gets the config prompt, the charter and
  learned rules. A persona gets its card and the persona safety lines. A
  seat gets the study's instructions verbatim (S10).
- **Tool ceiling.** A persona's registry is built by
  `persona::agent::registry_as`: eligible only through `Tool::for_persona`,
  read-only, and below `Egress::Chosen`. PERSONA-DESIGN §3.3: no
  conversation holds both the assistant's reach and a persona's voice.
- **Stores.** The assistant has the learning store, the brief and the
  charter. A persona has its memory database, recall and files.
- **Persona safety.** The crisis sensor and judge, the dose and call
  meters, re-anchoring, the library lock.
- **Storage location.** Persona sessions sit under
  `personas/<name>/sessions/`, which no reader of the owner's corpus scans.

**The owner is assumed at every door.** The approver's cards; `Deny` mined
as the owner's correction; a spoken "yes" releasing a draft (`voice/confirm.rs`,
`review_policy::parse_answer`); persona memory filing the human side as
facts about the owner; call notes that say the owner is speaking. PR #724's
review passes kept finding another door with the same premise, and each
got its own patch. That is the case for E1: one type instead of one patch
per door.

## 2. The model

A session is three values, fixed when it opens and recorded in its file:

```rust
pub enum Principal {
    Owner,
    Guest { issuer: Issuer, label: String },   // e.g. Issuer::HyperStudy
}

pub enum Profile {
    Assistant(AssistantSpec),
    Persona(PersonaSpec),
    Seat(SeatSpec),
}

pub enum Recording {
    Kept,        // the owner's stores, as today
    Incognito,   // no transcript, by type (INCOGNITO-DESIGN)
    Sealed,      // kept in full, under the guest's own directory (S8)
}
```

- **The principal** is set by the door from authentication: the tailnet
  login (`owner_guard`), the local OS user, or a HyperStudy `mhs_` token.
  There is no upgrade path: no frame, field or spoken phrase changes it.
- **The profile** is a closed set defined in code. Config fills in its
  parameters but cannot invent a fourth kind.
- **Recording** is its own axis. That is what makes an incognito persona
  a combination rather than a feature.

`Session::open(principal, profile, recording, setup)` refuses every
combination not on this list:

| Principal | Profile | Recording |
|---|---|---|
| Owner | Assistant | Kept, Incognito |
| Owner | Persona | Kept, Incognito |
| Guest | Seat | Sealed only |

A guest never gets Assistant or Persona: S3, and a guest-assistant profile
is designed when it is wanted, not admitted by a missing check. The owner
never gets Seat: a pilot of a study connects through HyperStudy like a
participant, so the pilot tests what participants get.

**The principal alone decides:**

- whether the conversation starts armed untrusted;
- which approver is installed. A guest's approver never returns
  `Decision::Deny`: it allows a call in the seat's catalogue and returns
  `Blocked` for anything else. Seat tool calls do reach it. They are
  dispatched as proxy `Tool` impls through the same interlock → hook →
  approver path as any tool (a seat has no hooks), so no tool class
  bypasses the approver. The loop keeps choosing the refusal prefix from
  the variant alone (`Denied by the user:` mined, `Blocked by policy:`
  never). The principal is never a second input to that `match`: an input
  there beside the variant is how machine policy could be labelled as a
  human correction and mined into a rule;
- whether a spoken confirmation can release anything;
- the wording of notes that name the human ("the owner is speaking"
  becomes the principal's description);
- which process runs it: the owner's mecha, or a sealed child with its own
  home (PERSONA-SERVING §3).

**The principal vetoes, alongside the other two axes.** Two eligibilities
are conjunctions over principal ∩ profile ∩ recording, and the principal's
part in them is only a veto:

- **Persona memory may write** only for Owner, *and* the Persona profile,
  *and* `Kept`. Assistant and Seat have no writer (§4); `Incognito`
  switches it off (§6).
- **The session may enter the owner's corpus** only for Owner, *and* the
  Assistant profile, *and* `Kept` (§5). That is necessary, not sufficient.
  It narrows alongside the existing `SessionKind` filters and never
  replaces them: a `MECHA_SESSION_KIND=test` smoke test is Owner ×
  Assistant × Kept, and `Scan::admits` still drops it on its kind.

Each is computed in one function of all three values, never derived from
the principal alone. The incognito persona that arrives in step 8 is
where a principal-only rule would bite, with every earlier step green.

The principal goes into the session file as a record, and the record is a
wire format: a principal that fails to parse loads as `Guest`, never as
`Owner`, and specifically as `Guest { issuer: Issuer::Unknown, label:
"(unreadable principal)" }`. That label cannot be mistaken for a person's
name, and `Issuer::Unknown` opens no profile, so the session reads back
and cannot be resumed. Unknown is never clean. An *absent* record is a different case
with its own rule (§3), and the two must stay two code paths.

## 3. Settings: where each one lives

Four layers. Three of them already exist and keep their formats:

| Layer | Where | Who writes it | Decides |
|---|---|---|---|
| **Code** | the profile's definition in core | nobody at runtime | the invariants: a persona never holds a `Chosen` tool; a seat enforces the trifecta check whatever the owner's config says |
| **Machine config** | `config.toml`, layered, as today | the owner, reviewed as a diff | providers, router, sandbox, `[security]`, MCP servers, outbox routing, `[hyperstudy]`: the ceiling for the whole box |
| **Owner stores** | `~/.mecha`: `persona.toml`, charter, skills, the library | the owner (files, the web settings page, the TUI) | one profile instance: this persona's voice, model, `tools.allow`, `[safety]` switches |
| **Session setup** | the door, at session start | the connection, within what its principal allows | an Owner door: the model chip and picture settings, and the approval mode (below). A Guest door: HyperStudy's `session.update` (prompt, study tools, turn-taking) |

Two kinds of setting, resolved differently:

- **Capabilities only narrow.** Tools, MCP servers, egress, memory writes.
  The effective set is code ∩ config ∩ instance ∩ setup. A lower layer
  removes, and none adds. This is today's rule spelled once: a persona's
  `tools.allow` is already intersected with an eligibility each tool
  declares, and an MCP capability override can already only distrust a
  server more.
- **Preferences: the most specific layer wins, falling back up.** Voice,
  speed, model, picture settings. Also today's rule: a persona's picture
  settings fall back to the chat's (#654).

Mixing the two kinds is how a setting ends up widening security, so each
field's kind is part of its type, not a convention.

**The approval mode is a third kind: owner-only.** The web page's mode
chip is the `PermissionMode`, and `parse_mode` accepts `"allow"` off the
wire against the session's `ReadOnly` default. So it widens, and is
neither a narrowing capability nor a preference. It is settable only
through a door whose principal is `Owner`. A guest door's setup has no
mode field at all, and a frame carrying one is refused as an unsupported
field.

**Worked examples:**

- **A persona's web chat.** Tools = persona-eligible (code) ∩ MCP servers
  with `personas = true` (config) ∩ `tools.allow` (`persona.toml`). Voice =
  `persona.toml`'s, else the worker's default. Prompt = the card.
- **A HyperStudy seat.** Tools = the seat catalogue (code) ∩ what the study
  declares. Prompt = the study's instructions (S10). Voice = the persona's.
  Model = the owner's, pinned (`follow_loaded = false`). The study chooses
  the prompt and cannot add a tool or reach the config.

**What a guest connection may set:** the system prompt (where the profile
takes one), the tools it runs on its side, voice and speed, turn-taking,
which persona by reference, text or voice, a display label for the human.
**What it never sets:** the principal, the owner's stores, any security
switch, the approval mode, the provider or its keys. An owner door adds
the approval mode and nothing else from that second list.

**E3 for configuration:** no *existing* field of `Config` or `ConfigLayer`
changes, no key of `persona.toml` changes, the `~/.mecha` layout does not move,
and the session file's existing records keep their shape. The principal
record is the one addition, and a file without it loads as today: `Owner`
for files under the owner's sessions, and refused for a seat directory,
where every session is written with one. The one new table is
`[hyperstudy]`, and it arrives with PR #724's feature, not with the engine.
It is CLAUDE.md's two edits, `Config` and `ConfigLayer`, held by
`every_field_of_config_is_reachable_from_a_file`; one edit alone would
make the table a startup parse error while every unit test stayed green.
*Absent* and *unparseable* are
separate paths: a single `#[serde(default)]` resolving to `Owner` would
merge them, and an unparseable seat principal would then load as the
owner. Absent is decided by where the file sits; unparseable is always
`Guest` (§2).

## 4. Modules: what a profile is made of

Inside, a profile is a fixed composition of modules. The composition is
chosen in code, and `Session::open` checks its invariants. It is never
assembled at runtime from parts, because persona safety rests on certain
capabilities never meeting, and free composition is how that breaks.

| Module | Assistant | Persona | Seat |
|---|---|---|---|
| **Prompt source** | config prompt + charter + learned rules | the card + safety lines | the study's instructions, verbatim |
| **Tool policy** | the full registry, the approver's modes, outbox, hooks, grants | `registry_as` allowlist, read-only | the seat catalogue only (PERSONA-SERVING §4a); HyperStudy runs the tools, and each result comes back `external`, marked `.from_outside()`, the only tool class whose result never passes through mecha's own code |
| **Context providers** | brief, homeostat, late notes, voice note | memory, recall, files, re-anchor, variety, scene, call note, late notes | context items, the clock (in UTC) |
| **Store** | `sessions/`, corpus: yes | `personas/<n>/sessions/`, corpus: no | the seat directory, corpus: never |
| **Safety** | — | crisis sensor (per-persona switch), dose, lock | crisis sensor, forced on |
| **Post-run** | appraisal, title, staged-draft card | citations | — |
| **Voice binding** | the worker's voice | the persona's voice, checked at bind | the persona's voice, checked by the wiring |
| **Workspace** (the path jail every model-supplied path resolves inside, `ToolCtx::resolve`) | `~/.mecha/work/web/<key>/`, plus the owner's grants | `~/.mecha/work/persona/<key>/`, no grants | the seat directory's own `workspace/`, no grants (PERSONA-SERVING §3) |

**What each function becomes:**

| Function | Assistant | Persona | Seat |
|---|---|---|---|
| Learning store, charter, skills, hooks | on | off | off |
| Persona memory and recall | — | on, the owner's only | off |
| Outbox, approvals, `ask_user` | on | off (read-only) | off; study tools only |
| Crisis sensor | off | on (switchable per persona) | forced on |
| Pictures | on | on | off (S5) |
| Voice calls | yes | yes | yes, over HRP |
| Background jobs, steer, cancel | one implementation | the same | the same |
| Slack | yes | never (Slack's servers would read it) | — |

**The jail root is fixed by profile and recording in code, never by a
connection or setup.** `Incognito` replaces any profile's root with its
tmpfs room. The `upload` and `file` routes (§1) are descriptor-relative
I/O inside that root (`workspace_files.rs`), so the host owns the root and
the doors only carry bytes. For a guest, where the jail is rooted is a
security answer, not a detail: a seat's root sits inside its sealed home,
and `Session::open` refuses a root outside it.

**Separate on purpose:**

- **Persona memory and the learning store stay two modules.** They answer
  different questions, and merging them reopens PERSONA-DESIGN §3.
- **The persona tool ceiling is an invariant**, checked at construction,
  not a setting.
- **Guests run in a sealed child.** Same code, different process.

## 5. Appraisal

Appraisal becomes the post-run module. The engine runs it after
`record_run` and emits one readout event, and each door renders that
event. The three wirings go.

| Session | Appraisal | Goal source | Enters the owner's corpus |
|---|---|---|---|
| Assistant, owner | on, as today | charter, project, task | yes |
| Persona, owner | off | — | no |
| Persona, scripted scenario | later (`GoalRef::Scenario`, PERSONA-DESIGN §7.4, not built) | the scenario | its own store only |
| Seat | off | — | never |

Two rules carry it:

- **The principal shapes the channels.** An interruption or a refusal
  counts as a correction only when the principal is the owner. With a
  guest in the seat, the intervention channel measures something else
  while looking identical in the data.
- **Corpus exclusion keeps the folder as the guard, and adds a fact in
  the type as the second check.** PERSONA-DESIGN §3.2 ("Why not a
  `SessionKind`") ruled the folder primary. `Scan::admits` admits every
  kind it does not name, `de_lenient_kind` reads an unknown kind as
  `None`, and so a binary built before a new variant (an older install,
  `mecha-mail`, the graph's MCP server) admits it, and nothing added here
  can teach that binary otherwise. A directory no reader scans excludes
  for every binary, old and new. The type is what lets *this* binary
  refuse early, and what holds against a recursive reader added later.
  The stakes are why the fail-open guard is never the first one. The corpus is not only a log:
  rumination, the diagnostician and learned rules close loops through
  it, and config rumination accepts changes on its own. A seat in the
  corpus would let strangers steer the owner's harness, which is the lane
  promoting itself with strangers as the lane.

Appraising seats on purpose (offline, over a study's sealed transcripts,
against goals the study declares) is a later analysis tool for
experimenters. It writes under `hyperstudy/` and never feeds rules,
lessons or rumination. A text appraisal there would mean a model reading
wholly untrusted transcripts, so it would take the front door's
quarantined, typed-output shape.

## 6. Incognito

Today `Recording::Incognito` is a variant inside `serve/chat.rs`. The
transcript is absent by type, the workspace is a tmpfs room
(`incognito::Room`), and the tools are an allowlist whose complement is
withheld (`incognito::withheld`). An incognito persona (PERSONA-DESIGN
§8.4) is designed and not built.

In the engine, `Incognito` is a value of the Recording axis:

- **It combines with Assistant and Persona.** An incognito persona needs
  only its room and the persona allowlist intersected with the incognito
  one.
- **It requires the owner.** A guest cannot ask to leave no record: S8
  keeps every seat in full for investigation.
- **It switches off, by construction:** appraisal, corpus entry, the
  persona memory writer, learning, and the dose record. Its own audit
  (INCOGNITO-DESIGN §8) moves with it as tests, unchanged.

## 7. The host: one turn loop

`ConversationHost` in `mecha-core` owns the turn. The name is chosen to
stay clear of `voice::SessionHost`, the existing CLI trait through which a
web chat exposes its sessions to calls; the two coexist for the whole
migration (§9.3). The host's job:
record → fold → run → `record_run` → roll back on failure → hand back late
jobs → emit events. Recording comes first and is a guard: a turn whose
record cannot be written does not run, as both copies refuse today. A
guard that cannot run stops the run. It is the only place that does each, with one steer,
one cancel, one job reorder, one job queue and one event stream. The
shared `WireEvent` gains profile extensions rather than a second enum.

- **Voice** enters through one host entry. The binding (which voice, checked
  how) is the profile's Voice binding module, so persona calls stop
  depending on `mecha serve`.
- **The crisis sensor** becomes a Safety module in core. PERSONA-SERVING
  §8 step 2 has to lift it out of `persona_chat.rs` anyway; here it is
  lifted once, for every door.
- **Rollback** keeps PERSONA-SERVING §4a's split: a door that can resend
  pops the failed user turn, and a door that cannot (a seat) restores the
  snapshot without popping. Which applies is the door's declaration, not
  the profile's.

## 8. Doors, and the protocol at the door

A door is a transport, how it authenticates (which fixes the principal),
and how it renders. Nothing else.

| Door | Transport | Authentication → principal | Renders |
|---|---|---|---|
| Web page | HTTP + SSE, WebRTC for calls, on the tailnet | tailnet login → Owner | the browser UI |
| CLI and TUI | in process | local OS user → Owner | the terminal |
| Slack | Socket Mode | the allowlisted user → Owner | Slack messages (Assistant only) |
| HyperStudy | dial-out WebSocket, HRP | `mhs_` token → Guest | nothing; HyperStudy renders |

**The standard sits at the door, not in the engine.** HyperStudy speaks
HRP, a pinned subset of OpenAI's Realtime API with a conformance suite.
The engine is mecha's own Rust and is not shaped by one vendor's event
format, so a different standard later costs one adapter. Whether the web
page also moves onto the Realtime protocol is open (O4) and is decided
after HyperStudy has met the protocol from a real outside client.

**Other doors.** The HyperStudy door is also the template for an API to
other applications: every new client is a door, and the principal rule
decides what it gets. A guest door gets a sealed profile. An owner door
reaching the owner's whole assistant from outside the tailnet is a
different matter, because it carries the full trifecta: it stays on the
tailnet (REMOTE-SURFACE-DESIGN: "Funnel is never the answer"), with its own
token, and is designed when wanted, not admitted by this document.

## 9. Moving without losing anything (E3)

The risk is not that the new engine is wrong. It is that a behaviour
nobody listed stops happening and nothing fails. Every rule here makes a
behaviour visible before it moves.

1. **An inventory before any code.** Step 0 writes `PARITY.md`: every
   behaviour of every surface, one row each, each mapped to the test that
   holds it or to a scripted manual check where no test can (a phone
   call's audio, a picture's edit modal). It is built from the ten
   dimensions of the code survey behind §1 and from every control on the
   web page. The owner reviews it, because the row nobody wrote is the
   behaviour that gets lost. A step moves only rows the inventory holds,
   and closes them by name.
2. **Byte-identical requests, over the rendered body.** What the model
   sees is the provider's rendered request, not the `CompletionRequest`
   struct. The struct carries no cache breakpoints, because `cache_control`
   placement is decided inside the private `Anthropic::body`. A
   struct-equality check would therefore pass while the breakpoints moved,
   which is exactly the regression "the cached prefix is sacred" is about.
   So each extraction step runs a fixed set of made-up conversations through
   the old path and the new one, and compares each provider's rendered JSON
   body byte for byte: `Anthropic::body`, and `openai.rs`'s body, which is
   what the router receives and what llama-server's prompt cache matches
   on. One equality then covers the prompt, the tool list, every note and
   the cached prefix. The harness does not exist yet, and step 0 builds it:
   - **A request recorder exported from `mecha-core`** behind a test-only
     feature, so `mecha-cli`'s old path and core's new host drive the same
     recorder. Today's candidates are private `#[cfg(test)]` fixtures that
     don't fit. `agent.rs`'s `ScriptedProvider` records `CompletionRequest`s
     but only inside core's tests. `provider/mod.rs`'s `Recording` keeps
     only `req.model`, and `agent.rs`'s `Recording` is an `Approver`.
   - **A test-only render for each provider's body**, of which `openai.rs`'s
     `pub(crate) body_for_test` is the seed on one side. On the other,
     `anthropic.rs`'s own test module already renders `.body(req, true)` and
     compares its two encoders' bodies, so the Anthropic render exists and
     is only module-private.
   - **Multi-turn fixtures.** A one-turn conversation cannot catch the
     breakpoint that moves, the one on the last message block. Every
     fixture runs at least two turns with a tool round between them.
   - **A compacting fixture.** Compaction is tangled with what steps 4
     and 5 move. Rollback after a compaction meets a live list *shorter*
     than the snapshot (`chat.rs`'s
     `a_failure_after_a_compaction_still_drops_the_triggering_message`).
     The persona re-anchor is keyed on `compactions > 0`, not
     `context_overflows`. The persona files block must sit in
     `messages[0]`, which compaction keeps whole. A two-turn fixture
     reaches none of these, so at least one fixture runs with a
     fixture-sized `context_window` until the cut happens. Its security
     snapshot also records that a `rewrite` was written and that the cut
     landed on an assistant message, since an orphaned `tool_result` is a
     400 the rendered body only shows if a fixture gets there.

   A rendered body is blind to the security plumbing the host moves. A
   tool that loses `.from_outside()`, a dropped `Capabilities` field, or a
   taint seed that resets per turn all render identically, because none of
   them is in a request. So the harness takes a second snapshot beside the
   body, compared old path against new in the same way:
   - **the conversation's taint axes** after each turn;
   - **the run's jail root** (`RunContext`) and **which approver was
     installed**, so a moved workspace or the wrong approver fails the
     step rather than passing it;
   - **each dispatched tool's `Capabilities`** and its result's `external`
     flag;
   - **at least one fixture whose expected outcome is an interlock
     refusal** (an armed conversation calling a `Chosen` tool). A fixture
     that never trips the interlock cannot notice it has gone.

   "Identical" in §10 means both snapshots. A step meant to change bytes
   (the principal's wording, say) does it in its own PR, with the rendered
   diff shown.
3. **Beside, then instead.** The new host lands next to the old code. One
   surface switches at a time, behind the same routes. The old path is
   deleted only after the new one has run live (deployed with the
   `update` skill and used). How long counts as lived with is the owner's
   to set (§11).
4. **Tests move, never vanish.** The test attributes in §1's table move
   with their code. Each PR lists its test-count delta by name (a diff of
   `cargo test -- --list`), and a removed test needs its replacement
   named next to it. **The written invariants move with them.** Five
   `ARCHITECTURE.md` sections describe code that becomes core:
   §Interruption and steering, §Background jobs, §Incognito chat,
   §Personas and §Compaction. Each step updates the sections whose subject
   it moved, step 4 adds §The session engine, and CLAUDE.md's module map
   gains its row. CLAUDE.md makes a subsystem's section a precondition for
   changing it, so a section describing a deleted path is a wrong
   precondition, not just drift.
5. **Setup is not in the path.** Guided setup does not touch either chat
   stack: `git grep -l -E 'persona_chat|serve::chat|PersonaChats'` over
   `commands/setup_guided.rs`, `setup_chat.rs`, `setup_engine.rs`,
   `features.rs`, `mecha-core/src/onboarding.rs` and `feature.rs` finds
   nothing at `dc00a90b2`. One name misleads: `mecha-cli/src/setup.rs`
   (`prepare_carrying`, `persona_agent`) is the *agent builder* and does
   move into the engine. `mecha setup`, the features registry, the units
   and the config formats (§3) do not change. A new feature
   (`hyperstudy`) follows `ARCHITECTURE.md` §Adding a feature and adds a
   step, without altering an existing one.
6. **Routes and the page last.** `/api/chat` and `/api/persona-chat`
   stay as they are until every surface is on the host. One route set is
   a later step with the old routes kept as aliases, and the page's own
   rewrite is optional (O4).
7. **One PR per step, each revertible by its merge revert.**

## 10. Build order

Each step ships, deploys and is lived with before the next. "Identical"
means §9.2's two snapshots: the rendered bodies and the security snapshot.

0. **The inventory** (`PARITY.md`), reviewed by the owner, and the parity
   harness of §9.2: the exported recorder, the per-provider render and the
   security snapshot. Exit: the harness fails on a moved cache breakpoint
   and on a tool stripped of `.from_outside()`, each shown by a test that
   breaks it.
1. **The principal type.** `Principal` in core, recorded per session. The
   taint seed and the approver installed are derived from it, and its veto
   enters memory-write and corpus eligibility, each computed over all three
   axes (§2), with every existing surface passing `Owner`. Exit:
   requests and security snapshots identical, every existing test green,
   no `Deny` able to originate under a guest principal, a guest-labelled
   session refused memory writes and spoken confirmations, and an
   Owner × Persona × Incognito combination refused memory writes.
Steps 2–4 are written for O1's first answer, both web chats moving
together. Under the second (personas first), each of these steps moves the
persona chat alone, and the assistant chat repeats steps 2–4 after step 6.

2. **The job queue and its delivery** into core; both web chats on one
   instance. Exit: identical; the late-job tests of both files green
   against the one implementation.
3. **Steer, cancel, reorder, barge-in.** Exit: identical; one
   `BARGE_IN_TRIES`.
4. **The turn body, as `ConversationHost`.** Both web chats on it, routes
   unchanged. This is the big step and the one §9.3 exists for. Exit:
   identical; the persona and chat suites green; lived with for the
   owner's chosen period (§11) before the old bodies are deleted.
5. **Context providers and the Safety module.** The crisis sensor leaves
   `persona_chat.rs`. This is PERSONA-SERVING §8 step 2's prerequisite,
   done once. Exit: identical; the sensor's tests run against the module.
6. **One voice host.** Standalone voice's `Slot` map joins. Exit: both
   call kinds pass their existing tests, and the owner's phone check
   passes (an inventory row).
7. **The Seat profile and the HyperStudy door:** PERSONA-SERVING §8
   steps 2–5, now on the host.
8. **The rest onto the host:** TUI, `mecha chat`, Slack. Personas reach
   the TUI and incognito personas exist, both as combinations.
9. **Optional:** one route set, and the page on the Realtime protocol
   (O4).

PERSONA-SERVING §8 step 1 (the feature and the token) depends on none of
this and can land at any point.

## 11. Out of scope, and owed

**Deliberately out:** a guest-assistant profile, until it is designed as
its own sandboxed instance (S3); any public listener; runtime composition
of modules; merging persona memory with the learning store; personas on
Slack; a change to any config or store format (E3).

**Owed to the owner:**

- **O1 Scope.** Do the owner's assistant sessions move onto the host in
  steps 2–4 alongside personas, or do personas move first and the
  assistant follow? The duplication is between the two, so moving both
  is what removes it; moving personas first is the smaller first risk,
  but under §9.3 it means three turn bodies live at once for a while (the
  new host, the old persona body until deleted, and the assistant's), not
  two.
- **O2 Order against #724.** Build seats on the engine (step 7), or ship
  a seat on today's persona code and move it later? The recommendation is
  the engine first: a seat needs steps 4 and 5 either way.
- **O3 Settings pages.** One page per profile instance, or today's
  separate pages kept?
- **O4 The page's protocol.** Does the web page eventually speak the
  Realtime protocol too? Decided after step 7.
- **How long a step is lived with** before its old path is deleted (§9.3).
