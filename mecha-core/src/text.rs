//! One byte-safe truncation, shared rather than copied a fourth time.
//!
//! `&s[..n]` on a raw byte index panics the instant `n` lands inside a
//! multi-byte character, and the same `&text[start..=end.min(start + 400)]`
//! slice — cutting an unparseable model reply down for an error message —
//! was written at three call sites (`frontdoor::extract`,
//! `mail_triage::classify_with`, `appraisal::parse_appraiser_verdict`)
//! before any of them guarded it. One of those three reads a stranger's free
//! text through the front door's extractor: an em-dash or a curly quote
//! landing on that offset in a malformed extraction aborted the process, in
//! the module whose whole job is being the safe boundary for outside input.

/// The largest byte index `<= max` that lands on a char boundary of `s`.
pub(crate) fn char_boundary_at_or_before(s: &str, max: usize) -> usize {
    (0..=max.min(s.len()))
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boundary_already_on_a_char_falls_through_unchanged() {
        assert_eq!(char_boundary_at_or_before("abcdef", 3), 3);
    }

    #[test]
    fn a_cutoff_mid_character_steps_back_to_the_boundary_before_it() {
        let s = "ab—cd"; // '—' is 3 bytes: indices 2,3,4
        assert!(!s.is_char_boundary(3));
        assert_eq!(char_boundary_at_or_before(s, 3), 2);
    }

    #[test]
    fn a_cutoff_past_the_end_clamps_to_the_string_s_length() {
        assert_eq!(char_boundary_at_or_before("abc", 50), 3);
    }
}
