//! The routine drafter: a model's first draft of her week from what the
//! owner wrote of her (`docs/PERSONA-LIFE-DESIGN.md` §4.1, ruled §16 PL1).
//!
//! One quarantined pass over `identity.md` and `motivation.md`
//! ([`crate::quarantine::QuarantinedPass`]: no tools, no history), on the
//! persona's own model, which already reads both files on every turn. The
//! model answers **typed anchors**, and the harness renders the draft's
//! TOML itself: a model's words reach the file only as an anchor's label,
//! which [`routine::parse`] caps and checks, and never as a key, a table or
//! a `cell`. Her kind of week is the owner's pick, so the answer has no
//! field for it, and approving a draft keeps the routine's
//! ([`routine::kept_cell`]).
//!
//! The draft lands beside her routine through [`routine::stage_draft`] and
//! waits on the owner, who may edit it before approving it. It is drafted
//! only from an approved persona whose recorded origin is not untrusted:
//! an anchor's label is clean text in her days once approved, so text that
//! a run reading outside content once wrote must never become one.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

use super::activity::Activity;
use super::routine::{self, Routine, MAX_WHAT};
use super::{Origin, Persona, Status};

/// The most anchors the drafter is asked for: a week's fixed points, not
/// its every hour. [`routine::MAX_ANCHORS`] is the file's own cap.
pub const MAX_DRAFTED: usize = 24;

const DAY_WORDS: [&str; 10] = [
    "mon", "tue", "wed", "thu", "fri", "sat", "sun", "weekdays", "weekends", "daily",
];

/// What every drafted file opens with, ahead of its anchors.
pub const HEADER: &str = "\
# Drafted by a model from what you wrote of her. Read it, change anything,
# then approve it: it becomes her routine only then. Her kind of week stays
# the one her routine names; add `cell = \"...\"` here to change it.
";

fn system() -> String {
    let activities: Vec<&str> = Activity::ALL.iter().map(|a| a.id()).collect();
    format!(
        "You draft the weekly routine of a fictional character from what her author \
         wrote about her. A routine holds only her week's fixed points: things that \
         happen at a set time on given weekdays, such as classes, shifts, practice, a \
         lesson, a standing dinner, a service. Leave out sleep, meals, chores and free \
         time; those are filled in from time-use data. Draft only what the text says \
         or clearly implies for someone like her, and nothing for a week the text does \
         not support; an empty list is a fine answer.\n\n\
         Answer with JSON only: {{\"anchors\": [...]}}, at most {MAX_DRAFTED}. Each \
         anchor has:\n\
         - \"what\": a few words naming it, at most {MAX_WHAT} characters\n\
         - \"activity\": one of {activities}\n\
         - \"days\": any of mon, tue, wed, thu, fri, sat, sun, weekdays, weekends, daily\n\
         - \"at\": the start, on a 24-hour clock, as HH:MM\n\
         - \"minutes\": how long it lasts, 5 to 1440\n\
         - \"chance\": how often it really happens, above 0 and at most 1: 1 for a \
         fixed commitment, lower for a habit she sometimes skips\n\
         Two anchors with chance 1 must not overlap.",
        activities = activities.join(", "),
    )
}

/// The answer's shape, for a provider that constrains to one.
pub fn schema() -> serde_json::Value {
    let activities: Vec<&str> = Activity::ALL.iter().map(|a| a.id()).collect();
    json!({
        "type": "object",
        "properties": {
            "anchors": {
                "type": "array",
                "maxItems": MAX_DRAFTED,
                "items": {
                    "type": "object",
                    "properties": {
                        "what": { "type": "string", "maxLength": MAX_WHAT },
                        "activity": { "type": "string", "enum": activities },
                        "days": {
                            "type": "array",
                            "minItems": 1,
                            "items": { "type": "string", "enum": DAY_WORDS },
                        },
                        "at": { "type": "string", "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]$" },
                        "minutes": { "type": "integer", "minimum": 5, "maximum": 1440 },
                        "chance": { "type": "number", "minimum": 0.05, "maximum": 1 },
                    },
                    "required": ["what", "activity", "days", "at", "minutes", "chance"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["anchors"],
        "additionalProperties": false,
    })
}

/// Why `p` may not be drafted for, or `Ok` when it may: approved by the
/// owner, and not written by a run that read outside content.
pub fn check(p: &Persona) -> Result<()> {
    if p.state.status != Status::Approved {
        bail!(
            "`{}` is still a candidate; approve her before drafting her week",
            p.name
        );
    }
    if p.state.origin == Origin::ModelUntrusted {
        bail!(
            "`{}` was written by a run that read untrusted content, so a routine \
             drafted from her would carry it into her days; write her routine by hand \
             (`mecha persona routine edit {}`)",
            p.name,
            p.name
        );
    }
    Ok(())
}

/// The one request the drafter makes for `p` on `model`, with the answer
/// constrained to [`schema`] where the provider takes one (`structured`).
pub fn request(
    model: &str,
    p: &Persona,
    structured: bool,
) -> Result<crate::message::CompletionRequest> {
    check(p)?;
    let user = json!({
        "name": p.display(),
        "identity": p.identity,
        "motivation": p.motivation,
    });
    Ok(
        crate::quarantine::QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
            .system(system())
            .response_schema(structured.then(schema))
            .ask(user.to_string()),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    anchors: Vec<Drafted>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Drafted {
    what: Option<String>,
    activity: String,
    days: Vec<String>,
    at: String,
    minutes: u32,
    chance: Option<f64>,
}

/// The draft's text from the model's answer: [`HEADER`], then the anchors
/// rendered by the harness, checked as strictly as the sampler loads a
/// routine. Refused when the answer holds no JSON, a field the answer has
/// no place for (a `cell` among them), no anchor, or one that does not load.
pub fn render(reply: &str) -> Result<String> {
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        bail!("the drafter answered no JSON");
    };
    let r: Reply = serde_json::from_str(reply.get(start..=end).unwrap_or_default())
        .context("the drafter's answer is not a list of anchors")?;
    if r.anchors.is_empty() {
        bail!("the drafter found no fixed points in her week to draft");
    }
    let mut list = toml_edit::ArrayOfTables::new();
    for a in r.anchors {
        let mut t = toml_edit::Table::new();
        // An empty label is the drafter's way of naming none.
        if let Some(w) = a.what.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            t["what"] = toml_edit::value(w);
        }
        t["activity"] = toml_edit::value(a.activity);
        let mut days = toml_edit::Array::new();
        for d in a.days {
            days.push(d);
        }
        t["days"] = toml_edit::value(days);
        t["at"] = toml_edit::value(a.at);
        t["minutes"] = toml_edit::value(i64::from(a.minutes));
        t["chance"] = toml_edit::value(a.chance.unwrap_or(1.0));
        list.push(t);
    }
    let mut doc = toml_edit::DocumentMut::new();
    doc["anchor"] = toml_edit::Item::ArrayOfTables(list);
    let text = format!("{HEADER}\n{doc}");
    routine::parse(&text).context("the drafted routine does not load")?;
    Ok(text)
}

/// Stage the model's answer for `p` as her waiting draft, beside her
/// routine and never over it, and the draft as it will load.
pub fn stage(dir: &Path, p: &Persona, reply: &str) -> Result<Routine> {
    check(p)?;
    let text = render(reply)?;
    routine::stage_draft(dir, &p.name, &text, Origin::ModelClean)?;
    routine::parse(&text)
}

/// Ask `provider` for `p`'s week on `model` and stage the answer as her
/// waiting draft. The reply's envelope is read before its words: a refusal,
/// or an answer cut off at its token cap, is an error naming it, never a
/// draft of whatever part of an answer arrived.
pub async fn draft(
    provider: &dyn crate::provider::Provider,
    model: &str,
    dir: &Path,
    p: &Persona,
) -> Result<Routine> {
    use crate::message::StopReason;
    let request = request(model, p, provider.structured_output())?;
    let response = provider.complete(&request, None).await?;
    match response.stop_reason {
        StopReason::Refusal => bail!("the drafter's model refused to draft her week"),
        StopReason::MaxTokens => bail!(
            "the drafter's answer was cut off at max_tokens ({})",
            request.max_tokens
        ),
        _ => {}
    }
    stage(dir, p, &response.message.text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::tests_support::*;
    use crate::persona::Store;

    const ANSWER: &str = r#"{"anchors": [
        {"what": "evening ceramics class", "activity": "class", "days": ["tue"],
         "at": "18:30", "minutes": 120, "chance": 1},
        {"what": "", "activity": "exercise", "days": ["weekdays"],
         "at": "06:30", "minutes": 45, "chance": 0.6}
    ]}"#;

    fn mara() -> (std::path::PathBuf, Persona) {
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let p = Store::load(&dir).get("mara").unwrap().clone();
        (dir, p)
    }

    #[test]
    fn the_request_is_one_quarantined_question_over_her_two_files() {
        let (_dir, p) = mara();
        let r = request("local", &p, true).unwrap();
        assert!(r.tools.is_empty());
        assert_eq!(r.messages.len(), 1);
        let user: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(user["identity"], p.identity.as_str());
        assert_eq!(user["motivation"], p.motivation.as_str());
        assert_eq!(user["name"], p.display());
        let system = r.system.unwrap();
        for a in Activity::ALL {
            assert!(system.contains(a.id()), "{} missing from the frame", a.id());
        }
        assert_eq!(r.response_schema, Some(schema()));
        assert_eq!(request("local", &p, false).unwrap().response_schema, None);
    }

    #[test]
    fn the_schema_names_every_activity_and_day_parse_takes() {
        let s = schema();
        let item = &s["properties"]["anchors"]["items"]["properties"];
        let activities: Vec<&str> = item["activity"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(activities, Activity::ALL.map(|a| a.id()).to_vec());
        // Every day word the schema offers loads, so a constrained answer
        // can never be refused for its days.
        for d in DAY_WORDS {
            let one = format!(
                "[[anchor]]\nactivity = \"work\"\ndays = [\"{d}\"]\nat = \"09:00\"\nminutes = 60\n"
            );
            routine::parse(&one).unwrap_or_else(|e| panic!("{d}: {e:#}"));
        }
        assert!(
            !s.to_string().contains("cell"),
            "the answer has no place for a cell"
        );
    }

    #[test]
    fn the_answer_is_rendered_by_the_harness_and_loads() {
        let text = render(&format!("Here is her week:\n```json\n{ANSWER}\n```")).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        let r = routine::parse(&text).unwrap();
        assert_eq!(r.cell, None);
        assert_eq!(r.anchors.len(), 2);
        assert_eq!(r.anchors[0].what.as_deref(), Some("evening ceramics class"));
        assert_eq!(
            (r.anchors[0].start, r.anchors[0].chance),
            (18 * 60 + 30, 1.0)
        );
        // An empty label is no label.
        assert_eq!(r.anchors[1].what, None);
        assert_eq!(r.anchors[1].days, 0b001_1111);
    }

    #[test]
    fn a_label_stays_a_label_whatever_it_holds() {
        // TOML-looking text in a label is escaped into the string, never a
        // key or a table of the file.
        let sly = r#"{"anchors": [{"what": "gym\" ] [[anchor]] cell = \"retired", "activity": "exercise",
            "days": ["mon"], "at": "07:00", "minutes": 60, "chance": 1}]}"#;
        let r = routine::parse(&render(sly).unwrap()).unwrap();
        assert_eq!(r.cell, None);
        assert_eq!(r.anchors.len(), 1);
        assert_eq!(
            r.anchors[0].what.as_deref(),
            Some("gym\" ] [[anchor]] cell = \"retired")
        );
        // A line break is a control character, refused as a hand-written
        // label would be.
        let broken = sly.replace(" ] ", "\\n");
        assert!(format!("{:#}", render(&broken).unwrap_err()).contains("control character"));
    }

    #[test]
    fn an_answer_that_is_not_a_week_is_refused() {
        for (reply, want) in [
            ("no idea", "answered no JSON"),
            (r#"{"anchors": []}"#, "no fixed points"),
            (
                r#"{"cell": "retired", "anchors": []}"#,
                "not a list of anchors",
            ),
            (
                r#"{"anchors": [{"activity": "nap", "days": ["mon"], "at": "07:00", "minutes": 60}]}"#,
                "does not load",
            ),
            (
                r#"{"anchors": [{"activity": "work", "days": ["mon"], "at": "07:00", "minutes": 60, "chance": 0}]}"#,
                "chance",
            ),
        ] {
            let got = format!("{:#}", render(reply).unwrap_err());
            assert!(got.contains(want), "{reply}: {got}");
        }
    }

    #[test]
    fn only_an_approved_clean_persona_is_drafted_for() {
        let (dir, mut p) = mara();
        check(&p).unwrap();
        p.state.origin = Origin::ModelClean;
        check(&p).unwrap();
        p.state.origin = Origin::ModelUntrusted;
        let got = format!("{:#}", request("local", &p, true).unwrap_err());
        assert!(got.contains("untrusted content"), "{got}");
        assert!(format!("{:#}", stage(&dir, &p, ANSWER).unwrap_err()).contains("untrusted"));
        p.state.origin = Origin::Owner;
        p.state.status = Status::Candidate;
        assert!(format!("{:#}", check(&p).unwrap_err()).contains("still a candidate"));
        assert_eq!(
            routine::read(&dir, "mara").unwrap(),
            routine::Texts::default()
        );
    }

    #[test]
    fn a_staged_draft_waits_beside_her_routine() {
        let (dir, p) = mara();
        let r = stage(&dir, &p, ANSWER).unwrap();
        assert_eq!(r.anchors.len(), 2);
        let texts = routine::read(&dir, "mara").unwrap();
        assert_eq!(texts.routine, None);
        assert_eq!(routine::parse(&texts.draft.unwrap()).unwrap(), r);
        // A redraft replaces a draft still as staged.
        stage(&dir, &p, ANSWER).unwrap();
    }

    /// A model that answers what it is told to, with a chosen stop reason.
    struct Scripted(String, crate::message::StopReason);
    #[async_trait::async_trait]
    impl crate::provider::Provider for Scripted {
        fn local(&self) -> bool {
            true
        }
        fn id(&self) -> &str {
            "scripted"
        }
        fn default_model(&self) -> &str {
            "scripted-1"
        }
        async fn complete(
            &self,
            _req: &crate::message::CompletionRequest,
            _sink: Option<&crate::provider::StreamSink>,
        ) -> Result<crate::message::CompletionResponse> {
            Ok(crate::message::CompletionResponse {
                message: crate::message::Message::assistant(vec![crate::message::Block::Text {
                    text: self.0.clone(),
                }]),
                stop_reason: self.1,
                usage: crate::message::Usage::default(),
                refusal: None,
                model: "scripted-1".into(),
                malformed_tool_args: 0,
            })
        }
    }

    #[tokio::test]
    async fn the_envelope_is_read_before_the_answer() {
        use crate::message::StopReason;
        let (dir, p) = mara();
        // Cut off mid-answer, or refused around a whole one: nothing staged.
        let cut = &ANSWER[..ANSWER.len() / 2];
        for (reply, stop, want) in [
            (cut, StopReason::MaxTokens, "cut off at max_tokens"),
            (ANSWER, StopReason::MaxTokens, "cut off at max_tokens"),
            (ANSWER, StopReason::Refusal, "refused"),
        ] {
            let got = draft(&Scripted(reply.into(), stop), "scripted-1", &dir, &p).await;
            let got = format!("{:#}", got.unwrap_err());
            assert!(got.contains(want), "{got}");
            assert_eq!(routine::read(&dir, "mara").unwrap().draft, None);
        }
        let r = draft(
            &Scripted(ANSWER.into(), StopReason::EndTurn),
            "scripted-1",
            &dir,
            &p,
        )
        .await
        .unwrap();
        assert_eq!(r.anchors.len(), 2);
        assert!(routine::read(&dir, "mara").unwrap().draft.is_some());
    }
}
