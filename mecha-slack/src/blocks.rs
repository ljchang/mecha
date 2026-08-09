//! The Block Kit shapes this transport actually posts, and their limits.
//!
//! Every builder here **truncates rather than drops**. Slack silently discards
//! blocks past its cap and silently removes oversized images, and a surface
//! that loses content without saying so is the failure this project keeps
//! finding in other people's tools: the human reads a complete-looking message
//! that is missing the part that mattered. Where something is cut, the cut is
//! visible in the text.

use serde_json::{json, Value};

/// Slack's documented ceilings, named so a call site reads as a decision
/// rather than as a magic number.
pub mod limits {
    pub const SECTION_TEXT: usize = 3000;
    pub const BUTTON_TEXT: usize = 75;
    pub const BUTTON_VALUE: usize = 2000;
    pub const ALT_TEXT: usize = 2000;
    pub const BLOCKS_PER_MESSAGE: usize = 50;
    /// `task_update` and `plan_update` streaming chunks.
    pub const TASK_TEXT: usize = 256;
    /// One `chat.appendStream` call's worth of markdown.
    pub const STREAM_MARKDOWN: usize = 12_000;
}

/// Cut to `max` characters, saying so. Operates on characters rather than
/// bytes, because Slack counts characters and a byte-wise cut can also split a
/// multi-byte character in half.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    const MARK: &str = " […truncated]";
    let keep = max.saturating_sub(MARK.chars().count());
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(MARK);
    out
}

/// Keep a message within Slack's block ceiling, saying what was dropped rather
/// than letting Slack discard the tail with a warning nobody reads.
pub fn cap_blocks(mut blocks: Vec<Value>) -> Vec<Value> {
    if blocks.len() <= limits::BLOCKS_PER_MESSAGE {
        return blocks;
    }
    let dropped = blocks.len() - (limits::BLOCKS_PER_MESSAGE - 1);
    blocks.truncate(limits::BLOCKS_PER_MESSAGE - 1);
    blocks.push(context(&format!("_{dropped} more blocks not shown_")));
    blocks
}

pub fn section(text: &str) -> Value {
    json!({
        "type": "section",
        "text": { "type": "mrkdwn", "text": truncate(text, limits::SECTION_TEXT) }
    })
}

pub fn context(text: &str) -> Value {
    json!({
        "type": "context",
        "elements": [{ "type": "mrkdwn", "text": truncate(text, limits::SECTION_TEXT) }]
    })
}

pub fn divider() -> Value {
    json!({ "type": "divider" })
}

/// A button. `style` is `None`, `Some("primary")` or `Some("danger")`.
pub fn button(action_id: &str, text: &str, value: &str, style: Option<&str>) -> Value {
    let mut b = json!({
        "type": "button",
        "action_id": action_id,
        "text": { "type": "plain_text", "text": truncate(text, limits::BUTTON_TEXT) },
        "value": truncate(value, limits::BUTTON_VALUE),
    });
    if let Some(style) = style {
        b["style"] = json!(style);
    }
    b
}

pub fn actions(elements: Vec<Value>) -> Value {
    json!({ "type": "actions", "elements": elements })
}

/// A code block with syntax highlighting. Preferred over a mrkdwn section for
/// anything code-shaped: `rich_text_preformatted` takes a language and has no
/// documented character cap, where a section block is bound to 3,000.
pub fn preformatted(code: &str, language: Option<&str>) -> Value {
    let mut inner = json!({
        "type": "rich_text_preformatted",
        "elements": [{ "type": "text", "text": code }]
    });
    if let Some(language) = language {
        inner["language"] = json!(language);
    }
    json!({ "type": "rich_text", "elements": [inner] })
}

/// An image that was uploaded privately, referenced by file id.
///
/// This is the shape that lets a rendered chart appear inline in a thread
/// **without the file ever being made public** — the upload names no channel,
/// so nothing was shared, and this block is the only thing that reveals it.
/// The documented footgun is that the upload and the post must use the same
/// token, or the app cannot display its own file.
pub fn image_from_file(file_id: &str, alt_text: &str) -> Value {
    json!({
        "type": "image",
        "alt_text": truncate(alt_text, limits::ALT_TEXT),
        "slack_file": { "id": file_id }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_visible_rather_than_silent() {
        let cut = truncate(&"x".repeat(100), 20);
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.contains("truncated"), "a silent cut is the bug: {cut}");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // Each of these is three bytes and one character; a byte-wise cut
        // would both over-trim and risk splitting one in half.
        let text = "日".repeat(50);
        let cut = truncate(&text, 40);
        assert_eq!(cut.chars().count(), 40);
    }

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(truncate("hello", 3000), "hello");
    }

    #[test]
    fn capping_blocks_says_how_many_went_missing() {
        let blocks: Vec<Value> = (0..60).map(|i| section(&i.to_string())).collect();
        let capped = cap_blocks(blocks);
        assert_eq!(capped.len(), limits::BLOCKS_PER_MESSAGE);
        let last = serde_json::to_string(capped.last().unwrap()).unwrap();
        assert!(last.contains("11 more blocks"), "{last}");
    }

    #[test]
    fn a_message_at_the_limit_is_not_rewritten() {
        let blocks: Vec<Value> = (0..limits::BLOCKS_PER_MESSAGE)
            .map(|i| section(&i.to_string()))
            .collect();
        assert_eq!(cap_blocks(blocks).len(), limits::BLOCKS_PER_MESSAGE);
    }

    #[test]
    fn a_button_carries_its_correlation_value_but_authorises_nothing() {
        // The value is a correlation id and is deliberately not trusted at the
        // other end; the gate is `payload.user.id`. This test exists to keep
        // the field small and boring.
        let b = button("approve", "Approve", "call-123", Some("primary"));
        assert_eq!(b["value"], "call-123");
        assert_eq!(b["style"], "primary");
    }

    #[test]
    fn an_image_references_a_private_file_rather_than_a_url() {
        let img = image_from_file("F123", "a chart");
        assert_eq!(img["slack_file"]["id"], "F123");
        assert!(
            img.get("image_url").is_none(),
            "a public URL would defeat the point of a private upload"
        );
    }
}
