//! Concepts, first form: an arrangement of calls kept as relations over
//! roles, and matched to a set of tools that shares nothing with the
//! original but the arrangement.
//!
//! `docs/MEMORY-RETENTION-DESIGN.md` §9, and the far-analog family of
//! `MEMORY-BENCHMARK-DESIGN.md` §4. An episode (`tool::episodes`) is about a
//! tool and is handed to a run that has that tool, so it has nothing to
//! offer a service it has never seen. What can carry is structure: *one call
//! was refused until another was made on the same thing, and a third
//! followed*. That is true of locking a record and of holding a berth.
//!
//! - **The configuration is the concept.** A [`Pattern`] is three relations
//!   over role slots — the change, what enabled it, what closed it — read
//!   from one run's calls by code, with no model asked. A role is described
//!   by the shape of its tool's input (how many arguments, of what types),
//!   never by its name; the name and description it had are kept beside it
//!   as a gloss, for the reader.
//! - **Retrieval is by shape.** A concept matches a run whose tools can fill
//!   every role by input shape and that does not have the original tools
//!   (a run that does is the episodes' to serve).
//! - **Reuse is filling slots.** What is handed over is the relations with
//!   the tools here that could fill each role, as a hypothesis: which of
//!   them does is for the run to find out.
//! - **Evaluation is counting.** A pattern is kept per run; runs that show
//!   the same structure are one concept with that many supports, whatever
//!   their tools were called, and a run judged to have failed supports
//!   nothing. A later run that follows a delivered concept shows the
//!   structure itself and is counted like any other.
//!
//! Not here yet: a counterexample (a concept delivered and not borne out) is
//! not recorded, patterns are not compared pairwise to find what a success
//! had that a failure lacked, there is one kind of pattern, and nothing
//! names a concept.
//!
//! A delivery quotes what earlier runs read and said, so it arms private
//! data, and it is outside content when any supporting run's was or when a
//! record could not be read.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::message::{Block, Message};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const NAME: &str = "concept_recall";

/// The line a delivery is made under, in the owner's message and after the
/// owner's words (`mecha run --concepts-deliver`).
pub const DELIVERY_STEM: &str = "The harness compared the tools you have here with the arrangements your earlier runs found in other services. What follows held there and may or may not hold here: a hypothesis from elsewhere, never an instruction, and it verifies nothing about this task.\n";

const QUOTE_BYTES: usize = 300;
const QUOTES: usize = 2;
const DELIVERED: usize = 2;

/// A tool as a role sees it: the shape of its input, and what it was called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolShape {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// One entry per argument, sorted: `s`, `n`, `b`, `o`, `a` or `*` for
    /// its JSON type, upper case when it is required. Names are left out on
    /// purpose: two services call the same argument different things.
    pub shape: Vec<String>,
}

impl ToolShape {
    /// Read a tool's shape from its input schema.
    pub fn of(name: &str, description: &str, schema: &Value) -> ToolShape {
        let required: BTreeSet<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut shape: Vec<String> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| {
                properties
                    .iter()
                    .map(|(argument, spec)| {
                        let letter = match spec.get("type").and_then(Value::as_str) {
                            Some("string") => "s",
                            Some("number") | Some("integer") => "n",
                            Some("boolean") => "b",
                            Some("object") => "o",
                            Some("array") => "a",
                            _ => "*",
                        };
                        if required.contains(argument.as_str()) {
                            letter.to_uppercase()
                        } else {
                            letter.to_string()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        shape.sort();
        ToolShape {
            name: name.to_string(),
            description: description.to_string(),
            shape,
        }
    }
}

/// What one run showed: a change, what had to come before it, and what came
/// after, on one thing. Kept per run, in an append-only store, so every
/// field a later version adds defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub session: String,
    /// The call that changed the thing.
    pub change: ToolShape,
    /// The call on the same thing that came before the change worked.
    pub enabler: ToolShape,
    /// The call on the same thing that came after it, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closer: Option<ToolShape>,
    /// What the change was refused with before the enabler, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_with: Option<String>,
    /// What the run read from other tools on the way: `tool input → result`.
    #[serde(default)]
    pub read: Vec<String>,
    /// Whether anything quoted came from outside, or nobody recorded.
    #[serde(default = "yes")]
    pub external: bool,
    /// How the run's task was judged, once someone has. `Some(false)`: the
    /// run supports nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judged: Option<bool>,
}

fn yes() -> bool {
    true
}

impl Pattern {
    /// What names this record in a store that keeps one object per source.
    pub fn source_id(&self) -> String {
        format!("{}:pattern", self.session)
    }

    /// The structure, with every name left out: two patterns with one key
    /// are one concept.
    pub fn key(&self) -> String {
        format!(
            "change[{}] after[{}] then[{}]",
            self.change.shape.join(""),
            self.enabler.shape.join(""),
            self.closer
                .as_ref()
                .map(|c| c.shape.join(""))
                .unwrap_or_else(|| "-".into())
        )
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

struct Call {
    name: String,
    input: Value,
    result: String,
    is_error: bool,
    external: bool,
}

/// The run's finished calls to the tools in `shapes`, in order, less the
/// ones the harness refused.
fn calls(
    messages: &[Message],
    shapes: &BTreeMap<&str, &ToolShape>,
    refused: &super::episodes::Refused,
) -> Vec<Call> {
    let mut pending: BTreeMap<String, (String, Value)> = BTreeMap::new();
    let mut out = Vec::new();
    for message in messages {
        for block in &message.content {
            match block {
                Block::ToolUse { id, name, input } => {
                    pending.insert(id.clone(), (name.clone(), input.clone()));
                }
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let Some((name, input)) = pending.remove(tool_use_id) else {
                        continue;
                    };
                    if !shapes.contains_key(name.as_str())
                        || content.starts_with("Blocked by")
                        || content.starts_with("Denied by the user")
                        || refused.contains(&(name.clone(), input.to_string()))
                    {
                        continue;
                    }
                    out.push(Call {
                        name,
                        input,
                        result: content.clone(),
                        is_error: *is_error,
                        external: message
                            .tool_provenance
                            .get(tool_use_id)
                            .copied()
                            .unwrap_or(true),
                    });
                }
                _ => {}
            }
        }
    }
    out
}

/// The scalar values a call was made with, as text.
fn values(input: &Value) -> Vec<String> {
    input
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(|v| match v {
                    Value::String(s) if s.len() >= 2 => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The pattern in one run's calls, if it has one.
///
/// The thing is the argument value the most different tools were called
/// with (at least two, and it must be clear which). The change is the tool
/// on that thing that was refused and then worked; with no refusal, the one
/// called with the most arguments. The enabler is the last other tool that
/// worked on the thing before the change did; the closer, the last one
/// after.
pub fn extract(
    session: &str,
    messages: &[Message],
    shapes: &[ToolShape],
    refused: &super::episodes::Refused,
) -> Option<Pattern> {
    let by_name: BTreeMap<&str, &ToolShape> = shapes.iter().map(|s| (s.name.as_str(), s)).collect();
    let calls = calls(messages, &by_name, refused);
    // Which tools were called with each value.
    let mut tools_of: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for call in &calls {
        for value in values(&call.input) {
            tools_of
                .entry(value)
                .or_default()
                .insert(call.name.as_str());
        }
    }
    let most = tools_of
        .values()
        .map(BTreeSet::len)
        .max()
        .filter(|n| *n >= 2)?;
    let mut things = tools_of.iter().filter(|(_, tools)| tools.len() == most);
    let (thing, _) = things.next()?;
    if things.next().is_some() {
        return None;
    }
    let thread: Vec<&Call> = calls
        .iter()
        .filter(|c| values(&c.input).contains(thing))
        .collect();
    let refused_then_worked = thread.iter().find(|c| {
        c.is_error
            && thread
                .iter()
                .any(|later| later.name == c.name && !later.is_error)
    });
    let change = match refused_then_worked {
        Some(call) => call.name.as_str(),
        None => {
            let arity = |c: &&Call| c.input.as_object().map_or(0, |o| o.len());
            let widest = thread
                .iter()
                .filter(|c| !c.is_error)
                .map(|c| arity(c))
                .max()?;
            let mut wide = thread
                .iter()
                .filter(|c| !c.is_error && arity(c) == widest)
                .map(|c| c.name.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter();
            let only = wide.next()?;
            if wide.next().is_some() {
                return None;
            }
            only
        }
    };
    let worked_at = thread
        .iter()
        .position(|c| c.name == change && !c.is_error)?;
    let enabler = thread[..worked_at]
        .iter()
        .rev()
        .find(|c| c.name != change && !c.is_error)?
        .name
        .as_str();
    let closer = thread[worked_at + 1..]
        .iter()
        .rev()
        .find(|c| c.name != change && c.name != enabler && !c.is_error)
        .map(|c| c.name.as_str());
    // What the run read on the way, from tools that are not on the thread:
    // documentation, mostly.
    let on_thread: BTreeSet<&str> = thread.iter().map(|c| c.name.as_str()).collect();
    let first_success_overall = calls
        .iter()
        .position(|c| c.name == change && !c.is_error)
        .unwrap_or(calls.len());
    let reads: Vec<&Call> = calls[..first_success_overall]
        .iter()
        .filter(|c| !c.is_error && !on_thread.contains(c.name.as_str()))
        .collect();
    let reads = &reads[reads.len().saturating_sub(QUOTES)..];
    let shape = |name: &str| (*by_name.get(name).expect("a call's tool has a shape")).clone();
    Some(Pattern {
        session: session.to_string(),
        change: shape(change),
        enabler: shape(enabler),
        closer: closer.map(shape),
        refused_with: refused_then_worked.map(|c| cut(c.result.trim(), QUOTE_BYTES)),
        read: reads
            .iter()
            .map(|c| {
                format!(
                    "{} {} → {}",
                    c.name,
                    c.input,
                    cut(c.result.trim(), QUOTE_BYTES)
                )
            })
            .collect(),
        external: thread.iter().any(|c| c.external) || reads.iter().any(|c| c.external),
        judged: None,
    })
}

/// What a finished run showed, read from its transcript. As with an
/// episode, the session's recorded taint decides whether it is outside
/// content: anything else the run read can ride in what it quotes.
pub fn found(
    session: &str,
    transcript: &str,
    shapes: &[ToolShape],
    refused: &super::episodes::Refused,
) -> Option<Pattern> {
    let messages = crate::session::Session::messages_ever(transcript);
    let mut pattern = extract(session, &messages, shapes, refused)?;
    let recorded = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<crate::session::Record>(line).ok())
        .filter_map(|record| match record {
            crate::session::Record::Taint(taint) => Some(taint.untrusted),
            _ => None,
        })
        .next_back();
    if recorded != Some(false) {
        pattern.external = true;
    }
    Some(pattern)
}

/// Runs that showed one structure.
#[derive(Debug, Clone, PartialEq)]
pub struct Concept {
    pub key: String,
    /// The supporting runs' patterns, oldest first. Never empty.
    pub support: Vec<Pattern>,
}

impl Concept {
    fn external(&self) -> bool {
        self.support.iter().any(|p| p.external)
    }

    /// The tools here that could fill each role, by input shape; `None`
    /// when a role has none, or when this run has the tools the concept was
    /// learned on.
    fn fill<'a>(&self, here: &'a [ToolShape]) -> Option<Filled<'a>> {
        let first = &self.support[0];
        let known: BTreeSet<&str> = self
            .support
            .iter()
            .map(|p| p.change.name.as_str())
            .collect();
        if here.iter().any(|t| known.contains(t.name.as_str())) {
            return None;
        }
        let like = |role: &ToolShape| -> Vec<&'a ToolShape> {
            here.iter().filter(|t| t.shape == role.shape).collect()
        };
        let change = like(&first.change);
        let enabler = like(&first.enabler);
        let closer = first.closer.as_ref().map(like);
        if change.is_empty() || enabler.is_empty() || closer.as_ref().is_some_and(|c| c.is_empty())
        {
            return None;
        }
        Some(Filled {
            change,
            enabler,
            closer,
        })
    }

    fn render(&self, filled: &Filled<'_>) -> String {
        // The support that says most speaks for the concept: one that met
        // the refusal, then one that read something on the way, then the
        // newest. The counts say how many agree.
        let shown = self
            .support
            .iter()
            .max_by_key(|p| (p.refused_with.is_some(), !p.read.is_empty()))
            .expect("never empty");
        let names = |tools: &[&ToolShape]| {
            tools
                .iter()
                .map(|t| format!("`{}`", t.name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let gloss = |t: &ToolShape| {
            if t.description.trim().is_empty() {
                format!("`{}`", t.name)
            } else {
                format!("`{}` (\"{}\")", t.name, t.description.trim())
            }
        };
        let judged = self
            .support
            .iter()
            .filter(|p| p.judged == Some(true))
            .count();
        let mut out = format!(
            "- In another service, in {} run(s){}, these calls went together on one thing:\n",
            self.support.len(),
            match judged {
                0 => String::new(),
                n if n == self.support.len() => ", each judged to have succeeded".to_string(),
                n => format!(", {n} of them judged to have succeeded"),
            }
        );
        out.push_str(&format!("    1. first {};\n", gloss(&shown.enabler)));
        out.push_str(&format!(
            "    2. then the change, {}{};\n",
            gloss(&shown.change),
            match &shown.refused_with {
                Some(error) => format!(", which was refused (\"{error}\") without step 1"),
                None => String::new(),
            }
        ));
        match &shown.closer {
            Some(closer) => out.push_str(&format!(
                "    3. then {} on the same thing. The task was not finished until this was done.\n",
                gloss(closer)
            )),
            None => out.push_str("    3. and nothing on the thing after the change.\n"),
        }
        for read in &shown.read {
            out.push_str(&format!("  What that run had read: {read}\n"));
        }
        out.push_str(&format!(
            "  If the same holds here: the change is {}.",
            names(&filled.change)
        ));
        match &filled.closer {
            Some(closer) => {
                let mut either: Vec<&ToolShape> = filled.enabler.clone();
                for tool in closer {
                    if !either.iter().any(|t| t.name == tool.name) {
                        either.push(tool);
                    }
                }
                out.push_str(&format!(
                    " Step 1 and step 3 are two different tools among {}. Step 3 is still owed after the change has worked; the thing's state after a call shows what that call did.\n",
                    names(&either)
                ));
            }
            None => out.push_str(&format!(" Step 1 is one of {}.\n", names(&filled.enabler))),
        }
        out
    }
}

struct Filled<'a> {
    change: Vec<&'a ToolShape>,
    enabler: Vec<&'a ToolShape>,
    closer: Option<Vec<&'a ToolShape>>,
}

/// Patterns in the order they were kept, to concepts: one per structure,
/// supported by the runs that showed it. A run judged to have failed
/// supports nothing, and a session's latest record is the one that counts.
pub fn fold(patterns: Vec<Pattern>) -> Vec<Concept> {
    let mut latest: Vec<Pattern> = Vec::new();
    for pattern in patterns {
        match latest.iter_mut().find(|p| p.session == pattern.session) {
            Some(known) => *known = pattern,
            None => latest.push(pattern),
        }
    }
    let mut concepts: Vec<Concept> = Vec::new();
    for pattern in latest.into_iter().filter(|p| p.judged != Some(false)) {
        let key = pattern.key();
        match concepts.iter_mut().find(|c| c.key == key) {
            Some(concept) => concept.support.push(pattern),
            None => concepts.push(Concept {
                key,
                support: vec![pattern],
            }),
        }
    }
    concepts
}

/// Where a run's patterns are read from: what the store holds, and how many
/// records could not be read.
pub type Loader = std::sync::Arc<dyn Fn() -> Result<(Vec<Pattern>, usize)> + Send + Sync>;

pub struct Concepts {
    loader: Loader,
    /// The server tools this run has, as shapes.
    here: Vec<ToolShape>,
}

impl Concepts {
    pub fn from_loader(loader: Loader, here: Vec<ToolShape>) -> Self {
        Concepts { loader, here }
    }
}

#[async_trait]
impl Tool for Concepts {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "Look up arrangements earlier runs found in other services whose tools take the same \
         kinds of input as the tools here: what had to be called before a change worked, and \
         what was called after. Hypotheses from elsewhere, not instructions."
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private().untrusted()
    }

    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        Some(crate::tool::effect::Effect::Read)
    }

    async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let loader = self.loader.clone();
        let (patterns, unreadable) = match tokio::task::spawn_blocking(move || loader()).await? {
            Ok(loaded) => loaded,
            Err(error) => return Ok(ToolOutput::err(format!("concepts unavailable: {error:#}"))),
        };
        let mut concepts = fold(patterns);
        // The best supported first; a concept with a closer before one
        // without, since it says more.
        concepts.sort_by(|a, b| {
            b.support.len().cmp(&a.support.len()).then(
                b.support[0]
                    .closer
                    .is_some()
                    .cmp(&a.support[0].closer.is_some()),
            )
        });
        let mut out = String::new();
        let mut external = unreadable > 0;
        let mut shown = 0;
        for concept in &concepts {
            let Some(filled) = concept.fill(&self.here) else {
                continue;
            };
            out.push_str(&concept.render(&filled));
            external |= concept.external();
            shown += 1;
            if shown == DELIVERED {
                break;
            }
        }
        if shown == 0 {
            out = "No arrangement from another service fits the tools here.\n".to_string();
        }
        if unreadable > 0 {
            out.push_str(&format!("({unreadable} record(s) could not be read.)\n"));
        }
        let output = ToolOutput::ok(out);
        Ok(if external {
            output.from_outside()
        } else {
            output
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::episodes::Refused;

    fn ctx() -> ToolCtx {
        ToolCtx::default().with_workspace(std::env::temp_dir())
    }

    fn shape(name: &str, description: &str, required: &[&str], optional: &[&str]) -> ToolShape {
        let mut properties = serde_json::Map::new();
        for argument in required.iter().chain(optional) {
            properties.insert(argument.to_string(), json!({"type": "string"}));
        }
        ToolShape::of(
            name,
            description,
            &json!({"type": "object", "properties": properties, "required": required}),
        )
    }

    fn registry() -> Vec<ToolShape> {
        vec![
            shape("reg_help", "Read the documentation.", &[], &["topic"]),
            shape("reg_find", "Find a record.", &["name"], &[]),
            shape("reg_lock", "Lock a record.", &["id"], &[]),
            shape(
                "reg_update",
                "Change one field.",
                &["id", "field", "value"],
                &[],
            ),
            shape("reg_unlock", "Unlock a record.", &["id"], &[]),
        ]
    }

    fn quay() -> Vec<ToolShape> {
        let mut tools = vec![
            shape("quay_find", "Find a berth.", &["name"], &[]),
            shape(
                "quay_set",
                "Set one value.",
                &["berth", "key", "value"],
                &[],
            ),
        ];
        for letter in ["a", "b", "c"] {
            tools.push(shape(
                &format!("quay_op_{letter}"),
                "An operation.",
                &["berth"],
                &[],
            ));
        }
        tools
    }

    fn call(id: &str, name: &str, input: Value, result: &str, is_error: bool) -> [Message; 2] {
        let mut answer = Message::tool_results(vec![Block::ToolResult {
            tool_use_id: id.into(),
            content: result.into(),
            is_error,
        }]);
        answer.tool_provenance.insert(id.into(), false);
        [
            Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }]),
            answer,
        ]
    }

    fn run(calls: Vec<[Message; 2]>) -> Vec<Message> {
        let mut messages = vec![Message::user("do the task")];
        messages.extend(calls.into_iter().flatten());
        messages
    }

    fn locked_then_changed(unlock: bool) -> Vec<Message> {
        let mut calls = vec![
            call(
                "a",
                "reg_find",
                json!({"name": "ledger"}),
                r#"{"id":"R-7"}"#,
                false,
            ),
            call(
                "b",
                "reg_update",
                json!({"id": "R-7", "field": "owner", "value": "Hale"}),
                "E4",
                true,
            ),
            call(
                "c",
                "reg_help",
                json!({"topic": "update"}),
                "Lock first; unlock when done.",
                false,
            ),
            call("d", "reg_lock", json!({"id": "R-7"}), "locked", false),
            call(
                "e",
                "reg_update",
                json!({"id": "R-7", "field": "owner", "value": "Hale"}),
                "updated",
                false,
            ),
        ];
        if unlock {
            calls.push(call(
                "f",
                "reg_unlock",
                json!({"id": "R-7"}),
                "unlocked",
                false,
            ));
        }
        run(calls)
    }

    #[test]
    fn a_tool_s_shape_is_its_arguments_types_and_never_their_names() {
        let a = shape("reg_lock", "", &["id"], &[]);
        let b = shape("quay_op_a", "", &["berth"], &[]);
        assert_eq!(a.shape, b.shape);
        assert_eq!(a.shape, vec!["S"]);
        assert_ne!(a.shape, shape("reg_help", "", &[], &["topic"]).shape);
        let mixed = ToolShape::of(
            "t",
            "",
            &json!({"properties": {"n": {"type": "integer"}, "v": {}, "s": {"type": "string"}}, "required": ["s"]}),
        );
        assert_eq!(mixed.shape, vec!["*", "S", "n"]);
    }

    #[test]
    fn a_refusal_an_enabler_and_a_closer_on_one_thing_are_the_pattern() {
        let pattern = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        assert_eq!(pattern.change.name, "reg_update");
        assert_eq!(pattern.enabler.name, "reg_lock");
        assert_eq!(pattern.closer.as_ref().unwrap().name, "reg_unlock");
        assert_eq!(pattern.refused_with.as_deref(), Some("E4"));
        // The documentation it read is quoted; the lookup that named the
        // thing is not on the thread and is quoted too.
        assert!(pattern.read.iter().any(|r| r.contains("unlock when done")));
        assert_eq!(pattern.key(), "change[SSS] after[S] then[S]");
        // Left locked: the same change and enabler, and no closer.
        let open = extract(
            "s2",
            &locked_then_changed(false),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        assert!(open.closer.is_none());
        assert_ne!(open.key(), pattern.key());
    }

    #[test]
    fn a_run_that_was_never_refused_still_shows_the_order() {
        let messages = run(vec![
            call("d", "reg_lock", json!({"id": "R-7"}), "locked", false),
            call(
                "e",
                "reg_update",
                json!({"id": "R-7", "field": "owner", "value": "Hale"}),
                "updated",
                false,
            ),
            call("f", "reg_unlock", json!({"id": "R-7"}), "unlocked", false),
        ]);
        let pattern = extract("s1", &messages, &registry(), &Refused::new()).unwrap();
        assert_eq!(
            (pattern.change.name.as_str(), pattern.enabler.name.as_str()),
            ("reg_update", "reg_lock")
        );
        assert_eq!(pattern.refused_with, None);
        assert_eq!(pattern.key(), "change[SSS] after[S] then[S]");
    }

    #[test]
    fn nothing_shared_between_tools_is_no_pattern_and_a_harness_refusal_is_not_the_tool_s() {
        let lone = run(vec![call(
            "a",
            "reg_find",
            json!({"name": "ledger"}),
            "{}",
            false,
        )]);
        assert!(extract("s1", &lone, &registry(), &Refused::new()).is_none());
        // The harness refused the update; the tool never did.
        let messages = locked_then_changed(true);
        let refused: Refused = [(
            "reg_update".to_string(),
            json!({"id": "R-7", "field": "owner", "value": "Hale"}).to_string(),
        )]
        .into();
        // Both update calls share that input, so both are left out and no
        // change remains to anchor a pattern on.
        let pattern = extract("s1", &messages, &registry(), &refused);
        assert!(pattern.is_none_or(|p| p.refused_with.is_none()));
    }

    #[test]
    fn runs_with_one_structure_are_one_concept_and_a_failed_run_supports_nothing() {
        let first = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let mut second = first.clone();
        second.session = "s2".into();
        let mut failed = extract(
            "s3",
            &locked_then_changed(false),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        failed.judged = Some(false);
        let concepts = fold(vec![first.clone(), second, failed.clone()]);
        assert_eq!(concepts.len(), 1);
        assert_eq!(concepts[0].support.len(), 2);
        // A later record for a session replaces its earlier one: the grade.
        let mut regraded = first.clone();
        regraded.judged = Some(false);
        assert!(fold(vec![first, regraded]).is_empty());
        // Not judged, the open pattern is its own concept.
        failed.judged = None;
        assert_eq!(fold(vec![failed]).len(), 1);
    }

    #[tokio::test]
    async fn a_concept_is_handed_to_tools_of_the_same_shape_and_not_to_its_own() {
        let pattern = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let kept = vec![pattern];
        let loader = |patterns: Vec<Pattern>| -> Loader {
            std::sync::Arc::new(move || Ok((patterns.clone(), 0)))
        };

        let elsewhere = Concepts::from_loader(loader(kept.clone()), quay());
        let out = elsewhere.call(json!({}), &ctx()).await.unwrap();
        assert!(!out.is_error && !out.external, "{}", out.content);
        // The change binds to the one tool of its shape; the roles before
        // and after to every tool of theirs, the lookup included.
        let said =
            |needle: &str| assert!(out.content.contains(needle), "{needle}: {}", out.content);
        said("the change is `quay_set`.");
        said("two different tools among `quay_find`, `quay_op_a`, `quay_op_b`, `quay_op_c`.");
        said("Step 3 is still owed after the change has worked");
        // What held there, in its own words.
        said("1. first `reg_lock` (\"Lock a record.\");");
        said("which was refused (\"E4\") without step 1");
        said("3. then `reg_unlock` (\"Unlock a record.\") on the same thing. The task was not finished until this was done.");
        said("unlock when done");

        // Its own service: the episodes' to serve.
        let home = Concepts::from_loader(loader(kept.clone()), registry());
        assert!(home
            .call(json!({}), &ctx())
            .await
            .unwrap()
            .content
            .starts_with("No arrangement"));
        // No tool of the change's shape: no fit.
        let narrow = Concepts::from_loader(loader(kept), vec![shape("x", "", &["a"], &[])]);
        assert!(narrow
            .call(json!({}), &ctx())
            .await
            .unwrap()
            .content
            .starts_with("No arrangement"));
    }

    #[tokio::test]
    async fn the_support_that_met_the_refusal_speaks_for_the_concept() {
        let refused = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let smooth = run(vec![
            call("d", "reg_lock", json!({"id": "R-12"}), "locked", false),
            call(
                "e",
                "reg_update",
                json!({"id": "R-12", "field": "tier", "value": "3"}),
                "updated",
                false,
            ),
            call("f", "reg_unlock", json!({"id": "R-12"}), "unlocked", false),
        ]);
        let mut later = extract("s2", &smooth, &registry(), &Refused::new()).unwrap();
        later.judged = Some(true);
        let mut first = refused;
        first.judged = Some(true);
        let kept = vec![first, later];
        let loader: Loader = std::sync::Arc::new(move || Ok((kept.clone(), 0)));
        let out = Concepts::from_loader(loader, quay())
            .call(json!({}), &ctx())
            .await
            .unwrap();
        // The newer run was never refused; the older one says why the order
        // matters, and it is the one quoted.
        assert!(
            out.content
                .contains("in 2 run(s), each judged to have succeeded"),
            "{}",
            out.content
        );
        assert!(
            out.content
                .contains("which was refused (\"E4\") without step 1"),
            "{}",
            out.content
        );
        assert!(out.content.contains("unlock when done"), "{}", out.content);
    }

    #[test]
    fn the_run_s_recorded_taint_decides_whether_its_pattern_is_outside_content() {
        let record =
            |r: crate::session::Record| format!("{}\n", serde_json::to_string(&r).unwrap());
        let calls: String = locked_then_changed(true)
            .into_iter()
            .map(|m| record(crate::session::Record::Message(m)))
            .collect();
        let taint = |untrusted| {
            record(crate::session::Record::Taint(crate::agent::Taint {
                untrusted,
                ..Default::default()
            }))
        };
        let shapes = registry();
        let none = Refused::new();
        assert!(
            !found("s1", &format!("{calls}{}", taint(false)), &shapes, &none)
                .unwrap()
                .external
        );
        assert!(
            found("s1", &format!("{calls}{}", taint(true)), &shapes, &none)
                .unwrap()
                .external
        );
        assert!(found("s1", &calls, &shapes, &none).unwrap().external);
    }
}
