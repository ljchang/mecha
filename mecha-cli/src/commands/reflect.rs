//! `mecha reflect` — mine recorded sessions for the user stepping in, and turn
//! each intervention into a reflection.
//!
//! The behavior half of the self-learning system needs nothing the harness
//! does not already record: a mid-run steer, an approval denial, and a
//! corrective follow-up turn are all in the session JSONL. This command reads
//! sessions that have not been mined yet, extracts those moments (pure,
//! unit-tested in core), asks a model for the reusable lesson behind each, and
//! appends the results to `~/.mecha/learning/reflections.jsonl`.
//!
//! Idempotent by design: mined session ids are recorded, so running it nightly
//! (or after every session) only ever pays for the new ones.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::learning::{
    evidence_for, extract_interventions, Evidence, Intervention, LearningStore, Origin, Reflector,
    Trigger,
};
use mecha_core::session::Session;
use std::path::{Path, PathBuf};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Directory of session transcripts. Defaults to the standard location.
    #[arg(long)]
    pub sessions_dir: Option<PathBuf>,

    /// List what would be mined without calling a model or writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Mine at most this many sessions this run.
    #[arg(long)]
    pub limit: Option<usize>,

    /// One-shot backfill: re-mine the sessions whose reflections the
    /// provenance gate excluded, through the clean-evidence path — the
    /// user's own words and tool names, with the tainted excerpts withheld.
    /// Idempotent: an intervention already carrying a user-turns reflection
    /// is skipped, and clean-covered interventions are never re-mined.
    #[arg(long)]
    pub remine_untrusted: bool,

    /// One-shot backfill: give the reflections mined before the situation
    /// field a situation, recomputed from their transcripts — the tool
    /// window, surface and workspace — with no model call. A reflection
    /// whose intervention cannot be found once in its transcript stays
    /// without one. Idempotent: a reflection that has a situation is never
    /// touched. `--dry-run` reports what would be written.
    #[arg(long, conflicts_with_all = ["remine_untrusted", "limit"])]
    pub backfill_situations: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let sessions_dir = match &args.sessions_dir {
        Some(dir) => dir.clone(),
        None => Session::default_dir()?,
    };
    let store = LearningStore::open(LearningStore::default_root()?)?;
    if args.backfill_situations {
        return backfill_situations(&store, &sessions_dir, args.dry_run);
    }
    // The writer lock, taken *before* reading what has been mined — that read
    // is where the race lives now that a session_end hook fires a detached
    // reflect at every close: two closes in quick succession must not both
    // see the same session as unmined. Blocking is right: the second pass
    // waits, re-reads, finds nothing left, and exits. A dry run only reads.
    let _lock = if args.dry_run {
        None
    } else {
        Some(store.lock()?)
    };
    let mined = store.mined_sessions()?;

    // The backfill re-visits exactly the sessions whose lessons the gate
    // excluded; its dedup key is (session, intervention text) against the
    // user-turns reflections already on file, so running it twice is free.
    let excluded_sessions: std::collections::HashSet<String> = if args.remine_untrusted {
        store
            .reflexions()?
            .iter()
            .filter(|r| r.origin != Origin::Clean && !r.session_id.is_empty())
            .map(|r| r.session_id.clone())
            .collect()
    } else {
        Default::default()
    };
    let already_user_turns: std::collections::HashSet<(String, String)> = if args.remine_untrusted {
        store
            .reflexions()?
            .iter()
            .filter(|r| r.evidence == Evidence::UserTurns)
            .map(|r| (r.session_id.clone(), r.intervention.clone()))
            .collect()
    } else {
        Default::default()
    };

    // `list_counting`, so the reconcile below can say when a cited
    // transcript is one the store could not read rather than one deleted.
    let (sessions, unreadable_sessions) = Session::list_counting(&sessions_dir)?;
    let paths: std::collections::HashMap<String, PathBuf> = sessions
        .iter()
        .map(|(meta, path)| (meta.id.clone(), path.clone()))
        .collect();
    // Test and stray experiment sessions are the harness measuring itself,
    // not the owner's work: never mined (`session::split_admitted`),
    // and counted aloud so a skip is not mistaken for an empty store.
    // Skipped sessions are never marked mined, so this is every test or
    // experiment session in the store, reprinted each pass — worded so, and
    // deliberately not marked: a session mislabelled `Test` must stay
    // re-admittable once the label is fixed.
    let candidates: Vec<_> = sessions
        .into_iter()
        .filter(|(meta, _)| {
            if args.remine_untrusted {
                excluded_sessions.contains(&meta.id)
            } else {
                !mined.contains(&meta.id)
            }
        })
        .collect();
    let (mut todo, skipped) = mecha_core::session::split_admitted(candidates);
    if skipped > 0 {
        println!("passing over {skipped} test or experiment session(s) in the store");
    }
    if let Some(limit) = args.limit {
        todo.truncate(limit);
    }

    // The outbox pass mines a different kind of correction: a `sent` item
    // whose released arguments differ from the drafted ones is the user
    // editing mecha's writing, recorded structurally by `mecha outbox edit`.
    // Open non-creating: reflect must not conjure an outbox as a side effect.
    //
    // **Messages only, and this is a security filter rather than a tidiness
    // one.** A publish's arguments are a path and a visibility flag, so its
    // diff is a changed directory name — and a `writing`-domain reflection
    // rides in every future run's system prompt inside the cached prefix. That
    // is the longest-half-life path anything in this project has, and teaching
    // it voice rules from filesystem noise is the same mistake as learning from
    // `"Blocked by a hook:"`: machine bookkeeping read as a human correction.
    // The filter is structural, before any prompt is built.
    let outbox = mecha_core::outbox::OutboxStore::open_existing_default();
    let outbox_mined = store.mined_outbox()?;
    //
    // **And a rejection the owner gave a reason for** (`APPRAISAL-WIRING-
    // DESIGN.md` R16a): the reason reaches the reflector as an owner
    // correction, on the same message-only, model-authored terms
    // (`OutboxItem::rejection_reason`). One mined-ledger for both: an item
    // is sent or rejected, never both, so its id cannot be mined twice.
    let outbox_todo: Vec<_> = match &outbox {
        Some(ob) => ob
            .items()?
            .into_iter()
            .filter(|i| {
                (i.mineable_as_writing() || i.rejection_reason().is_some())
                    && !outbox_mined.contains(&i.id)
            })
            .collect(),
        None => Vec::new(),
    };

    // Every pass, before anything is mined or batched: the workspace on
    // each recorded situation against the run record. Not a flag a human
    // runs once after installing — the nightly's `learn --auto` follows
    // this pass, and a row stamped with a jail before the key existed
    // would have scoped a rule to nowhere while the flag waited to be run
    // (found on review). Free when there is nothing to apply.
    reconcile_recorded_keys(&store, &paths, unreadable_sessions, args.dry_run)?;

    if todo.is_empty() && outbox_todo.is_empty() {
        println!("nothing to mine: every session and sent draft is already reflected on");
        return Ok(());
    }

    // The reflector is only built when something needs it, so --dry-run and
    // the nothing-new case work with no provider configured at all.
    let reflector = if args.dry_run {
        None
    } else {
        let cwd = std::env::current_dir().context("cannot determine the working directory")?;
        let cfg = Config::load(&cwd)?;
        let (name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
        let provider = mecha_core::provider::build(provider_cfg)?;
        let model = global.model.clone().or_else(|| provider_cfg.model.clone());
        let reflector = Reflector::new(provider, model);
        eprintln!("reflecting with {} ({name})", reflector.model());
        Some(reflector)
    };

    let mut sessions_mined = 0usize;
    let mut interventions_found = 0usize;
    let mut reflections_written = 0usize;

    for (meta, path) in &todo {
        // The workspace the session's rules block was matched against, off
        // its run record — the key a match presents. Never the session's
        // jail: on `serve` the block is rendered against the producer root
        // and each session is jailed a level below, so a lesson stamped
        // with the jail scoped its rule to a workspace no run presents
        // (found on review). A record from before the field gives none.
        // One read of the transcript for the conversation and its run
        // records both — `load` and `run_configs` each parsed the whole
        // file, twice per session on the nightly's hot path (found on
        // review).
        // The text is kept: D3 places a correction against everything the
        // run had read, which a compaction since may have dropped from the
        // loaded list (`Session::messages_ever_before`), and the second view
        // must be of the same bytes.
        let read = std::fs::read_to_string(path)
            .map_err(anyhow::Error::from)
            .and_then(|text| Session::parse(path, &text).map(|t| (text, t)));
        let (text, t) = match read {
            Ok(read) => read,
            Err(e) => {
                // A transcript that does not load is not this command's bug to
                // fix; skip it *without* marking it mined, so a later mecha
                // that can read it still gets the chance.
                eprintln!("skipping {}: {e:#}", meta.id);
                continue;
            }
        };

        let convo = &t.convo;
        let mut interventions = extract_interventions(&convo.messages);
        interventions.extend(mecha_core::learning::extract_mismatches(
            &convo.messages,
            &t.outcome_positions,
        ));
        interventions_found += interventions.len();

        // Provenance, read from the transcript's recorded taint — not from
        // anything a model says. A reflection from a conversation that held
        // third-party content becomes a rule in every future run's prompt,
        // so the classification must fail closed: a timeline that cannot be
        // read covers nothing, and uncovered means Untrusted.
        // Off the transcript already read, like the run records: one
        // reading of the file for its messages, its configs and its taint,
        // rather than a strict re-parse beside a lenient one (found on
        // review).
        let timeline = t.taint_timeline.clone();

        if args.dry_run {
            for i in &interventions {
                let (_, origin, evidence) = evidence_for(timeline.covering(i.at), i);
                println!(
                    "{} [{}] ({}) {}",
                    meta.id,
                    i.trigger.as_str(),
                    match (origin, evidence) {
                        (Origin::Clean, Evidence::Full) => "clean",
                        (Origin::Clean, Evidence::UserTurns) => "clean, user-turns only",
                        (Origin::Untrusted, _) => "untrusted",
                        (Origin::Derived, _) => "derived",
                    },
                    i.text.lines().next().unwrap_or("")
                );
            }
            sessions_mined += 1;
            continue;
        }

        // All-or-nothing per session. An error here is usually the provider
        // being down — and reflect now runs unattended, where "print and mark
        // mined anyway" turns an outage into silent permanent loss. Nothing
        // is appended until every intervention reflected, so a retry after a
        // partial failure cannot duplicate the ones that succeeded; the
        // session stays unmined and the next run pays again, which local
        // inference makes free.
        let reflector = reflector.as_ref().expect("built unless dry-run");
        let mut pending = Vec::new();
        let mut failed = false;
        for intervention in &interventions {
            // What the reflector may see, decided per intervention: full
            // excerpts under provably clean coverage, the user's own words
            // and tool names otherwise. See `learning::evidence_for`.
            let (input, origin, evidence) =
                evidence_for(timeline.covering(intervention.at), intervention);
            if args.remine_untrusted {
                // Backfill visits only what the gate excluded: an
                // intervention that was clean is already on file in full,
                // and one already re-mined must not double.
                if evidence != Evidence::UserTurns
                    || already_user_turns.contains(&(meta.id.clone(), intervention.text.clone()))
                {
                    continue;
                }
            }
            if intervention.trigger == Trigger::Mismatch && origin != Origin::Clean {
                continue;
            }
            match reflector.reflect(&input).await {
                Ok(Some((mut r, answer))) => {
                    r.session_id = meta.id.clone();
                    // D3 (row 2e-3): data error, behaviour error or gap, by
                    // what the run had been given when the owner stepped in
                    // — decided here, from the transcript, never by the
                    // reflector, which only copied the spans. Deterministic
                    // code over the recorded results, whatever the taint:
                    // nothing it reads reaches a model or a rule.
                    if mecha_core::attribution::in_scope(&r.domain, &r.trigger) {
                        let ever = ever_before(&text, &convo.messages, intervention.at);
                        r.attribution = Some(mecha_core::attribution::decide(
                            &answer,
                            &intervention.text,
                            &given_in(ever.as_deref()),
                        ));
                    }
                    r.goals = goals_for(&t, intervention);
                    r.origin = origin;
                    r.evidence = evidence;
                    // Where it happened, from what the miner already held:
                    // the tool window is registry names (it survives the
                    // user-evidence-only view for the same reason), the
                    // surface, workspace and goal are the run record's
                    // matched ones — never `meta.kind`, never the jail,
                    // never the anchor (and never `r.goals` above, which a
                    // plan names). Set here and not by the reflector, which
                    // saw prose.
                    // The keys of the run record covering *this*
                    // intervention, not the session's first: a session may
                    // hold runs matched on different keys (a resumed
                    // question, a `/model` switch), and `config_covering`
                    // is the exact answer per message (found on review).
                    let (matched_workspace, matched_surface, matched_goal) =
                        keys_covering(&t, intervention.at);
                    r.situation = Some(
                        mecha_core::situation::Situation::recorded(
                            &intervention.tools_before,
                            intervention.trigger.as_str(),
                            matched_surface,
                            matched_workspace.as_deref(),
                        )
                        .toward(matched_goal),
                    );
                    pending.push(r);
                }
                Ok(None) => {
                    if intervention.trigger != Trigger::Followup {
                        // Steers and denials are unambiguous interventions; a
                        // skip there is worth seeing. Followup skips are the
                        // common case and would only be noise.
                        eprintln!("· [{}] no lesson drawn", intervention.trigger.as_str());
                    }
                }
                Err(e) => {
                    eprintln!(
                        "· reflection failed: {e:#}\n  leaving {} unmined so a later run retries",
                        meta.id
                    );
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        for r in &pending {
            store.append_reflexion(r)?;
            reflections_written += 1;
            println!("· [{}] {}", r.trigger, r.reflexion_text);
        }

        if !args.remine_untrusted {
            store.mark_mined(&meta.id)?;
        }
        sessions_mined += 1;
    }

    // The outbox pass. Per-item rather than all-or-nothing: items are
    // independent corrections, so one reflection failure leaves only that
    // item unmined for the next run.
    let mut edits_mined = 0usize;
    let mut rejections_mined = 0usize;
    for item in &outbox_todo {
        let rejection = rejection_intervention(item);
        let trigger = if rejection.is_some() {
            Trigger::Reject
        } else {
            Trigger::Edit
        };
        if args.dry_run {
            match trigger {
                Trigger::Reject => println!(
                    "{} [reject] {} draft rejected with a reason",
                    item.id, item.tool
                ),
                _ => println!(
                    "{} [edit] {} draft edited before sending",
                    item.id, item.tool
                ),
            }
            continue;
        }
        // What the reflector may see. An edit's lesson *is* the model's
        // draft against the owner's rewrite, so it is shown whole and its
        // provenance is the staging snapshot's. A rejection's lesson is the
        // owner's own words, so it takes the transcript path's gate
        // (`evidence_for`): the drafted call in full under clean staging
        // taint, the reason and the tool name alone otherwise — third-party
        // bytes never reach the reflector, and the owner's words are clean.
        let (input, origin, evidence) = match &rejection {
            Some(i) => evidence_for(Some(item.taint), i),
            None => (
                outbox_intervention(item),
                // The item snapshots the conversation's taint at staging,
                // which is exactly the provenance question: was there
                // third-party text in context when this draft was written.
                if item.taint.untrusted {
                    Origin::Untrusted
                } else {
                    Origin::Clean
                },
                Evidence::Full,
            ),
        };
        let reflector = reflector.as_ref().expect("built unless dry-run");
        match reflector.reflect(&input).await {
            Ok(reflected) => {
                if let Some((mut r, answer)) = reflected {
                    // A rejection is a correction D3 attributes; what the
                    // run was given is the staging session's results before
                    // the staging call, and what it said includes the draft.
                    // A session that cannot be read, or a staging call it
                    // does not carry, places no fact — never a gap.
                    if let (Some(reason), true) = (
                        item.rejection_reason(),
                        mecha_core::attribution::in_scope(&r.domain, &r.trigger),
                    ) {
                        let messages =
                            mecha_core::outbox_source::messages_for_item(item, &sessions_dir);
                        r.attribution = Some(mecha_core::attribution::decide(
                            &answer,
                            reason,
                            &given_at_staging(item, &messages),
                        ));
                    }
                    // The drafting session, when the front-end knew it — the
                    // same lineage a behavior reflection carries.
                    r.session_id = item.session_id.clone().unwrap_or_default();
                    r.origin = origin;
                    r.evidence = evidence;
                    // The drafting tool is the focus: a lesson from editing
                    // a mail draft scopes to `mail_send` and loads only
                    // where that tool is registered. The item records no
                    // surface or workspace.
                    r.situation = Some(mecha_core::situation::Situation::recorded(
                        std::slice::from_ref(&item.tool),
                        trigger.as_str(),
                        None,
                        None,
                    ));
                    store.append_reflexion(&r)?;
                    reflections_written += 1;
                    println!("· [{}] {}", trigger.as_str(), r.reflexion_text);
                }
                // Mined either way: a skip means the edit taught nothing
                // (a typo fix), and re-arguing it nightly will not change
                // that.
                store.mark_outbox_mined(&item.id)?;
                match trigger {
                    Trigger::Reject => rejections_mined += 1,
                    _ => edits_mined += 1,
                }
            }
            Err(e) => {
                eprintln!(
                    "· reflection failed: {e:#}\n  leaving outbox item {} unmined so a \
                     later run retries",
                    item.id
                );
            }
        }
    }

    if args.dry_run {
        println!(
            "dry run: {sessions_mined} session(s) with {interventions_found} intervention(s), \
             {} edited or reasoned-rejected draft(s); nothing written",
            outbox_todo.len()
        );
    } else {
        store.commit(&format!(
            "reflect: {sessions_mined} session(s), {edits_mined} draft edit(s), \
             {rejections_mined} draft rejection(s), {reflections_written} reflection(s)"
        ));
        println!(
            "mined {sessions_mined} session(s), {edits_mined} draft edit(s) and \
             {rejections_mined} draft rejection(s): {interventions_found} intervention(s), \
             {reflections_written} reflection(s) → {}",
            store.root().join("reflections.jsonl").display()
        );
    }
    Ok(())
}

/// The workspace and surface one run record says its block was matched
/// against — the two keys the reconcile compares. The goal is not one of
/// them: no row carried a goal before `rules_goal` existed, and every door
/// stamps it from that field, so there is nothing to reconcile.
type ReconciledKeys = (Option<PathBuf>, Option<mecha_core::session::SessionKind>);

/// The keys a session's rules block was matched against, one entry per run
/// record in order (`RunConfig::rules_workspace`, `rules_surface`) — what a
/// match presents, never the session's jail and never `SessionMeta::kind`.
/// An entry is `(None, None)` for a record from before the fields, or a run
/// that declared neither; the vec is empty for a transcript with no run
/// record at all. `Err` when the transcript cannot be read, which confirms
/// nothing either way.
fn matched_keys_of(path: &Path) -> std::result::Result<Vec<ReconciledKeys>, String> {
    // Through `Session::read`, the same reader the miner holds, so the
    // reconcile and the miner cannot disagree about one record by parsing
    // it two ways (found on review). Every run record, in order: a stored
    // reflection carries no message index, so the reconcile cannot ask
    // which attach covered it, and must instead accept a key *any* attach
    // presented — comparing against the first alone rewrote the key the
    // miner had just stamped off a continuation's second record (found on
    // review).
    Session::read(path)
        .map(|t| {
            t.configs
                .iter()
                .map(|rc| (rc.rules_workspace.clone(), rc.rules_surface))
                .collect()
        })
        .map_err(|e| format!("session unreadable: {e:#}"))
}

/// What a reflection serves (`Reflexion::goals`), from two sources in
/// order (`APPRAISAL-WIRING-DESIGN.md` L3, row 2e-5a):
///
/// 1. **The plan at the intervention**: a mismatch's failed step's goals,
///    or the step goals on the intervention message's planning metadata,
///    else the plan or question in force there (`appraisal::goal_at`).
///    Evidence local to the moment, so it wins where it names one.
/// 2. **The conversation's anchor** (S1), which the run covering the
///    intervention recorded (`Transcript::anchor_covering`): a task run's
///    `task:<id>`, a trigger run's `trigger:<name>`, a front-door drain's
///    `request:<id>`, or a goal the owner confirmed. Only when the first
///    names none — which, since the model stopped planning, is every
///    reflection, and why `goal_lessons` and `goal_context` served
///    nothing.
///
/// The anchor is the harness's seed or the owner's confirmation, never a
/// model's claim, so it takes every kind (a `trigger:` or `request:`
/// pointer included, which the plan-named source may not carry). It is
/// not the situation's goal key: that is `rules_goal`, what the rules
/// block was matched toward (`keys_covering`), and the two differ wherever
/// a hand-over resumes an older anchor. An intervention no recorded run
/// covers stamps none from the anchor — absent, never the session's last
/// anchor read back onto it.
fn goals_for(
    t: &mecha_core::session::Transcript,
    intervention: &Intervention,
) -> Vec<mecha_core::goal::GoalRef> {
    let messages = &t.convo.messages;
    let mut goals = if intervention.trigger == Trigger::Mismatch {
        serde_json::from_str::<mecha_core::planning::StepFeedback>(&intervention.context)
            .ok()
            .map(|s| s.goals())
            .unwrap_or_default()
    } else {
        messages
            .get(intervention.at)
            .and_then(|m| m.planning.as_ref())
            .map(|f| {
                f.steps
                    .iter()
                    .filter_map(|s| s.goal.clone())
                    .collect::<Vec<_>>()
            })
            .filter(|goals| !goals.is_empty())
            .unwrap_or_else(|| {
                messages
                    .get(..=intervention.at)
                    .and_then(mecha_core::appraisal::goal_at)
                    .into_iter()
                    .collect()
            })
    };
    if goals.is_empty() {
        goals.extend(t.anchor_covering(intervention.at).cloned());
    }
    goals.sort_by_key(|g| g.to_string());
    goals.dedup();
    goals
}

/// The keys of the run record covering message `at` of a transcript
/// already read (`Transcript::config_covering`): what the miner and the
/// backfill stamp on an intervention, since a session may hold runs
/// matched on different keys. The goal is `rules_goal`, what the block was
/// matched toward — never the conversation's anchor. `(None, None, None)`
/// for a transcript recorded before configs were kept.
fn keys_covering(
    t: &mecha_core::session::Transcript,
    at: usize,
) -> mecha_core::learning::MatchedKeys {
    t.config_covering(at)
        .map(|rc| {
            (
                rc.rules_workspace.clone(),
                rc.rules_surface,
                rc.rules_goal.clone(),
            )
        })
        .unwrap_or((None, None, None))
}

/// The first run record's keys off a transcript already read.
#[cfg(test)]
fn matched_keys_in(
    t: &mecha_core::session::Transcript,
) -> (Option<PathBuf>, Option<mecha_core::session::SessionKind>) {
    t.configs
        .first()
        .map(|rc| (rc.rules_workspace.clone(), rc.rules_surface))
        .unwrap_or((None, None))
}

/// Reconcile the keys on every recorded situation — the workspace and the
/// surface — with what its session's rules block was matched against,
/// decided key by key by `learning::reconcile_key` before anything is
/// written: a key the row does not record is never added (an outbox
/// edit's lesson scopes by tools on purpose, and a key added here would
/// narrow it on no conviction); a session that cannot be found or read
/// confirms nothing and the row stays, with the reason; a record that was
/// read and disagrees sets what it names, `None` included. Runs on every
/// `reflect` pass — the nightly's `learn --auto` follows it — so
/// correctness does not depend on a human running a flag first (found on
/// review). Commits its own writes; a pass with nothing to apply does not
/// touch the file. Returns how many rows were (or would be) rewritten.
fn reconcile_recorded_keys(
    store: &LearningStore,
    paths: &std::collections::HashMap<String, PathBuf>,
    unreadable_sessions: usize,
    dry_run: bool,
) -> Result<usize> {
    use mecha_core::learning::{reconcile_key, KeyReconcile, KeyUpdate};
    // Rows a transcript produced, and only those: a pass domain's rows —
    // the mail classifier's triage corrections — carry a synthetic session
    // id and a surface the pass stamped itself, and every one of them
    // printed as a session not found on every pass, forever, in the line
    // that exists to say a transcript really went missing (found on
    // review).
    let present: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| {
            !mecha_core::learning::PASS_DOMAINS.contains(&r.domain.as_str())
                && r.situation.as_ref().is_some_and(|s| {
                    s.workspace.is_some() || s.surface.is_some() || s.surface_unread.is_some()
                })
                && !r.session_id.is_empty()
        })
        .collect();
    if present.is_empty() {
        return Ok(0);
    }
    type Matched = Vec<ReconciledKeys>;
    // What the record confirms for one recorded key: the key itself when
    // any attach presented it, else the first attach's — or none.
    fn confirmed<T: PartialEq + Clone>(
        recorded: Option<&T>,
        attaches: Vec<Option<T>>,
    ) -> Option<T> {
        match recorded {
            Some(r) if attaches.iter().any(|a| a.as_ref() == Some(r)) => Some(r.clone()),
            _ => attaches.into_iter().next().flatten(),
        }
    }
    let mut record_by_session: std::collections::HashMap<
        String,
        std::result::Result<Matched, String>,
    > = Default::default();
    let mut reconcile: Vec<(String, KeyUpdate)> = Vec::new();
    let mut to_none = 0usize;
    let mut left = 0usize;
    let show_w = |w: Option<&PathBuf>| {
        w.map(|w| w.display().to_string())
            .unwrap_or_else(|| "none".into())
    };
    let show_k =
        |k: Option<mecha_core::session::SessionKind>| k.map(|k| k.as_str()).unwrap_or("none");
    for r in &present {
        let Some(s) = &r.situation else { continue };
        let record = record_by_session
            .entry(r.session_id.clone())
            .or_insert_with(|| match paths.get(&r.session_id) {
                Some(path) => matched_keys_of(path),
                None if unreadable_sessions > 0 => Err(format!(
                    "no readable session matching \"{}\" — {unreadable_sessions} \
                     transcript(s) in the store could not be read, and it may be one",
                    r.session_id
                )),
                None => Err(format!("no session matching \"{}\"", r.session_id)),
            })
            .clone();
        let mut update = KeyUpdate::default();
        let workspace_record = record
            .as_ref()
            .map(|all| {
                confirmed(
                    s.workspace.as_ref(),
                    all.iter().map(|(w, _)| w.clone()).collect(),
                )
            })
            .map_err(Clone::clone);
        match reconcile_key(
            s.workspace.as_ref(),
            workspace_record
                .as_ref()
                .map(|m| m.as_ref())
                .map_err(Clone::clone),
        ) {
            KeyReconcile::Keep => {}
            KeyReconcile::Unreadable(why) => {
                left += 1;
                println!(
                    "· {} keeps workspace {} — {why}",
                    r.id,
                    show_w(s.workspace.as_ref())
                );
            }
            KeyReconcile::Set(matched) => {
                if matched.is_none() {
                    to_none += 1;
                }
                println!(
                    "· {} workspace {} → {}",
                    r.id,
                    show_w(s.workspace.as_ref()),
                    if matched.is_none() {
                        "none (the run record carries none)".to_string()
                    } else {
                        show_w(matched.as_ref())
                    }
                );
                update.workspace = Some(matched);
            }
        }
        // A surface this build could not name is a recorded key no attach
        // can confirm, so it takes what the first attach presented, or
        // none. Stood in for by a corpus mark here, since `reconcile_key`
        // compares kinds and no attach ever presents a mark: the outcome
        // is the same — nothing confirms it — and the log prints the raw.
        let surface_recorded = if s.surface_unread.is_some() {
            Some(mecha_core::session::SessionKind::Test)
        } else {
            s.surface
        };
        let show_recorded = || match &s.surface_unread {
            Some(raw) => format!("{raw} (a surface this build cannot name)"),
            None => show_k(s.surface).to_string(),
        };
        // A parked surface against a record that names none stays parked:
        // the record side reads a surface it cannot name as none, so a
        // downgrade would otherwise clear the parked key here — the very
        // key the parking exists to keep — and hand the rule every surface
        // (found on review). A record that names a surface still replaces
        // it.
        let parked_and_unnamed = s.surface_unread.is_some()
            && record
                .as_ref()
                .is_ok_and(|all| all.iter().all(|(_, k)| k.is_none()));
        if parked_and_unnamed {
            println!(
                "· {} keeps surface {} — no run record names a surface to replace it with",
                r.id,
                show_recorded()
            );
        }
        // The replacement for a parked surface is the first attach that
        // *names* one — the same quantifier as the guard above. `confirmed`
        // falls back to the first attach whatever it says, and a session
        // whose first run carried no rules block and whose resumed run did
        // read [none, web]: the guard let it through and the fallback
        // cleared the parked key to nothing (found on review).
        let surface_record = record
            .as_ref()
            .map(|all| {
                if s.surface_unread.is_some() {
                    all.iter().find_map(|(_, k)| *k)
                } else {
                    confirmed(
                        surface_recorded.as_ref(),
                        all.iter().map(|(_, k)| *k).collect(),
                    )
                }
            })
            .map_err(Clone::clone);
        match reconcile_key(
            if parked_and_unnamed {
                None
            } else {
                surface_recorded.as_ref()
            },
            surface_record
                .as_ref()
                .map(|m| m.as_ref())
                .map_err(Clone::clone),
        ) {
            KeyReconcile::Keep => {}
            KeyReconcile::Unreadable(why) => {
                // Counted once per row, above, when the workspace was also
                // recorded; a surface-only row is counted here.
                if s.workspace.is_none() {
                    left += 1;
                    println!("· {} keeps surface {} — {why}", r.id, show_recorded());
                }
            }
            KeyReconcile::Set(matched) => {
                if matched.is_none() {
                    to_none += 1;
                }
                println!(
                    "· {} surface {} → {}",
                    r.id,
                    show_recorded(),
                    if matched.is_none() {
                        "none (the run record carries none)".to_string()
                    } else {
                        show_k(matched).to_string()
                    }
                );
                update.surface = Some(matched);
            }
        }
        if !update.is_empty() {
            reconcile.push((r.id.clone(), update));
        }
    }
    let written = if dry_run {
        reconcile.len()
    } else {
        let written = store.reconcile_keys(&reconcile, &chrono::Utc::now().to_rfc3339())?;
        if written > 0 {
            store.commit(&format!(
                "reflect: {written} situation(s)' keys reconciled with the run record"
            ));
        }
        written
    };
    if written + left > 0 {
        println!(
            "{} {written} situation(s) of {} recorded against the run record ({to_none} key(s) \
             to none: the record carries none; {left} left as recorded: session not found or \
             unreadable)",
            if dry_run {
                "would reconcile"
            } else {
                "reconciled"
            },
            present.len()
        );
    }
    Ok(written)
}

/// Frame one draft the owner rejected with a reason as an intervention for
/// the behaviour reflector (R16a) — `None` for any item
/// [`OutboxItem::rejection_reason`] refuses. The drafted call is the
/// context, the owner's reason is what they said, and the aftermath is that
/// nothing went out. `text` carries only the owner's words (beside a fixed
/// harness phrase), because `user_evidence_only` keeps `text` and withholds
/// the rest: under untrusted staging taint the reflector sees the reason and
/// the tool name, never the draft.
///
/// [`OutboxItem::rejection_reason`]: mecha_core::outbox::OutboxItem::rejection_reason
fn rejection_intervention(item: &mecha_core::outbox::OutboxItem) -> Option<Intervention> {
    let reason = item.rejection_reason()?;
    let pretty =
        |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    Some(Intervention {
        trigger: Trigger::Reject,
        context: format!(
            "mecha drafted this `{}` call (outbox item {}):\n{}",
            item.tool,
            item.id,
            pretty(&item.args)
        ),
        text: format!("the user rejected the draft, saying: {reason}"),
        aftermath: "nothing was sent".to_string(),
        // Not a transcript position, as for an edit: provenance comes from
        // the item's staging taint.
        at: 0,
        tools_before: vec![item.tool.clone()],
        tools_after: Vec::new(),
    })
}

/// What the run had been given before the correction at `at` of the loaded
/// list: everything recorded before that message, compacted since or not —
/// `thin_old_results` truncates a result in place, so the loaded list can
/// hold the right value in full and the wrong one cut off, which reads as
/// a behaviour error over what was a data error (found on review of #332).
/// A correction that cannot be placed in the record is unreadable.
fn ever_before(
    text: &str,
    messages: &[mecha_core::message::Message],
    at: usize,
) -> Option<Vec<mecha_core::message::Message>> {
    messages
        .get(at)
        .and_then(|m| Session::messages_ever_before(text, m))
}

fn given_in(ever: Option<&[mecha_core::message::Message]>) -> mecha_core::attribution::Given<'_> {
    match ever {
        Some(ever) => mecha_core::attribution::Given::before(ever, ever.len()),
        None => mecha_core::attribution::Given::unreadable(),
    }
}

/// What a rejected draft's run had been given when it staged the draft:
/// the staging session's calls before the staging message, and — among
/// what the run said — the draft itself, which rides in the staging call.
/// A session that could not be read (no messages) or that does not carry
/// the staging call is unreadable: D3 places no fact against it.
fn given_at_staging<'a>(
    item: &mecha_core::outbox::OutboxItem,
    messages: &'a [mecha_core::message::Message],
) -> mecha_core::attribution::Given<'a> {
    match mecha_core::outbox_source::staged_in(item, messages) {
        Some(at) => {
            let mut given = mecha_core::attribution::Given::before(messages, at);
            given.said.push(item.args_before.to_string());
            given
        }
        None => mecha_core::attribution::Given::unreadable(),
    }
}

/// Frame one edited-then-sent outbox item as an intervention for the
/// writing-domain reflector: the draft is the context, the diff is what the
/// user did, the sent version is the aftermath.
fn outbox_intervention(item: &mecha_core::outbox::OutboxItem) -> Intervention {
    let pretty =
        |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    Intervention {
        trigger: Trigger::Edit,
        context: format!(
            "mecha drafted this `{}` call (outbox item {}):\n{}",
            item.tool,
            item.id,
            pretty(&item.args_before)
        ),
        text: format!(
            "the user edited the draft before releasing it:\n{}",
            mecha_core::outbox::diff_args(&item.args_before, &item.args)
        ),
        aftermath: format!("the user sent the edited version:\n{}", pretty(&item.args)),
        // Not a transcript position: an edit lives in the outbox item, and its
        // provenance comes from the item's taint snapshot, not a timeline.
        at: 0,
        tools_before: Vec::new(),
        tools_after: Vec::new(),
    }
}

/// `--backfill-situations`: `docs/GOAL-SYSTEM-DESIGN.md` §17.7 item 6.
///
/// Deterministic end to end — `extract_interventions` over each transcript,
/// the match on (session, trigger, intervention text), `Situation::recorded`
/// off the window and the session header — so it costs no model and can be
/// re-run. Each transcript is read once for all the reflections that cite
/// it. A reflection is left without a situation, and said so, when its
/// session cannot be found or read, when no intervention in the transcript
/// carries its trigger and text (a compaction since mining, or an outbox
/// edit with no transcript), or when several do with different windows;
/// absent is the honest reading and the pass never picks one. The
/// reflection's `goals` are never backfilled; its situation's goal key is,
/// off the covering run record like the workspace and the surface.
fn backfill_situations(store: &LearningStore, sessions_dir: &Path, dry_run: bool) -> Result<()> {
    use mecha_core::learning::{backfill_situation, extract_interventions, Backfilled};
    let _lock = if dry_run { None } else { Some(store.lock()?) };
    // The store listed once — `Session::find` is a full scan of the
    // directory per call (found on review) — then one read per cited
    // session, shared by every reflection that cites it.
    // `list_counting`, not `list`: a transcript whose header cannot be read
    // is absent from the map, and a reflection citing it must not read as
    // citing a session that was deleted (found on review) — the count is
    // carried into that reason and the summary line.
    let (listed, unreadable_sessions) = Session::list_counting(sessions_dir)?;
    let paths: std::collections::HashMap<String, std::path::PathBuf> = listed
        .into_iter()
        .map(|(meta, path)| (meta.id, path))
        .collect();
    // The workspace reconcile first — the ordinary pass runs it too, and
    // this flag is the place to run it by hand and read every line.
    reconcile_recorded_keys(store, &paths, unreadable_sessions, dry_run)?;
    let todo: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| r.situation.is_none())
        .collect();
    if todo.is_empty() {
        println!("every reflection carries a situation — nothing to backfill");
        return Ok(());
    }
    // One session read: its record, its interventions, and the workspace
    // its rules block was matched against; or why it could not be read.
    type SessionRead = Result<
        (
            Vec<mecha_core::learning::Intervention>,
            mecha_core::session::Transcript,
        ),
        String,
    >;
    let mut by_session: std::collections::HashMap<String, SessionRead> = Default::default();
    let mut updates: Vec<(String, mecha_core::situation::Situation)> = Vec::new();
    let mut unmatched: Vec<(String, String)> = Vec::new();
    for r in &todo {
        if r.session_id.is_empty() {
            unmatched.push((r.id.clone(), "no session recorded (an outbox edit)".into()));
            continue;
        }
        let read = by_session.entry(r.session_id.clone()).or_insert_with(|| {
            let path = paths.get(&r.session_id).ok_or_else(|| {
                if unreadable_sessions > 0 {
                    format!(
                        "no readable session matching \"{}\" — {unreadable_sessions} \
                             transcript(s) in the store could not be read, and it may be one",
                        r.session_id
                    )
                } else {
                    format!("no session matching \"{}\"", r.session_id)
                }
            })?;
            let t = Session::read(path).map_err(|e| format!("session unreadable: {e:#}"))?;
            Ok((extract_interventions(&t.convo.messages), t))
        });
        match read {
            Err(why) => unmatched.push((r.id.clone(), why.clone())),
            Ok((interventions, t)) => {
                match backfill_situation(r, interventions, &|at| keys_covering(t, at)) {
                    Backfilled::Matched(s) => updates.push((r.id.clone(), s)),
                    Backfilled::NoMatch => unmatched.push((
                        r.id.clone(),
                        "no intervention with this trigger and text in the transcript".into(),
                    )),
                    Backfilled::Ambiguous(n) => {
                        unmatched.push((r.id.clone(), format!("fits {n} different tool windows")))
                    }
                }
            }
        }
    }
    for (id, s) in &updates {
        println!("· {id} ← {}", s.describe());
    }
    for (id, why) in &unmatched {
        println!("· {id} stays without a situation — {why}");
    }
    let verb = if dry_run {
        "would recompute"
    } else {
        "recomputed"
    };
    let written = if dry_run {
        updates.len()
    } else {
        let written = store.set_situations(&updates, &chrono::Utc::now().to_rfc3339())?;
        // Committed on its own, like every batch pass over this store: the
        // rewrite changes which region batches the next `learn --auto`
        // argues, and left uncommitted it would ride into the next
        // nightly's `reflect: 0 session(s)` commit (found on review).
        if written > 0 {
            store.commit(&format!(
                "reflect --backfill-situations: {written} situation(s) recomputed, {} left absent",
                unmatched.len()
            ));
        }
        written
    };
    println!(
        "{verb} {written} of {} situation(s); {} left absent, {} session(s) read{}",
        todo.len(),
        unmatched.len(),
        by_session.values().filter(|r| r.is_ok()).count(),
        if unreadable_sessions > 0 {
            format!(", {unreadable_sessions} transcript(s) in the store unreadable")
        } else {
            String::new()
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::session::{Record, RunConfig, SessionKind, SessionMeta};

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    fn session_in(dir: &Path, jail: &str, matched: Option<&str>) -> Session {
        session_on(
            dir,
            jail,
            matched,
            Some(SessionKind::Web),
            Some(SessionKind::Web),
        )
    }

    fn session_on(
        dir: &Path,
        jail: &str,
        matched: Option<&str>,
        kind: Option<SessionKind>,
        matched_surface: Option<SessionKind>,
    ) -> Session {
        let s = Session::create(
            dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from(jail),
                title: None,
                kind,
            },
        )
        .unwrap();
        s.append(&Record::Config(RunConfig {
            workspace: PathBuf::from(jail),
            rules_workspace: matched.map(PathBuf::from),
            rules_surface: matched_surface,
            ..Default::default()
        }))
        .unwrap();
        s
    }

    /// A rejected draft is a correction D3 attributes (row 2e-3), against
    /// what the staging run had read before it staged: the calls before the
    /// staging message, never the staging call or anything after it. The
    /// wrong value in a result the run read is a data error pointing at that
    /// read; the same draft from a run that read neither value is a gap; and
    /// a staging session that cannot be read places no fact at all.
    #[test]
    fn a_rejected_draft_is_attributed_by_what_the_staging_run_had_read() {
        use mecha_core::agent::Taint;
        use mecha_core::attribution::{decide, Answer, Basis, Class, Fact};
        use mecha_core::message::{Block, Message};
        use mecha_core::outbox::{OutboxKind, OutboxStore, Provenance};
        let root = scratch("reflect-reject-attribution");
        let store = OutboxStore::open(&root).unwrap();
        let draft = serde_json::json!({
            "to": "sam@example.edu",
            "body": "Dana is at Northwind Labs, so write to her there."
        });
        let staged = store
            .stage(
                "mail_send",
                OutboxKind::Message,
                draft.clone(),
                Taint::default(),
                Provenance {
                    session_id: Some("20260926T090000-ada".into()),
                    call_id: Some("s1".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let item = store
            .resolve(
                &staged.id,
                "rejected",
                Some("She moved to Lakeside Institute in March.".into()),
            )
            .unwrap();
        let reason = item.rejection_reason().unwrap().to_string();
        let transcript = |read: &str| {
            vec![
                Message::user("Tell Sam where Dana works now."),
                Message::assistant(vec![Block::ToolUse {
                    id: "t1".into(),
                    name: "kg_entity".into(),
                    input: serde_json::json!({"name": "Dana Whitfield"}),
                }]),
                Message::tool_results(vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: read.into(),
                    is_error: false,
                }]),
                Message::assistant(vec![Block::ToolUse {
                    id: "s1".into(),
                    name: "mail_send".into(),
                    input: draft.clone(),
                }]),
                // After the staging call: never what the draft was written from.
                Message::tool_results(vec![Block::ToolResult {
                    tool_use_id: "s1".into(),
                    content: "Drafted, not sent. Lakeside Institute".into(),
                    is_error: false,
                }]),
            ]
        };
        let answer = Answer::Fact(Fact {
            wrong: "Northwind Labs".into(),
            right: Some("Lakeside Institute".into()),
        });

        let read_wrong = transcript("Dana Whitfield — employer: Northwind Labs.");
        let a = decide(&answer, &reason, &given_at_staging(&item, &read_wrong));
        assert_eq!((a.class, a.basis), (Class::Data, Basis::WrongGiven));
        assert_eq!(a.source.unwrap().call, "t1");

        let read_nothing = transcript("Dana Whitfield — no employer on record.");
        let a = decide(&answer, &reason, &given_at_staging(&item, &read_nothing));
        assert_eq!(
            (a.class, a.basis),
            (Class::Gap, Basis::NeitherGiven),
            "the wrong value is grounded in the draft; the staging result is not given"
        );

        let a = decide(&answer, &reason, &given_at_staging(&item, &[]));
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::NoRecord));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// R16a: a draft the owner rejected with a reason reaches the reflector
    /// as an owner correction, framed for the behaviour reflector, and the
    /// draft itself is withheld when third-party text was in context at
    /// staging — the owner's words survive the withholding. Fails on the
    /// tree before 1d, where a rejected draft was never mined.
    #[test]
    fn a_reasoned_rejection_reaches_the_reflector_in_the_owners_words() {
        use mecha_core::agent::Taint;
        use mecha_core::outbox::{OutboxKind, OutboxStore, Provenance};
        let root = scratch("reflect-reject");
        let store = OutboxStore::open(&root).unwrap();
        let stage = |taint: Taint| {
            let staged = store
                .stage(
                    "mail_send",
                    OutboxKind::Message,
                    serde_json::json!({"to": "dirk@example.invalid", "body": "IGNORE THE OWNER"}),
                    taint,
                    Provenance {
                        session_id: Some("20260925T090000-ada".into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            store
                .resolve(&staged.id, "rejected", Some("he already has it".into()))
                .unwrap()
        };
        let clean = stage(Taint::default());
        let i = rejection_intervention(&clean).expect("a reasoned rejection is a correction");
        assert_eq!(i.trigger, Trigger::Reject);
        assert!(i.text.contains("he already has it"));
        assert_eq!(i.tools_before, vec!["mail_send".to_string()]);
        let (input, origin, evidence) = evidence_for(Some(clean.taint), &i);
        assert_eq!((origin, evidence), (Origin::Clean, Evidence::Full));
        assert!(input.context.contains("IGNORE THE OWNER"));

        let armed = stage(Taint {
            untrusted: true,
            ..Default::default()
        });
        let i = rejection_intervention(&armed).unwrap();
        let (input, origin, evidence) = evidence_for(Some(armed.taint), &i);
        assert_eq!((origin, evidence), (Origin::Clean, Evidence::UserTurns));
        assert!(
            !input.context.contains("IGNORE THE OWNER") && !input.aftermath.contains("IGNORE"),
            "a draft written under third-party content never reaches the reflector"
        );
        assert!(input.text.contains("he already has it"));

        // No reason, no words to learn from: the reject is signed already.
        let staged = store
            .stage(
                "mail_send",
                OutboxKind::Message,
                serde_json::json!({"body": "x"}),
                Taint::default(),
                Provenance::default(),
            )
            .unwrap();
        let silent = store.resolve(&staged.id, "rejected", None).unwrap();
        assert!(rejection_intervention(&silent).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The miner stamps the workspace and the surface the block was
    /// matched against, never the session's jail and never its kind — the
    /// bug that recurred on `serve`, Slack and task sessions, each of which
    /// jails a run somewhere the block was not rendered, and the board's
    /// task door, which records a task while the block was matched as web.
    /// Fails on the old reads of `meta.workspace` and `meta.kind`.
    #[test]
    fn the_miner_reads_the_matched_keys_and_never_the_jail_or_the_kind() {
        mecha_core::session::ignore_kind_env_for_tests();
        let dir = scratch("mecha-reflect-test");
        let s = session_in(
            &dir,
            "/home/x/.mecha/work/web/main",
            Some("/home/x/.mecha/work/web"),
        );
        assert_eq!(
            s.meta.workspace,
            PathBuf::from("/home/x/.mecha/work/web/main")
        );
        let (matched, surface) = matched_keys_of(&s.path).unwrap().remove(0);
        assert_eq!(
            matched.as_deref(),
            Some(Path::new("/home/x/.mecha/work/web"))
        );
        let recorded = mecha_core::situation::Situation::recorded(
            &["shell".into()],
            "denial",
            surface,
            matched.as_deref(),
        );
        assert_eq!(
            recorded.workspace.as_deref(),
            Some(Path::new("/home/x/.mecha/work/web")),
            "the key a match presents"
        );
        assert_ne!(
            recorded.workspace,
            Some(s.meta.workspace.clone()),
            "not the jail"
        );
        // A record from before the fields: no key, never the jail or kind.
        let old = session_on(&dir, "/jail", None, Some(SessionKind::Tui), None);
        assert_eq!(matched_keys_of(&old.path).unwrap(), vec![(None, None)]);
        assert_eq!(
            matched_keys_in(&Session::read(&old.path).unwrap()),
            (None, None)
        );
        // The board's task door on serve: recorded as a task, matched as
        // web — the surface stamped is the matched one, never the kind.
        let door = session_on(
            &dir,
            "/jail",
            Some("/root"),
            Some(SessionKind::Task),
            Some(SessionKind::Web),
        );
        assert_eq!(
            matched_keys_of(&door.path).unwrap()[0].1,
            Some(SessionKind::Web)
        );
        assert_eq!(door.meta.kind, Some(SessionKind::Task));
        // No transcript at all: unreadable, not "none".
        assert!(matched_keys_of(&dir.join("missing.jsonl")).is_err());
    }

    /// The miner and the backfill stamp the goal the block was matched
    /// toward (`RunConfig::rules_goal`), never the conversation's anchor —
    /// a hand-over resumed on an older task keeps that task as its anchor
    /// while the block was matched toward the one it was handed. Per run
    /// record, like the other keys; a record with no goal gives none, and a
    /// goal this build cannot name is stamped parked, never as none. Fails
    /// on a read of `convo.goal_anchor`.
    #[test]
    fn the_miner_stamps_the_matched_goal_and_never_the_anchor() {
        use mecha_core::message::Message;
        use mecha_core::situation::GoalKey;
        let dir = scratch("mecha-reflect-goal");
        let s = session_on(&dir, "/jail", Some("/w"), Some(SessionKind::Task), None);
        let handed = GoalKey::Named("task:t-handed".parse().unwrap());
        s.append(&Record::Config(RunConfig {
            rules_workspace: Some(PathBuf::from("/w")),
            rules_surface: Some(SessionKind::Task),
            rules_goal: Some(handed.clone()),
            ..Default::default()
        }))
        .unwrap();
        s.append(&Record::GoalAnchor {
            goal: Some("task:t-older".parse().unwrap()),
        })
        .unwrap();
        s.append(&Record::Message(Message::user("go on"))).unwrap();
        let t = Session::read(&s.path).unwrap();
        assert_eq!(
            t.convo
                .goal_anchor
                .as_ref()
                .map(|g| g.to_string())
                .as_deref(),
            Some("task:t-older")
        );
        let (_, _, goal) = keys_covering(&t, 0);
        assert_eq!(
            goal,
            Some(handed.clone()),
            "the matched goal, not the anchor"
        );
        // The first run record, matched toward none, covers nothing here;
        // a transcript whose only record names none stamps none.
        let none = session_on(&dir, "/jail", Some("/w"), Some(SessionKind::Tui), None);
        none.append(&Record::GoalAnchor {
            goal: Some("task:t-confirmed".parse().unwrap()),
        })
        .unwrap();
        none.append(&Record::Message(Message::user("hi"))).unwrap();
        assert_eq!(
            keys_covering(&Session::read(&none.path).unwrap(), 0).2,
            None
        );
        // A goal this build cannot name stays parked through the stamp.
        let newer = session_on(&dir, "/jail", None, None, None);
        newer
            .append(&Record::Config(RunConfig {
                rules_goal: Some(GoalKey::Unread("dream:x".into())),
                ..Default::default()
            }))
            .unwrap();
        newer.append(&Record::Message(Message::user("hi"))).unwrap();
        let (_, _, parked) = keys_covering(&Session::read(&newer.path).unwrap(), 0);
        assert_eq!(parked, Some(GoalKey::Unread("dream:x".into())));
        let stamped =
            mecha_core::situation::Situation::recorded(&["shell".into()], "denial", None, None)
                .toward(parked);
        assert!(!stamped
            .scope()
            .matches(&mecha_core::situation::Situation::of_run(
                &["shell".into()],
                None
            )));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Row 2e-5a: the anchor is the second source of `Reflexion::goals`.
    /// A correction in a run that planned nothing takes the anchor its run
    /// recorded; a plan naming a goal at the intervention still wins; each
    /// run's own anchor is read, never the session's last; and an
    /// intervention no recorded run covers takes none.
    #[test]
    fn a_reflection_serves_the_anchor_its_run_recorded_when_no_plan_names_one() {
        use mecha_core::goal::GoalRef;
        use mecha_core::learning::Trigger;
        use mecha_core::message::{Block, Message};
        use mecha_core::planning::{Feedback, StepFeedback};
        use mecha_core::session::RunStats;
        let dir = scratch("mecha-reflect-anchor");
        let task: GoalRef = "task:t-northwind-report".parse().unwrap();
        let later: GoalRef = "trigger:lakeside-digest".parse().unwrap();
        let s = session_on(&dir, "/jail", None, Some(SessionKind::Task), None);
        let outcome = |anchor: &GoalRef| {
            Record::Outcome(RunStats {
                goal_anchor: Some(anchor.clone()),
                ..Default::default()
            })
        };
        // Run one, anchored to the task: 0 user, 1 assistant.
        s.append(&Record::Message(Message::user(
            "draft the Northwind report for Dana Whitfield",
        )))
        .unwrap();
        s.append(&Record::Message(Message::assistant(vec![Block::Text {
            text: "drafted".into(),
        }])))
        .unwrap();
        s.append(&Record::GoalAnchor {
            goal: Some(task.clone()),
        })
        .unwrap();
        s.append(&outcome(&task)).unwrap();
        // Run two, re-anchored: 2 user (a correction), 3 assistant whose
        // planning metadata names a goal of its own.
        s.append(&Record::Message(Message::user(
            "no — cite the Lakeside figures",
        )))
        .unwrap();
        let mut planned = Message::assistant(vec![Block::Text {
            text: "revised".into(),
        }]);
        planned.planning = Some(Feedback {
            steps: vec![StepFeedback {
                goal: Some("task:t-planned".parse().unwrap()),
                ..serde_json::from_value(
                    serde_json::json!({"step": "revise", "verification": "not_declared"}),
                )
                .unwrap()
            }],
            ..Default::default()
        });
        s.append(&Record::Message(planned)).unwrap();
        s.append(&Record::GoalAnchor {
            goal: Some(later.clone()),
        })
        .unwrap();
        s.append(&outcome(&later)).unwrap();
        // 4: a message no run's outcome covers yet.
        s.append(&Record::Message(Message::user("and send it")))
            .unwrap();
        let t = Session::read(&s.path).unwrap();
        let at = |at: usize| Intervention {
            trigger: Trigger::Followup,
            context: String::new(),
            text: String::new(),
            aftermath: String::new(),
            at,
            tools_before: Vec::new(),
            tools_after: Vec::new(),
        };
        assert_eq!(
            goals_for(&t, &at(1)),
            vec![task.clone()],
            "no plan names a goal, so the run's anchor is the source"
        );
        assert_eq!(
            goals_for(&t, &at(2)),
            vec![later.clone()],
            "the second run's anchor, not the first's"
        );
        assert_eq!(
            goals_for(&t, &at(3)),
            vec!["task:t-planned".parse::<GoalRef>().unwrap()],
            "a plan naming a goal at the intervention is the first source"
        );
        assert!(
            goals_for(&t, &at(4)).is_empty(),
            "uncovered: absent, never the session's last anchor ({:?})",
            t.convo.goal_anchor
        );
        assert_eq!(t.convo.goal_anchor, Some(later));

        // And the loop the row names closes: a clean reflection stamped
        // this way, behind a live rule, is a lesson `goal_lessons` serves
        // toward the anchor — where it served nothing before.
        let store = LearningStore::open(dir.join("learning")).unwrap();
        store
            .append_reflexion(&mecha_core::learning::Reflexion {
                goals: goals_for(&t, &at(1)),
                id: "r-anchored".into(),
                domain: "behavior".into(),
                session_id: t.meta.id.clone(),
                trigger: "followup".into(),
                context: "c".into(),
                intervention: "no — cite the Lakeside figures".into(),
                reflexion_text: "Cite the source figures in a report.".into(),
                error_type: None,
                confidence: None,
                is_processed: true,
                leap_run_id: None,
                created_at: "2026-09-26T00:00:00Z".into(),
                origin: Origin::Clean,
                evidence: Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                situation: None,
                situation_recomputed_at: None,
                attribution: None,
            })
            .unwrap();
        store
            .write_learned_rules(
                "behavior",
                &[mecha_core::learning::Rule {
                    text: "Cite the source figures in a report.".into(),
                    id: Some("rule-cite".into()),
                    sources: vec!["r-anchored".into()],
                    ..Default::default()
                }],
            )
            .unwrap();
        let lessons = mecha_core::learning::goal_lessons(
            &store,
            &mecha_core::situation::Situation::of_run(&[], None),
        )
        .unwrap();
        assert_eq!(
            lessons.iter().map(|l| &l.goal).collect::<Vec<_>>(),
            vec![&task],
            "{lessons:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pass over a store: a jailed row goes to the matched keys, a
    /// row whose session is gone stays as it is, a row with no key is
    /// never given one, and the second pass writes nothing.
    #[test]
    fn the_reconcile_pass_sets_from_a_read_record_and_leaves_the_rest() {
        use mecha_core::learning::Reflexion;
        let dir = scratch("mecha-reconcile-pass");
        let sessions = dir.join("sessions");
        let store = LearningStore::open(dir.join("learning")).unwrap();
        // The door: recorded as a task, the block matched as web.
        let s = session_on(
            &sessions,
            "/jail",
            Some("/root"),
            Some(SessionKind::Task),
            Some(SessionKind::Web),
        );
        let situated = |id: &str, session_id: &str, ws: Option<&str>| {
            let r = Reflexion {
                goals: Vec::new(),
                id: id.into(),
                domain: "behavior".into(),
                session_id: session_id.into(),
                trigger: "denial".into(),
                context: "c".into(),
                intervention: "Denied by the user: no".into(),
                reflexion_text: "t".into(),
                error_type: None,
                confidence: None,
                is_processed: false,
                leap_run_id: None,
                created_at: "2026-09-07T00:00:00Z".into(),
                origin: mecha_core::learning::Origin::Clean,
                evidence: mecha_core::learning::Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                // Stamped the old way: the jail, and the session's kind.
                situation: Some(mecha_core::situation::Situation::recorded(
                    &["shell".into()],
                    "denial",
                    ws.map(|_| SessionKind::Task),
                    ws.map(Path::new),
                )),
                situation_recomputed_at: None,
                attribution: None,
            };
            store.append_reflexion(&r).unwrap();
        };
        // A second attach — a question continuation — matched against
        // another workspace: a row stamped with *its* key is a key the
        // session presented, and stays.
        s.append(&Record::Config(RunConfig {
            workspace: PathBuf::from("/jail"),
            rules_workspace: Some(PathBuf::from("/second")),
            rules_surface: Some(SessionKind::Web),
            ..Default::default()
        }))
        .unwrap();
        situated("jailed", &s.meta.id, Some("/jail"));
        situated("agrees", &s.meta.id, Some("/root"));
        situated("second", &s.meta.id, Some("/second"));
        situated("gone", "20260101T000000-deadbeef", Some("/jail"));
        situated("edit", &s.meta.id, None);
        // A row whose stored surface this build cannot name — parked
        // verbatim by the wire form — against a session matched as web:
        // the reconcile replaces it, and nothing stays parked. Fails on the
        // decision being handed `surface` (always none for such a row).
        store
            .append_reflexion(&Reflexion {
                goals: Vec::new(),
                id: "parked".into(),
                domain: "behavior".into(),
                session_id: s.meta.id.clone(),
                trigger: "denial".into(),
                context: "c".into(),
                intervention: "Denied by the user: no".into(),
                reflexion_text: "t".into(),
                error_type: None,
                confidence: None,
                is_processed: false,
                leap_run_id: None,
                created_at: "2026-09-07T00:00:00Z".into(),
                origin: mecha_core::learning::Origin::Clean,
                evidence: mecha_core::learning::Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                situation: Some(
                    serde_json::from_str(r#"{"tools":["shell"],"surface":"copilot"}"#).unwrap(),
                ),
                situation_recomputed_at: None,
                attribution: None,
            })
            .unwrap();
        assert_eq!(
            store
                .reflexion("parked")
                .unwrap()
                .situation
                .unwrap()
                .surface_unread
                .as_deref(),
            Some("copilot"),
            "parked on the way in"
        );
        // A triage correction: a pass domain's row with a synthetic session
        // id and the surface the pass stamped. No transcript produced it,
        // so it is neither a missing session nor a row to touch.
        store
            .append_reflexion(&Reflexion {
                goals: Vec::new(),
                id: "triage-x".into(),
                domain: mecha_core::learning::TRIAGE_DOMAIN.into(),
                session_id: "acct/thread-1".into(),
                trigger: "correction".into(),
                context: "c".into(),
                intervention: "not spam".into(),
                reflexion_text: "t".into(),
                error_type: None,
                confidence: None,
                is_processed: false,
                leap_run_id: None,
                created_at: "2026-09-07T00:00:00Z".into(),
                origin: mecha_core::learning::Origin::Clean,
                evidence: mecha_core::learning::Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                situation: Some(mecha_core::situation::Situation::recorded(
                    &[],
                    "correction",
                    Some(SessionKind::Mail),
                    None,
                )),
                situation_recomputed_at: None,
                attribution: None,
            })
            .unwrap();
        let (listed, unreadable) = Session::list_counting(&sessions).unwrap();
        let paths: std::collections::HashMap<String, PathBuf> =
            listed.into_iter().map(|(m, p)| (m.id, p)).collect();
        assert_eq!(
            reconcile_recorded_keys(&store, &paths, unreadable, false).unwrap(),
            4,
            "jailed (both keys), agrees and second (the surface alone), parked"
        );
        let sit = |id: &str| store.reflexion(id).unwrap().situation.unwrap();
        assert_eq!(sit("jailed").workspace.as_deref(), Some(Path::new("/root")));
        assert_eq!(
            sit("jailed").surface,
            Some(SessionKind::Web),
            "the door: matched as web, whatever the session is recorded as"
        );
        assert_eq!(sit("agrees").workspace.as_deref(), Some(Path::new("/root")));
        assert_eq!(sit("agrees").surface, Some(SessionKind::Web));
        assert_eq!(
            sit("second").workspace.as_deref(),
            Some(Path::new("/second")),
            "the second attach's key is one the session presented: kept, not reverted"
        );
        assert_eq!(sit("second").surface, Some(SessionKind::Web));
        assert_eq!(
            sit("gone").workspace.as_deref(),
            Some(Path::new("/jail")),
            "unreadable: left"
        );
        assert_eq!(sit("gone").surface, Some(SessionKind::Task));
        assert_eq!(sit("edit").workspace, None, "never given a key");
        assert_eq!(sit("edit").surface, None);
        assert_eq!(
            sit("parked").surface,
            Some(SessionKind::Web),
            "replaced by what the record confirms"
        );
        assert_eq!(sit("parked").surface_unread, None, "nothing stays parked");
        let triage = store.reflexion("triage-x").unwrap();
        assert_eq!(
            triage.situation.unwrap().surface,
            Some(SessionKind::Mail),
            "untouched"
        );
        assert_eq!(
            triage.situation_recomputed_at, None,
            "never read as a session"
        );
        // A parked surface against a record that names none stays parked
        // — the downgrade case: the record side reads the newer kind as
        // none too, and clearing here would hand the rule every surface.
        let unnamed = session_on(
            &sessions,
            "/jail",
            Some("/root"),
            Some(SessionKind::Tui),
            None,
        );
        store
            .append_reflexion(&Reflexion {
                goals: Vec::new(),
                id: "parked-unnamed".into(),
                domain: "behavior".into(),
                session_id: unnamed.meta.id.clone(),
                trigger: "denial".into(),
                context: "c".into(),
                intervention: "Denied by the user: no".into(),
                reflexion_text: "t".into(),
                error_type: None,
                confidence: None,
                is_processed: false,
                leap_run_id: None,
                created_at: "2026-09-07T00:00:00Z".into(),
                origin: mecha_core::learning::Origin::Clean,
                evidence: mecha_core::learning::Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                situation: Some(
                    serde_json::from_str(r#"{"tools":["shell"],"surface":"copilot"}"#).unwrap(),
                ),
                situation_recomputed_at: None,
                attribution: None,
            })
            .unwrap();
        let (listed, unreadable) = Session::list_counting(&sessions).unwrap();
        let paths: std::collections::HashMap<String, PathBuf> =
            listed.into_iter().map(|(m, p)| (m.id, p)).collect();
        assert_eq!(
            reconcile_recorded_keys(&store, &paths, unreadable, false).unwrap(),
            0,
            "nothing to replace it with: left parked"
        );
        assert_eq!(
            sit("parked-unnamed").surface_unread.as_deref(),
            Some("copilot"),
            "still parked, still matching nothing"
        );
        // Two attaches, the first naming no surface (rules off), the second
        // naming web: the parked row takes web — never cleared to nothing.
        let mixed = session_on(
            &sessions,
            "/jail",
            Some("/root"),
            Some(SessionKind::Tui),
            None,
        );
        mixed
            .append(&Record::Config(RunConfig {
                workspace: PathBuf::from("/jail"),
                rules_workspace: Some(PathBuf::from("/root")),
                rules_surface: Some(SessionKind::Web),
                ..Default::default()
            }))
            .unwrap();
        store
            .append_reflexion(&Reflexion {
                goals: Vec::new(),
                id: "parked-mixed".into(),
                domain: "behavior".into(),
                session_id: mixed.meta.id.clone(),
                trigger: "denial".into(),
                context: "c".into(),
                intervention: "Denied by the user: no".into(),
                reflexion_text: "t".into(),
                error_type: None,
                confidence: None,
                is_processed: false,
                leap_run_id: None,
                created_at: "2026-09-07T00:00:00Z".into(),
                origin: mecha_core::learning::Origin::Clean,
                evidence: mecha_core::learning::Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                situation: Some(
                    serde_json::from_str(r#"{"tools":["shell"],"surface":"copilot"}"#).unwrap(),
                ),
                situation_recomputed_at: None,
                attribution: None,
            })
            .unwrap();
        let (listed, unreadable) = Session::list_counting(&sessions).unwrap();
        let paths: std::collections::HashMap<String, PathBuf> =
            listed.into_iter().map(|(m, p)| (m.id, p)).collect();
        assert_eq!(
            reconcile_recorded_keys(&store, &paths, unreadable, false).unwrap(),
            1
        );
        assert_eq!(
            sit("parked-mixed").surface,
            Some(SessionKind::Web),
            "the attach that names one"
        );
        assert_eq!(sit("parked-mixed").surface_unread, None);
        let file = dir.join("learning").join("reflections.jsonl");
        let before = std::fs::read(&file).unwrap();
        assert_eq!(
            reconcile_recorded_keys(&store, &paths, unreadable, false).unwrap(),
            0
        );
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "free the second time"
        );
    }

    /// A correction is placed against everything the run had read, not the
    /// list a compaction left (review of #332). `thin_old_results` cuts an
    /// old result in place with no stale marker, so against the loaded list
    /// the wrong value is gone from the result that carried it while the
    /// right one rides in full in a newer result: a data error read as the
    /// agent's. A correction the record holds twice cannot be placed, and one
    /// an extension grew afterwards still is.
    #[test]
    fn a_correction_is_placed_against_what_the_run_read_before_compaction() {
        use mecha_core::attribution::{decide, Answer, Basis, Class, Fact};
        use mecha_core::message::{Block, Message};
        let root = scratch("reflect-attribution-thinned");
        std::fs::create_dir_all(&root).unwrap();
        let s = session_in(&root, "/w", None);
        let call = |id: &str, name: &str| {
            Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: name.into(),
                input: serde_json::json!({"name": "Dana Whitfield"}),
            }])
        };
        let result = |id: &str, content: &str| {
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: id.into(),
                content: content.into(),
                is_error: false,
            }])
        };
        let long = format!(
            "Dana Whitfield. {} employer: Northwind Labs.",
            "x ".repeat(400)
        );
        let thinned = format!("{}{}", &long[..200], mecha_core::compact::TRUNCATION_MARKER);
        let correction = "No, she moved to Lakeside Institute.";
        let head = vec![
            Message::user("Where does Dana work?"),
            call("t1", "kg_entity"),
            result("t1", &long),
            call("t2", "kg_search"),
            result("t2", "Lakeside Institute: new staff this spring."),
            Message::assistant(vec![Block::text("Dana works at Northwind Labs.")]),
        ];
        for m in &head {
            s.append(&Record::Message(m.clone())).unwrap();
        }
        let mut rewritten = head.clone();
        rewritten[2] = result("t1", &thinned);
        s.append(&Record::Rewrite {
            messages: rewritten,
        })
        .unwrap();
        s.append(&Record::Message(Message::user(correction)))
            .unwrap();

        let answer = Answer::Fact(Fact {
            wrong: "Northwind Labs".into(),
            right: Some("Lakeside Institute".into()),
        });
        let text = std::fs::read_to_string(&s.path).unwrap();
        let t = Session::parse(&s.path, &text).unwrap();
        let at = t.convo.messages.len() - 1;
        assert!(!t.convo.messages[2].text().contains("Northwind"), "thinned");
        let a = decide(
            &answer,
            correction,
            &given_in(ever_before(&text, &t.convo.messages, at).as_deref()),
        );
        assert_eq!((a.class, a.basis), (Class::Data, Basis::WrongGiven));
        assert_eq!(a.source.unwrap().call, "t1");

        // Grown by an extension after it was recorded: still the one record.
        s.append(&Record::Extend {
            index: at,
            blocks: vec![Block::text("(calendar reference)")],
        })
        .unwrap();
        let text = std::fs::read_to_string(&s.path).unwrap();
        let t = Session::parse(&s.path, &text).unwrap();
        assert_eq!(t.convo.messages[at].content.len(), 2, "extended");
        let a = decide(
            &answer,
            correction,
            &given_in(ever_before(&text, &t.convo.messages, at).as_deref()),
        );
        assert_eq!((a.class, a.basis), (Class::Data, Basis::WrongGiven));

        // Said twice, word for word: which one is this is not placeable.
        s.append(&Record::Message(Message::assistant(vec![Block::text(
            "Noted.",
        )])))
        .unwrap();
        s.append(&Record::Message(Message::user(correction)))
            .unwrap();
        let text = std::fs::read_to_string(&s.path).unwrap();
        let t = Session::parse(&s.path, &text).unwrap();
        let at = t.convo.messages.len() - 1;
        let a = decide(
            &answer,
            correction,
            &given_in(ever_before(&text, &t.convo.messages, at).as_deref()),
        );
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::NoRecord));
        let _ = std::fs::remove_dir_all(&root);
    }
}
