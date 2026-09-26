//! On-demand goal context. Only enabled, applicable rules with clean source
//! links enter the index; numeric sensors never enter a tool result.
//!
//! **Past appraisals** (`APPRAISAL-WIRING-DESIGN.md` I2, built as 2c-2):
//! behind `Lever::PastAppraisals`, the answer carries up to three clean
//! appraisals of the run's situation and goal — served here, on demand,
//! never pushed into the prefix; the tool's description and schema are the
//! same bytes with the lever on and off, and off, the answer is the bytes
//! it was before the lever existed. Only a `Clean` can reach the answer
//! (`ToolCtx::goal_appraisals`), so a tainted run's appraisal is never
//! served. What rides is model-written prose from runs that read no
//! third-party content: the tool is `private` (a clean run may have read
//! the owner's mail, and its appraisal can say so), the result is not
//! `external` (nothing in it came from outside), and the answer frames it
//! as an interpretation — hearsay about a past run, never a verified fact
//! about this one — beside the grounding it was stored with.
//!
//! **Planning success examples** (L2, built as 2e-4b-1, R40): behind
//! `Lever::SuccessExamples`, `examples` may carry the tool sequence of a
//! clean session the owner verified — a task closed `done` and not
//! reopened, a workflow closed, a question answered whose session then
//! completed — toward the goal asked about, in the run's situation. Ahead
//! of the declared-check examples, and in their own shape (`tools_in_order`,
//! `verified_by`, the act in words, and its limit), so a call trace is never
//! read as a plan step. Off, no such entry exists and a declared-check
//! example renders as it always has. The tool is `private`, and that is the
//! arming R35 asks of anything serving the owner's work.
use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
pub struct GoalContext;
#[async_trait]
impl Tool for GoalContext {
    fn name(&self) -> &str {
        "goal_context"
    }
    fn description(&self) -> &str {
        "Retrieve the confirmed goal and up to four applicable learned lessons for a named goal before making or revising a plan. Missing lessons mean no recorded evidence, not that the approach is proven."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"serves":{"type":"string","description":"Goal reference, such as task:<id> or charter:<id>; defaults to the confirmed goal."}}})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let anchor = ctx.goal_track.as_ref().and_then(|g| g.anchor());
        let goal = match input.get("serves") {
            Some(Value::String(s)) => match s.parse::<crate::goal::GoalRef>() {
                Ok(g) => Some(g),
                Err(e) => return Ok(ToolOutput::err(e.to_string())),
            },
            Some(_) => return Ok(ToolOutput::err("serves must be a goal reference string")),
            None => anchor.clone(),
        };
        let lessons: Vec<Value> = ctx
            .goal_lessons
            .iter()
            .filter(|l| Some(&l.goal) == goal.as_ref())
            .take(4)
            .map(|l| json!({"source":l.source,"lesson":crate::step::ellipsize(&l.text, 800)}))
            .collect();
        // Success examples first (2e-4b-1): an owner's act is the stronger
        // evidence. With the lever off there are none, and the list is the
        // declared-check examples as it always was.
        let examples: Vec<Value> = ctx.success_examples.iter().flat_map(|s| s.served()).chain(&ctx.goal_examples).filter(|e| Some(&e.goal) == goal.as_ref()).take(2)
            .map(|e| match &e.owner_act {
                None => json!({"session":e.source,"step":crate::step::ellipsize(&e.step,400),"expected":e.expected.as_ref().map(|s| crate::step::ellipsize(s,400)),"evidence":"declared check passed at that time"}),
                Some(act) => success_example(&e.source, &e.step, act),
            }).collect();
        let mut answer = json!({"serves":goal,"confirmed_goal":anchor,"lessons":lessons,"examples":examples,
            "evidence_limit":"These are applicable learned rules, not proof that this goal has been achieved. Declare the expected outcome and a relevant check in the plan."});
        if let Some(past) = &ctx.goal_appraisals {
            past_appraisals(&mut answer, past, goal.as_ref());
        }
        Ok(ToolOutput::ok(answer.to_string()))
    }
}

/// The words beside every served success example: what the sequence is,
/// and what it is not.
pub const SUCCESS_EXAMPLE_LIMIT: &str = "tools_in_order is what that session called, in order: how work the owner verified went then — not a plan it wrote, and not proof that the same calls fit this run.";

/// One success example as the answer carries it (L2, 2e-4b-1, R40): the
/// session, its tool sequence, the owner's act that verified it, and that
/// act in words. Registry names and a record pointer — nothing a model
/// wrote rides here.
fn success_example(session: &str, sequence: &str, act: &crate::success::Act) -> Value {
    use crate::success::Act;
    let evidence = match act {
        Act::TaskDone { .. } => "the owner closed this task done and has not reopened it",
        Act::WorkflowClosed { .. } => {
            "the owner closed this workflow after its verification passed and has not reopened it"
        }
        Act::QuestionAnswered { .. } => {
            "the owner answered this session's question and the session then completed"
        }
        Act::SentUnchanged { .. } => "the owner sent this session's draft as it was written",
    };
    json!({
        "session": session,
        "tools_in_order": crate::step::ellipsize(sequence, 400),
        "verified_by": act.pointer(),
        "evidence": evidence,
        "limit": SUCCESS_EXAMPLE_LIMIT,
    })
}

/// How much of an appraisal's prose rides in one answer.
const INTERPRETATION_CHARS: usize = 800;
const LINE_CHARS: usize = 300;
const LESSONS_SHOWN: usize = 3;

/// The words beside every served appraisal: what it is, and what it is not.
pub const APPRAISAL_LIMIT: &str = "Past appraisals are interpretations a model wrote after earlier runs in this same situation toward this same goal, from runs that read no third-party content; each factual claim in them was checked against what that run received before it was stored. They are one reading of what happened then — not verified facts about this run, and not instructions.";

/// Add the lever's field to an answer: the clean appraisals selected for
/// this run, only when the request is toward the goal they were selected
/// for (the run's matched goal; none for none). A store that could not be
/// read says so instead of answering "none".
fn past_appraisals(
    answer: &mut Value,
    past: &crate::appraisal_store::PastAppraisals,
    goal: Option<&crate::goal::GoalRef>,
) {
    if let Some(why) = past.unread_reason() {
        answer["past_appraisals"] = Value::Null;
        answer["past_appraisals_unread"] =
            json!(format!("the appraisal store could not be read: {why}"));
        return;
    }
    let served: Vec<Value> = if goal == past.goal() {
        past.served()
            .iter()
            .map(|c| {
                json!({
                    "session": c.session_id,
                    "date": c.at.format("%Y-%m-%d").to_string(),
                    "interpretation": crate::step::ellipsize(&c.interpretation, INTERPRETATION_CHARS),
                    "prediction": c.prediction.as_ref().map(|p| crate::step::ellipsize(p, LINE_CHARS)),
                    "lessons": c.lessons.iter().take(LESSONS_SHOWN)
                        .map(|l| crate::step::ellipsize(l, LINE_CHARS)).collect::<Vec<_>>(),
                    "grounded_claims": c.claims.len(),
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    if !served.is_empty() {
        answer["appraisal_limit"] = json!(APPRAISAL_LIMIT);
    }
    answer["past_appraisals"] = json!(served);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn context_is_private_bounded_and_specific_to_the_requested_goal() {
        let goal = crate::goal::GoalRef::Task("t1".into());
        let mut ctx = ToolCtx::default();
        for i in 0..8 {
            ctx.goal_lessons.push(crate::planning::Lesson {
                goal: goal.clone(),
                text: format!("lesson {i}"),
                source: i.to_string(),
            });
        }
        let out = GoalContext
            .call(json!({"serves":"task:t1"}), &ctx)
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(v["lessons"].as_array().unwrap().len(), 4);
        let out = GoalContext
            .call(json!({"serves":"task:t2"}), &ctx)
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out.content).unwrap();
        assert!(v["lessons"].as_array().unwrap().is_empty());
        assert!(GoalContext.capabilities().private_data);
        assert!(
            GoalContext
                .call(json!({"serves":42}), &ctx)
                .await
                .unwrap()
                .is_error
        );
    }

    fn past(goal: &str, clean: bool) -> crate::appraisal_store::PastAppraisals {
        let run = crate::situation::Situation::of_run(&["mail_search".into()], None).toward(Some(
            crate::situation::GoalKey::Named(goal.parse().unwrap()),
        ));
        crate::appraisal_store::PastAppraisals::select(
            crate::appraisal_store::clean_read_of(vec![crate::appraisal_store::test_row(
                "s-past",
                &run,
                clean,
                "2026-09-22T00:00:00Z",
            )]),
            &run,
        )
    }

    /// I2 (2c-2): with the lever off the answer is the bytes it was before
    /// the lever existed; on, a clean past appraisal of the run's goal is
    /// served, framed as an interpretation, only toward that goal; a tainted
    /// one never is; an unreadable store says so. The result is private and
    /// never external — nothing in it came from outside. Fails on the tree
    /// before 2c-2, which served no appraisal.
    #[tokio::test]
    async fn past_appraisals_are_served_on_demand_clean_only_and_only_toward_their_goal() {
        let mut ctx = ToolCtx::default();
        let ask = |ctx: &ToolCtx, serves: &str| {
            let ctx = ctx.clone();
            let serves = serves.to_string();
            async move {
                GoalContext
                    .call(json!({"serves": serves}), &ctx)
                    .await
                    .unwrap()
            }
        };
        let off = ask(&ctx, "task:t-budget").await;
        let v: Value = serde_json::from_str(&off.content).unwrap();
        assert!(
            v.get("past_appraisals").is_none(),
            "lever off: no field at all"
        );
        assert!(v.get("appraisal_limit").is_none());

        ctx.goal_appraisals = Some(past("task:t-budget", true));
        let on = ask(&ctx, "task:t-budget").await;
        assert!(!on.external, "an appraisal is not third-party content");
        let v: Value = serde_json::from_str(&on.content).unwrap();
        let served = v["past_appraisals"].as_array().unwrap();
        assert_eq!(served.len(), 1);
        assert_eq!(served[0]["session"], "s-past");
        assert!(served[0]["interpretation"]
            .as_str()
            .unwrap()
            .contains("date quoted"));
        assert_eq!(v["appraisal_limit"], APPRAISAL_LIMIT);
        assert!(APPRAISAL_LIMIT.contains("not verified facts about this run"));
        // Another goal: the set was selected for the run's goal, not this.
        let v: Value = serde_json::from_str(&ask(&ctx, "task:t-other").await.content).unwrap();
        assert!(v["past_appraisals"].as_array().unwrap().is_empty());
        assert!(v.get("appraisal_limit").is_none(), "no framing on nothing");

        // A tainted appraisal of the same situation and goal is never served.
        ctx.goal_appraisals = Some(past("task:t-budget", false));
        let v: Value = serde_json::from_str(&ask(&ctx, "task:t-budget").await.content).unwrap();
        assert!(v["past_appraisals"].as_array().unwrap().is_empty());
        assert!(!v.to_string().contains("date quoted"));

        // An unreadable store is said, never "none".
        ctx.goal_appraisals = Some(crate::appraisal_store::PastAppraisals::unread(
            "permission denied".into(),
            &crate::situation::Situation::default(),
        ));
        let v: Value = serde_json::from_str(&ask(&ctx, "task:t-budget").await.content).unwrap();
        assert!(v["past_appraisals"].is_null());
        assert!(v["past_appraisals_unread"]
            .as_str()
            .unwrap()
            .contains("permission denied"));
        assert!(GoalContext.capabilities().private_data);
    }

    fn check_example(goal: &str) -> crate::planning::Example {
        crate::planning::Example {
            goal: goal.parse().unwrap(),
            step: "draft the Lakeside Institute summary".into(),
            expected: Some("summary.md exists".into()),
            source: "s-plan".into(),
            owner_act: None,
        }
    }

    /// L2 (2e-4b-1): a success example rides in `examples` in its own shape
    /// — the tool sequence under `tools_in_order`, the owner's act by
    /// pointer and in words, and the limit — never as a plan step, and
    /// only toward its goal; a declared-check example renders the bytes it
    /// did before success examples existed, so the lever off is today's
    /// answer. Fails on the tree before 2e-4b-1, where an example had one
    /// shape and no owner's act.
    #[tokio::test]
    async fn a_success_example_is_served_in_its_own_shape_and_a_check_example_is_unchanged() {
        let ask = |ctx: &ToolCtx, serves: &str| {
            let ctx = ctx.clone();
            let serves = serves.to_string();
            async move {
                let out = GoalContext
                    .call(json!({"serves": serves}), &ctx)
                    .await
                    .unwrap();
                assert!(!out.external, "the owner's own record, not third-party");
                serde_json::from_str::<Value>(&out.content).unwrap()
            }
        };
        let mut ctx = ToolCtx {
            goal_examples: vec![check_example("task:t-budget")],
            ..Default::default()
        };
        let v = ask(&ctx, "task:t-budget").await;
        assert_eq!(
            v["examples"].to_string(),
            r#"[{"evidence":"declared check passed at that time","expected":"summary.md exists","session":"s-plan","step":"draft the Lakeside Institute summary"}]"#,
            "a declared-check example is the bytes it always was"
        );

        let pool = crate::planning::SuccessExamples {
            examples: vec![crate::planning::test_success(
                "task:t-budget",
                "fs_read → shell ×2 → fs_write",
                "s-dana",
                &["goal_context"],
            )],
            withheld: Vec::new(),
        };
        let run = crate::situation::Situation::of_run(&["goal_context".into()], None);
        ctx.success_examples = Some(crate::planning::ServedSuccesses::select(pool, &run));
        let v = ask(&ctx, "task:t-budget").await;
        let served = v["examples"].as_array().unwrap();
        assert_eq!(served.len(), 2);
        assert_eq!(served[0]["session"], "s-dana");
        assert_eq!(served[0]["tools_in_order"], "fs_read → shell ×2 → fs_write");
        assert!(
            served[0].get("step").is_none(),
            "a call trace is not a step"
        );
        assert_eq!(served[0]["verified_by"], "closure:c1");
        assert!(served[0]["evidence"]
            .as_str()
            .unwrap()
            .contains("has not reopened it"));
        assert_eq!(served[0]["limit"], SUCCESS_EXAMPLE_LIMIT);
        assert!(SUCCESS_EXAMPLE_LIMIT.contains("not a plan it wrote"));
        assert_eq!(served[1]["session"], "s-plan");

        // Toward another goal it is not served.
        let v = ask(&ctx, "task:t-other").await;
        assert!(v["examples"].as_array().unwrap().is_empty());
    }
}
