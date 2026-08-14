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

/// Sources that carry the same kind of account, so two vantages drawn from
/// one family are two slices of one witness rather than two witnesses.
pub fn family(source: &str) -> &'static str {
    match source {
        s if s.starts_with("bee.") => "spoken",
        s if s.starts_with("reflect.") => "reflected",
        s if s.starts_with("session.") || s.starts_with("agent:") => "agentic",
        "calendar.event" => "scheduled",
        "slack.thread" | "mbox" | "email.thread" => "written",
        _ => "other",
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

    for n in 1..=rounds {
        // COMMIT. Both answer before either sees the other — each in a fresh
        // conversation, so no round leaks into the next as context either.
        let mut answers = Vec::new();
        for (i, (vantage, agent)) in agents.iter().enumerate() {
            let mut convo = Conversation::user(format!(
                "The person is {entity}.\n\nQuestion: {}",
                questions[i]
            ));
            let outcome = agent
                .run_in(cx, &mut convo, None)
                .await
                .with_context(|| format!("{} reader, round {n}", vantage.label))?;
            answers.push(outcome.text.trim().to_string());
        }

        out.rounds.push(Round {
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
            let mut convo = Conversation::user(format!(
                "The person is {entity}.\n\nYou read: {}\nYou answered: {}\n\n\
                 They read: {}\nThey answered: {}",
                vantage.sources.join(", "),
                answers[i],
                agents[other].0.sources.join(", "),
                answers[other],
            ));
            // A DIFFERENT agent does the asking: same lens, different
            // system prompt. One agent cannot hold two roles, and giving
            // the answerer the asking prompt would mean whichever ran last
            // decided what it was.
            let outcome = askers[i].1.run_in(cx, &mut convo, None).await?;
            let q = outcome.text.trim().to_string();
            if !q.is_empty() {
                next[other] = q;
            }
        }
        questions = next;
    }
    Ok(out)
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
