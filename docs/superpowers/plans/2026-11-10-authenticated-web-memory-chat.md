# Bounded authenticated web memory chat: next-slice contract

Status: implemented and qualified as an explicit bounded prototype; no production
activation. Core/host/frontend regressions, full workspace suites, standalone
memory, required backends and actual HTTP/compiled-browser workflows passed.
See `docs/HISTORY.md` for source-grounded evidence and preserved failures; current
daily-use gaps are in `docs/HANDOFF.md`. The frozen contract remains below as the
design record. Todos/plans, graph MCP, voice/uploads and compaction are not enabled
in this first web-memory profile.
Basis: current working tree after the read-only graph CLI memory increment. This
slice is one usable owner-authenticated web conversation: opt in, send two text
turns, revise the objective, recall retained evidence, restart the server, resume.

## Frozen ingress and wire contract

| Attachment / marker | Native kind / owner source | Observation / recorder | Surface |
| --- | --- | --- | --- |
| Existing ordinary-chat-memory-v1 | Chat / CliChat | Existing observation-v1 / recorder-v1 | Local |
| Existing ordinary-chat-memory-v2 | Chat / CliChat | Existing observation-v1 / recorder-v1 | Selected graph |
| New ordinary-chat-memory-v3 | Web / AuthenticatedWeb | New ordinary-chat-observation-v2 / ordinary-chat-recorder-v2 | Local only |

V3 is not a superset or an automatic upgrade of either CLI protocol. Its private
attachment requires a strict `ingress` member with the single accepted pair
`session_kind: web`, `owner_source: authenticated_web`, and rejects graph metadata.
V1/V2 reject any ingress member. Preserve their encoded bytes and hash the original
authenticated attachment bytes; do not reserialize historical identity. Older
binaries refuse the new marker. The request, key, database and SQL schema remain
unchanged.

Use host-library methods `attach_authenticated_web_chat`,
`begin_authenticated_web_turn(..., expected_ordinal)` and `next_turn_ordinal`.
Keep existing CLI constructors and `begin_turn` CLI-only. A shared private helper
checks the closed ingress against attachment, native Meta, every context owner
and capture protocol. `from_version` remains CLI-only; a separate authenticated-web
converter emits the new sanitized owner variant. Exact observation/recorder pairs
reserve both labels, so mixed pairs cannot fall back to legacy admission. Test
every wrong kind/source/protocol combination, including mixed prior contexts.

The web route, not a serialized owner-source label, authenticates the owner. A
private non-deserializable route admission type is constructed only after the
existing owner and mutation guards. Task/voice/internal entry points cannot
obtain it. Retry and forget remain storage-only operations using the existing
sealed authority; neither needs live HTTP authentication or a new execution.

An opted-in send carries the expected turn ordinal from the server snapshot.
The server checks it while reserving that conversation's single writer, then
core rechecks the authenticated ledger before writing a begin record. A delayed
duplicate cannot execute after completion or restart. The ordinal is a rejection
precondition, never source authority or browser-selected persistent identity.

Use startup-only `serve --episodic-memory REQUEST_JSON`, required
`--memory-provider`, optional `--memory-model`, and repeated explicit
`--memory-tool` flags. Require dynamic goals and goal_context. These options
configure only the owned memory runtime; ordinary web following stays unchanged.
Expose only availability and limits to the browser. Open/resume requests may opt
into `memory: episodic`; sends carry text, a correlation request ID and
`memory_turn`. Strict memory request DTOs reject paths, keys, scopes, ingress,
protocol and tool metadata. The server's stored session mode routes later sends.

The server implementation should use a sibling host module for owned memory
runtime orchestration, rather than distributing memory authority through the
shared ordinary run loop. Core remains independent of HTTP and the CLI.
Use one shared session-key map with closed ordinary, memory and opening variants;
ordinary helpers must not create a second session under a memory key. Release a
blocked runtime and all tracked readers after finalization so host retry does not
require restarting the whole server. Keep its safe status, and require explicit
reauthenticated resume after repair instead of returning a blocked key as ready.

The memory snapshot contains `mode`, `phase`, `next_turn_ordinal`,
`blocked_reason`, a sanitized `last_report`, and limits. Phases are ready,
starting, running and blocked; no ordinal is offered unless continuation is
authenticated. A `type: memory` SSE event precedes execution's Done event on
finalization. Keep receipt identity and NotEvaluated separate from task success;
omit private paths and raw error chains. Authenticated resume reports a verified
next ordinal but `last_report: null`; do not invent a historical receipt or parse
private sidecars in the web host merely to fill that UI field. The frontend uses RAM and existing
authenticated history/navigation, never new localStorage or sessionStorage.

## Product boundary

- One explicitly configured encrypted SessionMemory request/profile at server
  startup; the existing SQLCipher store, key policy, scopes, authority directory,
  transcript prefix seals and recovery machinery remain the only memory store.
- The owner opts a new, recorded assistant web chat into that configured profile.
  The browser may select the server's opt-in, never submit a manifest/key/database
  path, account, source scope, tool schema, or attachment authority.
- Initial tool surface: goal_context plus explicitly selected fs_read/fs_list and,
  if the host profile enables it, fs_write inside the conversation workspace.
  Require goal_context. No graph MCP in this slice: the CLI graph adapter remains
  independent and can be reused later with an exact per-conversation launch.
- Typed text input only. Existing ordinary web, task, voice, persona and incognito
  behavior and defaults remain unchanged. The memory profile refuses those other
  entry points, images/uploads, grants, delegation, outbox, hooks, fallback,
  code, deferred jobs, legacy recall, automatic title inference and hidden board
  reads. Todo, formal planning and compaction remain outside this slice.
- Every completion distinguishes executed, retained, withheld and NotEvaluated.
  Successful tool completion is not semantic success. Withholding is visible on
  the page and prevents the next turn in that conversation.
- While busy, use a visible 409/busy response initially, preserving the unsent
  browser draft. Do not steer into the active run and do not silently enqueue.
  A later queued-turn UX can reuse the existing waiting queue only after each
  predecessor has retained successfully.
- Stop cancels the active run, records an unavailable/interrupted memory state,
  and prevents unsafe continuation. It never rewrites a previously sealed prefix,
  automatically replays tools, or claims an incomplete run was retained. A
  completed sealed capture may use existing host-only retry; an unsealed stopped
  turn requires a fresh chat under the current recovery contract.

## Existing seams and why they matter

`mecha-cli/src/commands/serve/mod.rs`:

- `owner_guard` requires the configured Tailscale owner login for every route;
  `request_verified` requires `x-mecha-request: 1` and rejects foreign
  `sec-fetch-site` on mutations. New opt-in/create/send/resume routes remain
  inside this router and guard; no loopback exemption or new authentication store.
- Authenticated HTTP ingress is the authority. `OwnerSource::AuthenticatedWeb`
  in a native record describes ingress; it must not independently authorize an
  internal voice/task caller to create a memory turn.

`mecha-cli/src/commands/serve/chat.rs`:

- `ChatState` owns a shared `Arc<Follower>`, a `Mutex<HashMap<String, WebSession>>`,
  session/work directories, events and task tracking. `WebSession.conversation`
  is moved out while its run executes, then returned. Preserve that single-writer
  pattern for a memory runtime/lease, without holding the map lock across I/O.
- `ensure_session_as` currently treats `Session::load` failure as a request for a
  fresh session. An explicit memory resume must never take that fallback. Its
  missing/invalid marker, ledger, source, receipt or lease is a refusal.
- `resume` currently loads through the ordinary loader, unarchives and appends
  Config. Memory resume needs a separate authenticated path: acquire the core
  attachment lease and validate the retained chain/surface first; do not mutate
  transcript/archive or observe a provider on refusal.
- `send_text` calls `Follower::enter` outside the sessions lock, then calls
  `begin_turn`. A memory send must validate attachment/continuation before
  provider/router work. Reserve one session start under the lock, do bounded
  preparation outside it, then prove the same reservation still owns the session.
- `begin_turn` creates `OwnerInput::new(AuthenticatedWeb, text)`, records config
  and owner text, and clones a per-run ToolCtx. Its ordinary behavior also folds
  pending user messages into Rewrite records. The restricted memory branch must
  use the core turn handle and exact owner/native ordering instead.
- The run task calls `Agent::run_in`, `Session::record_run`, `record_outcome` and
  appends Taint. Ordinary successful `record_run` errors are currently ignored;
  the memory branch must propagate all recording failures and withhold capture.
  Ordinary provider failures roll back with `Conversation::roll_back_failed_turn`
  and native Rewrite; that behavior cannot be applied to authenticated memory.
- The same run task calls `read_board_for_brief`, touches outbox/workflow state,
  lands deferred results, and schedules title inference/Record::Title after a
  run. These paths must be explicitly bypassed for the memory profile. Current
  core `ChatMemory::continuation` accepts only validated Config records after a
  retained seal, so an automatic Title record would block the next turn.
- `WireEvent`/session snapshot/SSE provide the existing place for per-turn memory
  status. UI send/busy/Stop state must follow final recording/capture outcome,
  not merely receipt of the assistant's final token.

`mecha-cli/src/follow.rs`:

- `Bound.agent` is an `Arc<Agent>` shared by all ordinary conversations; `Follower`
  reloads config each turn and can rebuild through `setup::prepare`. Installing
  `ChatMemory::goal_memory(turn)` into that registry would race conversation A's
  tracked authority with B's and could leak or misattribute evidence.
- Choose an owned restricted `setup::Prepared` per opted-in WebSession, moved with
  its Conversation and ChatMemory during a run. Replace its goal_context tool
  only while that conversation has exclusive ownership, using `context_tool`
  before Config and `goal_memory(turn)` after begin_turn. Do not add Tool trait
  methods or a global key-to-reader routing table.
- Keep the selected provider/runtime fixed for a resident memory conversation;
  no Follower rebuild or hidden MCP attachment. Preserve router holds for the
  duration of each run. A detected incompatible model/config change is a visible
  refusal before inference, not an automatic authority rebind. Ordinary chats
  retain existing following behavior.

`mecha-cli/src/commands/memory_profile.rs` and `setup.rs`:

- Factor pure host/workspace/profile validation from CLI terminal/argv checks.
  Web supplies its canonical conversation workspace and a server-owned request.
  Keep the existing local-only supported tool list and restrictive options.
- Reuse `SessionMemory::validate_request_path`, `load_request`, `open` and
  `validate_surface`; prepare an owned restricted runtime with no MCP.
  `finish_prepared_tools` is the common provider/build seam after authorization.
  The web runtime must not inherit ordinary web's ask_user or shared tool pool.
- Do not weaken all web defaults or add memory fields to general Tool/Agent
  interfaces. If durable config is chosen for the host profile, Config and
  ConfigLayer must change together; startup-only explicit options are the chosen
  first binding, using the memory-prefixed flags above.

## Core changes actually required

Current core admission is intentionally CLI-only:

- `memory_session::chat::SessionMemory::attach_chat_protocol` requires
  `SessionKind::Chat` at initial creation.
- `ChatMemory::begin_turn` requires `OwnerSource::CliChat`.
- `memory::chat_observation::ChatOwnerSource` has only CliChat and
  `ChatContextVersion::from_version` refuses other runtime owner sources.

Introduce a narrowly versioned, host-owned ingress binding for authenticated web
creation/turns rather than deleting these checks or accepting all OwnerSource
variants. Bind the permitted SessionKind and owner source in private attachment
admission, and validate them again at turn capture and source reauthorization.
The frozen attachment/observation matrix above makes this boundary explicit:
CLI V1/V2 bytes, reports, source guards and marker decoding must remain unchanged,
and older binaries must refuse new authority. Keep stored DTOs runtime-independent
so the standalone memory harness continues compiling. Reuse existing exclusive
lease, ChatMemoryTurn identity, attempt tracking, exact native provenance and
input matching, dependency roots, seal/retry/forget paths and SQL schema.

A per-conversation owned runtime avoids any core Agent registry override or Tool
trait refactor. No new memory database or alternative retrieval implementation is
needed. A shared-reader dispatch abstraction is unnecessary for this first slice.

## Implementation sequence: one usable feature

1. Implement the frozen web ingress/version contract. Add authored native tests that
   reject the new source on old CLI protocols and reject non-authenticated ingress,
   then add the new host-only attachment/turn path and sanitized evidence variant.
2. Add explicit host-profile configuration and owner-authenticated opt-in creation
   and resume. Store per-conversation ChatMemory plus owned restricted Prepared;
   acquire/validate authority before model startup or native transcript mutation.
3. Wire one text turn through core begin/reader/run/native writes/finish. Surface
   retained/withheld/unavailable status through existing events/snapshot. Refuse
   busy sends visibly; disable steering, unsupported mutation and side paths for
   this profile. Stop preserves audit history and blocks unsafe continuation.
4. Add the minimal UI opt-in/status/busy/Stop/resume flow, then run actual browser
   controls against the authored HTTP provider with isolated storage. Do not claim
   completion from component tests, HTTP status or compilation alone.
5. Qualify old and new core/CLI/web behavior, standalone memory, feature-off marker
   refusal, lint/format and complete workspace suites. Document encryption limits:
   SQLCipher does not encrypt the external JSONL, private seals, or browser state.

## Required acceptance tests

- Router-level unauthorized owner, missing mutation header and cross-site requests
  produce no memory files, transcript changes, subprocesses or provider requests.
  Arbitrary manifest/path/key/account/scope fields are rejected, never honored.
- Actual HTTP/SSE and browser flow: create opted-in chat; file observation; revised
  objective; exact retained recall; server restart; authenticated resume. Verify
  native owner source, decisions, invocation/provenance/result/source pointers,
  dependency chain and per-turn NotEvaluated receipt; no fabricated criterion.
- Two simultaneously active memory chats never exchange turn handles, provisional
  recall attempts, ToolCtx/workspace state or registry entries. Historical recall
  already authorized in their shared owner store remains legitimate; test wrong
  attribution separately from intentionally shared historical access.
- Same chat through two tabs, duplicate request, CLI/web writer conflict and an
  aborted startup reservation never produce two native writers or two executions.
  Busy rejection is visible and keeps the user's unsent text.
- Missing opt-in, altered attachment/source/marker/scope/workspace, wrong profile
  and stale runtime binding refuse before provider work or transcript mutation.
  Explicit resume never silently creates a fresh conversation.
- Disconnect, cancellation/Stop, process death, provider failure, append failure,
  clipped recall and oversized result visibly withhold/block continuation without
  unsafe Rewrite or automatic tool replay. Sealed retry/forget works without a
  live provider and after historical workspace removal where already supported.
- Instrument hidden board reads, title generation, workflow/outbox operations,
  deferred jobs and Follower rebuilds: zero for this profile. Unsupported clear,
  grants, uploads, incognito/persona/task/voice entry points refuse before mutation.
- Ordinary web, persona, incognito, CLI V1/V2 and default-off/feature-off behavior
  stay unchanged. Run standalone harness and full required suites, not only the
  authored happy-path fixture.
