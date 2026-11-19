//! `propose_follow_up`: the model asks to come back to something later.
//!
//! A proposal and nothing more (`crate::followup`): the call writes one record
//! to the follow-up store and returns. Nothing is scheduled until the owner
//! accepts it, and the accept — never this tool — writes the workflow whose
//! reminder comes due (`ASSISTANT-FOLLOW-THROUGH-DESIGN.md` decision 6).
//!
//! Registered, when `[features] followups` is on, by the front ends whose
//! conversations a follow-up can return to — the web chat (its voice calls
//! included) and the TUI — and refused at call time where the run has
//! no recorded conversation ([`ToolCtx::recorded_session`]), because a
//! follow-up that points at no transcript is a reminder about nothing.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::agent::Taint;
use crate::clock::Clock;
use crate::followup::{self, FollowUpStore};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;

pub const NAME: &str = "propose_follow_up";

pub struct ProposeFollowUp {
    store: Arc<FollowUpStore>,
    clock: Arc<dyn Clock>,
}

impl ProposeFollowUp {
    pub fn new(store: Arc<FollowUpStore>, clock: Arc<dyn Clock>) -> Self {
        ProposeFollowUp { store, clock }
    }

    /// When the model asked for: exactly one of `at` (RFC 3339, with its
    /// offset) or `in_hours`. The error is a sentence it can act on.
    fn when(input: &Value, now: DateTime<Utc>) -> std::result::Result<DateTime<Utc>, String> {
        let at = input.get("at").filter(|v| !v.is_null());
        let in_hours = input.get("in_hours").filter(|v| !v.is_null());
        match (at, in_hours) {
            (Some(_), Some(_)) => Err("give `at` or `in_hours`, not both".into()),
            (None, None) => {
                Err("say when: `at` (an RFC 3339 time with its offset) or `in_hours`".into())
            }
            (Some(at), None) => at
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
                .map(|t| t.with_timezone(&Utc))
                .ok_or_else(|| {
                    "`at` must be an RFC 3339 time with its offset, e.g. 2026-11-19T09:00:00-05:00"
                        .into()
                }),
            (None, Some(h)) => {
                let hours = h
                    .as_f64()
                    .filter(|h| h.is_finite() && *h > 0.0)
                    .ok_or_else(|| "`in_hours` must be a positive number".to_string())?;
                // Bounded before the conversion, so a huge number is the
                // window's refusal and never an overflow.
                let minutes = (hours * 60.0).min(followup::MAX_LEAD.num_minutes() as f64 + 1.0);
                Ok(now + chrono::Duration::minutes(minutes.round() as i64))
            }
        }
    }
}

#[async_trait]
impl Tool for ProposeFollowUp {
    fn name(&self) -> &str {
        "propose_follow_up"
    }

    fn description(&self) -> &str {
        "Propose coming back to this conversation later — to check whether something \
         happened, or to pick a thread up again — when the user would want that and it \
         cannot be done now. It is only a proposal: the user accepts or rejects it, and \
         nothing is scheduled until they accept. Once accepted, they are reminded at the \
         time you name, and the reminder opens this conversation.\n\
         \n\
         `what` is the reminder's title: one line saying what to check, in words the user \
         will recognise later without this conversation in front of them. Give the time \
         as `at` (RFC 3339 with its offset, in the user's timezone) or `in_hours`. Propose \
         at most one per thing; do not propose a follow-up for work you can finish now."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "what": {
                    "type": "string",
                    "description": "What to check or pick up, in one line."
                },
                "at": {
                    "type": "string",
                    "description": "When, as an RFC 3339 time with its offset."
                },
                "in_hours": {
                    "type": "number",
                    "description": "When, as hours from now. Instead of `at`."
                }
            },
            "required": ["what"]
        })
    }

    /// A proposal reaches nobody and changes nothing the owner has not
    /// decided, so it is available while planning, as `ask_user` is.
    fn read_only(&self) -> bool {
        true
    }

    /// Nothing: it returns only its own confirmation.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        Some(crate::tool::effect::Effect::StoreWrite)
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(session) = ctx.recorded_session.as_deref().filter(|s| !s.is_empty()) else {
            return Ok(ToolOutput::err(
                "This conversation is not recorded, so a follow-up would have nothing to come \
                 back to. Tell the user what to check and when instead.",
            ));
        };
        let now = self.clock.now();
        let due = match Self::when(&input, now) {
            Ok(due) => due,
            Err(e) => return Ok(ToolOutput::err(e)),
        };
        let what = input.get("what").and_then(Value::as_str).unwrap_or("");
        let what = match followup::admit(what, due, now) {
            Ok(what) => what,
            Err(e) => return Ok(ToolOutput::err(e)),
        };
        // Unknown taint is full taint: a context nobody stamped must not
        // record a proposal as cleaner than the conversation it came from.
        // The conversation's taint as recorded before this batch — a record
        // of authorship, like every proposer's (`ToolCtx::recorded_taint`).
        // `taint` is the send gate's union over the whole batch, and would
        // mark a proposal made beside an `fs_read` as having read outside.
        let taint = ctx.recorded_taint.unwrap_or(Taint {
            private: true,
            untrusted: true,
        });
        match self
            .store
            .propose(what, due, session, Some(ctx.workspace.clone()), taint, now)
        {
            Ok(f) => Ok(ToolOutput::ok(format!(
                "Proposed follow-up {} for {}: \"{}\". The user accepts or rejects it; nothing is \
                 scheduled until they do.",
                followup::short(&f.id),
                f.due_at.to_rfc3339(),
                f.what
            ))),
            // A store that cannot be written is said, never reported as
            // proposed: the user would wait for a reminder nobody recorded.
            Err(e) => Ok(ToolOutput::err(format!(
                "The follow-up could not be recorded ({e:#}). Tell the user what to check and \
                 when instead."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tool() -> (Scratch, Arc<FollowUpStore>, ProposeFollowUp) {
        let dir = std::env::temp_dir().join(format!("follow-up-tool-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(FollowUpStore::open(dir.join("followups")).unwrap());
        let clock = Arc::new(FixedClock("2026-11-18T12:00:00Z".parse().unwrap()));
        let t = ProposeFollowUp::new(Arc::clone(&store), clock);
        (Scratch(dir), store, t)
    }

    fn ctx(session: Option<&str>, taint: Option<Taint>) -> ToolCtx {
        ToolCtx {
            recorded_session: session.map(String::from),
            recorded_taint: taint,
            workspace: "/tmp/w".into(),
            ..ToolCtx::default()
        }
    }

    #[tokio::test]
    async fn a_proposal_is_recorded_with_its_conversation_and_nothing_else_happens() {
        let (_d, store, t) = tool();
        let out = t
            .call(
                json!({"what": "check the build", "at": "2026-11-19T09:00:00-05:00"}),
                &ctx(Some("sess-a"), Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        // Spelled as a literal in `name()` for the replay classifier's
        // source scan; the constant callers use must agree with it.
        assert_eq!(t.name(), NAME);
        let items = store.items().unwrap();
        assert_eq!(items.len(), 1);
        let f = &items[0];
        assert_eq!(f.session_id, "sess-a");
        assert_eq!(f.status, followup::PROPOSED);
        assert_eq!(f.due_at.to_rfc3339(), "2026-11-19T14:00:00+00:00");
        assert_eq!(f.workspace.as_deref(), Some(std::path::Path::new("/tmp/w")));
        assert!(f.workflow_id.is_none());
        assert!(out.content.contains("nothing is scheduled"));
    }

    #[tokio::test]
    async fn no_recorded_conversation_is_refused_and_nothing_is_stored() {
        let (_d, store, t) = tool();
        let out = t
            .call(json!({"what": "x", "in_hours": 2}), &ctx(None, None))
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(store.items().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unstamped_taint_is_recorded_as_full_taint() {
        let (_d, store, t) = tool();
        let out = t
            .call(json!({"what": "x", "in_hours": 2}), &ctx(Some("s"), None))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let f = &store.items().unwrap()[0];
        let taint = f.taint.unwrap();
        assert!(taint.private && taint.untrusted);
    }

    /// Authorship, not the send gate: a proposal batched beside an
    /// `fs_read` or a `web_search` is recorded with what the conversation
    /// had read before the batch (review of #896).
    #[tokio::test]
    async fn the_recorded_taint_is_the_conversations_not_the_batchs() {
        let (_d, store, t) = tool();
        let c = ToolCtx {
            taint: Some(Taint {
                private: true,
                untrusted: true,
            }),
            ..ctx(Some("s"), Some(Taint::default()))
        };
        let out = t
            .call(json!({"what": "x", "in_hours": 2}), &c)
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let f = &store.items().unwrap()[0];
        assert_eq!(f.taint, Some(Taint::default()));
        assert!(!f.armed());
    }

    #[tokio::test]
    async fn the_time_is_one_of_two_forms_inside_the_window() {
        let (_d, store, t) = tool();
        let c = ctx(Some("s"), Some(Taint::default()));
        for bad in [
            json!({"what": "x"}),
            json!({"what": "x", "at": "tomorrow"}),
            json!({"what": "x", "at": "2026-11-19T09:00:00"}),
            json!({"what": "x", "at": "2026-11-19T09:00:00Z", "in_hours": 3}),
            json!({"what": "x", "in_hours": -1}),
            json!({"what": "x", "in_hours": 0.05}),
            json!({"what": "x", "in_hours": 1e300}),
        ] {
            let out = t.call(bad.clone(), &c).await.unwrap();
            assert!(out.is_error, "{bad} was accepted");
        }
        assert!(store.items().unwrap().is_empty());
        let out = t
            .call(json!({"what": "x", "in_hours": 1.5}), &c)
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            store.items().unwrap()[0].due_at.to_rfc3339(),
            "2026-11-18T13:30:00+00:00"
        );
    }
}
