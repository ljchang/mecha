# Bounded Read-only Graph in CLI Memory Chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Two isolated implementation workers own the disjoint files below; root integrates, reviews and publishes after current prerequisites qualify.

**Goal:** An explicitly opted-in encrypted CLI chat can read a selected mecha-graph MCP server, retain bounded historical observations, revise its objective, recall the same source, and resume without widening its tool authority.

**Architecture:** Reuse the current chat lease/ledger/seals, MCP transport, ordinary tool dispatch and native `Message::tool_provenance`. Add a narrow selected-server MCP inventory and a versioned immutable chat surface binding; do not add MCP knowledge to the agent loop or change the general Tool registry architecture.

**Tech Stack:** Existing Rust workspace, feature `memory-prototype`, serde/sha2, stdio MCP, authored Python MCP and loopback provider fixtures. No new dependency or database engine.

**Spec:** The complete concrete contract below; repository basis `docs/MECHA-MEMORY-DESIGN.md` §31.25 and ARCHITECTURE Experimental memory / MCP / trifecta sections. Implementation starts only after the daily-use prerequisites qualify. The current product does not yet support this graph profile.

## Global constraints

- One usable CLI-chat increment, not separate prerequisite-only releases. Default ordinary/chat and existing local-only memory protocols remain unchanged.
- Only existing local fs_read/fs_write/fs_list/goal_context plus selected graph remote names `kg_search`, `kg_entity`, `kg_task_list`. No todo, formal planning, graph writes, migration, source synchronization, new store, web memory or compaction support.
- Explicit owner global-config server selection; no project config, discovery of arbitrary stores, model-selected server, prefix/suffix inference, extra grants, hooks, children, code, outbox, fallback or inbound delivery.
- Preserve path jail, sandbox preflight/environment clearing, taint, original source authorization, full-input recall matching, complete-seal retry and NotEvaluated. Graph `ReadThatRecords` knowingly permits retrieval telemetry; it does not mean zero writes or binary attestation.
- Authored fixtures first. No private graph calls, production activation or merge. Qualification owns Cargo/actual CLI checks under the current lane constraints.

## Review focus

- A wrong server exposes the same unprefixed names, or tools/list repeats names over pages: admission refuses ambiguity rather than matching strings.
- Selected config or live schema/effect changes at resume: config mismatch refuses before child spawn; live mismatch refuses before provider preflight/inference or transcript mutation.
- A guard refuses before MCP execution: native provenance remains false; server output and transport errors remain true; success is not inferred from a requested action.
- A result is clipped/wrapped, empty, nontext or too large: retain only exact delivered bounded text with honest receipt, or visibly withhold; never certify an omitted original result as complete.
- Retry/forget occurs while the server is absent or now configured differently: sealed source authorization remains independent of live server execution; recovery never starts MCP/provider.

## Product contract and owner invocation

Add chat-only `--memory-graph-server NAME` (requires `--episodic-memory`, `--dynamic-goals`; conflicts with `--no-mcp`). NAME is an exact enabled owner-global `[[mcp]]` name. Keep explicit `--tool` choices. Require goal_context, allow existing fs tools, and admit only chosen local names corresponding to the three remote graph names. At least one graph tool must be selected. Prefix mapping comes from the selected config/runtime descriptor, never suffix matching. `run --episodic-memory` remains local-only.

Example intent: `mecha chat --episodic-memory REQUEST --dynamic-goals --memory-graph-server graph --tool goal_context --tool fs_read --tool graph__kg_search ...existing restrictive flags...`. Exact server names/prefix policy remain owner-specific, not hardcoded as `graph`.

The product promises a native-session snapshot of what an external graph read returned at that time. It does not promise canonical entity revision identity or propagation of graph-side deletion; those need a later source adapter. Existing transcript mutation/forget/scope withdrawal still invalidates memory evidence. Result and history bounds remain current; no silent truncation or cap increases.

## Proposed implementation interfaces

### A. MCP runtime inventory — worker A, in `mecha-core/src/mcp.rs`

Introduce module-owned DTOs, not Tool trait methods:

- `McpToolDescriptor { server_name: String, local_name: String, remote_name: String, schema_digest: String, description_digest: String, read_only: bool, capabilities: Capabilities, effect: Option<Effect>, deferred: bool, fixed_workspace: PathBuf }`.
- `ListedMcpTool { tool: Arc<dyn Tool>, descriptor: McpToolDescriptor }` with private fields and read-only accessors, constructed only beside McpTool creation. This prevents rebuilding origin by parsing a tool name or downcasting a trait object.
- `McpClient::list_tools_with_descriptors(self: &Arc<Self>) -> Result<Vec<ListedMcpTool>>` uses the same conversion as `list_tools`, but strictly refuses incomplete pagination, duplicate remote/local names and malformed selected schema. Existing `list_tools` behavior remains compatible; share parsing, do not copy capability/effect rules.
- `connect_selected_graph(config: &McpServerConfig, sandbox: &Sandbox, workspace: &Path, selected_remote_names: &BTreeSet<String>) -> Result<SelectedGraphTools>` where `SelectedGraphTools` owns client, selected `ListedMcpTool`s, and an effective launch digest. It connects exactly one server and fails if any required name is unavailable. Other advertised graph tools may exist but are never registered.

Admission predicate is exact: operator-origin enabled Graph config; forced untrusted_input; remote name in the three-name set; read_only=true; private_data=true; untrusted_input=true; egress=None; destructive=false; effect=Some(ReadThatRecords); deferred=false; fixed workspace equals canonical execution workspace. Reject undeclared or merely server-asserted read-only effects. Existing `FirstParty::of` is operator-config classification, not executable attestation. Do not invent a pin guarantee for an arbitrary PATH executable.

`launch_digest` hashes a canonical versioned launch descriptor: server name, effective command/args, explicit env plus effective permitted passthrough values, derived owner_zone, prefix mode, disabled tools, capability/effect overrides, trust-result setting, sandbox/backend/network configuration and canonical fixed workspace. Reuse actual MCP environment construction so the fingerprint and spawned environment agree. Store only its digest in private memory authority; never include raw env/credentials in transcript, report or log. State that this binds launch configuration, not executable bytes or graph contents. Resolve command consistently and fingerprint the resolved launch path; replacement-at-same-path remains outside the attestation claim.

Selected graph delivery is text-only: on nontext/invalid content, return a bounded external error explaining unsupported response, not the generic `[image content omitted]` success placeholder. Preserve wire/transport external classification. Do not alter ordinary MCP behavior globally. Oversized delivered text may still cause current capture withholding; do not silently clip it to gain retention. Respect existing dispatch truncation/wrapping: capture authenticates the actual delivered native result, not raw server bytes.

### B. Chat memory surface authority — worker B, new `mecha-core/src/memory_session/surface.rs`

Runtime-independent serializable DTOs (no import from `mcp` or Tool in stored types):

- `GraphToolBinding { local_name: String, remote_name: String, schema_digest: String, description_digest: String, read_only: bool, private_data: bool, untrusted_input: bool, egress: String, destructive: bool, effect: String, deferred: bool, fixed_workspace: PathBuf }`.
- `ChatGraphSurface { version: String, server_name: String, launch_digest: String, local_tools: Vec<String>, graph_tools: Vec<GraphToolBinding> }` with `version="ordinary-chat-graph-surface-v1"`, sorted unique tool lists; fixed allowed strings are egress="none" and effect="read_that_records". All bounds/duplicates/digests validated. No defaulted unknown authority fields.
- `ChatGraphSurface::validate(&self, workspace: &Path) -> Result<()>`, `tool_names(&self) -> Vec<String>`, `contains_tool(&self, name: &str) -> bool`, `digest(&self) -> Result<String>`.
- `SessionMemory::attach_chat_with_graph(self, session: &Session, mode: ChatAttachMode, graph: Option<ChatGraphSurface>) -> Result<ChatMemory>`: Create requires Some(final live-admitted surface); Resume authenticates existing attachment without live input, then uses methods below. Existing `attach_chat` remains the V1 local-only door and refuses V2 rather than upgrading it silently.
- `ChatMemory::graph_surface(&self) -> Result<Option<ChatGraphSurface>>` authenticates attachment and returns its stored surface for preflight comparison.
- `ChatMemory::validate_graph_surface(&self, actual: &ChatGraphSurface) -> Result<()>` exact equality with authenticated stored binding; called before provider preparation and before admitting each turn if runtime/config can change.

DTO fields above describe a frozen surface, not standalone authorization: only private attachment admission makes them authoritative. Host can construct them from inventory; transcript/model JSON cannot select or replace them.

### Required version decision

Use a new attachment/marker protocol `ordinary-chat-memory-v2` only for graph-enabled chats, with required `graph_surface` in its attachment payload. Keep V1 attachment bytes, report protocol and local-only behavior unchanged. Add feature-independent recognition of both marker protocols in `session.rs`; ordinary `Session::load` refuses both. Older binaries already fail closed on unknown/malformed dynamic markers. Newer source guards/recovery dispatch V1 or V2 explicitly, not a permissive unknown-version fallback.

Represent private attachment reading as explicit V1/V2 wire structs (or raw-tag dispatch with equivalent deny_unknown_fields); validate before normalizing for internal use. Hash the original stored attachment bytes; never reserialize a V1 into V2 to compute identity. Turns already bind attachment digest and seals bind turns, so the frozen surface becomes part of authority transitively without duplicating it in every turn. Keep ordinary-chat-observation-v1 and reader payload shape unchanged: bounded ToolResult snapshot, ActionInvocation and receipt already express the evidence. No SQL schema migration. Report protocol must come from authenticated attachment version, including retry. Unknown versions refuse. A future semantic change to observation meaning would require its own protocol decision.

### C. Thin CLI/setup bridge — root integration after A/B APIs land

- `commands/chat.rs::Args`: add only chat flag above; `MemoryPreparation` carries explicit optional graph selection alongside Fresh/Resumed core state.
- `commands/memory_profile.rs`: keep `open_profile` for one-shot/V1. Add `open_chat_profile(global: &mut GlobalOpts, request: &Path, graph_server: Option<&str>) -> Result<ChatProfilePreparation>`; validate config, names, flags and authority before external startup. `ChatProfilePreparation` contains opened memory and one validated operator-selected MCP config, never all configs.
- Prefer a narrow `setup::prepare_memory_chat(opts, selection, expected_surface)` entry that uses the existing tool-building path with an explicit selected-MCP override, validates runtime inventory, then calls existing provider preflight/build. Keep general `prepare` unchanged; do not put memory/server matching into Agent. Return `(Prepared, ChatGraphSurface)` for graph mode. Worker A can expose `PreparedTools` inventory needed here. If an internal setup options struct is necessary, it is caller-owned and not a new TOML Config field.
- For Resume: authenticate source/ledger and hold lease first; compare selected launch config with authenticated stored binding before spawning any child; then connect only selected server and compare actual descriptor surface before provider preflight. Missing graph flag or V1/V2 mismatch refuses. Do not rebind the stored surface to current config automatically.
- For Create: validate request/workspace before child startup; prepare/admit runtime surface; create native Session; attach V2 before Config; install metadata-only context_tool, require exact final registry; begin typed owner turn and install tracked goal_memory as currently. Do not let setup add builtin legacy recall or hidden graph brief reads.
- No provider follow/rebuild in this slice. Current chat profile's provider/model/fallback rules stay. Configuration edits during a resident session do not silently rebuild its already-frozen clients; resume revalidates them.

## Capture policy — worker B

V1 retains its existing accepted wire and tests. V2 uses authenticated graph_surface for Config and ToolUse membership instead of broadening the global local-only `recording::surface`. Only chat capture expands; one-shot remains unchanged.

For V2 every message with tool results must have a provenance key exactly once for every result call ID and no unrelated entries; messages without results cannot smuggle such entries. Require matching unique preceding ToolUse and admitted tool; preserve call ID/source pointer/receipt. The Agent already initializes provenance=false for guard refusals and overwrites it with ToolOutput.external on execution; no new general dispatch metadata is needed. Never derive external from tool name or text. A graph result may be false for a harness refusal; graph origin alone is not reason to reject that valid denial. In this direct/no-outbox profile, a graph result with external=false must have is_error=true; refuse a purported successful clean graph result. The false bit attests host classification, not a reason inferred from its prose. All true values require recorded cumulative untrusted taint; graph delivery also stays private. Existing exact goal_context tracking remains independent and checks its external bit/results.

Store exact native delivered ToolResult content as the original passage. Calls with failure receipts are observations of a failed/refused result, not proof of graph facts. Full-input digest behavior and omitted argument handling are unchanged. Missing/unknown provenance or non-admitted call withholds the complete turn. Source mutation, dependency withdrawal and sealed retry use existing authority. Do not relabel a result as an owner instruction or confirmed criterion.

## Parallel implementation and qualification

### Worker A — selected MCP runtime and fail-closed setup seam

**Owns:** `mecha-core/src/mcp.rs`, narrow supporting MCP tests/fixtures; `mecha-cli/src/setup.rs` selected-MCP plumbing only. Does not edit memory_session, chat command, main, session parser or shared docs.

- [ ] Add failing authored MCP tests for selection isolation (unrelated server leaves no spawn marker), same suffix/wrong server, prefixed/unprefixed mappings, duplicate names/pages, missing required tool, schema/effect/capability changes, deferred/open-world/destructive/undeclared refusal, nontext refusal, external wire/transport errors and sandbox/env behavior.
- [ ] Qualification records behavioral RED with compiling tests before production implementation.
- [ ] Implement inventory + selected connection and the narrow setup seam. Preserve ordinary connect_all behavior and BTreeMap registry ordering.
- [ ] Qualify focused MCP/setup tests, then produce exact patch/postimage manifest and report. No live graph execution.

### Worker B — V2 bound surface, capture and recovery

**Owns:** new `memory_session/surface.rs`; `memory_session.rs`, `memory_session/chat.rs`, `chat_recording.rs`, `authority.rs` only as required; feature-independent marker validation in `session.rs`; ordinary chat lifecycle tests. Does not edit MCP/setup/CLI commands/shared docs.

- [ ] Add failing V2 authored native lifecycle tests for bound surface, V1 unchanged bytes, unknown-version/missing/changed/duplicate descriptor refusal, source membership, matching/missing/extra provenance, external taint, guard refusal false, exact snapshot/pointer/digest/receipt and NotEvaluated.
- [ ] Add failed-admission sealed retry with server absent; retry/forget must not require runtime inventory or execute anything. Test changed source/withdrawal, conflicting lease, altered surface digest, restart and standalone memory/feature-off marker compatibility.
- [ ] Qualification records compiling behavioral RED, then implement immutable versioned attachment/capture path and public APIs above.
- [ ] Qualify lifecycle/reader/authority tests, provide exact patch/postimage manifest and report.

The workers are independent because A exposes runtime inventory without importing B's DTO and B consumes authored DTO/native transcripts without connecting MCP. Root performs the one explicit inventory→surface conversion and CLI wiring. Neither worker changes the other's interfaces opportunistically; send a concrete contract change first.

### Root — integrate one usable CLI workflow

**Owns:** `commands/chat.rs`, `commands/memory_profile.rs`, any main dispatch preparation changes, inventory→DTO conversion, actual CLI fixture script, shared docs/handoff/changelog/site updates. Root resolves small setup seam adjustments with worker A rather than racing the same file.

- [ ] Review both patches against frozen interfaces and integrate exact hashes only after current prerequisite qualification finishes.
- [ ] Add actual CLI PTY fixture: two normal turns with file read + bounded graph search/entity/task-list, objective revision, exact graph snapshot recall, quit/restart/resume with same authority. Verify native invocation/result provenance, immutable surface binding, taint and per-turn NotEvaluated receipts.
- [ ] Negative actual CLI starts: absent flag, wrong server, changed launch config, changed schema/capabilities/effect, unrelated server not spawned, missing required tool; assert zero model requests and no transcript mutation for refused Resume. Test large results visibly withhold and block next turn; failed MCP call retains external failed receipt when otherwise admissible.
- [ ] Use existing dispatch-level injection/Chosen-egress regression with graph fixture content across turn and resume; the product profile itself does not gain a send tool just to test blocking. Verify harness refusal remains nonexternal.
- [ ] Full qualification: fmt; strict all-target/all-feature Clippy; required default and memory workspace checks; focused MCP/chat/reader/authority tests; standalone memory harness; exact-head CI/review and privacy checks. Run all required checks under the assigned qualifier, not competing remote Cargo jobs.
- [ ] Update existing docs/handoff only from checked source/results: graph snapshots versus live revisions, read-only domain vs retrieval telemetry, limits, default-off, retry/no duplicate execution. No semantic-benefit or production-ready claim from fixture success.

## Known boundaries and self-review result

This plan deliberately does not guarantee executable attestation, same-path binary replacement detection, graph-side deletion propagation, unlimited output/history, compaction, or arbitrary tools. Selected configured server is an owner trust decision; result content remains untrusted. Frozen launch/schema/effects prevent accidental authority drift across attachment/resume within that boundary.

The new V2 authority, runtime inventory and CLI integration are one product increment; the two workers' testable pieces are not separate advertised releases. All five review-focus cases map to A/B/root tests above. Existing recovery never depends on a currently running graph server. The interfaces and acceptance gates have been reviewed against the current seams; implementation and runtime qualification remain pending.
