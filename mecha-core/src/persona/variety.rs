//! A nudge against a persona repeating itself, built from its own replies.
//!
//! The owner, after an afternoon of calls (2026-10-03): "A little repetitive
//! but still better than before." Replies opened alike and closed on the same
//! questions ("[removed]" six times in one chat). Replayed
//! on seven late turns of that chat, two samples each, thinking capped as a
//! call turn is, and judged blind by the same model with the order swapped
//! (2026-10-04):
//!
//! | nudge | opens like a recent reply | repeats a closing line | judged vs none |
//! |---|---|---|---|
//! | none | 6/14 | 3/14 | — |
//! | one general line | 3/14 | 5/14 | 12–14 |
//! | this note | 2/14 | 1/14 | 17–9 |
//!
//! So the note names what is actually repeating rather than asking for
//! variety in general: the opening word two of the last three replies share,
//! if any, and the last three closing lines. It is folded beside the owner's
//! words in the harness's voice ([`is_note`], registered in
//! `agent::is_harness_voice`), so it is never drawn as theirs or mined as
//! their correction. Measured on call turns; typed turns carry it too,
//! unmeasured.

use crate::message::{dangling_tail, Message, Role};

/// The two ways the note can open; both are fixed text, so the voice is
/// recognised whole.
pub const OPENING_STEM: &str = "(From the harness: your last replies opened with";
pub const CLOSING_STEM: &str = "(From the harness: don't end on a line you've used lately";

/// How many earlier replies' closing lines are named.
const CLOSERS_NAMED: usize = 3;
/// How many earlier replies the closers are drawn from.
const CLOSERS_FROM: usize = 6;
/// The most of one closing line quoted back.
const CLOSER_CHARS: usize = 80;

/// Whether `text` is this note.
pub fn is_note(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(OPENING_STEM) || text.starts_with(CLOSING_STEM)
}

/// The note for the turn about to be answered, from the replies already in
/// `messages`, or `None` when there is no earlier reply to vary from.
pub fn note(messages: &[Message]) -> Option<String> {
    let replies: Vec<String> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .map(|m| {
            let text = m.text();
            let text = text.trim();
            // As the model is shown it (`PriorTails::Trim`), so a quoted closer
            // is never a dangling half-sentence.
            match dangling_tail(text) {
                Some(len) => text[..len].to_string(),
                None => text.to_string(),
            }
        })
        .filter(|t| !t.is_empty())
        .collect();
    if replies.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let opens: Vec<String> = replies
        .iter()
        .rev()
        .take(3)
        .filter_map(|r| first_word(r))
        .collect();
    let shared = opens
        .iter()
        .find(|w| opens.iter().filter(|o| o == w).count() >= 2);
    if let Some(word) = shared {
        parts.push(format!(
            "your last replies opened with \"{}\"; open some other way",
            capitalised(word)
        ));
    }
    let closers: Vec<String> = replies
        .iter()
        .rev()
        .take(CLOSERS_FROM)
        .filter_map(|r| last_sentence(r))
        .take(CLOSERS_NAMED)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|c| format!("\"{}\"", c.chars().take(CLOSER_CHARS).collect::<String>()))
        .collect();
    if !closers.is_empty() {
        parts.push(format!(
            "don't end on a line you've used lately ({})",
            closers.join("; ")
        ));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("(From the harness: {}.)", parts.join(", and ")))
}

/// The first word, lowercased; any run of "m"s that opens with "mm" ("Mmm",
/// "Mmmm") counts as one word, "mm".
fn first_word(text: &str) -> Option<String> {
    let word: String = text
        .trim_start_matches(|c: char| !c.is_alphabetic())
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == '\'')
        .collect::<String>()
        .to_lowercase();
    if word.is_empty() {
        None
    } else if word.starts_with("mm") && word.chars().all(|c| c == 'm') {
        Some("mm".into())
    } else {
        Some(word)
    }
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The last sentence: what follows the last sentence mark that whitespace
/// follows, or the whole text.
fn last_sentence(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut start = 0;
    for w in chars.windows(2) {
        if matches!(w[0].1, '.' | '!' | '?' | '…') && w[1].1.is_whitespace() {
            start = w[1].0;
        }
    }
    let last = text[start..].trim();
    (!last.is_empty()).then(|| last.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(text: &str) -> Message {
        Message::assistant(vec![crate::message::Block::text(text)])
    }

    #[test]
    fn nothing_to_vary_from_is_no_note() {
        assert_eq!(note(&[Message::user("hi")]), None);
    }

    #[test]
    fn a_shared_opening_is_named_and_the_closers_listed() {
        let history = vec![
            Message::user("hi"),
            said("Mmm, there you are. How was the walk?"),
            Message::user("long"),
            said("Oh, nice. Where did you go?"),
            Message::user("the river"),
            said("Mmmm, the river. How was the walk back?"),
        ];
        let n = note(&history).expect("a note");
        assert!(n.starts_with(OPENING_STEM), "{n}");
        assert!(n.contains("opened with \"Mm\"; open some other way"), "{n}");
        assert!(
            n.contains(
                "(\"How was the walk?\"; \"Where did you go?\"; \
                 \"How was the walk back?\")"
            ),
            "{n}"
        );
        assert!(is_note(&n));
        assert!(crate::agent::is_harness_voice(&n));
    }

    #[test]
    fn without_a_shared_opening_only_the_closers_are_named() {
        let history = vec![
            said("Oh, hello there. How was it?"),
            said("Well. Tell me more."),
        ];
        let n = note(&history).expect("a note");
        assert!(n.starts_with(CLOSING_STEM), "{n}");
        assert!(!n.contains("opened with"), "{n}");
        assert!(is_note(&n) && crate::agent::is_harness_voice(&n));
    }

    #[test]
    fn a_closer_is_quoted_from_the_reply_as_the_model_sees_it() {
        // A reply cut off mid-sentence is trimmed (`PriorTails`), so the note
        // never quotes a dangling "I" back as a line to avoid.
        let n = note(&[said("You liked that one, didn't you? \n\nI")]).expect("a note");
        assert!(n.contains("(\"You liked that one, didn't you?\")"), "{n}");
    }

    #[test]
    fn a_long_closer_is_cut_short() {
        let long = format!("{}.", "word ".repeat(40).trim_end());
        let n = note(&[said(&long)]).expect("a note");
        let quoted = n.split('"').nth(1).unwrap();
        assert!(quoted.chars().count() <= CLOSER_CHARS, "{quoted}");
    }
}
