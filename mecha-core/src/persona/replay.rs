//! Replaying a persona chat from one of its turns: what the turn's run
//! started from, what an arm changes, and what each sample did.
//!
//! The run itself is built by [`super::turn::context`], the function serve
//! calls, on an agent built by `setup::persona_agent`, which serve calls too.
//! This module holds the parts that are not serve's:
//!
//! - [`Branch`]: the transcript read up to an owner turn's record line, with
//!   that turn's recorded notes. `Session::parse` reads the prefix, so the
//!   messages, rewrites and taint are what the run started from rather than a
//!   reconstruction of them.
//! - [`Overlay`]: an arm's text edits, applied to the built request (system
//!   text, tool descriptions, notes) and never to the transcript. An edit that
//!   matches nothing fails the sample: an arm that silently changed nothing
//!   would measure the baseline twice.
//! - [`Capture`]: a provider wrapper that applies the overlay, records each
//!   exchange, and ends the run after the requests it was allowed.
//! - [`Sample`]: one sample's facts, as JSON. It holds what the model wrote,
//!   so it lives in the private research store; a report quotes counts.

use crate::agent::Taint;
use crate::message::{Block, CompletionRequest, CompletionResponse, Message, Role, ToolSpec};
use crate::provider::{Provider, StreamSink};
use crate::session::{RunConfig, Session};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// One owner turn in a transcript: where a sample can branch.
#[derive(Debug, Clone, Serialize)]
pub struct OwnerTurn {
    /// 1-based record line of the turn's message, or of the rewrite that
    /// folded it into a tail the owner already held.
    pub line: usize,
    /// 1-based count of owner turns up to this one.
    pub turn: usize,
    /// How many messages the run started from.
    pub messages: usize,
    /// How many notes were recorded for the run.
    pub notes: usize,
    pub spoken: bool,
    /// A panel turn: its fact was added during the run (`Record::Extend`).
    pub panel: bool,
}

/// What a turn's run started from.
#[derive(Debug, Clone)]
pub struct Branch {
    pub line: usize,
    pub messages: Vec<Message>,
    pub taint: Taint,
    /// The notes recorded for this run, in order (`Record::Notes`).
    pub notes: Vec<String>,
    /// The owner's words this turn, harness text removed.
    pub owner: String,
    pub spoken: bool,
    /// The run config in force for the turn: the last recorded before it.
    pub config: Option<RunConfig>,
    /// The calendar reference the turn's run was sent, as recorded after
    /// it (`Session::record_run`): what [`clock_for`] matches a clock to.
    pub calendar: Option<String>,
    /// A branch inside the turn's run, after the batch holding its `n`th
    /// tool call (1-based): [`branch_at_call`].
    pub call: Option<usize>,
}

/// The calendar reference recorded for the run that a turn at `index` began:
/// the first harness note after the turn and before the next one.
fn calendar_after(lines: &[&str], index: usize) -> Option<String> {
    for l in &lines[index + 1..] {
        if opens_owner_turn(l) {
            return None;
        }
        if record_kind(l).as_deref() != Some("notes") {
            continue;
        }
        let v: Value = serde_json::from_str(l).ok()?;
        let found = v
            .get("notes")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .find(|n| n.starts_with(crate::date_context::REFERENCE_STEM));
        if let Some(n) = found {
            return Some(n.to_string());
        }
    }
    None
}

/// A clock for the replay whose calendar reference reads as the recorded
/// one: noon, in the run's zone, on the first day from `from` (up to 400
/// days on) that renders `calendar` exactly. `None` when no day does — a
/// note from another zone, or a format this build no longer writes — and
/// the caller says the calendar differs rather than sending a wrong one
/// quietly.
pub fn clock_for(
    calendar: &str,
    from: chrono::DateTime<chrono::Utc>,
    tz: Option<chrono_tz::Tz>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Duration, TimeZone};
    let start = from.date_naive();
    (0..=400).find_map(|k| {
        let day = start + Duration::days(k);
        let noon = day.and_hms_opt(12, 0, 0)?;
        let at = match tz {
            Some(tz) => tz
                .from_local_datetime(&noon)
                .earliest()?
                .with_timezone(&chrono::Utc),
            None => chrono::Local
                .from_local_datetime(&noon)
                .earliest()?
                .with_timezone(&chrono::Utc),
        };
        (crate::date_context::render(at, tz) == calendar).then_some(at)
    })
}

fn record_kind(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    v.get("record")?.as_str().map(str::to_string)
}

/// Whether record line `line` (one JSONL line) opens an owner turn: a user
/// message with the owner's words, or a rewrite whose tail is one.
fn opens_owner_turn(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let message = match v.get("record").and_then(Value::as_str) {
        Some("message") => v.clone(),
        Some("rewrite") => match v.get("messages").and_then(Value::as_array) {
            Some(list) => match list.last() {
                Some(last) => last.clone(),
                None => return false,
            },
            None => return false,
        },
        _ => return false,
    };
    let Ok(m) = serde_json::from_value::<Message>(message) else {
        return false;
    };
    // A rewrite is also how compaction and rollbacks are recorded; only one
    // the owner's words end, and that a notes record or a message precedes
    // as serve writes them, is a turn. Tool results ride in user messages
    // too, and carry no owner words.
    m.role == Role::User
        && !m
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolResult { .. }))
        && !crate::agent::owner_text(&m).trim().is_empty()
}

/// The notes recorded for the run whose owner turn is at `index` (0-based):
/// serve writes them on the line just before the turn, after any `Config`.
///
/// Stepping back over `Config` records can never reach the previous run's
/// notes, because `Session::record_run` closes every run with a `GoalAnchor`
/// after its harness notes, and the walk stops at any other record.
fn notes_before(lines: &[&str], index: usize) -> Vec<String> {
    let mut i = index;
    while i > 0 {
        i -= 1;
        match record_kind(lines[i]).as_deref() {
            Some("notes") => {
                let v: Value = serde_json::from_str(lines[i]).unwrap_or_default();
                return v
                    .get("notes")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Some("config") => continue,
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

/// Whether the run that a turn at `index` started added the panel's fact.
fn is_panel_turn(lines: &[&str], index: usize) -> bool {
    lines
        .get(index + 1)
        .is_some_and(|l| record_kind(l).as_deref() == Some("extend"))
}

fn non_empty(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.trim().is_empty()).collect()
}

/// Every owner turn in a transcript's text, in order.
pub fn owner_turns(path: &Path, text: &str) -> Result<Vec<OwnerTurn>> {
    let lines = non_empty(text);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !opens_owner_turn(line) {
            continue;
        }
        let prefix = lines[..=i].join("\n");
        let messages = Session::parse(path, &prefix)?.convo.messages.len();
        let notes = notes_before(&lines, i);
        out.push(OwnerTurn {
            line: i + 1,
            turn: out.len() + 1,
            messages,
            spoken: notes.iter().any(|n| super::call::is_note(n)),
            notes: notes.len(),
            panel: is_panel_turn(&lines, i),
        });
    }
    Ok(out)
}

/// The transcript at the owner turn on record line `line` (1-based, blank
/// lines not counted), as the turn's run started from it.
pub fn branch_at(path: &Path, text: &str, line: usize) -> Result<Branch> {
    let lines = non_empty(text);
    if line == 0 || line > lines.len() {
        bail!(
            "{} has {} record lines; line {line} is not one of them",
            path.display(),
            lines.len()
        );
    }
    let index = line - 1;
    if !opens_owner_turn(lines[index]) {
        bail!(
            "record line {line} of {} is a `{}` record, not an owner turn — \
             `mecha replay --persona {} --list` names the turns",
            path.display(),
            record_kind(lines[index]).unwrap_or_else(|| "unreadable".into()),
            path.display()
        );
    }
    if is_panel_turn(&lines, index) {
        bail!(
            "record line {line} is a picture-panel turn: the harness drew its change \
             before the persona replied, and replay does not redraw it yet"
        );
    }
    let prefix = lines[..=index].join("\n");
    let transcript = Session::parse(path, &prefix)?;
    let messages = transcript.convo.messages;
    let owner = messages
        .last()
        .map(crate::agent::owner_text)
        .unwrap_or_default();
    let notes = notes_before(&lines, index);
    // The taint the run started under: the transcript's, which `parse` armed
    // for the content and the notes it read, as the run arms them at start.
    let mut taint = transcript.convo.taint;
    taint.arm_for_content(&messages);
    taint.arm_for_notes(&notes);
    Ok(Branch {
        call: None,
        calendar: calendar_after(&lines, index),
        line,
        spoken: notes.iter().any(|n| super::call::is_note(n)),
        config: transcript.configs.last().cloned(),
        messages,
        taint,
        notes,
        owner,
    })
}

/// The turn on record line `line`, branched inside its run: after the batch
/// holding the run's `call`th tool call (1-based) and the results that
/// answered it, as recorded. The run's notes are the turn's, as they were
/// for every request of it.
///
/// Refused where the run's next request was not an ordinary one: a picture
/// the call deferred made the next request the closing one, which replay
/// does not rebuild; and a run that rewrote its history (a compaction) has
/// no recorded place for the call among the messages the turn began with.
pub fn branch_at_call(path: &Path, text: &str, line: usize, call: usize) -> Result<Branch> {
    if call == 0 {
        bail!("calls count from 1");
    }
    let mut branch = branch_at(path, text, line)?;
    let lines = non_empty(text);
    let end = lines
        .iter()
        .enumerate()
        .skip(line)
        .find(|(_, l)| opens_owner_turn(l))
        .map_or(lines.len(), |(i, _)| i);
    let run = Session::parse(path, &lines[..end].join("\n"))?;
    let start = branch.messages.len();
    let messages = run.convo.messages;
    if messages.len() < start || messages[..start] != branch.messages[..] {
        bail!(
            "the run at line {line} rewrote its history, so call {call} has no place \
             among the messages the turn began with"
        );
    }
    let mut seen = 0;
    let mut target = None;
    'find: for m in &messages[start..] {
        for b in &m.content {
            if let Block::ToolUse { id, .. } = b {
                seen += 1;
                if seen == call {
                    target = Some(id.clone());
                    break 'find;
                }
            }
        }
    }
    let Some(target) = target else {
        bail!("the run at line {line} made {seen} tool call(s), not {call}");
    };
    let answered = messages[start..]
        .iter()
        .position(|m| {
            m.content.iter().any(
                |b| matches!(b, Block::ToolResult { tool_use_id, .. } if *tool_use_id == target),
            )
        })
        .map(|i| start + i)
        .with_context(|| format!("call {call} has no recorded result"))?;
    let deferred = messages[answered].content.iter().any(|b| {
        matches!(b, Block::ToolResult { content, .. }
            if content.starts_with(crate::imagegen::BEING_MADE))
    });
    if deferred {
        bail!(
            "call {call}'s picture was deferred, so the run's next request was its closing \
             one, which replay does not rebuild"
        );
    }
    branch.messages = messages[..=answered].to_vec();
    let mut taint = run.convo.taint;
    taint.arm_for_content(&branch.messages);
    taint.arm_for_notes(&branch.notes);
    branch.taint = taint;
    branch.call = Some(call);
    Ok(branch)
}

/// What a sample's picture call made, read back from its scratch store:
/// the stage it ran against, the call's results, and the files it wrote.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RenderFacts {
    /// The scratch folder the sample was staged into, kept for the judges.
    pub scratch: String,
    /// What the staged scene was recovered from (`scene::stage::AsOf`).
    pub as_of: String,
    /// What the stage could not bring.
    pub missing: Vec<String>,
    /// Whether the scene reader and the role splitter were stamped.
    pub readers: bool,
    /// Each picture call's result as the model was handed it.
    pub results: Vec<CallResult>,
    /// Pictures and manifests the sample wrote, as absolute paths.
    pub pictures: Vec<String>,
    pub manifests: Vec<Value>,
    /// The prompt log's lines: what the image model was given.
    pub prompt_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallResult {
    pub name: String,
    pub is_error: bool,
    pub content: String,
}

/// The results in `after` (the messages a sample's run added) of calls to
/// `name`.
pub fn results_of(after: &[Message], name: &str) -> Vec<CallResult> {
    let ids: std::collections::BTreeSet<&str> = after
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolUse { id, name: n, .. } if n == name => Some(id.as_str()),
            _ => None,
        })
        .collect();
    after
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if ids.contains(tool_use_id.as_str()) => Some(CallResult {
                name: name.to_string(),
                is_error: *is_error,
                content: content.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Where an overlay edit applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The system prompt.
    System,
    /// Every tool's description and the `description` strings in its schema.
    Tools,
    /// One tool's.
    Tool(String),
    /// The run's notes, as sent at the end of the last user message.
    Notes,
}

impl Target {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "system" => Target::System,
            "tools" => Target::Tools,
            "notes" => Target::Notes,
            _ => match s.strip_prefix("tool:") {
                Some(name) if !name.is_empty() => Target::Tool(name.to_string()),
                _ => bail!("overlay `in` must be system, tools, tool:<name> or notes, not `{s}`"),
            },
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ReplaceFile {
    #[serde(rename = "in")]
    target: String,
    find: String,
    with: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlayFile {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    replace: Vec<ReplaceFile>,
}

/// One text edit.
#[derive(Debug, Clone)]
pub struct Replace {
    pub target: Target,
    pub find: String,
    pub with: String,
}

/// An arm's text edits to the built request (`--overlay arm.toml`):
///
/// ```toml
/// name = "C"
/// [[replace]]
/// in = "tool:image_generate"   # system | tools | tool:<name> | notes
/// find = "an old sentence"
/// with = "a new one"
/// ```
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    pub name: Option<String>,
    pub replace: Vec<Replace>,
    /// sha256 of the file as read, so a sample names the arm it ran.
    pub digest: Option<String>,
}

impl Overlay {
    pub fn parse(text: &str) -> Result<Self> {
        let file: OverlayFile = toml::from_str(text).context("reading the overlay")?;
        let mut replace = Vec::new();
        for r in file.replace {
            if r.find.is_empty() {
                bail!("an overlay `find` is empty");
            }
            replace.push(Replace {
                target: Target::parse(&r.target)?,
                find: r.find,
                with: r.with,
            });
        }
        Ok(Overlay {
            name: file.name,
            replace,
            digest: Some(crate::document::sha256_hex(text.as_bytes())),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.replace.is_empty()
    }

    /// Apply every edit to `req`, and how many places each matched.
    pub fn apply(&self, req: &mut CompletionRequest) -> Vec<usize> {
        self.replace.iter().map(|r| r.apply(req)).collect()
    }
}

fn replace_in(text: &mut String, find: &str, with: &str) -> usize {
    let n = text.matches(find).count();
    if n > 0 {
        *text = text.replace(find, with);
    }
    n
}

/// Replace inside every `description` string of a JSON schema.
fn replace_in_schema(schema: &mut Value, find: &str, with: &str) -> usize {
    match schema {
        Value::Object(map) => map
            .iter_mut()
            .map(|(k, v)| match v {
                Value::String(s) if k == "description" => replace_in(s, find, with),
                other => replace_in_schema(other, find, with),
            })
            .sum(),
        Value::Array(items) => items
            .iter_mut()
            .map(|v| replace_in_schema(v, find, with))
            .sum(),
        _ => 0,
    }
}

fn replace_in_tool(spec: &mut ToolSpec, find: &str, with: &str) -> usize {
    replace_in(&mut spec.description, find, with)
        + replace_in_schema(&mut spec.input_schema, find, with)
}

impl Replace {
    fn apply(&self, req: &mut CompletionRequest) -> usize {
        let (find, with) = (self.find.as_str(), self.with.as_str());
        match &self.target {
            Target::System => req.system.as_mut().map_or(0, |s| replace_in(s, find, with)),
            Target::Tools => req
                .tools
                .iter_mut()
                .map(|t| replace_in_tool(t, find, with))
                .sum(),
            Target::Tool(name) => req
                .tools
                .iter_mut()
                .filter(|t| &t.name == name)
                .map(|t| replace_in_tool(t, find, with))
                .sum(),
            // The notes are the last `trailing_notes` blocks of the last
            // user message (`agent::wire`).
            Target::Notes => {
                let n = req.trailing_notes;
                let Some(last) = req.messages.last_mut().filter(|m| m.role == Role::User) else {
                    return 0;
                };
                let from = last.content.len().saturating_sub(n);
                last.content[from..]
                    .iter_mut()
                    .map(|b| match b {
                        Block::Text { text } => replace_in(text, find, with),
                        _ => 0,
                    })
                    .sum()
            }
        }
    }
}

/// A fingerprint of a request's content: two requests with the same one
/// sent the same system text, tools, messages and limits. Not the sampler:
/// effort, thinking and cache flags are left out, and only the wire body's
/// digest (`Sample::wire_digest`) says two requests were sent the same.
pub fn request_digest(req: &CompletionRequest) -> String {
    let v = serde_json::json!({
        "model": req.model,
        "system": req.system,
        "tools": req.tools,
        "messages": req.messages,
        "max_tokens": req.max_tokens,
        "think_budget": req.think_budget,
        "trailing_notes": req.trailing_notes,
        "tool_choice": format!("{:?}", req.tool_choice),
    });
    crate::document::sha256_hex(v.to_string().as_bytes())
}

/// One request and what came back.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub digest: String,
    /// The body the provider built for the request, as sent
    /// (`Provider::wire_body`): stream flag, sampler fields and all.
    pub wire: Option<Value>,
    /// The history's fingerprint alone: equal across arms that change only
    /// the system text or the tools.
    pub messages_digest: String,
    pub response: std::result::Result<CompletionResponse, String>,
    pub wall_secs: f64,
}

/// A sample asked for a request past those it was allowed: how a sample
/// that rendered ends, and not a failure.
#[derive(Debug, Clone, Copy)]
pub struct SampleEnded(pub usize);

impl std::fmt::Display for SampleEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "replay: this sample has sent the {} request(s) it was allowed",
            self.0
        )
    }
}

impl std::error::Error for SampleEnded {}

/// Whether `e` is a sample ending where it was meant to.
pub fn ended(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<SampleEnded>())
}

/// A provider that applies an arm's overlay, records each exchange, and
/// ends the run once it has sent `requests` of them: it cancels the run's
/// token as the last allowed response comes back, so the loop stops at its
/// next safe point, and refuses any request past that.
pub struct Capture {
    pub inner: Box<dyn Provider>,
    pub overlay: Arc<Overlay>,
    /// Sent as each request's `max_tokens` when set.
    pub max_tokens: Option<u32>,
    pub requests: usize,
    /// Cancel as the last allowed response comes back, so its calls are
    /// never run (a call-only sample). Off for a sample that renders: its
    /// picture call runs, and the run ends when it asks again, with
    /// [`SampleEnded`]. A cancel before the call would stop the render it
    /// is there to make.
    pub cancel_after_last: bool,
    pub cancel: CancellationToken,
    pub log: Arc<Mutex<Vec<Exchange>>>,
}

#[async_trait::async_trait]
impl Provider for Capture {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn default_model(&self) -> &str {
        self.inner.default_model()
    }
    fn vision(&self) -> bool {
        self.inner.vision()
    }
    fn structured_output(&self) -> bool {
        self.inner.structured_output()
    }
    fn supports_effort(&self) -> bool {
        self.inner.supports_effort()
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let sent = self.log.lock().unwrap_or_else(|e| e.into_inner()).len();
        if sent >= self.requests {
            self.cancel.cancel();
            return Err(SampleEnded(self.requests).into());
        }
        let mut req = req.clone();
        let matched = self.overlay.apply(&mut req);
        if let Some((r, _)) = self
            .overlay
            .replace
            .iter()
            .zip(&matched)
            .find(|(_, n)| **n == 0)
        {
            self.cancel.cancel();
            bail!(
                "overlay: `{}` matched nothing in {:?}; the arm would measure the baseline",
                r.find,
                r.target
            );
        }
        if let Some(m) = self.max_tokens {
            req.max_tokens = m;
        }
        let started = std::time::Instant::now();
        let response = self.inner.complete(&req, sink).await;
        let wall_secs = started.elapsed().as_secs_f64();
        let messages_digest = crate::document::sha256_hex(
            serde_json::to_string(&req.messages)
                .unwrap_or_default()
                .as_bytes(),
        );
        let exchange = Exchange {
            digest: request_digest(&req),
            wire: self.inner.wire_body(&req, sink.is_some()),
            messages_digest,
            response: response
                .as_ref()
                .map(Clone::clone)
                .map_err(|e| format!("{e:#}")),
            wall_secs,
        };
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        log.push(exchange);
        if self.cancel_after_last && log.len() >= self.requests {
            self.cancel.cancel();
        }
        response
    }
}

/// One tool call a sample made.
#[derive(Debug, Clone, Serialize)]
pub struct CallFacts {
    pub name: String,
    /// Bytes of the arguments as the model wrote them, when they did not
    /// parse; as re-serialised, when they did.
    pub argument_bytes: usize,
    pub parsed: bool,
    pub arguments: Value,
}

/// One sample: what the model did with the turn, as facts.
#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub sample: usize,
    pub seed: Option<u64>,
    pub line: usize,
    pub arm: Option<String>,
    pub overlay_digest: Option<String>,
    pub model: String,
    pub request_digest: Option<String>,
    pub messages_digest: Option<String>,
    /// The body sent, and its sha256: equal across samples but for `seed`.
    pub wire_digest: Option<String>,
    pub wire: Option<Value>,
    pub stop_reason: Option<crate::message::StopReason>,
    pub content: String,
    pub reasoning_chars: usize,
    pub tool_calls: Vec<CallFacts>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_secs: f64,
    pub requests: usize,
    pub error: Option<String>,
    /// A branch inside the run: after this call (`--at-call`).
    pub call: Option<usize>,
    /// What the sample's picture call made, when it rendered.
    pub render: Option<RenderFacts>,
}

impl Sample {
    /// The facts of the first exchange in `log`, or the error that stopped it.
    pub fn of(
        sample: usize,
        seed: Option<u64>,
        line: usize,
        model: &str,
        overlay: &Overlay,
        log: &[Exchange],
        error: Option<String>,
    ) -> Self {
        let mut s = Sample {
            sample,
            seed,
            line,
            arm: overlay.name.clone(),
            overlay_digest: overlay.digest.clone(),
            model: model.to_string(),
            request_digest: None,
            messages_digest: None,
            wire_digest: None,
            wire: None,
            stop_reason: None,
            content: String::new(),
            reasoning_chars: 0,
            tool_calls: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            wall_secs: 0.0,
            requests: log.len(),
            error,
            call: None,
            render: None,
        };
        let Some(first) = log.first() else {
            return s;
        };
        s.request_digest = Some(first.digest.clone());
        s.messages_digest = Some(first.messages_digest.clone());
        s.wire_digest = first
            .wire
            .as_ref()
            .map(|w| crate::document::sha256_hex(w.to_string().as_bytes()));
        s.wire = first.wire.clone();
        s.wall_secs = first.wall_secs;
        match &first.response {
            Err(e) => {
                s.error.get_or_insert_with(|| e.clone());
            }
            Ok(r) => {
                s.stop_reason = Some(r.stop_reason);
                s.input_tokens = r.usage.total_input();
                s.output_tokens = r.usage.output_tokens;
                for b in &r.message.content {
                    match b {
                        Block::Text { text } => s.content.push_str(text),
                        Block::Thinking { text, .. } => s.reasoning_chars += text.chars().count(),
                        Block::ToolUse { name, input, .. } => {
                            let raw = input.get("__malformed_arguments").and_then(Value::as_str);
                            s.tool_calls.push(CallFacts {
                                name: name.clone(),
                                argument_bytes: raw
                                    .map(str::len)
                                    .unwrap_or_else(|| input.to_string().len()),
                                parsed: raw.is_none(),
                                arguments: input.clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{StopReason, ToolChoice, Usage};
    use serde_json::json;

    /// A made-up calendar note: only its stem is the harness's.
    fn calendar() -> String {
        format!("{} a made-up day.", crate::date_context::REFERENCE_STEM)
    }

    fn transcript() -> String {
        use crate::session::{Record, SessionMeta};
        let meta: SessionMeta = serde_json::from_value(json!({
            "id": "t1", "created_at": "2026-10-09T00:00:00Z", "provider": "local",
            "model": "m", "workspace": "/tmp"
        }))
        .unwrap();
        let notes = |n: Vec<String>| Record::Notes { notes: n };
        let assistant = |t: &str| Record::Message(Message::assistant(vec![Block::text(t)]));
        [
            Record::Meta(meta),
            notes(vec!["(From the harness: a first note.)".into()]),
            Record::Message(Message::user("first ask")),
            assistant("first reply"),
            notes(vec![calendar()]),
            Record::GoalAnchor { goal: None },
            notes(vec![crate::persona::call::note(true, true)]),
            Record::Message(Message::user("second ask")),
            assistant("second reply"),
        ]
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn a_turn_is_read_with_its_own_notes_and_nothing_after_it() {
        let text = transcript();
        let path = Path::new("t1.jsonl");
        let turns = owner_turns(path, &text).unwrap();
        assert_eq!(turns.iter().map(|t| t.line).collect::<Vec<_>>(), [3, 8]);
        assert_eq!(turns[1].messages, 3);
        assert!(turns[1].spoken && !turns[0].spoken);

        let b = branch_at(path, &text, 8).unwrap();
        assert_eq!(b.messages.len(), 3, "the turn's own message, nothing after");
        assert_eq!(b.owner, "second ask");
        assert_eq!(
            b.notes.len(),
            1,
            "the run's notes, not the last run's calendar"
        );
        assert!(b.spoken);

        let first = branch_at(path, &text, 3).unwrap();
        assert_eq!(first.notes, ["(From the harness: a first note.)"]);
        assert_eq!(first.calendar.as_deref(), Some(calendar().as_str()));
        assert_eq!(b.calendar, None, "the second run recorded none");
    }

    #[test]
    fn the_replay_clock_renders_the_recorded_calendar() {
        let tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let then = chrono::DateTime::parse_from_rfc3339("2026-10-08T03:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let note = crate::date_context::render(then, Some(tz));
        let from = then - chrono::Duration::days(9);
        let at = clock_for(&note, from, Some(tz)).expect("a day renders it");
        assert_eq!(crate::date_context::render(at, Some(tz)), note);
        assert_eq!(clock_for("not a calendar", from, Some(tz)), None);
    }

    /// A run with two calls, then the next owner turn.
    fn run_with_calls(second: &str) -> String {
        use crate::session::{Record, SessionMeta};
        let meta: SessionMeta = serde_json::from_value(json!({
            "id": "t2", "created_at": "2026-10-09T00:00:00Z", "provider": "local",
            "model": "m", "workspace": "/tmp"
        }))
        .unwrap();
        let call = |id: &str| {
            Record::Message(Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: "widget".into(),
                input: json!({"n": 1}),
            }]))
        };
        let result = |id: &str, content: &str| {
            Record::Message(Message::tool_results(vec![Block::ToolResult {
                tool_use_id: id.into(),
                content: content.into(),
                is_error: false,
            }]))
        };
        [
            Record::Meta(meta),
            Record::Notes {
                notes: vec!["(From the harness: a note.)".into()],
            },
            Record::Message(Message::user("an ask")),
            call("c1"),
            result("c1", "first result"),
            call("c2"),
            result("c2", second),
            Record::Message(Message::assistant(vec![Block::text("a reply")])),
            Record::Message(Message::user("the next ask")),
        ]
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn a_branch_at_a_call_stops_after_its_result_with_the_turns_notes() {
        let text = run_with_calls("second result");
        let path = Path::new("t2.jsonl");
        let b = branch_at_call(path, &text, 3, 1).unwrap();
        assert_eq!(b.messages.len(), 3, "the ask, the call, its result");
        assert!(matches!(
            &b.messages[2].content[0],
            Block::ToolResult { tool_use_id, .. } if tool_use_id == "c1"
        ));
        assert_eq!(b.notes, ["(From the harness: a note.)"]);
        assert_eq!(b.call, Some(1));
        assert_eq!(branch_at_call(path, &text, 3, 2).unwrap().messages.len(), 5);
        let err = branch_at_call(path, &text, 3, 3).unwrap_err();
        assert!(
            format!("{err:#}").contains("made 2 tool call(s)"),
            "{err:#}"
        );
    }

    #[test]
    fn a_branch_after_a_deferred_picture_is_refused() {
        let text = run_with_calls(&format!("{}images/x.png", crate::imagegen::BEING_MADE));
        let err = branch_at_call(Path::new("t2.jsonl"), &text, 3, 2).unwrap_err();
        assert!(format!("{err:#}").contains("closing"), "{err:#}");
    }

    #[test]
    fn a_line_that_is_not_an_owner_turn_is_refused_by_kind() {
        let text = transcript();
        let err = branch_at(Path::new("t1.jsonl"), &text, 4).unwrap_err();
        assert!(
            format!("{err:#}").contains("`message` record, not an owner turn"),
            "{err:#}"
        );
        let err = branch_at(Path::new("t1.jsonl"), &text, 99).unwrap_err();
        assert!(format!("{err:#}").contains("has 9 record lines"), "{err:#}");
    }

    fn request() -> CompletionRequest {
        let mut last = Message::user("the ask");
        last.content.push(Block::text("(a note about pictures)"));
        CompletionRequest {
            response_schema: None,
            model: "m".into(),
            system: Some("You are a test persona.".into()),
            messages: vec![last],
            tools: vec![ToolSpec {
                name: "image_generate".into(),
                description: "Draws a test widget.".into(),
                input_schema: json!({"type": "object", "properties": {
                    "prompt": {"type": "string", "description": "the widget colour code"}}}),
            }],
            max_tokens: 100,
            effort: None,
            thinking: false,
            cache_prompt: false,
            think: None,
            think_budget: None,
            trailing_notes: 1,
            tool_choice: ToolChoice::Auto,
        }
    }

    #[test]
    fn an_overlay_edits_the_request_where_it_says_and_counts_matches() {
        let o = Overlay::parse(
            r#"
            name = "C"
            [[replace]]
            in = "tool:image_generate"
            find = "the widget colour code"
            with = "the widget colour code as hex"
            [[replace]]
            in = "notes"
            find = "pictures"
            with = "drawings"
            [[replace]]
            in = "system"
            find = "the ask"
            with = "never"
            "#,
        )
        .unwrap();
        let mut req = request();
        assert_eq!(
            o.apply(&mut req),
            [1, 1, 0],
            "the system text holds no `the ask`"
        );
        assert_eq!(
            req.tools[0].input_schema["properties"]["prompt"]["description"],
            "the widget colour code as hex"
        );
        assert_eq!(
            req.messages[0].content[1],
            Block::text("(a note about drawings)")
        );
        assert_eq!(
            req.messages[0].content[0],
            Block::text("the ask"),
            "the owner's words untouched"
        );
        assert_eq!(o.name.as_deref(), Some("C"));
        assert!(
            Overlay::parse("[[replace]]\nin = \"history\"\nfind = \"a\"\nwith = \"b\"").is_err()
        );
    }

    /// What a sample records as sent is what the provider builds, through
    /// the wrapper `provider::build` puts around it: the seed a sample is
    /// given and the stream flag a cancellable run sets are both in it.
    #[test]
    fn the_recorded_body_is_the_one_the_provider_sends() {
        let cfg = crate::config::ProviderConfig {
            kind: "local".into(),
            base_url: Some("http://127.0.0.1:9/v1".into()),
            model: Some("m".into()),
            seed: Some(7),
            ..Default::default()
        };
        let p = crate::provider::build(&cfg).unwrap();
        let body = p.wire_body(&request(), true).expect("a body");
        assert_eq!(body["seed"], 7);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 100);
    }

    struct Fixed;
    #[async_trait::async_trait]
    impl Provider for Fixed {
        fn id(&self) -> &str {
            "fixed"
        }
        fn default_model(&self) -> &str {
            "m"
        }
        async fn complete(
            &self,
            _req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            Ok(CompletionResponse {
                message: Message::assistant(vec![
                    Block::Thinking {
                        text: "plan".into(),
                        signature: None,
                    },
                    Block::ToolUse {
                        id: "c1".into(),
                        name: "image_generate".into(),
                        input: json!({"__malformed_arguments": "{\"prompt\": \"cut"}),
                    },
                ]),
                stop_reason: StopReason::MaxTokens,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                refusal: None,
                model: "m".into(),
                malformed_tool_args: 1,
            })
        }
    }

    #[tokio::test]
    async fn capture_ends_the_run_after_its_requests_and_a_dead_overlay_fails_it() {
        let cancel = CancellationToken::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let c = Capture {
            inner: Box::new(Fixed),
            overlay: Arc::new(Overlay::default()),
            max_tokens: Some(3000),
            requests: 1,
            cancel_after_last: true,
            cancel: cancel.clone(),
            log: Arc::clone(&log),
        };
        c.complete(&request(), None).await.unwrap();
        assert!(
            cancel.is_cancelled(),
            "the run stops at its next safe point"
        );
        assert!(
            c.complete(&request(), None).await.is_err(),
            "and sends nothing more"
        );
        let s = Sample::of(
            0,
            Some(7),
            3,
            "m",
            &Overlay::default(),
            &log.lock().unwrap(),
            None,
        );
        assert_eq!(s.stop_reason, Some(StopReason::MaxTokens));
        assert_eq!(s.reasoning_chars, 4);
        assert_eq!(s.tool_calls.len(), 1);
        assert!(!s.tool_calls[0].parsed);
        assert_eq!(s.tool_calls[0].argument_bytes, "{\"prompt\": \"cut".len());

        let dead = Capture {
            inner: Box::new(Fixed),
            overlay: Arc::new(
                Overlay::parse("[[replace]]\nin = \"system\"\nfind = \"absent\"\nwith = \"x\"")
                    .unwrap(),
            ),
            max_tokens: None,
            requests: 1,
            cancel_after_last: true,
            cancel: CancellationToken::new(),
            log: Arc::new(Mutex::new(Vec::new())),
        };
        let err = dead.complete(&request(), None).await.unwrap_err();
        assert!(format!("{err:#}").contains("matched nothing"), "{err:#}");
    }
}
