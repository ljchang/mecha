//! Trusted, bounded artifact fixtures for task-level mismatch validation.
//!
//! These are supplied by the owner before a run, never inferred from a model's
//! check command. Gold stays in the run record, outside the model's workspace.
//! Repeating a task from a pinned initial state measures artifact success; it
//! does not claim to reconstruct a mid-step filesystem or validate a forecast.
use crate::{counterfactual::ProbeVerdict, goal::GoalRef, tool::ToolCtx};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const MAX_BYTES: usize = 1_048_576;
pub const TOOLS: &[&str] = &["fs_edit", "fs_list", "fs_read", "fs_write", "todo"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCase {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub criteria: BTreeMap<String, Criterion>,
    pub prompt: String,
    pub goal: String,
    pub files: BTreeMap<String, String>,
    pub artifacts: BTreeMap<String, Value>,
    #[serde(default)]
    pub preserve: Vec<String>,
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

impl ArtifactCase {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.prompt.trim().is_empty(),
            "artifact case needs a prompt"
        );
        self.goal
            .parse::<GoalRef>()
            .context("invalid artifact goal")?;
        ensure!(
            !self.artifacts.is_empty(),
            "artifact case needs independent expected output"
        );
        ensure!(
            self.files.len() + self.artifacts.len() <= 128,
            "too many artifact paths"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_BYTES,
            "artifact case exceeds size limit"
        );
        for p in self.files.keys().chain(self.artifacts.keys()) {
            ensure!(
                relative(p),
                "artifact path must be relative without traversal: {p}"
            );
        }
        for p in &self.preserve {
            ensure!(
                self.files.contains_key(p) && !self.artifacts.contains_key(p),
                "preserved file must be an input, not an output: {p}"
            );
        }
        self.validate_criteria()?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = regular_bytes(path)?;
        let case: Self = serde_json::from_slice(&bytes)?;
        case.validate()?;
        Ok(case)
    }

    /// Reject binding a fixture to a different task or starting state.
    pub fn bind(&self, prompt: &str, goal: Option<&GoalRef>, workspace: &Path) -> Result<()> {
        self.validate()?;
        ensure!(
            self.prompt == prompt,
            "artifact case prompt differs from the run"
        );
        ensure!(
            goal.map(ToString::to_string).as_deref() == Some(self.goal.as_str()),
            "artifact case requires its confirmed goal"
        );
        let ctx = ToolCtx {
            workspace: workspace.into(),
            ..Default::default()
        };
        for (p, text) in &self.files {
            ensure!(
                regular_bytes(&ctx.resolve(p)?)? == text.as_bytes(),
                "artifact input differs from fixture: {p}"
            );
        }
        // No unrecorded input can influence the original task. Nested directories
        // are allowed, but every leaf must belong to the pinned initial state.
        fn walk(root: &Path, at: &Path, files: &BTreeMap<String, String>) -> Result<()> {
            for entry in std::fs::read_dir(at)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                ensure!(!kind.is_symlink(), "artifact fixture contains a symlink");
                if kind.is_dir() {
                    walk(root, &entry.path(), files)?;
                } else {
                    let path = entry.path();
                    let rel = path
                        .strip_prefix(root)?
                        .to_str()
                        .context("non-UTF8 fixture path")?;
                    ensure!(
                        kind.is_file() && files.contains_key(rel),
                        "unregistered artifact input: {rel}"
                    );
                }
            }
            Ok(())
        }
        walk(workspace, workspace, &self.files)
    }

    pub fn stage(&self) -> Result<Workspace> {
        self.validate()?;
        let ws = Workspace::new()?;
        let ctx = ToolCtx {
            workspace: ws.0.clone(),
            ..Default::default()
        };
        for (p, content) in &self.files {
            let to = ctx.resolve(p)?;
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(to, content)?;
        }
        Ok(ws)
    }

    /// A missing, malformed, oversized or nonregular output cannot pass. The
    /// expected object is held by the caller, never read back from the jail.
    pub fn grade(&self, workspace: &Path) -> ProbeVerdict {
        let ctx = ToolCtx {
            workspace: workspace.into(),
            ..Default::default()
        };
        let check = || -> Result<bool> {
            self.validate()?;
            for (p, expected) in &self.artifacts {
                let path = ctx.resolve(p)?;
                ensure!(
                    !std::fs::symlink_metadata(workspace.join(p))?
                        .file_type()
                        .is_symlink(),
                    "symlink output"
                );
                let actual = serde_json::from_slice::<UniqueValue>(&regular_bytes(&path)?)?.0;
                if &actual != expected {
                    return Ok(false);
                }
            }
            for p in &self.preserve {
                if regular_bytes(&ctx.resolve(p)?)? != self.files[p].as_bytes() {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        match check() {
            Ok(true) => ProbeVerdict::Pass,
            _ => ProbeVerdict::Fail,
        }
    }
}

pub struct Workspace(PathBuf);
impl Workspace {
    pub fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("mecha-mismatch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn regular_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "not a regular artifact file");
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "artifact exceeds size limit");
    Ok(bytes)
}

// Duplicate object keys make an artifact ambiguous even if a JSON parser's
// last-wins policy happens to recover the expected value. Apply at every depth.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element::<UniqueValue>()? {
                    v.push(x.0)
                }
                Ok(UniqueValue(Value::Array(v)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut v = serde_json::Map::new();
                while let Some((k, x)) = a.next_entry::<String, UniqueValue>()? {
                    if v.insert(k, x.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(UniqueValue(Value::Object(v)))
            }
        }
        d.deserialize_any(Visitor)
    }
}

/// Unknown or malformed append-only fields remain ungradeable, not a broken
/// session and not an empty (therefore passing) artifact contract.
pub fn de_lenient<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ArtifactCase>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(
        v.and_then(|v| serde_json::from_value::<ArtifactCase>(v).ok())
            .filter(|c| c.validate().is_ok()),
    )
}

pub fn validate_recording(recorded: &crate::session::RunConfig) -> Result<()> {
    use crate::harness::Lever;
    let off = recorded
        .levers_off
        .as_ref()
        .context("unrecorded lever state")?;
    for lever in [
        Lever::Hooks,
        Lever::Skills,
        Lever::Messages,
        Lever::Mcp,
        Lever::Fallback,
        Lever::StepEscalation,
        Lever::GoalGuidance,
        Lever::Outbox,
    ] {
        ensure!(
            off.contains(&lever),
            "artifact task repeat does not reproduce {lever:?}"
        );
    }
    let registry = registry(&recorded.tools)?;
    ensure!(
        recorded.tools_hash.as_deref()
            == Some(crate::surface::fingerprint(&registry.specs()).as_str()),
        "artifact tool surface changed or is unknown"
    );
    Ok(())
}

/// Only the recorded, supported surface may execute. A tool with a familiar
/// name from an MCP server cannot replace a builtin here.
pub fn registry(names: &[String]) -> Result<crate::tool::Registry> {
    use crate::tool::{builtin::*, todo::TodoTool, Registry};
    use std::sync::Arc;
    ensure!(
        !names.is_empty() && names.iter().all(|n| TOOLS.contains(&n.as_str())),
        "unsupported artifact probe tool surface"
    );
    let mut registry = Registry::new();
    for n in names {
        let tool: Arc<dyn crate::tool::Tool> = match n.as_str() {
            "fs_read" => Arc::new(FsRead),
            "fs_write" => Arc::new(FsWrite),
            "fs_edit" => Arc::new(FsEdit),
            "fs_list" => Arc::new(FsList),
            "todo" => Arc::new(TodoTool::new()),
            _ => unreachable!(),
        };
        registry.insert(tool);
    }
    Ok(registry)
}

/// Repeat the registered task with real file tools and independent gold. The
/// caller retains the normal approver, hooks and command policy; a refused
/// action is ungraded, never evidence that a rule harmed the task.
pub async fn drive(
    case: &ArtifactCase,
    recorded: &crate::session::RunConfig,
    provider: Box<dyn crate::provider::Provider>,
    cfg: crate::config::AgentConfig,
    model: &str,
    context: &crate::agent::RunContext,
) -> Result<(ProbeVerdict, crate::session::RunStats)> {
    use crate::agent::{Agent, Conversation, StopCause};
    use std::sync::Arc;
    let ws = case.stage()?;
    let mut cx = context.sandboxed(&ws.0, Arc::clone(&context.approver));
    cx.homeostat = None;
    cx.outbox = None;
    cx.mailbox = None;
    cx.queued_input = None;
    let tools = registry(&recorded.tools)?;
    let agent = Agent::new(
        provider,
        tools,
        Arc::clone(&cx.approver),
        (*cx.tools).clone(),
        cfg,
        Some(model.into()),
    )?;
    let mut convo = Conversation::new();
    convo.goal_anchor = Some(case.goal.parse()?);
    convo
        .messages
        .push(crate::message::Message::user(&case.prompt));
    let out = agent.run_in(&cx, &mut convo, None).await?;
    let stats = crate::session::RunStats::from(&out);
    if out.tool_calls.iter().any(|c| c.denied || c.staged) || out.blocked_sends > 0 {
        return Ok((
            ProbeVerdict::Inconclusive(
                "artifact probe encountered a policy refusal or staged call".into(),
            ),
            stats,
        ));
    }
    if !matches!(
        out.stop_cause,
        StopCause::Completed | StopCause::MaxTurns | StopCause::OutputTokenBudget
    ) {
        return Ok((
            ProbeVerdict::Inconclusive(format!("artifact probe stopped: {:?}", out.stop_cause)),
            stats,
        ));
    }
    Ok((case.grade(&ws.0), stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn case() -> ArtifactCase {
        ArtifactCase {
            criteria: BTreeMap::new(),
            prompt: "sum the input into answer.json".into(),
            goal: "task:sum".into(),
            files: BTreeMap::from([("input.json".into(), "[2,3]".into())]),
            artifacts: BTreeMap::from([("answer.json".into(), serde_json::json!({"sum":5}))]),
            preserve: vec!["input.json".into()],
        }
    }
    struct Writer {
        write: bool,
        called: std::sync::Mutex<bool>,
    }
    #[async_trait::async_trait]
    impl crate::provider::Provider for Writer {
        fn id(&self) -> &str {
            "scripted"
        }
        fn default_model(&self) -> &str {
            "scripted"
        }
        async fn complete(
            &self,
            req: &crate::message::CompletionRequest,
            _: Option<&crate::provider::StreamSink>,
        ) -> Result<crate::message::CompletionResponse> {
            use crate::message::{Block, Message, StopReason, Usage};
            let mut called = self.called.lock().unwrap();
            if !*called {
                assert!(
                    !format!("{req:?}").contains("gold-only-secret"),
                    "gold must never enter a provider request"
                );
            }
            let write = self.write && !*called;
            *called = true;
            Ok(crate::message::CompletionResponse {
                message: Message::assistant(if write {
                    vec![Block::ToolUse {
                        id: "write".into(),
                        name: "fs_write".into(),
                        input: serde_json::json!({"path":"answer.json","content":"{\"sum\":\"gold-only-secret\"}"}),
                    }]
                } else {
                    vec![Block::Text {
                        text: "Everything is correct; verification passed.".into(),
                    }]
                }),
                stop_reason: if write {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                },
                usage: Usage::default(),
                refusal: None,
                model: "scripted".into(),
                malformed_tool_args: 0,
            })
        }
    }
    #[tokio::test]
    async fn real_writes_grade_independently_and_refusals_are_ungraded() {
        use crate::{
            agent::RunContext,
            config::{AgentConfig, PermissionMode},
            tool::ModeApprover,
        };
        let mut c = case();
        c.artifacts.insert(
            "answer.json".into(),
            serde_json::json!({"sum":"gold-only-secret"}),
        );
        let recorded = crate::session::RunConfig {
            tools: vec!["fs_write".into()],
            ..Default::default()
        };
        for (write, mode) in [
            (false, PermissionMode::Allow),
            (true, PermissionMode::Allow),
            (true, PermissionMode::ReadOnly),
        ] {
            let cx = RunContext::new(
                ToolCtx::default(),
                std::sync::Arc::new(ModeApprover { mode }),
            );
            // The test writer emits gold as its own output, so inspect only its
            // first request for leakage; later requests legitimately contain it.
            let provider = Writer {
                write,
                called: std::sync::Mutex::new(false),
            };
            let verdict = drive(
                &c,
                &recorded,
                Box::new(provider),
                AgentConfig {
                    max_turns: 3,
                    ..Default::default()
                },
                "scripted",
                &cx,
            )
            .await
            .unwrap()
            .0;
            match (write, mode) {
                (false, _) => assert_eq!(verdict, ProbeVerdict::Fail),
                (true, PermissionMode::Allow) => assert_eq!(verdict, ProbeVerdict::Pass),
                _ => assert!(matches!(verdict, ProbeVerdict::Inconclusive(_))),
            }
        }
    }
    #[test]
    fn corrupt_append_only_contract_remains_ungradeable() {
        let mut value = serde_json::to_value(crate::session::RunConfig::default()).unwrap();
        value["mismatch_case"] = serde_json::json!({"future":"unsupported"});
        let recorded: crate::session::RunConfig = serde_json::from_value(value).unwrap();
        assert!(recorded.mismatch_case.is_none());
    }
    #[test]
    fn independent_gold_and_fresh_arms_reject_wrong_or_mutated_artifacts() {
        let c = case();
        let a = c.stage().unwrap();
        let b = c.stage().unwrap();
        c.bind(&c.prompt, Some(&c.goal.parse().unwrap()), &a.0)
            .unwrap();
        assert_eq!(c.grade(&a.0), ProbeVerdict::Fail);
        std::fs::write(a.0.join("answer.json"), r#"{"sum":5}"#).unwrap();
        assert_eq!(c.grade(&a.0), ProbeVerdict::Pass);
        std::fs::write(a.0.join("answer.json"), r#"{"sum":0,"sum":5}"#).unwrap();
        assert_eq!(
            c.grade(&a.0),
            ProbeVerdict::Fail,
            "duplicate keys cannot satisfy the oracle"
        );
        assert_eq!(
            c.grade(&b.0),
            ProbeVerdict::Fail,
            "arms cannot share outputs"
        );
        std::fs::write(a.0.join("answer.json"), r#"{"sum":true}"#).unwrap();
        assert_eq!(c.grade(&a.0), ProbeVerdict::Fail);
        std::fs::write(a.0.join("answer.json"), r#"{"sum":5,"extra":1}"#).unwrap();
        assert_eq!(c.grade(&a.0), ProbeVerdict::Fail);
        std::fs::write(a.0.join("answer.json"), r#"{"sum":5}"#).unwrap();
        std::fs::write(a.0.join("input.json"), "[]").unwrap();
        assert_eq!(
            c.grade(&a.0),
            ProbeVerdict::Fail,
            "rewriting the problem cannot pass"
        );
        assert!(c
            .bind(&c.prompt, Some(&c.goal.parse().unwrap()), &a.0)
            .is_err());
    }
    #[test]
    fn contracts_reject_traversal_empty_gold_and_wrong_bindings() {
        let mut c = case();
        let ws = c.stage().unwrap();
        assert!(c
            .bind("another task", Some(&c.goal.parse().unwrap()), &ws.0)
            .is_err());
        assert!(c.bind(&c.prompt, None, &ws.0).is_err());
        std::fs::write(ws.0.join("unregistered"), "secret").unwrap();
        assert!(c
            .bind(&c.prompt, Some(&c.goal.parse().unwrap()), &ws.0)
            .is_err());
        c.artifacts.insert("../escape".into(), Value::Null);
        assert!(c.validate().is_err());
        c.artifacts.clear();
        assert!(c.validate().is_err());
    }
    #[cfg(unix)]
    #[test]
    fn nonregular_and_oversized_outputs_fail_without_blocking() {
        use std::os::unix::fs::symlink;
        let c = case();
        let ws = c.stage().unwrap();
        let answer = ws.0.join("answer.json");
        symlink("input.json", &answer).unwrap();
        assert_eq!(c.grade(&ws.0), ProbeVerdict::Fail);
        std::fs::remove_file(&answer).unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(&answer)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(c.grade(&ws.0), ProbeVerdict::Fail);
        std::fs::remove_file(&answer).unwrap();
        std::fs::write(&answer, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert_eq!(c.grade(&ws.0), ProbeVerdict::Fail);
    }
}

/// Optional, owner-registered training diagnostics. Empty on held-out tasks.
/// Gold stays in `artifacts`; feedback contains no expected answer value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub artifact: String,
    pub pointer: String,
    #[serde(default)]
    pub context: Option<CountConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountConstraint {
    pub source: String,
    pub observed_pointer: String,
    pub limit_pointer: String,
    pub relation: CountRelation,
    /// Owner-supplied association, not a resolved reading of the live charter.
    #[serde(default, deserialize_with = "de_charter_goal")]
    pub charter_goal: Option<GoalRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountRelation {
    GreaterThan,
    AtLeast,
    LessThan,
    AtMost,
}
impl CountRelation {
    fn holds(self, value: u64, limit: u64) -> bool {
        match self {
            Self::GreaterThan => value > limit,
            Self::AtLeast => value >= limit,
            Self::LessThan => value < limit,
            Self::AtMost => value <= limit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountEvidence {
    pub constraint: CountConstraint,
    pub observed: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriterionFeedback {
    pub id: String,
    pub artifact: String,
    pub pointer: String,
    pub context: Option<CountEvidence>,
}

impl ArtifactCase {
    fn criterion_context(&self, criterion: &Criterion) -> Result<Option<CountEvidence>> {
        criterion
            .context
            .as_ref()
            .map(|c| {
                let input = self
                    .files
                    .get(&c.source)
                    .context("criterion context is not a pinned input")?;
                let value = serde_json::from_str::<UniqueValue>(input)?.0;
                Ok(CountEvidence {
                    constraint: c.clone(),
                    observed: value.pointer(&c.observed_pointer).and_then(Value::as_u64),
                    limit: value.pointer(&c.limit_pointer).and_then(Value::as_u64),
                })
            })
            .transpose()
    }

    fn validate_criteria(&self) -> Result<()> {
        ensure!(self.criteria.len() <= 16, "too many diagnostic criteria");
        let pointer = |p: &str| p.len() <= 256 && (p.is_empty() || p.starts_with('/'));
        for (id, c) in &self.criteria {
            ensure!(
                !id.is_empty()
                    && id.len() <= 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "invalid criterion id"
            );
            ensure!(pointer(&c.pointer), "invalid criterion pointer");
            let expected = self
                .artifacts
                .get(&c.artifact)
                .and_then(|v| v.pointer(&c.pointer))
                .context("criterion does not name an expected artifact field")?;
            if let Some(ctx) = &c.context {
                ensure!(
                    self.preserve.contains(&ctx.source),
                    "criterion context must be a preserved input"
                );
                ensure!(
                    pointer(&ctx.observed_pointer) && pointer(&ctx.limit_pointer),
                    "invalid context pointer"
                );
                ensure!(
                    ctx.charter_goal
                        .as_ref()
                        .is_none_or(|g| matches!(g, GoalRef::Charter(_))),
                    "constraint association must name a charter goal"
                );
                let evidence = self
                    .criterion_context(c)?
                    .context("missing criterion context")?;
                if let (Some(value), Some(limit)) = (evidence.observed, evidence.limit) {
                    ensure!(
                        expected.as_bool() == Some(ctx.relation.holds(value, limit)),
                        "count constraint contradicts the registered artifact gold"
                    );
                }
            }
        }
        Ok(())
    }

    /// Called after a task ends, never as a model tool. Only bounded owner
    /// criteria are emitted; actual output text and gold never enter metadata.
    pub fn criterion_feedback(
        &self,
        workspace: &Path,
    ) -> Result<Vec<crate::planning::StepFeedback>> {
        use crate::planning::{StepFeedback, Verification};
        self.validate()?;
        let ctx = ToolCtx {
            workspace: workspace.into(),
            ..Default::default()
        };
        let mut out = Vec::new();
        for (id, c) in &self.criteria {
            let context = self.criterion_context(c)?;
            let source_unchanged = c.context.as_ref().is_none_or(|source| {
                ctx.resolve(&source.source)
                    .and_then(|p| regular_bytes(&p))
                    .is_ok_and(|v| v == self.files[&source.source].as_bytes())
            });
            let known = context
                .as_ref()
                .is_none_or(|e| e.observed.is_some() && e.limit.is_some());
            let actual = ctx.resolve(&c.artifact).and_then(|p| {
                ensure!(
                    !std::fs::symlink_metadata(workspace.join(&c.artifact))?
                        .file_type()
                        .is_symlink(),
                    "symlink output"
                );
                Ok(serde_json::from_slice::<UniqueValue>(&regular_bytes(&p)?)?.0)
            });
            let verification = if !source_unchanged || !known {
                Verification::Skipped
            } else if actual.as_ref().ok().and_then(|v| v.pointer(&c.pointer))
                == self.artifacts[&c.artifact].pointer(&c.pointer)
            {
                Verification::Passed
            } else {
                Verification::Failed
            };
            out.push(StepFeedback {
                criterion: Some(CriterionFeedback {
                    id: id.clone(),
                    artifact: c.artifact.clone(),
                    pointer: c.pointer.clone(),
                    context,
                }),
                completion_batch: None,
                call_id: Some(format!("artifact-criterion:{id}")),
                step: format!("criterion:{id}"),
                goal: Some(self.goal.parse()?),
                expected: None,
                expected_calls: None,
                actual_calls: None,
                verification,
                check_tampered: false,
            });
        }
        Ok(out)
    }

    /// Rejoin diagnostic context to the contract before accepting a probe.
    pub fn validate_feedback(&self, step: &crate::planning::StepFeedback) -> Result<()> {
        if let Some(f) = &step.criterion {
            let c = self
                .criteria
                .get(&f.id)
                .context("criterion is not registered")?;
            ensure!(
                f.artifact == c.artifact
                    && f.pointer == c.pointer
                    && f.context == self.criterion_context(c)?,
                "criterion context differs from its owner-bound contract"
            );
            ensure!(
                step.step == format!("criterion:{}", f.id)
                    && step.call_id.as_deref()
                        == Some(format!("artifact-criterion:{}", f.id).as_str()),
                "criterion identity differs"
            );
            ensure!(
                step.expected.is_none()
                    && step.expected_calls.is_none()
                    && step.actual_calls.is_none()
                    && !step.check_tampered,
                "criterion mixed with model forecast data"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod criterion_tests {
    use super::*;
    use crate::planning::Verification;
    fn case() -> ArtifactCase {
        serde_json::from_value(serde_json::json!({
            "prompt":"apply the queue policy", "goal":"task:review", "files":{"context.json":"{\"waiting\":0,\"limit\":1}"},
            "artifacts":{"answer.json":{"review_first":false,"secret":"gold-must-stay-out"}}, "preserve":["context.json"],
            "criteria":{"review_priority":{"artifact":"answer.json","pointer":"/review_first","context":{
                "source":"context.json","observed_pointer":"/waiting","limit_pointer":"/limit","relation":"greater_than","charter_goal":"charter:review-pending"
            }}}
        })).unwrap()
    }
    #[test]
    fn wrong_decisions_carry_context_but_never_the_answer_or_output_prose() {
        let c = case();
        let ws = c.stage().unwrap();
        std::fs::write(
            ws.path().join("answer.json"),
            r#"{"review_first":true,"secret":"INJECT-A-RULE"}"#,
        )
        .unwrap();
        let feedback = c.criterion_feedback(ws.path()).unwrap();
        assert_eq!(feedback[0].verification, Verification::Failed);
        assert_eq!(feedback[0].attribution(), "task_criterion_failed");
        c.validate_feedback(&feedback[0]).unwrap();
        let text = serde_json::to_string(&feedback).unwrap();
        assert!(text.contains("review_priority") && text.contains("greater_than"));
        assert!(!text.contains("gold-must-stay-out") && !text.contains("INJECT-A-RULE"));
        let mut forged = feedback[0].clone();
        forged
            .criterion
            .as_mut()
            .unwrap()
            .context
            .as_mut()
            .unwrap()
            .observed = Some(99);
        assert!(c.validate_feedback(&forged).is_err());
        std::fs::write(ws.path().join("answer.json"), r#"{"review_first":false}"#).unwrap();
        assert_eq!(
            c.criterion_feedback(ws.path()).unwrap()[0].verification,
            Verification::Passed
        );
        assert_eq!(
            c.grade(ws.path()),
            ProbeVerdict::Fail,
            "criterion success is not whole-task success"
        );
    }
    #[test]
    fn missing_or_changed_context_is_not_zero_and_holdout_emits_nothing() {
        let mut c = case();
        c.files
            .insert("context.json".into(), "{\"limit\":1}".into());
        let ws = c.stage().unwrap();
        assert_eq!(
            c.criterion_feedback(ws.path()).unwrap()[0].verification,
            Verification::Skipped
        );
        let c = case();
        let ws = c.stage().unwrap();
        std::fs::write(
            ws.path().join("context.json"),
            "{\"waiting\":50,\"limit\":1}",
        )
        .unwrap();
        assert_eq!(
            c.criterion_feedback(ws.path()).unwrap()[0].verification,
            Verification::Skipped
        );
        let mut held = c.clone();
        held.criteria.clear();
        assert!(held.criterion_feedback(ws.path()).unwrap().is_empty());
    }
    #[test]
    fn recorded_queue_failures_are_identified_by_the_correct_sensor() {
        let fixtures: Vec<Value> = serde_json::from_str(include_str!(
            "../../eval/fixtures/appraisal-mismatch/cases.json"
        ))
        .unwrap();
        for id in ["revision-2", "revision-6"] {
            let mut fixture = fixtures.iter().find(|c| c["id"] == id).unwrap().clone();
            fixture.as_object_mut().unwrap().remove("id");
            let mut c: ArtifactCase = serde_json::from_value(fixture).unwrap();
            c.criteria.insert(
                "review_priority".into(),
                Criterion {
                    artifact: "answer.json".into(),
                    pointer: "/review_first".into(),
                    context: Some(CountConstraint {
                        source: "context.json".into(),
                        observed_pointer: "/outbox_waiting".into(),
                        limit_pointer: "/review_threshold".into(),
                        relation: CountRelation::GreaterThan,
                        charter_goal: Some(GoalRef::Charter("review-pending".into())),
                    }),
                },
            );
            let ws = c.stage().unwrap();
            let actual = if id == "revision-2" {
                include_str!("../../results/appraisal-mismatch-qwen36-35b-20260909/artifacts/control__revision-2__s1__r1/answer.json")
            } else {
                include_str!("../../results/appraisal-mismatch-qwen36-35b-20260909/artifacts/learning__revision-6__s1__r1/answer.json")
            };
            std::fs::write(ws.path().join("answer.json"), actual).unwrap();
            let f = c.criterion_feedback(ws.path()).unwrap().remove(0);
            assert_eq!(f.verification, Verification::Failed);
            let e = f.criterion.as_ref().unwrap().context.as_ref().unwrap();
            assert_eq!((e.observed, e.limit), (Some(0), Some(1)));
            assert_eq!(
                f.goals(),
                vec![
                    c.goal.parse().unwrap(),
                    GoalRef::Charter("review-pending".into())
                ]
            );
        }
    }
    #[test]
    fn contradictory_contracts_and_duplicate_key_outputs_do_not_pass() {
        let mut c = case();
        c.artifacts.get_mut("answer.json").unwrap()["review_first"] = Value::Bool(true);
        assert!(c.validate().is_err());
        let c = case();
        let ws = c.stage().unwrap();
        std::fs::write(
            ws.path().join("answer.json"),
            r#"{"review_first":true,"review_first":false}"#,
        )
        .unwrap();
        assert_eq!(
            c.criterion_feedback(ws.path()).unwrap()[0].verification,
            Verification::Failed
        );
    }
}

fn de_charter_goal<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<GoalRef>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|s| s.parse().map_err(serde::de::Error::custom))
        .transpose()
}
