//! Gossip: two agents, different sources, generative follow-ups.
//!
//! The mechanism the knowledge graph's design has always put at its centre,
//! and the one every cheaper approximation has failed to substitute for.
//! What is *not* gossip, established by measurement on 2026-08-13: filling a
//! template, issuing two filtered retrievals and diffing the answers. That
//! found zero contradictions in 58 probes, because the split was
//! facts-versus-evidence — a distillation compared against its own origin,
//! never two witnesses.
//!
//! Here the perspectives are **sources**, which are independent: a calendar
//! entry is a plan, a Bee transcript is what was said, a Slack thread is what
//! was written. And the claim under test is not that they disagree. It is
//! that two readers asking *each other* questions surface things no template
//! names — "why do you know her?" is a move a slot list cannot make.
//!
//! ## Why this is Rust and not a prompt
//!
//! Commit-then-reveal is the protocol's load-bearing rule: conformity
//! corrupts answer *formation*, generativity lives in *follow-up*, so the
//! phases must be kept apart. A parent model told to "ask B without showing
//! it A's answer" can simply not comply, and nothing would notice. Here B's
//! context does not contain A's answer because the code has not put it there
//! yet. The rule is a property of the program, not an instruction.
//!
//! ## The capability boundary
//!
//! Each child gets exactly one tool: [`LensedSearch`], which is `kg_search`
//! with its `sources` and time window nailed shut and removed from the
//! schema. That is deliberate on two counts. It makes the perspective
//! structural — a child cannot widen its own lens to see what its partner
//! sees, which would collapse the two witnesses into one. And it keeps the
//! interlock disarmed: the children read private material, so handing either
//! of them anything outbound would arm the trifecta. A web query composed
//! after reading a private transcript *is* the leak, which is why the web is
//! a route that fills what gossip finds, in a separate untainted session —
//! never a third participant here.
//!
//! ## It cannot ask the user anything
//!
//! A gossip run is a background measurement; a question to the owner would
//! both block it and spend the scarcest budget in the design on a process
//! nobody is watching. [`reader`] makes that true three ways over, so no
//! single mistake restores it:
//!
//! 1. the registry holds exactly one tool, so `ask_user` and `message_send`
//!    are not merely forbidden but absent;
//! 2. that tool is read-only, so it never reaches the approval gate;
//! 3. the approver is [`ModeApprover`] in `ReadOnly`, which answers from
//!    policy without asking anyone — anything non-read-only is *blocked*,
//!    never prompted.
//!
//! Asking the owner remains a real channel in the design, with an attention
//! budget of its own. It is simply not this mechanism's to spend.

use crate::agent::{Agent, Conversation, RunContext};
use crate::mcp::McpClient;
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// `kg_search` with the lens welded on.
///
/// The child chooses its query and nothing else: `sources`, `since` and
/// `until` are injected here and absent from the schema it is shown, so
/// "only look at the calendar" is enforced rather than requested.
pub struct LensedSearch {
    client: Arc<McpClient>,
    label: String,
    sources: Vec<String>,
    since: String,
    until: String,
    description: String,
}

impl LensedSearch {
    pub fn new(
        client: Arc<McpClient>,
        label: &str,
        sources: Vec<String>,
        since: &str,
        until: &str,
    ) -> Self {
        let description = format!(
            "Search the user's knowledge graph. You can see ONLY these sources: {}. \
             Evidence is limited to {since}..{until}. Another assistant is reading \
             different sources and can see things you cannot.",
            sources.join(", ")
        );
        LensedSearch {
            client,
            label: label.to_string(),
            sources,
            since: since.to_string(),
            until: until.to_string(),
            description,
        }
    }
}

#[async_trait]
impl Tool for LensedSearch {
    fn name(&self) -> &str {
        "kg_search"
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        // No `sources`, no `since`, no `until`: what the model cannot name,
        // it cannot widen.
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What to look for"},
                "k": {"type": "integer", "description": "Max results (default 10)"}
            },
            "required": ["query"]
        })
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // The graph holds the user's own life; episodes are private-tier.
            private_data: true,
            // Episode bodies are third-party text — a calendar invite title
            // or a Slack message is written by someone else.
            untrusted_input: true,
            external_send: false,
            destructive: false,
        }
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(query) = input.get("query").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("missing required string argument `query`"));
        };
        let args = json!({
            "query": query,
            "k": input.get("k").and_then(Value::as_u64).unwrap_or(10),
            "include_private": true,
            "sources": self.sources,
            "since": self.since,
            "until": self.until,
            // EVIDENCE ONLY, and load-bearing rather than tidy. pkg's
            // source and window filters apply to the episode arm; the
            // facts arm takes neither. Leave scope at its default and both
            // readers are served the same distilled layer — the very thing
            // they are supposed to be independent of — so they agree by
            // construction and cite it back as their own reading. The
            // first live run did exactly that: both returned "892 shared
            // episodes, NPMI 1.00" and meetings from 2016 and 2020,
            // through a 2024+ window, because those are facts.
            "scope": "evidence_only",
        });
        let mut out = self.client.call_tool("kg_search", args).await?;
        // Label whose view this was, so a transcript read later says which
        // reader saw what without re-deriving it from the lens config.
        out.content = format!("[{} view]\n{}", self.label, out.content);
        Ok(out)
    }
}

/// A graph tool handed through unchanged, for the one participant that is
/// not a witness.
///
/// [`LensedSearch`] exists to *narrow*; this exists to refuse to. The
/// Verifier is the only role allowed the whole graph — every source, the
/// full window, and the fact layer the readers are deliberately kept away
/// from — because an adjudicator restricted to one vantage is just a third
/// witness with opinions about the other two.
///
/// It stays read-only and outbound-free like everything else here, so the
/// wider view costs nothing in interlock terms: it reads more, and it still
/// has no way to send.
pub struct GraphTool {
    client: Arc<McpClient>,
    name: String,
    description: String,
    schema: Value,
}

impl GraphTool {
    pub fn verify(client: Arc<McpClient>) -> Self {
        GraphTool {
            client,
            name: "kg_verify".into(),
            description: "Check what the graph BELIEVES against what its evidence \
                 actually says — deterministic, no model in the loop. Give a `node` \
                 (name or id) for every live claim about it. Verdicts include \
                 supported, contradicted, denied, stale, residue, unrooted."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "node": {"type": "string", "description": "Entity name, alias or id"},
                    "fact": {"type": "string", "description": "A single fact uid"},
                    "limit": {"type": "integer"}
                }
            }),
        }
    }

    /// Entity metadata: node id, aliases, identifiers, per-source coverage,
    /// interaction count, last seen.
    ///
    /// Added because the audit was inconsistent without it. "Luke was last
    /// seen on August 13" came back supported while "Luke has 1,212 recorded
    /// interactions" came back unsupported — the same class of claim, judged
    /// two ways, because both live in entity metadata and the verifier could
    /// only reach whichever of them happened to surface in a search result.
    /// An adjudicator that cannot see a field will call a true claim about
    /// it unsupported, which is the one verdict that must stay trustworthy.
    pub fn entity(client: Arc<McpClient>) -> Self {
        GraphTool {
            client,
            name: "kg_entity".into(),
            description: "Look up an entity's record: node id, aliases, identifiers \
                 (emails, Slack ids), which sources cover it, interaction count and \
                 when it was last seen. Use this for claims about the graph's own \
                 bookkeeping rather than about events."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name_or_id": {"type": "string", "description": "Entity name, alias or id"}
                },
                "required": ["name_or_id"]
            }),
        }
    }

    pub fn search_everything(client: Arc<McpClient>) -> Self {
        GraphTool {
            client,
            name: "kg_search".into(),
            description: "Search the whole knowledge graph — every source, no time \
                 limit, facts as well as evidence. Use it to find whether anything \
                 actually supports a claim."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "k": {"type": "integer", "description": "Max results (default 10)"}
                },
                "required": ["query"]
            }),
        }
    }
}

#[async_trait]
impl Tool for GraphTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        self.schema.clone()
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            private_data: true,
            untrusted_input: true,
            external_send: false,
            destructive: false,
        }
    }
    async fn call(&self, mut input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        if self.name == "kg_search" {
            if let Some(o) = input.as_object_mut() {
                o.insert("include_private".into(), json!(true));
            }
        }
        self.client.call_tool(&self.name, input).await
    }
}

/// Sources that carry the same kind of account, so two vantages drawn from
/// one family are two slices of one witness rather than two witnesses.
///
/// Reflect is split where Bee is not: `bee.daily` is a machine digest of
/// `bee.conversation` — one witness wearing two labels — while a Reflect
/// note and a Reflect daily are separately written accounts that happen to
/// share an app. Collapsed under one name, two runs could read different
/// shelves and present as one mechanism contradicting itself.
pub fn family(source: &str) -> &'static str {
    match source {
        s if s.starts_with("bee.") => "spoken",
        "reflect.note" => "reflected.note",
        "reflect.daily" => "reflected.daily",
        s if s.starts_with("reflect.") => "reflected",
        s if s.starts_with("session.") || s.starts_with("agent:") => "agentic",
        "calendar.event" => "scheduled",
        "slack.thread" | "mbox" | "email.thread" => "written",
        _ => "other",
    }
}

/// The family a candidate's ORIGIN belongs to — accepting either a full
/// source name (`bee.conversation`) or a proposer string (`bee:suggested`).
///
/// This must not be reconstructed by re-suffixing the origin's head token:
/// `slack.thread` round-tripped through `"slack."` lands in `other`, which
/// bars nothing and leaves the origin itself eligible as its own witness.
/// A proposer names a family only when the tool itself is the source (Bee's
/// fact API); an extractor proposer (`llm`, `llm:commitment`) says nothing
/// about where the evidence lived, and pretending otherwise is how a claim
/// gets corroborated by its own transcript — so those return None.
pub fn family_of_origin(origin: &str) -> Option<&'static str> {
    if origin.starts_with("agent:") {
        return Some(family(origin));
    }
    match origin.split_once(':') {
        Some(("bee", _)) => Some("spoken"),
        Some(_) => None,
        None if origin == "llm" => None,
        None => Some(family(origin)),
    }
}

/// What `kg_entity` reports about one source's coverage of an entity.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceCoverage {
    pub source: String,
    pub episodes: i64,
}

/// Pick two vantages from what actually covers this entity.
///
/// Deliberately NOT a fixed written-versus-spoken split. Coverage is
/// lopsided in practice — one person in the live graph has 493 Slack
/// episodes and 2 Bee conversations — and forcing the tidy split hands one
/// reader almost nothing, which produces a confident "I don't know" that
/// reads like a finding rather than like an empty shelf.
///
/// So: the best-covered source, then the best-covered source from a
/// DIFFERENT family, falling back to next-best overall when no second
/// family clears the floor. Two witnesses of the same kind is still better
/// than one witness and a silence, but the family preference comes first
/// because independence is the whole point.
pub fn choose_vantages(coverage: &[SourceCoverage], min: i64) -> Option<(Vantage, Vantage)> {
    let mut viable: Vec<&SourceCoverage> = coverage.iter().filter(|c| c.episodes >= min).collect();
    viable.sort_by_key(|c| -c.episodes);
    let first = *viable.first()?;
    let second = viable
        .iter()
        .find(|c| family(&c.source) != family(&first.source))
        .copied()
        .or_else(|| viable.get(1).copied())?;
    Some((
        Vantage {
            label: family(&first.source).into(),
            sources: vec![first.source.clone()],
        },
        Vantage {
            label: family(&second.source).into(),
            sources: vec![second.source.clone()],
        },
    ))
}

/// Ask the graph which sources cover an entity. Keeps `call_tool` crate-
/// private: a front-end should not be reaching into the MCP client to
/// hand-roll a graph call.
pub async fn coverage(
    client: &McpClient,
    entity: &str,
) -> Result<(String, Vec<SourceCoverage>, Vec<String>)> {
    let out = client
        .call_tool("kg_entity", json!({ "name_or_id": entity }))
        .await
        .context("kg_entity")?;
    let body: Value = serde_json::from_str(&out.content)
        .with_context(|| format!("kg_entity returned non-JSON: {}", out.content))?;
    if let Some(cands) = body.get("ambiguous").and_then(Value::as_array) {
        let names = cands
            .iter()
            .map(|c| format!("{} ({})", c["name"], c["id"]))
            .collect();
        return Ok((String::new(), vec![], names));
    }
    anyhow::ensure!(
        body["found"] != json!(false),
        "no entity matching '{entity}'"
    );
    let name = body["node"]["name"].as_str().unwrap_or(entity).to_string();
    let sources: Vec<SourceCoverage> =
        serde_json::from_value(body["sources"].clone()).unwrap_or_default();
    Ok((name, sources, vec![]))
}

/// Coverage for an entity, resolving an ambiguous name rather than dying on
/// it.
///
/// [`coverage`] reports ambiguity and stops, which is right for an
/// interactive command and wrong here. A positive control on the
/// `llm·uses` class — 78% human acceptance, durable properties that plainly
/// appear in several sources — returned "no witness" for all eight
/// candidates, because their subject is the bare string "Luke" and the
/// graph holds two Luke nodes. Coverage came back empty and every claim
/// about the graph's owner became unjudgeable.
///
/// That is the third distinct thing one duplicate identity has broken
/// today: `pkg dups` could not see the pair, staging dropped the subject
/// from 189 candidates, and now coverage cannot be measured at all. So this
/// picks the candidate with the most interactions, re-asks by id, and
/// returns whether it had to guess. Merging the nodes remains the real fix.
pub async fn coverage_best(
    client: &McpClient,
    entity: &str,
) -> Result<(String, Vec<SourceCoverage>, bool)> {
    let ask = |q: String| async move {
        let out = client
            .call_tool("kg_entity", json!({ "name_or_id": q }))
            .await?;
        let body: Value = serde_json::from_str(&out.content)
            .with_context(|| format!("kg_entity returned non-JSON: {}", out.content))?;
        anyhow::Ok(body)
    };

    let body = ask(entity.to_string()).await?;
    if let Some(cands) = body.get("ambiguous").and_then(Value::as_array) {
        // Most interactions wins. Not arbitrary: a name split across a
        // dominant node and a stub is the commonest shape of a duplicate,
        // and the dominant node is the one the sources actually cover.
        let Some(best) = cands.iter().max_by_key(|c| {
            c.get("interaction_count")
                .and_then(Value::as_i64)
                .unwrap_or(0)
        }) else {
            return Ok((String::new(), vec![], true));
        };
        // `id`, not `node_id`: kg_entity's ambiguity envelope and its
        // verify counterpart spell this field differently, and reading the
        // wrong one fails silently — the re-ask got an empty string, found
        // nothing, and fell back to the very name that was ambiguous, so
        // the control run reported "measured on 'Luke'" and no coverage.
        let Some(id) = best["id"].as_str().filter(|s| !s.is_empty()) else {
            return Ok((String::new(), vec![], true));
        };
        let body = ask(id.to_string()).await?;
        if body["found"] == json!(false) {
            return Ok((String::new(), vec![], true));
        }
        let name = body["node"]["name"].as_str().unwrap_or(entity).to_string();
        let sources = serde_json::from_value(body["sources"].clone()).unwrap_or_default();
        return Ok((name, sources, true));
    }
    if body["found"] == json!(false) {
        return Ok((String::new(), vec![], false));
    }
    let name = body["node"]["name"].as_str().unwrap_or(entity).to_string();
    let sources = serde_json::from_value(body["sources"].clone()).unwrap_or_default();
    Ok((name, sources, false))
}

/// How many episodes each source holds about this entity WITHIN the window.
///
/// `kg_entity` reports all-time coverage, and all-time is a different
/// number: one person here has 493 Slack episodes since 2015 and two since
/// 2024. Choosing vantages on the all-time figure picked a pair that was
/// nearly empty in the window actually read, and both readers correctly
/// reported knowing almost nothing — a null result manufactured by the
/// selection rather than found in the graph. Ask the question the run will
/// actually ask.
pub async fn windowed_coverage(
    client: &McpClient,
    entity: &str,
    sources: &[SourceCoverage],
    since: &str,
    until: &str,
) -> Result<Vec<SourceCoverage>> {
    let mut out = Vec::new();
    for c in sources {
        let res = client
            .call_tool(
                "kg_search",
                json!({
                    "query": entity, "k": 25, "include_private": true,
                    "scope": "evidence_only", "sources": [c.source.clone()],
                    "since": since, "until": until,
                }),
            )
            .await?;
        let body: Value = serde_json::from_str(&res.content).unwrap_or_else(|_| json!({}));
        let n = body["items"].as_array().map(|a| a.len()).unwrap_or(0) as i64;
        if n > 0 {
            out.push(SourceCoverage {
                source: c.source.clone(),
                episodes: n,
            });
        }
    }
    Ok(out)
}

/// Build the ASKER half of a pair: no tools at all.
///
/// An asker reasons over the two answers it is shown and produces one
/// question. Give it `kg_search` and it goes researching instead — the
/// first live run did exactly that, and round 2's "question" was the other
/// reader's answer pasted back, because a model holding a search tool and
/// told to ask something will answer instead. Removing the tool is the
/// difference between a rule and a hope.
pub fn asker(
    provider: Box<dyn crate::provider::Provider>,
    tool_ctx: ToolCtx,
    agent_cfg: crate::config::AgentConfig,
    model: Option<String>,
) -> Result<Agent> {
    let approver = Arc::new(crate::tool::ModeApprover {
        mode: crate::config::PermissionMode::ReadOnly,
    });
    let mut cfg = agent_cfg;
    cfg.system_prompt = Some(FOLLOWUP_SYS.to_string());
    Agent::new(
        provider,
        crate::tool::Registry::new(),
        approver,
        tool_ctx,
        cfg,
        model,
    )
}

/// Build the claim EXTRACTOR: no tools, for the same reason the asker has
/// none. An extractor that can search begins checking as it reads, and
/// returns a filtered list rather than a faithful one — the claims most
/// worth auditing are exactly the ones it would quietly drop.
pub fn extractor(
    provider: Box<dyn crate::provider::Provider>,
    tool_ctx: ToolCtx,
    agent_cfg: crate::config::AgentConfig,
    model: Option<String>,
) -> Result<Agent> {
    let approver = Arc::new(crate::tool::ModeApprover {
        mode: crate::config::PermissionMode::ReadOnly,
    });
    let mut cfg = agent_cfg;
    cfg.system_prompt = Some(EXTRACT_SYS.to_string());
    Agent::new(
        provider,
        crate::tool::Registry::new(),
        approver,
        tool_ctx,
        cfg,
        model,
    )
}

/// Build the VERIFIER: the whole graph, and still no way to send.
///
/// The one participant deliberately not given a lens. Readers are narrowed
/// so that their agreement means something; an adjudicator narrowed the
/// same way would just be a third witness. It gets `kg_verify` — pkg's
/// deterministic tier, which dereferences a stored claim to the evidence
/// cited for it with no model in the loop — and an unrestricted
/// `kg_search`.
///
/// The interlock is unchanged by the wider view: both tools are read-only,
/// the registry holds nothing outbound, and the approver answers from
/// policy. It reads more and can still tell nobody.
pub fn verifier(
    provider: Box<dyn crate::provider::Provider>,
    client: Arc<McpClient>,
    tool_ctx: ToolCtx,
    agent_cfg: crate::config::AgentConfig,
    model: Option<String>,
) -> Result<Agent> {
    let approver = Arc::new(crate::tool::ModeApprover {
        mode: crate::config::PermissionMode::ReadOnly,
    });
    let mut registry = crate::tool::Registry::new();
    registry.insert(Arc::new(GraphTool::verify(Arc::clone(&client))));
    registry.insert(Arc::new(GraphTool::entity(Arc::clone(&client))));
    registry.insert(Arc::new(GraphTool::search_everything(client)));
    let mut cfg = agent_cfg;
    cfg.system_prompt = Some(VERIFY_SYS.to_string());
    Agent::new(provider, registry, approver, tool_ctx, cfg, model)
}

/// One reader's vantage point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vantage {
    /// Short name used in the transcript: "written", "spoken".
    pub label: String,
    pub sources: Vec<String>,
}

/// Build one reader: a lens, one tool, and no way to reach anybody.
///
/// The only supported way to construct a gossip agent, because the
/// guarantees are properties of *how it is assembled* — a caller who built
/// the `Agent` themselves could hand it a mailbox, an outbox route, or an
/// interactive approver and quietly undo all three.
pub struct ReaderSetup {
    pub client: Arc<McpClient>,
    pub vantage: Vantage,
    /// Both readers must be given the SAME window, or a difference between
    /// them is the world having changed rather than the sources disagreeing.
    pub since: String,
    pub until: String,
    pub tool_ctx: ToolCtx,
    pub agent_cfg: crate::config::AgentConfig,
    pub model: Option<String>,
    pub system_prompt: String,
}

pub fn reader(provider: Box<dyn crate::provider::Provider>, setup: ReaderSetup) -> Result<Agent> {
    let ReaderSetup {
        client,
        vantage,
        since,
        until,
        tool_ctx,
        agent_cfg,
        model,
        system_prompt,
    } = setup;
    let mut registry = crate::tool::Registry::new();
    registry.insert(Arc::new(LensedSearch::new(
        client,
        &vantage.label,
        vantage.sources.clone(),
        &since,
        &until,
    )) as Arc<dyn Tool>);

    // Reads are allowed and nothing else is; a non-read-only tool would be
    // BLOCKED with a reason rather than raised as a question to a terminal
    // nobody is sitting at.
    let approver = Arc::new(crate::tool::ModeApprover {
        mode: crate::config::PermissionMode::ReadOnly,
    });

    let mut cfg = agent_cfg;
    cfg.system_prompt = Some(system_prompt);
    Agent::new(provider, registry, approver, tool_ctx, cfg, model)
}

/// What one round produced.
#[derive(Debug, Clone, Serialize)]
pub struct Round {
    pub n: u32,
    /// The question each reader was asked, keyed by vantage label.
    pub asked: Vec<(String, String)>,
    /// What each committed, before seeing the other.
    pub answered: Vec<(String, String)>,
    /// Readers whose asker produced nothing usable, paired with what it did
    /// emit. Recorded rather than hidden: a repeated round reads like a
    /// reader that changed its mind, when in fact the dialogue stalled and
    /// the orchestration papered over it — the first run with this field
    /// showed every asker failing in every round, which the transcript had
    /// been quietly presenting as three rounds of conversation.
    ///
    /// The rejected text is kept because "produced no question" is not a
    /// diagnosis. Empty output, a refusal and a paragraph of prose that
    /// happens to end in a full stop are three different bugs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stalled: Vec<(String, String)>,
}

/// The whole exchange about one entity.
#[derive(Debug, Clone, Serialize)]
pub struct Exchange {
    pub entity: String,
    pub vantages: Vec<Vantage>,
    pub rounds: Vec<Round>,
}

/// System prompt for a reader answering from its own sources.
pub const ANSWER_SYS: &str = "\
You and another assistant are each reading DIFFERENT sources about one \
person, comparing notes. Search your sources and answer from what you find.

You are talking to the other assistant, not to a user. Never address the \
user, never offer to look something up, never ask what they need — there is \
nobody there to answer, and an offer is a wasted turn.

At most three sentences. Say plainly and briefly when your sources do not \
cover it: the other assistant may see what you cannot, and 'my sources show \
nothing about that' is a real contribution. Report only what you read — do \
not speculate, and do not pad a thin answer by listing what you would need \
in order to answer.";

/// System prompt for a reader generating a question for the other.
pub const FOLLOWUP_SYS: &str = "\
You and another assistant each read DIFFERENT sources about one person, so \
each of you can see things the other cannot.

You have both just answered and you can see their answer. Ask ONE question \
that THEIR sources might answer and yours cannot — aim at what they seem to \
have seen and you did not. Prefer relationships, roles, commitments and the \
reasons behind things over dates and logistics. If their sources turned up \
nothing, ask instead about something yours hinted at and could not settle.

Output the question and nothing else: one interrogative sentence. Do not \
answer it yourself, do not summarise what was said, do not explain your \
reasoning. You have no tools and nothing to look up — the question IS your \
whole output.";

/// Run one exchange. Deterministic orchestration: the code decides who is
/// asked what and when, so commit-then-reveal cannot be skipped.
///
/// `ask` is the seed both readers start from. Each subsequent round asks
/// each reader the question the *other* generated after seeing its answer.
pub async fn exchange(
    answerers: &[(Vantage, Agent)],
    askers: &[(Vantage, Agent)],
    cx: &RunContext,
    entity: &str,
    seed: &str,
    rounds: u32,
) -> Result<Exchange> {
    anyhow::ensure!(
        answerers.len() == 2,
        "gossip is a pair; got {}",
        answerers.len()
    );
    anyhow::ensure!(askers.len() == 2, "one asker per reader");
    let agents = answerers;
    let mut questions: Vec<String> = vec![seed.to_string(), seed.to_string()];
    let mut out = Exchange {
        entity: entity.to_string(),
        vantages: agents.iter().map(|(v, _)| v.clone()).collect(),
        rounds: vec![],
    };

    // What each reader has already said, so a round can build on the last
    // one. Its OWN answers only. The leak commit-then-reveal exists to
    // prevent is seeing the other's answer before committing; a reader kept
    // blind to itself does not hold a conversation, it draws three
    // independent samples — which is what the third live run produced, the
    // same reader answering the same seed twice with different facts and no
    // sign it had noticed.
    let mut said: Vec<Vec<(String, String)>> = vec![vec![], vec![]];
    let mut stalled: Vec<(String, String)> = vec![];

    for n in 1..=rounds {
        // COMMIT. Both answer before either sees the other.
        let mut answers = Vec::new();
        for (i, (vantage, agent)) in agents.iter().enumerate() {
            let mut prior = String::new();
            for (q, a) in &said[i] {
                prior.push_str(&format!("\nEarlier you were asked: {q}\nYou said: {a}\n"));
            }
            // Framed as a peer speaking, not a user querying an assistant.
            // "Question:" on its own reads as a user prompt, and the model
            // answered it as one — bulleted, exhaustive, closing with an
            // offer of further help.
            let mut convo = Conversation::user(format!(
                "The person is {entity}.{prior}\nThe other assistant asks you: {}",
                questions[i]
            ));
            let outcome = agent
                .run_in(cx, &mut convo, None)
                .await
                .with_context(|| format!("{} reader, round {n}", vantage.label))?;
            let answer = strip_user_directed(outcome.text.trim());
            said[i].push((questions[i].clone(), answer.clone()));
            answers.push(answer);
        }

        out.rounds.push(Round {
            stalled: std::mem::take(&mut stalled),
            n,
            asked: agents
                .iter()
                .enumerate()
                .map(|(i, (v, _))| (v.label.clone(), questions[i].clone()))
                .collect(),
            answered: agents
                .iter()
                .enumerate()
                .map(|(i, (v, _))| (v.label.clone(), answers[i].clone()))
                .collect(),
        });

        if n == rounds {
            break;
        }

        // REVEAL, and only now. Each reader sees the other's answer and asks
        // it something its own sources cannot settle.
        let mut next = questions.clone();
        for (i, (vantage, _)) in agents.iter().enumerate() {
            let other = 1 - i;
            // Trimmed, and the imperative goes LAST. A system prompt
            // followed by two twenty-line answers is overwhelmed by the
            // shape of its own input: labelled answers read as "summarise
            // these", and the asker duly returned a consolidated profile
            // of the person instead of a question. Instruction position
            // beats instruction strength.
            let brief = |s: &String| -> String { s.chars().take(700).collect() };
            let reveal = format!(
                "The person is {entity}.\n\nYou read: {}\nYou answered: {}\n\n\
                 They read: {}\nThey answered: {}\n\n\
                 Now ask them ONE question. Do not summarise either answer. \
                 Your entire output is a single sentence ending in a question \
                 mark.",
                vantage.sources.join(", "),
                brief(&answers[i]),
                agents[other].0.sources.join(", "),
                brief(&answers[other]),
            );
            // A DIFFERENT agent does the asking: same lens, different
            // system prompt. One agent cannot hold two roles, and giving
            // the answerer the asking prompt would mean whichever ran last
            // decided what it was.
            let mut convo = Conversation::user(reveal);
            let mut outcome = askers[i].1.run_in(cx, &mut convo, None).await?;
            // One retry, stripped to the bone. Every asker failed in every
            // round of the run before this, so a single cheap retry is
            // worth more than a round silently repeating its question —
            // and if the bare form fails too, that is a finding rather
            // than a flake.
            if usable_question(&outcome.text).is_none() {
                let mut bare = Conversation::user(format!(
                    "They said this about {entity}: {}\n\n\
                     Ask them one question about it. Output only the question.",
                    brief(&answers[other]),
                ));
                outcome = askers[i].1.run_in(cx, &mut bare, None).await?;
            }
            // A non-question must not propagate. Keeping the previous
            // question repeats a round; feeding garbage forward corrupts
            // every round after it, and the reader answers the garbage
            // earnestly because it cannot tell it was never asked anything.
            match usable_question(&outcome.text) {
                Some(q) => next[other] = q,
                None => stalled.push((
                    agents[other].0.label.clone(),
                    outcome.text.trim().chars().take(300).collect(),
                )),
            }
        }
        questions = next;
    }
    Ok(out)
}

/// Drop trailing lines where the answerer stops reporting and starts
/// serving a user.
///
/// `ANSWER_SYS` already forbids this in as many words, and a 35B local model
/// ignored it in every live round: answers ran to twenty lines and closed
/// with "Would you like me to dig deeper?". The tic is not merely untidy.
/// An answer is the asker's entire input, and in the third live run one
/// reader's closing offer to the user became the other's question verbatim
/// — a politeness reflex promoted to the next agent's task. So it is cut
/// here rather than asked for once more in a prompt the model overrides.
///
/// Only the tail is cut. An answerer never has a legitimate reason to close
/// on a question: asking is the other role, and it has nobody to ask.
pub fn strip_user_directed(text: &str) -> String {
    let serves_a_user = |l: &str| {
        let lower = l.to_lowercase();
        l.ends_with('?')
            || lower.starts_with("let me know")
            || lower.starts_with("would you")
            || lower.starts_with("if you'd like")
            || lower.starts_with("i can look")
            || lower.starts_with("i can dig")
    };
    let mut lines: Vec<&str> = text.lines().collect();
    while let Some(last) = lines.last() {
        let t = last.trim().trim_start_matches(['*', '-', '#', '>', ' ']);
        if t.is_empty() || serves_a_user(t) {
            lines.pop();
        } else {
            break;
        }
    }
    let cut = lines.join("\n").trim().to_string();
    // A model that answers with nothing BUT an offer has said nothing. Say
    // that, rather than passing an empty string on as if it were a silence
    // the sources produced.
    if cut.is_empty() {
        "(no answer — the reader only offered to look things up)".to_string()
    } else {
        cut
    }
}

/// A question aimed at what the addressee *wants* rather than what its
/// sources *hold*.
///
/// The last shape of the assistant reflex to survive into the question
/// slot. A live round-2 asked "What specific aspect of Luke J Chang's work
/// or background are you most interested in?" — grammatically a question,
/// addressed to a peer, and worthless: the other reader has no preferences,
/// only sources, so it burned a whole round explaining that it could not
/// choose. A probe of someone's evidence is the only question worth asking
/// here.
fn elicits_a_preference(line: &str) -> bool {
    let l = line.to_lowercase();
    [
        "interested in",
        "would you like",
        "do you want",
        "should i",
        "can i help",
        "what would you",
        "how can i",
        "anything else",
    ]
    .iter()
    .any(|p| l.contains(p))
}

/// The first line of `text` that is actually a question, or `None`.
///
/// A model with no tools still emits tool syntax when it wants to look
/// something up — a live run produced `tool:pkg__kg_search args:{...}` in
/// the question slot, which the next reader then answered earnestly,
/// because a reader cannot tell it was never asked anything. Removing the
/// asker's tools stopped it *researching*; only validation stops the
/// wreckage of the attempt from propagating.
pub fn usable_question(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| {
            l.ends_with('?')
                && l.len() > 10
                // Tool syntax and JSON fragments are the failure mode, not
                // stray punctuation.
                && !l.contains("tool:")
                && !l.contains("args:")
                && !l.starts_with('{')
                && !elicits_a_preference(l)
        })
        .map(|l| l.trim_start_matches(['*', '-', '#', ' ']).to_string())
}

/// System prompt for the claim extractor. Tool-less on purpose: an
/// extractor that can search starts checking as it reads, and what comes
/// back is a filtered list rather than a faithful one.
pub const EXTRACT_SYS: &str = "\
You are given a transcript in which two assistants discussed one person. \
List the factual claims they made about that person or about the graph's \
records of them.

One claim per line, each a single short sentence that stands on its own — \
resolve pronouns and back-references so a line can be checked without the \
transcript. Include claims you suspect are wrong; judging them is not your \
job. Exclude questions, hedges about what a source failed to contain, and \
statements about the assistants themselves.

Output only the list. No numbering, no headings, no commentary.";

/// System prompt for the adjudicator.
pub const VERIFY_SYS: &str = "\
You check one claim against a knowledge graph. You can see everything: all \
sources, all time, facts as well as evidence.

Use kg_search to look for evidence, and kg_verify to see what the graph \
already believes about an entity and whether its own evidence holds up.

Then answer in exactly this form, two lines:
VERDICT: supported | unsupported | contradicted
BASIS: one sentence, naming what you found

'supported' means you found evidence that actually says this. 'contradicted' \
means the graph or its evidence says otherwise. 'unsupported' means you \
looked and found nothing either way — which is the verdict for anything the \
assistant knew from outside the graph, however true it may be in the world. \
Absence of evidence is 'unsupported', never 'contradicted'.";

/// One claim, checked.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimVerdict {
    pub claim: String,
    pub verdict: String,
    pub basis: String,
}

/// Parse the adjudicator's two-line reply.
///
/// Unparseable output becomes `unchecked` rather than a guess. A verdict
/// invented from prose that merely mentions "supported" is worse than an
/// admitted gap: it launders a model's mood into an audit result.
pub fn parse_verdict(text: &str) -> (String, String) {
    const WORDS: [&str; 3] = ["supported", "unsupported", "contradicted"];
    let word_at = |s: &str| -> Option<String> {
        let v = s.trim().to_lowercase();
        let head = v
            .split(|c: char| !c.is_ascii_alphabetic())
            .find(|w| !w.is_empty())?;
        WORDS.contains(&head).then(|| head.to_string())
    };

    let mut verdict = String::new();
    let mut basis = String::new();
    for line in text.lines() {
        let l = line
            .trim()
            .trim_start_matches(['*', '-', '#', '>', ' '])
            .trim_matches(['*', '`', ' '])
            .to_string();
        let upper = l.to_uppercase();
        if upper.starts_with("VERDICT:") {
            if let Some(w) = word_at(&l[8..]) {
                verdict = w;
            }
        } else if upper.starts_with("BASIS:") {
            basis = l[6..].trim().to_string();
        } else if verdict.is_empty() && WORDS.contains(&l.to_lowercase().as_str()) {
            // A bare verdict alone on its line. Accepting this is not the
            // guessing forbidden above: the entire line is the word, so
            // there is no prose to misread. Prose stays rejected.
            verdict = l.to_lowercase();
        }
    }
    if verdict.is_empty() {
        // Keep what it actually said. "Did not answer in form" is not a
        // diagnosis, and a run in which all eight claims came back
        // unchecked left nothing whatever to work from — exactly the
        // mistake the asker's stall record had already corrected once.
        let said: String = text.trim().chars().take(200).collect();
        return (
            "unchecked".into(),
            if said.is_empty() {
                "the adjudicator said nothing at all".into()
            } else {
                format!("not in form; it said: {}", said.replace('\n', " "))
            },
        );
    }
    (verdict, basis)
}

/// Split the extractor's output into claims.
///
/// The filter earns its place. A tool-less extractor handed a transcript of
/// two agents searching carries on searching: a live run produced
/// "search_query: U034F8HLM7S" and "I will search the knowledge graph
/// for..." in the claim slot, and all three were dutifully sent to the
/// adjudicator. A claim is a statement about the person; an announcement of
/// what the model is about to do is not one, and passing it on wastes a
/// verification call to conclude nothing.
pub fn claim_lines(text: &str, max: usize) -> Vec<String> {
    let is_intent = |l: &str| {
        let lower = l.to_lowercase();
        lower.contains("search_query")
            || lower.contains("tool:")
            || lower.starts_with("i will ")
            || lower.starts_with("i'll ")
            || lower.starts_with("let me ")
            || lower.starts_with("i need to ")
            || lower.starts_with("first, i")
    };
    text.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['*', '-', '#', '•', ' '])
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')')
                .trim()
                .to_string()
        })
        .filter(|l| l.len() > 15 && !l.ends_with(':') && !l.ends_with('?') && !is_intent(l))
        .take(max)
        .collect()
}

/// pkg's deterministic verdicts on its own stored claims about an entity.
///
/// No model in the loop: `kg_verify` dereferences each live claim to the
/// evidence cited for it. This is the one part of an audit that cannot
/// hallucinate, which is why it is reported alongside the model tier rather
/// than folded into it — and why it lives here rather than in the CLI,
/// where reaching into the MCP client to hand-roll a graph call would put a
/// second, unaudited path to the graph in the front end.
pub async fn graph_findings(client: &McpClient, entity: &str) -> Result<String> {
    let out = client
        .call_tool("kg_verify", json!({ "node": entity, "limit": 20 }))
        .await
        .context("kg_verify")?;
    Ok(out.content)
}

/// Audit an exchange: extract what was claimed, then check each claim
/// against the whole graph.
///
/// Runs after the rounds rather than during them, deliberately. A verifier
/// speaking mid-exchange would be a third voice the readers accommodate,
/// and the readers' independence is the only thing making the exchange
/// worth auditing.
pub async fn audit(
    extractor: &Agent,
    verifier: &Agent,
    cx: &RunContext,
    exchange: &Exchange,
    max_claims: usize,
) -> Result<Vec<ClaimVerdict>> {
    // The imperative goes LAST, after the transcript. The same lesson the
    // asker taught and this call initially ignored: a system prompt in
    // front of a long input loses to the shape of that input. The
    // transcript ends with two agents searching, so the extractor carried
    // on searching — it has no tools, and still emitted "search_query:
    // U034F8HLM7S" where a claim belonged.
    let mut convo = Conversation::user(format!(
        "The person is {}.\n\n{}\n\n\
         Now list the factual claims made about {} in the transcript above. \
         One per line. Do not search, do not comment, do not explain — you \
         have no tools and the list is your whole output.",
        exchange.entity,
        render(exchange),
        exchange.entity,
    ));
    let listed = extractor
        .run_in(cx, &mut convo, None)
        .await
        .context("extracting claims from the exchange")?;

    let mut out = Vec::new();
    for claim in claim_lines(&listed.text, max_claims) {
        // Imperative last, again. This is the third role to need it: the
        // required form lived only in VERIFY_SYS, and after a few tool
        // calls the model was far enough from it to answer in prose. Every
        // claim of one run came back unchecked for that reason alone.
        let mut convo = Conversation::user(format!(
            "The person is {}.\n\nClaim to check: {claim}\n\n\
             Search first, then reply with exactly two lines:\n\
             VERDICT: supported | unsupported | contradicted\n\
             BASIS: one sentence naming what you found",
            exchange.entity
        ));
        let res = verifier
            .run_in(cx, &mut convo, None)
            .await
            .with_context(|| format!("checking claim: {claim}"))?;
        let (verdict, basis) = parse_verdict(&res.text);
        out.push(ClaimVerdict {
            claim,
            verdict,
            basis,
        });
    }
    Ok(out)
}

/// Render an audit, findings first — an audit read top-down should hit
/// what is wrong before what is fine.
pub fn render_audit(verdicts: &[ClaimVerdict]) -> String {
    let rank = |v: &str| match v {
        "contradicted" => 0,
        "unsupported" => 1,
        "unchecked" => 2,
        _ => 3,
    };
    let mut sorted: Vec<&ClaimVerdict> = verdicts.iter().collect();
    sorted.sort_by_key(|c| rank(&c.verdict));

    let mut s = String::from("\nAudit\n");
    for c in &sorted {
        s.push_str(&format!(
            "  [{}] {}\n      {}\n",
            c.verdict, c.claim, c.basis
        ));
    }
    let n = |v: &str| verdicts.iter().filter(|c| c.verdict == v).count();
    s.push_str(&format!(
        "  — {} claim(s): {} supported, {} unsupported, {} contradicted, {} unchecked\n",
        verdicts.len(),
        n("supported"),
        n("unsupported"),
        n("contradicted"),
        n("unchecked"),
    ));
    s
}

/// Render an exchange for a distiller or a reader.
pub fn render(x: &Exchange) -> String {
    let mut s = format!("Gossip about {} \n", x.entity);
    for v in &x.vantages {
        s.push_str(&format!("  {} reads: {}\n", v.label, v.sources.join(", ")));
    }
    for r in &x.rounds {
        s.push_str(&format!("\nRound {}\n", r.n));
        for ((who, q), (_, a)) in r.asked.iter().zip(r.answered.iter()) {
            if let Some((_, raw)) = r.stalled.iter().find(|(l, _)| l == who) {
                let raw = if raw.is_empty() {
                    "(nothing at all)".to_string()
                } else {
                    raw.replace('\n', " ")
                };
                s.push_str(&format!(
                    "  ! {who}'s asker produced no question. It emitted: {raw}\n"
                ));
            }
            s.push_str(&format!("  {who} was asked: {q}\n  {who} said: {a}\n"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cov(pairs: &[(&str, i64)]) -> Vec<SourceCoverage> {
        pairs
            .iter()
            .map(|(s, n)| SourceCoverage {
                source: s.to_string(),
                episodes: *n,
            })
            .collect()
    }

    #[test]
    fn an_offer_to_the_user_never_reaches_the_other_reader() {
        // The live failure: reader A closed with an offer of further help,
        // and that offer became reader B's question in the next round.
        let answered = "Slack shows he ran a hyperscanning practice with Rutgers.\n\
             His birthday is May 28.\n\n\
             Would you like me to dig deeper into one of these workstreams?";
        let cut = strip_user_directed(answered);
        assert!(cut.ends_with("His birthday is May 28."));
        assert!(!cut.contains("dig deeper"));
        // And what survives must still be answerable material, not a stub.
        assert!(cut.contains("hyperscanning"));

        // A real silence is preserved — it is a contribution, not a defect.
        assert_eq!(
            strip_user_directed("My sources show nothing about that."),
            "My sources show nothing about that."
        );
        // An answer that is ONLY an offer is not silence, and must not be
        // passed on as though the sources had been consulted.
        assert!(strip_user_directed("Would you like me to search?").starts_with("(no answer"));
    }

    #[test]
    fn a_verdict_is_computed_from_two_sightings_not_asked_for() {
        use Sighting::*;
        assert_eq!(corroboration_verdict(Seen, Seen), "corroborated");
        assert_eq!(corroboration_verdict(Seen, Unseen), "single_source");
        assert_eq!(corroboration_verdict(Unseen, Seen), "single_source");
        assert_eq!(corroboration_verdict(Unseen, Unseen), "unseen");
        // A contradiction outranks agreement in both directions: one source
        // actively denying it matters more than another merely echoing it.
        assert_eq!(corroboration_verdict(Seen, Contradicted), "contradicted");
        assert_eq!(corroboration_verdict(Contradicted, Seen), "contradicted");
        // A mangled reply is our failure, and must never be counted as
        // absence of evidence — that would convert harness bugs into
        // rejections of true claims.
        assert_eq!(corroboration_verdict(Seen, Unclear), "unclear");
        assert_eq!(corroboration_verdict(Unclear, Unseen), "unclear");
    }

    #[test]
    fn a_sighting_is_parsed_or_admitted() {
        let (s, cite) = parse_sighting("SIGHTING: SEEN\nCITE: slack #random, 2026-05-28");
        assert_eq!(s, Sighting::Seen);
        assert_eq!(cite, "slack #random, 2026-05-28");
        assert_eq!(
            parse_sighting("SIGHTING: UNSEEN\nCITE: nothing").0,
            Sighting::Unseen
        );
        assert_eq!(
            parse_sighting("**SIGHTING:** CONTRADICTED").0,
            Sighting::Contradicted
        );
        // A bare word alone on a line is a format.
        assert_eq!(parse_sighting("UNSEEN").0, Sighting::Unseen);
        // Prose that merely contains the word is not.
        let (s, basis) = parse_sighting("I have not seen anything like this claim.");
        assert_eq!(s, Sighting::Unclear);
        assert!(
            basis.contains("I have not seen"),
            "the rejected text is kept"
        );
    }

    #[test]
    fn corroboration_never_reads_the_source_it_came_from() {
        // A reader that can see the episode a claim was extracted from will
        // find the claim there and call it corroborated: the same witness
        // twice, which is the facts-versus-evidence failure one level up.
        let spread = cov(&[
            ("bee.conversation", 40),
            ("bee.daily", 35),
            ("slack.thread", 30),
            ("reflect.daily", 20),
        ]);
        // The whole FAMILY goes, not just the one source. bee.daily is a
        // summary of bee.conversation: excluding only the exact origin
        // would let a claim be corroborated by a digest of itself.
        let (a, b) = vantages_excluding(&spread, Some("bee.conversation"), 3).unwrap();
        for v in [&a, &b] {
            assert!(!v.sources.iter().any(|s| s.starts_with("bee.")), "{v:?}");
        }
        // A proposer works in place of a source, because Bee's fact API
        // stages 200 candidates with no originating episode at all and the
        // prefix is then the only honest record of where they came from.
        let (a, b) = vantages_excluding(&spread, Some("bee:suggested"), 3).unwrap();
        for v in [&a, &b] {
            assert!(!v.sources.iter().any(|s| s.starts_with("bee.")), "{v:?}");
        }
        // With nothing else covering the subject there is no pair — better
        // no verdict than a self-corroborating one.
        let only = cov(&[("bee.conversation", 40), ("bee.daily", 9)]);
        assert!(vantages_excluding(&only, Some("bee:suggested"), 3).is_none());
    }

    #[test]
    fn every_origin_bars_its_own_family_not_a_reconstruction() {
        // Regression: the origin's family used to be rebuilt from its head
        // token (`slack.thread` → "slack." → "other"), which barred nothing
        // and left the origin itself eligible as its own witness for every
        // Slack, calendar, mail, and llm-proposed candidate.
        let spread = cov(&[
            ("slack.thread", 40),
            ("bee.conversation", 30),
            ("calendar.event", 20),
        ]);
        for origin in ["slack.thread", "mbox", "email.thread"] {
            let (a, b) = vantages_excluding(&spread, Some(origin), 3).unwrap();
            for v in [&a, &b] {
                assert!(
                    !v.sources.iter().any(|s| s == "slack.thread"),
                    "origin {origin} left its own family eligible: {v:?}"
                );
            }
        }
        let (a, b) = vantages_excluding(&spread, Some("calendar.event"), 3).unwrap();
        for v in [&a, &b] {
            assert!(!v.sources.iter().any(|s| s == "calendar.event"), "{v:?}");
        }
        // An extractor proposer names no source at all. With the origin
        // unknowable, refusing is the only answer that cannot let a claim
        // vote for itself.
        assert!(vantages_excluding(&spread, Some("llm:commitment"), 3).is_none());
        assert!(vantages_excluding(&spread, Some("llm"), 3).is_none());
        // agent:mecha is a real source whose name happens to hold a colon.
        assert_eq!(family_of_origin("agent:mecha"), Some("agentic"));
    }

    #[test]
    fn an_unparseable_verdict_is_never_guessed() {
        assert_eq!(
            parse_verdict("VERDICT: contradicted\nBASIS: the graph lists one node."),
            ("contradicted".into(), "the graph lists one node.".into())
        );
        // Prose that merely mentions a verdict word must not become one:
        // laundering a model's mood into an audit result is worse than an
        // admitted gap.
        let (v, basis) = parse_verdict("I think this is probably supported by the Slack thread.");
        assert_eq!(v, "unchecked");
        // And the rejected text is kept: "did not answer in form" is not a
        // diagnosis, as a run of eight unchecked claims demonstrated.
        assert!(basis.contains("I think this is probably supported"));

        // A bare verdict alone on its line is a format, not prose.
        assert_eq!(parse_verdict("supported").0, "supported");
        assert_eq!(parse_verdict("**VERDICT:** contradicted").0, "contradicted");
        assert_eq!(
            parse_verdict("Verdict: unsupported\nBasis: nothing found").0,
            "unsupported"
        );
        assert_eq!(parse_verdict("").0, "unchecked");
        // A verdict outside the vocabulary is not a verdict.
        assert_eq!(
            parse_verdict("VERDICT: mostly true\nBASIS: x").0,
            "unchecked"
        );
    }

    #[test]
    fn claim_extraction_drops_scaffolding() {
        let listed = "**Claims:**\n\
             1. Luke J Chang works at Dartmouth.\n\
             - py-feat is a tool for fNIRS analysis.\n\
             Is he the lab PI?\n\
             short\n\
             He maintains the /home/ljchang/Git directory.";
        let claims = claim_lines(listed, 8);
        assert_eq!(claims.len(), 3, "got {claims:?}");
        assert!(claims[0].starts_with("Luke J Chang works"));
        assert!(
            !claims.iter().any(|c| c.ends_with('?')),
            "questions are not claims"
        );
        assert!(!claims.iter().any(|c| c.contains("Claims:")));
        // The cap is a cap, not a suggestion.
        assert_eq!(claim_lines(listed, 2).len(), 2);

        // The live failure: a tool-less extractor announcing searches, and
        // every one of them sent to the adjudicator as a claim.
        let intent = "I will search the knowledge graph for the Slack handle U034F8HLM7S.\n\
             search_query: ljchang@email.arizona.edu\n\
             Let me check whether the two entities are distinct.\n\
             Luke J Chang presented a poster on April 19, 2026.";
        let claims = claim_lines(intent, 8);
        assert_eq!(
            claims,
            vec!["Luke J Chang presented a poster on April 19, 2026."]
        );
    }

    #[test]
    fn a_question_must_probe_sources_not_preferences() {
        // Live round 2. Grammatical, addressed to the peer, and worthless:
        // the other reader has no interests, only evidence.
        assert_eq!(
            usable_question(
                "What specific aspect of Luke J Chang's work or background \
                 are you most interested in?"
            ),
            None
        );
        assert_eq!(usable_question("Would you like me to dig deeper?"), None);
        // The good ones from the same run must survive. Both are second
        // person, so the rule cannot simply reject "you".
        for q in [
            "Are you referring to the Luke J Chang associated with the Chang \
             lab at Dartmouth and the 'py-feat' paper?",
            "Can you confirm if Luke J Chang is associated with the Chang lab?",
        ] {
            assert!(usable_question(q).is_some(), "rejected a real probe: {q}");
        }
    }

    #[test]
    fn a_non_question_never_propagates() {
        // The live failure: an asker with no tools still emitted tool
        // syntax, which became the next round's "question" and was
        // answered earnestly.
        assert_eq!(
            usable_question("tool:pkg__kg_search\nargs:{\"query\": \"ljchang\"}"),
            None
        );
        assert_eq!(
            usable_question("Based on my searches, here is what I found:"),
            None
        );
        assert_eq!(usable_question(""), None);
        assert_eq!(
            usable_question("ok?"),
            None,
            "too short to be a real question"
        );

        // A real question survives, and decoration is trimmed.
        assert_eq!(
            usable_question("Who does she collaborate with on the grant?").as_deref(),
            Some("Who does she collaborate with on the grant?")
        );
        assert_eq!(
            usable_question("Some preamble.\n- What role does he hold in the lab?").as_deref(),
            Some("What role does he hold in the lab?"),
            "the question is found past preamble and stripped of its bullet"
        );
    }

    #[test]
    fn vantages_prefer_independence_over_volume() {
        // Slack and mbox are both "written" — two slices of one witness.
        // The calendar is a different kind of account, so it wins the second
        // seat despite having fewer episodes.
        let c = cov(&[("slack.thread", 400), ("mbox", 300), ("calendar.event", 50)]);
        let (a, b) = choose_vantages(&c, 3).unwrap();
        assert_eq!(a.sources, vec!["slack.thread"]);
        assert_eq!(
            b.sources,
            vec!["calendar.event"],
            "a second family beats a bigger sibling"
        );
        assert_ne!(a.label, b.label);
    }

    #[test]
    fn a_thin_source_is_not_a_witness() {
        // The live shape that motivated this: 493 slack episodes and 2 bee
        // conversations. Handing a reader the 2 produces a confident "I do
        // not know" that reads like a finding rather than an empty shelf.
        let c = cov(&[("slack.thread", 493), ("bee.conversation", 2)]);
        assert!(
            choose_vantages(&c, 3).is_none(),
            "one witness and a silence is not a pair"
        );
        // Lower the floor and it becomes a legitimate, if lopsided, pair.
        assert!(choose_vantages(&c, 2).is_some());
    }

    #[test]
    fn same_family_is_better_than_no_pair() {
        // Independence is preferred, not required: two written sources still
        // beat refusing to run.
        let c = cov(&[("slack.thread", 40), ("mbox", 30)]);
        let (a, b) = choose_vantages(&c, 3).unwrap();
        assert_eq!(
            (a.sources[0].as_str(), b.sources[0].as_str()),
            ("slack.thread", "mbox")
        );
    }

    #[test]
    fn families_split_the_kinds_of_account_apart() {
        assert_eq!(family("bee.conversation"), family("bee.daily"));
        assert_ne!(family("calendar.event"), family("slack.thread"));
        assert_ne!(family("reflect.note"), family("bee.conversation"));
        // bee.daily is a digest of bee.conversation — one witness. A Reflect
        // note and a Reflect daily are separately written accounts, so they
        // may serve as each other's witness.
        assert_ne!(family("reflect.note"), family("reflect.daily"));
    }

    #[test]
    fn lensed_search_hides_what_it_pins() {
        // The point of the wrapper: a child cannot widen its own lens,
        // because the schema it is shown has no way to name the lens.
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What to look for"},
                "k": {"type": "integer", "description": "Max results (default 10)"}
            },
            "required": ["query"]
        });
        let props = schema["properties"].as_object().unwrap();
        for pinned in ["sources", "since", "until", "include_private"] {
            assert!(
                !props.contains_key(pinned),
                "{pinned} must not be nameable by the child"
            );
        }
    }

    #[test]
    fn the_readonly_approver_blocks_rather_than_asks() {
        // The guarantee the owner asked for: a gossip run cannot put a
        // question to them. ModeApprover in ReadOnly answers from policy —
        // anything not read-only is Blocked with a reason, and nothing is
        // ever raised to a terminal nobody is sitting at.
        let a = crate::tool::ModeApprover {
            mode: crate::config::PermissionMode::ReadOnly,
        };
        assert!(matches!(a.mode, crate::config::PermissionMode::ReadOnly));
        // Ask mode would be the wrong choice here: it blocks too, but its
        // message invites `--yes`, which is exactly the escape hatch a
        // background measurement must not have.
        assert!(!matches!(a.mode, crate::config::PermissionMode::Ask));
    }

    #[test]
    fn a_reader_declares_private_and_untrusted_but_never_send() {
        // Both halves of the trifecta enter a gossip child by design — it
        // reads the user's life, and episode bodies are third-party text.
        // The third leg must therefore never be handed over, or the pair
        // becomes an exfiltration path.
        let caps = Capabilities {
            private_data: true,
            untrusted_input: true,
            external_send: false,
            destructive: false,
        };
        assert!(caps.private_data && caps.untrusted_input);
        assert!(
            !caps.external_send,
            "a gossip reader with a way to send is the leak the interlock exists for"
        );
    }
}

// ─── Corroboration: is a generalisation more than its one transcript? ────────

/// A pending fact candidate, as the queue hands it over.
#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub candidate_id: i64,
    pub statement: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub origin_source: Option<String>,
    /// The subject was guessed from an ambiguous name match. Carried
    /// through rather than hidden: the guess is usually right and the
    /// ambiguity is usually a duplicate identity worth fixing at the root.
    #[serde(default)]
    pub subject_ambiguous: bool,
    #[serde(default)]
    pub confidence: Option<f64>,
    /// The origin episode, when `pending` was asked for evidence — what
    /// the verification mechanism judges the claim against.
    #[serde(default)]
    pub evidence: Option<EvidenceClip>,
}

/// The origin episode as `kg_pending include_evidence` hands it over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceClip {
    pub source: String,
    #[serde(default)]
    pub occurred_at: String,
    pub body: String,
}

/// One class of the review queue, oldest first.
/// `unjudged_by`: name the mechanism to skip candidates it has already
/// filed a verdict on — a batch run then extends coverage instead of
/// re-judging the same oldest N (pkg keeps verdict history, so re-judging
/// duplicates opinions).
pub async fn pending(
    client: &McpClient,
    proposed_by: &str,
    predicate: &str,
    limit: usize,
    unjudged_by: Option<&str>,
    include_evidence: bool,
) -> Result<Vec<Candidate>> {
    let out = client
        .call_tool(
            "kg_pending",
            json!({
                "proposed_by": proposed_by,
                "predicate": predicate,
                "limit": limit,
                "unjudged_by": unjudged_by,
                "include_evidence": include_evidence,
            }),
        )
        .await
        .context("kg_pending")?;
    let body: Value = serde_json::from_str(&out.content)
        .with_context(|| format!("kg_pending returned non-JSON: {}", out.content))?;
    if let Some(e) = body.get("error").and_then(Value::as_str) {
        anyhow::bail!("kg_pending: {e}");
    }
    Ok(serde_json::from_value(body["items"].clone()).unwrap_or_default())
}

/// File an opinion beside a candidate. Decides nothing.
pub async fn file_verdict(
    client: &McpClient,
    candidate_id: i64,
    mechanism: &str,
    verdict: &str,
    basis: &str,
    model: Option<&str>,
) -> Result<()> {
    client
        .call_tool(
            "kg_verdict",
            json!({
                "candidate_id": candidate_id, "mechanism": mechanism,
                "verdict": verdict, "basis": basis, "model": model,
            }),
        )
        .await
        .context("kg_verdict")?;
    Ok(())
}

/// What one reader found in its own sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Sighting {
    Seen,
    Unseen,
    Contradicted,
    /// The reader did not answer in form. Not the same as `Unseen`: a
    /// mangled reply is our failure, and counting it as absence of evidence
    /// would quietly convert harness bugs into rejections.
    Unclear,
}

pub fn parse_sighting(text: &str) -> (Sighting, String) {
    let mut sighting = None;
    let mut basis = String::new();
    for line in text.lines() {
        let l = line
            .trim()
            .trim_start_matches(['*', '-', '#', '>', ' '])
            .trim_matches(['*', '`', ' '])
            .to_string();
        let upper = l.to_uppercase();
        let body = upper.strip_prefix("SIGHTING:").map(str::trim);
        let word = body.unwrap_or(&upper);
        if (body.is_some() || sighting.is_none()) && sighting.is_none() {
            let head = word
                .split(|c: char| !c.is_ascii_alphabetic())
                .find(|w| !w.is_empty())
                .unwrap_or_default();
            // A bare word alone on its line counts, exactly as a bare
            // verdict does; prose containing the word does not.
            if body.is_some() || word.trim() == head {
                sighting = match head {
                    "SEEN" => Some(Sighting::Seen),
                    "UNSEEN" => Some(Sighting::Unseen),
                    "CONTRADICTED" => Some(Sighting::Contradicted),
                    _ => None,
                };
            }
        }
        if let Some(rest) = l.strip_prefix("CITE:").or(l.strip_prefix("Cite:")) {
            basis = rest.trim().to_string();
        }
    }
    match sighting {
        Some(s) => (s, basis),
        None => (
            Sighting::Unclear,
            format!("not in form; it said: {}", {
                let t: String = text.trim().chars().take(160).collect();
                t.replace('\n', " ")
            }),
        ),
    }
}

/// The verdict, computed from two sightings rather than asked for.
///
/// Deliberately code and not a third model call. The whole value of
/// commit-then-reveal is that two independent judgements were formed; a
/// model asked to "summarise the verdict" can and will overrule them, and
/// then the independence bought nothing.
pub fn corroboration_verdict(a: Sighting, b: Sighting) -> &'static str {
    use Sighting::*;
    match (a, b) {
        // A contradiction anywhere outranks agreement: one source actively
        // denying it matters more than another merely echoing it.
        (Contradicted, _) | (_, Contradicted) => "contradicted",
        (Seen, Seen) => "corroborated",
        (Seen, Unseen) | (Unseen, Seen) => "single_source",
        (Unseen, Unseen) => "unseen",
        // Anything touching Unclear is not a finding about the world.
        _ => "unclear",
    }
}

pub const SIGHT_SYS: &str = "\
You are checking whether a claim about a person shows up in YOUR sources. \
Another assistant is checking DIFFERENT sources.

The claim came from somewhere else entirely; your job is not to judge \
whether it sounds right, but whether your own evidence shows it. Search, \
then answer. 'UNSEEN' is the honest and expected answer for most claims, \
and it is a real contribution — a generalisation drawn from one \
conversation and visible nowhere else is exactly what needs finding.

Reply in exactly this form, two lines:
SIGHTING: SEEN | UNSEEN | CONTRADICTED
CITE: what you found, or 'nothing' — quote or name the episode

CONTRADICTED means your evidence shows the opposite, not merely that it is \
absent. Absence is UNSEEN.";

/// One candidate, judged by two readers on sources that exclude its origin.
#[derive(Debug, Clone, Serialize)]
pub struct Corroboration {
    pub candidate_id: i64,
    pub statement: String,
    pub verdict: &'static str,
    pub sightings: Vec<(String, Sighting, String)>,
    pub rechecked: bool,
    /// The dissenter's answer BEFORE the reveal, when one happened. A flip
    /// from Unseen after being shown the other's citation and an independent
    /// Seen are different findings — overwriting the first look erased the
    /// distinction, and it is exactly the datum rule-derivation needs.
    pub pre_reveal: Option<(String, Sighting, String)>,
}

/// Commit, then reveal, then compute.
///
/// The reveal is narrower than in an open exchange, and deliberately: only
/// a lone dissenter is shown the other's citation, and only to look again
/// at its OWN sources. Showing both readers everything would let the one
/// with nothing simply agree, which is the conformity commit-then-reveal
/// exists to prevent.
pub async fn corroborate(
    readers: &[(Vantage, Agent)],
    cx: &RunContext,
    cand: &Candidate,
) -> Result<Corroboration> {
    anyhow::ensure!(readers.len() == 2, "corroboration is a pair");
    let ask = format!(
        "Claim to check against your sources:\n\n{}\n\n\
         Search your sources, then reply with exactly two lines:\n\
         SIGHTING: SEEN | UNSEEN | CONTRADICTED\n\
         CITE: what you found, or 'nothing'",
        cand.statement
    );

    let mut found = Vec::new();
    for (v, agent) in readers {
        let mut convo = Conversation::user(ask.clone());
        let out = agent
            .run_in(cx, &mut convo, None)
            .await
            .with_context(|| format!("{} reader on candidate {}", v.label, cand.candidate_id))?;
        let (s, basis) = parse_sighting(&out.text);
        // The SOURCE, not just the family label. Two readers both labelled
        // "reflected" read reflect.note (2,263 episodes) and reflect.daily
        // (118) — different sources entirely — and a live run showed one
        // "reflected" seeing Flowmail and another not, which reads as a
        // mechanism contradicting itself until you can see it was never
        // the same shelf.
        found.push((format!("{} [{}]", v.label, v.sources.join(",")), s, basis));
    }

    // REVEAL, only on a split, and only to the dissenter. Being pointed at
    // something is how a second look differs from a first — the reader may
    // hold the same episode under a wording its own query never reached.
    let mut rechecked = false;
    let mut pre_reveal = None;
    let seen_at = found.iter().position(|(_, s, _)| *s == Sighting::Seen);
    let unseen_at = found.iter().position(|(_, s, _)| *s == Sighting::Unseen);
    if let (Some(hit), Some(miss)) = (seen_at, unseen_at) {
        rechecked = true;
        pre_reveal = Some(found[miss].clone());
        let mut convo = Conversation::user(format!(
            "Claim: {}\n\nAnother assistant, reading {}, found this:\n{}\n\n\
             Search YOUR sources once more with that in mind. Do not take \
             their word for it — report only what your own evidence shows.\n\
             SIGHTING: SEEN | UNSEEN | CONTRADICTED\n\
             CITE: what you found, or 'nothing'",
            cand.statement,
            readers[hit].0.sources.join(", "),
            found[hit].2,
        ));
        let out = readers[miss].1.run_in(cx, &mut convo, None).await?;
        let (s, basis) = parse_sighting(&out.text);
        found[miss] = (
            format!(
                "{} [{}]",
                readers[miss].0.label,
                readers[miss].0.sources.join(",")
            ),
            s,
            basis,
        );
    }

    Ok(Corroboration {
        candidate_id: cand.candidate_id,
        statement: cand.statement.clone(),
        verdict: corroboration_verdict(found[0].1, found[1].1),
        sightings: found,
        rechecked,
        pre_reveal,
    })
}

/// Sources that may serve as a vantage for a candidate: everything the
/// subject is covered by, minus the FAMILY the claim came from.
///
/// Family, not source. `bee.conversation` and `bee.daily` are one witness
/// wearing two labels, and excluding only the exact origin would let a
/// claim taken from a Bee transcript be corroborated by the Bee daily
/// summary of that same transcript.
///
/// `origin` takes either a source (`bee.conversation`) or a proposer
/// (`bee:suggested`), because many candidates have no originating episode
/// at all — Bee's fact API stages 200 with a null episode_id — and the
/// proposer prefix is then the only honest record of where they came from.
/// An origin whose family cannot be determined refuses outright: no verdict
/// is strictly better than one the origin may have voted in.
pub fn vantages_excluding(
    coverage: &[SourceCoverage],
    origin: Option<&str>,
    min: i64,
) -> Option<(Vantage, Vantage)> {
    let barred = match origin {
        Some(o) => match family_of_origin(o) {
            Some(f) => Some(f),
            None => return None,
        },
        None => None,
    };
    let kept: Vec<SourceCoverage> = coverage
        .iter()
        .filter(|c| barred != Some(family(&c.source)))
        .cloned()
        .collect();
    choose_vantages(&kept, min)
}

// ─── Verification: does the evidence a claim cites actually say it? ──────────
//
// Corroboration asks whether a claim holds BEYOND its origin; this asks the
// prior question — whether the origin ever said it. Complementary by
// construction: bee:suggested candidates cite no episode and can only be
// corroborated, llm-extracted candidates cite exactly one and can be vetted
// against it. No search, no tools, one model call per candidate: the
// evidence is handed over, not hunted for.

/// What vetting a claim against its own origin can conclude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Vet {
    /// The evidence says what the claim says.
    Supported,
    /// The evidence does not contain it. Judged against THIS evidence only —
    /// the claim may still be true; that question belongs to corroboration.
    Unsupported,
    /// The evidence shows the statement — about someone else. The wearable's
    /// diarization credits unknown speakers to the owner, so a claim wearing
    /// the wrong name is a distinct and common failure, and it is repaired
    /// by rebinding, not rejection.
    Misattributed,
    /// The evidence shows something weaker or narrower than the claim.
    Overreach,
    /// The judge did not answer in form. A harness failure, never a finding.
    Unclear,
}

impl Vet {
    pub fn as_str(self) -> &'static str {
        match self {
            Vet::Supported => "supported",
            Vet::Unsupported => "unsupported",
            Vet::Misattributed => "misattributed",
            Vet::Overreach => "overreach",
            Vet::Unclear => "unclear",
        }
    }
}

pub const VET_SYS: &str = "\
You judge whether a piece of evidence supports a claim that was extracted \
from it. Only the evidence in front of you counts — no outside knowledge, \
no guessing at what other conversations might show. You will be told the \
exact reply form; keep to it.";

/// Build the vet judge: no tools, no graph, nothing but the handed evidence.
///
/// Deliberately blind for the same reason the corroboration readers are
/// lensed: a judge that can search the graph will find the claim there —
/// extraction put it there — and call that support.
pub fn vet_judge(
    provider: Box<dyn crate::provider::Provider>,
    tool_ctx: ToolCtx,
    agent_cfg: crate::config::AgentConfig,
    model: Option<String>,
) -> Result<Agent> {
    let approver = Arc::new(crate::tool::ModeApprover {
        mode: crate::config::PermissionMode::ReadOnly,
    });
    let mut cfg = agent_cfg;
    cfg.system_prompt = Some(VET_SYS.to_string());
    Agent::new(
        provider,
        crate::tool::Registry::new(),
        approver,
        tool_ctx,
        cfg,
        model,
    )
}

/// The question, evidence first and the imperative LAST — with a long input
/// a local model keeps the instruction it read most recently.
pub fn vet_question(cand: &Candidate, ev: &EvidenceClip) -> String {
    format!(
        "Evidence — one episode from {}, {}:\n\n---\n{}\n---\n\n\
         Claim extracted from that evidence:\n\n  {}\n{}\n\
         Judge whether THIS evidence supports THAT claim. Absence from the \
         evidence is UNSUPPORTED even if the claim sounds plausible. If the \
         evidence shows the statement but credits it to a different person \
         than the claim's subject, that is MISATTRIBUTED. If the evidence \
         shows a weaker or narrower version, that is OVERREACH.\n\n\
         Reply in exactly this form:\n\
         VERDICT: SUPPORTED | UNSUPPORTED | MISATTRIBUTED | OVERREACH\n\
         WHO: only for MISATTRIBUTED — who the evidence actually shows\n\
         QUOTE: the evidence line that decides it, or 'nothing'",
        ev.source,
        ev.occurred_at,
        ev.body,
        cand.statement,
        cand.subject
            .as_deref()
            .map(|s| format!("  (subject: {s})\n"))
            .unwrap_or_default(),
    )
}

/// Parse the judge's reply; an out-of-form reply is `Unclear` and the
/// rejected text is kept — \"did not answer in form\" is not a diagnosis.
pub fn parse_vet(text: &str) -> (Vet, Option<String>, String) {
    let mut verdict = None;
    let mut who = None;
    let mut quote = String::new();
    for line in text.lines() {
        let l = line
            .trim()
            .trim_start_matches(['*', '-', '#', '>', ' '])
            .trim_matches(['*', '`', ' '])
            .to_string();
        let upper = l.to_uppercase();
        let body = upper.strip_prefix("VERDICT:").map(str::trim);
        let word = body.unwrap_or(&upper);
        if verdict.is_none() {
            let head = word
                .split(|c: char| !c.is_ascii_alphabetic())
                .find(|w| !w.is_empty())
                .unwrap_or_default();
            // A bare word alone on its line counts; prose containing the
            // word does not.
            if body.is_some() || word.trim() == head {
                verdict = match head {
                    "SUPPORTED" => Some(Vet::Supported),
                    "UNSUPPORTED" => Some(Vet::Unsupported),
                    "MISATTRIBUTED" => Some(Vet::Misattributed),
                    "OVERREACH" => Some(Vet::Overreach),
                    _ => None,
                };
            }
        }
        if let Some(rest) = l.strip_prefix("WHO:").or(l.strip_prefix("Who:")) {
            let w = rest.trim();
            if !w.is_empty() && !w.eq_ignore_ascii_case("n/a") {
                who = Some(w.to_string());
            }
        }
        if let Some(rest) = l.strip_prefix("QUOTE:").or(l.strip_prefix("Quote:")) {
            quote = rest.trim().to_string();
        }
    }
    match verdict {
        Some(v) => (v, who, quote),
        None => (
            Vet::Unclear,
            None,
            format!("not in form; it said: {}", {
                let t: String = text.trim().chars().take(160).collect();
                t.replace('\n', " ")
            }),
        ),
    }
}

/// One candidate, judged against the evidence it cites.
#[derive(Debug, Clone, Serialize)]
pub struct Vetting {
    pub candidate_id: i64,
    pub statement: String,
    pub verdict: Vet,
    /// For `Misattributed`: who the evidence actually shows. The repair is
    /// a rebind (review `b`), so the name is the finding.
    pub who: Option<String>,
    pub quote: String,
}

/// Judge one candidate against its origin evidence. Errors when the
/// candidate carries none — the caller should have skipped it.
pub async fn vet(agent: &Agent, cx: &RunContext, cand: &Candidate) -> Result<Vetting> {
    let ev = cand
        .evidence
        .as_ref()
        .context("candidate has no origin evidence to vet against")?;
    let mut convo = Conversation::user(vet_question(cand, ev));
    let out = agent
        .run_in(cx, &mut convo, None)
        .await
        .with_context(|| format!("vet judge on candidate {}", cand.candidate_id))?;
    let (verdict, who, quote) = parse_vet(&out.text);
    Ok(Vetting {
        candidate_id: cand.candidate_id,
        statement: cand.statement.clone(),
        verdict,
        who,
        quote,
    })
}

#[cfg(test)]
mod vet_tests {
    use super::*;

    #[test]
    fn a_vet_verdict_is_parsed_or_admitted() {
        let (v, who, quote) =
            parse_vet("VERDICT: MISATTRIBUTED\nWHO: Eunice\nQUOTE: Eunice said she prefers DIY.");
        assert_eq!(v, Vet::Misattributed);
        assert_eq!(who.as_deref(), Some("Eunice"));
        assert!(quote.contains("prefers DIY"));

        assert_eq!(parse_vet("SUPPORTED").0, Vet::Supported, "a bare word alone is a format");
        assert_eq!(parse_vet("**VERDICT:** OVERREACH").0, Vet::Overreach);

        // Prose containing a verdict word is not a verdict, and the
        // rejected text is kept.
        let (v, _, quote) = parse_vet("I believe this is supported by the transcript.");
        assert_eq!(v, Vet::Unclear);
        assert!(quote.contains("I believe"), "the rejected text is kept");
    }

    #[test]
    fn the_question_puts_the_imperative_last() {
        // The harness lesson that cost the most reruns: with a long input a
        // 35B keeps the instruction it read most recently, so the evidence
        // must come first and the reply form last.
        let cand = Candidate {
            candidate_id: 1,
            statement: "Luke prefers DIY.".into(),
            subject: Some("Luke J Chang".into()),
            origin_source: None,
            subject_ambiguous: false,
            confidence: None,
            evidence: Some(EvidenceClip {
                source: "bee.conversation".into(),
                occurred_at: "2026-08-01".into(),
                body: "a long transcript".into(),
            }),
        };
        let q = vet_question(&cand, cand.evidence.as_ref().unwrap());
        let ev_at = q.find("a long transcript").unwrap();
        let claim_at = q.find("Luke prefers DIY.").unwrap();
        let form_at = q.rfind("VERDICT:").unwrap();
        assert!(ev_at < claim_at && claim_at < form_at);
    }
}
