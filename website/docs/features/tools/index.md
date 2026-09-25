---
title: Tools and MCP
sidebar_position: 1
description: The Tool trait, the registry, approval, the built-in tools, and the stdio MCP client with its environment allowlist.
---

# Tools and MCP

:::tip[In this section]

[Mail and calendar](/docs/features/tools/mail) ·
[Documents](/docs/features/tools/documents) ·
[Web search](/docs/features/tools/web-search). Tools that put something in
front of other people, [publishing](/docs/features/public-surface/publishing)
and [polls](/docs/features/public-surface/polls), are documented under
[the public surface](/docs/features/public-surface).

:::

A tool is a name, a description, a JSON Schema, and an async function. The
registry holds them; MCP servers and native Rust functions both land there as
the same trait object, so **the agent loop never learns where a tool came
from**. That is the invariant worth protecting — both the tool and the provider
are trait objects, and if the loop starts matching on either, something has
leaked.

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;

    /// Read-only tools skip the approval gate and are safe to run in parallel.
    fn read_only(&self) -> bool { false }

    /// Declared risk surface.
    fn capabilities(&self) -> Capabilities { Capabilities::default() }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput>;
}
```

## Expected failures are results, not errors

```rust
// The model can recover from this.
Ok(ToolOutput::err("cannot read notes/missing.md: No such file or directory"))

// Reserve this for what the model cannot route around.
Err(anyhow!("..."))
```

`ToolOutput { is_error: true }` reaches the model as a `tool_result` with
`is_error` set, so it can try a different path, a different tool, or tell you
what is missing. `Err` propagates out of the loop and ends the run.

Almost everything is the first kind. `fs_read` on a missing file, `fs_edit`
whose `old` string appears zero or five times, a shell command that exits
non-zero, an MCP server whose transport died — all of these come back as
recoverable results. `fs_edit` refuses an ambiguous match outright rather than
guessing, because silently editing the wrong line is worse than a retry:

```
`old` appears 3 times; include more surrounding context to make it unique
```

A result also says whether it **actually came from outside the machine**,
which is not the same as the tool's declared capability: a refusal from
mecha's own guard is not third-party content, and labelling it so makes the
model invent explanations for its own harness. Any tool that reaches the
network must mark its output as external — see
[`untrusted_input` versus `external`](/docs/features/security#untrusted_input-versus-external).

## Capabilities

Four axes, three of which are the lethal trifecta:

```rust
pub struct Capabilities {
    pub private_data: bool,      // returns data the user considers private
    pub untrusted_input: bool,   // returns content a third party can influence
    pub egress: Egress,          // whether data leaves — and who picks where
    pub destructive: bool,       // may destroy or overwrite data
}

pub enum Egress {
    None,    // nothing leaves
    Blind,   // it leaves, but only to a destination your config fixed
    Chosen,  // the model names the recipient: a url, a `to`, a channel
}
```

The loop tracks which of these have entered the conversation and refuses any
**`Egress::Chosen`** tool once both private and untrusted are present — an
injection can only be *directed* somewhere the attacker picks. A `Blind` tool
such as `web_search` is left alone by that interlock and refused by
`block_sends_after_private` instead. Full treatment in
[Security](/docs/features/security).

`Capabilities::union` is the only combining operation, and it only ever widens.
Letting config *narrow* a tool's declared capabilities would disarm the
interlock on the strength of a claim nothing enforces — the same mistake as a
sandbox that silently degrades, and it would make the cheapest configuration
the most dangerous one.

## Built-in tools

No server required — these are ordinary Rust functions, registered on every
run unless `[tools]` or `--tool` narrows them away.

| Tool | `read_only` | Declares |
|---|---|---|
| `fs_read` | yes | private |
| `fs_list` | yes | private |
| `fs_write` | no | destructive |
| `fs_edit` | no | destructive |
| `shell` | no | private, destructive; `chosen` send unless a sandbox takes the network away |
| `http_fetch` | yes | untrusted, `chosen` send |
| `todo` | yes | nothing |
| `goal_context` | yes | private — reads the confirmed goal and its learned lessons; see [Plan steps](/docs/features/appraisal/plan-steps) |

These come with a feature, and exist only when it does:

| Tool | Present when | Declares |
|---|---|---|
| `web_search`, `web_open` | a `[[search]]` backend is configured — see [Web search](/docs/features/tools/web-search) | untrusted, `blind` send (`web_search` is `chosen` if no blind backend is configured) |
| `image_generate` | `[image]` is configured — see [Image generation](/docs/features/tools/image-generation) | nothing; read-only (it only adds new files under `images/`); the server must be on this machine |
| `compact` | the provider has a compaction threshold (`context_window` or `compact_at_tokens`); `--no-compact-tool` withholds it — see [Compaction](/docs/features/models/compaction) | nothing |
| `recall` | the session can hold its history: `chat`, the TUI, a resumed `run` — see [Sessions and replay](/docs/features/memory/sessions-and-replay#recall-the-record-is-searchable) | nothing — what it returns already entered the conversation |
| `skill` | at least one skill is installed and `--no-skills` was not passed — see [Skills](/docs/features/learning/skills) | nothing |
| `ask_user` | a front end with someone to ask or a place to park the question: the TUI, web chat, a delegated task, `mecha questions answer` — see [Delegated tasks](/docs/features/automation/tasks#parked-questions) | nothing |
| `message_send` | `[messages] enabled` and not `--no-messages` — writes to another of this machine's agents; see [Messages between agents](/docs/features/automation/messages) | nothing; taint travels with the message |
| `show_file` | the TUI — it works only while the session is mirrored to a Slack thread; see [Slack](/docs/features/interfaces/slack#remote-control-one-session-two-places) | private |
| a subagent profile | one `[[subagent]]` entry each — see [Subagents](#subagents) | derived from the child's tools |

`mecha tools --json` prints what a given run would actually get.

Two of these look wrong until you read the reasoning.

**`http_fetch` is `read_only` but is still a `chosen`-egress sink.** It is
read-only with respect to *your* data — it touches nothing on disk — but a GET
is an exfiltration channel, because the payload fits in the query string and
the `url` argument lets the model pick who reads it. `web_search` is the
contrast that makes the class worth having: same payload problem, no
destination argument, so it is `blind`.

**`shell` is not marked as an untrusted *source*.** Taint tracking cannot see
inside a command, so labelling it untrusted would arm the interlock on every
`ls`. The mitigation is the sandbox, not a label. What confinement *does*
narrow is egress: with no network there is no way out, so a confined shell
drops to `Egress::None` and stops being a trifecta sink. `private_data` stays true regardless — a
confined shell still reads the workspace, and `fs_read` reads the same files
under the same label. Narrowing it would mean `shell: cat secrets` sets no
taint where `fs_read: secrets` does, making the cheapest route around the
interlock the more dangerous tool.

The sandbox policy lives on the `Shell` tool itself rather than in `ToolCtx`,
because it decides the tool's *capabilities* and `capabilities()` has no
context to consult. The workspace still comes from the context at call time, so
a per-run jail — an eval case's private fixture copy — is what gets mounted.
See [Sandbox](/docs/features/security/sandbox).

`http_fetch` additionally refuses loopback, private, link-local (including
`169.254.169.254`) and CGNAT addresses, does not follow redirects, and pins the
connection to the addresses that passed the check so a TTL-0 DNS answer cannot
rebind between check and connect.

`web_search` is registered only when a `[[search]]` backend is configured, and
it is a *chain*: backends are tried in order and the first that answers wins, so
a rate-limited provider degrades to the next one rather than to nothing. Like
`http_fetch` it is an untrusted source — what comes back is whatever a
stranger published — and a send, because a query string is a way out. Unlike
`http_fetch` its send is `blind`: the query can only reach the backends you
configured, so the interlock leaves it alone in an armed conversation. See
[Web search](/docs/features/tools/web-search). With it comes `web_open`, which reads the page behind a result by the handle
`web_search` printed beside it. It follows redirects, vetting every hop the
way `http_fetch` vets its one, and it keeps working in a conversation where
`http_fetch` is refused, because the model picks a result instead of writing
a URL.

`todo` is planning as a tool rather than as a mode: a list the model rewrites as
it goes stays honest where a plan produced up front goes stale on the first
surprise, and the current state is echoed back in every tool result so the model
re-reads its own plan without anyone re-prompting it.

`ask_user` can ask a present owner, or park a delegated task's question for
`mecha questions answer` to resume later. A generic batch or trigger does not
get a blocking terminal question.

Which of the built-ins are registered is config, via `[tools] enabled` /
`disabled`. `--tool` on the command line narrows further, and reaches
`web_search`, `compact` and `message_send` as well.

## A tool's own state can cross a compaction

A compaction summarises old messages away, and some tools keep their state
*in* messages — the `todo` list reached the model through the echo in its last
result. So a tool may hand its own state to the compaction to be carried
across **verbatim**, read fresh at the moment of compaction, with exactly one
copy surviving however many compactions follow. Your `todo` list and the
[skills](/docs/features/learning/skills) you loaded are still there after a
long run compacts. See [Compaction](/docs/features/models/compaction).

## The path jail

Every model-supplied path goes through `ToolCtx::resolve` before anything
touches the filesystem. Never call `fs::*` on a raw path from tool input.

```rust
let path = ctx.resolve(arg_str(&input, "path")?)?;
```

`resolve` joins relative paths against the workspace, canonicalizes the nearest
*existing* ancestor (the file may not exist yet, for a write), re-appends the
rest, and then proves containment. `..`, symlinks, and absolute paths outside
the root are all checked after canonicalization, not before.

There is exactly one sanctioned exception: the per-context spill directory,
where oversized tool output is saved in full. The truncation marker tells the
model to read the rest from that path, so `fs_read` has to be able to follow
it; its contents are the context's own tool results, so nothing new becomes
reachable. A re-rooted context gets a fresh spill directory, because two eval
cases sharing one could read each other's output through it.

## Output budgets and spilling

`ToolCtx::output_budget_bytes` is the byte budget one *turn's* tool results
share, divided across the calls in the batch so one runaway tool cannot starve
its siblings — mecha executes a turn's calls concurrently, so they land
together. The old per-tool cap was 200 KB, roughly 50k tokens: not a cap so
much as a promise to overflow. Left unset in `[tools]`, the budget derives
from the provider's `context_window` — an eighth of the window in tokens,
~3 bytes each — because one turn's results must not leap the gap between the
compaction threshold and the window: a flat 24 KB of numeric data is bigger
than that gap at a 32k window, and a benchmark trial died on exactly that
jump.

An oversized result is written whole to the spill directory and its transcript
copy is cut, with a marker that **names the recovery**:

```
[truncated by the harness: showing the first 24000 of 91234 bytes; the rest
begins on line 812. The full output is saved at /tmp/mecha-spill-.../shell-t1-
9f2c1a04.txt — continue with fs_read {"path": "...", "offset": 812}, or search
it with grep.]
```

A truncation notice that only says "gone" leaves the model to conclude the rest
never existed. A failed spill degrades to a plain cut that admits the loss and
says to re-run the tool — losing the tail must never lose the run — and it
never promises a path that does not exist.

## The registry

Tools are always sent to the model in the same, sorted order, because **the
tool list is the very front of the cached prompt prefix** — reordering it
would invalidate the cache on every request. See
[Providers](/docs/features/models/providers).

A planning phase sends a shorter list — only the read-only tools — which
changes the front of the prefix and makes the next turn re-pay for it; that is
the price of the tools being genuinely absent rather than merely refused.

## Approval

```rust
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, tool: &dyn Tool, input: &Value) -> Decision;
}
```

`Decision::Deny(reason)` passes its reason to the model so it can pick another
approach. Approval is sequential — it may block on a human — while execution is
concurrent.

`ModeApprover` answers from the configured `PermissionMode` without asking
anyone: `allow` permits everything, `read-only` permits `read_only()` tools and
refuses the rest, and `ask` **denies**, because nothing is watching to answer
and the safe reading of a question nobody hears is no.

```
`fs_write` needs approval and this run is non-interactive (use --yes to allow)
```

The CLI supplies interactive approvers instead — a terminal prompt for `run`
and `chat`, a modal for the TUI.

Configured outbox actions pass the hook gate and then stage for owner review,
skipping the interlock and execution approval checks. For executing calls the order is **interlock → hook → approval rules →
approver**. A hook can narrow policy and never loosen security.

[`[[rule]]` entries](/docs/reference/configuration#rule-and-approval) distinguish
commands inside one tool: allow `git status`, require a fresh decision for
`git push`, or forbid a command. `prompt` refuses when nobody can answer, even
under `--yes`; `allow` cannot bypass read-only mode or the interlock.
Policy refusals use `Decision::Blocked` and are not mined as user corrections.
See [Hooks](/docs/features/security/hooks) and [The outbox](/docs/features/security/outbox).

## MCP

`mcp.rs` is a minimal MCP client over stdio, speaking JSON-RPC 2.0 line by line
to a child process and exposing whatever tools it advertises as ordinary `Tool`
implementations.

```toml
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"            # or an absolute path to the binary
prefix_tools = false                   # its kg_* tools carry their own namespace
env_passthrough = ["MECHA_GRAPH_DB"]
```

Every server is also handed your `[agent] timezone` as `MECHA_TZ`, so a
server that renders or resolves times needs no line of its own; an explicit
`MECHA_TZ` in its `env` still wins.

Two things about that `command` line. A bare name resolves against the
PATH of whatever started mecha, so a systemd unit needs `~/.cargo/bin` on
its `Environment=PATH` (the shipped units carry it); what a failed spawn
then does depends on the command — the front-ends and the corpus commands
that go through the shared setup (`serve`, `slack`, `triggers`, `mail
classify`, `frontdoor`, `validate`, `learn`) report it once on stderr and
carry on with the `kg_*` tools simply absent, while `distill`,
`corroborate`, `gossip` and `vet` connect directly and exit non-zero. And
this server is deliberately **not** confined: `sandbox = true` replaces
`PATH` with the system directories (so a bare name can never resolve
there), binds nothing under your home directory unless `[sandbox]
readable`/`writable` lists it, and the graph's store is
`~/.mecha-graph/graph.db`, read-write — confining it would need an absolute
`command`, its directory in `readable` and the store's in `writable`, which
is most of the sandbox given away for a server that runs as you anyway.

Tools are namespaced `<server>__<tool>` by default, so two servers can both
expose a `search`. A server whose tools already carry their own namespace —
[mecha-graph](https://github.com/ljchang/mecha-graph)'s `kg_*` family — can
set `prefix_tools = false` and register them under their raw names. That
setting is a promise of distinct names, and the promise is enforced: an
unprefixed tool that collides with anything already registered fails startup
loudly rather than shadowing it. Protocol version `2025-06-18`; each request
has a 120s timeout.

What to expect when a server misbehaves:

- **One broken server does not sink the session.** It is reported and
  skipped, and its tools are simply absent. (`mecha eval --mcp-file` is the
  exception: there a failure is fatal, because a case set graded against a
  partial tool surface measures nothing.)
- **A transport failure is a result, not an error.** `MCP call failed: ...`
  comes back to the model as a failed call it can route around, not a reason
  to abort the run.
- **A server's stderr is a log.** It never reaches your terminal directly;
  run with `MECHA_LOG=debug` to see it, tagged with the server's name.

### Annotations become capabilities

```rust
Capabilities {
    private_data: true,
    untrusted_input: hint("openWorldHint"),
    // `Chosen`, never `Blind`: a remote tool's input schema is the
    // server's to write, so nothing local can prove it holds no
    // destination.
    egress: if hint("openWorldHint") { Egress::Chosen } else { Egress::None },
    destructive: hint("destructiveHint"),
}.union(self.forced)
```

An unannotated server tool is assumed to return private data — that is what
most of them exist to do — but *not* to reach the open world, because assuming
otherwise would arm the interlock on every call. `openWorldHint` means the tool
talks to the wider world, which makes it both a source of attacker-influenced
content and a way out.

`readOnlyHint` is honoured unless config forces `destructive`. Only a forced
`destructive` contradicts a read-only claim; the other axes are orthogonal to
it, which is the same reason `http_fetch` is read-only while being a send sink.
Blanket narrowing here made every knowledge-graph retrieval prompt for
approval, which is unusable for a memory read at turn start.

Config can force capabilities on, per server, and the union means it can only
ever **distrust a server further, never less**.

### The environment is an allowlist, not an inheritance

This is the rule that matters most, because an MCP server is third-party code
running on your machine — where `shell` at least runs commands a model asked
for out loud, a server runs whatever its author wrote.

A server does **not** inherit mecha's environment, so your provider and
search API keys stay with mecha. It starts from a minimal base — `PATH`,
`HOME`, `LANG`, `LC_ALL`, `TZ`, because most runtimes cannot start without
them — plus the `MECHA_TZ` described above, whatever `env_passthrough` names,
and whatever `env` sets. If a server needs a credential, pass it by name:

```toml
[[mcp]]
name = "notes"
command = "notes-mcp"
env_passthrough = ["NOTES_API_TOKEN"]   # copied from mecha's environment
env = { NOTES_REGION = "us" }           # set literally
```

### `sandbox = true` that cannot be honoured is an error

```
MCP server `graph` is configured with `sandbox = true`, but no sandbox backend
is set. Set [sandbox] kind = "bwrap" or "docker", or drop `sandbox = true` to
accept that it runs unconfined.
```

The same rule as `shell`, for the same reason: running unconfined after being
told to confine leaves every downstream decision resting on a belief nothing is
enforcing. Per-server `network` overrides the global switch, because otherwise
you would have to give `shell` the network to let one server reach its own API.

### A server starts in the run's workspace, confined or not

A server's working directory is the run's workspace, so a relative path the
model hands it resolves where the model's own paths do — not wherever you
happened to launch mecha. That is about agreement, not containment: an
unconfined server can still reach everything you can.

## Subagents

A subagent is a child agent exposed to the parent as one more tool: the
parent calls it with a `task`, the child works in a fresh conversation, and
its final answer comes back as the tool result. Each
[`[[subagent]]` profile](/docs/reference/configuration#subagent) becomes one
tool, named by `name`, and its `description` is what decides whether the
parent ever delegates to it.

```toml
[[subagent]]
name = "research"
description = "Search the web and answer a factual question. Use before reading private data."
tools = ["web_search", "web_open"]
```

What a user should know about one:

- **`tools` is an allowlist, not an inheritance.** The child gets exactly the
  tools named there; empty means none, which suits a pure summariser. The
  child keeps its own context, so the pages it read never enter the parent's.
- **It inherits what must not be escaped by delegating:** the parent's
  [taint](/docs/features/security#taint-belongs-to-the-conversation-not-the-run),
  its [outbox](/docs/features/security/outbox) route, its
  [hooks](/docs/features/security/hooks) and its approval rules. A child sent
  into an armed conversation starts armed.
- **Its capabilities are derived from its tools.** It is `private` if any of
  its tools is, `untrusted` if any of them is, and its send class is the
  highest among them — so a search-only child is a `blind` send and stays
  callable from an armed conversation, while adding `http_fetch` makes the
  whole delegation `chosen`, and the interlock refuses it there.
- **`trusted_output` only offers trust; each answer has to earn it.** It must
  name an `answer_shape` — `"number"`, `"boolean"`, or a list of allowed
  answers — and a profile that sets one without the other refuses to start.
  An answer that parses as the shape comes back unmarked; anything else comes
  back as untrusted, with a note saying why. Private data stays private
  either way.

## Inspecting the surface

```bash
mecha tools                 # names, descriptions, and the active sandbox
mecha tools --schema        # exactly what the model sees
mecha tools --json          # each tool's capabilities
```

`mecha tools` runs without any provider configured, which makes it the right
smoke test for a newly wired MCP server.
