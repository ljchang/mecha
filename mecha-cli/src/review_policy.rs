//! One release policy, one encoding.
//!
//! Both review surfaces — the TUI's `/review now|later|auto` and the Slack
//! connector's `review` command word — decide the same question: what happens
//! to the drafts a finishing run staged. For a while each surface carried its
//! own copy of the mode enum and its own copy of the release rule, which is
//! exactly how the two drift: the TUI knew that an errored or early-stopped
//! run releases nothing, and the Slack copy did not.
//!
//! So the policy lives here, once, as a sibling of both front-ends — `slack/`
//! must not import `tui/` and vice versa — and the whole decision is
//! [`auto_releases`], a pure function. Two exclusions live *in* the function
//! rather than at call sites, so no surface can forget either:
//!
//! - **Tainted drafts never auto-release.** The approval a mode represents
//!   was given before the run read whatever armed the taint, so it covers
//!   nothing drafted after.
//! - **An errored or early-stopped run releases nothing.** A cancelled run's
//!   drafts are half a thought; its untainted drafts still surface for
//!   review — they just never release themselves.
//!
//! What deliberately does *not* live here: how a mode is uttered (a slash
//! command, a command word) and where held drafts are shown (a modal, a
//! card). Those are each surface's own.

/// What happens when a run this session started finishes having staged
/// outbox items.
///
/// Set by an explicit gesture only, never inferred from prompt or message
/// text: release policy must not be decidable by anything sharing a context
/// window with third-party text. Only the items the finishing run itself
/// staged are in scope — the rest of the queue is untouched by every mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewMode {
    /// The finishing run's drafts are put in front of the person at once.
    /// The default: a draft you just asked for is a draft you are about to
    /// read.
    #[default]
    Now,
    /// Drafts wait in the outbox; the surface says how many.
    Later,
    /// Untainted drafts release when the run finishes cleanly; everything
    /// else stops for review.
    Auto,
}

impl ReviewMode {
    pub fn name(self) -> &'static str {
        match self {
            ReviewMode::Now => "now",
            ReviewMode::Later => "later",
            ReviewMode::Auto => "auto",
        }
    }

    /// The mode word, parsed exactly: `now`, `later` or `auto`, any case,
    /// nothing else. Both surfaces refuse to guess at a typo, because
    /// silently keeping the old mode leaves someone believing their drafts
    /// now release themselves when they do not — or worse, vice versa.
    pub fn parse(s: &str) -> Option<ReviewMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "now" => Some(ReviewMode::Now),
            "later" => Some(ReviewMode::Later),
            "auto" => Some(ReviewMode::Auto),
            _ => None,
        }
    }

    /// One line on what the mode does, shown when it is set or asked about —
    /// the same line on every surface, because two descriptions of one policy
    /// would each be read as the whole truth.
    pub fn describe(self) -> &'static str {
        match self {
            ReviewMode::Now => "drafts a run stages are offered for review when it finishes",
            ReviewMode::Later => "drafts wait in the outbox until you get to them",
            ReviewMode::Auto => {
                "untainted drafts a run stages release when it finishes cleanly — \
                 tainted drafts still stop for review, and an errored or \
                 early-stopped run releases nothing"
            }
        }
    }
}

/// Whether one staged item releases without review under a mode. The whole
/// policy, in one testable place:
///
/// - `tainted` — drafted while the trifecta was armed. Never released by any
///   mode: the approval predates whatever armed the taint.
/// - `finished_clean` — the run said everything it meant to
///   (`!stop_cause.is_early()`, and an errored run is never clean). An
///   interrupted run's drafts are half a thought; they surface for review
///   instead.
///
/// The early-stop rule is *in* the signature on purpose: a surface that
/// consumes this function cannot forget to ask how the run ended, which is
/// precisely the bug the Slack connector shipped when the rule lived only in
/// the TUI's call site.
pub fn auto_releases(mode: ReviewMode, tainted: bool, finished_clean: bool) -> bool {
    mode == ReviewMode::Auto && !tainted && finished_clean
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [ReviewMode; 3] = [ReviewMode::Now, ReviewMode::Later, ReviewMode::Auto];

    #[test]
    fn tainted_drafts_never_auto_release_whatever_the_mode() {
        for mode in MODES {
            assert!(
                !auto_releases(mode, true, true),
                "{mode:?} must not release a tainted draft"
            );
        }
        assert!(auto_releases(ReviewMode::Auto, false, true));
        assert!(!auto_releases(ReviewMode::Now, false, true));
        assert!(!auto_releases(ReviewMode::Later, false, true));
    }

    /// The rule the policy signature exists to carry (F1): an errored or
    /// early-stopped run releases nothing, in any mode, tainted or not. The
    /// old Slack encoding had no way to ask how the run ended, which is how
    /// `review auto` released an interrupted run's drafts.
    #[test]
    fn an_early_stopped_run_releases_nothing_in_any_mode() {
        for mode in MODES {
            for tainted in [true, false] {
                assert!(
                    !auto_releases(mode, tainted, false),
                    "{mode:?}/tainted={tainted} must hold an early-stopped run's drafts"
                );
            }
        }
    }

    #[test]
    fn the_mode_words_parse_exactly_and_typos_are_refused() {
        assert_eq!(ReviewMode::parse("now"), Some(ReviewMode::Now));
        assert_eq!(ReviewMode::parse("  LATER "), Some(ReviewMode::Later));
        assert_eq!(ReviewMode::parse("Auto"), Some(ReviewMode::Auto));
        for typo in ["always", "sometimes", "on", "", "auto now"] {
            assert_eq!(ReviewMode::parse(typo), None, "{typo:?}");
        }
        assert_eq!(ReviewMode::default(), ReviewMode::Now, "the safe direction");
    }

    #[test]
    fn every_mode_names_itself_and_what_it_does() {
        for mode in MODES {
            assert_eq!(ReviewMode::parse(mode.name()), Some(mode));
            assert!(!mode.describe().is_empty());
        }
        // The auto description carries both exclusions — it is the one line
        // a person reads before trusting the mode.
        let auto = ReviewMode::Auto.describe();
        assert!(auto.contains("tainted"), "{auto}");
        assert!(auto.contains("early-stopped") || auto.contains("errored"), "{auto}");
    }
}
