//! A persona on a call (PERSONA-DESIGN.md §11): what the harness tells the
//! persona when the owner starts speaking rather than typing.
//!
//! The assistant's chat prefixes its voice preamble onto the owner's words
//! (`voice::open_spoken_turn`), and every reader of those words has to strip
//! it again (`title::owner_turns` says so). A persona chat already speaks to
//! its model in separate harness blocks — the files, the memory, the Core
//! handed back — so the call's note is one more of those: its own block,
//! registered in `agent::is_harness_voice`, never drawn in the owner's bubble
//! and never in what the memory writer reads as the owner's.

/// What every call note opens with; registered as the harness's voice.
pub const CALL_STEM: &str = "(From the harness: the owner is now speaking to you on a call";

/// A picture made on a call appears on the owner's screen (the call screen
/// draws it; it covered the chat's copy until 2026-10-03). Never what it
/// shows: the persona has not seen it.
const PICTURES: &str = "A picture you make appears on the owner's screen during the call; you \
have not seen it, so say you sent it rather than what it shows.";

/// The note, folded beside the owner's words on the first spoken turn of a
/// stretch — the first of a call, and the first after any typed turn, since
/// the persona has been writing for a reader since.
///
/// The persona's own voice says who it is; this says only how a reply is
/// heard. Citations are named in words because a bracketed quotation read
/// aloud is noise, and the check still runs on whatever the reply says.
///
/// `streams`: the speech engine streams audio as it synthesises (Breeze), so
/// the length rule goes — it was a latency control for an engine that
/// speaks a sentence only once all of it is made (owner ruling, 2026-10-03:
/// "now we don't need things short"). Two fixed texts, each a stable block.
///
/// Both end on [`PICTURES`]: the call screen shows a picture the persona
/// makes, and the persona should know the owner can see it.
pub fn note(streams: bool) -> String {
    if streams {
        return format!(
            "{CALL_STEM}. What you write is spoken aloud by a text-to-speech voice, \
and the owner is listening, not reading. Answer as you would out loud, in \
your own manner. No markdown, lists, headings or bracketed citations; if a \
file says something, say which file in words. Write numbers, dates and \
times as they are spoken. {PICTURES})"
        );
    }
    format!(
        "{CALL_STEM}. What you write is spoken aloud by a text-to-speech voice, \
and the owner is listening, not reading. Answer as you would out loud, in \
your own manner: short sentences, the first one short, since speaking starts \
when it ends. No markdown, lists, headings or bracketed citations; if a file \
says something, say which file in words. Write numbers, dates and times as \
they are spoken. {PICTURES})"
    )
}

/// Whether `text` is a call note.
pub fn is_note(text: &str) -> bool {
    text.trim_start().starts_with(CALL_STEM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_is_the_harness_speaking_never_the_owner() {
        for streams in [false, true] {
            let note = note(streams);
            assert!(is_note(&note));
            assert!(crate::agent::is_harness_voice(&note));
            let mut m = crate::message::Message::user("how was the dig");
            m.content.push(crate::message::Block::Text { text: note });
            assert_eq!(crate::agent::owner_text(&m), "how was the dig");
        }
    }

    #[test]
    fn a_streaming_call_drops_the_length_rule_and_keeps_the_rest() {
        let streaming = note(true);
        assert!(!streaming.contains("short"), "{streaming}");
        assert!(!streaming.contains("speaking starts"));
        for kept in [
            "No markdown",
            "say which file in words",
            "as they are spoken",
        ] {
            assert!(streaming.contains(kept), "{kept}");
        }
        assert!(note(false).contains("the first one short"));
    }

    #[test]
    fn either_note_says_a_picture_reaches_the_owners_screen_unseen() {
        for streams in [false, true] {
            let note = note(streams);
            assert!(note.contains("appears on the owner's screen"), "{note}");
            assert!(note.contains("you have not seen it"), "{note}");
            assert!(
                note.ends_with(')'),
                "the note closes its parenthesis: {note}"
            );
        }
    }
}
