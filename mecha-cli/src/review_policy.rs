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
///
/// **And an item the harness staged is never a run's draft**, whatever the
/// baseline says. A meeting poll's pick card is staged by `mecha polls
/// sweep` on a timer, clean-tainted, into the same store; one that landed
/// while a run was in flight would otherwise pass this diff and, under
/// `/review auto`, be released with `--yes` — the booking made and every
/// participant mailed with no card ever drawn. Releasing that card *is* the
/// owner's decision (MEETING-POLL-UX-DESIGN.md ruling 4), so it leaves only
/// through a review surface. The author is the field for this, not the
/// session id: a person's `mecha mail compose` has no session either and is
/// still theirs to release from here.
pub fn staged_since(
    items: Vec<mecha_core::outbox::OutboxItem>,
    baseline: &std::collections::HashSet<String>,
) -> Vec<mecha_core::outbox::OutboxItem> {
    items
        .into_iter()
        .filter(|i| {
            i.status == "pending"
                && !baseline.contains(&i.id)
                && i.author() == mecha_core::outbox::Author::Model
        })
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

const SEND_PHRASES: [&str; 28] = [
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
    // Multi-word affirmations, added 2026-09-13 after the first real answer
    // to a spoken offer fell through. Multi-word entries are cheap: the span
    // gate in `confirm::react` re-asks any accept that is a piece of what the
    // speaker *recently* played — two window slots, and a tail bounded by
    // `SPOKEN_UNPROMPTED_CHARS` — so a phrase the model uttered early in a
    // long narration is outside the window and ungated. Residual echo lasts
    // seconds, so that is accepted. A *one-word* entry is the kind that
    // needs care, because one word is immune to that gate by design — none
    // is added here without the owner's ruling
    // (`VOICE-APPROVAL-RESEARCH.md` §8).
    "go for it",
    "thats fine",
    "thats right",
    "sounds good",
    "looks good",
    "i agree",
    "approve it",
    "release it",
];

const LATER_PHRASES: [&str; 23] = [
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
    "not right now",
    "hold off",
    "maybe later",
    "leave it for now",
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

/// Words that may join answer phrases without changing what was answered.
///
/// A deliberately tiny closed set, on the same reasoning as the fillers: an
/// answer is *composed* of answer phrases and these, and nothing else. "go
/// ahead and send it" is `go ahead` · `and` · `send it`. "yes but change the
/// time first" is `yes` · residue, and residue means the words go to the
/// model. `but` is the word that must never be here.
const CONNECTIVES: [&str; 5] = ["and", "then", "now", "please", "just"];

/// One spoken answer, matched against the whole utterance.
///
/// **Whole-utterance, never substring, and that is the whole safety
/// argument.** A substring rule would read "yes" out of "yes, but change the
/// time first" and send the draft the speaker was about to correct — and out
/// of any sentence containing the word at all. So the normalised utterance
/// must be **entirely made of** answer phrases and [`CONNECTIVES`], all of
/// one kind; anything with a word left over is
/// [`SpokenAnswer::NotAnAnswer`] and reaches the model as ordinary words.
///
/// Composed rather than looked up, since 2026-09-13. The first real answer
/// to a spoken offer was *"Go ahead and send it."* — two entries of the
/// list joined by "and" — and equality against the list dropped it. Of
/// twenty natural spoken accepts tried that day, one matched. The safety
/// argument never rested on the list being short, only on the utterance
/// being consumed whole, and that is what [`segment`] still requires.
///
/// Failing that way round is the cheap direction: an unrecognised yes costs
/// one more question, an unrecognised anything-else costs a send nobody
/// authorised.
pub fn parse_answer(utterance: &str) -> SpokenAnswer {
    let normalised = normalise(utterance);
    if normalised.is_empty() {
        return SpokenAnswer::NotAnAnswer;
    }
    segment(&normalised)
}

/// Which lexicon a phrase belongs to, as a bit. Kept apart from
/// [`SpokenAnswer`] so a segmentation can carry "several kinds seen" — the
/// mixed case, which is not an answer.
const SEND: u8 = 1;
const LATER: u8 = 2;
const READ: u8 = 4;

fn lexicon() -> impl Iterator<Item = (&'static str, u8)> {
    SEND_PHRASES
        .iter()
        .map(|p| (*p, SEND))
        .chain(LATER_PHRASES.iter().map(|p| (*p, LATER)))
        .chain(READ_PHRASES.iter().map(|p| (*p, READ)))
}

/// Can the whole phrase be tiled with lexicon entries and connectives?
///
/// Every tiling is tried, not the first found, and the answer is read off
/// the **pure** tilings — those whose phrases are all of one kind. If the
/// pure tilings agree on a kind, that is the answer; if there are none, or
/// they disagree, it is no answer. A mixed tiling is simply not evidence:
/// "do it later" tiles as the `LATER` entry it is *and* as `do it` · `later`,
/// and the first cut unioned the two into a kind that matched nothing, so
/// a listed deferral was unreachable — found on review. "yes later" has
/// only the mixed tiling and stays a non-answer. At least one lexicon
/// phrase must have been used: connectives alone ("and then now") answer
/// nothing.
fn segment(phrase: &str) -> SpokenAnswer {
    let words: Vec<&str> = phrase.split(' ').collect();
    let n = words.len();
    // `reach[i]` is every kind-set some tiling of `words[..i]` produced;
    // `None` means no tiling reaches `i` at all.
    let mut reach: Vec<Vec<u8>> = vec![Vec::new(); n + 1];
    reach[0].push(0);
    for i in 0..n {
        if reach[i].is_empty() {
            continue;
        }
        let from = reach[i].clone();
        if CONNECTIVES.contains(&words[i]) {
            for kinds in &from {
                push_unique(&mut reach[i + 1], *kinds);
            }
        }
        for (entry, kind) in lexicon() {
            let len = entry.split(' ').count();
            if i + len <= n && words[i..i + len].join(" ") == entry {
                for kinds in &from {
                    push_unique(&mut reach[i + len], kinds | kind);
                }
            }
        }
    }
    let pure: Vec<u8> = reach[n]
        .iter()
        .copied()
        .filter(|k| matches!(k, &SEND | &LATER | &READ))
        .collect();
    match pure.as_slice() {
        [SEND] => SpokenAnswer::Send,
        [LATER] => SpokenAnswer::Later,
        [READ] => SpokenAnswer::ReadItOut,
        _ => SpokenAnswer::NotAnAnswer,
    }
}

fn push_unique(set: &mut Vec<u8>, kinds: u8) {
    if !set.contains(&kinds) {
        set.push(kinds);
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
        // An apostrophe joins rather than splits: "that's" is one spoken
        // word, and splitting it left `that s` for a lexicon nobody would
        // write. Both the straight and the typographic mark, because a
        // transcriber picks either.
        .filter(|c| !matches!(c, '\'' | '\u{2019}'))
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

    /// A card the harness staged is never a run's draft: with an empty
    /// baseline — everything pending would otherwise count as new — it is
    /// still not offered, so `/review auto` can never release it unseen.
    #[test]
    fn a_harness_staged_card_is_never_a_runs_draft() {
        use mecha_core::outbox::{OutboxKind, OutboxStore, Provenance};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("mecha-staged-since-{}-{nanos}", std::process::id()));
        let store = OutboxStore::open(&root).unwrap();
        let model = store
            .stage(
                "mail__mail_send",
                OutboxKind::Message,
                serde_json::json!({"to": "a@b"}),
                mecha_core::agent::Taint::default(),
                Provenance::default(),
            )
            .unwrap();
        let harness = store
            .stage_by_harness(
                "mail__calendar_create_event",
                serde_json::json!({"title": "Lab meeting"}),
            )
            .unwrap();
        let offered = staged_since(store.items().unwrap(), &std::collections::HashSet::new());
        let ids: Vec<&str> = offered.iter().map(|i| i.id.as_str()).collect();
        assert!(
            ids.contains(&model.id.as_str()),
            "the model's draft is the run's"
        );
        assert!(
            !ids.contains(&harness.id.as_str()),
            "the harness's card is nobody's run's"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

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

    /// The first real answer to a spoken offer, 2026-09-13 19:37:44 UTC:
    /// "Go ahead and send it." Equality against the list dropped it, the
    /// words went to the model, and the model told the owner to use the
    /// CLI. Every line here is a composition of things the list already
    /// accepted, and each must release.
    #[test]
    fn an_answer_composed_of_answers_is_an_answer() {
        for said in [
            "Go ahead and send it.",
            "Yes, go ahead.",
            "Sure, send it.",
            "Yep, do it.",
            "Okay, go ahead and send it.",
            "Yes please send it.",
            "Send it now.",
            "Approve it.",
            "Yeah send it",
            "Looks good, send it",
            "That's fine, send it",
            "That\u{2019}s right.",
            "I agree.",
            "Go for it.",
            "yes and then send it please",
        ] {
            assert_eq!(parse_answer(said), SpokenAnswer::Send, "{said:?}");
        }
        for said in [
            "No, leave it for now.",
            "Not right now, thanks.",
            "hold off then",
        ] {
            assert_eq!(parse_answer(said), SpokenAnswer::Later, "{said:?}");
        }
    }

    /// Two kinds in one breath is not an answer to either question. And
    /// connectives on their own answer nothing.
    #[test]
    fn a_mixed_answer_is_no_answer() {
        for said in [
            "yes later",
            "no send it",
            "yes no",
            "read it and send it",
            "and then",
            "now",
        ] {
            assert_eq!(parse_answer(said), SpokenAnswer::NotAnAnswer, "{said:?}");
        }
    }

    /// Every listed phrase must still mean what the list says. "do it
    /// later" was on `LATER_PHRASES` and parsed as nothing, because its
    /// second tiling (`do it` · `later`) was allowed to outvote it; nothing
    /// measured that until review. Composition too: an entry followed by a
    /// connective is still that entry.
    #[test]
    fn every_lexicon_entry_parses_as_its_own_kind() {
        for (entries, kind) in [
            (&SEND_PHRASES[..], SpokenAnswer::Send),
            (&LATER_PHRASES[..], SpokenAnswer::Later),
            (&READ_PHRASES[..], SpokenAnswer::ReadItOut),
        ] {
            for entry in entries {
                assert_eq!(parse_answer(entry), kind, "{entry:?}");
                // A *leading* connective: a trailing `then` is filler and
                // is stripped before `segment` sees it, which measured
                // nothing (found on review).
                assert_eq!(
                    parse_answer(&format!("just {entry}")),
                    kind,
                    "just {entry:?}"
                );
            }
        }
    }

    /// `normalise` and `voice::spoken_words` must split words identically,
    /// or the echo gate's normalised check — the one that closes the filler
    /// hole — compares words that cannot match. They drifted on the
    /// apostrophe once (this branch's first cut deleted it here and split
    /// on it there), and `"So, that's right."` off the speaker became a
    /// release. Checked on every entry and on its apostrophied spelling,
    /// because the contractions are exactly where the two can disagree.
    #[test]
    fn words_split_the_same_way_in_both_normalisations() {
        let all = SEND_PHRASES
            .iter()
            .chain(&LATER_PHRASES)
            .chain(&READ_PHRASES);
        for entry in all {
            for spelling in [
                entry.to_string(),
                entry.replace("thats", "that's"),
                entry.replace("thats", "that\u{2019}s"),
            ] {
                assert_eq!(
                    crate::voice::spoken_words(&normalise(&spelling)),
                    crate::voice::spoken_words(&spelling),
                    "{spelling:?}"
                );
            }
        }
    }

    /// The tiling argument needs the three lexicons disjoint and free of
    /// connectives — otherwise one word could tile as two kinds at once.
    #[test]
    fn the_lexicons_are_disjoint_and_no_connective_is_an_answer() {
        let all: Vec<&str> = SEND_PHRASES
            .iter()
            .chain(LATER_PHRASES.iter())
            .chain(READ_PHRASES.iter())
            .copied()
            .collect();
        for (i, a) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(a), "{a:?} appears in two lexicons");
            assert!(
                !CONNECTIVES.contains(a),
                "{a:?} is both an answer and a connective"
            );
            assert_eq!(*a, normalise(a), "{a:?} is not in normalised form");
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
