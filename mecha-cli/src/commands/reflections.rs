//! `mecha reflections` — the lessons, before anything consolidates them.
//!
//! **The store had no reader.** `mecha reflect` wrote reflections, `learn`
//! consumed them, `validate` probed them and `doctor` counted them, and there
//! was no way to see one short of `cat reflections.jsonl`. That is the wrong
//! end of the pipeline to be blind at: a rule is a *consolidation* of several
//! lessons, so by the time a proposal is reviewable the thing to disagree with
//! has already been merged with four others and rewritten. The lesson is where
//! a disagreement is cheap and precise.
//!
//! Four verbs, and the two that mutate are the point:
//!
//! - **`edit`** rewrites the lesson in the owner's own words, which is a
//!   *provenance promotion* rather than a text change — see
//!   [`LearningStore::edit_reflexion`]. It is the way an excluded reflection is
//!   rescued: a lesson the owner typed skips the model that would otherwise
//!   have laundered third-party bytes into it.
//! - **`drop`** refuses one. A flag, never a deletion, on the rule
//!   `retired_at` and the outbox's resolved items already follow — a store
//!   that forgets its refusals lets the same lesson come back next pass with
//!   nothing to say it was already judged. `restore` undoes it.
//!
//! Nothing here calls a model or touches the network, so it is safe to run
//! against a store the nightly is also using — every write takes the store
//! lock, which is what stops two rewrites from being a lost update.

use anyhow::{Context, Result};
use mecha_core::learning::{LearningStore, Origin, Reflexion};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Every reflection, newest first (default).
    List {
        /// Only this domain — `behavior`, `writing`, `triage`.
        #[arg(long)]
        domain: Option<String>,

        /// Include dropped ones, which are hidden by default.
        #[arg(long)]
        all: bool,

        /// Machine-readable, for `/learning`.
        #[arg(long)]
        json: bool,
    },

    /// One reflection in full: what was happening, what was said, the lesson,
    /// and whether it can become a rule.
    Show {
        id: String,

        #[arg(long)]
        json: bool,
    },

    /// Rewrite the lesson in your own words.
    ///
    /// With no `--text`, opens `$EDITOR` on the lesson alone — the outbox's
    /// rule, because editing prose inside a JSON string literal means typing
    /// `\n` for a paragraph break and one slip is a parse error that discards
    /// the whole edit.
    Edit {
        id: String,

        /// The new lesson. Omit to use `$EDITOR`.
        #[arg(long)]
        text: Option<String>,
    },

    /// Refuse a reflection: kept as evidence, never a candidate again.
    Drop {
        id: String,

        /// Why — recorded on the record for the next reader.
        #[arg(long)]
        reason: Option<String>,
    },

    /// Undo a drop.
    Restore { id: String },
}

pub async fn execute(args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    match args.cmd.unwrap_or(Cmd::List {
        domain: None,
        all: false,
        json: false,
    }) {
        Cmd::List { domain, all, json } => list(&store, domain.as_deref(), all, json),
        Cmd::Show { id, json } => show(&store, &id, json),
        Cmd::Edit { id, text } => edit(&store, &id, text),
        Cmd::Drop { id, reason } => {
            let _lock = store.lock()?;
            let r = store.drop_reflexion(&id, reason)?;
            println!("dropped {}", r.id);
            Ok(())
        }
        Cmd::Restore { id } => {
            let _lock = store.lock()?;
            let r = store.restore_reflexion(&id)?;
            println!("restored {} — learnable: {}", r.id, yes_no(r.learnable()));
            Ok(())
        }
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// Why this one will never become a rule, in one phrase, or `None` when it
/// can.
///
/// **A reason, never a bare "no".** The gate has four different ways to
/// refuse and they call for four different actions from the owner — edit it,
/// restore it, nothing, nothing — so a surface that only says *excluded*
/// makes the loop look broken when it is working.
fn blocked_because(r: &Reflexion) -> Option<String> {
    if r.dropped_at.is_some() {
        return Some(match &r.dropped_reason {
            Some(why) => format!("dropped — {why}"),
            None => "dropped".into(),
        });
    }
    match r.provenance() {
        Origin::Clean => None,
        Origin::Derived => {
            Some("mecha's own words — nothing can grade it (edit to adopt it)".into())
        }
        Origin::Untrusted => Some(
            "third-party content was in context when it was mined (edit to make it yours)".into(),
        ),
    }
}

fn list(store: &LearningStore, domain: Option<&str>, all: bool, as_json: bool) -> Result<()> {
    let mut rows = store.reflexions()?;
    rows.retain(|r| domain.is_none_or(|d| r.domain == d));
    if !all {
        rows.retain(|r| r.dropped_at.is_none());
    }
    rows.reverse();

    if as_json {
        let out: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "domain": r.domain,
                    "trigger": r.trigger,
                    "title": r.reflexion_text,
                    "origin": format!("{:?}", r.provenance()).to_lowercase(),
                    "learnable": r.learnable(),
                    "blocked": blocked_because(r),
                    "edited": r.edited_at.is_some(),
                    "dropped": r.dropped_at.is_some(),
                    "processed": r.is_processed,
                    "created_at": r.created_at,
                    "session_id": r.session_id,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("no reflections yet — they are mined by `mecha reflect`");
        return Ok(());
    }

    let learnable = rows.iter().filter(|r| r.learnable()).count();
    println!(
        "{} reflection(s) · {learnable} can become rules\n",
        rows.len()
    );
    for r in &rows {
        let mark = match (r.dropped_at.is_some(), r.edited_at.is_some()) {
            (true, _) => "✗",
            (_, true) => "✎",
            _ => " ",
        };
        println!(
            "{mark} {:<26} {:<9} {:<9} {}",
            short(&r.id),
            r.domain,
            r.trigger,
            first_line(&r.reflexion_text, 70)
        );
        if let Some(why) = blocked_because(r) {
            println!("  {:<26} └ {why}", "");
        }
    }
    println!("\n`mecha reflections show <id>` · `edit <id>` · `drop <id>`");
    Ok(())
}

/// One reflection as JSON: **the record plus the gate's own verdict**,
/// because the two can disagree and the raw field is not the one anything
/// acts on.
///
/// `origin` is what was *recorded*; `provenance()` recomputes it, and for a
/// harness-voice reflection stored before `is_harness_voice` existed the
/// stored field says `clean` while the gate says `derived`. `list --json`
/// already reports the computed answer and derives `blocked` from it, so a
/// detail view rendering the raw field would contradict the row directly
/// above it — on exactly the records a detail view exists to adjudicate. The
/// stored field is kept, under a name that says which one it is.
fn detail_payload(r: &Reflexion) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(r)?;
    if let Some(map) = value.as_object_mut() {
        if let Some(recorded) = map.remove("origin") {
            map.insert("recorded_origin".into(), recorded);
        }
        map.insert(
            "provenance".into(),
            serde_json::json!(format!("{:?}", r.provenance()).to_lowercase()),
        );
        map.insert("learnable".into(), serde_json::json!(r.learnable()));
        map.insert("blocked".into(), serde_json::json!(blocked_because(r)));
    }
    Ok(value)
}

fn show(store: &LearningStore, id: &str, as_json: bool) -> Result<()> {
    let r = store.reflexion(id)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&detail_payload(&r)?)?);
        return Ok(());
    }
    println!("{}  ·  {}  ·  {}", r.id, r.domain, r.trigger);
    println!("session {}  ·  {}", r.session_id, r.created_at);
    // Absent is said, never rendered as "everywhere": a reflection mined
    // before the field batches as standing, and the roster should not make
    // that look like a measured fact about where it happened.
    println!(
        "situation: {}",
        r.situation
            .as_ref()
            .map(|s| s.describe())
            .unwrap_or_else(|| "unknown (mined before it was recorded)".into())
    );
    if let Some(at) = &r.edited_at {
        println!("edited by you {at}");
    }
    println!();
    println!("lesson:");
    println!("  {}", r.reflexion_text);
    println!();
    println!("while:");
    println!("  {}", r.context.replace('\n', "\n  "));
    println!();
    // Labelled by what it is, because for a `steer` these are somebody's typed
    // words and for a harness voice they are mecha's own — and the whole
    // provenance question turns on which.
    println!("the intervention:");
    println!("  {}", r.intervention.replace('\n', "\n  "));
    println!();
    println!(
        "provenance {:?} · evidence {:?} · learnable {}",
        r.provenance(),
        r.evidence,
        yes_no(r.learnable())
    );
    if let Some(why) = blocked_because(&r) {
        println!("  └ {why}");
    }
    Ok(())
}

fn edit(store: &LearningStore, id: &str, text: Option<String>) -> Result<()> {
    let before = store.reflexion(id)?;
    let lesson = match text {
        Some(t) => t,
        None => {
            let edited = crate::editor::edit_text(
                &before.reflexion_text,
                &format!("mecha-lesson-{}.md", std::process::id()),
            )
            .context("editing the lesson")?;
            if edited.trim() == before.reflexion_text.trim() {
                println!("unchanged");
                return Ok(());
            }
            edited
        }
    };
    let _lock = store.lock()?;
    let after = store.edit_reflexion(&before.id, &lesson)?;
    println!("{}\n  {}", after.id, after.reflexion_text);
    // `provenance()` returns `Clean` unconditionally once `edited_at` is
    // set, so this is exactly "did `before` need the promotion" — the same
    // condition `edit_reflexion` withholds on, now that it no longer fires
    // on an already-clean record just because `evidence` was `Full`.
    if before.provenance() != after.provenance() {
        println!(
            "\nprovenance {:?} → {:?}: the lesson is yours now, so it can become a rule.",
            before.provenance(),
            after.provenance()
        );
        println!("what was happening has been withheld — it is what held the third-party text.");
    }
    Ok(())
}

fn short(id: &str) -> String {
    id.chars().take(26).collect()
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    match line.chars().count() > max {
        true => format!("{}…", line.chars().take(max - 1).collect::<String>()),
        false => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::learning::{Evidence, Trigger};

    fn r(origin: Origin) -> Reflexion {
        Reflexion {
            id: "r1".into(),
            domain: "behavior".into(),
            session_id: "s1".into(),
            trigger: Trigger::Steer.as_str().into(),
            context: "working".into(),
            intervention: "no, the other one".into(),
            reflexion_text: "Use the other config.".into(),
            error_type: None,
            confidence: None,
            is_processed: false,
            leap_run_id: None,
            created_at: "2026-08-27T00:00:00Z".into(),
            origin,
            evidence: Evidence::Full,
            edited_at: None,
            dropped_at: None,
            dropped_reason: None,
            situation: None,
        }
    }

    /// Four ways to be refused, four different things for the owner to do. A
    /// surface that says only "excluded" makes a working gate look broken.
    #[test]
    fn every_refusal_says_which_one_it_is_and_what_to_do() {
        assert_eq!(blocked_because(&r(Origin::Clean)), None);

        let untrusted = blocked_because(&r(Origin::Untrusted)).unwrap();
        assert!(untrusted.contains("third-party") && untrusted.contains("edit"));

        let mut derived = r(Origin::Clean);
        derived.intervention = "Your previous turn ended without producing anything — the token \
                                budget went entirely to reasoning before you began your answer. \
                                Do not start the task over and do not re-derive what you already \
                                worked out. Either give your answer now, briefly, using what you \
                                already know, or make the single next tool call. Keep your \
                                reasoning short this turn."
            .into();
        let own = blocked_because(&derived).unwrap();
        assert!(own.contains("mecha's own words"), "{own}");

        let mut dropped = r(Origin::Clean);
        dropped.dropped_at = Some("2026-08-27T01:00:00Z".into());
        dropped.dropped_reason = Some("too specific".into());
        assert_eq!(
            blocked_because(&dropped).as_deref(),
            Some("dropped — too specific")
        );
    }

    /// A dropped reflection is refused for being dropped, not for whatever
    /// else is also true of it — the owner said no, and that is the answer.
    #[test]
    fn a_drop_outranks_a_provenance_reason() {
        let mut both = r(Origin::Untrusted);
        both.dropped_at = Some("2026-08-27T01:00:00Z".into());
        assert_eq!(blocked_because(&both).as_deref(), Some("dropped"));
    }

    /// **`show --json` must answer with the gate's verdict, not the stored
    /// field**, because the two disagree on exactly the records a detail view
    /// exists to adjudicate: a harness-voice reflection recorded before
    /// `is_harness_voice` existed stores `clean` and computes `derived`. A
    /// pane rendering the raw field would contradict the listing right above
    /// it, which reports `provenance()`.
    #[test]
    fn the_detail_payload_reports_the_computed_provenance_not_the_stored_one() {
        let mut harness_voice = r(Origin::Clean);
        harness_voice.intervention = "Your previous turn ended without producing anything — the \
                                      token budget went entirely to reasoning before you began \
                                      your answer. Do not start the task over and do not \
                                      re-derive what you already worked out. Either give your \
                                      answer now, briefly, using what you already know, or make \
                                      the single next tool call. Keep your reasoning short this \
                                      turn."
            .into();
        assert_eq!(harness_voice.origin, Origin::Clean, "as recorded");
        assert_eq!(
            harness_voice.provenance(),
            Origin::Derived,
            "as the gate recomputes it — this is the disagreement"
        );

        let payload = detail_payload(&harness_voice).unwrap();
        assert_eq!(
            payload["provenance"], "derived",
            "the verdict anything acts on"
        );
        assert_eq!(payload["recorded_origin"], "clean", "kept, correctly named");
        assert!(
            payload.get("origin").is_none(),
            "the bare name would be read as the gate's answer"
        );
        assert_eq!(payload["learnable"], false);
        assert!(payload["blocked"]
            .as_str()
            .unwrap()
            .contains("mecha's own words"));
    }
}
