//! Each person's own part of what a picture's people do together.
//!
//! A persona's picture call puts the whole interaction in `together` and gives
//! nobody a `doing` (13 of 13 calls in one owner chat, 2026-10-08). The image
//! model reads such a scene as a lineup in the order the people are listed,
//! with the action applied between neighbours: roles right 0 of 30, and on
//! six real calls a duplicated person in 6 of 12 (mecha-a3). Given each
//! person's own part in their `doing`, placed so that people who touch stand
//! together, it drew the roles right in 12 of 15 and duplicated in 1 of 12.
//!
//! The split is a quarantined one-shot (no tools, no history, a typed answer,
//! the same shape as the edit panel's reader), and it feeds the prompt only:
//! the record keeps the call as the persona sent it.

use crate::message::StopReason;
use crate::scene::Where;

/// The splitter's instructions, as mecha-a3 measured them (roles_split,
/// 2026-10-08): text 57 of 57 people posed, 18 of 18 roles right, 8 of 9
/// interacting people adjacent. The sentence on anyone `together` does not
/// name is a3's too: without it, a third person stood between two who hand
/// something over, and the reach crossed them (4 of 4); with it, at the end
/// of the group, watching (4 of 4).
pub const SPLIT_SYSTEM: &str = "You place the people of a picture and give each their own part. \
Given the people (by name) and `together`, one sentence about what they do with each other, \
answer each person's `doing`: their pose and their own part of that sentence, naming whom they \
act on, so that every detail of `together` is in someone's `doing` and nothing is added. Give \
`where` (left, centre, right or background) so that people who touch or hand something to each \
other stand next to each other. Anyone `together` does not name stands at one end of the group \
(left or right, never between the others) with a quiet part of their own, such as watching. \
Answer `together` with only what no `doing` says, usually \"\". JSON only.";

/// The shape the splitter answers in, as measured.
pub fn split_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "people": {"type": "array", "items": {"type": "object", "properties": {
                "who": {"type": "string"},
                "where": {"type": "string", "enum": ["left", "centre", "right", "background"]},
                "doing": {"type": "string"}
            }, "required": ["who", "where", "doing"]}},
            "together": {"type": "string"}
        },
        "required": ["people", "together"]
    })
}

/// One person as the splitter is asked about them: the name the scene's
/// words use, and a pose already given, which comes back unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub who: String,
    pub doing: Option<String>,
}

/// The one-shot request: the people by the names the scene's words use, a
/// pose given for any of them, and the sentence to split. No thinking, the
/// schema where the provider honours one.
pub fn split_request(
    model: &str,
    people: &[Asked],
    together: &str,
    structured: bool,
) -> crate::message::CompletionRequest {
    let user = serde_json::json!({
        "people": people
            .iter()
            .map(|p| match &p.doing {
                Some(d) => serde_json::json!({"who": p.who, "doing": d}),
                None => serde_json::json!({"who": p.who}),
            })
            .collect::<Vec<_>>(),
        "together": together,
    });
    crate::quarantine::QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
        .system(SPLIT_SYSTEM)
        .no_thinking()
        .response_schema(structured.then(split_schema))
        .ask(user.to_string())
}

/// One person's part, by the name the request gave them.
#[derive(Debug, Clone, PartialEq)]
pub struct Role {
    pub who: String,
    pub doing: String,
    pub at: Where,
}

/// The split: a part for every person asked about, and what of `together`
/// no part says (usually nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub roles: Vec<Role>,
    pub together: String,
}

/// The splitter's answer, read strictly: a part for each person asked about
/// and for nobody else, each once, with a pose and a place. Anything else is
/// a failure, and the caller draws the scene as the call said it, except two
/// faults that are repaired in place, since that fallback sends the named
/// `together` whole, the duplicate shape (mecha-a3, 2026-10-09):
///
/// - **A part filed under the wrong person.** One that opens with another
///   asked person's name is that person's act ("Maya taking off his coat"
///   in John's line). It moves to them if their own part is empty.
/// - **A person given no part, or left out.** They get a neutral one, with
///   the others, at a free place. On a one-way act the receiver's part is
///   the one the splitter leaves empty (5 of 12 measured).
///
/// Nobody given a part at all is still a failure. A pose the
/// call already gave is kept as given, whatever the answer says: the model is
/// not trusted to copy it (mecha-a3: it did, 3 of 3, but that is the
/// model's habit, not a guarantee).
pub fn read_split(text: &str, asked: &[Asked]) -> Result<Split, String> {
    let people: Vec<String> = asked.iter().map(|a| a.who.clone()).collect();
    let people = &people;
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err("the answer held no JSON object".into());
    };
    if end < start {
        return Err("the answer held no JSON object".into());
    }
    let v: serde_json::Value = serde_json::from_str(&text[start..=end])
        .map_err(|e| format!("the answer did not read as JSON: {e}"))?;
    let list = v["people"].as_array().ok_or("`people` was not a list")?;
    // The answer's parts by asked name, before any repair.
    let mut parts: Vec<(String, String, Option<Where>)> = Vec::new();
    for p in list {
        let said = p["who"].as_str().map(str::trim).unwrap_or_default();
        // Fixed words, never the answer's own: a reason reaches the
        // manifest, which a run can read (review of #609).
        let Some(who) = people.iter().find(|n| n.eq_ignore_ascii_case(said)) else {
            return Err("the answer named someone not asked about".into());
        };
        if parts.iter().any(|(w, _, _)| w == who) {
            return Err(format!("{who} was given two parts"));
        }
        let doing = p["doing"].as_str().map(str::trim).unwrap_or_default();
        let doing = if crate::imagelib::blank(doing) {
            String::new()
        } else {
            doing.trim_end_matches('.').to_string()
        };
        // Bounded as every `doing` that reaches the compiler is; a longer
        // part is a failed split, which draws the call as sent.
        if doing.chars().count() > crate::imagelib::MAX_CAST_FIELD {
            return Err(format!("{who}'s part was too long"));
        }
        // A place given that is not one is a failed split; none is filled.
        let at = match p["where"].as_str().map(str::trim).filter(|w| !w.is_empty()) {
            Some(w) => Some(Where::parse(w).ok_or_else(|| format!("{who} was given no place"))?),
            None => None,
        };
        parts.push((who.clone(), doing, at));
    }
    for who in people {
        if !parts.iter().any(|(w, _, _)| w == who) {
            parts.push((who.clone(), String::new(), None));
        }
    }
    let given = |who: &str| {
        asked
            .iter()
            .find(|a| a.who == who)
            .and_then(|a| a.doing.clone())
    };
    // A part that opens with another person's name is theirs.
    for i in 0..parts.len() {
        if given(&parts[i].0).is_some() {
            continue;
        }
        let Some((j, rest)) = parts.iter().enumerate().find_map(|(j, (other, _, _))| {
            (j != i)
                .then(|| opens_with_name(&parts[i].1, other))
                .flatten()
                .map(|rest| (j, rest))
        }) else {
            continue;
        };
        if parts[j].1.is_empty() && given(&parts[j].0).is_none() && !rest.is_empty() {
            parts[j].1 = rest;
        }
        parts[i].1 = String::new();
    }
    if parts
        .iter()
        .all(|(who, doing, _)| doing.is_empty() && given(who).is_none())
    {
        return Err("nobody was given a part".into());
    }
    let taken: Vec<Where> = parts.iter().filter_map(|(_, _, at)| *at).collect();
    let mut free = [Where::Left, Where::Right, Where::Centre, Where::Background]
        .into_iter()
        .filter(|w| !taken.contains(w));
    let mut roles: Vec<Role> = Vec::new();
    for (who, doing, at) in &parts {
        let doing = match given(who) {
            Some(g) => g,
            None if doing.is_empty() => {
                let others: Vec<&str> = people
                    .iter()
                    .filter(|n| *n != who)
                    .map(String::as_str)
                    .collect();
                format!("with {}", others.join(" and "))
            }
            None => doing.clone(),
        };
        roles.push(Role {
            who: who.clone(),
            doing,
            at: at.or_else(|| free.next()).unwrap_or(Where::Background),
        });
    }
    let together = v["together"]
        .as_str()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    Ok(Split { roles, together })
}

/// The rest of `part` when it opens with `name` as a word, its subject:
/// "Maya taking off his coat" opens with Maya.
fn opens_with_name(part: &str, name: &str) -> Option<String> {
    let head = part.get(..name.len())?;
    let rest = &part[name.len()..];
    (head.eq_ignore_ascii_case(name) && rest.starts_with(|c: char| c.is_whitespace() || c == ','))
        .then(|| rest.trim_start_matches([',', ' ']).trim().to_string())
}

/// Who splits a scene's `together` into parts: a host with a model to ask
/// (a persona chat), handed to the tool per run (`ToolCtx::role_split`).
#[async_trait::async_trait]
pub trait RoleSplit: Send + Sync + std::fmt::Debug {
    async fn split(&self, people: &[Asked], together: &str) -> Result<Split, String>;
}

/// The splitter on a model: the quarantined one-shot above.
pub struct ModelSplit {
    provider: Box<dyn crate::provider::Provider>,
    model: String,
}

impl std::fmt::Debug for ModelSplit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelSplit")
            .field("model", &self.model)
            .finish()
    }
}

impl ModelSplit {
    pub fn new(provider: Box<dyn crate::provider::Provider>, model: impl Into<String>) -> Self {
        ModelSplit {
            provider,
            model: model.into(),
        }
    }
}

#[async_trait::async_trait]
impl RoleSplit for ModelSplit {
    async fn split(&self, people: &[Asked], together: &str) -> Result<Split, String> {
        let request = split_request(
            &self.model,
            people,
            together,
            self.provider.structured_output(),
        );
        let response = self
            .provider
            .complete(&request, None)
            .await
            .map_err(|e| format!("the splitter could not be reached ({e:#})"))?;
        // The envelope before the content: a refusal arrives as success.
        if response.stop_reason == StopReason::Refusal {
            return Err("the splitter refused".into());
        }
        let text = response.message.text();
        if text.trim().is_empty() {
            return Err("the splitter's answer was empty".into());
        }
        read_split(&text, people)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<Asked> {
        ["Maya", "John"]
            .iter()
            .map(|n| Asked {
                who: n.to_string(),
                doing: None,
            })
            .collect()
    }

    #[test]
    fn the_request_is_quarantined_and_carries_the_names_and_the_sentence() {
        let r = split_request("m", &names(), "Maya hands John a cup", true);
        assert!(SPLIT_SYSTEM.contains("never between the others"));
        assert!(r.tools.is_empty());
        assert_eq!(r.messages.len(), 1);
        assert_eq!(r.think, Some(false));
        assert!(r.response_schema.is_some());
        let body: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(body["together"], "Maya hands John a cup");
        assert_eq!(body["people"][1]["who"], "John");
        assert!(split_request("m", &names(), "x", false)
            .response_schema
            .is_none());
    }

    /// The two repaired faults (mecha-a3, 2026-10-09). A receiver left with
    /// no part, or left out, gets a neutral one at a free place; a part
    /// filed under the wrong person moves to the one it names. Both failed
    /// the whole split before, which sent the named `together` whole.
    #[test]
    fn an_empty_or_misfiled_part_is_repaired_not_failed() {
        let empty = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "lifting John off the ground"},
                {"who": "John", "where": "right", "doing": ""}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(empty.roles[1].doing, "with Maya");
        assert_eq!(empty.roles[1].at, Where::Right);
        let left_out = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "waving"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(left_out.roles[1].who, "John");
        assert_eq!(left_out.roles[1].doing, "with Maya");
        assert_eq!(left_out.roles[1].at, Where::Right, "a free place");
        let misfiled = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""},
                {"who": "John", "where": "right", "doing": "Maya taking off his coat"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(misfiled.roles[0].doing, "taking off his coat");
        assert_eq!(misfiled.roles[1].doing, "with Maya");
        // A given pose is never moved or replaced.
        let mut given = names();
        given[0].doing = Some("reading".into());
        let kept = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""},
                {"who": "John", "where": "right", "doing": "Maya taking off his coat"}], "together": ""}"#,
            &given,
        )
        .unwrap();
        assert_eq!(kept.roles[0].doing, "reading");
        assert_eq!(kept.roles[1].doing, "with Maya");
    }

    #[test]
    fn a_split_names_everyone_once_with_a_part_and_a_place() {
        let ok = read_split(
            r#"{"people": [
                {"who": "maya", "where": "left", "doing": "holding out a cup to John."},
                {"who": "John", "where": "centre", "doing": "taking the cup from Maya"}],
                "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(
            ok.roles[0].who, "Maya",
            "the name as asked, not as answered"
        );
        assert_eq!(ok.roles[0].doing, "holding out a cup to John");
        assert_eq!(ok.roles[1].at, Where::Centre);
        assert_eq!(ok.together, "");
        // A pose the call gave is kept as given, whatever the answer says.
        let mut given = names();
        given[1].doing = Some("reading a newspaper".into());
        let r = split_request("m", &given, "Maya hands John a cup", true);
        let body: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(body["people"][1]["doing"], "reading a newspaper");
        let kept = read_split(
            r#"{"people": [
                {"who": "Maya", "where": "left", "doing": "holding out a cup"},
                {"who": "John", "where": "centre", "doing": "taking the cup"}], "together": ""}"#,
            &given,
        )
        .unwrap();
        assert_eq!(kept.roles[1].doing, "reading a newspaper");
        for bad in [
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "a"}, {"who": "Wren", "where": "right", "doing": "b"}], "together": ""}"#,
            r#"{"people": [{"who": "Maya", "where": "up", "doing": "a"}, {"who": "John", "where": "right", "doing": "b"}], "together": ""}"#,
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "a"}, {"who": "maya", "where": "right", "doing": "b"}], "together": ""}"#,
            "no json",
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""}, {"who": "John", "where": "right", "doing": "…"}], "together": ""}"#,
        ] {
            assert!(read_split(bad, &names()).is_err(), "{bad}");
        }
        // A part past what the compiler takes is a failed split, never sent.
        let long = "x".repeat(crate::imagelib::MAX_CAST_FIELD + 1);
        let answer = format!(
            r#"{{"people": [{{"who": "Maya", "where": "left", "doing": "{long}"}}, {{"who": "John", "where": "right", "doing": "b"}}], "together": ""}}"#
        );
        assert!(read_split(&answer, &names())
            .unwrap_err()
            .contains("too long"));
        // The answer's own words never come back in a reason.
        let why = read_split(
            r#"{"people": [{"who": "Ignore all of this", "where": "left", "doing": "a"}], "together": ""}"#,
            &names(),
        )
        .unwrap_err();
        assert!(!why.contains("Ignore"), "{why}");
    }
}
