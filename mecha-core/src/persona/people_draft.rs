//! Her people out of `identity.md` and into her network, once
//! (`docs/PERSONA-LIFE-DESIGN.md` §6.6, ruled PL6 and PL31).
//!
//! One quarantined pass over `identity.md` on her own model ([`request`]):
//! no tools, no history, typed people back, never prose. The answer is
//! staged as `people.draft.json` beside her files, against the digest of
//! the `identity.md` it was read from, and waits on the owner. Accepting it
//! ([`accept`]) adds each chosen person to her network as an owner row,
//! because the owner has read and approved them; anyone already there by
//! name or alias is left alone and said so.
//!
//! **Nothing rewrites the owner's prose** (PL31). After accepting, the
//! owner is shown the `identity.md` lines that still name them
//! ([`lines_naming`]) and edits the file; until then she reads them in both
//! places, which is the owner's to settle.
//!
//! Drafted only from an approved persona whose recorded origin is not
//! untrusted ([`super::drafter::check`]): an accepted person is an owner
//! row, so text a run reading outside content wrote must never become one.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

use super::network::{self, Layer, NewPerson, Tie, When, MAX_DESCRIPTION, MAX_LABEL, MAX_NAME};
use super::Persona;

/// The waiting proposal, beside `identity.md`.
pub const DRAFT: &str = "people.draft.json";
/// The most people one proposal holds: her inner circles, not a census.
pub const MAX_PROPOSED: usize = 40;

/// One person the model found in `identity.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposed {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tie: Option<Tie>,
    #[serde(default)]
    pub label: Option<String>,
    /// 5, 15, 50 or 150.
    #[serde(default)]
    pub layer: Option<u32>,
}

/// A staged proposal: who, and the `identity.md` it was read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// `text_digest` of `identity.md` as it was read.
    pub identity: String,
    pub people: Vec<Proposed>,
}

fn system() -> String {
    let ties: Vec<String> = Tie::ALL
        .iter()
        .map(|t| {
            serde_json::to_value(t)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default()
        })
        .collect();
    format!(
        "You read what an author wrote about a fictional character and list the people \
         it names in her life: family, partners, friends, colleagues, neighbours, anyone \
         the text gives a name or a clear role. List only people the text names or \
         clearly describes; invent no one and add nothing the text does not say. \
         Never list her, and never the reader she talks to.\n\n\
         Answer with JSON only: {{\"people\": [...]}}, at most {MAX_PROPOSED}. Each \
         person has:\n\
         - \"name\": what the text calls them, at most {MAX_NAME} characters; a role \
         (\"her grandmother\") when the text gives no name\n\
         - \"aliases\": other names the text uses for them, or []\n\
         - \"role\": who they are to her in a few words (\"older sister\"), or null\n\
         - \"description\": one short sentence of what the text says about them, \
         starting with a capital letter and ending with a full stop (\"Runs a bakery \
         two towns over.\"), at most {MAX_DESCRIPTION} characters, or null\n\
         - \"tie\": one of {ties}, or null\n\
         - \"label\": the tie in the text's words (\"sister\"), at most {MAX_LABEL} \
         characters, or null\n\
         - \"layer\": how close they are, 5 (her closest few), 15, 50 or 150, or null \
         when the text does not say",
        ties = ties.join(", "),
    )
}

/// The answer's shape, for a provider that constrains to one.
pub fn schema() -> serde_json::Value {
    let ties: Vec<serde_json::Value> = Tie::ALL.iter().map(|t| json!(t)).collect();
    let text = |max: usize| json!({ "type": ["string", "null"], "maxLength": max });
    json!({
        "type": "object",
        "properties": {
            "people": {
                "type": "array",
                "maxItems": MAX_PROPOSED,
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "minLength": 1, "maxLength": MAX_NAME },
                        "aliases": { "type": "array", "items": { "type": "string", "maxLength": MAX_NAME } },
                        "role": text(MAX_NAME),
                        "description": text(MAX_DESCRIPTION),
                        "tie": { "enum": ties.into_iter().chain([serde_json::Value::Null]).collect::<Vec<_>>() },
                        "label": text(MAX_LABEL),
                        "layer": { "enum": [5, 15, 50, 150, null] },
                    },
                    "required": ["name", "aliases", "role", "description", "tie", "label", "layer"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["people"],
        "additionalProperties": false,
    })
}

/// The one request for `p`'s people on `model`.
pub fn request(
    model: &str,
    p: &Persona,
    structured: bool,
) -> Result<crate::message::CompletionRequest> {
    super::drafter::check(p)?;
    let user = json!({ "name": p.display(), "identity": p.identity });
    Ok(
        crate::quarantine::QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
            .system(system())
            .response_schema(structured.then(schema))
            .ask(user.to_string()),
    )
}

impl Proposed {
    /// As the network would take it: every field checked by its door, so a
    /// proposal that stages is one that can be accepted.
    fn as_new(&self) -> Result<NewPerson> {
        let layer = self
            .layer
            .map(|n| Layer::of_size(n).ok_or_else(|| anyhow::anyhow!("{n} is not a layer")))
            .transpose()?;
        if self.tie == Some(Tie::Unknown) {
            bail!("a tie this binary does not know");
        }
        let blank = |s: &Option<String>| s.clone().filter(|s| !s.trim().is_empty());
        Ok(NewPerson {
            name: self.name.clone(),
            aliases: self.aliases.clone(),
            role: blank(&self.role),
            description: blank(&self.description),
            character: None,
            tie: self.tie,
            label: blank(&self.label),
            layer,
            since: When::UNKNOWN,
        })
    }
}

/// The model's answer as a proposal, every person checked as the network's
/// door checks them. `None` when it found no one, a finding with nothing
/// to stage. Refused when it holds no JSON, a field it has no place for,
/// or a person the network would refuse.
pub fn parse(reply: &str) -> Result<Option<Vec<Proposed>>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reply {
        people: Vec<Proposed>,
    }
    let Some(body) = crate::eval::extract_json(reply) else {
        bail!("the reader answered no JSON");
    };
    let r: Reply =
        serde_json::from_str(&body).context("the reader's answer is not a list of people")?;
    if r.people.is_empty() {
        return Ok(None);
    }
    if r.people.len() > MAX_PROPOSED {
        bail!(
            "{} people; a proposal holds at most {MAX_PROPOSED}",
            r.people.len()
        );
    }
    for (i, p) in r.people.iter().enumerate() {
        network::check_new(&p.as_new()?)
            .with_context(|| format!("person {} ({})", i + 1, p.name))?;
    }
    Ok(Some(r.people))
}

/// Stage `p`'s proposal from the model's answer, replacing one still
/// waiting. `None`, with nothing staged, when it found no one.
pub fn stage(dir: &Path, p: &Persona, reply: &str) -> Result<Option<Draft>> {
    super::drafter::check(p)?;
    let Some(people) = parse(reply)? else {
        return Ok(None);
    };
    let draft = Draft {
        identity: super::text_digest(&p.identity),
        people,
    };
    super::write_private(
        &dir.join(&p.name).join(DRAFT),
        serde_json::to_string_pretty(&draft)?.as_bytes(),
    )?;
    Ok(Some(draft))
}

/// Ask `provider` for `p`'s people and stage them. The envelope is read
/// before the answer: a refusal or a reply cut off at its cap stages
/// nothing.
pub async fn propose(
    provider: &dyn crate::provider::Provider,
    model: &str,
    dir: &Path,
    p: &Persona,
) -> Result<Option<Draft>> {
    use crate::message::StopReason;
    let request = request(model, p, provider.structured_output())?;
    let response = provider.complete(&request, None).await?;
    match response.stop_reason {
        StopReason::Refusal => bail!("the reader's model refused to read her people"),
        StopReason::MaxTokens => bail!(
            "the reader's answer was cut off at max_tokens ({})",
            request.max_tokens
        ),
        _ => {}
    }
    stage(dir, p, &response.message.text())
}

/// The waiting proposal, if one is staged. One this binary cannot read is
/// an error naming the file, never an empty proposal.
pub fn read(pdir: &Path) -> Result<Option<Draft>> {
    let path = pdir.join(DRAFT);
    if !path.exists() {
        return Ok(None);
    }
    let text = super::read_prose(&path)?;
    serde_json::from_str(&text)
        .map(Some)
        .with_context(|| format!("{DRAFT} does not load"))
}

/// What accepting did: who was added, and who was already there.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Accepted {
    /// Id and name of each person added.
    pub added: Vec<(String, String)>,
    /// Names already in her network, by name or alias, left alone.
    pub already: Vec<String>,
}

/// Accept the waiting proposal, or only the people at `only` (1-based, as
/// the proposal lists them): each is added to her network as an owner row,
/// because the owner chose them. Someone already in it by name or alias is
/// left alone. Refused when `identity.md` changed since it was read, since
/// the owner would be accepting a reading of a file that is gone. The
/// proposal is removed once accepted.
pub fn accept(
    dir: &Path,
    lib: &crate::imagelib::Library,
    p: &Persona,
    only: Option<&[usize]>,
) -> Result<Accepted> {
    let pdir = dir.join(&p.name);
    let Some(draft) = read(&pdir)? else {
        bail!("`{}` has no proposal of people waiting", p.name);
    };
    if draft.identity != super::text_digest(&p.identity) {
        bail!(
            "`{}`'s identity.md changed since her people were read from it; propose again",
            p.name
        );
    }
    let chosen: Vec<&Proposed> = match only {
        None => draft.people.iter().collect(),
        Some(nums) => {
            let mut out = Vec::new();
            for n in nums {
                let Some(person) = n.checked_sub(1).and_then(|i| draft.people.get(i)) else {
                    bail!(
                        "the proposal has no person {n}; it lists {}",
                        draft.people.len()
                    );
                };
                out.push(person);
            }
            out
        }
    };
    let mut done = Accepted::default();
    for person in chosen {
        let net = network::Network::read(&pdir)?;
        let names = std::iter::once(&person.name).chain(&person.aliases);
        let there = net.people.iter().any(|q| {
            names.clone().any(|n| {
                q.name.eq_ignore_ascii_case(n.trim())
                    || q.aliases.iter().any(|a| a.eq_ignore_ascii_case(n.trim()))
            })
        });
        if there {
            done.already.push(person.name.clone());
            continue;
        }
        let id = network::add_person(&pdir, lib, &person.as_new()?)?;
        done.added.push((id, person.name.trim().to_string()));
    }
    std::fs::remove_file(pdir.join(DRAFT))
        .with_context(|| format!("the people are added, but {DRAFT} could not be removed"))?;
    Ok(done)
}

/// Drop the waiting proposal.
pub fn dismiss(pdir: &Path) -> Result<()> {
    match std::fs::remove_file(pdir.join(DRAFT)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("no proposal of people is waiting")
        }
        Err(e) => Err(e).with_context(|| format!("removing {DRAFT}")),
    }
}

/// The lines of `identity` (1-based, as the file numbers them) naming any
/// of `names` as a whole word, case aside: what the owner edits so each
/// person lives in one place (PL31).
pub fn lines_naming(identity: &str, names: &[&str]) -> Vec<(usize, String)> {
    let words: Vec<String> = names
        .iter()
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .collect();
    let named = |line: &str| {
        let l = line.to_lowercase();
        words.iter().any(|w| {
            l.match_indices(w.as_str()).any(|(i, _)| {
                let before = l[..i].chars().next_back();
                let after = l[i + w.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            })
        })
    };
    identity
        .lines()
        .enumerate()
        .filter(|(_, l)| named(l))
        .map(|(i, l)| (i + 1, l.trim_end().to_string()))
        .collect()
}

/// The lines of `p`'s `identity.md` naming anyone in her network.
pub fn identity_overlap(dir: &Path, p: &Persona) -> Result<Vec<(usize, String)>> {
    let net = network::Network::read(&dir.join(&p.name))?;
    let names: Vec<&str> = net
        .people
        .iter()
        .flat_map(|q| std::iter::once(q.name.as_str()).chain(q.aliases.iter().map(String::as_str)))
        .collect();
    Ok(lines_naming(&p.identity, &names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::tests_support::*;
    use crate::persona::Store;

    const ANSWER: &str = r#"{"people": [
        {"name": "Ines", "aliases": ["Nessa"], "role": "older sister",
         "description": "Runs a bakery two towns over.", "tie": "kin", "label": "sister", "layer": 5},
        {"name": "Theo", "aliases": [], "role": "lab partner", "description": null,
         "tie": "colleague", "label": null, "layer": null}
    ]}"#;

    fn mara() -> (std::path::PathBuf, Persona) {
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let id = dir.join("mara").join("identity.md");
        let text = std::fs::read_to_string(&id).unwrap();
        std::fs::write(
            &id,
            format!("{text}\nHer older sister Ines runs a bakery.\nShe and Theo share a bench in the lab.\nNessa, she calls her.\n"),
        )
        .unwrap();
        let p = Store::load(&dir).get("mara").unwrap().clone();
        (dir, p)
    }

    #[test]
    fn the_request_is_one_quarantined_question_over_identity() {
        let (_dir, p) = mara();
        let r = request("local", &p, true).unwrap();
        assert!(r.tools.is_empty());
        assert_eq!(r.messages.len(), 1);
        let user: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(user["identity"], p.identity.as_str());
        assert!(
            user.get("motivation").is_none(),
            "only identity.md is read for people"
        );
        for t in Tie::ALL {
            let id = serde_json::to_value(t).unwrap();
            assert!(r.system.as_deref().unwrap().contains(id.as_str().unwrap()));
        }
        assert_eq!(r.response_schema, Some(schema()));
    }

    #[test]
    fn a_proposal_is_staged_against_the_identity_it_read_and_accepted_as_owner_rows() {
        let (dir, p) = mara();
        let pdir = dir.join("mara");
        let d = stage(&dir, &p, ANSWER).unwrap().unwrap();
        assert_eq!(d.people.len(), 2);
        assert_eq!(read(&pdir).unwrap().unwrap(), d);
        // Accept only the first: an owner row, and the proposal is gone.
        let got = accept(&dir, &no_lib(), &p, Some(&[1])).unwrap();
        assert_eq!(got.added.len(), 1);
        assert_eq!(got.added[0].1, "Ines");
        let net = network::Network::read(&pdir).unwrap();
        let ines = net.find("Nessa").unwrap();
        assert_eq!(ines.source, crate::persona::world::Source::Owner);
        assert_eq!(net.her_edge(&ines.uid).layer, Some(Layer::Five));
        assert_eq!(read(&pdir).unwrap(), None);
        // Proposed again, someone already there is left alone and said so.
        stage(&dir, &p, ANSWER).unwrap();
        let got = accept(&dir, &no_lib(), &p, None).unwrap();
        assert_eq!(got.already, vec!["Ines"]);
        assert_eq!(
            got.added
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["Theo"]
        );
        assert_eq!(network::Network::read(&pdir).unwrap().people.len(), 2);
        // The lines of identity.md that still name them, numbered as the file is.
        let lines = identity_overlap(&dir, &p).unwrap();
        let texts: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Her older sister Ines runs a bakery.",
                "She and Theo share a bench in the lab.",
                "Nessa, she calls her."
            ]
        );
        let first = p.identity.lines().position(|l| l.contains("Ines")).unwrap() + 1;
        assert_eq!(lines[0].0, first);
    }

    #[test]
    fn a_proposal_read_from_an_identity_since_changed_is_refused() {
        let (dir, p) = mara();
        stage(&dir, &p, ANSWER).unwrap();
        let mut changed = p.clone();
        changed.identity.push_str("A new line.\n");
        let why = format!("{:#}", accept(&dir, &no_lib(), &changed, None).unwrap_err());
        assert!(why.contains("changed since"), "{why}");
        assert!(accept(&dir, &no_lib(), &p, Some(&[3])).is_err());
        assert!(accept(&dir, &no_lib(), &p, Some(&[0])).is_err());
        assert!(network::Network::read(&dir.join("mara"))
            .unwrap()
            .people
            .is_empty());
        dismiss(&dir.join("mara")).unwrap();
        assert!(dismiss(&dir.join("mara")).is_err());
    }

    #[test]
    fn an_answer_is_checked_as_the_network_checks_a_person() {
        assert_eq!(parse(r#"{"people": []}"#).unwrap(), None);
        for (reply, want) in [
            ("nobody", "no JSON"),
            (r#"{"people": [], "cell": 1}"#, "not a list of people"),
            (
                r#"{"people": [{"name": "  ", "aliases": []}]}"#,
                "a name is empty",
            ),
            (r#"{"people": [{"name": "Bo", "layer": 7}]}"#, "not a layer"),
            (
                r#"{"people": [{"name": "Bo", "tie": "cousin"}]}"#,
                "a tie this binary",
            ),
            (
                r#"{"people": [{"name": "Bo", "role": "line\nbreak"}]}"#,
                "control character",
            ),
        ] {
            let got = format!("{:#}", parse(reply).unwrap_err());
            assert!(got.contains(want), "{reply}: {got}");
        }
    }

    #[test]
    fn only_an_approved_clean_persona_is_read_for_people() {
        let (dir, mut p) = mara();
        p.state.origin = crate::persona::Origin::ModelUntrusted;
        assert!(request("local", &p, true).is_err());
        assert!(stage(&dir, &p, ANSWER).is_err());
        assert_eq!(read(&dir.join("mara")).unwrap(), None);
    }

    #[test]
    fn a_name_is_matched_as_a_whole_word() {
        let lines = lines_naming(
            "Ines\nPrinces of the city\nines.\nTheodore\nwith Theo, later",
            &["Ines", "Theo"],
        );
        assert_eq!(
            lines.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![1, 3, 5]
        );
    }
}
