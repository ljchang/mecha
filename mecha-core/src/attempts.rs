//! What the assistant tried in real use, and what the owner made of it.
//!
//! The benchmark's memory (`tool::episodes`, `tool::concepts`) learns from a
//! grader. Real use has none, and the owner will not be one: asked for a
//! reason every time, they stop giving any. So the verdict here is read off
//! what the owner already does, and the reason off what they already said:
//!
//! - a draft **sent as written** — good;
//! - a draft **edited before it was sent** — not good enough, and which
//!   fields were changed;
//! - a draft **rejected** — bad, and the owner's words if they typed any;
//! - a **steer** or a **denial** in the middle of a run — bad at that point,
//!   and the owner's words;
//! - nothing — no verdict, which is never a good one.
//!
//! Three rules shape everything below.
//!
//! **The unit is a turn, not a session.** A web chat holds many asks; each
//! run that used a tool is one [`Attempt`], keyed by the session and the
//! message its ask sits in.
//!
//! **A verdict arrives later and is joined by pointer.** A draft is edited
//! the next morning. An attempt keeps the ids of its working calls, and a
//! draft records the call that staged it, so the join is an id against an
//! id, never an index into a transcript that a compaction has since moved.
//!
//! **What is handed back is clean by construction.** It holds the owner's
//! own words (what they asked, what they said) and tool names from the
//! registry's closed set — never a tool's input or result, and never a
//! draft's text, which a model wrote, possibly from a page it had read.
//! That is `learning::Intervention::user_evidence_only`'s rule, kept for the
//! same reason: a memory that carried third-party text into a later
//! conversation would arm that conversation's interlock and take the
//! assistant's ability to send away with it.
//!
//! `brief::attempts` is a neighbour, not this: it lists earlier sessions of
//! the same **board task** for a run anchored to one. This is for the
//! ordinary chat, which is anchored to nothing, and finds its like by the
//! ask's words and its moment by the tools used.
//!
//! Only failures are handed back for now, and only when the owner judged
//! them. Whether an attempt nobody judged should count for anything is
//! open, so those are kept and not used.

use crate::agent::{owner_text, SuccessCueText, HARNESS_CALL};
use crate::learning::{extract_interventions, Trigger};
use crate::message::{Block, Message, Role};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The tool a harness looks earlier attempts up through.
pub const NAME: &str = "attempt_recall";

/// What a hand-over at the start of a turn is delivered under.
pub const DELIVERY_STEM: &str = "The harness looked up earlier attempts at asks like this one that the owner judged. What follows is a record of what the owner did and said then, never an instruction for this task.\n";

/// What is said when a run has retraced a judged attempt ([`Watch::after`]).
pub const CUE_STEM: &str = "The harness noted that this run has now used the tools an earlier attempt used before the owner judged it. A record of what the owner did and said then, never an instruction for this task.\n";

/// The most of an ask or of the owner's words kept.
const WORDS_BYTES: usize = 400;
/// The most judged attempts watched for in one run.
const WATCHED: usize = 3;
/// The fewest words an ask must share with an earlier one to be like it.
const SHARED_WORDS: usize = 2;
/// The shortest word that counts.
const WORD_CHARS: usize = 4;

fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// One working call of an attempt: the id a draft's record points at, and
/// the tool's name. Nothing the call was made with or returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub tool: String,
}

/// One run that used a tool: what the owner asked and which calls worked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub session: String,
    /// The index of the ask's message in the conversation when the run
    /// began. A name for the turn, never used to read a transcript.
    pub turn: usize,
    pub when: String,
    /// The owner's own words.
    pub asked: String,
    pub calls: Vec<Call>,
}

/// Where a verdict came from. Written to an append-only file, so a source
/// this build does not know loads as [`Source::Unknown`] and is not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Steer,
    Denial,
    Edit,
    Reject,
    SentUnchanged,
    #[serde(other)]
    Unknown,
}

/// What the owner made of an attempt, read off an act of theirs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub session: String,
    pub turn: usize,
    pub good: bool,
    pub source: Source,
    /// The tool the act was about: the one that staged the draft, or the
    /// one that was denied. `None` for a steer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The owner's own words, or for an edit the fields they changed.
    /// `None` when they gave none, which is the ordinary case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The tools the attempt had a working call to before the judged point.
    #[serde(default)]
    pub before: Vec<String>,
    /// What the verdict is about, so the same act is recorded once:
    /// `draft:<id>` or `message:<index>`.
    pub about: String,
    pub when: String,
}

/// Where the turn that begins at `start` ends: the next message that is
/// the owner asking again. Words beside tool results are a steer inside the
/// turn, and a message the harness wrote is not the owner's.
fn end_of_turn(messages: &[Message], start: usize) -> usize {
    messages
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, m)| {
            m.role == Role::User
                && !m
                    .content
                    .iter()
                    .any(|b| matches!(b, Block::ToolResult { .. }))
                && !owner_text(m).trim().is_empty()
        })
        .map_or(messages.len(), |(at, _)| at)
}

/// The working calls of the turn that begins at `start`, in order: not the
/// harness's own, not a refusal by the owner or by policy, and not a memory
/// lookup.
fn working_calls(messages: &[Message], start: usize) -> Vec<(usize, Call)> {
    let mut pending: BTreeMap<&str, &str> = BTreeMap::new();
    let mut out = Vec::new();
    let end = end_of_turn(messages, start);
    for (at, message) in messages.iter().enumerate().take(end).skip(start) {
        for block in &message.content {
            match block {
                Block::ToolUse { id, name, .. } if !message.harness => {
                    pending.insert(id.as_str(), name.as_str());
                }
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error: false,
                } => {
                    let Some(name) = pending.remove(tool_use_id.as_str()) else {
                        continue;
                    };
                    if tool_use_id.starts_with(HARNESS_CALL)
                        || content.starts_with("Blocked by")
                        || content.starts_with("Denied by the user")
                        || name == NAME
                        || crate::tool::episodes::MEMORY_TOOLS.contains(&name)
                    {
                        continue;
                    }
                    out.push((
                        at,
                        Call {
                            id: tool_use_id.clone(),
                            tool: name.to_string(),
                        },
                    ));
                }
                _ => {}
            }
        }
    }
    out
}

/// The attempt a finished run made, when it used a tool: `start` is the
/// index of the owner's ask. A turn that only talked is not one — there is
/// nothing in it a later run could retrace.
pub fn of_turn(session: &str, messages: &[Message], start: usize, when: &str) -> Option<Attempt> {
    let ask = messages.get(start).filter(|m| m.role == Role::User)?;
    let asked = cut(&owner_text(ask), WORDS_BYTES);
    if asked.is_empty() {
        return None;
    }
    let calls: Vec<Call> = working_calls(messages, start)
        .into_iter()
        .map(|(_, call)| call)
        .collect();
    if calls.is_empty() {
        return None;
    }
    Some(Attempt {
        session: session.to_string(),
        turn: start,
        when: when.to_string(),
        asked,
        calls,
    })
}

fn distinct(tools: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in tools {
        if !out.contains(&tool) {
            out.push(tool);
        }
    }
    out
}

/// The owner's steers and denials inside the run that began at `start`:
/// each a verdict against that turn, with their words and the tools that
/// had worked by then. A later message that may be a correction is not
/// read here: whether it is one is a model's judgement, made overnight.
pub fn in_run(session: &str, messages: &[Message], start: usize, when: &str) -> Vec<Verdict> {
    let calls = working_calls(messages, start);
    let end = end_of_turn(messages, start);
    extract_interventions(messages)
        .into_iter()
        .filter(|i| {
            i.at > start && i.at < end && matches!(i.trigger, Trigger::Steer | Trigger::Denial)
        })
        .map(|i| Verdict {
            session: session.to_string(),
            turn: start,
            good: false,
            source: match i.trigger {
                Trigger::Steer => Source::Steer,
                _ => Source::Denial,
            },
            tool: None,
            why: Some(cut(&i.text, WORDS_BYTES)).filter(|w| !w.is_empty()),
            before: distinct(
                calls
                    .iter()
                    .filter(|(at, _)| *at < i.at)
                    .map(|(_, c)| c.tool.clone()),
            ),
            about: format!("message:{}", i.at),
            when: when.to_string(),
        })
        .collect()
}

/// What the owner did with a draft, as far as it is a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    SentUnchanged,
    /// The fields they changed, by name.
    Edited(Vec<String>),
    /// Their words, if they typed any.
    Rejected(Option<String>),
}

/// A draft the owner has acted on: the call that staged it and the act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: String,
    pub session: String,
    pub call: String,
    pub act: Act,
}

impl Draft {
    /// Read an outbox item as a verdict, when it is one: a model's draft,
    /// staged by a recorded call in a recorded session, that the **owner**
    /// released unchanged, edited and released, or rejected. Anything a
    /// run's shell did behind the approver, and anything from before acts
    /// were stamped, is not the owner's verdict (unknown is never the
    /// owner), and a pending draft is none yet.
    pub fn of(item: &crate::outbox::OutboxItem) -> Option<Draft> {
        use crate::closure::Actor;
        use crate::outbox::Author;
        if item.author() != Author::Model {
            return None;
        }
        let act = if item.owners_unchanged_release() {
            Act::SentUnchanged
        } else if item.status == "sent" && item.owners_edit() {
            let (before, after) = (item.args_before.as_object()?, item.args.as_object()?);
            let mut changed: Vec<String> = after
                .iter()
                .filter(|(key, value)| before.get(*key) != Some(value))
                .map(|(key, _)| key.clone())
                .collect();
            changed.extend(
                before
                    .keys()
                    .filter(|key| !after.contains_key(*key))
                    .cloned(),
            );
            Act::Edited(changed)
        } else if item.status == "rejected" && item.resolved_by() == Actor::Owner {
            Act::Rejected(item.owners_reason().map(|r| cut(r, WORDS_BYTES)))
        } else {
            return None;
        };
        Some(Draft {
            id: item.id.clone(),
            session: item.session_id.clone()?,
            call: item.call_id.clone()?,
            act,
        })
    }
}

/// The verdicts the owner's acts on drafts make, each joined to the attempt
/// whose call staged the draft. A draft no kept attempt staged is no
/// verdict on anything kept.
pub fn of_drafts(attempts: &[Attempt], drafts: &[Draft], when: &str) -> Vec<Verdict> {
    drafts
        .iter()
        .filter_map(|draft| {
            let attempt = attempts.iter().find(|a| {
                a.session == draft.session && a.calls.iter().any(|c| c.id == draft.call)
            })?;
            let at = attempt.calls.iter().position(|c| c.id == draft.call)?;
            let (good, source, why) = match &draft.act {
                Act::SentUnchanged => (true, Source::SentUnchanged, None),
                Act::Edited(fields) => (
                    false,
                    Source::Edit,
                    Some(
                        fields
                            .iter()
                            .map(|f| format!("`{}`", cut(f, 60)))
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                    .filter(|w| !w.is_empty()),
                ),
                Act::Rejected(words) => (false, Source::Reject, words.clone()),
            };
            Some(Verdict {
                session: attempt.session.clone(),
                turn: attempt.turn,
                good,
                source,
                tool: Some(attempt.calls[at].tool.clone()),
                why,
                before: distinct(attempt.calls[..at].iter().map(|c| c.tool.clone())),
                about: format!("draft:{}", draft.id),
                when: when.to_string(),
            })
        })
        .collect()
}

/// One line of the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Line {
    Attempt(Attempt),
    Verdict(Verdict),
    /// The owner deleted the conversation: nothing of it is read again.
    Forgotten {
        session: String,
    },
    /// Something was handed to a run: which attempt, about what, and when
    /// in the run. The record a bad hand-over is traced by.
    Handed {
        to_session: String,
        session: String,
        turn: usize,
        about: String,
        at: Moment,
        when: String,
    },
}

/// When in a run something was handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Moment {
    Start,
    Retraced,
}

/// What the file holds, read once.
#[derive(Debug, Clone, Default)]
pub struct Kept {
    pub attempts: Vec<Attempt>,
    /// The latest verdict for each thing a verdict is about.
    pub verdicts: Vec<Verdict>,
    pub handed: usize,
    /// Lines that could not be read. A finding, never nothing.
    pub unreadable: usize,
}

/// An attempt the owner judged against, with that verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub attempt: Attempt,
    pub verdict: Verdict,
}

impl Kept {
    /// The attempts the owner judged against, newest verdict first. An
    /// attempt with a good verdict and a bad one is here for the bad one: a
    /// draft sent as written does not answer for a steer two calls earlier.
    pub fn failures(&self) -> Vec<Failure> {
        let mut out: Vec<Failure> = self
            .verdicts
            .iter()
            .filter(|v| !v.good && v.source != Source::Unknown)
            .filter_map(|v| {
                let attempt = self
                    .attempts
                    .iter()
                    .find(|a| a.session == v.session && a.turn == v.turn)?;
                Some(Failure {
                    attempt: attempt.clone(),
                    verdict: v.clone(),
                })
            })
            .collect();
        out.reverse();
        out
    }

    /// Whether this exact verdict is already the one on file for what it is
    /// about, so a pass that reads the same act again writes nothing.
    pub fn holds(&self, verdict: &Verdict) -> bool {
        self.verdicts.iter().any(|v| {
            v.session == verdict.session
                && v.about == verdict.about
                && v.good == verdict.good
                && v.source == verdict.source
                && v.why == verdict.why
        })
    }
}

/// Where a mecha home keeps its attempts.
pub fn dir_in(home: &Path) -> PathBuf {
    home.join("attempts")
}

/// The home's attempts: one append-only file, the owner's alone.
pub struct Store {
    file: PathBuf,
}

impl Store {
    /// Under a mecha home.
    pub fn in_home(home: &Path) -> Store {
        Store::in_dir(&dir_in(home))
    }

    /// In a directory of its own.
    pub fn in_dir(dir: &Path) -> Store {
        Store {
            file: dir.join("attempts.jsonl"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    fn append(&self, lines: &[Line]) -> Result<()> {
        use std::io::Write;
        if lines.is_empty() {
            return Ok(());
        }
        let dir = self
            .file
            .parent()
            .context("the attempts file has no directory")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(0o600);
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("closing {}", dir.display()))?;
        }
        let mut text = String::new();
        for line in lines {
            text.push_str(&serde_json::to_string(line)?);
            text.push('\n');
        }
        options
            .open(&self.file)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .with_context(|| format!("writing {}", self.file.display()))
    }

    /// Keep a finished turn: its attempt, and the verdicts already known.
    pub fn keep(&self, attempt: &Attempt, verdicts: &[Verdict]) -> Result<()> {
        let mut lines = vec![Line::Attempt(attempt.clone())];
        lines.extend(verdicts.iter().cloned().map(Line::Verdict));
        self.append(&lines)
    }

    /// Keep verdicts that arrived later; those already on file are skipped.
    /// How many were written.
    pub fn judge(&self, verdicts: &[Verdict]) -> Result<usize> {
        let kept = self.load()?;
        let new: Vec<Line> = verdicts
            .iter()
            .filter(|v| !kept.holds(v))
            .cloned()
            .map(Line::Verdict)
            .collect();
        self.append(&new)?;
        Ok(new.len())
    }

    /// The owner deleted a conversation: its attempts and verdicts are not
    /// read again, and nothing that came from it is handed over.
    pub fn forget(&self, session: &str) -> Result<()> {
        self.append(&[Line::Forgotten {
            session: session.to_string(),
        }])
    }

    /// Record what a run was handed.
    pub fn handed(&self, to_session: &str, what: &[(Failure, Moment)], when: &str) -> Result<()> {
        let lines: Vec<Line> = what
            .iter()
            .map(|(failure, at)| Line::Handed {
                to_session: to_session.to_string(),
                session: failure.attempt.session.clone(),
                turn: failure.attempt.turn,
                about: failure.verdict.about.clone(),
                at: *at,
                when: when.to_string(),
            })
            .collect();
        self.append(&lines)
    }

    /// Everything on file. A file that is not there is an empty store; one
    /// that cannot be read is an error, never an empty one.
    pub fn load(&self) -> Result<Kept> {
        let text = match std::fs::read_to_string(&self.file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Kept::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", self.file.display())),
        };
        let mut kept = Kept::default();
        let mut forgotten: BTreeSet<String> = BTreeSet::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Line>(line) {
                Ok(Line::Attempt(attempt)) => kept.attempts.push(attempt),
                Ok(Line::Verdict(verdict)) => {
                    kept.verdicts
                        .retain(|v| !(v.session == verdict.session && v.about == verdict.about));
                    kept.verdicts.push(verdict);
                }
                Ok(Line::Forgotten { session }) => {
                    forgotten.insert(session);
                }
                Ok(Line::Handed { .. }) => kept.handed += 1,
                Err(_) => kept.unreadable += 1,
            }
        }
        kept.attempts.retain(|a| !forgotten.contains(&a.session));
        kept.verdicts.retain(|v| !forgotten.contains(&v.session));
        Ok(kept)
    }
}

fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= WORD_CHARS)
        .map(str::to_lowercase)
        .collect()
}

fn tools(names: &[String]) -> String {
    names
        .iter()
        .map(|t| format!("`{t}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Failure {
    /// What the owner did and said, in the harness's words and theirs.
    fn render(&self) -> String {
        let v = &self.verdict;
        let tool = v.tool.as_deref().unwrap_or("a tool");
        let said = |lead: &str| match &v.why {
            Some(why) => format!("{lead} \"{why}\""),
            None => String::new(),
        };
        let what = match v.source {
            Source::Edit => format!(
                "The draft that attempt staged with `{tool}` was edited by the owner before it was sent{}.",
                match &v.why {
                    Some(fields) => format!(" (they changed {fields})"),
                    None => String::new(),
                }
            ),
            Source::Reject => format!(
                "The draft that attempt staged with `{tool}` was rejected by the owner{}.",
                said(", who said:")
            ),
            Source::Denial => format!(
                "The owner refused a call that attempt wanted to make{}.",
                said(":")
            ),
            _ => format!(
                "The owner stepped in during that attempt{}.",
                said(" and said:")
            ),
        };
        let mut out = format!("- You were asked before: \"{}\"\n", self.attempt.asked);
        if !v.before.is_empty() {
            out.push_str(&format!("  That attempt had used {}.\n", tools(&v.before)));
        }
        out.push_str(&format!("  {what}\n"));
        out
    }
}

/// What one run watches for: the judged attempts at asks like its own, and
/// the tools it has used.
pub struct Watch {
    failures: Vec<(Failure, bool)>,
    called: BTreeSet<String>,
    handed: Vec<(Failure, Moment)>,
}

impl Watch {
    /// The judged attempts whose ask shares enough words with `ask`,
    /// newest first, a few at most.
    pub fn new(failures: Vec<Failure>, ask: &str) -> Watch {
        let ask = words(ask);
        let like = failures
            .into_iter()
            .filter(|f| words(&f.attempt.asked).intersection(&ask).count() >= SHARED_WORDS)
            .take(WATCHED)
            .map(|f| (f, false))
            .collect();
        Watch {
            failures: like,
            called: BTreeSet::new(),
            handed: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
    }

    /// What is said before the run's first request: the judged attempts
    /// that had used no tool by the judged point, since nothing the run
    /// does later could mark the moment for them. `None` when there are
    /// none.
    pub fn start(&mut self) -> Option<String> {
        let mut out = String::new();
        for (failure, said) in &mut self.failures {
            if !*said && failure.verdict.before.is_empty() {
                *said = true;
                out.push_str(&failure.render());
                self.handed.push((failure.clone(), Moment::Start));
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// A call that worked, to what the harness says after it: each judged
    /// attempt once, at the first call after which this run has used every
    /// tool that attempt had used by the judged point. That is where the
    /// owner stepped in last time, or where the draft they changed was
    /// about to be written.
    pub fn after(&mut self, tool: &str) -> Option<String> {
        self.called.insert(tool.to_string());
        let called = &self.called;
        let (failure, said) = self.failures.iter_mut().find(|(f, said)| {
            !*said
                && !f.verdict.before.is_empty()
                && f.verdict.before.iter().all(|t| called.contains(t))
        })?;
        *said = true;
        self.handed.push((failure.clone(), Moment::Retraced));
        Some(failure.render())
    }

    /// What this run has been handed so far, for the record.
    pub fn handed(&self) -> &[(Failure, Moment)] {
        &self.handed
    }
}

/// A watch shared between the run that builds it and the loop that asks it.
pub type Shared = Arc<Mutex<Watch>>;

/// What the loop calls after a call that worked ([`Watch::after`]).
pub fn cue(watch: Shared) -> SuccessCueText {
    Arc::new(move |tool, _input, _result| watch.lock().ok()?.after(tool))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, result: &str, is_error: bool) -> [Message; 2] {
        let mut results = Message::tool_results(vec![Block::ToolResult {
            tool_use_id: id.into(),
            content: result.into(),
            is_error,
        }]);
        results.tool_provenance.insert(id.into(), true);
        [
            Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: name.into(),
                input: json!({"secret": "the body of a letter"}),
            }]),
            results,
        ]
    }

    fn chat(turns: Vec<Vec<Message>>) -> Vec<Message> {
        turns.into_iter().flatten().collect()
    }

    fn drafted(ask: &str, staged: &str) -> Vec<Message> {
        let mut out = vec![Message::user(ask)];
        out.extend(call("s1", "mail_search", "three threads", false));
        out.extend(call(staged, "mail_send", "staged", false));
        out.push(Message::assistant(vec![Block::text("Drafted.")]));
        out
    }

    #[test]
    fn a_turn_that_used_a_tool_is_an_attempt_and_one_that_only_talked_is_not() {
        let messages = chat(vec![
            vec![
                Message::user("what is the capital of the moon"),
                Message::assistant(vec![Block::text("It has none.")]),
            ],
            drafted("write to the harbour office about the berth", "c9"),
        ]);
        assert_eq!(of_turn("s", &messages, 0, "t"), None);
        let attempt = of_turn("s", &messages, 2, "t").unwrap();
        assert_eq!(attempt.turn, 2);
        assert_eq!(attempt.asked, "write to the harbour office about the berth");
        assert_eq!(
            attempt.calls,
            vec![
                Call {
                    id: "s1".into(),
                    tool: "mail_search".into()
                },
                Call {
                    id: "c9".into(),
                    tool: "mail_send".into()
                },
            ]
        );
        // Nothing a call was made with or returned is kept.
        let line = serde_json::to_string(&attempt).unwrap();
        assert!(
            !line.contains("letter") && !line.contains("threads"),
            "{line}"
        );
        // Not an ask: no attempt.
        assert_eq!(of_turn("s", &messages, 1, "t"), None);
    }

    #[test]
    fn a_refused_or_failed_call_and_a_memory_lookup_are_not_working_calls() {
        let mut messages = vec![Message::user("tidy the calendar for the week")];
        messages.extend(call("a", "calendar_list", "no such calendar", true));
        messages.extend(call(
            "b",
            "fs_write",
            "Denied by the user: not there",
            false,
        ));
        messages.extend(call("c", NAME, "nothing", false));
        messages.extend(call("d", "episode_recall", "nothing", false));
        assert_eq!(of_turn("s", &messages, 0, "t"), None);
        messages.extend(call("e", "calendar_list", "two events", false));
        let attempt = of_turn("s", &messages, 0, "t").unwrap();
        assert_eq!(attempt.calls.len(), 1);
        assert_eq!(attempt.calls[0].tool, "calendar_list");
    }

    #[test]
    fn a_steer_is_a_verdict_with_the_owner_s_words_and_the_tools_used_by_then() {
        let mut messages = vec![Message::user("find the invoice from the mill and file it")];
        messages.extend(call("a", "mail_search", "five threads", false));
        // The owner steers beside the next results.
        let [use_b, mut result_b] = call("b", "docs_search", "nothing", false);
        result_b
            .content
            .push(Block::text("not the documents, it is in the shared drive"));
        messages.extend([use_b, result_b]);
        messages.extend(call("c", "drive_search", "found", false));
        messages.push(Message::assistant(vec![Block::text("Filed.")]));
        let verdicts = in_run("s", &messages, 0, "t");
        assert_eq!(verdicts.len(), 1, "{verdicts:?}");
        let v = &verdicts[0];
        assert!(!v.good && v.source == Source::Steer);
        assert_eq!(
            v.why.as_deref(),
            Some("not the documents, it is in the shared drive")
        );
        // What had worked before the owner spoke; not what came after.
        assert_eq!(v.before, vec!["mail_search".to_string()]);
        // An earlier turn's steer is not this turn's.
        assert!(in_run("s", &messages, 4, "t").is_empty());
    }

    fn draft(id: &str, call: &str, act: Act) -> Draft {
        Draft {
            id: id.into(),
            session: "s".into(),
            call: call.into(),
            act,
        }
    }

    #[test]
    fn a_draft_s_fate_is_joined_to_the_attempt_whose_call_staged_it() {
        let messages = chat(vec![
            drafted("write to the harbour office about the berth", "c1"),
            drafted("write to the mill about the invoice", "c2"),
        ]);
        let attempts = vec![
            of_turn("s", &messages, 0, "t").unwrap(),
            of_turn("s", &messages, 6, "t").unwrap(),
        ];
        let verdicts = of_drafts(
            &attempts,
            &[
                draft("d1", "c1", Act::Rejected(Some("too formal".into()))),
                draft(
                    "d2",
                    "c2",
                    Act::Edited(vec!["body".into(), "subject".into()]),
                ),
                draft("d3", "c2", Act::SentUnchanged),
                // Staged by a call no kept attempt made.
                draft("d4", "zz", Act::Rejected(None)),
            ],
            "t",
        );
        assert_eq!(verdicts.len(), 3);
        let (reject, edit, sent) = (&verdicts[0], &verdicts[1], &verdicts[2]);
        assert_eq!(
            (reject.turn, reject.good, reject.source),
            (0, false, Source::Reject)
        );
        assert_eq!(reject.why.as_deref(), Some("too formal"));
        assert_eq!(reject.tool.as_deref(), Some("mail_send"));
        assert_eq!(reject.before, vec!["mail_search".to_string()]);
        assert_eq!(
            (edit.turn, edit.good, edit.source),
            (6, false, Source::Edit)
        );
        // Which fields, never what they held.
        assert_eq!(edit.why.as_deref(), Some("`body`, `subject`"));
        assert_eq!((sent.good, sent.source), (true, Source::SentUnchanged));
    }

    fn store() -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mecha-attempts-{}", uuid::Uuid::new_v4()));
        (Store::in_home(&dir), dir)
    }

    #[test]
    fn the_file_keeps_the_latest_verdict_and_forgets_a_deleted_conversation() {
        let (store, dir) = store();
        assert_eq!(store.load().unwrap().attempts.len(), 0);
        let messages = drafted("write to the harbour office about the berth", "c1");
        let attempt = of_turn("s", &messages, 0, "t").unwrap();
        store.keep(&attempt, &[]).unwrap();
        let reject = of_drafts(
            &[attempt.clone()],
            &[draft("d1", "c1", Act::Rejected(None))],
            "t",
        );
        assert_eq!(store.judge(&reject).unwrap(), 1);
        // The same act read again writes nothing.
        assert_eq!(store.judge(&reject).unwrap(), 0);
        // The owner then typed a reason: the later verdict stands.
        let reasoned = of_drafts(
            &[attempt.clone()],
            &[draft(
                "d1",
                "c1",
                Act::Rejected(Some("wrong person".into())),
            )],
            "t",
        );
        assert_eq!(store.judge(&reasoned).unwrap(), 1);
        let kept = store.load().unwrap();
        assert_eq!(
            (kept.attempts.len(), kept.verdicts.len(), kept.unreadable),
            (1, 1, 0)
        );
        assert_eq!(
            kept.failures()[0].verdict.why.as_deref(),
            Some("wrong person")
        );

        // A line this build cannot read is counted, never taken for nothing.
        std::fs::write(
            store.path(),
            format!(
                "{}not json\n",
                std::fs::read_to_string(store.path()).unwrap()
            ),
        )
        .unwrap();
        assert_eq!(store.load().unwrap().unreadable, 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        store.forget("s").unwrap();
        let kept = store.load().unwrap();
        assert!(kept.attempts.is_empty() && kept.verdicts.is_empty());
        assert!(kept.failures().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn failure(asked: &str, source: Source, before: &[&str], why: Option<&str>) -> Failure {
        Failure {
            attempt: Attempt {
                session: "old".into(),
                turn: 0,
                when: "t".into(),
                asked: asked.into(),
                calls: vec![],
            },
            verdict: Verdict {
                session: "old".into(),
                turn: 0,
                good: false,
                source,
                tool: Some("mail_send".into()),
                why: why.map(str::to_string),
                before: before.iter().map(|t| t.to_string()).collect(),
                about: format!("draft:{asked}"),
                when: "t".into(),
            },
        }
    }

    #[test]
    fn a_judged_attempt_at_a_like_ask_is_named_when_the_run_has_retraced_it() {
        let failures = vec![
            failure(
                "write to the harbour office about the berth",
                Source::Reject,
                &["mail_search", "contacts_find"],
                Some("too formal"),
            ),
            // Nothing had been used by the judged point: said at the start.
            failure(
                "write the harbour office a note",
                Source::Edit,
                &[],
                Some("`body`"),
            ),
            // An ask unlike this one.
            failure("book the dentist for Tuesday", Source::Reject, &[], None),
        ];
        let mut watch = Watch::new(failures, "write to the harbour office about the crane");
        let start = watch.start().unwrap();
        assert!(
            start.contains("was edited by the owner before it was sent (they changed `body`)")
                && !start.contains("dentist")
                && !start.contains("too formal"),
            "{start}"
        );
        assert_eq!(watch.start(), None);
        // Not until every tool that attempt had used has worked here.
        assert_eq!(watch.after("mail_search"), None);
        assert_eq!(watch.after("fs_read"), None);
        let said = watch.after("contacts_find").unwrap();
        for needle in [
            "You were asked before: \"write to the harbour office about the berth\"",
            "had used `mail_search`, `contacts_find`",
            "staged with `mail_send` was rejected by the owner, who said: \"too formal\"",
        ] {
            assert!(said.contains(needle), "{needle}: {said}");
        }
        // Once.
        assert_eq!(watch.after("contacts_find"), None);
        let moments: Vec<Moment> = watch.handed().iter().map(|(_, m)| *m).collect();
        assert_eq!(moments, vec![Moment::Start, Moment::Retraced]);
    }

    #[test]
    fn a_verdict_from_a_source_this_build_does_not_know_is_not_used() {
        let line = r#"{"verdict":{"session":"s","turn":0,"good":false,"source":"a_later_source","about":"x","when":"t"}}"#;
        let Line::Verdict(verdict) = serde_json::from_str::<Line>(line).unwrap() else {
            panic!("not a verdict");
        };
        assert_eq!(verdict.source, Source::Unknown);
        let kept = Kept {
            attempts: vec![Attempt {
                session: "s".into(),
                turn: 0,
                when: "t".into(),
                asked: "an ask".into(),
                calls: vec![],
            }],
            verdicts: vec![verdict],
            ..Default::default()
        };
        assert!(kept.failures().is_empty());
    }
}
