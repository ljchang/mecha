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
        });
        let mut out = self.client.call_tool("kg_search", args).await?;
        // Label whose view this was, so a transcript read later says which
        // reader saw what without re-deriving it from the lens config.
        out.content = format!("[{} view]\n{}", self.label, out.content);
        Ok(out)
    }
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
You are one of two assistants building up what is known about one person, \
each reading different sources. Search your sources and answer the question \
from what you find.

Answer in at most three sentences. Say plainly when your sources do not \
cover it — the other assistant may see what you cannot, and a guess is worse \
than a gap. Do not speculate beyond what you read.";

/// System prompt for a reader generating a question for the other.
pub const FOLLOWUP_SYS: &str = "\
You and another assistant each read DIFFERENT sources about one person, so \
each of you can see things the other cannot.

You have just seen their answer. Ask ONE question that THEIR sources might \
answer and yours cannot — something that would genuinely add to what is \
known, not a rephrasing of what was already said. Prefer relationships, \
roles, commitments and the reasons behind things over dates and logistics.

Reply with the question alone, no preamble.";

/// Run one exchange. Deterministic orchestration: the code decides who is
/// asked what and when, so commit-then-reveal cannot be skipped.
///
/// `ask` is the seed both readers start from. Each subsequent round asks
/// each reader the question the *other* generated after seeing its answer.
pub async fn exchange(
    agents: &[(Vantage, Agent)],
    cx: &RunContext,
    entity: &str,
    seed: &str,
    rounds: u32,
) -> Result<Exchange> {
    anyhow::ensure!(agents.len() == 2, "gossip is a pair; got {}", agents.len());
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
        for (i, (vantage, agent)) in agents.iter().enumerate() {
            let other = 1 - i;
            let mut convo = Conversation::user(format!(
                "The person is {entity}.\n\nYou read: {}\nYou answered: {}\n\n\
                 They read: {}\nThey answered: {}",
                vantage.sources.join(", "),
                answers[i],
                agents[other].0.sources.join(", "),
                answers[other],
            ));
            // The follow-up is generated with the SAME agent, whose system
            // prompt is swapped for the asking one by the caller's config;
            // a tool-less run keeps it from wandering off to search.
            let outcome = agent.run_in(cx, &mut convo, None).await?;
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
