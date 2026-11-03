//! A quarantined, typed first draft of the owner's `life.toml`.
//!
//! The model sees only what the persona already sees of herself, the
//! routine-drafter notes, and clean names from her People tab. It answers
//! closed fields; the harness renders TOML and [`super::life::parse`] checks
//! the exact file before it can wait on the owner.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

use super::activity::Activity;
use super::life::{self, Life, Relation, MAX_HOUSEHOLD};
use super::network::{Network, Tie};
use super::{Origin, Persona, Status};

pub const HEADER: &str = "\
# Drafted by a model from what you wrote of them. None of it reached their
# days until you approved it, and any line can be changed before or after.
";

#[derive(Debug, Clone, PartialEq)]
struct ListedPerson {
    name: String,
    tie: Option<&'static str>,
}

fn tie_id(tie: Tie) -> Option<&'static str> {
    match tie {
        Tie::Kin => Some("kin"),
        Tie::Romantic => Some("romantic"),
        Tie::Ex => Some("ex"),
        Tie::Friend => Some("friend"),
        Tie::Colleague => Some("colleague"),
        Tie::Mentor => Some("mentor"),
        Tie::Neighbour => Some("neighbour"),
        Tie::Acquaintance => Some("acquaintance"),
        Tie::Rival => Some("rival"),
        Tie::Unknown => None,
    }
}

/// Only names whose row and whole folded edge are clean can enter a request
/// that may become clean profile text after approval.
fn people(pdir: &Path) -> Result<Vec<ListedPerson>> {
    let network = Network::read(pdir)?;
    Ok(network
        .people
        .iter()
        .filter(|p| p.source.is_clean())
        .filter_map(|p| {
            let edge = network.her_edge(&p.uid);
            (!edge.unclean && edge.unreadable == 0).then(|| ListedPerson {
                name: p.name.clone(),
                tie: edge.tie.and_then(tie_id),
            })
        })
        .collect())
}

fn system() -> String {
    let activities = Activity::ALL
        .iter()
        .filter(|a| **a != Activity::Sleep)
        .map(|a| a.id())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "You draft the life profile of a fictional character from what their author wrote about them. \
         Draft only what the text states or clearly implies. Never invent a household member: \
         `person` must be one of the names in `people`. Someone who lives with them but is not \
         listed is left out; the author must add that person on the People tab first. `alone` is \
         true only when the text supports that they live alone. `never` holds activities the text \
         rules out, such as caring when they have no dependants or work when they have no job. \
         Leave `sleep` null unless the text implies a usual schedule.\n\n\
         Answer with JSON only: {{\"alone\": bool, \"household\": [...], \"sleep\": null, \
         \"never\": [...]}}. Each household entry has `person` and `relation`; relation is one of \
         partner, child, parent, family, housemate. Sleep, when known, is \
         {{\"bed\": \"HH:MM\", \"wake\": \"HH:MM\"}} on a 24-hour clock. `never` may use: \
         {activities}. It never contains sleep. The author's routine notes are direct evidence, \
         but fixed appointments alone do not imply who lives with them."
    )
}

/// The same provenance boundary as the routine drafter, with a refusal that
/// names the life profile the caller asked for.
pub fn check(p: &Persona) -> Result<()> {
    if p.state.status != Status::Approved {
        bail!(
            "`{}` is still a candidate; approve them before drafting their life",
            p.name
        );
    }
    if p.state.origin == Origin::ModelUntrusted {
        bail!(
            "`{}` was written by a run that read untrusted content, so a life profile drafted from them would carry it into their days; write it by hand (`mecha persona life edit {}`)",
            p.name,
            p.name
        );
    }
    Ok(())
}

/// The provider-side constraint. The listed names are the only possible
/// household values; with none, the array is constrained empty.
pub fn schema(names: &[String]) -> serde_json::Value {
    let activities: Vec<&str> = Activity::ALL
        .iter()
        .filter(|a| **a != Activity::Sleep)
        .map(|a| a.id())
        .collect();
    let household = if names.is_empty() {
        json!({
            "type": "array",
            "maxItems": 0,
        })
    } else {
        json!({
            "type": "array",
            "maxItems": MAX_HOUSEHOLD,
            "items": {
                "type": "object",
                "properties": {
                    "person": { "type": "string", "enum": names },
                    "relation": { "type": "string", "enum": Relation::ALL.map(Relation::id) },
                },
                "required": ["person", "relation"],
                "additionalProperties": false,
            },
        })
    };
    json!({
        "type": "object",
        "properties": {
            "alone": { "type": "boolean" },
            "household": household,
            "sleep": {
                "anyOf": [
                    { "type": "null" },
                    {
                        "type": "object",
                        "properties": {
                            "bed": { "type": "string", "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]$" },
                            "wake": { "type": "string", "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]$" },
                        },
                        "required": ["bed", "wake"],
                        "additionalProperties": false,
                    }
                ]
            },
            "never": {
                "type": "array",
                "items": { "type": "string", "enum": activities },
            },
        },
        "required": ["alone", "household", "sleep", "never"],
        "additionalProperties": false,
    })
}

/// One quarantined request over the two persona files, routine notes, and
/// the clean People-tab names and ties—nothing else from the network.
pub fn request(
    model: &str,
    dir: &Path,
    p: &Persona,
    notes: &str,
    structured: bool,
) -> Result<crate::message::CompletionRequest> {
    check(p)?;
    let listed = people(&dir.join(&p.name))?;
    let names = listed.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    let listed_json = listed
        .iter()
        .map(|person| {
            let mut value = json!({ "name": person.name });
            if let Some(tie) = person.tie {
                value["tie"] = json!(tie);
            }
            value
        })
        .collect::<Vec<_>>();
    let mut user = json!({
        "name": p.display(),
        "identity": p.identity,
        "motivation": p.motivation,
        "people": listed_json,
    });
    if !notes.trim().is_empty() {
        user["notes"] = json!(notes.trim());
    }
    Ok(
        crate::quarantine::QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
            .system(system())
            .response_schema(structured.then(|| schema(&names)))
            .ask(user.to_string()),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    alone: bool,
    household: Vec<DraftedHousehold>,
    sleep: Option<DraftedSleep>,
    never: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftedHousehold {
    person: String,
    relation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftedSleep {
    bed: String,
    wake: String,
}

/// Render a typed answer into TOML and check it through the sampler's parser.
/// A name outside `names` is refused even when a provider ignored the schema.
pub fn render(reply: &str, names: &[String]) -> Result<String> {
    let Some(body) = crate::eval::extract_json(reply) else {
        bail!("the drafter answered no JSON");
    };
    let r: Reply =
        serde_json::from_str(&body).context("the drafter's answer is not a life profile")?;
    for h in &r.household {
        if !names.iter().any(|name| name == &h.person) {
            bail!(
                "the drafted household names {:?}, who is not on their clean People tab; add them there first",
                h.person
            );
        }
        if Relation::from_id(&h.relation).is_none() {
            bail!(
                "the drafted household has unknown relation {:?}",
                h.relation
            );
        }
    }
    for id in &r.never {
        let Some(activity) = Activity::from_id(id) else {
            bail!("the draft names unknown activity {id:?}");
        };
        if activity == Activity::Sleep {
            bail!("the draft puts sleep in `never`; a sampled day must be able to sleep");
        }
    }

    let mut doc = toml_edit::DocumentMut::new();
    doc["alone"] = toml_edit::value(r.alone);
    let mut never = toml_edit::Array::new();
    for id in r.never {
        never.push(id);
    }
    doc["never"] = toml_edit::value(never);
    if !r.household.is_empty() {
        let mut list = toml_edit::ArrayOfTables::new();
        for h in r.household {
            let mut table = toml_edit::Table::new();
            table["person"] = toml_edit::value(h.person);
            table["relation"] = toml_edit::value(h.relation);
            list.push(table);
        }
        doc["household"] = toml_edit::Item::ArrayOfTables(list);
    }
    if let Some(s) = r.sleep {
        let mut table = toml_edit::Table::new();
        table["bed"] = toml_edit::value(s.bed);
        table["wake"] = toml_edit::value(s.wake);
        doc["sleep"] = toml_edit::Item::Table(table);
    }
    let text = format!("{HEADER}\n{doc}");
    life::parse(&text).context("the drafted life profile does not load")?;
    Ok(text)
}

pub fn stage(dir: &Path, p: &Persona, reply: &str) -> Result<Life> {
    check(p)?;
    let names = people(&dir.join(&p.name))?
        .into_iter()
        .map(|person| person.name)
        .collect::<Vec<_>>();
    let text = render(reply, &names)?;
    life::stage_draft(dir, &p.name, &text, Origin::ModelClean)?;
    life::parse(&text)
}

pub async fn draft(
    provider: &dyn crate::provider::Provider,
    model: &str,
    dir: &Path,
    p: &Persona,
) -> Result<Life> {
    use crate::message::StopReason;
    check(p)?;
    life::may_stage(dir, &p.name)?;
    let notes = super::routine::read_notes(dir, &p.name)?;
    let request = request(model, dir, p, &notes, provider.structured_output())?;
    let response = provider.complete(&request, None).await?;
    match response.stop_reason {
        StopReason::Refusal => bail!("the drafter's model refused to draft their life"),
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
    use crate::persona::network::{add_person, Layer, NewPerson, When};
    use crate::persona::tests_support::*;
    use crate::persona::Store;

    fn mara() -> (std::path::PathBuf, Persona) {
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        add_person(
            &dir.join("mara"),
            &no_lib(),
            &NewPerson {
                name: "Theo".into(),
                aliases: vec!["T".into()],
                role: Some("old friend".into()),
                description: Some("Keeps a garden.".into()),
                character: None,
                tie: Some(Tie::Friend),
                label: None,
                layer: Some(Layer::Five),
                since: When::UNKNOWN,
            },
        )
        .unwrap();
        let p = Store::load(&dir).get("mara").unwrap().clone();
        (dir, p)
    }

    #[test]
    fn the_request_carries_notes_and_clean_people_names_only() {
        let (dir, p) = mara();
        let notes = "No paid job. Lights out around eleven.";
        let r = request("local", &dir, &p, notes, true).unwrap();
        assert!(r.tools.is_empty());
        assert_eq!(r.messages.len(), 1);
        let user: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(user["identity"], p.identity);
        assert_eq!(user["motivation"], p.motivation);
        assert_eq!(user["notes"], notes);
        assert_eq!(user["people"], json!([{"name": "Theo", "tie": "friend"}]));
        assert!(user["people"][0].get("aliases").is_none());
        assert!(user["people"][0].get("role").is_none());
        assert!(user["people"][0].get("description").is_none());
        assert!(user["people"][0].get("uid").is_none());
        assert_eq!(r.response_schema, Some(schema(&["Theo".into()])));
    }

    #[test]
    fn render_refuses_values_outside_every_closed_set() {
        let names = vec!["Theo".to_string()];
        for (reply, want) in [
            (
                r#"{"alone":false,"household":[{"person":"Pell","relation":"family"}],"sleep":null,"never":[]}"#,
                "People tab",
            ),
            (
                r#"{"alone":false,"household":[{"person":"Theo","relation":"neighbour"}],"sleep":null,"never":[]}"#,
                "unknown relation",
            ),
            (
                r#"{"alone":true,"household":[],"sleep":null,"never":["juggling"]}"#,
                "unknown activity",
            ),
            (
                r#"{"alone":true,"household":[],"sleep":null,"never":["sleep"]}"#,
                "sleep in `never`",
            ),
        ] {
            let got = format!("{:#}", render(reply, &names).unwrap_err());
            assert!(got.contains(want), "{reply}: {got}");
        }
    }

    #[test]
    fn a_valid_answer_is_rendered_by_the_harness_and_loads() {
        let answer = r#"{"alone":false,"household":[{"person":"Theo","relation":"housemate"}],"sleep":{"bed":"23:00","wake":"07:00"},"never":["work","caring"]}"#;
        let text = render(answer, &["Theo".into()]).unwrap();
        assert!(text.starts_with(HEADER));
        let profile = life::parse(&text).unwrap();
        assert_eq!(profile.household[0].person, "Theo");
        assert_eq!(profile.household[0].relation, Relation::Housemate);
        assert_eq!(profile.never, vec![Activity::Work, Activity::Caring]);
    }

    #[test]
    fn only_an_approved_clean_persona_is_drafted_for() {
        let (dir, mut p) = mara();
        p.state.origin = Origin::ModelUntrusted;
        assert!(
            format!("{:#}", request("local", &dir, &p, "", true).unwrap_err())
                .contains("untrusted")
        );
        p.state.origin = Origin::Owner;
        p.state.status = super::super::Status::Candidate;
        assert!(
            format!("{:#}", request("local", &dir, &p, "", true).unwrap_err())
                .contains("candidate")
        );
        assert!(life::read_draft(&dir, "mara").unwrap().is_none());
    }

    #[test]
    fn no_people_means_the_schema_requires_an_empty_household() {
        let s = schema(&[]);
        let household = &s["properties"]["household"];
        assert_eq!(household["maxItems"], 0);
        assert!(household.get("items").is_none(), "{household}");
        assert!(!s.to_string().contains("\"enum\":[]"), "{s}");
    }
}
