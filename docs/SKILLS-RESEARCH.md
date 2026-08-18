# Skills — what the harnesses built, and what mecha should take

*2026-08-17. Agent Skills went from one vendor's feature to a cross-vendor
standard in about ninety days, and then produced the first coordinated
supply-chain attack against agent harnesses. Both halves matter. This is the
survey behind adding a skill mechanism to mecha, written before the design.*

---

## 0. The short answer

**Take the format, refuse the ecosystem.**

`SKILL.md` — a directory, YAML frontmatter with `name` and `description`, a
markdown body, optional bundled files — is now a genuine cross-vendor standard
with roughly forty implementations. There is no reason to invent a different
file format, and one strong reason not to: the procedures worth writing are
portable, and this repository already has two of them (`.claude/skills/update`,
`.claude/skills/handoff`) sitting in exactly that format.

What must not be taken is the distribution model. Snyk's ToxicSkills study
scanned 3,984 published skills: **36.8% carried at least one security flaw,
13.4% a critical one, and 76 confirmed malicious payloads** — credential
exfiltration by base64-obfuscated install commands, remote binaries in
password-protected ZIPs, and instructions telling the agent to disable its own
safeguards. 91% of the confirmed-malicious ones used prompt injection *as well
as* malicious code, which is the combination that defeats code scanners and
model safety training at the same time.

The single most relevant sentence in the whole survey, from Datadog's write-up
of the same wave:

> A cloned repository can bring skills into a trusted session even if the
> developer never installed a skill from a marketplace.

mecha already refuses that shape, for triggers, in writing:

> `[[hook]]`, `[[mcp]]` and `[[subagent]]` are all declarable in a project's
> `mecha.toml`, which is a file that arrives with a cloned repository. A
> trigger is a scheduled unattended agent run, so a repo that could declare one
> has been handed a cron slot on your machine.

That rule was written about cron and it predicted this. It applies to skills
with more force, because a skill is a page of instructions rather than one
line, and because mecha holds mail, calendar and a knowledge graph rather than
a checkout.

So: same file format, materially stricter trust policy than any harness
surveyed. Concretely — no marketplace, no install verb, no project-layer
skills, no remote fetch of a body, and no bundled executables in v1.

---

## 1. The standard, precisely

Anthropic published the Agent Skills spec on 2025-12-18. The unit is a
directory:

```
skill-name/
  SKILL.md          required
  REFERENCE.md      optional, loaded only if the body points at it
  scripts/…         optional, executed rather than read
  templates/…       optional resources
```

`SKILL.md` is YAML frontmatter plus a markdown body. **Two required fields:**

| Field | Constraint |
|---|---|
| `name` | ≤64 chars, lowercase letters/digits/hyphens only, no XML tags, may not contain "anthropic" or "claude" |
| `description` | non-empty, ≤1024 chars, no XML tags |

The `description` carries the entire discovery burden — it is what the model
matches a request against — so the spec's authoring guidance is that it must
say **what the skill does *and* when to use it**, not just what it is. This is
the same instruction `SubagentProfile::description` already carries in
`subagent.rs:54`: *"say when to use it, not just what it is."* Two independent
designs arriving at the same sentence is a good sign it is load-bearing.

Claude Code discovers skills from `~/.claude/skills/` (personal) and
`.claude/skills/` (project), plus plugins. Note that the project path is the
supply-chain hole named in §0.

---

## 2. Progressive disclosure is the whole idea

Three levels, with the published token costs:

| Level | Loaded | Cost | Content |
|---|---|---|---|
| 1 · Metadata | always, at startup | **~100 tokens per skill** | `name` + `description` |
| 2 · Instructions | when triggered | **under 5k tokens** | the SKILL.md body |
| 3 · Resources | when referenced | **zero until read** | bundled files; scripts contribute only their *output* |

The consequence is the reason the pattern won: **you can install many skills
without a context penalty**, because an untriggered skill costs a hundred
tokens. And a bundled script costs nothing at all — its code never enters
context, only what it prints.

That third row is worth dwelling on, because it is the part that is not merely
prompt engineering. A skill that ships `validate.py` gets deterministic
behaviour at zero context cost, where a skill that *describes* the validation
gets non-deterministic behaviour at full cost. It is the same argument
`compact.rs` makes about `carried_state`: some things should be data the tool
owns, not prose the model re-derives.

**For mecha this interacts with prompt caching in a way no other harness has to
think about.** Level-1 metadata lives in the system prompt, and the Anthropic
backend renders tools → system → messages with the cache breakpoint on the last
system block. So the skill list sits *inside the cached prefix*. Two rules
follow immediately, and they are the tool-registry rules again:

- **Stable order.** The registry is a `BTreeMap` because "tool order is the
  front of the cached prefix — reordering it invalidates the cache every turn."
  A skill list read from a directory must be sorted for the same reason;
  filesystem order is not an order.
- **Enabling or disabling a skill invalidates the prefix for that session.**
  Acceptable, and worth saying out loud so nobody adds a feature that toggles
  skills per turn.

---

## 3. The neighbours

**Goose recipes** (Block) are the most different, and the difference is
instructive. A recipe is a YAML file bundling instructions, *which extensions
to enable*, parameters, provider settings, retry logic and a structured
response schema — and subrecipes, where each is "effectively another goose
agent with its own configuration." That is not mecha's skill; **that is
mecha's `[[subagent]]` plus `[[trigger]]`**, and mecha already has both. Worth
noting because it clarifies the boundary: a recipe is a *run configuration*, a
skill is *knowledge loaded into a run*. Goose supports SKILL.md too, so even
Goose treats these as different objects.

**OpenHands** renamed microagents to skills and adopted the format, and
contributes one idea worth stealing: an explicit **`triggers` keyword list** in
the frontmatter for knowledge agents, separate from the prose description, plus
a `repo.md` that loads unconditionally for a repository. The keyword list is a
deterministic complement to description-matching — the model does not have to
infer relevance from prose alone.

**Cursor rules / AGENTS.md** are the always-on tier: no trigger, no
progressive disclosure, loaded every time. mecha's equivalent is learned rules
plus the system prompt, and the comparison is the useful part — see §6.

---

## 4. It is a standard, and that is the argument for the format

Within 48 hours of publication, Microsoft had it in VS Code and OpenAI in
ChatGPT and Codex CLI. By March 2026, 32 tools; by mid-2026, roughly 40 on
agentskills.io — Claude Code, Codex, Copilot, VS Code, Cursor, Gemini CLI,
Goose, OpenCode, Kiro, Junie, Databricks, Snowflake. Reported as one of the
fastest cross-vendor standardisations in the tooling space.

For mecha specifically this means a skill the user writes is not mecha-shaped
work. `handle-rec-letter` written for mecha is readable by Claude Code working
on this repository, and the two skills already in `.claude/skills/` are
readable by mecha the day the mechanism exists. Inventing a `[[skill]]` TOML
table would throw that away for nothing.

---

## 5. The security record, which is bad

This is the section that should decide the design.

**Snyk, ToxicSkills** — 3,984 skills scanned from ClawHub and skills.sh:

| Finding | Share |
|---|---|
| At least one security flaw | 36.8% (1,467) |
| At least one **critical** flaw | 13.4% (534) |
| Hardcoded secrets / API keys | 10.9% |
| **Fetch untrusted third-party content** | 17.7% |
| **Dynamically load remote instructions at runtime** | 2.9% |
| Confirmed malicious payloads (human-reviewed) | 76 |

Of the confirmed-malicious: 100% contained malicious code patterns and 91%
*also* used prompt injection. Techniques were external malware distribution
(password-protected ZIPs to defeat scanning), base64/Unicode-obfuscated
credential exfiltration, and instructions to disable the agent's own safety
mechanisms or plant persistence in agent memory files.

**Datadog Security Labs** documented the February 2026 ClawHub campaign — 30+
malicious skills, the first coordinated one — and named the mechanism that
matters most: *dynamic context commands run before the model sees the skill at
all*, so model-level injection defences never get a turn.

**The academic line agrees.** arXiv 2510.26328, *Agent Skills Enable a New
Class of Realistic and Trivially Simple Prompt Injections*, and arXiv
2606.02540, *SkillHarm*, on lifecycle-aware skill-based attacks.

Anthropic's own guidance is unambiguous and is the right starting posture:
use skills only from sources you created or that came from Anthropic; audit
every bundled file; treat installing one like installing software; skills that
fetch external URLs are particularly dangerous because the fetched content may
carry instructions.

### What this implies for a personal assistant specifically

Every harness above is primarily a *coding* agent. The blast radius of a
malicious skill there is a repository and whatever credentials are lying
around — bad, and the studies show it is being exploited. mecha's blast radius
is mail, calendar, a knowledge graph, an outbox with staged sends, and a front
door holding other people's data. The lethal trifecta is described in this
repo's own terms as "the permanent condition rather than an edge case."

A skill body is trusted text inside a privileged run. It is the *longest*
half-life injection path in the system if it can be installed — longer than a
learned rule, which is one line and provenance-gated, and far longer than a
fetched page, which the interlock at least accounts for. So the correct
conclusion is not "be careful with marketplaces." It is that **mecha should
have no installation path at all.**

---

## 6. Where mecha stands

mecha has no skills mechanism. `.claude/skills/` in this repository is Claude
Code's, for working *on* mecha; the agent cannot see it.

What exists, and why none of it is a skill:

| Mechanism | Shape | Why it is not this |
|---|---|---|
| `[[subagent]]` | name, description, tool allowlist, `system_prompt`, `max_turns`, model/provider override, `trusted_output` + `answer_shape`; exposed to the parent **as a tool** | closest by far — but config-only, the instructions are one inline TOML string, no bundled files, and invoking one always spawns a child agent with its own conversation and turn budget |
| Learned rules | prose in the cached prefix, always on | not selectable, hard-capped, and provenance-gated *because* it is always on |
| `[[trigger]]` | a stored prompt + tool allowlist + workspace | fired by cron, never chosen by the model |
| `[[hook]]` | a command at a lifecycle point | policy, not capability |
| System prompt | always on | one, global |

So the gap is: **a named, described, on-demand body of prose that the model
selects, that a human edits as a file, and that can carry resources.**

### The argument that connects it to the memory research

`docs/MEMORY-RESEARCH.md` produced a **hard cap of 15 active learned rules**,
because the always-on prefix is finite and curation beats accumulation by a
measured ~10%. That cap is a real constraint on how much procedural knowledge
mecha can hold, and today there is nowhere else to put any.

Skills are the pressure valve. A procedure like *how to handle a rec letter
request* is exactly the thing that is too long for a rule, too specific to be
worth 1/15th of the always-on budget, and irrelevant on 95% of runs — which is
the precise profile progressive disclosure was built for. Adding skills does
not loosen the rule cap; it makes the cap affordable.

---

## 7. What mecha should build

### The store

```
~/.mecha/skills/<name>/SKILL.md      frontmatter: name, description
                                      optional: triggers, tools
                                      body: the procedure, as prose
~/.mecha/skills/<name>/*             reference files
```

Global only. Owner-only permissions, like every other directory under
`~/.mecha`.

### Six rules, each of which is a bug if undone

- **User-authored only.** A skill is never written by a model, never derived
  from a session, never proposed by `reflect`, and there is no `mecha skill
  install`. If the agent could author a procedure a later run obeys, the
  provenance gate that `learning.rs` enforces has been routed around by a
  mechanism that did not exist when it was written. The absence of an install
  verb is the feature — §5 is what it buys.
- **Never the project layer.** The triggers rule, verbatim, and §0's citation
  is the evidence that it was right. `mecha.toml` may *enable or disable*
  skills by name — a list of strings — so a project can narrow the set without
  authoring anything.
- **No remote anything.** A body is never fetched; a `SKILL.md` that names a
  URL is prose the model may act on through ordinary tools under the ordinary
  interlock, not a loading mechanism. 17.7% and 2.9% in §5 are what this
  forecloses.
- **No bundled executables in v1.** Level 3 scripts are the best part of the
  spec and the worst part of the threat model, and mecha's default sandbox is
  `"none"`. If scripts land later they run confined or not at all — the same
  rule as `shell` and MCP servers, for the same reason.
- **Loading is a tool call, not a `cat`.** Claude Code loads Level 2 by having
  the model run bash. mecha should register an explicit `skill` tool instead,
  and the reasons are all mecha's: `shell` may be sandboxed or absent; a tool
  call is visible to `pre_tool` hooks, so a policy hook can gate which skills
  load; it appears in the trace, so an eval case can assert on it with
  `expect.trace`; and it does not require the model to know where the
  filesystem keeps things.
- **Tools may narrow, never widen.** If a skill declares a tool list, it
  restricts the surface while loaded. This is the MCP capability-override rule
  — config can distrust further, never less — and the same reason
  `Capabilities` narrowing under the sandbox is limited to `external_send`.

### Level 1, and where it lives

`name` + `description` for every enabled skill goes in the system prompt, in
**sorted order**, for the cache reason in §2. Taking OpenHands' idea, an
optional `triggers` keyword list is worth having beside the description: a
deterministic match is a cheap complement to a model inferring relevance from
prose, and it costs nothing at Level 1.

### What stays separate

**Subagents.** They are the *delegate* shape; skills are the *instruct* shape.
Collapsing them loses the fresh `Conversation` that makes delegation a clean
taint boundary — the single most useful property subagents have. A skill may
well say "delegate this to the `researcher` subagent"; that composition is
correct and needs no new machinery.

**Learned rules.** Different provenance model, different lifetime, different
budget. A skill is user-authored and on-demand; a rule is machine-derived and
always-on, which is why one is gated and the other is simply trusted. Keeping
them separate is what lets skills be liberal and rules stay strict.

---

## 8. Open questions

1. **Does a loaded skill survive compaction?** It is prose in the messages, so
   a summariser will paraphrase it — and a paraphrased procedure is a
   different procedure. `Tool::carried_state` exists for exactly this
   ("preserves what is true, drops how far you got"), and a loaded skill is a
   plausible second user of it. Leaning: yes, carry the *fact* that skill X is
   loaded and re-emit the body verbatim, since `rebuild` already places carried
   state after the summary as the part known to be current.
2. **Can a skill be loaded in an unattended run?** A trigger has no human. If
   the model may load any skill, a trigger's effective instruction set is
   larger than its trigger file shows — and `trigger show` printing the
   resolved workspace exists precisely because "what does this run actually
   do" must not be answered by an omitted line. Leaning: triggers name their
   skills explicitly, like they name their tools.
3. **Does the front door's extractor get skills?** No — it is issued a request
   with an empty tool list and a single user message, and that is the whole
   point. Worth writing down so it is not "fixed" later.
4. **Do skills appear in `mecha tools --json`?** They are not tools, but
   "what does this agent actually know how to do" is the same question, and
   `mecha tools` is already the answer to the tool half of it. A `mecha skills`
   command in the same shape is probably right.
5. **Eval.** A skill changes behaviour, so a scorecard taken with skills on is
   not comparable to one taken without. `mecha eval` already forces MCP, hooks,
   learned rules, fallbacks and the outbox off for exactly this reason. Skills
   should default off there too, with a flag to measure them deliberately.

---

## 9. What not to build

- **An install command, a marketplace, or a registry client.** §5.
- **Project-layer skills.** §0, §7.
- **Model-authored skills**, including "promote this reflection to a skill".
- **Remote bodies or runtime-fetched instructions.**
- **A new file format.** §4.
- **A skill that can widen the tool surface**, add a capability, or relax the
  interlock.
- **Auto-loading by keyword without a tool call.** The load must be visible in
  the trace and gateable by a hook; a silent context injection is the thing
  Datadog named as defeating every downstream defence.

---

## Sources

Surveyed 2026-08-17.

**The specification**
- [Agent Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview)
  — frontmatter fields and constraints, the three levels and their token
  costs, discovery paths, and the security guidance quoted in §5
- [Equipping agents for the real world with Agent Skills](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)
- [Use Skills in Claude Code](https://code.claude.com/docs/en/skills)

**The standard and its adoption**
- [The Agent Skills open standard: portable SKILL.md across 30+ tools](https://codex.danielvaughan.com/2026/05/05/agent-skills-open-standard-portable-skills-codex-cli-cross-agent/)
- [Agent Skills open standard explained](https://www.paperclipped.de/en/blog/agent-skills-open-standard-interoperability/)
- [The Agent Skills ecosystem in 2026](https://agentman.ai/blog/agent-skills-ecosystem-report-2026)

**The neighbours**
- [block/goose recipe.yaml](https://github.com/block/goose/blob/main/recipe.yaml) ·
  [Configuring agents with Goose recipes](https://www.pulsemcp.com/building-agents-with-goose/part-4-configure-your-agent-with-goose-recipes)
- [OpenHands microagents overview](https://docs.openhands.dev/openhands/usage/microagents/microagents-overview) ·
  [OpenHands/skills registry](https://github.com/OpenHands/skills)

**The security record**
- [Snyk — ToxicSkills: prompt injection in 36% of agent skills, 1,467 flawed, 76 malicious payloads](https://snyk.io/blog/toxicskills-malicious-ai-agent-skills-clawhub/)
- [Datadog Security Labs — malicious coding agent skills and the risk of dynamic context](https://securitylabs.datadoghq.com/articles/malicious-skills-supply-chain-risks-in-coding-agents-with-dynamic-context/)
- [Repello AI — how to audit any skill before you run it](https://repello.ai/blog/claude-code-skill-security)
- arXiv [2510.26328](https://arxiv.org/pdf/2510.26328) — *Agent Skills Enable a
  New Class of Realistic and Trivially Simple Prompt Injections*
- arXiv [2606.02540](https://arxiv.org/pdf/2606.02540) — *SkillHarm:
  Lifecycle-Aware Skill-Based Attacks via Automated Construction*

**Local**
- `mecha-core/src/subagent.rs` — `SubagentProfile`, the nearest existing shape
- `docs/MEMORY-RESEARCH.md` — the 15-rule cap that skills relieve
- `.claude/skills/{update,handoff}/SKILL.md` — two skills already in the
  format, written for the other side of the repository
