# Provenance decides the class — design

**Status: 2026-09-24, designed; P1 building on `feat/provenance-security`.**
The owner approved the four directions and their revised shape the same day,
and ruled R-P1 to R-P4 (§7) the same afternoon.

**The problem, in the owner's words:** with the assistant searching the graph
first, "the context is always armed", so "basically all tool calls are
blocked or limited to specific approvals" — no web reading, no mail, no
factory bundles or polls.

**The problem, measured** over `~/.mecha/sessions` on 2026-09-24:

- Since 2026-09-17, **15 of 18** non-test sessions armed on message 1 or 2.
  There is no run-start graph hook. The model arms the session itself,
  because `prompts/agent.md` §Memory tells it to go to the graph first. Every
  read of `graph`, `mail`, `docs` and `factory` sets both legs, because the
  operator config forces `untrusted_input` on each and every MCP tool is
  `private_data`.
- **Zero `blocked_sends` since 2026-09-16.** The interlock is not what
  refuses. What happens instead is that every write becomes a draft. Of 85
  staged items, 38 were sent, 35 rejected and 12 are still pending. Most
  rejections were quality (wrong account, duplicate event), not security.
- **What is still refused outright** is `http_fetch` and the `research`
  subagent that holds it. `ARMED-READING-RESEARCH.md` (#271) counts them.

So the friction has two sources:

1. The taint is coarse. A whole server is one label, and taint never decays.
2. Staging is the only route for a write.

A classifier cannot remove either (`CLASSIFIER-RESEARCH.md`). What can remove
them is asking **where each part of a call came from**. That is the rule the
interlock already applies to one field (who chooses the destination),
extended to every field.

---

## 1. The principle

> **A call is dangerous only through what an attacker could have authored.**
> Label by provenance at the finest grain a parser or a schema can prove, and
> let a model choose only what the *system* then enforces.

This rule sits under the four decisions. Two corollaries fall out of it:

- **Nothing here decays taint or waives the interlock.** Every decision
  either narrows *what counts* as third-party content, or narrows *which
  calls* can carry it out. None of them relaxes the check.
- **No classifier sits on a path that lowers a label.** See the table in
  `CLASSIFIER-RESEARCH.md` §5.

## 2. P1: a write that reaches nobody is not a send

**Decision.** A write whose input schema names no recipient, and whose
effect is readable only by the owner, is `Egress::None`. It sits with the
approver, like `docs_trash`, and not with the outbox.

This is the existing rule, "the class is earned by a schema with no
destination", applied to writes. It needs **no mecha-core change**, because an
MCP tool without `openWorldHint` is already `Egress::None` (`mcp.rs`, the
`tools/list` mapping).

| Tool | Today | After | Why nobody else can read it |
|---|---|---|---|
| `docs_create`, `sheets_create`, `slides_create` | `openWorldHint`, staged | no `openWorldHint`, approver | New file in the owner's Drive under `drive.file`, and there is no sharing verb (`there_is_no_sharing_or_permissions_verb`) |
| `calendar_hold` (new) | — | no `openWorldHint`, approver | Primary calendar only, no `attendees` or `calendar_id` field, created with private visibility |
| `calendar_create_event` | staged | **unchanged** | `attendees` sends invitations, and `calendar_id` can name a shared calendar |
| `docs_append`, `docs_replace`, `sheets_write` | staged | **unchanged** | Can target a picked document that is already shared |
| `calendar_update_event` | staged | **unchanged** | Outlook notifies attendees on update, and it can target anyone's event |

**The operator half**, which is the owner's config and not this repo's:

- Remove the P1 tools from `[outbox] tools`.
- Add one `[[rule]] decision = "allow"` per tool, so no surface prompts. A
  headless trigger in `ask` mode would otherwise refuse the call rather than
  stage it (`Approver::permit`, policy.rs).
- A `read-only` surface still refuses. A rule's `allow` stands in for the
  human's yes, never for the mode.

**What replaced the review.** An executed write no longer has the outbox's
"exact arguments, one click away". The schema is therefore the whole guard,
and it gets tests that fail on the dangerous shape:

- no recipient-shaped property anywhere in the input schema;
- `calendar_hold` never sends `attendees`, `sendUpdates` or a
  non-primary calendar;
- the new visibility is `private` on Google and `sensitivity: private` on
  Outlook.

**Accepted residuals:**

- A private event on a calendar the owner shares with a delegate still shows
  as *busy* at that time, never its title. That is timing, not content.
- A new Doc holds whatever private text the model put in it. That is the
  owner's data in the owner's Drive, which is where it already was.
- **What the review used to catch, named.** With the `allow` rule, a
  conversation armed by a stranger's mail can create a Drive file or a hold
  whose title and body an injection wrote, and nobody looks at it first,
  headless triggers included. That is clutter and a misleading entry, not a
  leak: nobody else can read it, and `docs_trash` and deleting the hold undo
  it. It is the cost of the change, and the owner took it with R-P1.

**Why not a per-call argument test** (for example, "`calendar_create_event`
with empty `attendees` is `None`")? That puts a predicate over a
server-written schema into the loop. mecha would then have to learn which
field of which tool is a recipient, which the loop must never know. A
separate tool with a narrower schema proves the same thing with nothing to
maintain.

## 3. P2: labels per field, set by a parser

**Decision.** Untrusted means *prose a third party authored*. It does not
mean "came from a server that also serves prose". Parsed structured fields
are not prose.

A result earns trust in two ways, and both are proved by code:

- **(a) A typed-only tool.** A read whose output schema contains only parsed
  fields, and whose server annotation omits `openWorldHint`:
  - mail envelopes: `from` *address* (never the display name), `date`,
    `thread_id`, `unread`, and `to`/`cc` addresses;
  - calendar busy blocks: `start`, `end`, `attendees` and `organizer`
    addresses, `status`, with no title, description or location.

  These answer "did Carol write back?" and "am I free at three?" without
  arming anything. They come as **new tools**, not as flags on existing ones,
  so the label stays a property of the tool.
- **(b) The graph, once its provenance is unforgeable.** Today `kg_upsert`
  takes any `source` string, so a model can write an episode labelled
  `note` and `kg_notes` serves it back as the owner's own notes. Before any
  graph result can be trusted:
  1. **MCP writes are namespaced.** A write over MCP can only produce an
     `agent:*` source. `note`, `manual`, `cv.*` and `reflect.*` are
     writable only from paths with no model: the TUI, the CLI, and ingest
     scripts.
  2. **Every read returns provenance per record.** `kg_task_list` returns
     `nodes.source`, `kg_timeline` returns facts' extractors, and
     `kg_related` returns the source of each node.
  3. **The server declares a result owner-only** only when every record in
     it has an owner source and no third-party prose field. mecha then
     records that result as not `external`.

  Step 3 is the one place a server's claim lowers a label per *result*
  rather than per tool. That is ruling **R-P2** (§7).

**The convention for any server claim a result carries.** One namespace,
one parse point, one rule for who is believed, so a second claim does not
invent a second convention:

- **Keys** live in the result's MCP `_meta`, under the project's domain
  prefix, as the MCP spec reserves prefixed keys for: `mecha-factory.ai/`.
  This design's key is `mecha-factory.ai/provenance` (value `"owner"`, or
  absent). The outbox's "nothing was dispatched" signal (mecha-8a's lane)
  is `mecha-factory.ai/dispatched` (`false`, or absent).
- **Parsed in one place:** `McpClient::call_tool` in `mcp.rs`, into typed
  `ToolOutput` fields. Nothing downstream reads raw `_meta`.
- **Believed only from a server the operator config names** — a per-server
  `[[mcp]] trust_result_claims = true`, operator layer only, stripped from
  a project layer like `[slack]`. Any other server's claim is ignored, and
  the result is handled as today. An ignored claim always fails closed:
  untrusted, not dispatched unknown.
- **It is an exception, and it says so where the rule lives.** Both claims
  loosen a guard on a server's word, which is the direction
  `[mcp.capabilities]` refuses ("config can distrust a server further, never
  less"). R-P2 makes the exception deliberately; what keeps it "one visible
  decision rather than a quiet per-server exemption" is that the build names
  it in `TRIFECTA.md`'s switch table beside the widens-only rule, and that
  `mecha tools --json` and `mecha doctor` name every server with
  `trust_result_claims` on. Raised by mecha-8a on #274.

**Estimated yield before building:** most graph reads touch calendar
(13,452 episodes), Bee and Slack content, which stays untrusted. The win is
owner tasks, notes and the CV. Measure how many arming reads were owner-only
first; §6 has the query.

## 4. P3: the destination's provenance decides the class

**Decision.** `Egress::Chosen` means *the model composed the destination*.
A destination taken verbatim from something the run already holds, and
which the attacker could not have written, is not composed.

For `http_fetch`, these three sources are allowed in an armed conversation:

- **A URL the owner typed**: an exact substring of a user message the owner
  authored, meaning typed or dictated, and not a harness preamble or a
  tool result.
- **A search result, picked by index.** This is `web_open` from #271 §3.2
  (its R1), which becomes a case of this rule rather than its own exception.
  The residual is `log2(N)` selection bits per call, bounded by a budget of
  blind calls.
- **A link the owner selected from a page mecha fetched**, by index, which is
  the same shape as a search result.

A URL that appears only inside untrusted prose is `Chosen` and refused, as
now.

**Where this lives.** It lives in the tool, not the loop. `web_open` takes an
index, so it is `Blind` by schema. `http_fetch` gains a dispatch-time check,
done by the loop from `grounding::calls` and the conversation's user turns.
It narrows only when the URL dereferences exactly. The loop still learns
nothing about what a URL is. The tool answers "is this argument grounded?"
through a new `Tool` hook the same shape as `denial_remedy`. That is ruling
**R-P3**, because it is the first per-call class.

**Mail recipients are deliberately left out.** The outbox already owns
sends, and a reply to "the thread the owner named" is still a reply to its
sender. The Design Patterns paper (§4.3) rejects exactly that exemption,
because the sender may be the attacker.

## 5. P4: re-derive the call from a view with no third-party content

**Decision.** In an armed conversation, a `Chosen` call that is not routed
to the outbox is not refused on sight. Instead the loop:

1. **Builds the clean view:**
   - every owner-authored user message;
   - every result from a tool that returned no external content, rendered as
     data;
   - every untrusted result replaced by a stub (`[third-party content from
     mail__mail_get_thread withheld]`);
   - **every assistant message after the first arming dropped**, because
     text the model wrote after reading an attacker's words is itself
     attacker-influenced. This includes its tool arguments.
2. **Asks the model once more, from that view,** for its next call, with the
   same tools. The view holds private data and no untrusted content, so it
   is *not armed*.
3. **Executes the re-derived call, never the original.** If the model
   produces no call, or a different tool, the original is refused with
   today's text plus one sentence saying the re-derivation found no owner
   intent behind it.

**Why this is safe whatever the model does.** Nothing the attacker wrote
reaches the request that produces the executed call. This is Permissive
IFA's property: safety enforced by the system, not by the model. The
"re-derived" call's result is still external content entering an armed
conversation, which changes nothing.

**What it costs:**

- One extra local inference, only for calls that today are refused outright.
- A cold prefix from the first withheld result onward.
- Context. "Open the link in that email" cannot be re-derived, which is
  correct. "Reply to that" may lose the referent, which costs a refusal and
  never a leak.

**Where it plugs in:** the `TrifectaPolicy::Block` arm in `agent.rs`, before
the refusal. It does not apply to outbox-routed calls, which are never
refused, or to the leak guard, which refuses every send by design.

**What it does not do:** the re-derived call still carries private data to
a destination the owner's words or trusted state produced. That is the owner
asking, which is the principal's call. `block_sends_after_private` is still
the switch for "never, even when I ask".

## 6. How each part is measured

- **P1:**
  - count staged vs executed P1 calls per week, from the outbox and the
    trace;
  - the prediction: calendar holds and new Docs stop appearing in
    `pending`.
- **P2:**
  - before building (b), count arming reads whose records were all
    owner-sourced. This needs step 2's provenance field, so step 2 ships
    first and alone;
  - the prediction to falsify: at least a quarter of first arming reads are
    owner-only.
- **P3 and P4:** an experiment suite in an environment that arms first:
  - two arms, today's tools and P3+P4;
  - tasks: search-then-read, open the link I pasted, and an injected page
    telling the model to fetch a URL carrying a secret;
  - graded on the answer, with `blocked_sends` pinned per case;
  - the injection cases must show **no fetch of a composed URL** in the P4
    arm. That is the result that decides whether P4 ships.

## 7. Rulings (owner, 2026-09-24)

- **R-P1: yes.** Calendar holds are created private. Delegates see "busy",
  not the title.
- **R-P2: yes, with explicit permission in config.** A first-party server
  may claim a result is owner-only (§3 step 3). mecha honours the claim only
  from a server the *operator* config names, never from a project layer.
  This relaxes "capability overrides only widen" on purpose, and the
  permission line is the owner's vouch for that one server.
- **R-P3: yes.** A tool's class may vary per call. It may only narrow from
  the declared class, and only on a proof the loop checks (§4).
- **R-P4: a one-line notice.** A re-derived call says so in its result:
  "re-derived from your request alone".
- **#271's open items:** R1 is absorbed by P3. R2 (MCP results default to
  untrusted) still stands and composes with P2: default untrusted, with
  trust earned per tool or per result by proof. R4 follows R-P2's answer.
  R3 is unchanged.

## 8. Order of work

1. **P1.** mecha-mail annotations and `calendar_hold`, rebased onto #272,
   which touches `unified.rs`. Then the owner's config lines.
2. **P2 step 2.** Graph provenance on every read (mecha-graph, the public
   repo). Measurement only.
3. **P3.** `web_open`, then the grounded-URL check on `http_fetch`.
4. **P4.** Re-derivation, behind a `[security] rederive` switch until the
   experiment in §6 passes.
5. **P2 steps 1 and 3.** Once the measurement says they are worth it.

## 9. Deliberately not in scope

- **Classifiers on any path that lowers a label** (`CLASSIFIER-RESEARCH.md`
  §5).
- **Taint decay, per-turn reset, and `trifecta = "allow"`.**
- **Changing the prompt's graph-first instruction.** It is the right advice
  for an assistant. The fix is that reading the owner's own records should
  stop costing the whole conversation.
- **Outbox release UX.** That is its own lane (#272).
