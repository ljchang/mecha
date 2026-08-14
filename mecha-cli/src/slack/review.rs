//! Per-thread release policy: what happens when this thread's runs stage
//! drafts. The Slack counterpart of the TUI's `/review now|later|auto`.
//!
//! **Set by an explicit owner gesture only — the `review` command word — and
//! never inferred from prompt or message text.** The rule is `/review`'s,
//! quoted because it is load-bearing: *release policy must not be decidable
//! by anything sharing a context window with third-party text.* A command
//! word is an owner's keystroke that short-circuits before the text can
//! become a prompt (the same precedence `doctor` gets), so no fetched page,
//! no mail body and no model output can utter it into effect; anything that
//! is not exactly the command word falls through and is just a message.
//!
//! **Session-scoped, expiring with the thread's in-memory state.** The
//! setting lives in the connector's process and is deliberately never written
//! to the thread record — the same eviction that orphans a mid-flight run on
//! restart clears every review mode with it. That is the owner decision of
//! 2026-08-14 (SLACK-ACTIONS-DESIGN §4): not an *unbounded* Always, a mode
//! that dies with the state that watched it get set. A connector restart
//! resets every thread to `now`, which is the safe direction: cards for
//! everything.
//!
//! **Tainted drafts never auto-release, regardless of mode.** The approval a
//! mode represents predates whatever armed the taint, so it authorises
//! nothing about what came after — the TUI's rule, unchanged here.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewMode {
    /// Drafts a run stages are carded for review when it finishes. The
    /// default, and what every thread returns to on restart.
    Now,
    /// Drafts wait in the outbox; the completion message says how many.
    Later,
    /// Untainted drafts staged by this thread's runs release when it
    /// finishes; tainted ones still stop for a card, and an errored or
    /// stopped run releases nothing.
    Auto,
}

impl ReviewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewMode::Now => "now",
            ReviewMode::Later => "later",
            ReviewMode::Auto => "auto",
        }
    }

    /// One line on what the mode does, shown when it is set or asked about.
    pub fn describe(self) -> &'static str {
        match self {
            ReviewMode::Now => "drafts this thread's runs stage are carded here for review",
            ReviewMode::Later => {
                "drafts wait in the outbox — the completion message says how many"
            }
            ReviewMode::Auto => {
                "untainted drafts this thread's runs stage are released when the run \
                 finishes; tainted drafts still stop for review. Expires when the \
                 connector restarts"
            }
        }
    }

    fn parse(s: &str) -> Option<ReviewMode> {
        match s.to_ascii_lowercase().as_str() {
            "now" => Some(ReviewMode::Now),
            "later" => Some(ReviewMode::Later),
            "auto" => Some(ReviewMode::Auto),
            _ => None,
        }
    }
}

/// Who set a thread's mode — the attribution every auto-released item's
/// ledger row carries as its `user_id`; the *when* of each release is the
/// ledger row's own stamp.
#[derive(Debug, Clone)]
pub struct Setting {
    pub mode: ReviewMode,
    /// The Slack user id from the signed payload of the message that set it.
    pub set_by: String,
}

/// The `review` command word: `review` alone asks, `review now|later|auto`
/// sets. Matched like `doctor` — trimmed, case-insensitive, and **nothing
/// longer**: "review the design doc" must reach the model, not the policy.
///
/// Returns `None` when the text is not the command word at all,
/// `Some(None)` for the bare question, `Some(Some(mode))` for a setting.
pub fn command(text: &str) -> Option<Option<ReviewMode>> {
    let mut words = text.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("review") {
        return None;
    }
    match (words.next(), words.next()) {
        (None, _) => Some(None),
        (Some(mode), None) => ReviewMode::parse(mode).map(Some),
        // A third word means prose, and prose is a prompt.
        _ => None,
    }
}

/// Whether one staged item releases without a card under a mode. The tainted
/// exclusion lives here, in a pure function, so it is testable and cannot be
/// forgotten at a call site.
pub fn auto_releases(mode: ReviewMode, tainted: bool) -> bool {
    mode == ReviewMode::Auto && !tainted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_set_by_the_exact_command_word_and_nothing_longer() {
        assert_eq!(command("review auto"), Some(Some(ReviewMode::Auto)));
        assert_eq!(command("  REVIEW Now  "), Some(Some(ReviewMode::Now)));
        assert_eq!(command("review later"), Some(Some(ReviewMode::Later)));
        assert_eq!(command("review"), Some(None), "the bare word asks");

        // Release policy must not be decidable by anything sharing a context
        // window with third-party text: prose that merely contains the words
        // is a prompt, never a gesture — including anything a model or a
        // fetched page could get echoed into the thread.
        for prose in [
            "please review auto tomorrow",
            "review auto now",
            "set review to auto",
            "review the design doc",
            "auto",
            "reviewauto",
            "",
        ] {
            assert_eq!(command(prose), None, "{prose:?} must not set a mode");
        }
        // An unknown mode word is not guessed at.
        assert_eq!(command("review always"), None);
    }

    #[test]
    fn tainted_drafts_never_auto_release_whatever_the_mode() {
        for mode in [ReviewMode::Now, ReviewMode::Later, ReviewMode::Auto] {
            assert!(
                !auto_releases(mode, true),
                "{mode:?} must not release a tainted draft"
            );
        }
        assert!(auto_releases(ReviewMode::Auto, false));
        assert!(!auto_releases(ReviewMode::Now, false));
        assert!(!auto_releases(ReviewMode::Later, false));
    }

    #[test]
    fn every_mode_names_itself_and_what_it_does() {
        for mode in [ReviewMode::Now, ReviewMode::Later, ReviewMode::Auto] {
            assert_eq!(command(&format!("review {}", mode.as_str())), Some(Some(mode)));
            assert!(!mode.describe().is_empty());
        }
    }
}
