//! Confirming a staged draft out loud.
//!
//! `ReviewMode::Now` — a draft you just asked for is a draft you are about to
//! read — has always been the default, and on a screen it is a card with
//! buttons. In a call it has to be a question asked aloud and answered aloud,
//! and that is a different problem, because the answer arrives as text in the
//! model's own medium.
//!
//! **The harness asks and the harness hears.** Every word of the offer is
//! composed here from the outbox store, through
//! [`mecha_core::outbox::DraftView::spoken`], which drops no argument; the
//! reply is matched by [`crate::review_policy::parse_answer`] before any
//! model sees it; and the release runs `mecha outbox approve` as a child
//! process. At no point does the decision pass through a context window.
//!
//! That is `mecha review`'s oldest rule in a new setting. The graph's tool
//! surface deliberately has no `kg_accept`, because a model that can accept
//! candidates can accept the ones its own extractor proposed — and a model
//! that could release drafts could release the ones an injection wrote.
//!
//! Three more decisions, each a bug if undone:
//!
//! - **The store is re-read at every step, never cached in the offer.** The
//!   pending state is a queue of *ids*; the bytes come back out of the store
//!   when they are spoken and again when they are released. A draft edited in
//!   the outbox between the question and the answer is therefore released as
//!   it now is, and the wrong-bytes review the recorded jail exists to
//!   prevent cannot arrive through this door either.
//! - **One draft at a time.** A run that staged three gets three questions,
//!   because each is its own reviewable object and a single "yes" covering
//!   three outbound messages is not a review of any of them.
//! - **An unanswered offer is dropped, not remembered.** Say anything that is
//!   not an answer and the question is simply gone; the draft stays pending
//!   in the outbox where it already was. The alternative — a question that
//!   survives until answered — turns every later "yes" in the conversation
//!   into a live release.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use mecha_core::outbox::{DraftView, OutboxItem, OutboxStore};
use tokio::sync::Mutex;

use crate::review_policy::{parse_answer, speakable, SpokenAnswer, SPOKEN_UNPROMPTED_CHARS};

/// Roughly how fast a TTS voice reads, in characters a second. Used only to
/// tell someone how long a draft would take to hear, so being a little wrong
/// costs nothing — "about a minute" is the whole point of the number.
const CHARS_PER_SECOND: usize = 15;

/// One conversation's open question, if it has one.
///
/// Ids and nothing else — see the module docs: the bytes are re-read from
/// the store every time they are spoken or released, so there is no cached
/// copy here to go stale against an outbox edited from the page.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The drafts still to ask about, oldest first. The head is the one the
    /// last question was about.
    pub queue: VecDeque<String>,
    /// **The exact words the speaker last played**, so an answer can be
    /// checked against them. Without this the confirmation door is the one
    /// place in the facade that cannot tell the room's echo from the owner:
    /// the offer is spoken through `say` and never joins any conversation,
    /// so `echoes_the_last_reply`'s usual anchor — the last assistant
    /// message — does not contain the question at all.
    pub asked: String,
    /// How many times this question has been put again because the answer
    /// came back as our own voice. Bounded, because the re-ask is spoken
    /// too and can echo in its turn.
    pub reasks: u8,
}

/// One repetition, then the draft is left in the outbox.
///
/// The re-ask is itself spoken, so it is itself echoable; an unbounded
/// "say that again" is a loop with a send at the end of it. Deferring is
/// the safe termination — it is where the draft already is.
const MAX_REASKS: u8 = 1;

/// Every conversation's open question. Keyed by the conversation the offer
/// was made in — a hosted chat session or a facade slot — because an answer
/// only means anything in the conversation that was asked.
#[derive(Default)]
pub struct Confirmations(Mutex<HashMap<String, Pending>>);

impl Confirmations {
    pub async fn set(&self, key: &str, pending: Pending) {
        if pending.queue.is_empty() {
            self.0.lock().await.remove(key);
        } else {
            self.0.lock().await.insert(key.to_string(), pending);
        }
    }

    pub async fn take(&self, key: &str) -> Option<Pending> {
        self.0.lock().await.remove(key)
    }
}

/// What to say, and what is still outstanding after saying it.
#[derive(Debug, PartialEq, Eq)]
pub struct Offer {
    pub speech: String,
    pub pending: Pending,
}

/// What the harness does about a spoken answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Reaction {
    /// Not an answer to the question. The offer is dropped and the words go
    /// to the model as an ordinary turn — which is what "actually, make it
    /// four o'clock" has to do.
    PassToModel,
    /// Say this, and take the head of the queue as answered.
    Say(String),
    /// Say this, and leave the head where it is — "read it out" is a request
    /// to hear the question again, not an answer to it. Distinct from
    /// [`Reaction::Say`] so the caller never has to re-parse the utterance to
    /// find out what it just did; two decision sites is how the surface and
    /// the policy drift apart.
    Reread(String),
    /// The words were a contiguous span of the question we just asked, so
    /// they are our own voice coming back off the speaker rather than an
    /// answer. Say this, keep the head where it is, and count it.
    ///
    /// Distinct from [`Reaction::PassToModel`] for the reason that variant
    /// exists at all: a non-answer *drops* the question, and dropping it
    /// because the room echoed leaves a staged draft nobody was ever asked
    /// about — silently, which is the failure the outbox exists to prevent.
    NotConvinced(String),
    /// Say `acknowledge`, release the draft, then report the outcome and ask
    /// about whatever is next. Split in two because a release rebuilds a tool
    /// surface and can take seconds, and seconds of silence after "yes" reads
    /// as the call having died.
    Release { acknowledge: String, id: String },
}

/// The question for one run's drafts.
///
/// Returns `None` when there is nothing to ask about — including the case
/// where everything staged was a publish, which is named rather than offered
/// and so leaves no question open.
pub fn compose_offer(items: &[OutboxItem]) -> Option<Offer> {
    let (speakable_items, unspeakable): (Vec<&OutboxItem>, Vec<&OutboxItem>) =
        items.iter().partition(|i| speakable(i.kind));

    let mut speech = String::new();
    let mut queue: VecDeque<String> = speakable_items.iter().map(|i| i.id.clone()).collect();

    if let Some(first) = speakable_items.first() {
        speech.push_str(&ask_about(first));
        if speakable_items.len() > 1 {
            speech.push_str(&format!(
                " There {} after that.",
                plural_drafts(speakable_items.len() - 1)
            ));
        }
    }

    // A publish is never offered by ear: its reviewable object is a rendered
    // page, and reading a path aloud is not reviewing a website. It is still
    // *named*, because a staged action nobody mentions is the silent failure
    // the whole outbox exists to avoid.
    if !unspeakable.is_empty() {
        if !speech.is_empty() {
            speech.push(' ');
        }
        speech.push_str(&format!(
            "There {} waiting too — that needs the screen, so it is in your outbox.",
            plural_publishes(unspeakable.len())
        ));
    }

    if speech.is_empty() {
        return None;
    }
    // Nothing speakable means nothing to answer: the publish line is a
    // statement, and leaving a question open that no word can answer would
    // make the next "yes" in the call land on nothing.
    if queue.is_empty() {
        return Some(Offer {
            speech,
            pending: Pending::default(),
        });
    }
    queue.make_contiguous();
    Some(Offer {
        pending: Pending {
            queue,
            // What we are about to say, recorded before we say it: `react`
            // has nothing else to compare an answer against, because the
            // offer is spoken through `say` and never joins a conversation.
            asked: speech.clone(),
            reasks: 0,
        },
        speech,
    })
}

/// The question about one draft: read out in full when it is short enough to
/// hear, named and offered when it is not.
fn ask_about(item: &OutboxItem) -> String {
    let view = DraftView::of(&item.args);
    // The pins the reviewer has not since changed — see
    // `OutboxItem::unedited_defaults`. Calling a value the *person* chose a
    // default is worst here and on the re-read below, the two surfaces where
    // they hear it once and cannot look back.
    let spoken = view.spoken(&item.unedited_defaults());
    let mut out = String::new();
    if spoken.chars() <= SPOKEN_UNPROMPTED_CHARS {
        // "Here it is, in full" rather than a second "I've drafted…": the
        // model has usually just said that, and the phrase that earns its
        // place is the one telling the listener that what follows is the
        // draft *verbatim* rather than another description of it.
        out.push_str(&format!("Here it is, in full. {}", spoken.text()));
        out.push_str(taint_line(item));
        out.push_str(" Say yes to send it, or later to leave it in your outbox.");
        out.push_str(&identity_tail(&view));
    } else {
        // Long enough that reading it unasked would be a monologue rather
        // than a question — so the choice of hearing it is the owner's.
        let headline = view
            .headers
            .iter()
            .find(|(k, _)| k == "subject" || k == "title")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| item.summary.clone());
        out.push_str(&format!(
            "I've drafted something longer: {headline}. It is about {} to read out.",
            seconds_aloud(spoken.chars())
        ));
        out.push_str(taint_line(item));
        out.push_str(
            " Say read it out to hear the whole thing, yes to send it, \
             or later to leave it in your outbox.",
        );
        out.push_str(&identity_tail(&view));
    }
    out
}

/// The last words of an offer, and deliberately not the menu.
///
/// **The tail is the part that echoes** — it is the most recent thing in the
/// room when the microphone opens — and until this the offer ended by
/// reciting the parser's own accept list: *"Say yes to send it, or later to
/// leave it in your outbox."* A clean two-word truncation of that is
/// `"send it"`, which `parse_answer` matches exactly and which released a
/// draft with nobody asked.
///
/// Moving the menu off the end does not remove it from the offer, so a span
/// of it is still possible mid-utterance — that is what the span gate in
/// [`react`] is for. What this changes is which words are *most likely* to
/// come back, and it spends them on the one fact a listener cannot re-read:
/// which account this is going out from. The same answer as #144, one
/// surface further on.
fn identity_tail(view: &DraftView) -> String {
    match view.headers.iter().find(|(k, _)| k == "account") {
        Some((_, account)) => format!(" That one is from your {account} account."),
        // Never empty, or the menu is the tail again by default. Not an
        // answer in any phrase list, and a span of it is caught like any
        // other span.
        None => " That one is waiting on you.".into(),
    }
}

/// The taint warning, spoken.
///
/// The same fact every other review surface puts above everything else, in
/// the one place a listener can act on it: they cannot re-read the addressing
/// line, so they have to be told to listen to it.
fn taint_line(item: &OutboxItem) -> &'static str {
    if item.taint.trifecta_armed() {
        " I had read outside content when I wrote this, so listen to the addressing."
    } else {
        ""
    }
}

/// "about forty seconds" / "about two minutes" — deliberately vague, because
/// the number is only there to help someone decide whether to listen.
fn seconds_aloud(chars: usize) -> String {
    let secs = chars / CHARS_PER_SECOND;
    if secs < 90 {
        format!("{} seconds", ((secs.max(5) + 5) / 10) * 10)
    } else {
        format!("{} minutes", (secs + 30) / 60)
    }
}

fn plural_drafts(n: usize) -> String {
    if n == 1 {
        "is one more draft".into()
    } else {
        format!("are {n} more drafts")
    }
}

fn plural_publishes(n: usize) -> String {
    if n == 1 {
        "is also a publish".into()
    } else {
        format!("are also {n} publishes")
    }
}

/// Decide what an answer means. Pure: the caller does the I/O.
///
/// `head` is the draft the question was about, re-read from the store — not
/// carried over from when the question was asked, so what is confirmed is
/// what is in the store now.
pub fn react(
    utterance: &str,
    pending: &Pending,
    head: Option<&OutboxItem>,
    next: Option<&OutboxItem>,
) -> Reaction {
    let Some(id) = pending.queue.front().cloned() else {
        return Reaction::PassToModel;
    };
    // **Before parsing, and deliberately before every branch.** A span of
    // the question is our own voice however it parses: as `Send` it
    // releases a draft with nobody asked, and as a non-answer it *drops*
    // the question and hands the words to the model, leaving a staged draft
    // that will never be mentioned again. Both are silent.
    //
    // `echoes_the_last_reply` needs two words, which is what keeps this
    // survivable: "yes", "ok" and "sure" are one word and can never reach
    // here, and they are what a person actually says. "send it" is two and
    // *is* a span of the menu, so a listener who says it is asked once
    // more and answers "yes" — one repetition, against a draft going out
    // unasked.
    if super::echoes_the_last_reply(utterance, &pending.asked) {
        return if pending.reasks >= MAX_REASKS {
            Reaction::Say(format!(
                "I keep hearing myself, so I have left it in your outbox.{}",
                next_question(next)
            ))
        } else {
            Reaction::NotConvinced("Sorry — I think that was my own echo. Was that a yes?".into())
        };
    }
    match parse_answer(utterance) {
        SpokenAnswer::NotAnAnswer => Reaction::PassToModel,
        SpokenAnswer::Later => {
            Reaction::Say(format!("Left in your outbox.{}", next_question(next)))
        }
        SpokenAnswer::ReadItOut => match head {
            // Spoken like any other question, so its tail is echoable like
            // any other tail — same rule, same reason. `unedited_defaults`
            // is #154's: the pins the reviewer has not since changed.
            Some(item) => {
                let view = DraftView::of(&item.args);
                Reaction::Reread(format!(
                    "{} Say yes to send it, or later to leave it.{}",
                    view.spoken(&item.unedited_defaults()).text(),
                    identity_tail(&view)
                ))
            }
            // The draft is gone from the store between the question and the
            // answer — sent from the page, or swept. Saying so beats reading
            // out nothing, and the queue moves on.
            None => Reaction::Say(format!(
                "That draft is not in the outbox any more.{}",
                next_question(next)
            )),
        },
        SpokenAnswer::Send => match head {
            Some(_) => Reaction::Release {
                acknowledge: "Sending it now.".into(),
                id,
            },
            None => Reaction::Say(format!(
                "That draft is not in the outbox any more, so there is nothing to send.{}",
                next_question(next)
            )),
        },
    }
}

/// The tail of a reply: the next draft, asked about in full.
///
/// **The follow-on is a whole question, not a pointer to one.** An earlier
/// cut said "there is one more draft — say next to hear it", which invented a
/// word the parser does not know: every listener who said "next" would have
/// been answered by the model instead, with the draft still sitting there.
/// A surface must not offer a verb the policy cannot recognise, and the way
/// to guarantee that is to offer no new verbs at all.
fn next_question(next: Option<&OutboxItem>) -> String {
    match next {
        Some(item) => format!(" Next: {}", ask_about(item)),
        None => String::new(),
    }
}

/// Read one item back out of the store.
///
/// A missing item is `None` rather than an error: between a question and its
/// answer a draft can legitimately be sent from the page or swept, and that
/// is a thing to say out loud, not a failure to report.
pub fn item_now(root: &Path, id: &str) -> Option<OutboxItem> {
    OutboxStore::open(root)
        .ok()?
        .item(id)
        .ok()
        .filter(|i| i.status == "pending")
}

/// Release one draft, through the same verb the review surfaces use.
///
/// A child process rather than an in-process call, on the `/triggers` rule:
/// `mecha outbox approve` rebuilds the tool surface rooted at the jail the
/// item recorded, which is a whole tool registry and possibly an MCP startup
/// — work that belongs in its own process, and work whose one implementation
/// must stay in the CLI so no surface can send in a way the terminal cannot.
pub async fn release(id: &str) -> Result<String, String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new(crate::exe::self_exe())
            .args(["outbox", "approve", id, "--yes"])
            .output(),
    )
    .await
    .map_err(|_| "that took too long — it is still in your outbox".to_string())?
    .map_err(|e| format!("could not run the release: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.lines().last().unwrap_or("it failed").to_string())
    }
}

/// What to say once a release has run.
pub fn report_release(outcome: Result<String, String>, next: Option<&OutboxItem>) -> String {
    match outcome {
        Ok(_) => format!("Sent.{}", next_question(next)),
        // The child's own last line, spoken. A release can fail for real
        // reasons — an expired token, a provider refusing — and "sorry,
        // something went wrong" is the sentence that makes a person retry
        // forever.
        Err(why) => format!("It did not send: {why} It is still in your outbox."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::Taint;
    use mecha_core::outbox::OutboxKind;
    use serde_json::json;

    /// A value the reviewer changed is no longer called a default — on both
    /// of the surfaces that speak it.
    ///
    /// Through `ask_about` and `react`, not through a filter re-implemented
    /// here. The first version of this test copied the derivation into its own
    /// body and asserted against `DraftView` directly, so no production line
    /// was on the path and deleting the filter left it green — the exact
    /// regression it was written to guard.
    #[test]
    fn an_edited_pin_is_spoken_as_the_reviewers_own() {
        let mut it = item(
            "i1",
            OutboxKind::Message,
            serde_json::json!({"title": "Reading group", "calendar_id": "primary"}),
            false,
        );
        it.args_before = it.args.clone();
        it.filled_defaults = vec!["calendar_id".into()];

        // As staged: the harness chose it, so it collapses into the clause.
        let said = ask_about(&it);
        assert!(said.contains("Defaults: "), "{said}");

        // After an edit the person chose it, and it gets its own sentence.
        it.args["calendar_id"] = serde_json::json!("team-shared");
        let said = ask_about(&it);
        assert!(said.contains("Calendar id: team-shared."), "{said}");
        assert!(!said.contains("Defaults: "), "{said}");

        // And the re-read, which is the worse place to get it wrong: it
        // exists to say what is in the store *now*, which is the edit.
        let pending = Pending {
            queue: VecDeque::from(vec!["i1".to_string()]),
        };
        match react("read it out", &pending, Some(&it), None) {
            Reaction::Reread(said) => {
                assert!(said.contains("Calendar id: team-shared."), "{said}");
                assert!(!said.contains("Defaults: "), "{said}");
            }
            other => panic!("expected the draft read back, got {other:?}"),
        }
    }

    fn item(id: &str, kind: OutboxKind, args: serde_json::Value, tainted: bool) -> OutboxItem {
        OutboxItem {
            filled_defaults: Vec::new(),
            call_id: None,
            id: id.into(),
            status: "pending".into(),
            tool: "mail__calendar_create".into(),
            kind,
            args_before: args.clone(),
            args,
            summary: "a draft".into(),
            session_id: None,
            workspace: None,
            taint: Taint {
                private: tainted,
                untrusted: tainted,
            },
            created_at: "2026-08-25T10:00:00Z".into(),
            resolved_at: None,
            reason: None,
            error: None,
        }
    }

    fn event() -> OutboxItem {
        item(
            "a",
            OutboxKind::Message,
            json!({"title": "Coffee with Thea", "when": "Thursday August 27, 3pm to 3:30pm"}),
            false,
        )
    }

    /// A short draft is read out in full, unasked. Every field of it: the one
    /// an injection would add is the one a summary would drop.
    #[test]
    fn a_short_draft_is_read_out_whole() {
        let offer = compose_offer(&[event()]).expect("an offer");
        assert!(
            offer.speech.contains("Coffee with Thea"),
            "{}",
            offer.speech
        );
        assert!(
            offer.speech.contains("Thursday August 27, 3pm to 3:30pm"),
            "{}",
            offer.speech
        );
        assert!(offer.speech.contains("Say yes to send it"));
        assert!(
            offer.speech.contains("in full"),
            "the listener is told the readback is verbatim: {}",
            offer.speech
        );
        assert_eq!(offer.pending.queue.len(), 1);
    }

    /// A long one is named and offered, never recited unasked — and the
    /// choice of hearing it stays the owner's.
    #[test]
    fn a_long_draft_is_offered_rather_than_recited() {
        let long = item(
            "b",
            OutboxKind::Message,
            json!({"subject": "Re: R01 resubmission", "body_markdown": "word ".repeat(200)}),
            false,
        );
        let offer = compose_offer(&[long]).expect("an offer");
        assert!(
            offer.speech.contains("Re: R01 resubmission"),
            "{}",
            offer.speech
        );
        assert!(offer.speech.contains("read it out"), "{}", offer.speech);
        assert!(
            !offer.speech.contains("word word word"),
            "a long draft must not be recited unasked: {}",
            offer.speech
        );
    }

    /// The taint warning is spoken, because a listener cannot go back and
    /// re-read the addressing line.
    #[test]
    fn a_tainted_draft_says_so_out_loud() {
        let offer = compose_offer(&[item("c", OutboxKind::Message, json!({"title": "x"}), true)])
            .expect("an offer");
        assert!(offer.speech.contains("outside content"), "{}", offer.speech);
        assert!(!compose_offer(&[event()])
            .unwrap()
            .speech
            .contains("outside content"));
    }

    /// A publish is named but never offered, and leaves no question open —
    /// its reviewable object is a rendered page, so no spoken answer could
    /// be a review of it.
    #[test]
    fn a_publish_is_named_and_never_offered() {
        let offer = compose_offer(&[item(
            "p",
            OutboxKind::Publish,
            json!({"bundle": "site"}),
            false,
        )])
        .expect("it is still mentioned");
        assert!(
            offer.speech.contains("needs the screen"),
            "{}",
            offer.speech
        );
        assert!(
            offer.pending.queue.is_empty(),
            "a question no word can answer must not stay open"
        );
    }

    /// Several drafts are one question each: a single yes covering three
    /// outbound messages is not a review of any of them.
    #[test]
    fn several_drafts_are_asked_about_one_at_a_time() {
        let offer = compose_offer(&[
            event(),
            item("b", OutboxKind::Message, json!({"title": "b"}), false),
        ])
        .expect("an offer");
        assert_eq!(offer.pending.queue.len(), 2);
        assert!(offer.speech.contains("one more draft"), "{}", offer.speech);
        // Only the head is read out.
        assert!(offer.speech.contains("Coffee with Thea"));
        assert!(!offer.speech.contains("Title: b."), "{}", offer.speech);
    }

    #[test]
    fn nothing_staged_asks_nothing() {
        assert_eq!(compose_offer(&[]), None);
    }

    /// The reaction table, including the one that matters: anything that is
    /// not an answer drops the question rather than holding it open, so a
    /// later "yes" in the same call cannot land on a forgotten draft.
    #[test]
    fn an_unanswered_offer_is_dropped_not_held() {
        let ev = event();
        let pending = compose_offer(std::slice::from_ref(&ev)).unwrap().pending;
        assert_eq!(
            react("actually make it four o'clock", &pending, Some(&ev), None),
            Reaction::PassToModel
        );
        assert!(matches!(
            react("yes", &pending, Some(&ev), None),
            Reaction::Release { .. }
        ));
        assert!(matches!(
            react("later", &pending, Some(&ev), None),
            Reaction::Say(_)
        ));
    }

    /// Hearing a draft again is not answering the question about it, and the
    /// caller must not have to re-parse the utterance to find that out.
    #[test]
    fn reading_it_out_again_does_not_consume_the_question() {
        let ev = event();
        let pending = compose_offer(std::slice::from_ref(&ev)).unwrap().pending;
        match react("read it out", &pending, Some(&ev), None) {
            Reaction::Reread(said) => assert!(said.contains("Coffee with Thea"), "{said}"),
            other => panic!("expected a re-read: {other:?}"),
        }
    }

    /// The follow-on is a whole question. An earlier cut said "say next to
    /// hear it", which invented a word `parse_answer` does not know — so
    /// every listener who said it would have been answered by the model
    /// while the draft sat there. No surface may offer a verb the policy
    /// cannot recognise.
    #[test]
    fn the_next_draft_is_asked_about_never_pointed_at() {
        let ev = event();
        let second = item(
            "b",
            OutboxKind::Message,
            json!({"title": "Second thing"}),
            false,
        );
        let pending = compose_offer(&[ev.clone(), second.clone()])
            .unwrap()
            .pending;
        let said = match react("later", &pending, Some(&ev), Some(&second)) {
            Reaction::Say(said) => said,
            other => panic!("{other:?}"),
        };
        assert!(said.contains("Second thing"), "{said}");
        assert!(said.contains("Say yes to send it"), "{said}");
        assert!(!said.contains("say next"), "{said}");
    }

    /// A draft that left the store between the question and the answer is
    /// said to be gone, rather than releasing something else or reading out
    /// nothing.
    #[test]
    fn a_draft_that_vanished_is_reported_not_guessed_at() {
        let ev = event();
        let pending = compose_offer(std::slice::from_ref(&ev)).unwrap().pending;
        match react("yes", &pending, None, None) {
            Reaction::Say(said) => assert!(said.contains("not in the outbox"), "{said}"),
            other => panic!("a missing draft must not be released: {other:?}"),
        }
    }

    #[test]
    fn a_failed_release_says_why_and_where_the_draft_is() {
        let said = report_release(Err("token expired".into()), None);
        assert!(said.contains("token expired"), "{said}");
        assert!(said.contains("still in your outbox"), "{said}");
    }

    #[test]
    fn a_length_is_spoken_roundly() {
        assert_eq!(seconds_aloud(150), "10 seconds");
        assert_eq!(seconds_aloud(1500), "2 minutes");
    }
}

/// The door that was open while the other two were being closed.
///
/// `completion` reaches `shared.confirmations.take` *before* either approval
/// gate, so nothing those added ever saw a confirmation. The offer is spoken
/// through `say` and joins no conversation, so `echoes_the_last_reply`'s
/// usual anchor does not contain the question either. What made it live
/// rather than theoretical: `ask_about` ended by reciting the accept list,
/// so the most echo-prone words in the whole utterance were `"send it"`.
#[cfg(test)]
mod echo_at_the_confirmation_door {
    use super::*;
    use mecha_core::agent::Taint;
    use mecha_core::outbox::OutboxKind;
    use serde_json::json;

    fn draft() -> OutboxItem {
        OutboxItem {
            filled_defaults: vec!["account".into()],
            call_id: None,
            id: "d1".into(),
            status: "pending".into(),
            tool: "mail__mail_send".into(),
            kind: OutboxKind::Message,
            args_before: json!({"to": "alice@example.com", "account": "dartmouth",
                                "subject": "Reading group", "body_markdown": "Thursday works."}),
            args: json!({"to": "alice@example.com", "account": "dartmouth",
                         "subject": "Reading group", "body_markdown": "Thursday works."}),
            summary: "a draft".into(),
            session_id: None,
            workspace: None,
            taint: Taint {
                private: false,
                untrusted: false,
            },
            created_at: "2026-09-03T10:00:00Z".into(),
            resolved_at: None,
            error: None,
            reason: None,
        }
    }

    /// Built through `compose_offer`, never hand-rolled: the whole defect was
    /// that `Pending` did not carry what had been said, so a `Pending` a test
    /// assembles itself would test the fix against a fixture rather than
    /// against the code that has to populate it.
    fn offered() -> (String, Pending) {
        let o = compose_offer(&[draft()]).expect("a speakable draft is offered");
        (o.speech, o.pending)
    }

    #[test]
    fn the_offer_records_what_it_said() {
        let (speech, pending) = offered();
        assert_eq!(
            pending.asked, speech,
            "`react` has nothing to compare against"
        );
        assert_eq!(pending.reasks, 0);
    }

    #[test]
    fn the_tail_is_not_the_accept_list() {
        let (speech, _) = offered();
        let tail = speech.trim_end_matches(['.', ' ']);
        let tail = &tail[tail.len().saturating_sub(60)..];
        assert!(
            tail.contains("dartmouth account"),
            "the last words are still the menu: {tail:?}"
        );
        // The precise failure: a clean truncation of the old tail parsed as
        // an accept and released a draft.
        assert!(
            !speech.trim_end().ends_with("in your outbox."),
            "the offer still ends by reciting the parser's own accept list"
        );
    }

    #[test]
    fn our_own_voice_is_asked_again_rather_than_acted_on() {
        let (_, pending) = offered();
        let item = draft();
        // A two-word truncation of the menu, which is what the room returns.
        match react("Send it.", &pending, Some(&item), None) {
            Reaction::NotConvinced(said) => assert!(said.contains("echo"), "{said}"),
            other => panic!("an echo of the question released or dropped it: {other:?}"),
        }
    }

    #[test]
    fn a_second_echo_defers_rather_than_looping() {
        let (_, mut pending) = offered();
        pending.reasks = MAX_REASKS;
        let item = draft();
        match react("Send it.", &pending, Some(&item), None) {
            Reaction::Say(said) => assert!(said.contains("outbox"), "{said}"),
            other => panic!("the re-ask is unbounded, and it is spoken: {other:?}"),
        }
    }

    /// The reason the gate is survivable at all.
    ///
    /// `echoes_the_last_reply` needs two words, and every one-word accept is
    /// therefore immune. That is not a lucky accident of the constant — it is
    /// what a person actually says, and if this ever fails the gate has
    /// started silencing real answers.
    #[test]
    fn a_bare_yes_still_sends() {
        let (_, pending) = offered();
        let item = draft();
        for heard in ["yes", "Yes.", "yeah", "ok", "sure"] {
            match react(heard, &pending, Some(&item), None) {
                Reaction::Release { .. } => {}
                other => panic!("{heard:?} no longer sends: {other:?}"),
            }
        }
    }

    /// A correction reuses the offer's words and must survive, which is why
    /// the rule is a contiguous span and not a bag of words.
    #[test]
    fn a_correction_still_reaches_the_model() {
        let (_, pending) = offered();
        let item = draft();
        for heard in [
            "actually make it four o'clock",
            "change the subject to reading group Friday",
        ] {
            assert!(
                matches!(
                    react(heard, &pending, Some(&item), None),
                    Reaction::PassToModel
                ),
                "{heard:?} was swallowed by the echo gate"
            );
        }
    }

    /// The quieter half of the same defect.
    ///
    /// A span that does not parse as an answer used to fall to
    /// `PassToModel`, which *drops* the question — leaving a staged draft
    /// nobody would ever be asked about again, and handing our own words to
    /// the model as a turn.
    #[test]
    fn an_echo_that_is_not_an_answer_keeps_the_question() {
        let (_, pending) = offered();
        let item = draft();
        match react("from your dartmouth account", &pending, Some(&item), None) {
            Reaction::NotConvinced(_) => {}
            other => panic!("an echo dropped the open question: {other:?}"),
        }
    }
}
