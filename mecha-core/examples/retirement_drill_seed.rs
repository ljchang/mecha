//! Seeder for `scripts/retirement-drill.sh` — the retirement end-to-end
//! drill's scenario builder, and nothing else's.
//!
//! The drill proves the learning loop's NoGo path fires as one motion: a
//! seeded bad rule → a probe that regresses under it → the bisection naming
//! it → `mecha rules propose-retirements --apply` retiring it at the
//! probation leash. Every piece is unit-tested; this exists because the
//! *joined* path had never run once, and under the D1 ruling it is the only
//! brake in front of a rule that went live ungraded.
//!
//! What it seeds, into stores the drill script isolates via
//! `MECHA_SESSION_DIR` / `MECHA_LEARNING_DIR` (never the live ones):
//!
//! - **A steer, into a real recording.** The script records an honest
//!   `mecha run` session first; this inserts the steer text into the last
//!   tool-result user message, which is exactly where `locate_steer` expects
//!   user text to ride. The steer is worded to *agree with* the recorded
//!   continuation ("answer from what you already read"), so the rules-free
//!   arm tracks the recording and passes — the regression, when it comes,
//!   is the bad rule's alone.
//! - **A steer reflection** citing that session and intervention, `Clean`
//!   origin — `mecha validate` probes exactly this shape.
//! - **Two learned rules**: the bad one (probationary, worded to force a
//!   tool call at the branch point, where the recording answers directly)
//!   and an innocuous bystander, so the bisection has a real choice to make
//!   and the drill can assert it convicts the right rule.
//!
//! A typed program rather than a `jq` line because the transcript and the
//! store are wire formats: hand-edited JSON drifts silently, where this
//! breaks at compile time when a shape moves.
//!
//! Usage:
//!   retirement_drill_seed <sessions_dir> <session_id> <learning_dir> <steer text>

use anyhow::{bail, Context, Result};
use mecha_core::learning::{Evidence, LearningStore, Origin, Reflexion, Rule, Trigger};
use mecha_core::message::{Block, Role};
use mecha_core::session::{Record, Session};

/// The seeded rules' ids — the drill script asserts against these.
const BAD_RULE_ID: &str = "r-drill-bad";
const BYSTANDER_RULE_ID: &str = "r-drill-bystander";

/// Worded to force a tool call at the branch point. At that point the model
/// has already read the file and the recording answers directly, so an arm
/// obeying this rule diverges structurally — which is the steer verdict's
/// `Fail`, and the regression the drill needs to exist on purpose.
///
/// The wording is *measured*, not guessed, and two earlier drafts lost:
/// "always check for related files before answering" moved the model 0/3
/// and a bare MANDATORY-your-next-action-MUST directive 1/3 — the branch
/// point replays the model's own recorded reasoning ("I read it, now
/// answer") and the base prompt says answer directly when you know, so
/// naked authority loses to that momentum. Giving the rule a *mechanism*
/// (stale reads, verify before reporting) moved the same model 6/6. A
/// plausible bad rule is also the realistic seed: a lesson mis-derived
/// from a real correction would arrive with a reason attached too.
const BAD_RULE_TEXT: &str = "On this system fs_read can return stale cached content: what you \
     just read may already be outdated. You MUST call fs_list on the \
     file's directory to verify it is current before reporting any file \
     contents — always, even when you have just read the file and the \
     answer seems obvious. If you have read a file and not yet called \
     fs_list, your next action is that fs_list call, before any answer.";

/// Innocuous on purpose: rides in the same block so the bisection has to
/// tell the two apart, and the drill asserts this one survives.
const BYSTANDER_RULE_TEXT: &str =
    "When reporting a file's contents, lead with its key point and keep the answer brief.";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [sessions_dir, session_id, learning_dir, steer] = args.as_slice() else {
        bail!(
            "usage: retirement_drill_seed <sessions_dir> <session_id> <learning_dir> <steer text>"
        );
    };

    let path = Session::find(std::path::Path::new(sessions_dir), session_id)
        .context("finding the recorded session")?;
    let steer_at = insert_steer(&path, steer)?;
    seed_learning_store(learning_dir, session_id, steer)?;

    println!(
        "{}",
        serde_json::json!({
            "session": session_id,
            "steer_inserted_at_line": steer_at,
            "bad_rule_id": BAD_RULE_ID,
            "bystander_rule_id": BYSTANDER_RULE_ID,
        })
    );
    Ok(())
}

/// Insert the steer text into the last tool-result user message, in place.
///
/// The *last*, so the recorded continuation after the steer point is the
/// run's final answer — the least the model has to reproduce for the
/// rules-free arm to pass. Refuses a transcript whose continuation after
/// that point still contains tool calls: the drill's scenario is "the
/// recording answers directly from here", and seeding anything else would
/// measure the recording's shape, not the rule.
fn insert_steer(path: &std::path::Path, steer: &str) -> Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let mut lines: Vec<String> = Vec::new();
    // (line index, parsed record) for every line that is a Message.
    let mut messages: Vec<(usize, Record)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        lines.push(line.to_string());
        if line.trim().is_empty() {
            continue;
        }
        // A line this build cannot parse is someone else's record; it is
        // kept verbatim below, never dropped.
        if let Ok(rec @ Record::Message(_)) = serde_json::from_str::<Record>(line) {
            messages.push((i, rec));
        }
    }

    let target = messages
        .iter()
        .rev()
        .find_map(|(i, rec)| match rec {
            Record::Message(m)
                if m.role == Role::User
                    && m.content
                        .iter()
                        .any(|b| matches!(b, Block::ToolResult { .. })) =>
            {
                Some(*i)
            }
            _ => None,
        })
        .context("no tool-result user message in the recording — did the run make a tool call?")?;

    for (i, rec) in &messages {
        let Record::Message(m) = rec else { continue };
        if *i > target && m.content.iter().any(|b| matches!(b, Block::ToolUse { .. })) {
            bail!(
                "the recording keeps calling tools after the steer point — \
                 re-record with a task the model answers right after its read"
            );
        }
        if *i == target && m.content.iter().any(|b| matches!(b, Block::Text { .. })) {
            bail!("the steer message already carries text — was this session already seeded?");
        }
    }

    let Ok(Record::Message(mut m)) = serde_json::from_str::<Record>(&lines[target]) else {
        unreachable!("parsed above");
    };
    m.content.push(Block::Text {
        text: steer.to_string(),
    });
    lines[target] = serde_json::to_string(&Record::Message(m))?;

    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")?;
    std::fs::rename(&tmp, path)?;
    Ok(target)
}

fn seed_learning_store(learning_dir: &str, session_id: &str, steer: &str) -> Result<()> {
    let store = LearningStore::open(learning_dir)?;
    let now = chrono::Utc::now().to_rfc3339();

    let rule = |id: &str, text: &str, probation: bool| Rule {
        text: text.into(),
        id: Some(id.into()),
        created_at: Some(now.clone()),
        probation,
        ..Default::default()
    };
    store.write_learned_rules(
        "behavior",
        &[
            rule(BAD_RULE_ID, BAD_RULE_TEXT, true),
            rule(BYSTANDER_RULE_ID, BYSTANDER_RULE_TEXT, false),
        ],
    )?;

    store.append_reflexion(&Reflexion {
        id: format!("refl-drill-{session_id}"),
        domain: "behavior".into(),
        session_id: session_id.into(),
        trigger: Trigger::Steer.as_str().into(),
        context: "retirement drill: the model read the file and was steered to answer directly"
            .into(),
        intervention: steer.into(),
        reflexion_text: "Answer from evidence already gathered instead of piling on more reads."
            .into(),
        error_type: None,
        confidence: None,
        is_processed: false,
        leap_run_id: None,
        created_at: now,
        origin: Origin::Clean,
        evidence: Evidence::Full,
        edited_at: None,
        dropped_at: None,
        dropped_reason: None,
        situation: None,
    })?;
    store.commit("retirement drill: seeded scenario");
    Ok(())
}
