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
}

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
        speech,
        pending: Pending { queue },
    })
}

/// The question about one draft: read out in full when it is short enough to
/// hear, named and offered when it is not.
fn ask_about(item: &OutboxItem) -> String {
    let view = DraftView::of(&item.args);
    // What the harness pinned *and the reviewer has not since changed*.
    //
    // `filled_defaults` records the fill at staging time and `update_args`
    // rewrites `args` without touching it, so after `mecha outbox edit` moves
    // a pinned field the list still names it — and the readback would call a
    // value the *person* chose a default, on the surface where they hear it
    // once and cannot look back. Headers are safe either way, since those keep
    // their own lines; this only bites the collapsed bucket.
    let pinned: Vec<String> = item
        .filled_defaults
        .iter()
        .filter(|k| item.args.get(k.as_str()) == item.args_before.get(k.as_str()))
        .cloned()
        .collect();
    let spoken = view.spoken(&pinned);
    let mut out = String::new();
    if spoken.chars() <= SPOKEN_UNPROMPTED_CHARS {
        // "Here it is, in full" rather than a second "I've drafted…": the
        // model has usually just said that, and the phrase that earns its
        // place is the one telling the listener that what follows is the
        // draft *verbatim* rather than another description of it.
        out.push_str(&format!("Here it is, in full. {}", spoken.text()));
        out.push_str(taint_line(item));
        out.push_str(" Say yes to send it, or later to leave it in your outbox.");
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
    }
    out
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
    match parse_answer(utterance) {
        SpokenAnswer::NotAnAnswer => Reaction::PassToModel,
        SpokenAnswer::Later => {
            Reaction::Say(format!("Left in your outbox.{}", next_question(next)))
        }
        SpokenAnswer::ReadItOut => match head {
            Some(item) => Reaction::Reread(format!(
                "{} Say yes to send it, or later to leave it.",
                DraftView::of(&item.args)
                    .spoken(&item.filled_defaults)
                    .text()
            )),
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

    /// A value the reviewer changed is no longer called a default.
    ///
    /// `filled_defaults` is written once, at staging; `update_args` rewrites
    /// `args` and leaves it alone. So an edited pinned field is still named
    /// there, and the readback would tell a listener the harness chose a
    /// value they chose themselves — on the surface where they hear it once.
    #[test]
    fn an_edited_pin_is_spoken_as_the_reviewers_own() {
        use mecha_core::outbox::DraftView;

        let mut it = item(
            "i1",
            OutboxKind::Message,
            serde_json::json!({"title": "Reading group", "calendar_id": "primary"}),
            false,
        );
        it.args_before = it.args.clone();
        it.filled_defaults = vec!["calendar_id".into()];

        // As staged: the harness chose it, so it collapses into the clause.
        let unedited: Vec<String> = it
            .filled_defaults
            .iter()
            .filter(|k| it.args.get(k.as_str()) == it.args_before.get(k.as_str()))
            .cloned()
            .collect();
        assert_eq!(unedited, vec!["calendar_id".to_string()]);
        let spoken = DraftView::of(&it.args).spoken(&unedited).text();
        assert!(spoken.contains("Defaults: "), "{spoken}");

        // After an edit the person chose it, and it gets its own sentence.
        it.args["calendar_id"] = serde_json::json!("team-shared");
        let after: Vec<String> = it
            .filled_defaults
            .iter()
            .filter(|k| it.args.get(k.as_str()) == it.args_before.get(k.as_str()))
            .cloned()
            .collect();
        assert!(after.is_empty(), "an edited pin is still called a default");
        let spoken = DraftView::of(&it.args).spoken(&after).text();
        assert!(spoken.contains("Calendar id: team-shared."), "{spoken}");
        assert!(!spoken.contains("Defaults: "), "{spoken}");
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
