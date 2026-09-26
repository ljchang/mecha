//! Informative decision points of a recorded session, found from records.
//!
//! `docs/APPRAISAL-WIRING-DESIGN.md` O1, row 2d-1. Whole-session paired
//! replay cannot evaluate a policy that changes behaviour — past the first
//! divergence there is no world left to grade it in, so pairs drop or tie
//! (inventory §10: twelve harness candidates rejected, four with every pair
//! tied). A point-wise comparison asks a narrower question at one moment the
//! owner already answered: from the recorded prefix, what would each of K
//! policies do *here*, and does it match what the owner decided?
//!
//! **Points come from records only.** Each kind is something the harness or
//! the owner wrote, never a model's account of its own run:
//!
//! | kind | the record | the owner's verdict | validator |
//! |---|---|---|---|
//! | steer | owner text riding beside tool results (`learning::extract_interventions`) | the steer's intent: the recording after it | `StructuralSteer` |
//! | denial | a `"Denied by the user:"` result | the refusal | `StructuralDenial` |
//! | edited draft | an outbox item sent with `args != args_before` | the released text | `ReleasedDraft` |
//! | rejected draft | an outbox item rejected | nothing sent | `RejectedDraft` |
//! | failed check | harness planning feedback: a check failed or was tampered with | an owner-bound criterion's pinned gold, when one was bound | `ArtifactGold`, else `Unposed` |
//! | surprise | harness planning feedback: a forecast the run's own count missed | none | `Unposed` |
//!
//! A point whose question no structural validator can pose — a surprise, a
//! check that is the agent's own (one-sided until the owner confirms it,
//! R11) or that a branch executing nothing cannot re-run — is **stored as
//! inconclusive, never judged** (R27): its arms are not driven, so it costs
//! no seat time and no model is ever asked to decide.
//!
//! **Drawn uniformly** ([`crate::sample`]) until 2e-6 ranks them: the pool is
//! sorted by [`Point::order`] before the seeded shuffle, because a
//! deterministic shuffle of a nondeterministic order is nondeterministic.
//!
//! What this module does not do: drive anything (the CLI's
//! `pointwise_pass`), or decide acceptance of a candidate (2d-2, R26), or
//! write a losing arm into an appraisal (2d-3, O3).

use crate::comparison::{Comparison, Kind, Role};
use crate::message::{Message, Role as MessageRole};
use crate::outbox::{Author, OutboxItem, OutboxKind};
use serde_json::Value;

/// The most policies one point compares — the recorded prompt, the deployed
/// rules and rules-free today; 2d-2's candidate takes a slot. §2.2 prices
/// K = 2–3 as what the box affords; more arms per point buy less than more
/// points, and every arm is a cold prefill (the system prompt differs, and
/// it renders ahead of the messages).
pub const ARMS_MAX: usize = 3;

/// The most assistant turns an arm may take from a point.
///
/// Every question here is decided at the first regenerated turn or soon
/// after: a steer's by the first call past the point, a denial's and a
/// draft's by the regenerated turn itself. And the branch replays under
/// `OnDivergence::Stop`, so an arm usually ends at its first departure from
/// the recording anyway. The cap bounds the worst case: at §2.2's ~45 tok/s
/// per stream under load, a turn of a few hundred tokens is 5–15 s, so an
/// arm is at most a cold prefill (~8 s for a 15k-token prefix) plus four
/// turns — about a minute — and a point of three arms about three.
pub const HORIZON_TURNS: u32 = 4;

/// Points driven per pass unless the caller says otherwise. At the horizon's
/// worst case this is under half an hour of one background seat, and
/// typically a few minutes — inside §2.2's nightly headroom for "a short
/// continuation per arm per point". Unposed points are not charged: they
/// drive nothing.
pub const DEFAULT_POINTS: usize = 8;

/// Whether a provider's endpoint is this machine — R29, held structurally:
/// every arm resubmits a recorded transcript, and sending transcripts to a
/// cloud model is the owner's privacy decision, not proposed. So only a
/// loopback URL may drive a point (`127.0.0.1`, `::1`, `localhost`); no URL
/// at all (a hosted provider's default endpoint) is not this machine. A
/// tunnel listening on loopback passes, as `imagegen::loopback_url` allows —
/// the operator built it on purpose, and no URL can reveal it.
pub fn on_this_machine(base_url: Option<&str>) -> bool {
    base_url
        .and_then(|u| reqwest::Url::parse(u).ok())
        .is_some_and(|url| crate::imagegen::is_loopback(&url))
}

/// What kind of decision point, which fixes the comparison's [`Kind`] and
/// the trigger its recorded `Situation` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PointKind {
    Steer,
    Denial,
    FailedCheck,
    EditedDraft,
    RejectedDraft,
    Surprise,
}

impl PointKind {
    pub const ALL: [PointKind; 6] = [
        PointKind::Steer,
        PointKind::Denial,
        PointKind::FailedCheck,
        PointKind::EditedDraft,
        PointKind::RejectedDraft,
        PointKind::Surprise,
    ];

    /// The comparison kind a point of this kind writes.
    pub fn comparison_kind(self) -> Kind {
        match self {
            PointKind::Steer => Kind::PointSteer,
            PointKind::Denial => Kind::PointDenial,
            PointKind::FailedCheck => Kind::PointCheck,
            PointKind::EditedDraft => Kind::PointEditedDraft,
            PointKind::RejectedDraft => Kind::PointRejectedDraft,
            PointKind::Surprise => Kind::PointSurprise,
        }
    }

    /// The trigger recorded on the point's `Situation` — the miner's word
    /// where one exists (`learning::Trigger::as_str`), so a comparison and
    /// the reflection mined at the same moment say the same thing. Recorded,
    /// never a scope key.
    pub fn trigger(self) -> &'static str {
        match self {
            PointKind::Steer => "steer",
            PointKind::Denial => "denial",
            PointKind::FailedCheck => "mismatch",
            PointKind::EditedDraft => "edit",
            PointKind::RejectedDraft => "reject",
            PointKind::Surprise => "surprise",
        }
    }

    /// The name a readout prints.
    pub fn as_str(self) -> &'static str {
        match self {
            PointKind::Steer => "steer",
            PointKind::Denial => "denial",
            PointKind::FailedCheck => "failed-check",
            PointKind::EditedDraft => "edited-draft",
            PointKind::RejectedDraft => "rejected-draft",
            PointKind::Surprise => "surprise",
        }
    }
}

/// How to find a point again in its transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locator {
    /// The intervention's recorded text, relocated by
    /// `counterfactual::locate_steer` / `locate_denial` — the probes' own
    /// matching, so a point and a reflection mined there agree.
    Intervention { text: String },
    /// An outbox item, whose `call_id` anchors its staging call.
    Draft { item_id: String },
    /// The `step`th entry of the planning feedback on the point's message.
    Step { step: usize },
}

/// One decision point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub session_id: String,
    pub kind: PointKind,
    /// The message the point is at: the intervention's message for a steer
    /// or denial, the assistant turn that staged a draft, the message the
    /// harness's planning feedback rode on for a check or a surprise.
    pub message_index: usize,
    pub locator: Locator,
    /// Registry-owned tool names around the point, the focus last — what
    /// its recorded `Situation` is built from, on the miner's construction:
    /// the intervention's window for a steer or denial, the staging turn's
    /// calls with the staging tool last for a draft, and the call the step
    /// feedback names for a check or a surprise. Names only, never inputs.
    pub tools_before: Vec<String>,
}

impl Point {
    /// A total order that does not depend on how the pool was assembled —
    /// what the seeded shuffle must be handed.
    pub fn order(&self) -> (String, usize, PointKind, String) {
        let tail = match &self.locator {
            Locator::Intervention { text, .. } => text.clone(),
            Locator::Draft { item_id } => item_id.clone(),
            Locator::Step { step } => format!("{step:08}"),
        };
        (self.session_id.clone(), self.message_index, self.kind, tail)
    }
}

/// Every decision point in one recorded session, from its records: the
/// transcript's interventions and harness planning feedback, and the outbox
/// items it staged (`drafts`, already narrowed to this session or not —
/// items naming another session are ignored).
///
/// A draft is a point only when the owner's act says something about the
/// *drafting*: a model-authored message (`Author::Model`, `OutboxKind::
/// Message` — the same filter `OutboxItem::writing_outcome` applies, since a
/// publish's diff is bookkeeping and a harness-staged card was never
/// drafted), with a recorded `call_id` to anchor it, sent after an edit that
/// survives [`crate::counterfactual::draft_form`] (a whitespace-only edit
/// cannot separate anything), or rejected. A draft sent unchanged is the
/// owner's approval, not a correction, and an arm could only tie it.
pub fn points_in(session_id: &str, messages: &[Message], drafts: &[&OutboxItem]) -> Vec<Point> {
    use crate::learning::Trigger;
    let mut out = Vec::new();
    for i in crate::learning::extract_interventions(messages) {
        let kind = match i.trigger {
            Trigger::Steer => PointKind::Steer,
            Trigger::Denial => PointKind::Denial,
            // A followup has no counterfactual to drive and the rest are
            // not found in a transcript (`appraisal_probe::replayable`).
            _ => continue,
        };
        out.push(Point {
            session_id: session_id.to_string(),
            kind,
            message_index: i.at,
            locator: Locator::Intervention { text: i.text },
            tools_before: i.tools_before,
        });
    }
    for (at, message) in messages.iter().enumerate() {
        let Some(feedback) = &message.planning else {
            continue;
        };
        // **One point per moment, not per step.** A comparison's pointers
        // name a message and a call, never a step, so two steps of one kind
        // on one message anchored to the same call are one decision the model
        // made, and would store as one row anyway: the second would read as
        // "already compared" when it was never measured (found on review).
        // The anchor is the call **as the transcript resolves it**
        // ([`call_index_of`], the coordinate the pointers carry), so an id
        // that names no call — none at all (the todo tool files a tampered
        // check with no call id), or an owner criterion's
        // `artifact-criterion:<id>`, which is never a `tool_use` — is the
        // same unanchored moment, and every criterion of one artifact case
        // is one point (its repeat is graded against the whole case anyway;
        // review pass 2). Of such steps a failed owner-bound criterion is
        // kept over a declared check, since it is the one a validator can
        // pose.
        let mut kept: Vec<(PointKind, Option<usize>, usize)> = Vec::new();
        for (step, f) in feedback.steps.iter().enumerate() {
            let kind = if f.learnable_failure() {
                PointKind::FailedCheck
            } else if f.forecast_miss() {
                PointKind::Surprise
            } else {
                continue;
            };
            // An owner-bound criterion is graded by repeating the whole
            // case, which a comparison records at no call (`call_index:
            // None`), so every one on this message is the same moment.
            let anchor = f
                .call_id
                .as_deref()
                .filter(|_| f.criterion.is_none())
                .and_then(|id| call_index_of(messages, id));
            match kept.iter_mut().find(|(k, a, _)| *k == kind && *a == anchor) {
                Some(slot) => {
                    let owner_bound = |i: usize| feedback.steps[i].criterion.is_some();
                    if !owner_bound(slot.2) && owner_bound(step) {
                        slot.2 = step;
                    }
                }
                None => kept.push((kind, anchor, step)),
            }
        }
        for (kind, _, step) in kept {
            // The call the feedback names, when the transcript still holds
            // it; the plan tool otherwise, as the mismatch miner records.
            let tool = feedback.steps[step]
                .call_id
                .as_deref()
                .and_then(|id| call_named(messages, id))
                .unwrap_or_else(|| "todo".to_string());
            out.push(Point {
                session_id: session_id.to_string(),
                kind,
                message_index: at,
                locator: Locator::Step { step },
                tools_before: vec![tool],
            });
        }
    }
    for item in drafts {
        if item.session_id.as_deref() != Some(session_id) {
            continue;
        }
        let Some(kind) = draft_kind(item) else {
            continue;
        };
        let Some(call_id) = item.call_id.as_deref() else {
            continue;
        };
        let Some(at) = staging_turn(messages, call_id) else {
            continue;
        };
        let mut tools_before: Vec<String> = Vec::new();
        for (id, name, _) in messages[at].tool_uses() {
            if id != call_id && !tools_before.iter().any(|t| t == name) {
                tools_before.push(name.to_string());
            }
        }
        tools_before.retain(|t| t != &item.tool);
        tools_before.push(item.tool.clone());
        out.push(Point {
            session_id: session_id.to_string(),
            kind,
            message_index: at,
            locator: Locator::Draft {
                item_id: item.id.clone(),
            },
            tools_before,
        });
    }
    out
}

/// The tool the call `call_id` named.
fn call_named(messages: &[Message], call_id: &str) -> Option<String> {
    messages
        .iter()
        .filter(|m| m.role == MessageRole::Assistant)
        .flat_map(|m| m.tool_uses())
        .find(|(id, _, _)| *id == call_id)
        .map(|(_, name, _)| name.to_string())
}

/// Which draft point an outbox item is, if any — see [`points_in`].
pub fn draft_kind(item: &OutboxItem) -> Option<PointKind> {
    if item.kind != OutboxKind::Message || item.author() != Author::Model {
        return None;
    }
    match item.status.as_str() {
        "rejected" => Some(PointKind::RejectedDraft),
        "sent" if item.edited() => {
            let form = |v: &Value| crate::counterfactual::draft_form(&Value::Null, v);
            (form(&item.args) != form(&item.args_before)).then_some(PointKind::EditedDraft)
        }
        _ => None,
    }
}

/// The assistant turn holding the call `call_id`.
fn staging_turn(messages: &[Message], call_id: &str) -> Option<usize> {
    messages.iter().position(|m| {
        m.role == MessageRole::Assistant
            && !m.harness
            && m.tool_uses().iter().any(|(id, _, _)| *id == call_id)
    })
}

/// The global index of the call `call_id` — the recording's coordinate a
/// comparison's `call_index` pointer uses.
pub fn call_index_of(messages: &[Message], call_id: &str) -> Option<usize> {
    let mut count = 0;
    for m in messages {
        if m.role != MessageRole::Assistant {
            continue;
        }
        for (id, _, _) in m.tool_uses() {
            if id == call_id {
                return Some(count);
            }
            count += 1;
        }
    }
    None
}

/// A uniform draw over `points`, reproducible from `seed`: the whole pool,
/// sorted by [`Point::order`] and then shuffled. A caller walks it in order
/// and stops when its budget does — any prefix of a complete shuffle is a
/// uniform sample, and so is the subsequence a predicate independent of the
/// shuffle leaves (points already compared, points not clean).
///
/// Generic over what carries the point (a caller keeps each point's
/// transcript path beside it), ordered by the point alone.
pub fn draw<T>(mut items: Vec<T>, seed: u64, point: impl Fn(&T) -> &Point) -> Vec<T> {
    items.sort_by_cached_key(|t| point(t).order());
    crate::sample::shuffled(items, seed)
}

/// `mecha sessions compare`'s draw since row 2e-6 (R39): the uniform
/// [`draw`], then stably re-ordered by the replay priority of each point's
/// session (`replay_priority::order_by_priority`) — so the points of the
/// sessions carrying the most regret are compared first, and among equal
/// priorities the seed still decides. A session `priority` has nothing for
/// has every factor unknown.
///
/// **Never the harness candidate's draw.** `compare_candidate`'s points are
/// R36's confirming sample (there is no separate holdout), and a
/// prioritised confirming sample is a biased one (`GOAL-SYSTEM-DESIGN.md`
/// §8.1); it keeps [`draw`] (R39).
pub fn draw_ranked<T>(
    items: Vec<T>,
    seed: u64,
    point: impl Fn(&T) -> &Point,
    priorities: &std::collections::BTreeMap<String, crate::replay_priority::Priority>,
) -> Vec<T> {
    let unread = crate::replay_priority::Priority::unread();
    let mut drawn = draw(items, seed, &point);
    // `sort_by` is stable: equal priorities keep the shuffle's order.
    drawn.sort_by(|a, b| {
        let p = |t: &T| priorities.get(&point(t).session_id).unwrap_or(&unread);
        crate::replay_priority::order_by_priority(p(a), p(b))
    });
    drawn
}

/// One policy a point's arms may run: its role, the rules hash it carries
/// (`RunConfig::rules_hash`'s convention), and the system prompt it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub role: Role,
    pub policy: Option<String>,
    pub system: String,
}

/// The policies a point compares, from the candidates in preference order:
/// duplicates by system prompt dropped (keeping the first — two arms under
/// one prompt measure nothing and would only tie), then at most
/// [`ARMS_MAX`]. Fewer than two left is not a comparison, which the caller
/// counts rather than drives.
pub fn distinct_policies(candidates: Vec<Policy>) -> Vec<Policy> {
    let mut out: Vec<Policy> = Vec::new();
    for c in candidates {
        if out.iter().any(|p| p.system == c.system) {
            continue;
        }
        out.push(c);
        if out.len() == ARMS_MAX {
            break;
        }
    }
    out
}

/// Whether `rows` already hold a comparison of this point under these
/// policies by this model — so a nightly pass spends its seats on points
/// it has not measured, not on the same verdict again. A different rule set
/// or model is a different measurement and is not a match.
pub fn already_compared(rows: &[Comparison], probe: &Comparison) -> bool {
    on_record(rows, probe).is_some()
}

/// The stored comparison of this point under these arms by this model, for
/// this candidate — what [`already_compared`] asks, returned so a caller
/// that needs the verdict (a candidate's re-measurement, R36) can reuse it
/// rather than pay for it again. The arms are matched as (role, policy)
/// pairs, and the candidate by `Pointers::proposal_id`: two harness
/// candidates share every rules hash at a point and differ only there.
pub fn on_record<'a>(rows: &'a [Comparison], probe: &Comparison) -> Option<&'a Comparison> {
    let arms = |c: &Comparison| {
        let mut p: Vec<(String, Option<String>)> = c
            .arms
            .iter()
            .map(|a| (crate::appraisal::enum_name(&a.role), a.policy.clone()))
            .collect();
        p.sort();
        p
    };
    let wanted = arms(probe);
    rows.iter().find(|r| {
        r.kind == probe.kind
            && r.validator == probe.validator
            && r.model == probe.model
            && r.pointers.session_id == probe.pointers.session_id
            && r.pointers.message_index == probe.pointers.message_index
            && r.pointers.call_index == probe.pointers.call_index
            && r.pointers.proposal_id == probe.pointers.proposal_id
            && arms(r) == wanted
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;
    use crate::comparison::{Arm, Outcome, Pointers, Validator};
    use crate::message::Block;
    use crate::planning::{Feedback, StepFeedback, Verification};
    use serde_json::json;

    fn tool_use(id: &str, name: &str, input: Value) -> Block {
        Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }
    fn result(id: &str, content: &str) -> Block {
        Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    fn step(
        verification: Verification,
        expected: Option<u32>,
        actual: Option<u32>,
    ) -> StepFeedback {
        StepFeedback {
            criterion: None,
            completion_batch: None,
            call_id: Some("t5".into()),
            step: "total the quarter".into(),
            goal: None,
            expected: None,
            expected_calls: expected,
            actual_calls: actual,
            verification,
            check_tampered: false,
        }
    }

    /// One session holding a point of every transcript kind: a steer, a
    /// denial, a staged draft, a failed check and a forecast miss.
    fn session() -> Vec<Message> {
        let mut steer = Message::tool_results(vec![result("t1", "q3.csv")]);
        steer
            .content
            .push(Block::text("only the third quarter, please"));
        let mut feedback = Message::tool_results(vec![result("t5", "done")]);
        feedback.planning = Some(Feedback {
            steps: vec![
                step(Verification::Failed, None, None),
                step(Verification::Passed, Some(2), Some(9)),
                step(Verification::Passed, Some(2), Some(2)),
            ],
            ..Default::default()
        });
        vec![
            Message::user("total the quarters and mail Dirk"),
            Message::assistant(vec![tool_use("t1", "fs_list", json!({}))]),
            steer,
            Message::assistant(vec![tool_use("t2", "fs_write", json!({"path": "q3.md"}))]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t2".into(),
                content: "Denied by the user: not that file".into(),
                is_error: true,
            }]),
            Message::assistant(vec![tool_use(
                "t3",
                "mail_send",
                json!({"body": "Totals attached."}),
            )]),
            Message::tool_results(vec![result("t3", "staged for review")]),
            Message::assistant(vec![tool_use("t5", "todo", json!({"op": "complete"}))]),
            feedback,
            Message::assistant(vec![Block::text("I believe it went very well")]),
        ]
    }

    fn draft(status: &str, edited: bool) -> OutboxItem {
        let args = json!({"to": "dirk@example.invalid", "body": "Totals attached."});
        let mut item: OutboxItem = serde_json::from_value(json!({
            "id": format!("draft-{status}-{edited}"),
            "status": status,
            "tool": "mail_send",
            "args_before": args,
            "args": args,
            "summary": "mail Dirk",
            "session_id": "s-ada",
            "created_at": "2026-09-25T00:00:00Z",
            "author": "model",
            "call_id": "t3",
        }))
        .unwrap();
        item.taint = Taint::default();
        if edited {
            item.args =
                json!({"to": "dirk@example.invalid", "body": "Totals attached; Q3 follows."});
        }
        item
    }

    /// Every kind is found from records, and nothing the model said is a
    /// point: the closing "I believe it went very well" is its own account
    /// and appears nowhere.
    #[test]
    fn every_kind_is_found_from_records_and_the_models_account_is_not_a_point() {
        let messages = session();
        let edited = draft("sent", true);
        let rejected = draft("rejected", false);
        let unchanged = draft("sent", false);
        let pending = draft("pending", false);
        let mut elsewhere = draft("rejected", false);
        elsewhere.session_id = Some("s-other".into());
        let points = points_in(
            "s-ada",
            &messages,
            &[&edited, &rejected, &unchanged, &pending, &elsewhere],
        );
        let kinds: Vec<(PointKind, usize)> =
            points.iter().map(|p| (p.kind, p.message_index)).collect();
        assert_eq!(
            kinds,
            vec![
                (PointKind::Steer, 2),
                (PointKind::Denial, 4),
                (PointKind::FailedCheck, 8),
                (PointKind::Surprise, 8),
                (PointKind::EditedDraft, 5),
                (PointKind::RejectedDraft, 5),
            ],
            "the forecast that held and the draft sent unchanged are not points"
        );
        assert!(points.iter().all(|p| !matches!(
            &p.locator,
            Locator::Intervention { text, .. } if text.contains("went very well")
        )));
        assert_eq!(call_index_of(&messages, "t5"), Some(3));
        let tools = |k: PointKind| {
            points
                .iter()
                .find(|p| p.kind == k)
                .map(|p| p.tools_before.clone())
                .unwrap()
        };
        assert_eq!(tools(PointKind::EditedDraft), vec!["mail_send".to_string()]);
        assert_eq!(tools(PointKind::FailedCheck), vec!["todo".to_string()]);
        assert_eq!(
            tools(PointKind::Denial).last().map(String::as_str),
            Some("fs_write"),
            "the refused tool is the focus"
        );
    }

    /// Two steps of one kind on one message, anchored to the same call or
    /// to none, are one moment and one point — not two points of which the
    /// store would keep the first and call the second "already compared".
    /// Of such steps the owner-bound criterion is kept; a step anchored to
    /// another call is its own point.
    #[test]
    fn steps_on_one_moment_are_one_point_and_the_owner_bound_one_is_kept() {
        let tampered = |call: Option<&str>| StepFeedback {
            call_id: call.map(String::from),
            check_tampered: true,
            ..step(Verification::Passed, None, None)
        };
        let owner_bound = |criterion: &str| StepFeedback {
            criterion: Some(crate::mismatch::CriterionFeedback {
                id: criterion.into(),
                artifact: "answer.json".into(),
                pointer: "/ok".into(),
                context: None,
            }),
            // What `ArtifactCase::criterion_feedback` writes: an anchor that
            // is never a `tool_use` in the transcript.
            call_id: Some(format!("artifact-criterion:{criterion}")),
            ..step(Verification::Failed, None, None)
        };
        let mut feedback = Message::tool_results(vec![result("t5", "done")]);
        feedback.planning = Some(Feedback {
            steps: vec![
                tampered(None),
                tampered(Some("t-gone")),
                owner_bound("result"),
                owner_bound("total"),
                tampered(Some("t5")),
            ],
            ..Default::default()
        });
        let messages = vec![
            Message::user("total the quarter"),
            Message::assistant(vec![tool_use("t5", "todo", json!({"op": "complete"}))]),
            feedback,
        ];
        let points = points_in("s-ada", &messages, &[]);
        let steps: Vec<(PointKind, &Locator)> =
            points.iter().map(|p| (p.kind, &p.locator)).collect();
        assert_eq!(
            steps,
            vec![
                (PointKind::FailedCheck, &Locator::Step { step: 2 }),
                (PointKind::FailedCheck, &Locator::Step { step: 4 }),
            ],
            "every step whose anchor the transcript cannot resolve — none, a \
             call that is gone, two criteria of one case — is one moment, the \
             first owner-bound one kept; the resolved anchor is its own"
        );
    }

    /// R29 as a structural check: only a model on this machine may drive a
    /// point, because every arm resubmits a recorded transcript.
    #[test]
    fn only_a_loopback_endpoint_is_on_this_machine() {
        for local in [
            "http://127.0.0.1:8080/v1",
            "http://localhost:8080",
            "http://[::1]:8080",
        ] {
            assert!(on_this_machine(Some(local)), "{local}");
        }
        for remote in [
            None,
            Some("https://api.anthropic.com"),
            Some("http://10.0.0.5:8080"),
            Some("http://localhost.example.com"),
            Some("not a url"),
        ] {
            assert!(!on_this_machine(remote), "{remote:?}");
        }
    }

    /// The draft filter: a publish, a harness-staged card, and an edit that
    /// only moved whitespace are not points; an item with no staging call
    /// recorded cannot be anchored and is skipped.
    #[test]
    fn only_a_model_drafted_message_the_owner_corrected_is_a_draft_point() {
        let mut publish = draft("rejected", false);
        publish.kind = OutboxKind::Publish;
        let mut card = draft("rejected", false);
        card.author = "harness".into();
        let mut cosmetic = draft("sent", false);
        cosmetic.args = json!({"to": "dirk@example.invalid", "body": " Totals\n attached. "});
        assert!(cosmetic.edited());
        for item in [&publish, &card, &cosmetic] {
            assert_eq!(draft_kind(item), None, "{}", item.id);
        }
        let mut anchorless = draft("rejected", false);
        anchorless.call_id = None;
        assert!(points_in("s-ada", &session(), &[&anchorless])
            .iter()
            .all(|p| p.kind != PointKind::RejectedDraft));
    }

    /// The draw is a function of the pool and the seed, not of the order the
    /// pool was assembled in — and a different seed draws differently.
    #[test]
    fn the_draw_is_seeded_and_independent_of_assembly_order() {
        let points: Vec<Point> = (0..12)
            .map(|i| Point {
                session_id: format!("s-{}", i % 3),
                kind: PointKind::ALL[i % 6],
                message_index: i,
                locator: Locator::Step { step: i },
                tools_before: vec!["todo".into()],
            })
            .collect();
        let mut reversed = points.clone();
        reversed.reverse();
        fn same(p: &Point) -> &Point {
            p
        }
        assert_eq!(draw(points.clone(), 7, same), draw(reversed, 7, same));
        assert_ne!(draw(points.clone(), 7, same), draw(points, 8, same));
    }

    /// R39: `sessions compare` ranks the uniform draw by the replay
    /// priority of each point's session, and among equal priorities keeps
    /// the shuffle's order — the seed still decides between equals.
    #[test]
    fn the_ranked_draw_leads_with_the_regret_and_keeps_the_shuffle_among_equals() {
        use crate::replay_priority::{Inputs, Priority};
        let points: Vec<Point> = (0..12)
            .map(|i| Point {
                session_id: format!("s-{}", i % 3),
                kind: PointKind::ALL[i % 6],
                message_index: i,
                locator: Locator::Step { step: i },
                tools_before: vec!["todo".into()],
            })
            .collect();
        fn same(p: &Point) -> &Point {
            p
        }
        let p = |owner_gain: f64| {
            Priority::of(&Inputs {
                owner_gain: Some(owner_gain),
                surprises: Some(0),
                recurrence: Some(1),
                age_days: 0.0,
                hopeless: Some(false),
            })
        };
        let priorities: std::collections::BTreeMap<String, Priority> = [
            ("s-0".to_string(), p(0.0)),
            ("s-1".to_string(), p(0.0)),
            ("s-2".to_string(), p(2.0)),
        ]
        .into();
        let ranked = draw_ranked(points.clone(), 7, same, &priorities);
        let uniform = draw(points, 7, same);
        assert!(ranked[..4].iter().all(|p| p.session_id == "s-2"));
        let rest: Vec<&Point> = ranked[4..].iter().collect();
        let uniform_rest: Vec<&Point> = uniform.iter().filter(|p| p.session_id != "s-2").collect();
        assert_eq!(rest, uniform_rest, "equals keep the shuffle's order");
    }

    fn policy(role: Role, system: &str) -> Policy {
        Policy {
            role,
            policy: Some(crate::learning::rules_hash(system)),
            system: system.into(),
        }
    }

    /// Two arms under one prompt measure nothing: the later duplicate goes,
    /// and at most `ARMS_MAX` remain.
    #[test]
    fn duplicate_policies_are_dropped_and_k_is_capped() {
        let kept = distinct_policies(vec![
            policy(
                Role::WithoutIntervention,
                "base\n\n## Learned rules\n- ask first",
            ),
            policy(Role::Rules, "base\n\n## Learned rules\n- ask first"),
            policy(Role::RulesFree, "base"),
        ]);
        assert_eq!(
            kept.iter().map(|p| p.role).collect::<Vec<_>>(),
            vec![Role::WithoutIntervention, Role::RulesFree]
        );
        let many = distinct_policies(
            (0..5)
                .map(|i| policy(Role::Candidate, &format!("prompt {i}")))
                .collect(),
        );
        assert_eq!(many.len(), ARMS_MAX);
    }

    fn row(policies: &[&str], model: &str) -> Comparison {
        Comparison::new(
            Kind::PointSteer,
            None,
            None,
            None,
            policies
                .iter()
                .map(|p| Arm::new(Role::Rules, Some((*p).into()), Outcome::Pass))
                .collect(),
            Validator::StructuralSteer,
            Pointers {
                session_id: "s-ada".into(),
                message_index: Some(2),
                call_index: Some(1),
                ..Pointers::default()
            },
            model,
        )
    }

    /// A point measured under these policies by this model is not measured
    /// again; a new rule set or a new model is a new measurement.
    #[test]
    fn a_point_already_compared_under_the_same_policies_is_not_compared_again() {
        let stored = vec![row(&["a", "b"], "local")];
        assert!(already_compared(&stored, &row(&["b", "a"], "local")));
        assert!(!already_compared(&stored, &row(&["a", "c"], "local")));
        assert!(!already_compared(&stored, &row(&["a", "b"], "other")));
        let mut elsewhere = row(&["a", "b"], "local");
        elsewhere.pointers.message_index = Some(4);
        assert!(!already_compared(&stored, &elsewhere));
        // The same policies under other roles are other arms: a candidate's
        // two arms share one rules hash and differ only by role.
        let mut recast = row(&["a", "b"], "local");
        recast.arms[0].role = Role::Candidate;
        assert!(!already_compared(&stored, &recast));
        let mut named = row(&["a", "b"], "local");
        named.pointers.proposal_id = Some("hc-ledger".into());
        assert!(
            !already_compared(&stored, &named),
            "a candidate's comparison is not the rules pass's"
        );
    }
}
