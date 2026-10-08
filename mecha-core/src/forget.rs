//! Forgetting a session: the transcript, and every trace of it in every
//! other store.
//!
//! **The opposite of [`crate::archive`]**, which files a conversation away
//! and leaves the record whole. This is the owner's "permanently delete"
//! (2026-09-28): what the conversation said, and what was derived from it,
//! stops existing. It is never automatic and no tool reaches it — the owner
//! asks for it by session, on a surface only the owner holds.
//!
//! **There is no room to remove here.** An incognito chat forgets by
//! deleting one directory, because nothing was ever written anywhere else
//! (`docs/INCOGNITO-DESIGN.md` §4.3). A recorded session was read by every
//! nightly reader after the fact, so forgetting it is an enumeration of the
//! stores that copy from transcripts, and the enumeration is this file. A
//! store that learns to hold a session's content and is not taught here is a
//! leak — [`Report::residue`] says out loud what is knowingly left.
//!
//! The order is what makes a failure recoverable:
//!
//! 1. The transcript is renamed to `<id>.jsonl.forgetting` first. Every
//!    reader lists `.jsonl` only, so from that instant nothing new is
//!    derived from it — and the file is still there to say where the
//!    workspace was.
//! 2. Every store is purged, each under its own writer lock. A failure is
//!    recorded and the walk goes on: stopping at the first error would leave
//!    more behind, not less.
//! 3. Only when every store answered does the transcript go. A purge that
//!    failed somewhere leaves `.forgetting` in place, so running it again
//!    picks up where it stopped — the silently-degrading guard, applied to
//!    deletion: an incomplete forget says so and keeps the handle to finish.
//!
//! **Lines are filtered as text, never re-serialised through a type.** A
//! store written by a newer binary carries fields this one cannot name, and
//! a typed round-trip would drop them from every row that was *kept*.

use crate::session::{Session, SessionMeta};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where each store lives. [`Roots::from_config`] is what a surface uses:
/// each store's own default (so every `MECHA_*_DIR` override is honoured),
/// then the directories config relocates.
#[derive(Debug, Clone)]
pub struct Roots {
    pub sessions: PathBuf,
    /// `~/.mecha`: the work directories, the spill root, and the loose files.
    pub home: PathBuf,
    pub outbox: PathBuf,
    pub questions: PathBuf,
    pub messages: PathBuf,
    pub learning: PathBuf,
    pub harness: PathBuf,
    pub appraisals: PathBuf,
    pub comparisons: PathBuf,
    pub closures: PathBuf,
    pub triggers: PathBuf,
    pub workflows: PathBuf,
    /// The front door's requests: a stranger's request is not the
    /// conversation's, so it survives, un-pointed from the triage run.
    pub requests: PathBuf,
    /// The standing regression check's pin list — `MECHA_REGRESSION_PINS`,
    /// as `scripts/replay-regression.sh` reads it.
    pub regression_pins: PathBuf,
    /// Mail triage records: the owner's threads, so they stay; a drafting
    /// conversation's pointer (`draft_session`) leaves them.
    pub triage: PathBuf,
    /// The document cache (`MECHA_DOCUMENTS_DIR`, or `~/.mecha/documents`):
    /// keyed by a PDF's hash, so named in the residue rather than purged.
    pub documents: PathBuf,
}

impl Roots {
    /// [`Roots::from_env`], with the stores `config` relocates: `[outbox]
    /// dir` and `[messages] dir`. Every surface that writes those stores
    /// resolves them this way, so a forget that asked only the environment
    /// would purge a default directory that does not exist and report the
    /// store clean while the real one still holds the session's rows.
    pub fn from_config(config: &crate::config::Config) -> Result<Self> {
        let mut roots = Self::from_env()?;
        if let Some(dir) = &config.outbox.dir {
            roots.outbox = dir.clone();
        }
        if let Some(dir) = &config.messages.dir {
            roots.messages = dir.clone();
        }
        Ok(roots)
    }

    /// Each store's own default, honouring every `MECHA_*_DIR` it does — and
    /// nothing from config; a caller with a config uses [`Roots::from_config`].
    pub fn from_env() -> Result<Self> {
        let home = crate::work::mecha_home()?;
        Ok(Roots {
            sessions: Session::default_dir()?,
            outbox: crate::outbox::OutboxStore::default_root()?,
            questions: crate::questions::QuestionStore::default_root()?,
            messages: crate::mailbox::MailboxStore::default_root()?,
            learning: crate::learning::LearningStore::default_root()?,
            harness: crate::harness::HarnessStore::default_root()?,
            appraisals: crate::appraisal_store::AppraisalStore::default_root()?,
            comparisons: crate::comparison::ComparisonStore::default_root()?,
            closures: crate::closure::ClosureStore::default_root()?,
            triggers: crate::trigger::TriggerStore::default_root()?,
            workflows: home.join("workflows"),
            requests: home.join("requests"),
            triage: crate::mail_triage::TriageStore::default_root()?,
            documents: crate::document::Cache::default_dir()?,
            regression_pins: std::env::var_os("MECHA_REGRESSION_PINS")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("regression-sessions.txt")),
            home,
        })
    }

    /// Every store under one directory — for tests, and for an experiment
    /// home, where the layout is `~/.mecha`'s own.
    pub fn under(home: &Path) -> Self {
        Roots {
            sessions: home.join("sessions"),
            outbox: home.join("outbox"),
            questions: home.join("questions"),
            messages: home.join("messages"),
            learning: home.join("learning"),
            harness: home.join("learning").join("harness"),
            appraisals: home.join("appraisals"),
            comparisons: home.join("comparisons"),
            closures: home.join("closures"),
            triggers: home.join("triggers"),
            workflows: home.join("workflows"),
            requests: home.join("requests"),
            triage: home.join("mail-triage"),
            documents: home.join("documents"),
            regression_pins: home.join("regression-sessions.txt"),
            home: home.to_path_buf(),
        }
    }
}

/// What the knowledge graph did with the session's episode. The graph is
/// another process's store, reached through its own binary, so the caller
/// supplies the reach ([`GraphRedactor`]) and this module only judges the
/// answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphOutcome {
    /// Redacted `n` episodes (zero is an answer: never distilled).
    Redacted(usize),
    /// No graph on this machine to hold anything.
    Absent,
}

/// Remove every episode the graph holds for a session.
pub trait GraphRedactor {
    fn redact_session(&self, session_id: &str) -> Result<GraphOutcome>;
}

/// What a forget did, store by store.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    pub id: String,
    /// `(store, rows or files removed)`, in walk order, zeros included —
    /// "looked and found nothing" is a finding, and the reader should be
    /// able to see every store was asked.
    pub removed: Vec<(String, usize)>,
    /// What is knowingly left, in words the owner can act on.
    pub residue: Vec<String>,
    /// Stores that could not be purged. Non-empty means the transcript was
    /// kept as `.forgetting` and running the forget again will retry.
    pub errors: Vec<String>,
    /// Whether the transcript itself is gone.
    pub complete: bool,
}

impl Report {
    fn count(&mut self, store: &str, n: usize) {
        self.removed.push((store.to_string(), n));
    }

    fn attempt(&mut self, store: &str, r: Result<usize>) -> usize {
        match r {
            Ok(n) => {
                self.count(store, n);
                n
            }
            Err(e) => {
                self.errors.push(format!("{store}: {e:#}"));
                0
            }
        }
    }
}

const FORGETTING: &str = "jsonl.forgetting";

/// Forget session `id` everywhere. `id` must be a whole id, never a prefix:
/// the caller resolves prefixes, because a prefix that grew a second match
/// between listing and deleting must not pick one.
pub fn forget(roots: &Roots, id: &str, graph: &dyn GraphRedactor) -> Result<Report> {
    // The archive's own rule, cap included: an id the archive mark would
    // refuse could never finish its "archive mark" step.
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid session id {id:?}"
    );
    let live = roots.sessions.join(format!("{id}.jsonl"));
    let parked = roots.sessions.join(format!("{id}.{FORGETTING}"));
    if live.exists() {
        std::fs::rename(&live, &parked)
            .with_context(|| format!("setting {} aside", live.display()))?;
    } else if !parked.exists() {
        anyhow::bail!("no session {id}");
    }
    let meta = Session::peek_meta(&parked);
    let mut report = Report {
        id: id.to_string(),
        ..Report::default()
    };

    // **A key is removed after everything keyed by it.** The outbox items'
    // ids key the learning store's mined-drafts ledger and the front door's
    // links, so the items are *found* here and removed only after both have
    // answered — removed first, a failure in between would leave a retry
    // with no ids to find those rows by, and it would report clean.
    let items = match matching_items(&roots.outbox, |v| field_is(v, "session_id", id)) {
        Ok(items) => items,
        Err(e) => {
            report.errors.push(format!("outbox: {e:#}"));
            Vec::new()
        }
    };
    // No writer lock of its own: the front door writes each record by temp
    // and rename, as `edit_items` does, so neither sees half of the other.
    report.attempt(
        "front-door requests",
        edit_items(&roots.requests, |v| {
            let nulled = null_fields(v, &["triage_session"], id);
            let unlinked = v
                .get_mut("outbox")
                .and_then(Value::as_array_mut)
                .is_some_and(|list| {
                    let before = list.len();
                    list.retain(|o| !o.as_str().is_some_and(|o| items.iter().any(|i| i == o)));
                    list.len() != before
                });
            nulled | unlinked
        }),
    );
    // `mecha mail draft` stamps the drafting session into the thread's
    // record. The thread is the owner's; only the pointer goes — removed, so
    // readers see it absent rather than null. Temp-and-rename, as there.
    report.attempt(
        "mail triage",
        edit_items(&roots.triage, |v| {
            v.as_object_mut().is_some_and(|m| {
                m.get(crate::mail_triage::DRAFT_SESSION)
                    .and_then(Value::as_str)
                    == Some(id)
                    && m.remove(crate::mail_triage::DRAFT_SESSION).is_some()
            })
        }),
    );
    // Found now, removed last — with the outbox items, for the same reason:
    // the workflows store is unlinked by these ids.
    let questions = match matching_items(&roots.questions, |v| field_is(v, "session_id", id)) {
        Ok(q) => q,
        Err(e) => {
            report.errors.push(format!("questions: {e:#}"));
            Vec::new()
        }
    };
    report.attempt("messages", purge_mailbox(&roots.messages, id));
    // The chat's picture records (IMAGE-DESIGN.md §6): its copy, and each
    // index entry it last advanced. One that cannot be read cannot be shown
    // to be this chat's, so it is kept and said.
    let scene = crate::scene::forget_assistant_chat(&roots.sessions, id);
    if let Ok(f) = &scene {
        if f.unreadable > 0 {
            report.residue.push(format!(
                "{} picture record(s) in the scene store could not be read, so they were kept",
                f.unreadable
            ));
        }
    }
    report.attempt("scene", scene.map(|f| f.removed).map_err(Into::into));
    // The chat's saved picture prompts, beside its transcript.
    let prompts = roots.sessions.join(format!("{id}.prompts.jsonl"));
    report.attempt(
        "picture prompts",
        match std::fs::remove_file(&prompts) {
            Ok(()) => Ok(1),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(anyhow::anyhow!("{}: {e}", prompts.display())),
        },
    );
    // The graph before the learning store: whether the session was distilled
    // is read from the ledger the learning purge is about to empty, and a
    // graph that failed this time must still be owed the episode next time.
    let distilled = match read_lines(&roots.learning.join("distilled.jsonl")) {
        Ok(lines) => lines.iter().any(|l| l.trim() == id),
        // Unknown is never clean: a ledger that cannot be read may name the
        // session, so the graph is owed the episode until it can be read.
        Err(e) => {
            report.errors.push(format!("distill ledger: {e:#}"));
            true
        }
    };
    let graph_failed = match graph.redact_session(id) {
        Ok(GraphOutcome::Redacted(n)) => {
            report.count("graph", n);
            report.residue.push(if n > 0 {
                "the graph keeps a tombstone naming this session id, so a nightly \
                 re-ingest cannot bring the episode back; its backups \
                 (~/.mecha-graph/*.bak and backups/) still hold the episode"
                    .into()
            } else {
                "the graph keeps a tombstone naming this session id, so a distill \
                 already in flight cannot add the conversation after the delete"
                    .into()
            });
            false
        }
        Ok(GraphOutcome::Absent) if distilled => {
            report.errors.push(
                "graph: the learning store says this session was distilled, but there \
                 is no graph to redact it from"
                    .into(),
            );
            true
        }
        Ok(GraphOutcome::Absent) => {
            report.count("graph", 0);
            false
        }
        Err(e) => {
            report.errors.push(format!("graph: {e:#}"));
            true
        }
    };
    purge_learning(roots, id, graph_failed, &mut report);
    report.attempt(
        "appraisals",
        with_lock(&roots.appraisals, || {
            let mut n = 0;
            for file in ["appraisals.jsonl", "scores.jsonl", "counterfactuals.jsonl"] {
                n += filter_jsonl(&roots.appraisals.join(file), |v| {
                    field_is(v, "session_id", id)
                })?;
            }
            Ok(n)
        }),
    );
    // The owner's mark on it (ruling 4D): a line in the sessions' marks
    // ledger, keyed by the session id — a trace like any other.
    let marks = crate::session::Marks::ledger(&roots.sessions);
    report.attempt(
        "session marks",
        with_lock(marks.parent().expect("the ledger has a directory"), || {
            filter_jsonl(&marks, |v| field_is(v, "session_id", id))
        }),
    );
    report.attempt(
        "closures",
        with_lock(&roots.closures, || {
            edit_jsonl(&roots.closures.join("closures.jsonl"), |v| {
                remove_from_array(v, "sessions", id)
            })
        }),
    );
    report.attempt(
        "trigger ledger",
        // The row stays: `runs.jsonl` *is* the schedule marker (`last_slots`
        // reads a trigger's last fired slot from it), and dropping the row
        // would rewind the schedule and fire that slot again — the briefing
        // sent twice. Only the pointer to the conversation leaves it.
        with_lock(&roots.triggers, || {
            edit_jsonl(&roots.triggers.join("runs.jsonl"), |v| {
                let Some(m) = v.as_object_mut() else {
                    return false;
                };
                if m.get("session_id").and_then(Value::as_str) != Some(id) {
                    return false;
                }
                m.remove("session_id");
                m.remove("summary");
                true
            })
        }),
    );
    report.attempt(
        "workflows",
        with_lock(&roots.workflows, || {
            edit_items(&roots.workflows, |v| {
                // The pointers, and the board's event log: `start_task`
                // records the session as a `started` event's `detail`, and
                // the owner-disposition reader resolves the session back out
                // of it — so nulling `session_id` alone would leave the
                // deleted conversation credited with the owner's close.
                let nulled = null_fields(v, &["session_id", "session"], id);
                // Blanked, not dropped: the board resolves which run an owner's
                // close disposed of *positionally* — the latest `started`
                // before it — so dropping this one would hand its close to the
                // run before. A blank detail resolves to unknown.
                let dropped = v
                    .get_mut("events")
                    .and_then(Value::as_array_mut)
                    .is_some_and(|events| {
                        let mut changed = false;
                        for e in events.iter_mut() {
                            if e.get("detail")
                                .and_then(Value::as_str)
                                .is_some_and(|d| d.contains(id))
                            {
                                e["detail"] = Value::String(String::new());
                                changed = true;
                            }
                        }
                        changed
                    });
                // And its links to the drafts and questions this delete
                // removed: the board reads a link to a missing item as
                // `unreadable`, which it files as urgent — forever.
                let unlinked = ["outbox", "questions"].into_iter().fold(false, |acc, key| {
                    let gone: &[String] = if key == "outbox" { &items } else { &questions };
                    v.get_mut(key)
                        .and_then(Value::as_array_mut)
                        .is_some_and(|list| {
                            let before = list.len();
                            list.retain(|x| {
                                !x.as_str().is_some_and(|x| gone.iter().any(|g| g == x))
                            });
                            list.len() != before
                        })
                        | acc
                });
                nulled | dropped | unlinked
            })
        }),
    );
    report.attempt(
        "slack threads",
        remove_items(&roots.home.join("slack").join("threads"), |v| {
            field_is(v, "session_id", id)
        })
        .map(|v| v.len()),
    );
    report.attempt(
        "regression pins",
        filter_lines(&roots.regression_pins, |l| l.trim() == id),
    );
    // The file's *name* is the session id, so a failure here is a trace.
    report.attempt(
        "live-session marker",
        remove_if_present(&roots.messages.join(".agents").join(format!("{id}.json"))),
    );

    // **The keys last.** The outbox items and the questions are the ids the
    // mined-drafts ledger, the front door and the workflows are unlinked by,
    // so they go only once every one of those has answered: a failure above
    // keeps them, and the retry can still find those rows by them.
    if report.errors.is_empty() {
        // The drafts, then the mined-drafts ledger lines naming them — in that
        // order, so a failure between can leave an opaque item id behind but
        // never a draft whose "already mined" mark is gone, which tonight's
        // reflect would mine into a new lesson.
        let removed = remove_items(&roots.outbox, |v| field_is(v, "session_id", id));
        let ok = removed.is_ok();
        report.attempt("outbox", removed.map(|v| v.len()));
        if ok {
            report.attempt(
                "mined-drafts ledger",
                with_lock(&roots.learning, || {
                    filter_lines(&roots.learning.join("mined_outbox.jsonl"), |l| {
                        items.iter().any(|i| i == l.trim())
                    })
                }),
            );
            // The harness's forecasts of the owner's act on those drafts
            // (`forecast`), keyed by the session and by the item: a trace
            // like any other.
            let forecasts = crate::forecast::ledger(&roots.outbox);
            report.attempt(
                "draft forecasts",
                // Under the lock `forecast::record` appends with.
                with_lock(forecasts.parent().expect("a directory"), || {
                    filter_jsonl(&forecasts, |v| {
                        field_is(v, "session_id", id)
                            || v["item_id"]
                                .as_str()
                                .is_some_and(|x| items.iter().any(|i| i == x))
                    })
                }),
            );
        }
        report.attempt(
            "questions",
            remove_items(&roots.questions, |v| field_is(v, "session_id", id)).map(|v| v.len()),
        );
    } else {
        report
            .errors
            .push("outbox and questions: kept until the stores above finish, for the retry".into());
    }

    // Before the backstop, which also reads file *names*: the mark's name is
    // the session id.
    report.attempt(
        "archive mark",
        crate::archive::unarchive(&roots.sessions, id).map(usize::from),
    );

    // The backstop under the enumeration: every store it walked, searched
    // for the id afterwards. A field this file was never taught about is
    // how a trace survives, and on review it was, repeatedly — a workflow
    // event's `detail`, a message's `delivered_to`, a triage record's
    // `draft_session`. What is found is said, file by file, rather than
    // reported as a clean delete.
    //
    // Only once every store answered: a partial delete keeps its keys on
    // purpose (the outbox items, the reflections, the distill line the graph
    // is owed), and naming those as traces delete "does not know" would
    // contradict the errors that say why they were kept. The retry that
    // finishes the delete runs the backstop then.
    if report.errors.is_empty() {
        let (named, unread) = still_naming(roots, id);
        for path in named {
            report.residue.push(format!(
                "{} still names this session, in a field delete does not know",
                path.display()
            ));
        }
        for path in unread {
            report.residue.push(format!(
                "{} could not be read, so it may still name this session",
                path.display()
            ));
        }
    }

    match &meta {
        Some(meta) => purge_workspace(roots, id, meta, &mut report),
        // Unknown is never clean: without the header nothing says where the
        // conversation worked, so its files cannot be found — and once the
        // transcript goes, never will be. Said, rather than skipped.
        None => report.residue.push(
            "the transcript's header could not be read, so its workspace could not be \
             found; anything it wrote under ~/.mecha/work/ was kept"
                .into(),
        ),
    }

    // The document cache is keyed by a PDF's hash, not by the session that
    // read it, so delete cannot find this conversation's entries in it. Said
    // rather than left silent: a report that reads `complete` while a
    // forgotten conversation's PDFs sit extracted on disk is the leak this
    // module's header names (found on review of #404).
    // Where the cache really is: `MECHA_DOCUMENTS_DIR` relocates it, and a
    // check that looked only under ~/.mecha would report a relocated cache
    // clean (found on review).
    if dir_has_entries(&roots.documents) {
        report.residue.push(format!(
            "{} keeps the text of every PDF any conversation read, by file rather than \
             by conversation, so this one's were kept — `mecha document forget <file>` \
             removes one, `mecha document prune --days 0` all of them",
            roots.documents.display()
        ));
    }

    if report.errors.is_empty() {
        std::fs::remove_file(&parked).with_context(|| format!("removing {}", parked.display()))?;
        report.complete = true;
    }
    Ok(report)
}

/// Sessions a forget set aside and did not finish. Each still holds the
/// whole conversation, and nothing lists it — `mecha doctor` reads this so an
/// incomplete delete cannot go quiet once the page that reported it closes.
pub fn unfinished(sessions: &Path) -> Result<Vec<String>> {
    // A store that cannot be listed is a finding, never an empty answer —
    // this is the doctor's one view of a half-finished delete.
    let read = match std::fs::read_dir(sessions) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("listing {}", sessions.display())),
    };
    let suffix = format!(".{FORGETTING}");
    let mut out: Vec<String> = read
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(&suffix))
                .map(str::to_string)
        })
        .collect();
    out.sort();
    Ok(out)
}

/// The learning store, under its writer lock. With `keep_distilled`, the
/// distill ledger keeps the session: the graph did not answer for it, and
/// the retry must still know it has an episode to redact.
fn purge_learning(roots: &Roots, id: &str, keep_distilled: bool, report: &mut Report) {
    let root = &roots.learning;
    // The reflections are the key every ledger below is found by, so they are
    // read first and removed last: a step that fails in between leaves them
    // in place, and the retry finds everything again. Read without the lock
    // — nothing adds a reflection for a session already set aside.
    let mut ids: HashSet<String> = HashSet::new();
    let mut texts: Vec<String> = Vec::new();
    match read_lines(&root.join("reflections.jsonl")) {
        Ok(lines) => {
            for line in lines {
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if field_is(&v, "session_id", id) {
                    if let Some(r) = v.get("id").and_then(Value::as_str) {
                        ids.insert(r.to_string());
                    }
                    if let Some(t) = v.get("reflexion_text").and_then(Value::as_str) {
                        texts.push(t.to_string());
                    }
                }
            }
        }
        Err(e) => {
            report.errors.push(format!("learning store: {e:#}"));
            return;
        }
    }

    // Sibling stores, each on its own existence: a home that appraises but
    // never reflected has comparisons and no learning store.
    let errors_before = report.errors.len();
    report.attempt(
        "harness candidates",
        with_lock(&roots.harness, || {
            edit_items(&roots.harness.join("candidates"), |v| scrub_deep(v, id))
        }),
    );
    report.attempt(
        "comparisons",
        with_lock(&roots.comparisons, || {
            filter_jsonl(&roots.comparisons.join("comparisons.jsonl"), |v| {
                let p = v.get("pointers");
                p.and_then(|p| p.get("session_id")).and_then(Value::as_str) == Some(id)
                    || p.and_then(|p| p.get("reflection_id"))
                        .and_then(Value::as_str)
                        .is_some_and(|r| ids.contains(r))
            })
        }),
    );
    let siblings_answered = report.errors.len() == errors_before;

    if !root.is_dir() {
        report.count("reflections", 0);
        return;
    }
    let result = with_lock(root, || {
        // An interrupted rule change first (`LearningStore::commit_rules`),
        // before anything below reads the rules or the proposals. Finishing
        // it writes rules and a proposal that the purges below then see.
        // Left pending, recovery would run *after* this forget and write back
        // a proposal `purge_proposals` had just removed. A record recovery
        // cannot finish is set aside, and scrubbed with the others below.
        if !ids.is_empty() {
            if let Err(e) = crate::learning::LearningStore::open(root.clone())
                .and_then(|s| s.resume_interrupted())
            {
                report.residue.push(format!(
                    "an interrupted rule change could not be finished before the purge ({e:#}); \
                     its record was scrubbed in place"
                ));
            }
        }
        report.count(
            "interrupted rule changes",
            purge_commit_records(root, &ids)?,
        );
        let mut ledgers = filter_lines(&root.join("mined.jsonl"), |l| l.trim() == id)?;
        if !keep_distilled {
            ledgers += filter_lines(&root.join("distilled.jsonl"), |l| l.trim() == id)?;
        }
        // Not `mined_outbox.jsonl`: it moves with the outbox items it names
        // (see `forget`), because a ledger line gone while its draft stays
        // makes the draft mineable again — and a fresh `writing` lesson from
        // a forgotten conversation would ride in every future prompt.
        report.count("learning ledgers", ledgers);

        let by_reflexion = |v: &Value| {
            v.get("reflexion_id")
                .and_then(Value::as_str)
                .is_some_and(|r| ids.contains(r))
        };
        let mut validation = filter_jsonl(&root.join("validations.jsonl"), by_reflexion)?;
        validation += filter_jsonl(&root.join("validation-attempts.jsonl"), by_reflexion)?;
        report.count("validation ledger", validation);
        report.count(
            "artifact probes",
            purge_probe_receipts(&root.join("artifact-probes"), id, &ids)?,
        );

        report.count("proposals", purge_proposals(&root.join("proposals"), &ids)?);
        report.count("learned rules", purge_rules(&root.join("rules"), &ids)?);
        report.count(
            "learning logs",
            purge_logs(&root.join("logs"), id, &ids, &texts)?,
        );
        // The key last, and only once every store keyed by it answered.
        let n = if siblings_answered {
            filter_jsonl(&root.join("reflections.jsonl"), |v| {
                field_is(v, "session_id", id)
            })?
        } else {
            0
        };
        report.count("reflections", n);

        if root.join(".git").exists() && (n > 0 || ledgers > 0) {
            report.residue.push(format!(
                "the learning store's git history ({}) still holds what was removed \
                 from it; the store no longer uses git, and deleting that directory \
                 removes the history",
                root.join(".git").display()
            ));
        }
        Ok(0)
    });
    if let Err(e) = result {
        report.errors.push(format!("learning store: {e:#}"));
    }
}

/// Whether `dir` exists and holds anything. Unreadable counts as holding
/// something: unknown is never clean.
fn dir_has_entries(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Artifact-task probe receipts (`probe.rs`, one file per repeat): each
/// names the session replayed and the reflection measured, so one naming
/// either is the forgotten conversation's and goes whole. A receipt that does
/// not parse is left, and the residue scan names it if it holds the id.
fn purge_probe_receipts(dir: &Path, id: &str, ids: &HashSet<String>) -> Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut n = 0;
    for path in json_files(dir)? {
        let Some(v) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            continue;
        };
        let names_reflection = v
            .get("reflection_id")
            .and_then(Value::as_str)
            .is_some_and(|r| ids.contains(r));
        if field_is(&v, "session_id", id) || names_reflection {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// The rule-change records (`commit.json`, and each
/// `commit.unfinished.<when>.json` recovery set aside): the proposals'
/// scrub, applied to the change and to the proposal it carries. A record is
/// the owner's evidence of an interrupted change, so it is scrubbed rather
/// than removed, even when nothing it names is left.
fn purge_commit_records(root: &Path, ids: &HashSet<String>) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut n = 0;
    for path in json_files(root)? {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name != crate::learning::COMMIT_FILE
            && !name.starts_with(crate::learning::UNFINISHED_COMMIT_PREFIX)
        {
            continue;
        }
        let Some(mut v) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            continue;
        };
        let mut changed = scrub_change(&mut v, ids);
        if let Some(p) = v.get_mut("proposal") {
            changed |= scrub_change(p, ids);
        }
        if changed {
            write_replacing(&path, serde_json::to_string_pretty(&v)?.as_bytes())?;
            n += 1;
        }
    }
    Ok(n)
}

/// `rules`, `rules_before` and `reflexion_ids` of one change or proposal,
/// scrubbed of the forgotten reflections.
fn scrub_change(v: &mut Value, ids: &HashSet<String>) -> bool {
    let mut changed = false;
    for key in ["rules", "rules_before"] {
        if let Some(rules) = v.get_mut(key).and_then(Value::as_array_mut) {
            changed |= scrub_rules(rules, ids);
        }
    }
    if let Some(list) = v.get_mut("reflexion_ids").and_then(Value::as_array_mut) {
        let before = list.len();
        list.retain(|r| !r.as_str().is_some_and(|r| ids.contains(r)));
        changed |= list.len() != before;
    }
    changed
}

/// Proposals: the forgotten reflections leave `reflexion_ids`, and a
/// proposal left arguing from nothing goes — its rule text was drawn from
/// them and from nothing else. **Every** proposal's `rules` and
/// `rules_before` are scrubbed by [`purge_rules`]' own rule, whether or not
/// it argued from the forgotten reflections: a proposal carries whole rule
/// values, so a rule removed from the live set would otherwise survive
/// verbatim in a snapshot — and `mecha proposals accept` writes a proposal's
/// `rules` back wholesale, which would put it back.
fn purge_proposals(dir: &Path, ids: &HashSet<String>) -> Result<usize> {
    if ids.is_empty() || !dir.is_dir() {
        return Ok(0);
    }
    let mut n = 0;
    for path in json_files(dir)? {
        let mut v: Value = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };
        let mut changed = false;
        for key in ["rules", "rules_before"] {
            if let Some(rules) = v.get_mut(key).and_then(Value::as_array_mut) {
                changed |= scrub_rules(rules, ids);
            }
        }
        let mut emptied = false;
        if let Some(list) = v.get_mut("reflexion_ids").and_then(Value::as_array_mut) {
            let before = list.len();
            list.retain(|r| !r.as_str().is_some_and(|r| ids.contains(r)));
            if list.len() != before {
                changed = true;
                emptied = list.is_empty();
            }
        }
        if !changed {
            continue;
        }
        n += 1;
        if emptied {
            std::fs::remove_file(&path)?;
        } else {
            write_replacing(&path, serde_json::to_string_pretty(&v)?.as_bytes())?;
        }
    }
    Ok(n)
}

/// [`purge_rules`]' rule over JSON rule values: the forgotten ids leave each
/// rule's `sources`, and a rule whose every source was forgotten goes.
fn scrub_rules(rules: &mut Vec<Value>, ids: &HashSet<String>) -> bool {
    let mut changed = false;
    rules.retain_mut(|rule| {
        let Some(sources) = rule.get_mut("sources").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = sources.len();
        sources.retain(|s| !s.as_str().is_some_and(|s| ids.contains(s)));
        if sources.len() == before {
            return true;
        }
        changed = true;
        !sources.is_empty()
    });
    changed
}

/// Learned rules: the forgotten reflections leave every rule's `sources`,
/// and a rule whose every source was forgotten is removed outright.
///
/// **Removed, not retired.** A retired rule stays in the file and is quoted
/// to the learner as "measured harmful — never restate or re-derive"; a rule
/// whose evidence was deleted was never measured harmful, and keeping its
/// text would keep the forgotten conversation's lesson in the store. The
/// owner's own rules (`*.user.toml`) carry no sources and are never touched.
fn purge_rules(dir: &Path, ids: &HashSet<String>) -> Result<usize> {
    if ids.is_empty() || !dir.is_dir() {
        return Ok(0);
    }
    let mut n = 0;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if !path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.ends_with(".learned.toml"))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let mut file: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let Some(rules) = file.get_mut("rules").and_then(toml::Value::as_array_mut) else {
            continue;
        };
        let mut changed = false;
        rules.retain_mut(|rule| {
            let Some(sources) = rule.get_mut("sources").and_then(toml::Value::as_array_mut) else {
                return true;
            };
            let before = sources.len();
            sources.retain(|s| !s.as_str().is_some_and(|s| ids.contains(s)));
            if sources.len() == before {
                return true;
            }
            changed = true;
            n += 1;
            !sources.is_empty()
        });
        if changed {
            write_replacing(&path, toml::to_string_pretty(&file)?.as_bytes())?;
        }
    }
    Ok(n)
}

/// The nightly scripts' stdout: `reflect` prints each lesson it wrote and
/// `distill` each session it pushed. A line naming the session, one of its
/// reflections, or quoting a lesson drawn from it goes.
fn purge_logs(dir: &Path, id: &str, ids: &HashSet<String>, texts: &[String]) -> Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    // A lesson short enough to be a common phrase would take unrelated lines
    // with it; one that long is a sentence, and a sentence is the lesson.
    let texts: Vec<&str> = texts
        .iter()
        .map(|t| t.trim())
        .filter(|t| t.len() >= 24)
        .collect();
    let mut n = 0;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        n += filter_lines(&path, |l| {
            l.contains(id)
                || ids.iter().any(|r| l.contains(r.as_str()))
                || texts.iter().any(|t| l.contains(t))
        })?;
    }
    Ok(n)
}

/// Every recipient's messages the session sent or received, under that
/// recipient's lock.
fn purge_mailbox(root: &Path, id: &str) -> Result<usize> {
    let read = match std::fs::read_dir(root) {
        Ok(r) => r,
        // Absent is nothing to purge; unreadable is a finding — an error, so
        // the transcript is kept and the retry can finish once it can read.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("listing {}", root.display())),
    };
    let mut n = 0;
    for entry in read {
        let dir = entry?.path();
        let hidden = dir
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.starts_with('.'));
        if !dir.is_dir() || hidden {
            continue;
        }
        // Both ends: what it sent, and what it received — `claim_pending`
        // writes the claiming session into `delivered_to`, and a delivered
        // message is kept (up to `keep_resolved`), not transient. Removed
        // rather than un-pointed: a message with `delivered_to` cleared reads
        // as undelivered and would be handed to the next session.
        n += remove_items(&dir, |v| {
            field_is(v, "from_session", id) || field_is(v, "delivered_to", id)
        })?
        .len();
    }
    Ok(n)
}

/// The session's jail and spill directory — only when mecha made it and no
/// other transcript names it.
///
/// A web chat's jail is keyed by the chat's *key*, and keys are reused: `main`
/// across every restart, and a resumed chat keeps its original workspace. A
/// voice call's is one directory for every call. So "this session's
/// directory" is a claim to check against every other header, never a path
/// to derive; a directory another conversation also wrote in stays, and the
/// report says so. A workspace outside `~/.mecha/work` is the owner's
/// project, and its files are the owner's, never this module's.
fn purge_workspace(roots: &Roots, id: &str, meta: &SessionMeta, report: &mut Report) {
    let work = roots.home.join("work");
    let ws = meta
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| meta.workspace.clone());
    let work = work.canonicalize().unwrap_or(work);
    // Mecha's own jail, or the owner's project — whose files are never this
    // module's. The spill directory is mecha's either way (the capped tool
    // output of whatever ran there), so it follows the ownership rule below
    // even when the workspace itself is kept.
    let ours = ws.starts_with(&work) && ws != work;
    let others = match Session::list(&roots.sessions) {
        Ok(all) => all
            .into_iter()
            .filter(|(m, _)| m.id != id)
            .filter(|(m, _)| {
                m.workspace
                    .canonicalize()
                    .unwrap_or_else(|_| m.workspace.clone())
                    == ws
            })
            .count(),
        Err(e) => {
            report
                .errors
                .push(format!("workspace: listing sessions: {e:#}"));
            return;
        }
    };
    let spill = crate::tool::session_spill_dir_under(&roots.home, &ws);
    if !ours {
        report.residue.push(format!(
            "files the conversation wrote in {} are yours and were kept",
            meta.workspace.display()
        ));
    }
    if others > 0 {
        report.count("workspace", 0);
        report.residue.push(format!(
            "{} is shared with {others} other conversation(s); its files{} were kept",
            meta.workspace.display(),
            if spill.exists() {
                format!(" and its spilled tool output ({})", spill.display())
            } else {
                String::new()
            }
        ));
        return;
    }
    let dirs: Vec<PathBuf> = if ours {
        vec![ws.clone(), spill]
    } else {
        vec![spill]
    };
    let mut n = 0;
    for dir in dirs {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => n += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => report
                .errors
                .push(format!("workspace {}: {e}", dir.display())),
        }
    }
    report.count("workspace", n);
}

/// Every file in the stores [`forget`] purges whose bytes still contain `id`.
/// Skips each store's `.lock` and the learning store's legacy `.git`, which
/// the report names on its own.
/// Also answers what could not be read: unknown is never clean, so a store
/// the search could not open is said, never counted as holding nothing.
fn still_naming(roots: &Roots, id: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut hits = Vec::new();
    let mut unread = Vec::new();
    let mut stack: Vec<PathBuf> = [
        &roots.outbox,
        &roots.questions,
        &roots.messages,
        &roots.learning,
        &roots.harness,
        &roots.appraisals,
        &roots.comparisons,
        &roots.closures,
        &roots.triggers,
        &roots.workflows,
        &roots.requests,
        &roots.triage,
    ]
    .into_iter()
    .cloned()
    .chain([
        roots.home.join("slack").join("threads"),
        roots.regression_pins.clone(),
        // Named, not purged: a Slack remote attach record keeps the session
        // and its workspace past detach, and a crashed task run leaves its
        // marker (`runmarker`).
        roots.home.join("remote"),
        roots.home.join("taskruns"),
        // Other conversations' transcripts: a message this one sent was
        // delivered into its recipient's conversation, id and text both.
        // Their words are theirs, so named, never edited.
        roots.sessions.clone(),
    ])
    .collect();
    let mut seen = HashSet::new();
    while let Some(path) = stack.pop() {
        if !seen.insert(path.clone()) {
            continue; // the harness store lives inside the learning store
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // The store's lock, the legacy history the report names on its own,
        // and the transcript being forgotten, set aside and removed after.
        if name == ".git" || name == ".lock" || name == format!("{id}.{FORGETTING}") {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                unread.push(path);
                continue;
            }
        };
        if meta.is_dir() {
            match std::fs::read_dir(&path) {
                Ok(read) => {
                    for entry in read {
                        match entry {
                            Ok(e) => stack.push(e.path()),
                            Err(_) => unread.push(path.clone()),
                        }
                    }
                }
                Err(_) => unread.push(path),
            }
        } else if meta.is_file() {
            // A name is a copy too: a marker keyed by the session id holds it
            // only in its name.
            match std::fs::read(&path) {
                Ok(_) if name.contains(id) => hits.push(path),
                Ok(b) if b.windows(id.len()).any(|w| w == id.as_bytes()) => hits.push(path),
                Ok(_) => {}
                Err(_) => unread.push(path),
            }
        }
    }
    hits.sort();
    unread.sort();
    unread.dedup();
    (hits, unread)
}

// ─── Store primitives ───────────────────────────────────────────────────────

/// Hold `<root>/.lock` — the writer lock every JSONL store here takes, on the
/// same file, so a forget and a nightly writer serialise. A root that does
/// not exist holds nothing to purge, and is not created.
fn with_lock<T: Default>(root: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    use std::os::unix::io::AsRawFd;
    if !root.is_dir() {
        return Ok(T::default());
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".lock"))
        .with_context(|| format!("opening {}", root.join(".lock").display()))?;
    // SAFETY: flock on an fd we own, held open until `file` drops below.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error()).context("locking the store");
    }
    let out = f();
    drop(file);
    out
}

/// Remove one file; absent is nothing to remove, any other failure is one.
fn remove_if_present(path: &Path) -> Result<usize> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(1),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// A file's lines; absent is none.
fn read_lines(path: &Path) -> Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t.lines().map(str::to_string).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The ids (file stems) of the `*.json` items in `dir` the predicate picks,
/// touching nothing — the read half of [`remove_items`].
fn matching_items(dir: &Path, pick: impl Fn(&Value) -> bool) -> Result<Vec<String>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for path in json_files(dir)? {
        let hit = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .is_some_and(|v| pick(&v));
        if hit {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
    }
    Ok(out)
}

fn field_is(v: &Value, key: &str, id: &str) -> bool {
    v.get(key).and_then(Value::as_str) == Some(id)
}

/// Drop `id` from the string array under `key`. `true` when the row changed.
fn remove_from_array(v: &mut Value, key: &str, id: &str) -> bool {
    let Some(list) = v.get_mut(key).and_then(Value::as_array_mut) else {
        return false;
    };
    let before = list.len();
    list.retain(|s| s.as_str() != Some(id));
    list.len() != before
}

/// Null every field named in `keys`, at any depth, whose value is `id`. For
/// records that *point at* a session and are the owner's own objects.
fn null_fields(v: &mut Value, keys: &[&str], id: &str) -> bool {
    let mut changed = false;
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if keys.contains(&k.as_str()) && child.as_str() == Some(id) {
                    *child = Value::Null;
                    changed = true;
                } else {
                    changed |= null_fields(child, keys, id);
                }
            }
        }
        Value::Array(list) => {
            for child in list {
                changed |= null_fields(child, keys, id);
            }
        }
        _ => {}
    }
    changed
}

/// Take `id` out of a record, at any depth, whatever shape it is in: an
/// array entry that holds it anywhere inside — a bare id, a caveat string
/// naming it, an object whose field is it — goes whole, and a lone string
/// field containing it is blanked. For a measurement record, whose session
/// ids sit in `episodes` lists, in `divergence_detail` entries' scalar
/// `episode`, in `replay_caveats` text and inside opaque `arm_receipts`: an
/// entry about a forgotten episode is that episode's, and goes with it.
fn scrub_deep(v: &mut Value, id: &str) -> bool {
    fn holds(v: &Value, id: &str) -> bool {
        match v {
            Value::String(s) => s.contains(id),
            Value::Array(list) => list.iter().any(|c| holds(c, id)),
            Value::Object(map) => map.values().any(|c| holds(c, id)),
            _ => false,
        }
    }
    match v {
        Value::Array(list) => {
            let before = list.len();
            list.retain(|c| !holds(c, id));
            list.len() != before
        }
        Value::Object(map) => {
            let mut changed = false;
            for child in map.values_mut() {
                if child.as_str().is_some_and(|s| s.contains(id)) {
                    *child = Value::String(String::new());
                    changed = true;
                } else {
                    changed |= scrub_deep(child, id);
                }
            }
            changed
        }
        _ => false,
    }
}

fn json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
    Ok(out)
}

/// Delete every `*.json` item in `dir` the predicate picks, under the
/// store's lock. Returns the removed items' file stems (their ids). An item
/// that does not parse is kept: it cannot be shown to be the session's, and
/// unknown is never permission to delete.
fn remove_items(dir: &Path, pick: impl Fn(&Value) -> bool) -> Result<Vec<String>> {
    with_lock(dir, || {
        let mut out = Vec::new();
        for path in json_files(dir)? {
            let Some(v) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            else {
                continue;
            };
            if pick(&v) {
                std::fs::remove_file(&path)?;
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
        Ok(out)
    })
}

/// Rewrite every `*.json` item in `dir` the edit changes. Caller holds the
/// lock. Returns how many changed.
fn edit_items(dir: &Path, edit: impl Fn(&mut Value) -> bool) -> Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut n = 0;
    for path in json_files(dir)? {
        let Some(mut v) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            continue;
        };
        if edit(&mut v) {
            write_replacing(&path, serde_json::to_string_pretty(&v)?.as_bytes())?;
            n += 1;
        }
    }
    Ok(n)
}

/// Drop the JSONL rows `drop` picks; every other line is kept byte for byte,
/// including one that does not parse. Caller holds the lock.
fn filter_jsonl(path: &Path, mut drop: impl FnMut(&Value) -> bool) -> Result<usize> {
    filter_lines(path, |line| {
        serde_json::from_str::<Value>(line).is_ok_and(|v| drop(&v))
    })
}

/// Rewrite the JSONL rows `edit` changes, keeping every other line as it was.
fn edit_jsonl(path: &Path, mut edit: impl FnMut(&mut Value) -> bool) -> Result<usize> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    for line in text.lines() {
        match serde_json::from_str::<Value>(line) {
            Ok(mut v) => {
                if edit(&mut v) {
                    out.push_str(&serde_json::to_string(&v)?);
                    n += 1;
                } else {
                    out.push_str(line);
                }
            }
            Err(_) => out.push_str(line),
        }
        out.push('\n');
    }
    if n > 0 {
        write_replacing(path, out.as_bytes())?;
    }
    Ok(n)
}

/// Drop the lines `drop` picks. Absent file: nothing to drop.
fn filter_lines(path: &Path, mut drop: impl FnMut(&str) -> bool) -> Result<usize> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    for line in text.lines() {
        if !line.trim().is_empty() && drop(line) {
            n += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if n > 0 {
        write_replacing(path, out.as_bytes())?;
    }
    Ok(n)
}

/// Temp sibling and rename, with the original's permissions: every store
/// here is read without a lock by something, and must be whole at every
/// instant — and owner-only stays owner-only.
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("forget.tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests;
