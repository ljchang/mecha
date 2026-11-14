//! Narrow inline loop voices supported by the ordinary recording profiles.
//! Recognition never replaces owner matching, placement or history validation.
use std::collections::BTreeSet;

pub(super) fn empty_retry(text: &str) -> bool {
    text == crate::agent::EMPTY_TURN_NUDGE
}

pub(super) fn supported(
    text: &str,
    observed_tools: &BTreeSet<String>,
    max_arguments: usize,
) -> bool {
    if empty_retry(text) {
        return true;
    }
    let escapes = crate::boredom::Escapes { delegate: None };
    observed_tools.iter().any(|tool| {
        use crate::boredom::Rung;
        text == Rung::Change.notice(tool, &escapes)
            || text == Rung::Delegate.notice(tool, &escapes)
            || (1..=max_arguments).any(|arguments| {
                text == Rung::Guessing { arguments }.notice(tool, &escapes)
                    || text == Rung::Rewording { arguments }.notice(tool, &escapes)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_notices_require_exact_bytes_and_observed_tools() {
        use crate::boredom::{Escapes, Rung};
        let tools = BTreeSet::from(["fs_read".to_string()]);
        let notice = Rung::Change.notice("fs_read", &Escapes::default());
        assert!(supported(&notice, &tools, 3));
        assert!(!supported(
            &format!("{notice} additional instruction"),
            &tools,
            3
        ));
        assert!(!supported(&format!(" {notice}"), &tools, 3));
        assert!(!supported(&notice, &BTreeSet::new(), 3));
        let guesses = Rung::Guessing { arguments: 4 }.notice("fs_read", &Escapes::default());
        assert!(!supported(&guesses, &tools, 3));
        assert!(supported(&guesses, &tools, 4));
        assert!(empty_retry(crate::agent::EMPTY_TURN_NUDGE));
        assert!(!empty_retry(&notice));
    }
}
