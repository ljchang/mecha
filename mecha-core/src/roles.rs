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
//! the record keeps the call as the persona sent it. The one exception is a
//! picture built in layers (IMAGE-DESIGN.md §15.4): its `layers` entry keeps
//! each person's part from the split, under the `together`'s origin, because
//! a later re-place is built on them.

use crate::message::StopReason;
use crate::scene::Where;

/// The splitter's instructions, as mecha-a3 measured them (roles_split,
/// 2026-10-08): text 57 of 57 people posed, 18 of 18 roles right, 8 of 9
/// interacting people adjacent. The sentence on anyone `together` does not
/// name is a3's too: without it, a third person stood between two who hand
/// something over, and the reach crossed them (4 of 4); with it, at the end
/// of the group, watching (4 of 4). The sentence on naming whom a part acts
/// on is for layers: the placing pass swaps names for image tags, and a
/// pronoun has nothing to swap, so an act on the other person's clothes was
/// drawn on the actor's own, 3 of 3 (mecha-a3, 2026-10-10).
pub const SPLIT_SYSTEM: &str = "You place the people of a picture and give each their own part. \
Given the people (by name) and `together`, one sentence about what they do with each other, \
answer each person's `doing`: their pose and their own part of that sentence, naming whom they \
act on, so that every detail of `together` is in someone's `doing` and nothing is added. Name \
anyone else a `doing` acts on, and anything of theirs, by name (\"Maya's coat\"), never by he, \
she, him, her or his. Give \
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
///   in John's line). It moves to them if their own part is empty, and is
///   dropped from the line it was filed under either way: it was never
///   that person's act, and keeping it is the inversion being repaired.
/// - **A person given no part, or left out.** They get a neutral part that
///   names nobody, at a free place: an end of the group, else the
///   background, never between the others.
/// - **A person with no place.** They get a free place the same way. On a one-way act the receiver's part is
///   the one the splitter leaves empty (5 of 12 measured).
///
/// A part made only of the asked names (and "and") is read as no part
/// before either repair (mecha-a3, 2026-10-09: an answer of only names was
/// applied with no act in the prompt). Nobody given a part at all is still a
/// failure. A pose the
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
        // A part that is only a name says nothing anyone does: read as no
        // part, so an answer of only names fails and the call is drawn as
        // sent (mecha-a3, 2026-10-09: {"doing": "Maya"}, {"doing": "John"}
        // was applied with no act in the prompt).
        let doing = if crate::imagelib::blank(doing) || names_only(doing, people) {
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
            Some(w) => Some(Where::parse(w).ok_or_else(|| format!("{who}'s place was not one"))?),
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
    // A part that opens with another person's name is theirs. Every move is
    // read from the answer as given, then applied: a pair filed under each
    // other (the inversion this repairs) swaps whole, where moving one at a
    // time lost the second person's part (review of #614, pass 3).
    let mut moves: Vec<(usize, usize, String)> = Vec::new();
    for i in 0..parts.len() {
        // A part opening with the person's own name is theirs, whatever
        // other name it also begins with ("Maya Chen smiling" beside a
        // Maya; review of #614).
        if given(&parts[i].0).is_some() || opens_with_name(&parts[i].1, &parts[i].0).is_some() {
            continue;
        }
        // The longest name the part opens with, so "Maya Chen …" goes to
        // Maya Chen, never to a Maya beside her.
        if let Some((j, rest)) = parts
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .filter_map(|(j, (other, _, _))| {
                opens_with_name(&parts[i].1, other).map(|rest| (j, other.len(), rest))
            })
            .max_by_key(|(_, len, _)| *len)
            .map(|(j, _, rest)| (j, rest))
        {
            moves.push((i, j, rest));
        }
    }
    let vacated: Vec<usize> = moves.iter().map(|(i, _, _)| *i).collect();
    let before: Vec<String> = parts.iter().map(|(_, doing, _)| doing.clone()).collect();
    for &i in &vacated {
        parts[i].1 = String::new();
    }
    for (_, j, rest) in moves {
        // To a person whose own line was empty, or was itself filed under
        // someone else; never over a part that was theirs, never twice.
        let free_line = before[j].is_empty() || vacated.contains(&j);
        if free_line
            && parts[j].1.is_empty()
            && given(&parts[j].0).is_none()
            && !crate::imagelib::blank(&rest)
        {
            parts[j].1 = rest;
        }
    }
    // The answer's own parts decide it: a pose the call gave is not the
    // splitter's, and a split with no part from the answer drops the
    // `together` it was asked to divide (review of #614, pass 4).
    if parts.iter().all(|(_, doing, _)| doing.is_empty()) {
        return Err("nobody was given a part".into());
    }
    let taken: Vec<Where> = parts.iter().filter_map(|(_, _, at)| *at).collect();
    // The ends first, then the background: the person given a place here
    // is the one with no part, and the measured rule is that they stand at
    // an end, never between the others (4 of 4; review of #614).
    let mut free = [Where::Left, Where::Right, Where::Background]
        .into_iter()
        .filter(|w| !taken.contains(w));
    let mut roles: Vec<Role> = Vec::new();
    for (who, doing, at) in &parts {
        let doing = match given(who) {
            Some(g) => g,
            // Named by nobody: a described person's part naming a library
            // character is refused by the compiler, and a description is
            // already as long as a part may be; the `together` carries the
            // act (review of #614, pass 5).
            None if doing.is_empty() => {
                if people.len() == 2 {
                    "together with the other person".to_string()
                } else {
                    "together with the others".to_string()
                }
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

/// Whether `part` names people and says nothing else: "Wren", "Maya and
/// John".
fn names_only(part: &str, people: &[String]) -> bool {
    let mut rest = format!(" {} ", part.to_lowercase());
    let mut names: Vec<String> = people.iter().map(|n| n.to_lowercase()).collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for n in &names {
        rest = rest.replace(n.as_str(), " ");
    }
    rest.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .all(|w| w == "and")
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
        assert!(SPLIT_SYSTEM.contains("never by he, she, him, her or his"));
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
        assert_eq!(empty.roles[1].doing, "together with the other person");
        assert_eq!(empty.roles[1].at, Where::Right);
        let left_out = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "waving"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(left_out.roles[1].who, "John");
        assert_eq!(left_out.roles[1].doing, "together with the other person");
        assert_eq!(left_out.roles[1].at, Where::Right, "a free place");
        let misfiled = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""},
                {"who": "John", "where": "right", "doing": "Maya taking off his coat"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(misfiled.roles[0].doing, "taking off his coat");
        assert_eq!(misfiled.roles[1].doing, "together with the other person");
        // A given pose is never moved or replaced.
        let mut given = names();
        given[0].doing = Some("reading".into());
        let kept = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "standing up"},
                {"who": "John", "where": "right", "doing": "taking off his coat"}], "together": ""}"#,
            &given,
        )
        .unwrap();
        assert_eq!(kept.roles[0].doing, "reading");
        assert_eq!(kept.roles[1].doing, "taking off his coat");
        // Its only part misfiled onto a person the call posed, the answer
        // gave nobody a part, and the split fails over to the call as sent.
        assert!(read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""},
                {"who": "John", "where": "right", "doing": "Maya taking off his coat"}], "together": ""}"#,
            &given,
        )
        .is_err());
        // A third person left out stands at an end, never between the two.
        let mut three = names();
        three.push(Asked {
            who: "Wren".into(),
            doing: None,
        });
        let ends = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "handing John a cup"},
                {"who": "John", "where": "right", "doing": "taking the cup"}], "together": ""}"#,
            &three,
        )
        .unwrap();
        assert_eq!(ends.roles[2].at, Where::Background);
        // A named person with no place gets a free one.
        let placeless = read_split(
            r#"{"people": [{"who": "Maya", "doing": "waving"},
                {"who": "John", "where": "left", "doing": "waving back"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(placeless.roles[0].at, Where::Right);
        // A moved part must be a part, by the parse loop's own test.
        assert!(read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""},
                {"who": "John", "where": "right", "doing": "Maya …"}], "together": ""}"#,
            &names(),
        )
        .is_err());
        // A part opening with the person's own name is theirs.
        let both = vec![
            Asked {
                who: "Maya".into(),
                doing: None,
            },
            Asked {
                who: "Maya Chen".into(),
                doing: None,
            },
        ];
        let own = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "laughing"},
                {"who": "Maya Chen", "where": "right", "doing": "Maya Chen smiling"}], "together": ""}"#,
            &both,
        )
        .unwrap();
        assert_eq!(own.roles[1].doing, "Maya Chen smiling");
        // An answer with no part fails even where the call posed someone,
        // so the `together` is drawn whole rather than dropped.
        assert!(read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": ""},
                {"who": "John", "where": "right", "doing": ""}], "together": ""}"#,
            &given,
        )
        .is_err());
        // A misfiled part goes to the longest name it opens with, and a
        // neutral part names nobody, so a described person's never names a
        // library character (review of #614, pass 5).
        let mixed: Vec<Asked> = ["Maya", "Maya Chen", "a tall man in a grey coat"]
            .iter()
            .map(|n| Asked {
                who: n.to_string(),
                doing: None,
            })
            .collect();
        let longest = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "laughing"},
                {"who": "Maya Chen", "where": "right", "doing": ""},
                {"who": "a tall man in a grey coat", "where": "background",
                 "doing": "Maya Chen taking off her coat"}], "together": ""}"#,
            &mixed,
        )
        .unwrap();
        assert_eq!(longest.roles[1].doing, "taking off her coat");
        assert_eq!(longest.roles[2].doing, "together with the others");
        // A part that is only a name is no part: both only names fails,
        // and one only a name is made neutral.
        assert!(read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "Maya"},
                {"who": "John", "where": "right", "doing": "John."}], "together": ""}"#,
            &names(),
        )
        .is_err());
        // A bare other name, which no earlier repair touches: the misfile
        // move needs words after the name (review of #622).
        let one = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "John"},
                {"who": "John", "where": "right", "doing": "lifting Maya off the ground"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(one.roles[0].doing, "together with the other person");
        assert_eq!(one.roles[1].doing, "lifting Maya off the ground");
        assert!(!names_only("Maya laughing", &["Maya".into()]));
        // A pair filed under each other swaps whole: neither part is lost.
        let crossed = read_split(
            r#"{"people": [{"who": "Maya", "where": "left", "doing": "John handing her the keys"},
                {"who": "John", "where": "right", "doing": "Maya taking the keys"}], "together": ""}"#,
            &names(),
        )
        .unwrap();
        assert_eq!(crossed.roles[0].doing, "taking the keys");
        assert_eq!(crossed.roles[1].doing, "handing her the keys");
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
