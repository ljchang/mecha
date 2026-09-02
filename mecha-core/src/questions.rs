//! Questions a run could not answer for itself, kept until someone does.
//!
//! **The inbound twin of the outbox, and it exists for the same reason that
//! does.** A staged send is a run's *outbound* act surviving the run's end —
//! written by one process, released by another, hours later. Nothing let a
//! run's *question* do the same: `ask_user` parks the run itself, for ten
//! minutes, and then declines. That is the right shape for a present human, a
//! page open in a hand. It is the wrong one for a delegated task, where the
//! honest case is that nobody answers until morning.
//!
//! So a delegated run that needs an answer **ends**. The partial work is kept,
//! the question is stored here, and the task's `waiting_on` moves from the
//! agent to the owner. Answering resumes the session with the answer as the
//! next user turn, and the ball moves back.
//!
//! Three things fall out of that arrangement rather than being added to it:
//!
//! - **The ball-passing is already modelled.** `waiting_on` alternating between
//!   owner and agent is the GTD semantics the board has natively, so the
//!   Waiting view becomes the queue of blocked delegations with no new noun.
//! - **No slot is held.** A parked run occupies one of four llama-server slots
//!   and a cached prefix for ten minutes doing nothing. Ending releases both,
//!   and leaves the prefix in the prompt cache for the resume to find.
//! - **It is a queue, so it is counted.** An unanswered question is exactly the
//!   sort of store that reaches 6,434 items without anybody deciding to let it,
//!   which is the incident `/queues` exists because of.
//!
//! Deliberately **not** a second approval surface: nothing here is approved.
//! It is a question store, and the only thing a person does to an item is
//! answer it or give up on it.
//!
//! Store conventions are the outbox's, for the same reasons: one pretty JSON
//! per item so `$EDITOR` and `git diff` work on it, temp-sibling-and-rename so
//! a reader never sees half a record, an advisory flock for writers, and
//! **nothing is ever deleted** — an answered question is the record of how the
//! run got unstuck.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent::Taint;
use crate::session::Session;

/// One question, and everything needed to put its answer back where it came
/// from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    /// `open` | `answered` | `abandoned`.
    pub status: String,
    /// The question as the model put it.
    pub question: String,
    /// Concrete choices, when the model was confident the answer is one of
    /// them. Never exhaustive — the tool's own description says so, and an
    /// answer outside the list is ordinary.
    #[serde(default)]
    pub options: Vec<String>,
    /// The conversation this came out of.
    ///
    /// **Required, unlike the outbox's**, and the difference is the whole
    /// design: a staged send is executable on its own, so its session is
    /// provenance. An answer is only meaningful as the next turn of the
    /// conversation that asked, so a question with no session is one nobody
    /// can answer — it would be a note, not a question.
    pub session_id: String,
    /// The board item the run was working, when it was working one.
    #[serde(default)]
    pub task_id: Option<String>,
    /// The jail the asking run was held to, so the resume runs where the
    /// question was asked. The outbox's recorded-jail rule, for the same
    /// reason: a deferred continuation resolved against a different root is a
    /// different run.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// The conversation's taint when the question was asked.
    ///
    /// **A question is an inbound request for information, composed by a model
    /// that may have been reading third-party text.** "What is the API key for
    /// the deploy?" is a perfectly well-formed question and an injection's
    /// dream, so an armed snapshot has to reach the person the same way it
    /// does on a staged draft — the outbox warns before a send, and this
    /// warns before an answer.
    #[serde(default)]
    pub taint: Taint,
    pub asked_at: String,
    #[serde(default)]
    pub answered_at: Option<String>,
    /// What the owner said. `None` while open, and on an abandoned question —
    /// giving up is not an answer, and recording it as one would put words in
    /// their mouth in the transcript the resume writes.
    #[serde(default)]
    pub answer: Option<String>,
}

/// The three states a question can be in. Named so a reader elsewhere
/// (`appraisal::of_session`) cannot spell one wrong — `frontdoor`'s own
/// constants exist for the same reason.
pub const OPEN: &str = "open";
pub const ANSWERED: &str = "answered";
pub const ABANDONED: &str = "abandoned";

impl Question {
    pub fn is_open(&self) -> bool {
        self.status == OPEN
    }

    /// One line for a listing.
    pub fn summary(&self) -> String {
        let q = self.question.trim().replace('\n', " ");
        let q: String = q.chars().take(72).collect();
        match &self.task_id {
            Some(t) => format!("{q}  ({t})"),
            None => q,
        }
    }
}

pub struct QuestionStore {
    root: PathBuf,
}

/// Holds the store's writer lock for as long as it lives.
pub struct QuestionLock {
    _file: std::fs::File,
}

impl QuestionStore {
    pub fn default_root() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_QUESTIONS_DIR") {
            return Ok(PathBuf::from(dir));
        }
        Ok(crate::work::mecha_home()?.join("questions"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(QuestionStore { root })
    }

    /// Open at the default location only if it already exists — for read paths
    /// that must not create state as a side effect. Doctor's rule: an
    /// examination that creates what it was about to report is measuring
    /// itself.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(QuestionStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    /// Take the writer lock. Held across a read-modify-write, never across an
    /// `$EDITOR` or a run.
    pub fn lock(&self) -> Result<QuestionLock> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the question store");
        }
        Ok(QuestionLock { _file: file })
    }

    /// Every question, newest first. An unreadable record is skipped rather
    /// than failing the listing — one corrupt file must not hide the queue.
    pub fn items(&self) -> Result<Vec<Question>> {
        self.items_counting().map(|(items, _)| items)
    }

    /// [`items`](Self::items), and how many `.json` files it skipped — for
    /// a reader whose "the store was read" claim must mean every row, not
    /// only that the directory opened (`OutboxStore::items_counting`'s
    /// shape; found on review of the appraisal's `questions_read` field).
    pub fn items_counting(&self) -> Result<(Vec<Question>, usize)> {
        let mut out = Vec::new();
        let mut skipped = 0usize;
        let dir = match std::fs::read_dir(&self.root) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, 0)),
            Err(e) => return Err(e).context("reading the question store"),
        };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Question>(&t).ok())
            {
                Some(q) => out.push(q),
                None => {
                    skipped += 1;
                    tracing::warn!(path = %path.display(), "skipping unreadable question")
                }
            }
        }
        out.sort_by(|a, b| b.asked_at.cmp(&a.asked_at));
        Ok((out, skipped))
    }

    pub fn open_items(&self) -> Result<Vec<Question>> {
        Ok(self
            .items()?
            .into_iter()
            .filter(Question::is_open)
            .collect())
    }

    pub fn get(&self, id: &str) -> Result<Question> {
        let text = std::fs::read_to_string(self.path(id))
            .with_context(|| format!("no such question: {id}"))?;
        serde_json::from_str(&text).with_context(|| format!("unreadable question: {id}"))
    }

    /// The distinguishing tail of an id, for display.
    ///
    /// **Not a prefix**, and that is the whole point. `Session::new_id` is
    /// `YYYYMMDDTHHMMSS-xxxxxxxx`, so the leading characters are a timestamp:
    /// abbreviating to the first eight gives every question asked *on the same
    /// day* the identical handle. Printed one, it looked like an id and was a
    /// date. `serve::resume_key` reached the same conclusion about the same
    /// ids and takes the tail too.
    pub fn short(id: &str) -> &str {
        id.rsplit_once('-').map(|(_, tail)| tail).unwrap_or(id)
    }

    /// Resolve an abbreviated id — head or tail — and refuse an ambiguous one
    /// rather than guessing, the way the outbox and sessions do.
    pub fn find(&self, needle: &str) -> Result<Question> {
        if let Ok(q) = self.get(needle) {
            return Ok(q);
        }
        // Both ends, because the useful abbreviation is the tail and the
        // habitual one is the head. Accepting only what this store prints
        // would refuse a perfectly unambiguous id somebody copied whole from
        // a session line.
        let matches: Vec<Question> = self
            .items()?
            .into_iter()
            .filter(|q| q.id.starts_with(needle) || q.id.ends_with(needle))
            .collect();
        match matches.len() {
            0 => anyhow::bail!("no such question: {needle}"),
            1 => Ok(matches.into_iter().next().expect("just checked")),
            n => anyhow::bail!("{needle} matches {n} questions — use more of the id"),
        }
    }

    pub fn put(&self, q: &Question) -> Result<()> {
        let path = self.path(&q.id);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(q)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Record a question and hand back what was stored.
    #[allow(clippy::too_many_arguments)]
    pub fn park(
        &self,
        question: &str,
        options: Vec<String>,
        session_id: &str,
        task_id: Option<String>,
        workspace: Option<PathBuf>,
        taint: Taint,
    ) -> Result<Question> {
        let q = Question {
            id: Session::new_id(),
            status: OPEN.into(),
            question: question.to_string(),
            options,
            session_id: session_id.to_string(),
            task_id,
            workspace,
            taint,
            asked_at: chrono::Utc::now().to_rfc3339(),
            answered_at: None,
            answer: None,
        };
        let _lock = self.lock()?;
        self.put(&q)?;
        Ok(q)
    }

    /// Record the owner's answer. Returns the question as it now stands.
    ///
    /// Answering does not itself resume anything — the caller does that, and
    /// the split is deliberate: a store that started agent runs would be a
    /// store that can spend the model, and this one is meant to be safe for a
    /// listing to touch.
    pub fn answer(&self, id: &str, answer: &str) -> Result<Question> {
        let _lock = self.lock()?;
        let mut q = self.find(id)?;
        anyhow::ensure!(
            q.is_open(),
            "question {} is already {} — answering it again would resume a conversation that \
             already moved on",
            q.id,
            q.status
        );
        q.status = ANSWERED.into();
        q.answered_at = Some(chrono::Utc::now().to_rfc3339());
        q.answer = Some(answer.to_string());
        self.put(&q)?;
        Ok(q)
    }

    /// Give up on a question without answering it.
    ///
    /// The answer stays `None` on purpose. Abandoning is a decision about the
    /// question, not a reply to it, and writing "abandoned" into the answer
    /// would put words in the owner's mouth in the transcript a later resume
    /// reads back.
    pub fn abandon(&self, id: &str) -> Result<Question> {
        let _lock = self.lock()?;
        let mut q = self.find(id)?;
        anyhow::ensure!(q.is_open(), "question {} is already {}", q.id, q.status);
        q.status = ABANDONED.into();
        q.answered_at = Some(chrono::Utc::now().to_rfc3339());
        self.put(&q)?;
        Ok(q)
    }
}

/// An [`Asker`] that stores the question and stops the run, instead of
/// blocking on an answer that is not coming.
///
/// [`Asker`]: crate::tool::ask::Asker
///
/// **The run ends; it does not wait.** `ask_user`'s contract is that it never
/// blocks forever, and the web's implementation honours that with a ten-minute
/// timeout and then the tool's measured decline. For a delegated task neither
/// half is right: ten minutes is far too short for "answer at breakfast", and
/// a decline tells the model nobody was willing to answer when in truth nobody
/// was *asked yet*.
///
/// Stopping is done through `ToolCtx::cancel`, which is the mechanism the
/// harness already has for exactly this shape — it stops at the next safe
/// point and **keeps the partial turn**, so the work done before the question
/// survives into the transcript the resume reads back. It is not an abort.
pub struct ParkingAsker {
    store: std::sync::Arc<QuestionStore>,
    session_id: String,
    task_id: Option<String>,
    parked: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl ParkingAsker {
    pub fn new(
        store: std::sync::Arc<QuestionStore>,
        session_id: impl Into<String>,
        task_id: Option<String>,
    ) -> Self {
        ParkingAsker {
            store,
            session_id: session_id.into(),
            task_id,
            parked: Default::default(),
        }
    }

    /// Ids parked during this run, in the order they were asked.
    pub fn parked(&self) -> Vec<String> {
        self.parked.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// Merge the conversation's post-run taint into everything this run
    /// parked — a refinement of what `ToolCtx` already knew, never a
    /// replacement for it.
    ///
    /// **Merges rather than overwrites, and that is load-bearing.** The park
    /// seeds fail-closed when the context carries no taint, so an overwrite
    /// here would let a clean post-run snapshot *downgrade* a question that
    /// was recorded as unknown-and-therefore-untrusted. Taint only ever grows;
    /// so does this.
    ///
    /// Best-effort by design: a question is already correct at park time, and
    /// this only sharpens it. That matters because both callers reach this
    /// line through `?` operators on session writes — an I/O failure must not
    /// be able to leave a question recorded as cleaner than it was.
    pub fn stamp_taint(&self, taint: Taint) {
        for id in self.parked() {
            let updated = self.store.get(&id).map(|mut q| {
                q.taint.merge(taint);
                q
            });
            if let Ok(q) = updated {
                let _lock = self.store.lock();
                if let Err(e) = self.store.put(&q) {
                    tracing::warn!(error = %e, id, "could not record taint on a parked question");
                }
            }
        }
    }

    fn record(
        &self,
        question: &str,
        options: &[String],
        workspace: Option<PathBuf>,
        taint: Option<Taint>,
    ) -> String {
        // **Unknown taint is untrusted, at the moment of writing.** The stamp
        // that follows the run is a refinement, and everything between park
        // and stamp — two `?` on session writes, a kill, a full disk — would
        // otherwise leave a question asked out of a conversation full of
        // third-party text recorded as clean, with no warning on `show` and a
        // zero in `/queues`. Every other unknown in this codebase reads as
        // untrusted (`distill::corrections_for`, `Session::taint_timeline`);
        // so does this one.
        let taint = taint.unwrap_or(Taint {
            private: true,
            untrusted: true,
        });
        match self.store.park(
            question,
            options.to_vec(),
            &self.session_id,
            self.task_id.clone(),
            workspace,
            taint,
        ) {
            Ok(q) => {
                if let Ok(mut p) = self.parked.lock() {
                    p.push(q.id.clone());
                }
                format!(
                    "Put to the owner as question {}. This run is ending here — it resumes with \
                     their answer as the next turn, so there is nothing further to do now. Use \
                     your last words to say where you got to.",
                    q.id
                )
            }
            // Fails **open** toward the model, which is the opposite of the
            // outbox's staging rule and right for the opposite reason. A send
            // that cannot be staged must not execute, so it fails closed. A
            // question that cannot be stored is already lost; ending the run
            // as well would discard the work with no record of why, so the
            // model is told plainly and left to report.
            Err(e) => format!(
                "The question could not be stored ({e:#}), so nobody will see it. Do not wait \
                 on an answer — carry on if you can, and say what you needed if you cannot."
            ),
        }
    }
}

#[async_trait::async_trait]
impl crate::tool::ask::Asker for ParkingAsker {
    /// The context-free path: no jail to record and no token to cancel with,
    /// so the question is stored and the run carries on. Reachable only from a
    /// caller that never routes through `ask_in`, which no front-end here does.
    async fn ask(&self, question: &str, options: &[String]) -> Option<String> {
        Some(self.record(question, options, None, None))
    }

    async fn ask_in(
        &self,
        ctx: &crate::tool::ToolCtx,
        question: &str,
        options: &[String],
    ) -> Option<String> {
        let before = self.parked().len();
        let answer = self.record(question, options, Some(ctx.workspace.clone()), ctx.taint);
        // Only stop if it actually landed. A question that failed to store
        // leaves the run alive to report, per `record`.
        if self.parked().len() > before {
            if let Some(cancel) = &ctx.cancel {
                cancel.cancel();
            }
        }
        Some(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ask::Asker;
    use crate::tool::ToolCtx;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-questions-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn store(name: &str) -> QuestionStore {
        QuestionStore::open(scratch(name)).unwrap()
    }

    #[test]
    fn a_parked_question_is_open_and_carries_its_session() {
        let s = store("park");
        let q = s
            .park(
                "Which address should the letter go to?",
                vec!["work".into(), "home".into()],
                "sess-1",
                Some("task-9".into()),
                Some(PathBuf::from("/w/a")),
                Taint::default(),
            )
            .unwrap();

        assert!(q.is_open());
        assert_eq!(q.session_id, "sess-1");
        assert_eq!(q.task_id.as_deref(), Some("task-9"));
        assert_eq!(s.open_items().unwrap().len(), 1);
        assert_eq!(s.get(&q.id).unwrap().options.len(), 2);
    }

    #[test]
    fn answering_records_the_words_and_closes_it_once() {
        let s = store("answer");
        let q = s
            .park("Which one?", vec![], "sess-1", None, None, Taint::default())
            .unwrap();

        let answered = s.answer(&q.id, "the work address").unwrap();
        assert_eq!(answered.status, "answered");
        assert_eq!(answered.answer.as_deref(), Some("the work address"));
        assert!(answered.answered_at.is_some());
        assert!(s.open_items().unwrap().is_empty());

        // A second answer would resume a conversation that already moved on.
        assert!(s.answer(&q.id, "no, home").is_err());
    }

    /// Giving up is a decision about the question, not a reply to it. Writing
    /// a word into `answer` would put it in the owner's mouth in the
    /// transcript a resume reads back.
    #[test]
    fn abandoning_leaves_the_answer_empty() {
        let s = store("abandon");
        let q = s
            .park("Which one?", vec![], "sess-1", None, None, Taint::default())
            .unwrap();
        let done = s.abandon(&q.id).unwrap();
        assert_eq!(done.status, "abandoned");
        assert!(done.answer.is_none());
        assert!(s.open_items().unwrap().is_empty());
    }

    /// The bug a live run printed: `&id[..8]` of a `Session::new_id` is the
    /// date, so two questions asked on one day abbreviate identically. Fails
    /// on the old `short`, which is why it is written against two ids from the
    /// same day rather than two arbitrary ones.
    #[test]
    fn the_short_form_is_the_tail_because_the_head_is_a_date() {
        let a = "20260826T101804-476080dd";
        let b = "20260826T134102-91ac33fe";
        assert_eq!(&a[..8], &b[..8], "the premise: same day, same prefix");
        assert_eq!(QuestionStore::short(a), "476080dd");
        assert_ne!(QuestionStore::short(a), QuestionStore::short(b));
    }

    #[test]
    fn a_question_is_found_by_its_printed_tail() {
        let s = store("tail");
        let q = s
            .park("a?", vec![], "sess", None, None, Taint::default())
            .unwrap();
        let tail = QuestionStore::short(&q.id).to_string();
        assert_eq!(s.find(&tail).unwrap().id, q.id, "what is printed must work");
    }

    #[test]
    fn an_ambiguous_prefix_is_an_error_rather_than_a_guess() {
        let s = store("find");
        let a = s
            .park("a?", vec![], "sess", None, None, Taint::default())
            .unwrap();
        assert!(s.find(&a.id).is_ok());
        assert!(s.find(&a.id[..8]).is_ok());
        assert!(s.find("nope").is_err());

        // Two records sharing a prefix must not resolve to whichever was read
        // first — the outbox and sessions both refuse here.
        let mut twin = a.clone();
        twin.id = format!("{}zz", a.id);
        s.put(&twin).unwrap();
        assert!(s.find(&a.id[..8]).is_err(), "ambiguous prefix must refuse");
    }

    /// The load-bearing behaviour: asking stops the run instead of waiting in
    /// it. Fails on the old shape — an `Asker` that blocks leaves the token
    /// uncancelled and the slot held.
    #[tokio::test]
    async fn asking_parks_the_question_and_stops_the_run() {
        let s = std::sync::Arc::new(store("asker"));
        let asker = ParkingAsker::new(std::sync::Arc::clone(&s), "sess-7", Some("task-3".into()));

        let cancel = tokio_util::sync::CancellationToken::new();
        let ctx = ToolCtx {
            workspace: PathBuf::from("/w/a"),
            cancel: Some(cancel.clone()),
            ..Default::default()
        };

        assert!(!cancel.is_cancelled());
        let answer = asker.ask_in(&ctx, "Which address?", &[]).await.unwrap();

        assert!(cancel.is_cancelled(), "the run must end, not wait");
        assert_eq!(asker.parked().len(), 1);
        let q = s.get(&asker.parked()[0]).unwrap();
        assert_eq!(q.question, "Which address?");
        assert_eq!(q.session_id, "sess-7");
        assert_eq!(q.workspace.as_deref(), Some(Path::new("/w/a")));
        // The model is told what happened, not handed a decline.
        assert!(answer.contains(&q.id));
        assert!(!answer.to_lowercase().contains("declined"));
    }

    /// A question that could not be stored must not also cost the run. Nobody
    /// will see it, so ending as well would discard the work with no record of
    /// why — the opposite of the outbox's fail-closed rule, for the opposite
    /// reason.
    #[tokio::test]
    async fn a_store_that_cannot_write_leaves_the_run_alive() {
        let dir = scratch("broken");
        let s = std::sync::Arc::new(QuestionStore::open(&dir).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();

        let asker = ParkingAsker::new(s, "sess-7", None);
        let cancel = tokio_util::sync::CancellationToken::new();
        let ctx = ToolCtx {
            cancel: Some(cancel.clone()),
            ..Default::default()
        };

        let answer = asker.ask_in(&ctx, "Which address?", &[]).await.unwrap();
        assert!(
            !cancel.is_cancelled(),
            "a lost question must not end the run"
        );
        assert!(asker.parked().is_empty());
        assert!(answer.contains("could not be stored"));
    }

    /// Taint is recorded **at park time** from the context, so nothing that
    /// happens between the question and the end of the run can leave it
    /// looking clean. The later stamp only sharpens it.
    #[tokio::test]
    async fn taint_is_recorded_when_the_question_is_parked() {
        let s = std::sync::Arc::new(store("taint"));
        let asker = ParkingAsker::new(std::sync::Arc::clone(&s), "sess-7", None);
        let ctx = ToolCtx {
            taint: Some(Taint {
                private: true,
                untrusted: true,
            }),
            ..Default::default()
        };
        asker.ask_in(&ctx, "Which address?", &[]).await.unwrap();

        let q = s.get(&asker.parked()[0]).unwrap();
        assert!(
            q.taint.private && q.taint.untrusted,
            "the warning must not depend on a stamp that may never run"
        );
    }

    /// **Unknown taint is untrusted.** A context with no snapshot is not
    /// evidence of a clean conversation, and every other unknown in this
    /// codebase reads the same way. Fails on the old behaviour, which
    /// defaulted the field and left the question recorded as clean until a
    /// post-run stamp that two `?` operators could skip.
    #[tokio::test]
    async fn a_context_with_no_taint_parks_as_untrusted() {
        let s = std::sync::Arc::new(store("taint-unknown"));
        let asker = ParkingAsker::new(std::sync::Arc::clone(&s), "sess-7", None);
        asker
            .ask_in(&ToolCtx::default(), "Which address?", &[])
            .await
            .unwrap();

        let q = s.get(&asker.parked()[0]).unwrap();
        assert!(q.taint.untrusted, "unknown must not read as clean");
    }

    /// The stamp merges and never replaces. Overwriting would let a clean
    /// post-run snapshot downgrade a question parked as unknown — taint only
    /// grows, and so must this.
    #[tokio::test]
    async fn the_stamp_can_only_add_taint_never_remove_it() {
        let s = std::sync::Arc::new(store("taint-merge"));
        let asker = ParkingAsker::new(std::sync::Arc::clone(&s), "sess-7", None);
        asker
            .ask_in(&ToolCtx::default(), "Which address?", &[])
            .await
            .unwrap();
        let id = asker.parked()[0].clone();

        asker.stamp_taint(Taint::default());
        assert!(
            s.get(&id).unwrap().taint.untrusted,
            "a clean stamp must not launder an unknown park"
        );

        // And it does still sharpen a known-clean one upward.
        let s2 = std::sync::Arc::new(store("taint-merge-2"));
        let a2 = ParkingAsker::new(std::sync::Arc::clone(&s2), "sess-8", None);
        let clean = ToolCtx {
            taint: Some(Taint::default()),
            ..Default::default()
        };
        a2.ask_in(&clean, "Which?", &[]).await.unwrap();
        let id2 = a2.parked()[0].clone();
        assert!(!s2.get(&id2).unwrap().taint.untrusted);
        a2.stamp_taint(Taint {
            private: false,
            untrusted: true,
        });
        assert!(s2.get(&id2).unwrap().taint.untrusted, "growth still lands");
    }

    #[test]
    fn items_counting_says_how_many_files_it_skipped() {
        let dir = std::env::temp_dir().join(format!("questions-count-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("20260901T000000-bad.json"), "{not json").unwrap();
        let store = QuestionStore::open(&dir).unwrap();
        let (items, skipped) = store.items_counting().unwrap();
        assert!(items.is_empty());
        assert_eq!(skipped, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
