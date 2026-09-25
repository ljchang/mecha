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
        let examples: Vec<Value> = ctx.goal_examples.iter().filter(|e| Some(&e.goal) == goal.as_ref()).take(2)
            .map(|e| json!({"session":e.source,"step":crate::step::ellipsize(&e.step,400),"expected":e.expected.as_ref().map(|s| crate::step::ellipsize(s,400)),"evidence":"declared check passed at that time"})).collect();
        let mut answer = json!({"serves":goal,"confirmed_goal":anchor,"lessons":lessons,"examples":examples,
            "evidence_limit":"These are applicable learned rules, not proof that this goal has been achieved. Declare the expected outcome and a relevant check in the plan."});
        if let Some(past) = &ctx.goal_appraisals {
            past_appraisals(&mut answer, past, goal.as_ref());
        }
        Ok(ToolOutput::ok(answer.to_string()))
    }
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
    answer["past_appraisals"] = json!(served);
    answer["appraisal_limit"] = json!(APPRAISAL_LIMIT);
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
}
