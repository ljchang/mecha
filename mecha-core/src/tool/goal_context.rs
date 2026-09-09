//! On-demand goal context. Only enabled, applicable rules with clean source
//! links enter the index; numeric sensors never enter a tool result.
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
        Ok(ToolOutput::ok(json!({"serves":goal,"confirmed_goal":anchor,"lessons":lessons,"examples":examples,
            "evidence_limit":"These are applicable learned rules, not proof that this goal has been achieved. Declare the expected outcome and a relevant check in the plan."}).to_string()))
    }
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
}
