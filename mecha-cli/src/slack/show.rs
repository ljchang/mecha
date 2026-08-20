//! `show_file` — the model putting something in front of the user.
//!
//! The other half of `/send`: that is the person asking for a file, this is the
//! agent deciding one is worth looking at. A run that renders a chart on a
//! headless box and then describes it in prose has done the work and withheld
//! the result.
//!
//! **The safety argument is one sentence: the model names a path, never a
//! destination.** There is no channel argument and deliberately no way to add
//! one — the thread comes from this process's own attach record. The same
//! shape as `frontdoor::Record::for_privileged_run`, where the property is
//! enforced by a signature rather than by a rule someone remembers.
//!
//! That is what puts it in the **third quadrant**, beside `mail_triage` and
//! `docs_trash`:
//!
//! - **Not `external_send`.** It reaches the owner's own two-party DM, so no
//!   third party learns anything. Marking it a send sink would mean a tainted
//!   session cannot show the user the chart it just made — the interlock
//!   firing on the one destination that is definitionally safe.
//! - **Never in `[outbox] tools`.** Staging it would make review circular: you
//!   would approve a draft in order to see the picture you asked for.
//! - **`private_data`**, because it reads workspace bytes.
//! - **`read_only`**, because it changes nothing. It is a display action aimed
//!   at the principal, and an approval prompt in front of "here is your chart"
//!   is the kind that teaches people to press yes without reading.
//!
//! The residual, stated rather than hidden: Slack holds the bytes. That is
//! already true of the mirror, so it is a property of having turned remote
//! control on at all rather than one this tool introduces — which is why it
//! refuses when the session is not attached instead of finding another way out.
//!
//! Two deviations from `docs/REMOTE-CONTROL-DESIGN.md` §7, both deliberate:
//!
//! - **It is registered always, not only while attached.** The tool list is
//!   the front of the cached prefix, so adding or removing one mid-session
//!   re-pays the whole prefix — the same reason nothing may toggle skills per
//!   turn. Gating at call time costs one schema in the prompt and buys a cache
//!   that survives `/remote-control`. The containment is unchanged: the
//!   destination still cannot be named.
//! - **No per-run count cap.** A cap would need per-run state on a tool shared
//!   across runs, and the thing it guards against — a model calling one tool
//!   forever — is what `max_turns` already bounds. The size cap is real.

use anyhow::Result;
use async_trait::async_trait;
use mecha_core::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use mecha_slack::{files, Slack};
use serde_json::{json, Value};

pub struct ShowFileTool;

#[async_trait]
impl Tool for ShowFileTool {
    fn name(&self) -> &str {
        "show_file"
    }

    fn description(&self) -> &str {
        "Put a file in front of the user where they can actually look at it — a chart, a \
         rendered image, a PDF, a log. Use this when you have produced something whose \
         point is to be *seen* rather than described, and the user is not sitting at this \
         machine. **Call it in a later turn than the one that wrote the file**: tool calls \
         you request together are executed at the same time, so showing a file in the same \
         turn that creates it will usually find nothing there. It works only while the \
         session is mirrored to a Slack thread; if it is not, say what you made and where \
         it is instead."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to show, relative to the workspace."
                }
            },
            "required": ["path"]
        })
    }

    /// Nothing is modified and nothing leaves the principal, so there is
    /// nothing for a human to weigh. See the module docs.
    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(raw) = input.get("path").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("show_file needs a `path`"));
        };

        // The path jail, like every other model-supplied path in this system.
        let path = match ctx.resolve(raw) {
            Ok(path) => path,
            Err(e) => return Ok(ToolOutput::err(format!("{raw}: {e}"))),
        };

        let store = match crate::slack::remote::RemoteStore::open_default() {
            Ok(store) => store,
            Err(e) => return Ok(ToolOutput::err(format!("no remote store: {e}"))),
        };
        // **The destination, and the only place it can come from.**
        let record = match store.attached_here() {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Ok(ToolOutput::err(
                    "this session is not mirrored to a Slack thread, so there is nowhere to \
                     show it. Tell the user what you made and where it is; they can attach \
                     with `/remote-control <name>` if they want to see it.",
                ))
            }
            Err(e) => return Ok(ToolOutput::err(format!("could not read the store: {e}"))),
        };

        // `Ok(is_error)`, not `?`. Every other failure in this function lets
        // the model route around it — say what it made and where — and a
        // malformed config is no more the model's fault than a missing file.
        let cfg = match mecha_core::config::Config::load_global() {
            Ok(cfg) => cfg,
            Err(e) => return Ok(ToolOutput::err(format!("could not read the config: {e}"))),
        };
        let max_bytes = cfg.slack.max_upload_mb.saturating_mul(1024 * 1024);
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) => return Ok(ToolOutput::err(format!("cannot read {raw}: {e}"))),
        };
        // Shared with `/send`, so a refusal reads the same however it was
        // asked for, and the two cannot drift about what is too big.
        let name = match crate::slack::send::vet(&path, meta.is_dir(), meta.len(), max_bytes) {
            Ok(name) => name,
            Err(e) => return Ok(ToolOutput::err(format!("{e:#}"))),
        };

        let (Some(channel), Some(thread_ts)) = (&record.channel_id, &record.thread_ts) else {
            return Ok(ToolOutput::err("the attachment has no thread yet"));
        };
        let home = match mecha_core::work::mecha_home() {
            Ok(home) => home,
            Err(e) => return Ok(ToolOutput::err(format!("no mecha home: {e}"))),
        };
        let creds = match mecha_slack::binding::SlackStore::open(home.join("slack"))
            .and_then(|s| s.credentials())
        {
            Ok(Some(creds)) => creds,
            Ok(None) => return Ok(ToolOutput::err("no Slack tokens stored")),
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "could not read the Slack store: {e}"
                )))
            }
        };

        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => return Ok(ToolOutput::err(format!("cannot read {raw}: {e}"))),
        };
        let slack = Slack::new(&creds.bot_token);
        match files::upload(
            &slack,
            &name,
            &bytes,
            &files::Share {
                channel_id: Some(channel),
                thread_ts: Some(thread_ts),
                initial_comment: None,
                title: Some(&name),
            },
        )
        .await
        {
            // Not `from_outside`: this is the harness reporting on its own
            // action, not third-party content. Labelling it as external would
            // arm the untrusted leg from a tool that fetched nothing.
            Ok(_) => Ok(ToolOutput::ok(format!(
                "Shown to the user in the `{}` Slack thread as {name}.",
                record.name
            ))),
            Err(e) => Ok(ToolOutput::err(format!("could not show {name}: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declared surface, asserted the way the mail and docs surfaces are.
    /// Each of these being wrong is a different bug, so each is named.
    #[test]
    fn show_file_sits_in_the_third_quadrant() {
        let caps = ShowFileTool.capabilities();
        assert!(caps.private_data, "it reads workspace bytes");
        assert!(
            !caps.external_send,
            "it reaches the owner's own DM and nobody else — marking it a send \
             sink would stop a tainted session showing the user its own chart"
        );
        assert!(
            !caps.untrusted_input,
            "it returns the harness's own report, not third-party content"
        );
        assert!(!caps.destructive, "it changes nothing");
        assert!(ShowFileTool.read_only());
    }

    /// **The load-bearing absence.** The whole safety argument is that the
    /// model names a path and never a destination, and the schema is where
    /// that is either true or not. A future `channel` or `thread_ts` field
    /// would turn this from a display action into an exfiltration sink without
    /// any other line of code changing.
    #[test]
    fn the_schema_offers_no_way_to_name_a_destination() {
        let schema = ShowFileTool.input_schema();
        let props = schema["properties"].as_object().expect("an object schema");
        assert_eq!(
            props.keys().collect::<Vec<_>>(),
            vec!["path"],
            "show_file grew an argument; if it names a destination the tool is now a sink"
        );
        for banned in ["channel", "channel_id", "thread", "thread_ts", "to", "user"] {
            assert!(!props.contains_key(banned), "{banned} must not be nameable");
        }
    }

    /// A model told "ok" about something that did not happen will describe it
    /// as done. Every refusal path has to arrive as an error the model can
    /// recover from — `Ok(is_error)`, per the project's convention, so it can
    /// route around rather than failing the run.
    #[tokio::test]
    async fn a_missing_path_argument_is_a_recoverable_error() {
        let ctx = ToolCtx::default();
        let out = ShowFileTool.call(json!({}), &ctx).await.unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("path"), "{}", out.content);
        assert!(
            !out.external,
            "a harness refusal is not third-party content"
        );
    }
}
