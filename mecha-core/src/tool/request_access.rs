//! `request_access`: the model asks the owner to open a directory outside
//! the workspace for the rest of this conversation ([`crate::grant`]).
//!
//! The tool decides nothing. Its whole content is a person's yes, so it
//! declares [`Tool::needs_a_person`] for **every** call and the loop sends it
//! to [`super::Approver::escalate_access`] — the door a permission mode, an
//! "always" list and `--yes` never answer, and a headless approver answers
//! `Blocked` (a voice call's `--voice-yes` approver puts it on the page's
//! card instead). Unconditionally, even for a path [`crate::grant::admit`] will
//! refuse: deciding "no person needed" from a check made before the batch
//! executes is a race a sibling call in the same turn can win (a `shell`
//! that creates the directory the check found missing), and the grant would
//! then land with nobody asked. The owner may be asked about a request the
//! tool then refuses; that costs a click, and the other order costs the jail.
//!
//! What the person reads is the canonical path and the access, which this
//! tool computes, beside the reason, which the model wrote and is labelled
//! so — a reason is exactly where an injection would argue its case.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::grant::{self, Access, Grant, Source};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct RequestAccess;

/// The registered name, for the places that ask whether it is reachable.
pub const NAME: &str = "request_access";

fn parse(input: &Value) -> std::result::Result<(String, Access, String), String> {
    let path = input
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or("missing required string argument `path`")?;
    let access = match input.get("access").and_then(Value::as_str) {
        None => Access::Read,
        Some(a) => Access::parse(a).ok_or_else(|| format!("access `{a}` is not read or write"))?,
    };
    let reason = input
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or("missing required string argument `reason` — say what the task needs it for")?;
    Ok((path.to_string(), access, reason.to_string()))
}

#[async_trait]
impl Tool for RequestAccess {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "Ask the owner to let you reach a directory (or file) outside your workspace for the \
         rest of this conversation. The owner sees the path, the access and your reason, and \
         decides; nothing is reachable until they say yes. Name the narrowest directory the \
         task needs, in full (absolute or from `~/`). Credential directories and mecha's own \
         home are never granted. After a yes, use the normal file tools with the full path."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory or file, absolute or starting with ~/"
                },
                "access": {
                    "type": "string",
                    "enum": ["read", "write"],
                    "default": "read",
                    "description": "read (the default), or write — which includes read"
                },
                "reason": {
                    "type": "string",
                    "description": "One sentence the owner reads: what the task needs it for"
                }
            },
            "required": ["path", "reason"]
        })
    }

    /// Read-only, so a read-only chat can still put the question in front of
    /// the owner: the web and Slack approvers refuse an escalation for a
    /// tool that is not read-only under a read-only mode, which took the one
    /// way to ask away in the mode a web chat opens in (2026-10-18). It is
    /// safe there because the escalation, not the mode, is what gates it, and
    /// its whole effect is that more can be *read*: a `write` grant made in a
    /// read-only chat still writes nothing until the mode allows writes.
    ///
    /// **Read-only for the approver, not for the planning phase**, which keys
    /// on this flag too: `call` refuses in `Phase::Plan` itself, as
    /// `message_send` and `image_library_reference` do, because a grant made
    /// while planning would outlive the plan (review of #714). And no card is
    /// raised there first: `needs_a_person` reads the phase and stays quiet,
    /// so the owner is never asked a question whose yes is thrown away.
    fn read_only(&self) -> bool {
        true
    }

    /// Reads nothing and sends nothing itself. What is read through the
    /// grant arms `private_data` on the tool that reads it.
    ///
    /// `destructive`, though it deletes nothing, for `image_library_reference`'s
    /// reason: a tool profile keeps any read-only tool that is neither private
    /// nor destructive as a public read, and a `research` or `coding` child
    /// was keeping the one tool that widens the jail once this became
    /// read-only (review of #714). `assistant` keeps it, as it keeps every
    /// read-only tool. The flag is how a profile is told; nothing
    /// in the loop or the interlock reads it.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().destructive()
    }

    /// No `grant` on the answer: `call` makes its own, re-admitting the
    /// path at execution, because the filesystem may change between the ask
    /// and the yes.
    fn needs_a_person(&self, input: &Value, ctx: &ToolCtx) -> Option<super::PersonAsk> {
        // Planning: `call` refuses whatever the answer, so asking would cost
        // the owner a click for nothing (review of #714, pass 3).
        if ctx.phase == crate::agent::Phase::Plan {
            return None;
        }
        let ask = |why: String| Some(super::PersonAsk { why, grant: None });
        let Ok((raw, access, reason)) = parse(input) else {
            return ask(
                "The model is asking for access outside its workspace with a malformed \
                 request; it will be refused whatever you answer."
                    .into(),
            );
        };
        let requested = grant::expand(&raw);
        let shown = match grant::admit(&requested) {
            Ok(path) => format!("{} ({})", path.display(), access.describe()),
            Err(e) => format!(
                "{} — which cannot be granted and will be refused whatever you answer: {e:#}",
                requested.display()
            ),
        };
        ask(format!(
            "The model asks to reach {shown}, outside its workspace, for the rest of this \
             conversation. Its reason, in its own words: \"{reason}\". Saying yes lets every \
             later call in this conversation (and its subagents) use that path."
        ))
    }

    /// A question for a person, as `ask_user` is; it declares `destructive`
    /// too, so it resolves `Irreversible`.
    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        Some(crate::tool::effect::Effect::StoreWrite)
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        if ctx.phase == crate::agent::Phase::Plan {
            return Ok(ToolOutput::err(
                "request_access is not available while planning: a grant widens what every \
                 later call in this conversation can touch. Nothing was granted; ask again \
                 once the plan is accepted.",
            ));
        }
        let (raw, access, _reason) = match parse(&input) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::err(e)),
        };
        // Admitted again here, not trusted from the prompt: the filesystem may
        // have changed between the ask and the yes.
        let path = match grant::admit(&grant::expand(&raw)) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::err(format!("not granted: {e:#}"))),
        };
        let new = match ctx.grants.add(Grant {
            path: path.clone(),
            access,
            source: Source::Approved,
        }) {
            Ok(new) => new,
            Err(e) => return Ok(ToolOutput::err(format!("not granted: {e:#}"))),
        };
        let what = access.describe();
        Ok(ToolOutput::ok(if new {
            format!(
                "Granted: you may {what} {} for the rest of this conversation. Use the full \
                 path with the file tools.",
                path.display()
            )
        } else {
            format!(
                "Already granted: you may {what} {} in this conversation.",
                path.display()
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_needs_a_person_even_a_refused_one() {
        for input in [
            json!({"path": "/", "reason": "x"}),
            json!({"path": "/definitely/not/here", "reason": "x"}),
            json!({"reason": "no path"}),
            json!({"path": std::env::temp_dir(), "reason": "papers"}),
        ] {
            assert!(
                RequestAccess
                    .needs_a_person(&input, &ToolCtx::default())
                    .is_some(),
                "{input} would run with nobody asked"
            );
        }
    }

    /// A grant made while planning would outlive the plan, so the planning
    /// phase refuses it even after a yes (review of #714: read-only for the
    /// approver let the phase gate offer it).
    /// And the owner is not asked a question whose yes would be discarded.
    #[test]
    fn a_planning_run_raises_no_card() {
        let planning = ToolCtx {
            phase: crate::agent::Phase::Plan,
            ..ToolCtx::default()
        };
        let input = json!({"path": "/srv/papers", "reason": "plan"});
        assert!(RequestAccess.needs_a_person(&input, &planning).is_none());
        assert!(RequestAccess
            .needs_a_person(&input, &ToolCtx::default())
            .is_some());
    }

    #[tokio::test]
    async fn a_planning_run_grants_nothing() {
        let dir = std::env::temp_dir().join(format!("mecha-req-plan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = ToolCtx {
            grants: crate::grant::Grants::default().for_run(&[]),
            phase: crate::agent::Phase::Plan,
            ..ToolCtx::default()
        };
        let out = RequestAccess
            .call(
                json!({"path": dir.display().to_string(), "reason": "plan"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(ctx.grants.live().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_approved_call_adds_a_live_grant_and_a_refused_path_adds_none() {
        let dir = std::env::temp_dir().join(format!("mecha-req-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = ToolCtx {
            grants: crate::grant::Grants::default().for_run(&[]),
            ..ToolCtx::default()
        };
        let out = RequestAccess
            .call(json!({"path": "/", "reason": "everything"}), &ctx)
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(ctx.grants.live().is_empty());

        let out = RequestAccess
            .call(
                json!({"path": dir.display().to_string(), "access": "write", "reason": "notes"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let live = ctx.grants.live();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].access, Access::Write);
        assert_eq!(live[0].source, Source::Approved);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
