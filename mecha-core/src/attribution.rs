//! A correction attributed by what the run was given — mecha-graph's D3
//! error contract, ported (`APPRAISAL-WIRING-DESIGN.md` L7, row 2e-3).
//!
//! **The contract, as the graph states it** (`mecha-graph` `docs/PLAN.md`,
//! "D3 — The error contract: who repairs what"): one correction event, two
//! independent consumers, and the attribution decided by *what the context
//! pack contained* — so the verdict is mechanical:
//!
//! | what the run was given           | class               | who repairs          |
//! |----------------------------------|---------------------|----------------------|
//! | the wrong fact                   | **data error**      | the source           |
//! | the right fact, ignored/misused  | **behaviour error** | the agent            |
//! | neither                          | **gap**, nobody's fault | a retrieval target |
//!
//! and on the agent's side, "its reflector mines a lesson **only** on a
//! behavior-error verdict". The graph's half — supersede, negate, demote the
//! producing class, sweep its other output — is `mecha-graph-core`'s
//! `corrections.rs`, fed by the distiller's `meta.corrections`, which mecha
//! already ships for every clean session (D3's "the distiller ships the
//! correction always"; `distill::Correction`). This module is the agent's
//! half, which was never built: every correction mecha's reflector saw was
//! mined as a behaviour lesson, including the ones where the run did exactly
//! what the data told it to.
//!
//! **What was ported, and what had to change.**
//!
//! - *The table*, row for row, including its order: the wrong value among
//!   what the run was given decides a data error even when the right value
//!   was there too, as D3's first row does. The run then held two sources
//!   that disagreed, and the one to repair is the source.
//! - *The pack* is [`crate::grounding::calls`] over the messages before the
//!   correction — the results the model actually read, first seen wins,
//!   stale never evidence. A result that arrived *with* a steer was not yet
//!   acted on, so it is not given. Results a compaction has since cut are
//!   not in the live list; such a correction reads as a gap, which is never
//!   mined — the conservative direction.
//! - *The correction's content* — what was wrong, what is right — comes
//!   from a model in D3 too: the graph's correction event is the distiller's
//!   `{wrong, right, about}`. Here it is the reflector's, because the
//!   distiller's prompt is pinned byte-identical (R32) and its corrections
//!   are per session, while a reflection is per intervention. The model
//!   extracts two spans; it never names the class. [`decide`] does.
//! - *The spans are grounded before they are used* — `right` must be the
//!   owner's words and `wrong` must be something the run said or was given
//!   (both by [`crate::grounding::holds`]) — because a paraphrase would read
//!   as absent, and absence decides a gap. A span that does not dereference
//!   makes the attribution unknown rather than a guess.
//!
//! **Where each class lands in mecha.** A behaviour error is the only class
//! `learn` may mine into a behaviour rule ([`crate::learning::Reflexion::attribution_admits`]).
//! A data error is recorded on the reflection with the call that carried the
//! wrong value ([`Source`]); its repair is the graph's existing path when that
//! source is the graph, since the distiller ships the same correction there —
//! nothing new crosses. A gap is recorded on the reflection with the fact it
//! lacked, as a retrieval target; **mecha has no retrieval-target store**, and
//! none is invented here (the graph's `query_log` gap queue has no write verb
//! from mecha). An unknown attribution is counted and never mined.
//!
//! **A correction with no fact at issue** — "stop, don't run that", "shorter,
//! please" — is outside D3's table, which is about facts. The reflector says
//! so (`"fact": false`) and it is a behaviour correction, as every correction
//! was before this module: the change only ever narrows what is mined.
//! A reply that answers neither way is unknown — never clean.

use crate::grounding::{holds, Call};
use crate::message::{Message, Role};
use serde::{Deserialize, Serialize};

/// The fewest letters and digits a span may carry and still be looked up.
/// Below it a span matches by coincidence — "a", "no", a stray initial — and
/// a coincidental match decides a class. Three admits "9th" and "Yale".
pub const MIN_SPAN_CHARS: usize = 3;

/// D3's three verdicts, and the one this build adds for a correction it
/// could not place. A wire format: a word this build cannot read loads as
/// [`Class::Unknown`], which is never mined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// The run had the right data, or no fact was at issue, and acted
    /// wrongly. The only class a behaviour rule is mined from.
    Behaviour,
    /// The run was given the wrong data and used it. The source is repaired.
    Data,
    /// The run was given neither. Nobody's fault; a retrieval target.
    Gap,
    /// Not placed — see [`Basis`] for why.
    #[serde(other)]
    Unknown,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Behaviour => "behaviour",
            Class::Data => "data",
            Class::Gap => "gap",
            Class::Unknown => "unknown",
        }
    }
}

/// Why a correction has the class it has. Kept beside the class so a reader
/// can tell a behaviour error found by lookup from one with no fact at
/// issue, and one unknown for a missing answer from one for an ungrounded
/// span. A wire format, like [`Class`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Behaviour: the reflector named no fact at issue.
    NoFact,
    /// Behaviour: the right value was in a result the run read, the wrong
    /// one in none.
    RightGiven,
    /// Data: the wrong value was in a result the run read.
    WrongGiven,
    /// Gap: neither value was in any result the run read.
    NeitherGiven,
    /// Unknown: the reflector's reply said nothing either way.
    NotAnswered,
    /// Unknown: a span was not where it must be — `right` not in the
    /// owner's words, or `wrong` in nothing the run said or was given.
    Ungrounded,
    /// Unknown: a span below [`MIN_SPAN_CHARS`], or no `wrong` at all.
    TooShort,
    /// Unknown: what the run was given could not be read.
    NoRecord,
    /// A basis word this build cannot read.
    #[serde(other)]
    Unread,
}

/// The two spans the reflector copied: what was wrong, and what is right.
/// `right` absent is a rejection with no replacement, as in
/// `distill::Correction`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub wrong: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
}

/// The call whose result decided the class: the wrong value's carrier for a
/// data error — where the repair is owed — or the right value's for a
/// behaviour error. A pointer, never the result's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// The `tool_use` id.
    pub call: String,
    /// The tool's registry name.
    pub tool: String,
}

/// What the attribution found, as recorded on a reflection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    #[serde(default = "unknown_class")]
    pub class: Class,
    #[serde(default = "unread_basis")]
    pub basis: Basis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact: Option<Fact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

fn unknown_class() -> Class {
    Class::Unknown
}

fn unread_basis() -> Basis {
    Basis::Unread
}

impl Attribution {
    /// The one constructor that derives the class from the basis, so the two
    /// cannot disagree on a record this build writes.
    pub fn new(basis: Basis, fact: Option<Fact>, source: Option<Source>) -> Attribution {
        let class = match basis {
            Basis::NoFact | Basis::RightGiven => Class::Behaviour,
            Basis::WrongGiven => Class::Data,
            Basis::NeitherGiven => Class::Gap,
            Basis::NotAnswered
            | Basis::Ungrounded
            | Basis::TooShort
            | Basis::NoRecord
            | Basis::Unread => Class::Unknown,
        };
        Attribution {
            class,
            basis,
            fact,
            source,
        }
    }

    /// A recorded attribution this build could not parse at all — unknown,
    /// never absent, since absent means "recorded before attribution".
    pub fn unread() -> Attribution {
        Attribution::new(Basis::Unread, None, None)
    }
}

/// Deserialize a reflection's attribution without letting it fail the
/// record: `null` or absent is `None`, anything else that does not parse is
/// [`Attribution::unread`] — unknown, and so never mined.
pub fn de_lenient<'de, D>(d: D) -> Result<Option<Attribution>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_value(v).unwrap_or_else(|_| Attribution::unread()),
    ))
}

/// What the reflector said about a fact, before anything is looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The reply carried no answer either way — a reply shape from before
    /// the field, or a model that ignored it.
    NotAnswered,
    /// No fact at issue: the correction was about how the work was done.
    NoFact,
    /// A fact was corrected; these are the spans as the reflector copied them.
    Fact(Fact),
}

impl Answer {
    /// Read the reflector's three optional fields, leniently: a `"fact"`
    /// that is a boolean or the strings `"true"` / `"false"` is an answer,
    /// anything else is none. A formatting slip in an optional field must
    /// not cost the lesson it rides beside — the distiller's corrections
    /// learned that first (`distill::DistillerReply`).
    pub fn from_reply(
        fact: Option<&serde_json::Value>,
        wrong: Option<&serde_json::Value>,
        right: Option<&serde_json::Value>,
    ) -> Answer {
        let said = match fact {
            Some(serde_json::Value::Bool(b)) => Some(*b),
            Some(serde_json::Value::String(s)) => match s.trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        };
        let text = |v: Option<&serde_json::Value>| {
            v.and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        match said {
            None => Answer::NotAnswered,
            Some(false) => Answer::NoFact,
            Some(true) => Answer::Fact(Fact {
                wrong: text(wrong).unwrap_or_default(),
                right: text(right),
            }),
        }
    }
}

/// What the run had in front of it when it was corrected.
#[derive(Debug, Clone)]
pub struct Given<'a> {
    /// The calls issued before the correction, each with the result the
    /// model read ([`crate::grounding::calls`]). `None` when the record
    /// could not be read — which makes a fact unplaceable, never a gap.
    pub calls: Option<Vec<Call<'a>>>,
    /// What the run itself said or did before the correction: its text and
    /// its calls' arguments. Where a `wrong` the run asserted without being
    /// handed it is found — the grounding of the span, not the verdict.
    pub said: Vec<String>,
}

impl<'a> Given<'a> {
    /// Everything before message `at`: the calls whose results the model
    /// had read, and what the assistant wrote. Harness calls are kept, as
    /// `grounding::calls` keeps them — what came back was shown to the
    /// model whoever asked.
    pub fn before(messages: &'a [Message], at: usize) -> Given<'a> {
        let upto = &messages[..at.min(messages.len())];
        let mut said = Vec::new();
        for m in upto.iter().filter(|m| m.role == Role::Assistant) {
            let text = m.text();
            if !text.trim().is_empty() {
                said.push(text);
            }
            for (_, _, input) in m.tool_uses() {
                said.push(input.to_string());
            }
        }
        Given {
            calls: Some(crate::grounding::calls(upto)),
            said,
        }
    }

    /// A record that could not be read: nothing is placeable against it.
    pub fn unreadable() -> Given<'static> {
        Given {
            calls: None,
            said: Vec::new(),
        }
    }
}

/// Is a reflection from this domain and trigger an owner's correction that
/// D3 attributes? The `behavior` domain's corrections are; a `mismatch` is
/// the harness observing a failed check, not the owner correcting anything,
/// and keeps its own gate (`StepFeedback::learnable_failure`); `writing`
/// learns the owner's voice from edits and `triage` a classifier's buckets,
/// neither of which is a behaviour rule.
///
/// **Fail-closed on a trigger this build does not know**: in `behavior`,
/// anything but `mismatch` is a correction and needs a behaviour
/// attribution to be mined. A later trigger that is not an owner's
/// correction — 2e-4's owner-verified successes, say — joins the exemption
/// here deliberately, with its reason, rather than by default.
pub fn in_scope(domain: &str, trigger: &str) -> bool {
    domain == "behavior" && trigger != "mismatch"
}

/// Place one correction by what the run was given — D3's table, in order.
///
/// `owner_words` is the correction as the owner gave it (the steer, the
/// denial's reason, the follow-up, the rejection); `right` must be in it.
pub fn decide(answer: &Answer, owner_words: &str, given: &Given<'_>) -> Attribution {
    let fact = match answer {
        Answer::NotAnswered => return Attribution::new(Basis::NotAnswered, None, None),
        Answer::NoFact => return Attribution::new(Basis::NoFact, None, None),
        Answer::Fact(f) => f.clone(),
    };
    let long_enough = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_SPAN_CHARS;
    if !long_enough(&fact.wrong) || fact.right.as_deref().is_some_and(|r| !long_enough(r)) {
        return Attribution::new(Basis::TooShort, Some(fact), None);
    }
    // The spans are grounded before they decide anything: a paraphrase is
    // absent from every result, and absence decides a gap.
    if fact
        .right
        .as_deref()
        .is_some_and(|r| !holds(owner_words, r))
    {
        return Attribution::new(Basis::Ungrounded, Some(fact), None);
    }
    let Some(calls) = &given.calls else {
        return Attribution::new(Basis::NoRecord, Some(fact), None);
    };
    let carrier = |span: &str| {
        calls.iter().find_map(|c| {
            c.result.filter(|r| holds(r, span)).map(|_| Source {
                call: c.id.to_string(),
                tool: c.name.to_string(),
            })
        })
    };
    let wrong_given = carrier(&fact.wrong);
    let wrong_grounded = wrong_given.is_some()
        || holds(owner_words, &fact.wrong)
        || given.said.iter().any(|s| holds(s, &fact.wrong));
    if !wrong_grounded {
        return Attribution::new(Basis::Ungrounded, Some(fact), None);
    }
    // D3's first row decides first: the wrong value among what the run was
    // given is the source's to repair, whatever else it was given.
    if let Some(source) = wrong_given {
        return Attribution::new(Basis::WrongGiven, Some(fact), Some(source));
    }
    if let Some(source) = fact.right.as_deref().and_then(carrier) {
        return Attribution::new(Basis::RightGiven, Some(fact), Some(source));
    }
    Attribution::new(Basis::NeitherGiven, Some(fact), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Block;
    use serde_json::json;

    fn fact(wrong: &str, right: Option<&str>) -> Answer {
        Answer::Fact(Fact {
            wrong: wrong.into(),
            right: right.map(Into::into),
        })
    }

    /// A run that looked Dana up, said where she works, and was corrected.
    fn run(result: &str) -> Vec<Message> {
        vec![
            Message::user("Where does Dana Whitfield work now?"),
            Message::assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "kg_entity".into(),
                input: json!({"name": "Dana Whitfield"}),
            }]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: result.into(),
                is_error: false,
            }]),
            Message::assistant(vec![Block::text("Dana Whitfield works at Northwind Labs.")]),
            Message::user("No — she moved to Lakeside Institute in the spring."),
        ]
    }

    const OWNER: &str = "No — she moved to Lakeside Institute in the spring.";

    #[test]
    fn the_wrong_value_among_what_was_given_is_a_data_error_pointing_at_its_carrier() {
        let m = run("Dana Whitfield — employer: Northwind Labs (2024).");
        let a = decide(
            &fact("Northwind Labs", Some("Lakeside Institute")),
            OWNER,
            &Given::before(&m, 4),
        );
        assert_eq!((a.class, a.basis), (Class::Data, Basis::WrongGiven));
        assert_eq!(
            a.source,
            Some(Source {
                call: "t1".into(),
                tool: "kg_entity".into()
            })
        );
    }

    #[test]
    fn the_right_value_given_and_the_wrong_one_not_is_a_behaviour_error() {
        let m = run("Dana Whitfield — employer: Lakeside Institute (since March).");
        let a = decide(
            &fact("Northwind Labs", Some("Lakeside Institute")),
            OWNER,
            &Given::before(&m, 4),
        );
        assert_eq!((a.class, a.basis), (Class::Behaviour, Basis::RightGiven));
        assert_eq!(a.source.unwrap().tool, "kg_entity");
    }

    #[test]
    fn neither_value_given_is_a_gap() {
        let m = run("Dana Whitfield — no employer on record.");
        let a = decide(
            &fact("Northwind Labs", Some("Lakeside Institute")),
            OWNER,
            &Given::before(&m, 4),
        );
        assert_eq!((a.class, a.basis), (Class::Gap, Basis::NeitherGiven));
        assert_eq!(a.source, None);
    }

    /// D3's first row outranks its second: two results that disagree are a
    /// source to repair, not a run that chose badly.
    #[test]
    fn both_values_given_is_still_a_data_error() {
        let m = run("Northwind Labs (2024); Lakeside Institute (2026).");
        let a = decide(
            &fact("Northwind Labs", Some("Lakeside Institute")),
            OWNER,
            &Given::before(&m, 4),
        );
        assert_eq!(a.class, Class::Data);
    }

    /// A rejection names no replacement: given, the wrong value is data;
    /// not given, the run had nothing — a gap, per D3's third row.
    #[test]
    fn a_rejection_with_no_replacement_is_data_or_a_gap() {
        let given = run("Dana Whitfield — employer: Northwind Labs.");
        let a = decide(
            &fact("Northwind Labs", None),
            "She never worked at Northwind Labs.",
            &Given::before(&given, 4),
        );
        assert_eq!(a.class, Class::Data);
        let not = run("Dana Whitfield — nothing on record.");
        let a = decide(
            &fact("Northwind Labs", None),
            "She never worked at Northwind Labs.",
            &Given::before(&not, 4),
        );
        assert_eq!(a.class, Class::Gap);
    }

    #[test]
    fn no_fact_at_issue_is_behaviour_and_no_answer_is_unknown() {
        let m = run("irrelevant");
        let g = Given::before(&m, 4);
        let a = decide(&Answer::NoFact, "stop, don't look her up", &g);
        assert_eq!((a.class, a.basis), (Class::Behaviour, Basis::NoFact));
        let a = decide(&Answer::NotAnswered, "stop, don't look her up", &g);
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::NotAnswered));
    }

    /// A paraphrase is absent from every result, and absence decides a gap —
    /// so a span that does not dereference makes the attribution unknown.
    #[test]
    fn an_ungrounded_span_is_unknown_never_a_gap() {
        let m = run("Dana Whitfield — employer: Northwind Labs.");
        let g = Given::before(&m, 4);
        // `right` not in the owner's words.
        let a = decide(&fact("Northwind Labs", Some("Lakeside U")), OWNER, &g);
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::Ungrounded));
        // `wrong` in nothing the run said, was given, or the owner said.
        let a = decide(
            &fact("the Northwind job", Some("Lakeside Institute")),
            OWNER,
            &g,
        );
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::Ungrounded));
    }

    /// The run asserted the wrong value itself, from nothing it was given.
    /// Grounded (the run said it), and a gap by D3's table.
    #[test]
    fn a_wrong_value_only_the_run_said_is_grounded_and_a_gap() {
        let m = run("Dana Whitfield — no employer on record.");
        let a = decide(
            &fact("works at Northwind Labs", Some("Lakeside Institute")),
            OWNER,
            &Given::before(&m, 4),
        );
        assert_eq!((a.class, a.basis), (Class::Gap, Basis::NeitherGiven));
    }

    #[test]
    fn a_short_or_missing_span_is_unknown() {
        let m = run("Dana Whitfield — employer: Northwind Labs.");
        let g = Given::before(&m, 4);
        for answer in [fact("", Some("Lakeside Institute")), fact("NW", None)] {
            let a = decide(&answer, OWNER, &g);
            assert_eq!((a.class, a.basis), (Class::Unknown, Basis::TooShort));
        }
    }

    #[test]
    fn an_unreadable_record_places_no_fact() {
        let a = decide(
            &fact("Northwind Labs", Some("Lakeside Institute")),
            OWNER,
            &Given::unreadable(),
        );
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::NoRecord));
        // A correction with no fact at issue needs no record.
        let a = decide(&Answer::NoFact, OWNER, &Given::unreadable());
        assert_eq!(a.class, Class::Behaviour);
    }

    /// Only what the run had read before the correction was given: a result
    /// riding in the correcting message itself — a steer beside tool
    /// results — arrived with the correction and was never acted on.
    #[test]
    fn a_result_arriving_with_the_correction_was_not_given() {
        let m = vec![
            Message::user("Where does Dana Whitfield work now?"),
            Message::assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "kg_entity".into(),
                input: json!({"name": "Dana Whitfield"}),
            }]),
            Message {
                content: vec![
                    Block::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "employer: Northwind Labs".into(),
                        is_error: false,
                    },
                    Block::text("She moved to Lakeside Institute, ignore Northwind Labs."),
                ],
                ..Message::user("")
            },
        ];
        let a = decide(
            &fact("Northwind Labs", Some("Lakeside Institute")),
            "She moved to Lakeside Institute, ignore Northwind Labs.",
            &Given::before(&m, 2),
        );
        assert_eq!(a.class, Class::Gap, "the result came with the steer");
    }

    /// The fixture the row asks for, end to end from a recorded transcript:
    /// the miner's own extraction finds the correction, its position is
    /// what `Given::before` cuts at, and each fixture lands in its class —
    /// the same words from the owner, placed by what the run had read.
    #[test]
    fn fixture_corrections_of_each_class_are_routed_to_their_class() {
        let answer = fact("Northwind Labs", Some("Lakeside Institute"));
        for (read, class) in [
            ("Dana Whitfield — employer: Northwind Labs.", Class::Data),
            (
                "Dana Whitfield — employer: Lakeside Institute.",
                Class::Behaviour,
            ),
            ("Dana Whitfield — no employer on record.", Class::Gap),
        ] {
            let m = run(read);
            let found = crate::learning::extract_interventions(&m);
            let [i] = found.as_slice() else {
                panic!("one correction in the fixture: {found:?}")
            };
            let a = decide(&answer, &i.text, &Given::before(&m, i.at));
            assert_eq!(a.class, class, "{read}");
        }
    }

    #[test]
    fn the_reply_is_read_leniently() {
        let v = |j: serde_json::Value| j;
        assert_eq!(Answer::from_reply(None, None, None), Answer::NotAnswered);
        assert_eq!(
            Answer::from_reply(Some(&v(json!(false))), None, None),
            Answer::NoFact
        );
        assert_eq!(
            Answer::from_reply(Some(&v(json!("false"))), None, None),
            Answer::NoFact
        );
        assert_eq!(
            Answer::from_reply(Some(&v(json!({"x": 1}))), None, None),
            Answer::NotAnswered
        );
        assert_eq!(
            Answer::from_reply(
                Some(&v(json!(true))),
                Some(&v(json!(" Northwind Labs "))),
                Some(&v(json!(""))),
            ),
            fact("Northwind Labs", None),
            "trimmed, and an empty right is a rejection"
        );
    }

    /// A closed enum written to an append-only store is a wire format.
    #[test]
    fn an_unreadable_attribution_loads_as_unknown_never_as_absent() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "de_lenient")]
            a: Option<Attribution>,
        }
        let row: Row = serde_json::from_str(r#"{"a": null}"#).unwrap();
        assert_eq!(row.a, None);
        let row: Row = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(row.a, None);
        let row: Row =
            serde_json::from_str(r#"{"a": {"class": "misread", "basis": "hunch"}}"#).unwrap();
        let a = row.a.unwrap();
        assert_eq!((a.class, a.basis), (Class::Unknown, Basis::Unread));
        let row: Row = serde_json::from_str(r#"{"a": "behaviour"}"#).unwrap();
        assert_eq!(row.a.unwrap().class, Class::Unknown);
    }
}
