//! What a typed or spoken capture says about *when*, without rewriting what
//! it says.
//!
//! **B2, and the whole of it is two rules.** The parse happens here rather
//! than in a model — a capture that costs a model call is a capture nobody
//! uses, and a model that rewrites what you typed is worse than one that does
//! nothing. And it follows Things rather than Todoist on the one thing the
//! surveyed apps disagree about: the token is *reported* so a surface can show
//! a chip you can dismiss, and **the name is returned untouched**. A capture
//! surface that silently edits what you said is the wrong default for a store
//! whose job is to hold your own intentions verbatim.
//!
//! **It detects; it does not resolve.** The span it finds is handed to the
//! graph's `gtd::parse_due` as the `--due` argument, which already owns what
//! `+3d` means. Resolving dates here would be a second date parser in a second
//! repository, drifting against the one that actually writes the field — the
//! divergence this project refuses everywhere else. The consequence worth
//! knowing: this only ever emits spellings that parser accepts.
//!
//! **There is no time of day, and that is the store's shape rather than an
//! omission.** `due_at` is written `%Y-%m-%d`. So *"call Bob tomorrow at 3"*
//! yields `tomorrow`, and *"at 3"* stays in the name where the owner put it —
//! the honest outcome, and the one consistent with keeping names literal.

/// A `when` found in a capture: what to pass to `--due`, and the exact span of
/// the original it was read from.
///
/// The span is what lets a surface render a dismissable chip *and* show which
/// words produced it. Byte offsets into the input, so a caller can slice
/// without re-searching and without assuming the token is unique — "tomorrow,
/// and tell Bob tomorrow" has two, and only the one that was matched may be
/// highlighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct When {
    /// The spelling to hand to the graph, always one `parse_due` accepts.
    pub due: String,
    /// What the owner actually wrote, verbatim — the chip's label.
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Weekday names are deliberately **absent**, and so is "next week".
///
/// `parse_due` accepts `today`, `tomorrow`, `+Nd` and `YYYY-MM-DD` and nothing
/// else, so a "friday" detected here could only be honoured by resolving it to
/// a date locally — which is the second parser this module exists not to be.
/// Detecting a token the store cannot accept would produce a chip that lies,
/// or an error on a capture that looked fine. Narrow and honest beats wide and
/// wrong; widening this means widening `parse_due` first, in the repo that
/// owns the meaning.
const RELATIVE_DAYS: &[(&str, u32)] = &[("tomorrow", 1), ("tmrw", 1), ("overmorrow", 2)];

/// Find the one `when` in a capture, or none.
///
/// **The first match wins and there is deliberately no second.** A capture is
/// one sentence with one deadline; a parser that collected several would have
/// to decide which the task is due on, which is a guess with the owner's own
/// words available to ask about instead. Scanning left to right matches how
/// the sentence was said.
pub fn find_when(input: &str) -> Option<When> {
    let lower = input.to_lowercase();

    // Longest-first, so "the day after tomorrow" is not read as "tomorrow"
    // with three stray words in front of it — the classic substring bug, and
    // the one that silently sets a date a day early.
    let mut best: Option<When> = None;
    let mut consider = |due: String, start: usize, end: usize| {
        let cand = When {
            due,
            text: input[start..end].to_string(),
            start,
            end,
        };
        // **Earlier wins; at the same position, longer wins.** Spelled out
        // rather than as a tuple comparison, which is how the first cut got
        // it backwards: `(start, len)` orders *ascending* on start, so `>=`
        // preferred the match further into the sentence. That read "the day
        // after tomorrow" as "tomorrow" and set the date a day early —
        // silently, on the one field where being off by one matters.
        let better = match &best {
            None => true,
            Some(b) if cand.start < b.start => true,
            Some(b) if cand.start == b.start => (cand.end - cand.start) > (b.end - b.start),
            Some(_) => false,
        };
        if better {
            best = Some(cand);
        }
    };

    if let Some(m) = find_word(&lower, "the day after tomorrow") {
        consider("+2d".into(), m.0, m.1);
    }
    if let Some(m) = find_word(&lower, "today") {
        consider("today".into(), m.0, m.1);
    }
    if let Some(m) = find_word(&lower, "tonight") {
        consider("today".into(), m.0, m.1);
    }
    for (word, days) in RELATIVE_DAYS {
        if let Some(m) = find_word(&lower, word) {
            let due = if *days == 1 {
                "tomorrow".to_string()
            } else {
                format!("+{days}d")
            };
            consider(due, m.0, m.1);
        }
    }
    if let Some(m) = find_in_n_days(&lower) {
        consider(m.2, m.0, m.1);
    }
    if let Some(m) = find_iso_date(&lower) {
        consider(input[m.0..m.1].to_string(), m.0, m.1);
    }
    best
}

/// Whole-word search, so "todays" and "tomorrowland" do not match.
///
/// The boundary test is "not alphanumeric" rather than whitespace: a capture
/// ends sentences with punctuation, and *"call Bob tomorrow."* is the common
/// case rather than the edge one.
fn find_word(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(i) = haystack[from..].find(needle) {
        let start = from + i;
        let end = start + needle.len();
        let before_ok = start == 0
            || !haystack[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        let after_ok = end == haystack.len()
            || !haystack[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric());
        if before_ok && after_ok {
            return Some((start, end));
        }
        from = end;
    }
    None
}

/// `in 3 days` / `in 2 weeks` → the `+Nd` the graph already understands.
fn find_in_n_days(haystack: &str) -> Option<(usize, usize, String)> {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(i) = haystack[from..].find("in ") {
        let start = from + i;
        let before_ok = start == 0 || !(bytes[start - 1] as char).is_alphanumeric();
        let rest = &haystack[start + 3..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if before_ok && !digits.is_empty() {
            let after = rest[digits.len()..].trim_start();
            let gap = rest.len() - digits.len() - after.len();
            for (unit, mult) in [("days", 1u32), ("day", 1), ("weeks", 7), ("week", 7)] {
                if let Some(m) = after.strip_prefix(unit) {
                    if m.is_empty() || !m.starts_with(|c: char| c.is_alphanumeric()) {
                        let n: u32 = digits.parse().ok()?;
                        let end = start + 3 + digits.len() + gap + unit.len();
                        return Some((start, end, format!("+{}d", n * mult)));
                    }
                }
            }
        }
        from = start + 3;
    }
    None
}

/// A bare `YYYY-MM-DD`, which `parse_due` takes verbatim.
fn find_iso_date(haystack: &str) -> Option<(usize, usize)> {
    let b = haystack.as_bytes();
    for start in 0..b.len().saturating_sub(9) {
        let w = &haystack[start..start + 10];
        let ok = w.as_bytes().iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        });
        if !ok {
            continue;
        }
        let before_ok = start == 0 || !(b[start - 1] as char).is_alphanumeric();
        let end = start + 10;
        let after_ok = end == b.len() || !(b[end] as char).is_alphanumeric();
        if before_ok && after_ok {
            return Some((start, end));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due(input: &str) -> Option<String> {
        find_when(input).map(|w| w.due)
    }

    /// **The name is never rewritten**, which is the half of B2 that decides
    /// whether this is a capture surface or an editor. Things keeps the name
    /// literal; Todoist strips the token out of it. The token is *reported*
    /// with its span so a chip can be drawn and dismissed, and the caller is
    /// handed the original untouched.
    #[test]
    fn the_owners_words_come_back_exactly_as_typed() {
        let input = "Call Bob tomorrow about the grant";
        let w = find_when(input).unwrap();
        assert_eq!(w.due, "tomorrow");
        assert_eq!(
            &input[w.start..w.end],
            "tomorrow",
            "the span points at the real bytes"
        );
        assert_eq!(w.text, "tomorrow");
        // Nothing here returns a rewritten name, and there is deliberately no
        // function that does.
    }

    /// **The store has no time of day**, so `at 3` is not a failure to parse
    /// — it is a thing with nowhere to go. It stays in the name, where the
    /// owner put it, and the chip claims only the date.
    #[test]
    fn a_time_of_day_is_left_in_the_name_because_the_board_cannot_hold_one() {
        let input = "call Bob tomorrow at 3";
        let w = find_when(input).unwrap();
        assert_eq!(w.due, "tomorrow");
        assert_eq!(w.text, "tomorrow", "the chip does not claim the time");
        assert!(
            input[w.end..].contains("at 3"),
            "and the time survives in the name"
        );
    }

    /// The substring bug this ordering exists to prevent: "the day after
    /// tomorrow" contains "tomorrow", and matching the shorter one sets a
    /// date a day early — silently, on a task with a deadline.
    #[test]
    fn the_day_after_tomorrow_is_not_tomorrow() {
        assert_eq!(due("ship it the day after tomorrow"), Some("+2d".into()));
        assert_eq!(due("ship it tomorrow"), Some("tomorrow".into()));
    }

    /// Whole words only. A parser that fired on any substring would date a
    /// task from the middle of an ordinary noun.
    #[test]
    fn a_word_that_merely_contains_one_is_not_one() {
        assert_eq!(due("visit tomorrowland"), None);
        assert_eq!(due("read the todays paper archive"), None);
        assert_eq!(due("book a stay in 3 daysworth of rooms"), None);
    }

    /// Punctuation is the common case in a real capture, not the edge one.
    #[test]
    fn a_sentence_that_ends_in_punctuation_still_parses() {
        assert_eq!(due("call Bob tomorrow."), Some("tomorrow".into()));
        assert_eq!(due("today: sort the inbox"), Some("today".into()));
    }

    /// Only spellings `gtd::parse_due` accepts are ever emitted — this
    /// detects, the graph resolves. `in 2 weeks` becomes `+14d` rather than a
    /// date computed here.
    #[test]
    fn everything_emitted_is_something_the_graph_already_understands() {
        for (input, expect) in [
            ("do it today", "today"),
            ("do it tonight", "today"),
            ("do it tomorrow", "tomorrow"),
            ("do it in 3 days", "+3d"),
            ("do it in 1 day", "+1d"),
            ("do it in 2 weeks", "+14d"),
            ("do it 2026-09-05", "2026-09-05"),
        ] {
            let got = due(input).unwrap_or_else(|| panic!("no when in {input:?}"));
            assert_eq!(got, expect, "for {input:?}");
            // The contract with the other repo, asserted rather than assumed.
            assert!(
                got == "today"
                    || got == "tomorrow"
                    || (got.starts_with('+') && got.ends_with('d'))
                    || got.len() == 10,
                "{got:?} is not a spelling parse_due accepts"
            );
        }
    }

    /// A weekday is **not** detected, deliberately. `parse_due` cannot take
    /// one, so honouring it would mean resolving a date here — the second
    /// parser this module exists not to be — and detecting it without
    /// honouring it would draw a chip that lies.
    #[test]
    fn a_weekday_is_not_detected_because_the_store_could_not_take_it() {
        assert_eq!(due("call Bob on friday"), None);
        assert_eq!(due("call Bob next week"), None);
    }

    /// One capture, one deadline. Two would make the parser choose, and the
    /// owner is right there to be asked instead.
    #[test]
    fn the_first_when_wins() {
        let w = find_when("tomorrow tell Bob about today").unwrap();
        assert_eq!(w.due, "tomorrow");
        assert_eq!(w.start, 0);
    }

    #[test]
    fn a_capture_with_no_date_says_so() {
        assert_eq!(due("call Bob about the grant"), None);
        assert_eq!(due(""), None);
    }
}
