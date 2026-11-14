//! Explicit, synthetic-only durable reads through the ordinary guarded dispatcher.
//!
//! This module discovers no configuration, provider, session or source. The caller
//! binds a lifecycle receipt and current run authority. Only the host dispatch path
//! can construct the observation accepted by the lifecycle store.

use crate::agent::{Agent, Conversation, Dispatched, RunContext, Taint, ToolCallTrace};
use crate::config::AgentConfig;
use crate::memory::durable::{DispatchGuard, HostObservation, Phase, Receipt, Status, TaskSpec};
use crate::memory::{Access, Memory};
use crate::message::{CompletionRequest, CompletionResponse};
use crate::provider::{Provider, StreamSink};
use crate::tool::builtin::FsRead;
use crate::tool::{Capabilities, Registry, Tool, ToolCtx, ToolOutput};
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Component;
use std::sync::Arc;

/// Acquire at most one host-bound read, or reuse the recorded observation.
///
/// The reservation commits before dispatch. Dropping this future after reservation
/// leaves the allowance spent and indeterminate; resuming never retries it. A
/// cancellation after a settled read retains its actual observation, with the task
/// marked cancelled. Returned evidence remains scoped by the lifecycle store.
/// Reuse rechecks current access, the source closure and workspace binding, then
/// restores recorded taint. It never dispatches the file or its hooks again merely
/// to retrieve the saved observation, and never rewrites its external provenance.
pub async fn acquire(
    memory: &mut Memory,
    receipt: &Receipt,
    access: &Access,
    cx: &RunContext,
    conversation: &mut Conversation,
) -> Result<Status> {
    let status = memory.durable_status(receipt, access)?;
    let spec = memory.durable_spec(receipt, access)?;
    validate_binding(&spec, cx)?;
    if status.acquisition_spent {
        restore_observation_taint(
            memory.durable_observation(receipt, access)?.as_ref(),
            conversation,
        );
        return Ok(status);
    }
    ensure!(
        status.phase == Phase::Ready,
        "durable task is not ready for a read"
    );
    let permit = memory.durable_reserve_dispatch(receipt, access)?;
    let guard = memory.durable_dispatch_guard(&permit, access)?;
    let result = dispatch_read(permit.spec(), cx, conversation, Some(guard)).await?;
    memory.durable_settle(
        permit,
        HostObservation::from_dispatch(serde_json::to_value(result)?),
        access,
    )
}

fn restore_observation_taint(observation: Option<&Value>, conversation: &mut Conversation) {
    let observation = observation.and_then(|v| v.get("observation"));
    // A spent reservation with no result cannot establish what entered the run.
    // Unknown fields (including an older result schema) are conservative as well.
    conversation.taint.merge(Taint {
        private: observation
            .and_then(|v| v.get("private_taint"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        untrusted: observation
            .and_then(|v| v.get("untrusted_taint"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReadStatus {
    Successful,
    RefusedPolicy,
    RefusedUser,
    Failed,
    Indeterminate,
    CancelledBeforeDispatch,
}

// No Deserialize and no public constructor: arbitrary captured or model-written
// JSON can never become a host observation through this adapter.
#[derive(Debug, Serialize)]
struct ReadResult {
    status: ReadStatus,
    content: String,
    external: bool,
    private_taint: bool,
    untrusted_taint: bool,
    taint_before: Taint,
    cancelled: bool,
    call_id: Option<String>,
    trace: Vec<ToolCallTrace>,
}

impl ReadResult {
    fn without_dispatch(
        status: ReadStatus,
        content: String,
        cx: &RunContext,
        taint: Taint,
    ) -> Self {
        Self {
            status,
            content,
            external: false,
            private_taint: taint.private,
            untrusted_taint: taint.untrusted,
            taint_before: taint,
            cancelled: cx.cancelled(),
            call_id: None,
            trace: Vec::new(),
        }
    }
}

fn validate_binding(spec: &TaskSpec, cx: &RunContext) -> Result<()> {
    ensure!(
        (1..=256).contains(&spec.max_lines),
        "read window must be 1..=256 lines"
    );
    ensure!(
        (1024..=24_000).contains(&spec.max_bytes),
        "read byte bound must be 1024..=24000"
    );
    ensure!(
        !spec.path.as_os_str().is_empty()
            && spec
                .path
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "read path must be a relative path without traversal"
    );
    ensure!(spec.path.to_str().is_some(), "read path must be UTF-8");
    let workspace = spec
        .workspace
        .canonicalize()
        .context("resolve bound workspace")?;
    synthetic_path(&spec.path)?;
    synthetic_path(&workspace)?;
    // This names/checks the home boundary; it loads no configuration or stores.
    crate::work::ensure_outside_mecha_home(&workspace)?;
    ensure!(
        workspace == spec.workspace && workspace.is_dir(),
        "bound workspace must be a canonical directory"
    );
    ensure!(
        cx.tools.workspace.canonicalize()? == workspace,
        "current workspace differs from bound workspace"
    );
    Ok(())
}

fn bound_input(spec: &TaskSpec) -> Value {
    json!({ "path": spec.path.to_str().expect("validated path"), "offset": 1, "limit": spec.max_lines })
}

async fn dispatch_read(
    spec: &TaskSpec,
    cx: &RunContext,
    conversation: &mut Conversation,
    guard: Option<DispatchGuard>,
) -> Result<ReadResult> {
    validate_binding(spec, cx)?;
    let before = conversation.taint;
    if cx.cancelled() {
        return Ok(ReadResult::without_dispatch(
            ReadStatus::CancelledBeforeDispatch,
            "Cancelled before dispatch; the file was not read.".into(),
            cx,
            before,
        ));
    }
    if cx
        .outbox
        .as_ref()
        .is_some_and(|route| route.routes("fs_read"))
    {
        // A separately releasable staged call would escape this receipt's one
        // acquisition allowance. Refuse rather than create another action owner.
        return Ok(ReadResult::without_dispatch(
            ReadStatus::RefusedPolicy,
            "Blocked by policy: this guarded read cannot be staged in an outbox.".into(),
            cx,
            before,
        ));
    }
    // Narrow only. A recorded path cannot request a grant or reuse a spill jail.
    let mut narrowed = cx.clone();
    let tools = Arc::make_mut(&mut narrowed.tools);
    tools.grants = crate::grant::Grants::off();
    tools.spill_dir = None;
    tools.output_budget_bytes = tools.output_budget_bytes.min(spec.max_bytes);
    let input = bound_input(spec);
    if let Err(e) = strictly_jailed(&narrowed.tools, &input, &spec.workspace) {
        return Ok(ReadResult::without_dispatch(
            ReadStatus::RefusedPolicy,
            format!("Blocked by policy: {e:#}"),
            cx,
            before,
        ));
    }
    let mut registry = Registry::new();
    registry.insert(Arc::new(BoundRead {
        input: input.clone(),
        workspace: spec.workspace.clone(),
        max_bytes: spec
            .max_bytes
            .min(cx.tools.program_room.unwrap_or(usize::MAX)),
        guard,
    }));
    let agent = Agent::new(
        Box::new(NoInference),
        registry,
        narrowed.approver.clone(),
        (*narrowed.tools).clone(),
        AgentConfig::default(),
        None,
    )?;
    let result = agent
        .dispatch_one(&narrowed, conversation, "fs_read", input, &None)
        .await;
    Ok(from_dispatch(
        result,
        before,
        conversation.taint,
        cx.cancelled(),
    ))
}

fn from_dispatch(result: Dispatched, before: Taint, after: Taint, cancelled: bool) -> ReadResult {
    let status = match result.trace.as_slice() {
        [trace] if trace.name == "fs_read" && !trace.in_program => {
            if trace.staged || result.staged {
                ReadStatus::Indeterminate
            } else if trace.denied {
                if trace.blocked {
                    ReadStatus::RefusedPolicy
                } else {
                    ReadStatus::RefusedUser
                }
            } else if trace.unknown || trace.is_error || result.is_error {
                ReadStatus::Failed
            } else {
                ReadStatus::Successful
            }
        }
        _ => ReadStatus::Indeterminate,
    };
    ReadResult {
        status,
        content: result.content,
        external: result.external,
        private_taint: after.private,
        untrusted_taint: after.untrusted,
        taint_before: before,
        cancelled,
        call_id: Some(result.call_id),
        trace: result.trace,
    }
}

fn strictly_jailed(ctx: &ToolCtx, input: &Value, workspace: &std::path::Path) -> Result<()> {
    let path = ctx.resolve(input["path"].as_str().context("missing bound path")?)?;
    ensure!(
        path.starts_with(workspace),
        "bound file resolves outside its workspace"
    );
    synthetic_path(&path)?;
    if let Ok(metadata) = path.metadata() {
        ensure!(
            metadata.is_file(),
            "expected a regular synthetic fixture file"
        );
    }
    Ok(())
}

fn synthetic_path(path: &std::path::Path) -> Result<()> {
    ensure!(
        !path
            .components()
            .any(|c| c.as_os_str() == ".mecha" || c.as_os_str() == ".mecha-graph"),
        "live Mecha directories are not synthetic inputs"
    );
    Ok(())
}

/// The built-in implementation, with a host binding and a cancellation safe point
/// after hooks and approval. None of these checks widen the normal dispatcher.
struct BoundRead {
    input: Value,
    workspace: std::path::PathBuf,
    max_bytes: usize,
    guard: Option<DispatchGuard>,
}

#[async_trait]
impl Tool for BoundRead {
    fn name(&self) -> &str {
        FsRead.name()
    }
    fn description(&self) -> &str {
        FsRead.description()
    }
    fn input_schema(&self) -> Value {
        FsRead.input_schema()
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities {
        FsRead.capabilities()
    }
    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        FsRead.effect()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        if ctx.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Ok(ToolOutput::refusal(
                "Blocked by policy: cancelled before the file read.",
            ));
        }
        if input != self.input {
            return Ok(ToolOutput::refusal(
                "Blocked by policy: the call differs from the host-bound read.",
            ));
        }
        if let Err(e) = strictly_jailed(ctx, &input, &self.workspace) {
            return Ok(ToolOutput::refusal(format!("Blocked by policy: {e:#}")));
        }
        // dispatch_one intentionally clears inherited per-call program_room.
        // Apply FsRead's existing byte bound at this delegation boundary, after
        // dispatch stamping; it reserves room for its truncation notice too.
        let bounded = ToolCtx {
            program_room: Some(self.max_bytes.min(ctx.program_room.unwrap_or(usize::MAX))),
            ..ctx.clone()
        };
        if let Some(guard) = &self.guard {
            // Hooks and approval can wait while another process cancels, forgets
            // or changes an input. Recheck the original receipt at this final
            // cooperative authorization point, with no transaction over the read.
            if let Err(e) = guard.check() {
                return Ok(ToolOutput::refusal(format!(
                    "Blocked by policy: durable read authorization changed: {e:#}"
                )));
            }
        }
        FsRead.call(input, &bounded).await
    }
}

/// Agent construction requires a provider, but this path never requests a turn.
struct NoInference;

#[async_trait]
impl Provider for NoInference {
    fn id(&self) -> &str {
        "host-read-only"
    }
    fn default_model(&self) -> &str {
        "none"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        _: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        anyhow::bail!("durable guarded reads cannot invoke a model")
    }
}

#[cfg(test)]
#[path = "memory_runtime_tests.rs"]
mod tests;
