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
    /// The utterance before [`Pending::asked`]. See
    /// [`Pending::recently_said`] for why there are two of these and not one,
    /// and not a log.
    pub asked_before: String,
}

impl Pending {
    /// Everything said recently enough to still be coming out of the speaker.
    ///
    /// Two slots, not one, and not an unbounded log. One was wrong: after a
    /// re-read `asked` held only the re-read, while the offer that produced
    /// the echo was still playing — so the next segment of that same
    /// playback matched nothing, fell to `PassToModel`, and *dropped* the
    /// question. An unbounded join is wrong the other way: a listener can
    /// ask for a re-read as often as they like, and every draft read aloud
    /// would accumulate.
    pub fn recently_said(&self) -> [&str; 2] {
        [self.asked.as_str(), self.asked_before.as_str()]
    }

    /// The window slides: what we are about to say becomes `asked`, and what
    /// was `asked` becomes `asked_before`.
    fn sliding(&self, said: &str, reasks: u8) -> Pending {
        Pending {
            queue: self.queue.clone(),
            asked: said.to_string(),
            asked_before: self.asked.clone(),
            reasks,
        }
    }

    /// Something was said in answer, so the echo streak is over.
    ///
    /// `reasks` resets here and only here. It did not at first, and the one
    /// transition that matters is the re-read: an echo, then a genuine "read
    /// it out", then a genuine "send it" spent the last of the budget and
    /// answered "I keep hearing myself" — untrue by then.
    pub fn after_saying(&self, said: &str) -> Pending {
        self.sliding(said, 0)
    }

    /// The same question, put again because the answer was our own voice.
    ///
    /// **Identical to [`Pending::after_saying`] except for the counter**, and
    /// that is the point. An earlier version made these two opposites — one
    /// extending `asked`, the other replacing it — and each was wrong in the
    /// direction the other was right. The sliding window makes the
    /// distinction unnecessary, because the previous utterance survives in
    /// `asked_before` either way. The budget goes up rather than resetting
    /// because nothing has been answered, and the re-ask is itself spoken:
    /// `MAX_REASKS` is what stops that being a loop with a send at the end.
    ///
    /// Both live on the type because both were got wrong once as a `match`
    /// arm in another module.
    pub fn after_reask(&self, said: &str) -> Pending {
        self.sliding(said, self.reasks.saturating_add(1))
    }
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
/// `preceded_by` is the model's own reply, spoken immediately before this
/// offer and in the *same stretch* out of the speaker — `completion` says the
/// answer and then `say(" {offer}")`, with no pause between. So it echoes
/// exactly as the offer does, and it seeds `asked_before`.
///
/// Found on review, and it is the offer's own tail one step back: any
/// `SEND_PHRASES` entry a reply can contain but the offer cannot —
/// `"go ahead"`, `"do that"`, `"confirm"`, `"approve"`, `"book it"` — was an
/// unasked release. Neither window slot held it, `parse_answer` returned
/// `Send`, `Release` fired.
pub fn compose_offer(items: &[OutboxItem], preceded_by: &str) -> Option<Offer> {
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
            asked_before: preceded_by.to_string(),
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

/// Is this our own voice coming back, under either normalisation?
///
/// **Two spellings of "the same words" decide this, and they disagree.**
/// `spoken_words` lowercases and strips punctuation; `review_policy`'s
/// `normalise`, which decides whether those words *release a draft*,
/// additionally strips `LEADING_FILLER` and `TRAILING_FILLER`. Every word on
/// those lists is a hole in the gate by construction: `"So, send it."` is not
/// a span of anything, normalises to `"send it"`, and releases. It needs the
/// transcriber to insert a filler the offer did not contain, which Parakeet
/// does — "Um options for" is in this project's own measurement table.
///
/// So the span is tried against both forms. Checking only the raw one is the
/// drift `Reread`'s comment warns about three functions down: two decision
/// sites over one utterance is how a surface and its policy come apart.
fn ours_coming_back(utterance: &str, pending: &Pending) -> bool {
    pending.recently_said().iter().any(|said| {
        super::echoes_the_last_reply(utterance, said)
            || super::echoes_the_last_reply(&crate::review_policy::normalise(utterance), said)
    })
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
    // Our own voice, off the speaker — but this is checked *per outcome*,
    // not before them all. A first version gated everything and was wrong
    // in the dangerous direction: the offer recites the whole menu, so
    // `"read it out"` and `"leave it"` are spans of it too. That refused
    // the verb the offer had just taught, on precisely the drafts too long
    // to have been read aloud, and then asked "was that a yes?" — to which
    // a bare `"yes"` is one word, immune, and releases a draft the listener
    // had asked to *hear*. Found on review.
    //
    // `Later` and `ReadItOut` cause neither failure this gate exists for:
    // one leaves the draft where it already is, the other re-reads the
    // question and keeps it open. They are honoured however they parse.
    let ours = ours_coming_back(utterance, pending);
    let reask = |pending: &Pending| {
        // The draft can leave the store between the question and the answer,
        // and then there is nothing to ask again *about*. Both branches
        // check it now; only the exhausted one did, which meant the ordinary
        // re-ask said "is that what you want?" about a draft that no longer
        // existed. No wrong action followed either way — but a spoken
        // surface's output is the whole of what it does.
        if head.is_none() {
            return Reaction::Say(format!(
                "That draft is not in the outbox any more.{}",
                next_question(next)
            ));
        }
        if pending.reasks >= MAX_REASKS {
            // `head` is known present here — the guard above returned on
            // `None` — so this can say plainly where the draft is.
            Reaction::Say(format!(
                "I keep hearing myself, so I have left it in your outbox.{}",
                next_question(next)
            ))
        } else {
            // Two constraints, and they pull against each other.
            //
            // It must not name an accept phrase: a one-word answer is
            // invisible to the span rule, so a re-ask containing `"yes"` —
            // or `"sure"`, which is also in `SEND_PHRASES` — hands back the
            // bypass it just closed. `no_single_word_of_the_reask_is_an_answer`
            // checks that against the real lists rather than trusting this
            // comment.
            //
            // And it must not ask for a *repeat*, which the first wording
            // did: complying with "could you say it again?" means saying
            // "send it" again, which is the same span, which spends the
            // budget and defers. Asking for a decision leaves the listener
            // the one-word answer that works. Found on review.
            Reaction::NotConvinced(
                "Sorry — that may have been my own echo. Is that what you want?".into(),
            )
        }
    };
    match parse_answer(utterance) {
        // The silent half of the defect: a span that does not parse used to
        // reach `PassToModel`, which *drops* the question — leaving a staged
        // draft nobody is asked about again and handing our own words to the
        // model as a turn.
        SpokenAnswer::NotAnAnswer if ours => reask(pending),
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
        SpokenAnswer::Send if ours => reask(pending),
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
        // **`next_question` here too, and that was a real hole.**
        // `answer_completion` pops the head whatever the outcome, so without
        // it a failed release with a second draft queued left the
        // confirmation armed on a draft that had never been offered — and
        // the next "yes" released it unasked. No echo needed to reach it.
        // Found on review of the echo gate; it predates that gate.
        Err(why) => format!(
            "It did not send: {why} It is still in your outbox.{}",
            next_question(next)
        ),
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
        // `..Default::default()` for the echo window: this test is about
        // what the re-read *says*, and an empty window gates nothing.
        let pending = Pending {
            queue: VecDeque::from(vec!["i1".to_string()]),
            ..Default::default()
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
        let offer = compose_offer(&[event()], "").expect("an offer");
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
        let offer = compose_offer(&[long], "").expect("an offer");
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
        let offer = compose_offer(
            &[item("c", OutboxKind::Message, json!({"title": "x"}), true)],
            "",
        )
        .expect("an offer");
        assert!(offer.speech.contains("outside content"), "{}", offer.speech);
        assert!(!compose_offer(&[event()], "")
            .unwrap()
            .speech
            .contains("outside content"));
    }

    /// A publish is named but never offered, and leaves no question open —
    /// its reviewable object is a rendered page, so no spoken answer could
    /// be a review of it.
    #[test]
    fn a_publish_is_named_and_never_offered() {
        let offer = compose_offer(
            &[item(
                "p",
                OutboxKind::Publish,
                json!({"bundle": "site"}),
                false,
            )],
            "",
        )
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
        let offer = compose_offer(
            &[
                event(),
                item("b", OutboxKind::Message, json!({"title": "b"}), false),
            ],
            "",
        )
        .expect("an offer");
        assert_eq!(offer.pending.queue.len(), 2);
        assert!(offer.speech.contains("one more draft"), "{}", offer.speech);
        // Only the head is read out.
        assert!(offer.speech.contains("Coffee with Thea"));
        assert!(!offer.speech.contains("Title: b."), "{}", offer.speech);
    }

    #[test]
    fn nothing_staged_asks_nothing() {
        assert_eq!(compose_offer(&[], ""), None);
    }

    /// The reaction table, including the one that matters: anything that is
    /// not an answer drops the question rather than holding it open, so a
    /// later "yes" in the same call cannot land on a forgotten draft.
    #[test]
    fn an_unanswered_offer_is_dropped_not_held() {
        let ev = event();
        let pending = compose_offer(std::slice::from_ref(&ev), "")
            .unwrap()
            .pending;
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
        let pending = compose_offer(std::slice::from_ref(&ev), "")
            .unwrap()
            .pending;
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
        let pending = compose_offer(&[ev.clone(), second.clone()], "")
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
        let pending = compose_offer(std::slice::from_ref(&ev), "")
            .unwrap()
            .pending;
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

    pub(super) fn draft() -> OutboxItem {
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
    pub(super) fn offered() -> (String, Pending) {
        let o = compose_offer(&[draft()], "").expect("a speakable draft is offered");
        (o.speech, o.pending)
    }

    /// The long branch of `ask_about`, which is the one that recites
    /// `"read it out"` — and therefore the one where refusing that verb
    /// refuses the only way to hear a draft too long to be read unasked.
    pub(super) fn long_draft() -> OutboxItem {
        let mut item = draft();
        let body = "Thursday works for me, and so does Friday afternoon if that is easier. \
                    I have put a hold on the seminar room either way, and I will bring the \
                    printed copies along with the revised handout for the second half."
            .repeat(4);
        item.args = json!({"to": "alice@example.com", "account": "dartmouth",
                           "subject": "Reading group", "body_markdown": body});
        item.args_before = item.args.clone();
        item
    }

    pub(super) fn long_offer() -> (String, Pending) {
        let o = compose_offer(&[long_draft()], "").expect("a speakable draft is offered");
        assert!(
            o.speech.contains("read it out"),
            "the fixture is not on the long branch, so it teaches no verb to refuse"
        );
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
        // Char-bounded, not byte-bounded. `&tail[len - 60..]` cannot panic
        // on today's ASCII fixture, but it is the trap the sibling test in
        // `mod.rs` documents avoiding, and a draft with an em-dash in its
        // last sixty bytes turns a clear assertion failure into an
        // unrelated one.
        let trimmed = speech.trim_end_matches(['.', ' ']);
        let tail: String = trimmed
            .chars()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
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
        let (_, pending) = offered();
        // Driven through the transition rather than by setting the field:
        // hand-built state is the fixture these tests avoid on purpose.
        let pending = pending.after_reask("Sorry — that may have been my own echo.");
        assert_eq!(pending.reasks, MAX_REASKS);
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

/// The three ways the first version of the gate was wrong, each found by
/// review rather than by the tests above.
#[cfg(test)]
mod the_gate_must_not_eat_real_answers {
    use super::echo_at_the_confirmation_door::*;
    use super::*;

    /// **The safety inversion.** The offer recites the whole menu, so the
    /// safe verbs are spans of it as much as `"send it"` is — and the first
    /// version refused them, then asked "was that a yes?". A bare `"yes"` is
    /// one word, immune to the gate, and releases a draft the listener had
    /// asked to *hear* or to defer.
    #[test]
    fn the_safe_answers_are_honoured_even_though_they_are_spans() {
        let (_, pending) = long_offer();
        let item = long_draft();
        // Both really are spans, or this test proves nothing.
        assert!(ours_coming_back("read it out", &pending));
        assert!(ours_coming_back("leave it", &pending));

        assert!(
            matches!(
                react("read it out", &pending, Some(&item), None),
                Reaction::Reread(_)
            ),
            "the verb the offer just taught was refused on first use"
        );
        assert!(
            matches!(
                react("leave it", &pending, Some(&item), None),
                Reaction::Say(_)
            ),
            "deferring a draft was refused, and deferring is where it already is"
        );
    }

    /// And the re-ask must not lead: the one-word answer is the one the gate
    /// cannot see, so inviting it is inviting the bypass.
    #[test]
    fn the_reask_does_not_ask_for_a_yes() {
        let (_, pending) = offered();
        let item = draft();
        let Reaction::NotConvinced(said) = react("Send it.", &pending, Some(&item), None) else {
            panic!("the dangerous branch is no longer gated");
        };
        let words = said.to_lowercase();
        for leading in ["yes", "a yes", "yeah", "ok"] {
            assert!(
                !words.contains(leading),
                "the re-ask invites {leading:?}, which is one word and immune: {said:?}"
            );
        }
    }

    /// **The filler hole.** `spoken_words` keeps `"so"`; `normalise` strips
    /// it before deciding whether the same words release a draft. Every word
    /// on those lists was a hole by construction.
    #[test]
    fn a_filler_does_not_smuggle_an_echo_past_the_span() {
        let (_, pending) = offered();
        let item = draft();
        for heard in ["So, send it.", "Um, send it", "send it please"] {
            assert_eq!(
                parse_answer(heard),
                crate::review_policy::SpokenAnswer::Send,
                "{heard:?} no longer parses as a send; this test has stopped measuring anything"
            );
            assert!(
                matches!(
                    react(heard, &pending, Some(&item), None),
                    Reaction::NotConvinced(_)
                ),
                "{heard:?} released a draft the span rule would have caught unfilled"
            );
        }
    }

    /// **The re-ask must not narrow what we compare against.** The offer is
    /// still playing when its first echo is caught, so the next segment of
    /// the same playback has to be checked against the offer too — not
    /// against the eleven words of the re-ask alone.
    #[test]
    fn a_second_echo_of_the_same_offer_is_still_ours() {
        let (_, pending) = offered();
        let after =
            pending.after_reask("Sorry — that may have been my own echo. Could you say it again?");
        // A different fragment of the *original* offer, which is what a
        // second segment of the same playback is.
        assert!(
            ours_coming_back("to send it", &after),
            "the next segment of the same playback is no longer recognised"
        );
        assert_eq!(after.reasks, pending.reasks + 1);
        assert_eq!(after.queue, pending.queue, "the head must not move");
    }

    /// **The same guarantee across a re-read**, which is the transition that
    /// was wrong.
    ///
    /// `react` honours `ReadItOut` however it parses — deliberately, and
    /// `the_safe_answers_are_honoured_even_though_they_are_spans` asserts
    /// `"read it out"` *is* a span of the long offer. So a re-read is
    /// reachable **from an echo**, and on that path "a real answer arrived,
    /// so the offer has finished playing" is false. Replacing the window
    /// there dropped the offer while it was still in the speaker, and the
    /// next segment of it fell through to `PassToModel` — which strands the
    /// draft. Found on review; it is this branch's own defect, one
    /// transition further on.
    #[test]
    fn a_reread_does_not_forget_the_offer_still_playing() {
        let (_, pending) = long_offer();
        let after = pending.after_saying("Here it is, in full. Thursday works.");
        assert!(
            ours_coming_back("to hear the whole thing", &after),
            "the offer left the window while it was still playing"
        );
        assert_eq!(after.reasks, 0, "a real answer must clear the streak");
        assert_eq!(after.queue, pending.queue, "the head must not move");
    }
}

/// Three ways a listener could still be surprised, all found on review.
#[cfg(test)]
mod the_escape_hatches_must_stay_open {
    use super::echo_at_the_confirmation_door::*;
    use super::*;
    use crate::review_policy::SpokenAnswer;

    /// The gate runs ahead of the `head.is_none()` check, so the exhausted
    /// message has to consult it itself: a draft can leave the store between
    /// the question and the answer, and "I have left it in your outbox" is
    /// then simply untrue. No wrong action follows — but on a spoken surface
    /// what it says is the whole of what it does.
    #[test]
    fn a_deferral_does_not_claim_a_vanished_draft_is_waiting() {
        let (_, pending) = offered();
        let spent = pending.after_reask("Sorry — that may have been my own echo.");
        // Gone before the answer arrived: neither branch may claim it is
        // waiting, and the unexhausted one must not ask again about nothing.
        for pending in [&pending, &spent] {
            let Reaction::Say(said) = react("Send it.", pending, None, None) else {
                panic!("an echo about a vanished draft must say so, not re-ask it");
            };
            assert!(
                said.contains("not in the outbox any more"),
                "the draft is gone and we did not say so: {said:?}"
            );
        }
        // And the ordinary case still reads the ordinary way.
        let item = draft();
        let Reaction::Say(still_there) = react("Send it.", &spent, Some(&item), None) else {
            panic!("an exhausted budget must defer");
        };
        assert!(
            still_there.contains("left it in your outbox"),
            "{still_there:?}"
        );
    }

    /// **The reply is spoken in the same breath as the offer.**
    ///
    /// `completion` says the model's answer and then `say(" {offer}")` — one
    /// stretch out of the speaker, no pause. So the reply echoes exactly as
    /// the offer does, and the accept phrases it can contain are ones the
    /// offer never does: `"go ahead"`, `"do that"`, `"confirm"`, `"approve"`,
    /// `"book it"`. Before `preceded_by` seeded `asked_before`, neither
    /// window slot held them, `parse_answer` returned `Send`, and `Release`
    /// fired on a draft nobody was asked about. Found on review.
    #[test]
    fn the_reply_before_the_offer_is_in_the_window_too() {
        let reply = "I have drafted that. If it looks right, go ahead and I will \
                     send it — or say the word and I can book it for Thursday instead.";
        let offer = compose_offer(&[draft()], reply).expect("a speakable draft is offered");
        let item = draft();

        for heard in ["go ahead", "book it"] {
            // Each really is an accept the *offer alone* would not have
            // caught, or this test proves nothing about the seeding.
            assert_eq!(
                parse_answer(heard),
                crate::review_policy::SpokenAnswer::Send,
                "{heard:?} no longer parses as a send"
            );
            let offer_only = Pending {
                asked_before: String::new(),
                ..offer.pending.clone()
            };
            assert!(
                !ours_coming_back(heard, &offer_only),
                "{heard:?} is in the offer after all, so this fixture measures nothing"
            );
            assert!(
                matches!(
                    react(heard, &offer.pending, Some(&item), None),
                    Reaction::NotConvinced(_)
                ),
                "{heard:?} echoed out of the reply and released the draft"
            );
        }
    }

    /// The rule the re-ask's wording rests on, checked against the real
    /// phrase lists instead of against a comment.
    ///
    /// A one-word answer is invisible to the span rule by design, so any
    /// single word of the re-ask that parses as an answer is a bypass the
    /// harness spoke aloud itself.
    #[test]
    fn no_single_word_of_the_reask_is_an_answer() {
        let (_, pending) = offered();
        let item = draft();
        let Reaction::NotConvinced(said) = react("Send it.", &pending, Some(&item), None) else {
            panic!("the dangerous branch is no longer gated");
        };
        for word in said.split_whitespace() {
            assert_eq!(
                parse_answer(word),
                SpokenAnswer::NotAnAnswer,
                "the re-ask says {word:?} on its own, which is a one-word answer \
                 the span rule cannot see: {said:?}"
            );
        }
    }

    /// A genuine answer between two echoes must not spend the echo budget.
    #[test]
    fn a_real_answer_clears_the_streak() {
        let (offer, pending) = long_offer();
        // Echo, caught: the budget is now spent.
        let after = pending.after_reask("Sorry — that may have been my own echo.");
        assert_eq!(after.reasks, MAX_REASKS);
        // A genuine "read it out" between the echo and the answer. Driven
        // through the transition the arm calls, not re-derived here, because
        // the whole bug was that the arm carried the count forward.
        let resumed = after.after_saying("Here it is, in full. Thursday works.");
        assert_eq!(
            resumed.reasks, 0,
            "a real answer left the budget spent, so the next echo defers on a stale count"
        );
        assert!(
            !resumed.asked.contains(&offer),
            "the re-read must supersede the offer, not accumulate it"
        );
        assert_eq!(resumed.queue, pending.queue, "the head must not move");
    }

    /// A failed release popped the head but never offered the next draft, so
    /// the confirmation stayed armed on something nobody had been asked
    /// about. Pre-existing, and reachable without any echo at all.
    #[test]
    fn a_failed_release_still_asks_about_the_next_draft() {
        let second = draft();
        let report = report_release(Err("the token expired.".into()), Some(&second));
        assert!(report.contains("still in your outbox"), "{report}");
        // The subject is the load-bearing half: `"send it"` appears in every
        // short `ask_about`, so a disjunction on it would pass without the
        // next draft ever being named.
        assert!(
            report.contains("Reading group"),
            "the next draft is never offered, so the next yes releases it unasked: {report}"
        );
        // And the successful branch has always done this — the two must not
        // disagree, which is how the hole opened.
        let ok = report_release(Ok("sent".into()), Some(&second));
        assert!(
            ok.contains("Reading group") || ok.contains("send it"),
            "{ok}"
        );
    }
}
