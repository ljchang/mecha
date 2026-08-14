//! Modal views: the `views.*` call and the Block Kit view builders.
//!
//! Transport only, like everything in this crate: a view here is a title, a
//! callback id, some blocks and an opaque `private_metadata` string. What a
//! modal *means* — which store the callback id resolves against, what the
//! typed field authorises — lives on the other side of the crate boundary,
//! exactly as it does for buttons.
//!
//! Two rules inherited from the rest of the crate:
//!
//! - **Every call goes through [`Slack::call`]**, so a refusal at HTTP 200 is
//!   an error here like everywhere else. `views.open` has one extra trap of
//!   its own: the `trigger_id` it needs expires in three seconds and may be
//!   used once, so the caller opens the modal before doing any other work.
//! - **Every builder truncates visibly rather than dropping.** Slack refuses
//!   an oversized view title with an error nobody relays to the person who
//!   tapped, and silently ignores nothing it should — but a cut this crate
//!   makes says so in the text, the same as every message builder.

use serde_json::{json, Value};

use crate::blocks;
use crate::error::SlackResult;
use crate::http::Slack;

/// Slack's documented ceilings for views, named like the message limits.
pub mod limits {
    /// A view's `title`, `submit` and `close` labels are plain text capped at
    /// 24 characters — far below the message caps, which is why they get
    /// their own names.
    pub const VIEW_TITLE: usize = 24;
    pub const VIEW_BUTTON: usize = 24;
    /// `private_metadata` is an opaque string the submission echoes back.
    pub const PRIVATE_METADATA: usize = 3000;
    /// A view holds up to 100 blocks.
    pub const BLOCKS_PER_VIEW: usize = 100;
    /// An input label is plain text, same ceiling as a button's.
    pub const INPUT_LABEL: usize = 75;
}

/// Open a modal against a `trigger_id`.
///
/// The trigger expires in three seconds and is single-use; open the view
/// before any other work. The refusal-at-200 check is [`Slack::call`]'s,
/// like every method in this crate.
pub async fn open(slack: &Slack, trigger_id: &str, view: Value) -> SlackResult<()> {
    let _: Value = slack
        .call(
            "views.open",
            json!({ "trigger_id": trigger_id, "view": view }),
        )
        .await?;
    Ok(())
}

/// A modal view. `callback_id` names the submission for the caller's parser;
/// `private_metadata` is an opaque string the submission echoes back — the
/// caller's correlation state, never something Slack or the person edits.
///
/// Blocks past the view ceiling are cut with a visible marker, exactly as
/// [`blocks::cap_blocks`] does for a message.
pub fn modal(
    title: &str,
    callback_id: &str,
    private_metadata: &str,
    blocks_in: Vec<Value>,
    submit: &str,
) -> Value {
    json!({
        "type": "modal",
        "callback_id": callback_id,
        "private_metadata": blocks::truncate(private_metadata, limits::PRIVATE_METADATA),
        "title": { "type": "plain_text", "text": blocks::truncate(title, limits::VIEW_TITLE) },
        "submit": { "type": "plain_text", "text": blocks::truncate(submit, limits::VIEW_BUTTON) },
        "close": { "type": "plain_text", "text": "Cancel" },
        "blocks": cap_view_blocks(blocks_in),
    })
}

/// A **required** text input. There is deliberately no `optional` parameter:
/// the one thing this crate's callers open modals for is a field that must be
/// filled, and a builder that could quietly make it optional would let a
/// required reason become an empty one. `max_length` is enforced by Slack in
/// the client, so the person is told at typing time rather than at submit.
pub fn required_text_input(
    block_id: &str,
    action_id: &str,
    label: &str,
    multiline: bool,
    max_length: usize,
) -> Value {
    json!({
        "type": "input",
        "block_id": block_id,
        "label": { "type": "plain_text", "text": blocks::truncate(label, limits::INPUT_LABEL) },
        "element": {
            "type": "plain_text_input",
            "action_id": action_id,
            "multiline": multiline,
            "max_length": max_length,
        },
    })
}

/// Keep a view within its block ceiling, saying what was dropped.
fn cap_view_blocks(mut blocks_in: Vec<Value>) -> Vec<Value> {
    if blocks_in.len() <= limits::BLOCKS_PER_VIEW {
        return blocks_in;
    }
    let dropped = blocks_in.len() - (limits::BLOCKS_PER_VIEW - 1);
    blocks_in.truncate(limits::BLOCKS_PER_VIEW - 1);
    blocks_in.push(blocks::context(&format!("_{dropped} more blocks not shown_")));
    blocks_in
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modal_carries_its_callback_id_and_metadata_and_is_typed_as_a_modal() {
        let v = modal(
            "Close request",
            "my_callback",
            r#"{"seq":5}"#,
            vec![blocks::section("why?")],
            "Close",
        );
        assert_eq!(v["type"], "modal");
        assert_eq!(v["callback_id"], "my_callback");
        assert_eq!(v["private_metadata"], r#"{"seq":5}"#);
        assert_eq!(v["submit"]["text"], "Close");
    }

    #[test]
    fn a_view_title_past_slacks_24_characters_is_cut_visibly_not_refused_silently() {
        // Slack refuses an oversized title with an API error the person who
        // tapped never sees — the modal simply fails to appear. The builder
        // cuts instead, and the cut says so.
        let v = modal("a".repeat(80).as_str(), "cb", "", vec![], "Go");
        let title = v["title"]["text"].as_str().unwrap();
        assert!(title.chars().count() <= limits::VIEW_TITLE, "{title}");
        assert!(title.contains('…'), "a silent cut is the bug: {title}");
    }

    #[test]
    fn oversized_metadata_is_cut_visibly_rather_than_bounced_by_slack() {
        let v = modal("t", "cb", &"m".repeat(5000), vec![], "Go");
        let meta = v["private_metadata"].as_str().unwrap();
        assert!(meta.chars().count() <= limits::PRIVATE_METADATA);
        assert!(meta.contains("truncated"), "{meta}");
    }

    #[test]
    fn a_view_past_the_block_ceiling_says_how_many_went_missing() {
        let many: Vec<Value> = (0..120).map(|i| blocks::section(&i.to_string())).collect();
        let v = modal("t", "cb", "", many, "Go");
        let rendered = v["blocks"].as_array().unwrap();
        assert_eq!(rendered.len(), limits::BLOCKS_PER_VIEW);
        let last = serde_json::to_string(rendered.last().unwrap()).unwrap();
        assert!(last.contains("more blocks not shown"), "{last}");
    }

    #[test]
    fn a_text_input_is_required_and_carries_its_length_cap() {
        let v = required_text_input("b1", "reason", "Why is this closed?", true, 500);
        assert_eq!(v["type"], "input");
        assert_eq!(v["block_id"], "b1");
        assert_eq!(v["element"]["action_id"], "reason");
        assert_eq!(v["element"]["max_length"], 500);
        assert_eq!(v["element"]["multiline"], true);
        // Required by construction: Slack treats an input without
        // `optional: true` as required, and the builder has no way to say
        // otherwise — a required reason must not be quietly optional.
        assert!(
            v.get("optional").is_none() && v["element"].get("optional").is_none(),
            "{v}"
        );
    }
}
