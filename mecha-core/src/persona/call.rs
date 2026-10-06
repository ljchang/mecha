//! A persona on a call (PERSONA-DESIGN.md §11): what the harness tells the
//! persona when the owner starts speaking rather than typing.
//!
//! The assistant's chat prefixes its voice preamble onto the owner's words
//! (`voice::open_spoken_turn`), and every reader of those words has to strip
//! it again (`title::owner_turns` says so). A persona's call note is one of
//! the run's notes instead (`RunContext::notes`), so it never enters a stored
//! message. Its stem stays registered in `agent::is_harness_voice` for the
//! chats recorded before, where it sits beside the owner's words.

/// What every call note opens with; registered as the harness's voice.
pub const CALL_STEM: &str = "(From the harness: the owner is now speaking to you on a call";

/// A picture made on a call appears on the owner's screen (the call screen
/// draws it; it covered the chat's copy until 2026-10-03). Never what it
/// shows: the persona has not seen it.
const PICTURES: &str = "A picture you make appears on the owner's screen during the call; you \
have not seen it, so say you sent it rather than what it shows.";

/// The note for a spoken turn: one of the run's notes (`RunContext::notes`,
/// PERSONA-CONTEXT-DESIGN.md §5.1), so it is on every spoken turn and in no
/// stored message. It used to be folded beside the owner's words on the
/// first spoken turn of a stretch and then kept, re-sent with everything
/// after it.
///
/// The persona's own voice says who it is; this says only how a reply is
/// heard. Citations are named in words because a bracketed quotation read
/// aloud is noise, and the check still runs on whatever the reply says.
///
/// `streams`: the speech engine streams audio as it synthesises (Breeze), so
/// the length rule goes — it was a latency control for an engine that
/// speaks a sentence only once all of it is made (owner ruling, 2026-10-03:
/// "now we don't need things short").
///
/// `pictures`: the persona has `image_generate`. Only then does the note end
/// on [`PICTURES`] (§5.6, guidance travels with capability). Sent to a
/// persona with no image tool, the sentence was followed: on a call it said
/// it had sent a picture, and no tool was called (2026-10-05).
pub fn note(streams: bool, pictures: bool) -> String {
    let length = if streams {
        ""
    } else {
        " short sentences, the first one short, since speaking starts when it ends."
    };
    let manner = if streams {
        "Answer as you would out loud, in your own manner."
    } else {
        "Answer as you would out loud, in your own manner:"
    };
    let pictures = if pictures {
        format!(" {PICTURES}")
    } else {
        String::new()
    };
    format!(
        "{CALL_STEM}. What you write is spoken aloud by a text-to-speech voice, \
and the owner is listening, not reading. {manner}{length} No markdown, lists, \
headings or bracketed citations; if a file says something, say which file in \
words. Write numbers, dates and times as they are spoken.{pictures})"
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
        for (streams, pictures) in [(false, false), (true, true)] {
            let note = note(streams, pictures);
            assert!(is_note(&note));
            assert!(crate::agent::is_harness_voice(&note));
            let mut m = crate::message::Message::user("how was the dig");
            m.content.push(crate::message::Block::Text { text: note });
            assert_eq!(crate::agent::owner_text(&m), "how was the dig");
        }
    }

    #[test]
    fn a_streaming_call_drops_the_length_rule_and_keeps_the_rest() {
        let streaming = note(true, true);
        assert!(!streaming.contains("short"), "{streaming}");
        assert!(!streaming.contains("speaking starts"));
        for kept in [
            "No markdown",
            "say which file in words",
            "as they are spoken",
        ] {
            assert!(streaming.contains(kept), "{kept}");
        }
        assert!(note(false, true).contains("the first one short"));
    }

    #[test]
    fn only_a_persona_that_draws_is_told_its_pictures_reach_the_screen() {
        for streams in [false, true] {
            let drawing = note(streams, true);
            assert!(
                drawing.contains("appears on the owner's screen"),
                "{drawing}"
            );
            assert!(drawing.contains("you have not seen it"), "{drawing}");
            let not = note(streams, false);
            assert!(!not.contains("picture"), "{not}");
            for n in [drawing, not] {
                assert!(n.ends_with(".)"), "the note closes its parenthesis: {n}");
                assert!(!n.contains("  "), "{n}");
            }
        }
    }
}
