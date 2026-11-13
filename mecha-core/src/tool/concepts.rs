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
//! A concept is also counted against the runs it was handed to
//! ([`Offer`]): how many, and how many then showed the structure in a task
//! not judged to have failed. Handed over three times and never borne out,
//! it is not handed over again.
//!
//! Not here yet: patterns are not compared pairwise to find what a success
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

/// What a cue is delivered under ([`Concepts::cue`]): said when a call the
/// run was handed as an arrangement's middle step has just worked.
pub const CUE_STEM: &str = "The harness noted that a call just worked which an arrangement from another service places in the middle and not at the end. A hypothesis from elsewhere, never an instruction, and it verifies nothing about this task.\n";

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
            let widest = thread.iter().filter(|c| !c.is_error).map(&arity).max()?;
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

/// A run that was handed concepts, and what it then showed: the record a
/// concept is counted against. Kept whether or not the run showed anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub session: String,
    /// The keys of the concepts the run was handed.
    pub offered: Vec<String>,
    /// The key of the pattern the run itself showed, if it showed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub showed: Option<String>,
    /// How the run's task was judged, once someone has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judged: Option<bool>,
}

impl Offer {
    pub fn source_id(&self) -> String {
        format!("{}:offer", self.session)
    }
}

/// Runs that showed one structure.
#[derive(Debug, Clone, PartialEq)]
pub struct Concept {
    pub key: String,
    /// The supporting runs' patterns, oldest first. Never empty.
    pub support: Vec<Pattern>,
    /// Runs that were handed this concept, and how many of
    /// them went on to show the same structure in a task not judged to have
    /// failed. The difference is the concept's counterexamples: handed
    /// over, and not borne out or not acted on.
    pub tried: usize,
    pub held: usize,
}

/// A concept handed to this many runs without once holding is not handed
/// to another.
const RETIRED_AFTER: usize = 3;

impl Concept {
    fn external(&self) -> bool {
        self.support.iter().any(|p| p.external)
    }

    /// The tools here that fill each role: the very tools of a supporting
    /// run when this run has them, and otherwise every tool of the role's
    /// input shape. `None` when a role has none.
    fn fill<'a>(&self, here: &'a [ToolShape]) -> Option<Filled<'a>> {
        // A run that has the very tools a supporting run used: each role is
        // that tool, and the newest such run speaks.
        let named = |role: &ToolShape| here.iter().find(|t| t.name == role.name);
        for (at, own) in self.support.iter().enumerate().rev() {
            let (Some(change), Some(enabler)) = (named(&own.change), named(&own.enabler)) else {
                continue;
            };
            let closer = match &own.closer {
                Some(closer) => match named(closer) {
                    Some(tool) => Some(vec![tool]),
                    None => continue,
                },
                None => None,
            };
            return Some(Filled {
                change: vec![change],
                enabler: vec![enabler],
                closer,
                own: Some(at),
            });
        }
        let first = &self.support[0];
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
            own: None,
        })
    }

    /// The step this arrangement still owes once its change has been made
    /// here, and the tools whose first success makes it due. `None` for an
    /// arrangement with nothing after the change.
    fn owed(&self, filled: &Filled<'_>) -> Option<Owed> {
        let closer = filled.closer.as_ref()?;
        let mut either: Vec<&ToolShape> = match filled.own {
            Some(_) => Vec::new(),
            None => filled.enabler.clone(),
        };
        for tool in closer {
            if !either.iter().any(|t| t.name == tool.name) {
                either.push(tool);
            }
        }
        Some(Owed {
            after: filled.change.iter().map(|t| t.name.clone()).collect(),
            among: either
                .iter()
                .map(|t| format!("`{}`", t.name))
                .collect::<Vec<_>>()
                .join(", "),
        })
    }

    fn render(&self, filled: &Filled<'_>) -> String {
        // The support that says most speaks for the concept: one that met
        // the refusal, then the one that read most on the way, then the
        // newest. A run that already knew the order (an episode told it)
        // was never refused and read no documentation, and quoted alone it
        // gives the steps without the reason for them. The counts say how
        // many agree.
        let shown = match filled.own {
            Some(at) => &self.support[at],
            None => self
                .support
                .iter()
                .max_by_key(|p| {
                    (
                        p.refused_with.is_some(),
                        p.read.len(),
                        p.read.iter().map(String::len).sum::<usize>(),
                    )
                })
                .expect("never empty"),
        };
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
            "- {} {} run(s){}, these calls went together on one thing:\n",
            match filled.own {
                Some(_) => "With the tools you have here or ones like them, in",
                None => "In another service, in",
            },
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
        if self.tried > 0 {
            out.push_str(&format!(
                "  Handed to {} other run(s) with tools unlike those: {} of them then showed the same three steps.\n",
                self.tried, self.held
            ));
        }
        if filled.own.is_some() {
            out.push_str(
                "  Those are tools you have here: step 3 is still owed after the change has worked.\n",
            );
            return out;
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

/// A step an arrangement handed to this run still owes: the tools whose
/// first success makes it due, and the tools here it could be.
#[derive(Debug, Clone, PartialEq)]
struct Owed {
    after: Vec<String>,
    among: String,
}

/// A reminder that what a look shows is still not as the run found it is
/// given this many times for one owed step, before it has once been put
/// right.
const REMINDERS: u32 = 2;

/// What a run's host watches so that a handed concept's last step is held
/// open until it is made ([`Concepts::cue`]). Everything here is this run's
/// own: its calls and what they returned.
#[derive(Default)]
struct Watch {
    owed: Vec<Owed>,
    /// What each lookup first returned, by the call that made it: the
    /// thing as the run found it.
    first: std::collections::BTreeMap<(String, String), serde_json::Map<String, Value>>,
    /// The step that has fallen due: the change has worked.
    due: Option<Due>,
}

struct Due {
    among: String,
    /// The values the change was called with. A field that now holds one
    /// of them is the change that was asked for, not something left over.
    asked: Vec<String>,
    reminded: u32,
    /// The last look showed the thing as it was found.
    settled: bool,
}

fn plain(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) | Value::Null => Some(value.to_string()),
        _ => None,
    }
}

impl Watch {
    /// A call that worked, to what the harness says after it. Three
    /// moments: nothing until the change works; then, once, that a step is
    /// still owed; then, each time the same look that was made earlier
    /// shows a field the change did not ask for still differing from what
    /// that look first showed, that it does. The words are the harness's
    /// and the names and values are this run's own tools and results, so
    /// nothing a run elsewhere said or read is in them.
    fn after(&mut self, tool: &str, input: &Value, result: &str) -> Option<String> {
        let seen = serde_json::from_str::<Value>(result)
            .ok()
            .and_then(|v| v.as_object().cloned());
        let key = (tool.to_string(), input.to_string());
        if let Some(at) = self
            .owed
            .iter()
            .position(|o| o.after.iter().any(|t| t == tool))
        {
            let owed = self.owed.remove(at);
            let text = format!(
                "The change has now worked. In the arrangement you were shown at the start, this was step 2 of 3: there, the task was not finished until one more call was made on the same thing, with a different tool from the one that made the change possible. If the same holds here, that call is still owed and is one of {}. Looking the thing up again shows whether it is as you first found it, apart from the change you were asked for.\n",
                owed.among
            );
            self.due = Some(Due {
                among: owed.among,
                asked: input
                    .as_object()
                    .map(|o| o.values().filter_map(plain).collect())
                    .unwrap_or_default(),
                reminded: 0,
                settled: false,
            });
            return Some(text);
        }
        let seen = seen?;
        let Some(first) = self.first.get(&key) else {
            self.first.insert(key, seen);
            return None;
        };
        let due = self.due.as_mut()?;
        let left: Vec<String> = seen
            .iter()
            .filter_map(|(field, now)| {
                let now = plain(now)?;
                let then = plain(first.get(field)?)?;
                (now != then && !due.asked.contains(&now))
                    .then(|| format!("`{field}` is {now} and was {then} when you first looked"))
            })
            .collect();
        if left.is_empty() {
            // As it was found, but for what was asked. Said once when the
            // run had been told otherwise, so it knows to stop looking for
            // the step; a later look that shows it disturbed again is
            // answered again.
            let told = due.reminded > 0 && !due.settled;
            due.settled = true;
            return told.then(|| {
                "This is now as it was when you first looked, apart from the change you were asked for. If the arrangement from elsewhere holds here, the step after the change has been made and nothing further is owed; a further call on it may undo that.\n".to_string()
            });
        }
        if due.reminded == REMINDERS && !due.settled {
            return None;
        }
        if due.settled {
            due.settled = false;
            return Some(format!(
                "This was as you first found it, and a call since then has changed that: {}. The call that put it right earlier is the step after the change.\n",
                left.join("; ")
            ));
        }
        due.reminded += 1;
        Some(format!(
            "The step after the change has not been made yet, if the arrangement from elsewhere holds here: {}, which the change you were asked for does not account for. There, the last step was a different tool from the one that made the change possible; here it would be one of {}. A call that leaves this as it is was not that step.\n",
            left.join("; "),
            due.among
        ))
    }
}

struct Filled<'a> {
    change: Vec<&'a ToolShape>,
    enabler: Vec<&'a ToolShape>,
    closer: Option<Vec<&'a ToolShape>>,
    /// The supporting run whose own tools these are, when this run has
    /// them.
    own: Option<usize>,
}

/// Patterns in the order they were kept, to concepts: one per structure,
/// supported by the runs that showed it. A run judged to have failed
/// supports nothing, and a session's latest record is the one that counts.
pub fn fold(patterns: Vec<Pattern>, offers: Vec<Offer>) -> Vec<Concept> {
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
                tried: 0,
                held: 0,
            }),
        }
    }
    let mut latest: Vec<Offer> = Vec::new();
    for offer in offers {
        match latest.iter_mut().find(|o| o.session == offer.session) {
            Some(known) => *known = offer,
            None => latest.push(offer),
        }
    }
    for concept in &mut concepts {
        for offer in latest.iter().filter(|o| o.offered.contains(&concept.key)) {
            concept.tried += 1;
            if offer.showed.as_deref() == Some(concept.key.as_str()) && offer.judged != Some(false)
            {
                concept.held += 1;
            }
        }
    }
    concepts
}

/// What the store holds for concepts: every run's pattern and every offer,
/// oldest first, and how many records could not be read.
#[derive(Debug, Clone, Default)]
pub struct Kept {
    pub patterns: Vec<Pattern>,
    pub offers: Vec<Offer>,
    pub unreadable: usize,
}

/// Where a run's concepts are read from.
pub type Loader = std::sync::Arc<dyn Fn() -> Result<Kept> + Send + Sync>;

pub struct Concepts {
    loader: Loader,
    /// The server tools this run has, as shapes.
    here: Vec<ToolShape>,
    /// The keys of the concepts this tool has handed over, for the offer
    /// the caller records when the run ends.
    handed: std::sync::Arc<std::sync::Mutex<BTreeSet<String>>>,
    /// The steps those concepts still owe, held open until they are made.
    watch: std::sync::Arc<std::sync::Mutex<Watch>>,
}

impl Concepts {
    pub fn from_loader(loader: Loader, here: Vec<ToolShape>) -> Self {
        Concepts {
            loader,
            here,
            handed: Default::default(),
            watch: Default::default(),
        }
    }

    /// A handle on what this tool hands over, readable after it has moved
    /// into a registry.
    pub fn handed(&self) -> std::sync::Arc<std::sync::Mutex<BTreeSet<String>>> {
        self.handed.clone()
    }

    /// What to say after a call that worked, readable after this tool has
    /// moved into a registry ([`Watch::after`]). Nothing before a concept
    /// is handed over.
    pub fn cue(&self) -> crate::agent::SuccessCueText {
        let watch = self.watch.clone();
        std::sync::Arc::new(move |tool, input, result| {
            watch.lock().ok()?.after(tool, input, result)
        })
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
        let Kept {
            patterns,
            offers,
            unreadable,
        } = match tokio::task::spawn_blocking(move || loader()).await? {
            Ok(loaded) => loaded,
            Err(error) => return Ok(ToolOutput::err(format!("concepts unavailable: {error:#}"))),
        };
        let mut concepts = fold(patterns, offers);
        // Handed over three times to tools unlike its own and never borne
        // out: not handed over again.
        concepts.retain(|c| !(c.tried >= RETIRED_AFTER && c.held == 0));
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
            if let Ok(mut handed) = self.handed.lock() {
                // Handed over once, owed once: a second call of this tool
                // in one run does not make the step due again.
                if handed.insert(concept.key.clone()) {
                    if let (Some(step), Ok(mut watch)) = (concept.owed(&filled), self.watch.lock())
                    {
                        watch.owed.push(step);
                    }
                }
            }
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
        let concepts = fold(vec![first.clone(), second, failed.clone()], vec![]);
        assert_eq!(concepts.len(), 1);
        assert_eq!(concepts[0].support.len(), 2);
        // A later record for a session replaces its earlier one: the grade.
        let mut regraded = first.clone();
        regraded.judged = Some(false);
        assert!(fold(vec![first, regraded], vec![]).is_empty());
        // Not judged, the open pattern is its own concept.
        failed.judged = None;
        assert_eq!(fold(vec![failed], vec![]).len(), 1);
    }

    #[tokio::test]
    async fn a_concept_is_handed_to_tools_of_the_same_shape_and_exactly_to_its_own() {
        let pattern = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let kept = vec![pattern];
        let loader = |patterns: Vec<Pattern>| -> Loader {
            std::sync::Arc::new(move || {
                Ok(Kept {
                    patterns: patterns.clone(),
                    ..Default::default()
                })
            })
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

        // Its own service: each role is the tool that filled it there.
        let home = Concepts::from_loader(loader(kept.clone()), registry());
        let cue = home.cue();
        let out = home.call(json!({}), &ctx()).await.unwrap();
        assert!(
            out.content.contains("Those are tools you have here")
                && !out.content.contains("If the same holds here"),
            "{}",
            out.content
        );
        let owed = cue("reg_update", &json!({"id": "R-1"}), "ok").unwrap();
        assert!(
            owed.contains("is one of `reg_unlock`.") && !owed.contains("reg_lock"),
            "{owed}"
        );
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
        let loader: Loader = std::sync::Arc::new(move || {
            Ok(Kept {
                patterns: kept.clone(),
                ..Default::default()
            })
        });
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

    #[tokio::test]
    async fn a_concept_is_counted_against_the_runs_it_was_handed_to() {
        let pattern = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let key = pattern.key();
        let offer = |session: &str, showed: Option<&str>, judged: Option<bool>| Offer {
            session: session.into(),
            offered: vec![key.clone()],
            showed: showed.map(str::to_string),
            judged,
        };
        // One run followed it and passed, one followed it and failed, one
        // showed something else, one showed nothing; and a later record for
        // a session replaces its earlier one.
        let concepts = fold(
            vec![pattern.clone()],
            vec![
                offer("q1", Some(&key), None),
                offer("q1", Some(&key), Some(true)),
                offer("q2", Some(&key), Some(false)),
                offer("q3", Some("change[SS] after[S] then[-]"), Some(true)),
                offer("q4", None, None),
            ],
        );
        assert_eq!((concepts[0].tried, concepts[0].held), (4, 1));

        let handed_over = |offers: Vec<Offer>| {
            let patterns = vec![pattern.clone()];
            let loader: Loader = std::sync::Arc::new(move || {
                Ok(Kept {
                    patterns: patterns.clone(),
                    offers: offers.clone(),
                    unreadable: 0,
                })
            });
            Concepts::from_loader(loader, quay())
        };
        let tool = handed_over(vec![
            offer("q1", Some(&key), Some(true)),
            offer("q4", None, None),
        ]);
        let handed = tool.handed();
        let out = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(out.content.contains("Handed to 2 other run(s) with tools unlike those: 1 of them then showed the same three steps."), "{}", out.content);
        assert_eq!(
            handed.lock().unwrap().iter().collect::<Vec<_>>(),
            vec![&key]
        );

        // Three runs, never borne out: retired.
        let tool = handed_over(vec![
            offer("q2", None, None),
            offer("q3", None, None),
            offer("q4", None, None),
        ]);
        let handed = tool.handed();
        assert!(tool
            .call(json!({}), &ctx())
            .await
            .unwrap()
            .content
            .starts_with("No arrangement"));
        assert!(handed.lock().unwrap().is_empty());
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

    #[tokio::test]
    async fn the_step_a_handed_concept_owes_is_held_open_until_the_thing_is_as_it_was_found() {
        let pattern = extract(
            "s1",
            &locked_then_changed(true),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let open = extract(
            "s2",
            &locked_then_changed(false),
            &registry(),
            &Refused::new(),
        )
        .unwrap();
        let loader = |patterns: Vec<Pattern>| -> Loader {
            std::sync::Arc::new(move || {
                Ok(Kept {
                    patterns: patterns.clone(),
                    ..Default::default()
                })
            })
        };
        let tool = Concepts::from_loader(loader(vec![pattern.clone()]), quay());
        let cue = tool.cue();
        let look = json!({"name": "Draft 9"});
        let found = r#"{"id": "Q-1", "depth": 4, "held": false}"#;
        let taken = r#"{"id": "Q-1", "depth": 4, "held": true}"#;
        let set = json!({"id": "Q-1", "field": "depth", "value": 6});
        let one = json!({"id": "Q-1"});
        // Nothing is owed before the concept has been handed over.
        assert_eq!(cue("quay_set", &set, "ok"), None);
        tool.call(json!({}), &ctx()).await.unwrap();
        // Looked up twice in one run: still owed once.
        tool.call(json!({}), &ctx()).await.unwrap();
        // Not on a look, nor on the step before the change, nor on the
        // look that shows what that step did.
        assert_eq!(cue("quay_find", &look, found), None);
        assert_eq!(cue("quay_op_a", &one, "ok"), None);
        assert_eq!(cue("quay_find", &look, taken), None);
        let said = cue("quay_set", &set, "ok").unwrap();
        assert!(
            said.contains("step 2 of 3")
                && said.contains("`quay_find`, `quay_op_a`, `quay_op_b`, `quay_op_c`"),
            "{said}"
        );
        // The harness's words and the names of tools here: nothing the
        // other service's run said or read, and none of its tools.
        for theirs in ["reg_", "E4", "Unlock", "unlock when done"] {
            assert!(!said.contains(theirs), "{theirs}: {said}");
        }
        assert_eq!(cue("quay_set", &set, "ok"), None);
        // A call that was not the step, and a look that shows it: what the
        // earlier step left is named, and what was asked for is not.
        assert_eq!(cue("quay_op_c", &one, "ok"), None);
        let changed = r#"{"id": "Q-1", "depth": 6, "held": true}"#;
        let still = cue("quay_find", &look, changed).unwrap();
        assert!(
            still.contains("`held` is true and was false when you first looked")
                && !still.contains("depth"),
            "{still}"
        );
        // Twice at most, however often it looks.
        assert!(cue("quay_find", &look, changed).is_some());
        assert_eq!(cue("quay_find", &look, changed), None);

        // Put back after being told: said once, so the run stops looking,
        // and said again if a later call disturbs it.
        let back = r#"{"id": "Q-1", "depth": 6, "held": false}"#;
        let done = cue("quay_find", &look, back).unwrap();
        assert!(done.contains("nothing further is owed"), "{done}");
        assert_eq!(cue("quay_find", &look, back), None);
        let again = cue("quay_find", &look, changed).unwrap();
        assert!(
            again.contains("a call since then has changed that"),
            "{again}"
        );
        assert!(cue("quay_find", &look, back).is_some());

        // Put back before anything was said: nothing is said.
        let tool = Concepts::from_loader(loader(vec![pattern.clone()]), quay());
        let cue = tool.cue();
        tool.call(json!({}), &ctx()).await.unwrap();
        assert_eq!(cue("quay_find", &look, found), None);
        assert!(cue("quay_set", &set, "ok").is_some());
        assert_eq!(
            cue(
                "quay_find",
                &look,
                r#"{"id": "Q-1", "depth": 6, "held": false}"#
            ),
            None
        );

        // An arrangement with nothing after the change owes nothing.
        let tool = Concepts::from_loader(loader(vec![open]), quay());
        let cue = tool.cue();
        let out = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(
            out.content.contains("nothing on the thing after"),
            "{}",
            out.content
        );
        assert_eq!(cue("quay_set", &set, "ok"), None);
    }
}
