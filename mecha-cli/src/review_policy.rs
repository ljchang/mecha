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

/// The drafts one run staged: pending items that were not pending when it
/// started.
///
/// **Scope is an id-diff, not a timestamp and not the session id.** A
/// timestamp cannot tell a draft this run made from one a trigger made in the
/// same second, and the session id is right only for the front-ends that
/// stamp one — while the property every surface actually needs is "the
/// overnight backlog is none of this run's business". The diff says that
/// exactly, and says it the same way on every surface, which is why it lives
/// here rather than being written out at each call site.
///
/// No baseline means no diff, and no diff means nothing is offered: a surface
/// that could not read the store before the run must not guess afterwards
/// that everything pending is new.
pub fn staged_since(
    items: Vec<mecha_core::outbox::OutboxItem>,
    baseline: &std::collections::HashSet<String>,
) -> Vec<mecha_core::outbox::OutboxItem> {
    items
        .into_iter()
        .filter(|i| i.status == "pending" && !baseline.contains(&i.id))
        .collect()
}

// ---------------------------------------------------------------- by voice
//
// `ReviewMode::Now` puts a finished run's drafts in front of you. On a screen
// that is a card with buttons; in a call it has to be a question asked out
// loud and answered out loud, and that is a different security problem,
// because the answer arrives as *text in the model's medium*.
//
// **The harness asks the question and the harness hears the answer.** The
// offer is composed from the store — the real staged arguments, through
// `DraftView::spoken`, which drops nothing — and the reply is matched here,
// before a model sees it. Nothing about release passes through the context
// window at any point.
//
// That is `mecha review`'s oldest rule wearing a different coat: the graph's
// tool surface deliberately has no `kg_accept`, because a model that can
// accept candidates can accept the ones its own extractor proposed. A model
// that could release drafts could release the ones an injection drafted.
//
// The remaining exposure is that a person hears rather than reads, which is
// answered by uttering the whole draft and by [`SPOKEN_UNPROMPTED_CHARS`]
// below — never by summarising it.

/// What the owner said when asked whether to send a draft.
///
/// A **closed enum**, on `SLACK-ACTIONS-DESIGN.md` §1's reasoning: the
/// alternative is a free-form label, and a free-form label is how `spam`
/// ends up inside a verb that reads as harmless. Here the harmless-looking
/// verb would be "send".
///
/// There is deliberately **no reject**. Rejecting an outbox item takes a
/// reason — the record of the refusal, which the learning miner reads — and
/// a reason nobody typed is worse than none. "No" therefore parks the draft
/// in the outbox, which is where it already was; nothing spoken can throw a
/// draft away, and the safe answer to every ambiguity is the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpokenAnswer {
    /// Release it now.
    Send,
    /// Leave it in the outbox — "later", "not now", "no".
    Later,
    /// Read the whole draft out before deciding.
    ReadItOut,
    /// Not an answer to the question that was asked. Nothing is released,
    /// the offer is dropped, and the words go to the model as an ordinary
    /// turn — which is what "actually, make it four o'clock" has to do.
    NotAnAnswer,
}

/// Fillers stripped before matching. A deliberately tiny closed set: every
/// word here widens what counts as an answer, and the cost of *not*
/// stripping one is being asked again, which is nothing.
const LEADING_FILLER: [&str; 4] = ["um", "uh", "well", "so"];
const TRAILING_FILLER: [&str; 4] = ["please", "thanks", "thank you", "then"];

const SEND_PHRASES: [&str; 20] = [
    "yes",
    "yeah",
    "yep",
    "yup",
    "yes do it",
    "yes send it",
    "do it",
    "do that",
    "send it",
    "send that",
    "send",
    "go ahead",
    "ok",
    "okay",
    "sure",
    "confirm",
    "confirmed",
    "approve",
    "add it",
    "book it",
];

const LATER_PHRASES: [&str; 19] = [
    "later",
    "do it later",
    "not now",
    "not yet",
    "outbox",
    "the outbox",
    "put it in the outbox",
    "leave it in the outbox",
    "leave it",
    "leave it for later",
    "save it",
    "save it for later",
    "hold it",
    "hold on",
    "wait",
    "skip",
    "skip it",
    "no",
    "ignore it",
];

const READ_PHRASES: [&str; 9] = [
    "read it",
    "read it out",
    "read it back",
    "read it to me",
    "read it out loud",
    "read the whole thing",
    "read",
    "read that",
    "what does it say",
];

/// One spoken answer, matched against the whole utterance.
///
/// **Whole-utterance, never substring, and that is the whole safety
/// argument.** A substring rule would read "yes" out of "yes, but change the
/// time first" and send the draft the speaker was about to correct — and out
/// of any sentence containing the word at all. So the normalised utterance
/// must *be* one of the phrases above; anything else is
/// [`SpokenAnswer::NotAnAnswer`] and reaches the model as ordinary words.
///
/// Failing that way round is the cheap direction: an unrecognised yes costs
/// one more question, an unrecognised anything-else costs a send nobody
/// authorised.
pub fn parse_answer(utterance: &str) -> SpokenAnswer {
    let normalised = normalise(utterance);
    if normalised.is_empty() {
        return SpokenAnswer::NotAnAnswer;
    }
    let phrase = normalised.as_str();
    if SEND_PHRASES.contains(&phrase) {
        SpokenAnswer::Send
    } else if LATER_PHRASES.contains(&phrase) {
        SpokenAnswer::Later
    } else if READ_PHRASES.contains(&phrase) {
        SpokenAnswer::ReadItOut
    } else {
        SpokenAnswer::NotAnAnswer
    }
}

/// Lowercase, letters and spaces only, filler trimmed off both ends.
///
/// Punctuation goes because it is the transcriber's guess, not the speaker's
/// — Parakeet writes "Yes." or "Yes!" from identical audio, and an answer
/// that depended on which would be a coin flip.
pub(crate) fn normalise(utterance: &str) -> String {
    let mut words: Vec<String> = utterance
        .chars()
        .map(|c| {
            if c.is_alphabetic() || c.is_whitespace() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    while words
        .first()
        .is_some_and(|w| LEADING_FILLER.contains(&w.as_str()))
    {
        words.remove(0);
    }
    // "thank you" is two words, so trailing filler is matched against the
    // tail of the phrase rather than the last word alone.
    let mut joined = words.join(" ");
    loop {
        let trimmed = TRAILING_FILLER
            .iter()
            .find_map(|f| joined.strip_suffix(&format!(" {f}")).map(str::to_string))
            .or_else(|| TRAILING_FILLER.contains(&joined.as_str()).then(String::new));
        match trimmed {
            Some(shorter) => joined = shorter,
            None => break,
        }
    }
    joined
}

/// How much of a draft is read out without being asked for.
///
/// 400 characters is roughly sixty-five words, or about twenty-five seconds
/// at an ordinary speaking rate. A calendar event is a fraction of it; a
/// letter is several times it. Past this the offer stops being a question and
/// becomes a monologue — and a listener cannot skim a monologue, which is the
/// property the whole spoken review depends on.
///
/// A longer draft is not refused: it is named, and the choice of hearing it
/// in full or leaving it for the screen is the owner's, spoken.
pub const SPOKEN_UNPROMPTED_CHARS: usize = 400;

/// Whether a staged item can be reviewed by ear at all.
///
/// A publish cannot: its reviewable object is a *rendered page*, which is why
/// `outbox show` leads with the bundle directory and the file to open rather
/// than with the arguments. Reading a path aloud is not reviewing a website.
/// That is a fact about the object, not a policy — so it is a `match` on the
/// kind with no configuration behind it.
pub fn speakable(kind: mecha_core::outbox::OutboxKind) -> bool {
    match kind {
        mecha_core::outbox::OutboxKind::Message => true,
        mecha_core::outbox::OutboxKind::Publish => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [ReviewMode; 3] = [ReviewMode::Now, ReviewMode::Later, ReviewMode::Auto];

    /// The property the whole spoken review rests on: a sentence that
    /// *contains* an answer is not an answer. Every line here is a real thing
    /// a person says while a draft is on offer, and every one of them must
    /// reach the model as ordinary words rather than releasing anything.
    #[test]
    fn a_sentence_containing_yes_is_not_a_yes() {
        for said in [
            "yes but change the time first",
            "yes and also add one on Friday",
            "actually make it four o'clock",
            "who else is invited",
            "did you say yes",
            "send it to Thea instead",
            "no wait what did you put in the subject line",
            "ok so what about Thursday",
        ] {
            assert_eq!(
                parse_answer(said),
                SpokenAnswer::NotAnAnswer,
                "{said:?} must not be read as an answer"
            );
        }
    }

    #[test]
    fn the_three_answers_parse_and_punctuation_is_the_transcribers_guess() {
        // Parakeet writes "Yes." or "Yes!" from identical audio.
        for yes in [
            "yes",
            "Yes.",
            "YES!",
            "  yeah  ",
            "yes please",
            "um do it",
            "Send it.",
        ] {
            assert_eq!(parse_answer(yes), SpokenAnswer::Send, "{yes:?}");
        }
        for later in [
            "later",
            "not now",
            "No.",
            "put it in the outbox",
            "skip it thanks",
        ] {
            assert_eq!(parse_answer(later), SpokenAnswer::Later, "{later:?}");
        }
        for read in ["read it", "Read it back.", "what does it say"] {
            assert_eq!(parse_answer(read), SpokenAnswer::ReadItOut, "{read:?}");
        }
        assert_eq!(parse_answer(""), SpokenAnswer::NotAnAnswer);
        assert_eq!(parse_answer("   "), SpokenAnswer::NotAnAnswer);
    }

    /// "No" parks the draft; it never throws one away. A spoken reject would
    /// need a reason — the record of the refusal, which the learning miner
    /// reads — and there is no way to speak one that is worth having.
    #[test]
    fn nothing_spoken_can_discard_a_draft() {
        for refusal in ["no", "not now", "later", "skip", "ignore it"] {
            assert_eq!(parse_answer(refusal), SpokenAnswer::Later, "{refusal:?}");
        }
    }

    /// A publish cannot be reviewed by ear at all: its reviewable object is a
    /// rendered page, and reading a path aloud is not reviewing a website.
    #[test]
    fn a_publish_is_never_speakable() {
        use mecha_core::outbox::OutboxKind;
        assert!(speakable(OutboxKind::Message));
        assert!(!speakable(OutboxKind::Publish));
    }

    /// The unprompted ceiling has to sit between the two cases it separates,
    /// or it is not separating them. A calendar event is well under; a letter
    /// is well over.
    #[test]
    fn the_spoken_ceiling_sits_between_an_event_and_a_letter() {
        use mecha_core::outbox::DraftView;
        use serde_json::json;
        let event = DraftView::of(&json!({
            "title": "Coffee with Thea",
            "when": "Thursday August 27, 3:00pm to 3:30pm",
            "account": "dartmouth",
        }))
        .spoken(&[]);
        assert!(
            event.chars() < SPOKEN_UNPROMPTED_CHARS,
            "an event is read out without asking: {} chars",
            event.chars()
        );
        let letter = DraftView::of(&json!({
            "to": "dirk@example.org",
            "subject": "Re: R01 resubmission",
            "body_markdown": "Dear Dirk,\n\n".to_string() + &"word ".repeat(120),
        }))
        .spoken(&[]);
        assert!(
            letter.chars() > SPOKEN_UNPROMPTED_CHARS,
            "a letter is offered, not recited: {} chars",
            letter.chars()
        );
    }

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
        assert!(
            auto.contains("early-stopped") || auto.contains("errored"),
            "{auto}"
        );
    }
}
