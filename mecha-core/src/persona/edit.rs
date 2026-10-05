//! A picture edit sent from the edit panel: what the harness tells the
//! persona when the owner's message is an instruction to the image model
//! rather than something said to it.
//!
//! The panel composes `Edit <picture>: <change>` (`web/src/lib/image-edit.js`)
//! and the page marks the turn as one (`SendBody::edit`), so the turn is known
//! by the door it came through, never by parsing the words. Without a note,
//! the persona answered an edit by describing a picture it has not seen and
//! then reusing the rest of its previous reply: across every persona chat, 11
//! of 34 edit replies were at least half copied from the five before
//! (2026-10-05), against 9 of 201 turns that drew nothing. Replayed on 22 edit
//! turns from two chats, three samples each, the recorded tool call kept and
//! only the reply after it re-sampled:
//!
//! | | at least half copied | longest copy | median length |
//! |---|---|---|---|
//! | no note | 37/64 | 206 words | 744 chars |
//! | this note | 16/66 | 126 words | 209 chars |
//!
//! Judged blind by the same model with the order swapped, 66–59 for the note
//! (a tie within noise); the edit prompts it wrote were unchanged (44/44
//! called with the picture, 38 vs 36 in the "Keep …" form).
//!
//! Typed turns only: on a call the panel's words go out as speech, and the
//! call note already says a picture reaches the owner unseen
//! (`persona::call`).

/// What the note opens with; registered as the harness's voice.
pub const EDIT_STEM: &str = "(From the harness: this message came from the picture edit panel";

/// The note, folded beside the owner's words on a turn the edit panel sent.
pub fn note() -> String {
    format!(
        "{EDIT_STEM}. Make the edit, then answer in a sentence or two, in your own \
voice. You have not seen the result, so don't describe the picture or retell the scene.)"
    )
}

/// Whether `text` is this note.
pub fn is_note(text: &str) -> bool {
    text.trim_start().starts_with(EDIT_STEM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_is_the_harness_speaking_never_the_owner() {
        let note = note();
        assert!(is_note(&note));
        assert!(crate::agent::is_harness_voice(&note));
        let mut m = crate::message::Message::user("Edit images/a.png: make the sky pink");
        m.content.push(crate::message::Block::Text { text: note });
        assert_eq!(
            crate::agent::owner_text(&m),
            "Edit images/a.png: make the sky pink"
        );
    }

    #[test]
    fn the_note_says_the_picture_is_unseen_and_closes_its_parenthesis() {
        let note = note();
        assert!(note.contains("You have not seen the result"), "{note}");
        assert!(note.contains("a sentence or two"), "{note}");
        assert!(note.ends_with(')'), "{note}");
        assert!(!crate::persona::variety::is_note(&note));
        assert!(!crate::persona::call::is_note(&note));
    }
}
