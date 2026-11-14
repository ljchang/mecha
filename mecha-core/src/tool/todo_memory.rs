//! Staged durable adapter over the same parser, tracker and renderer as volatile todo.
use super::*;
use crate::{
    memory::{plan::*, RevisionRef},
    memory_session::chat::task_plan::{LivePlanTurn, PlanBinding},
    planning::{Feedback, GoalDecisionStamp},
};
use anyhow::{ensure, Context};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

#[derive(Debug)]
pub struct PlanCall {
    live: Arc<LivePlanTurn>,
    occurrence: PlanOccurrence,
    batch: u32,
    goal: GoalDecisionStamp,
    anchor: Option<GoalRef>,
    cap: usize,
    contribution: Mutex<Option<(crate::message::PlanResultAudit, Feedback)>>,
}
impl PlanCall {
    pub(crate) fn new(
        live: Arc<LivePlanTurn>,
        ordinal: u32,
        batch: u32,
        goal: GoalDecisionStamp,
        anchor: Option<GoalRef>,
        cap: usize,
    ) -> Self {
        Self {
            occurrence: PlanOccurrence {
                turn_id: live.turn_id().into(),
                ordinal,
            },
            live,
            batch,
            goal,
            anchor,
            cap,
            contribution: Mutex::new(None),
        }
    }
    pub(crate) fn take(&self) -> Option<(crate::message::PlanResultAudit, Feedback)> {
        self.contribution
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}
struct State {
    tracked: Tracked,
    head: Option<RevisionRef>,
    batch: Option<(String, u32)>,
    attempts: BTreeMap<(String, u32), Attempt>,
    blocked: bool,
    observed: BTreeMap<(String, u32), String>,
}
struct Attempt {
    input: String,
    expected: Option<RevisionRef>,
    request: PlanCommitRequest,
}
pub(crate) struct TodoMemory {
    binding: Arc<PlanBinding>,
    state: Mutex<State>,
}
struct Staged {
    tracked: Tracked,
    counts: (u32, u32, u32, u32),
    goal_delta: (u32, u32, u32),
    removals: HashSet<String>,
    feedback: Feedback,
    rendered: String,
}
impl TodoTool {
    pub(crate) fn with_memory(binding: Arc<PlanBinding>) -> Result<Self> {
        let view = binding.read()?;
        let tracked = view
            .as_ref()
            .map(|v| restore(&v.value.request))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            lists: Mutex::default(),
            memory: Some(TodoMemory {
                binding,
                state: Mutex::new(State {
                    tracked,
                    head: view.map(|v| v.reference),
                    batch: None,
                    attempts: BTreeMap::new(),
                    blocked: false,
                    observed: BTreeMap::new(),
                }),
            }),
        })
    }
}
fn snapshot(tracked: &Tracked) -> PlanSnapshot {
    PlanSnapshot {
        serves: tracked.plan.goal.as_ref().map(ToString::to_string),
        items: tracked
            .plan
            .items
            .iter()
            .map(|i| PlanItem {
                content: i.content.clone(),
                status: match i.status {
                    Status::Pending => PlanItemStatus::Pending,
                    Status::InProgress => PlanItemStatus::InProgress,
                    Status::Completed => PlanItemStatus::Completed,
                },
                expect: i.expect.clone(),
                check: i.check.clone(),
                expect_calls: i.expect_calls,
            })
            .collect(),
        declarations: tracked
            .checks
            .iter()
            .map(|(k, f)| {
                (
                    k.clone(),
                    PlanCheckDeclaration {
                        check: f.check.clone(),
                        frozen: f.frozen,
                    },
                )
            })
            .collect(),
    }
}
pub(crate) fn validate_durable_request(request: &PlanCommitRequest) -> Result<()> {
    restore(request).map(|_| ())
}
fn restore(request: &PlanCommitRequest) -> Result<Tracked> {
    request.snapshot.validate()?;
    request.feedback.validate()?;
    crate::planning::durable_feedback(request.feedback.text(), &request.provider_call_id)?;
    let raw = json!({"items":request.snapshot.items,"serves":request.snapshot.serves});
    let plan = TodoTool::parse_plan(&raw).map_err(|e| anyhow::anyhow!("stored plan: {e}"))?;
    let mut tracked = Tracked {
        plan,
        ..Default::default()
    };
    for (content, declaration) in &request.snapshot.declarations {
        let declaration_plan = TodoTool::parse_plan(
            &json!({"items":[{"content": content,"status":"pending","check":declaration.check}]}),
        )
        .map_err(|e| anyhow::anyhow!("stored declaration: {e}"))?;
        ensure!(
            declaration_plan.items[0].content == *content
                && declaration_plan.items[0].check.as_deref() == Some(&declaration.check),
            "noncanonical stored declaration"
        );
        tracked.checks.insert(
            content.clone(),
            Freeze {
                hash: check_hash(&declaration.check),
                check: declaration.check.clone(),
                frozen: declaration.frozen,
            },
        );
    }
    ensure!(
        snapshot(&tracked) == request.snapshot,
        "stored plan requires repair"
    );
    Ok(tracked)
}
impl Staged {
    fn build(
        mut tracked: Tracked,
        plan: Plan,
        ctx: &ToolCtx,
        call: &PlanCall,
        own_before: u32,
    ) -> Result<Self> {
        let previous: HashSet<_> = tracked
            .plan
            .items
            .iter()
            .filter(|i| i.status == Status::Completed)
            .map(|i| i.content.clone())
            .collect();
        let counts = crate::tool::StepCounts::default();
        let goal_delta =
            crate::tool::GoalTrack::plan_delta(call.anchor.as_ref(), plan.goal.as_ref());
        let last_real = tracked.last_real;
        let mut findings =
            tracked.advance(plan, ctx.work, own_before, last_real, None, Some(&counts));
        let mut verified = ctx
            .verified_steps
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let (feedback, removals, guidance) =
            TodoTool::finish_feedback(&mut tracked, ctx, call.anchor.clone(), &mut verified, false);
        if tracked.plan.items.iter().any(|i| {
            i.status == Status::Completed && i.check.is_some() && !previous.contains(&i.content)
        }) {
            findings
                .push("A declared plan check was not run; completion remains unverified.".into());
        }
        findings.extend(guidance);
        let rendered = TodoTool::result_text(&tracked.plan, &findings, ctx.context.as_ref());
        ensure!(
            rendered.len() <= call.cap && rendered.len() <= 65536,
            "plan echo exceeds dispatch cap"
        );
        snapshot(&tracked).validate()?;
        crate::planning::durable_feedback(
            &serde_json::to_string(&feedback)?,
            ctx.call_id.as_deref().context("unstamped plan call")?,
        )?;
        Ok(Self {
            tracked,
            counts: counts.snapshot(),
            goal_delta,
            removals,
            feedback,
            rendered,
        })
    }
    fn publish(self, state: &mut State, ctx: &ToolCtx) {
        use std::sync::atomic::Ordering::Relaxed;
        state.tracked = self.tracked;
        if let Some(counts) = &ctx.step_counts {
            counts.nulls.fetch_add(self.counts.0, Relaxed);
            counts.reopens.fetch_add(self.counts.1, Relaxed);
            counts.completions.fetch_add(self.counts.2, Relaxed);
            counts.measured.fetch_add(self.counts.3, Relaxed);
        }
        if let Some(track) = &ctx.goal_track {
            track.apply_plan_delta(self.goal_delta);
        }
        ctx.verified_steps
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|k| !self.removals.contains(k));
    }
}
impl TodoMemory {
    pub(super) fn plan(&self) -> Plan {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tracked
            .plan
            .clone()
    }
    pub(super) fn tampered(&self) -> u32 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tracked
            .tampered
    }
    pub(super) fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        match self.try_call(input, ctx) {
            Ok(out) => Ok(out),
            Err(e) => Ok(ToolOutput::err(format!("durable plan unavailable: {e:#}"))),
        }
    }
    fn try_call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let call = ctx
            .plan_call
            .as_ref()
            .context("missing live plan occurrence")?;
        ensure!(call.live.matches(&self.binding), "wrong plan capability");
        ensure!(
            ctx.goal_decision
                .lock()
                .map_err(|_| anyhow::anyhow!("goal stamp poisoned"))?
                .as_ref()
                == Some(&call.goal),
            "goal decision changed"
        );
        let id = ctx.call_id.as_deref().context("missing provider call id")?;
        let input_json = serde_json::to_string(&input)?;
        let input_digest: String = Sha256::digest(input_json.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let key = (call.occurrence.turn_id.clone(), call.occurrence.ordinal);
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("plan controller poisoned"))?;
        if let Some(attempt) = state.attempts.get(&key) {
            ensure!(
                attempt.input == input_json && attempt.request.provider_call_id == id,
                "conflicting plan retry"
            );
            let result = call
                .live
                .commit(attempt.expected.as_ref(), attempt.request.clone())?;
            ensure!(result.replayed, "plan retry unexpectedly appended");
            crate::planning::durable_feedback(result.view.value.request.feedback.text(), id)?;
            return Ok(ToolOutput::ok(result.view.value.request.result_text));
        }
        ensure!(
            !state.blocked,
            "prior plan commit requires host reconciliation"
        );
        if let Some(observed) = state.observed.get(&key) {
            ensure!(observed == &input_digest, "conflicting rejected plan retry");
            anyhow::bail!("this plan occurrence was already rejected");
        }
        state.observed.retain(|(turn, _), _| turn == &key.0);
        state.observed.insert(key.clone(), input_digest);
        let mut candidate = state.tracked.clone();
        let batch = (call.occurrence.turn_id.clone(), call.batch);
        let own_before = if state.batch.as_ref() == Some(&batch) {
            let before = candidate.own_calls;
            candidate.own_calls += 1;
            before
        } else {
            candidate.observe(ctx.work)
        };
        // Rejected invocations are bookkeeping, but never progress.
        state.tracked.own_calls = candidate.own_calls;
        state.tracked.last_real = candidate.last_real;
        state.tracked.next_own_position = candidate.next_own_position;
        state.batch = Some(batch);
        ensure!(input_json.len() <= 65536, "oversized plan input");
        ensure!(
            input
                .get("items")
                .and_then(Value::as_array)
                .is_none_or(|items| items.len() <= 32),
            "too many plan items"
        );

        let plan = TodoTool::parse_plan(&input).map_err(|e| anyhow::anyhow!("{e}"))?;
        let staged = Staged::build(candidate, plan, ctx, call, own_before)?;
        let request = PlanCommitRequest {
            occurrence: call.occurrence.clone(),
            snapshot: snapshot(&staged.tracked),
            goal_decision_digest: call.goal.decision_digest.clone(),
            goal_version_digest: call.goal.version_digest.clone(),
            goal_version: call.goal.version,
            provider_call_id: id.into(),
            input_json: input_json.clone(),
            result_text: staged.rendered.clone(),
            feedback: StoredPlanFeedback::from_host_text(serde_json::to_string(&staged.feedback)?)?,
        };
        let expected = state.head.clone();
        let result = match call.live.commit(expected.as_ref(), request.clone()) {
            Ok(result) => result,
            Err(e) => {
                state.blocked = true;
                return Err(e);
            }
        };
        // Prepare all fallible validation before publication. Storage returns the exact request.
        let feedback =
            crate::planning::durable_feedback(result.view.value.request.feedback.text(), id)?;
        ensure!(
            result.view.value.request == request,
            "committed plan differs from staged request"
        );
        let out = ToolOutput::ok(staged.rendered.clone());
        if result.replayed {
            return Ok(out);
        }
        let audit = crate::message::PlanResultAudit {
            turn_id: call.occurrence.turn_id.clone(),
            ordinal: call.occurrence.ordinal,
            result_index: 0,
            revision_id: result.view.reference.revision.clone(),
            receipt_id: result.view.receipt_id,
            steps_start: 0,
            steps_end: feedback.steps.len(),
            decisions_start: 0,
            decisions_end: feedback.decisions.len(),
        };
        state.head = Some(result.view.reference);
        state.attempts.retain(|(turn, _), _| turn == &key.0);
        state.attempts.insert(
            key,
            Attempt {
                input: input_json,
                expected,
                request,
            },
        );
        staged.publish(&mut state, ctx);
        *call.contribution.lock().unwrap_or_else(|e| e.into_inner()) = Some((audit, feedback));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> PlanCommitRequest {
        let plan = Plan {
            goal: Some("task:fixture".parse().unwrap()),
            items: vec![TodoItem {
                content: "Inspect fixture".into(),
                status: Status::InProgress,
                check: Some("inspect fixture".into()),
                ..Default::default()
            }],
        };
        let feedback = Feedback {
            steps: vec![],
            decisions: vec![crate::planning::Decision::assess(
                &plan,
                None,
                None,
                None,
                &HashSet::new(),
                false,
                None,
            )],
        };
        let tracked = Tracked {
            plan,
            checks: [(
                "Inspect fixture".into(),
                Freeze {
                    check: "inspect fixture".into(),
                    hash: check_hash("inspect fixture"),
                    frozen: false,
                },
            )]
            .into(),
            ..Default::default()
        };
        PlanCommitRequest {
            occurrence: PlanOccurrence {
                turn_id: "fixture-turn".into(),
                ordinal: 0,
            },
            snapshot: snapshot(&tracked),
            goal_decision_digest: "a".repeat(64),
            goal_version_digest: "b".repeat(64),
            goal_version: 1,
            provider_call_id: "fixture-call".into(),
            input_json: "{}".into(),
            result_text: TodoTool::render(&tracked.plan),
            feedback: StoredPlanFeedback::from_host_text(serde_json::to_string(&feedback).unwrap())
                .unwrap(),
        }
    }
    #[test]
    fn durable_todo_restore_is_strict_and_rebuilds_absent_open_declarations() {
        let mut r = request();
        r.snapshot.items.clear();
        let tracked = restore(&r).unwrap();
        assert!(tracked.plan.items.is_empty());
        assert_eq!(
            tracked.checks["Inspect fixture"].hash,
            check_hash("inspect fixture")
        );
        assert!(!tracked.checks["Inspect fixture"].frozen);
        assert!(
            tracked.started.is_empty()
                && tracked.completed.is_empty()
                && tracked.ever_completed.is_empty()
        );
        assert_eq!(tracked.own_calls, 0);
        for corrupt in [" inspect fixture ", "inspect\nfixture"] {
            let mut r = r.clone();
            r.snapshot
                .declarations
                .get_mut("Inspect fixture")
                .unwrap()
                .check = corrupt.into();
            assert!(restore(&r).is_err());
        }
        for serves in ["request:fixture", "bogus:fixture"] {
            let mut r = r.clone();
            r.snapshot.serves = Some(serves.into());
            assert!(restore(&r).is_err());
        }
        let mut r = request();
        r.snapshot.items[0].content = " Inspect fixture ".into();
        assert!(restore(&r).is_err());
    }
    #[test]
    fn durable_todo_feedback_refuses_repair_and_profile_verification_claims() {
        let r = request();
        let valid = r.feedback.text();
        assert!(crate::planning::durable_feedback(valid, "fixture-call").is_ok());
        let mut raw: Value = serde_json::from_str(valid).unwrap();
        raw["unknown"] = true.into();
        assert!(crate::planning::durable_feedback(
            &serde_json::to_string(&raw).unwrap(),
            "fixture-call"
        )
        .is_err());
        let mut feedback: Feedback = serde_json::from_str(valid).unwrap();
        feedback.steps.push(crate::planning::StepFeedback {
            criterion: None,
            completion_batch: None,
            call_id: Some("fixture-call".into()),
            step: "Inspect fixture".into(),
            goal: None,
            expected: None,
            expected_calls: None,
            actual_calls: None,
            verification: crate::planning::Verification::Passed,
            check_tampered: false,
        });
        assert!(crate::planning::durable_feedback(
            &serde_json::to_string(&feedback).unwrap(),
            "fixture-call"
        )
        .is_err());
        feedback.steps[0].verification = crate::planning::Verification::Skipped;
        assert!(crate::planning::durable_feedback(
            &serde_json::to_string(&feedback).unwrap(),
            "fixture-call"
        )
        .is_ok());
        assert!(crate::planning::durable_feedback(
            &serde_json::to_string(&feedback).unwrap(),
            "other-call"
        )
        .is_err());
        let mut r = r.clone();
        r.feedback = StoredPlanFeedback::from_host_text(format!(" {valid}")).unwrap();
        assert!(restore(&r).is_err());
    }
    #[test]
    fn durable_todo_jobs_render_preserves_exact_success_bytes() {
        let r = request();
        let out = ToolOutput::ok(r.result_text.clone());
        let rendered = crate::jobs::render(
            "todo",
            "fixture-call",
            out,
            &crate::jobs::Terms {
                cap: r.result_text.len(),
                caps: Some(crate::tool::Capabilities {
                    private_data: true,
                    ..Default::default()
                }),
                spill_dir: None,
                mark_untrusted: true,
            },
        );
        assert_eq!(rendered.content, r.result_text);
        assert!(!rendered.is_error && !rendered.external);
    }
    #[tokio::test]
    async fn durable_todo_observes_common_batch_work_once_and_rejected_retry_once() -> Result<()> {
        let f = crate::memory_session::chat::task_plan::tests::Fixture::new()?;
        let mut first = f.call_ctx(0, 0, 24000);
        first.work = Some(crate::step::Work {
            calls: 5,
            in_flight: 1,
            last: Some(crate::step::Outcome::Failed),
            run: 1,
            ..Default::default()
        });
        let input = json!({"items":[{"content":"Inspect fixture","status":"in_progress"}]});
        assert!(!f.tool.call(input.clone(), &first).await?.is_error);
        let mut second = f.call_ctx(1, 0, 24000);
        second.work = Some(crate::step::Work {
            last: Some(crate::step::Outcome::Ok),
            ..first.work.unwrap()
        });
        assert!(!f.tool.call(input, &second).await?.is_error);
        let memory = f.tool.memory.as_ref().unwrap();
        {
            let state = memory.state.lock().unwrap();
            assert_eq!(state.tracked.own_calls, 2);
            assert_eq!(state.tracked.last_real, Some(crate::step::Outcome::Failed));
            assert_eq!(state.tracked.next_own_position, Some(7));
        }
        let rejected = f.call_ctx(2, 0, 24000);
        for _ in 0..2 {
            assert!(
                f.tool
                    .call(json!({"items":false}), &rejected)
                    .await?
                    .is_error
            );
        }
        assert_eq!(memory.state.lock().unwrap().tracked.own_calls, 3);
        let oversized = f.call_ctx(3, 0, 24000);
        for _ in 0..2 {
            assert!(f.tool.call(json!({"items":vec![json!({"content":"bounded item","status":"pending"});33]}),&oversized).await?.is_error);
        }
        assert_eq!(memory.state.lock().unwrap().tracked.own_calls, 4);

        Ok(())
    }
}
