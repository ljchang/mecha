//! Judges over a persona replay's renders: what each picture shows, read
//! against what its scene record says it should, so two arms can be
//! compared on the pictures and not only on the calls (owner, 2026-10-09:
//! "render so we can see if it improved").
//!
//! The judges are versioned ([`JUDGES`]): a judge that changes is its own
//! arm, so a verdict names the judges that gave it, and old renders can be
//! judged again without being drawn again. What is measured, per picture:
//!
//! - **People**: faces the detector finds (`face::FaceAnchors::faces_in`),
//!   the vision model's count ([`PEOPLE_QUESTION`]), and the record's own
//!   count. More asked than recorded is a duplicate or a stranger.
//! - **Clauses**: one yes/no question per thing the record says the
//!   picture shows ([`clauses`]), asked of the picture and, for an edit, of
//!   the picture it started from too. That control separates a clause the
//!   edit drew from one the base already showed.
//! - **Layout** against the base (`imagegen::layout_similarity`), and the
//!   manifest's own `roles` and `reader` outcomes, carried as they are.
//!
//! A clause names no one: the vision model cannot tell the cast apart by
//! name, so "a person in this picture" is what is asked. Who is who is a
//! face-identity judge's question, which this version does not ask.
//!
//! Everything here is a pure function of the answers; the questions are
//! asked by the caller (`mecha replay --judge`), as quarantined one-shots
//! with the picture beside them (`QuarantinedPass::ask_about`).

use crate::scene::{Scene, Setting};
use serde::{Deserialize, Serialize};

/// The judges' version. Bump it whenever a question, a parse or a count
/// changes, so a verdict is never compared with one the old judges gave.
pub const JUDGES: &str = "replay-judges-1";

/// How many people a picture shows, asked of the vision model.
pub const PEOPLE_QUESTION: &str = "How many people are in this picture? Count every \
person, whole or partly shown, including any reflection or picture of a person. Answer \
with a number only.";

/// The frame every judge question is asked under.
pub const SYSTEM: &str = "You answer questions about the picture you are shown. Answer \
only from what the picture shows. Give the answer alone, with no explanation.";

/// One thing a scene record says its picture shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clause {
    /// `setting`, `style`, `together`, `doing`, `wearing` or `expression`.
    pub field: String,
    /// The record's person, by index, for a person's clause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<usize>,
    pub text: String,
}

impl Clause {
    /// The yes/no question asked of a picture for this clause.
    pub fn question(&self) -> String {
        let quoted = self.text.trim();
        let ask = match self.field.as_str() {
            "setting" => format!("Is this picture's setting this: \"{quoted}\"?"),
            "style" => format!("Is this picture in this style: \"{quoted}\"?"),
            "together" => {
                format!("Does this picture show this between the people in it: \"{quoted}\"?")
            }
            "doing" => format!("Does this picture show a person doing this: \"{quoted}\"?"),
            "wearing" => format!("Is a person in this picture wearing this: \"{quoted}\"?"),
            "expression" => {
                format!("Does a person in this picture have this expression: \"{quoted}\"?")
            }
            "text" => format!("Are these words written in this picture: \"{quoted}\"?"),
            other => format!("Does this picture show this {other}: \"{quoted}\"?"),
        };
        format!("{ask} Answer yes or no.")
    }
}

/// What a scene record says its picture shows, as clauses: the setting's
/// words, the style, what is between the people, each person's doing,
/// wearing and expression, and each run of words the picture renders. A
/// photo used as the setting is the owner's picture, not words to check;
/// light and camera say how it was shot, which a yes/no reads poorly; and
/// an empty field says nothing. Those are left out.
pub fn clauses(scene: &Scene) -> Vec<Clause> {
    let mut out = Vec::new();
    let mut push = |field: &str, person: Option<usize>, text: &str| {
        if !text.trim().is_empty() {
            out.push(Clause {
                field: field.into(),
                person,
                text: text.trim().into(),
            });
        }
    };
    if let Some(setting) = &scene.setting {
        if let Setting::Words { text } = &setting.value {
            push("setting", None, text);
        }
    }
    if let Some(style) = &scene.style {
        push("style", None, &style.value);
    }
    if let Some(together) = &scene.together {
        push("together", None, &together.value);
    }
    for (i, p) in scene.people.iter().enumerate() {
        push("doing", Some(i), &p.doing);
        push("wearing", Some(i), &p.wearing);
        push("expression", Some(i), &p.expression);
    }
    for words in scene.text.iter().flat_map(|t| &t.value) {
        push("text", None, &words.words);
    }
    out
}

/// The answer after any reasoning the server let through.
fn answer_of(text: &str) -> &str {
    text.rsplit_once("</think>").map_or(text, |(_, a)| a).trim()
}

/// A yes or a no, read strictly: the answer's first word, and nothing
/// else. Anything else is no verdict, never a guess.
pub fn yes_no(text: &str) -> Option<bool> {
    let first = answer_of(text)
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| !w.is_empty())?
        .to_lowercase();
    match first.as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// A count, read strictly: the answer's first word as a number, in digits
/// or spelled out to twelve.
pub fn count(text: &str) -> Option<u32> {
    let first = answer_of(text)
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| !w.is_empty())?
        .to_lowercase();
    if let Ok(n) = first.parse() {
        return Some(n);
    }
    Some(match first.as_str() {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        _ => return None,
    })
}

/// One clause's answers: of the picture, and of the picture it started
/// from when it is an edit. `None` is no readable answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(flatten)]
    pub clause: Clause,
    pub picture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<bool>,
}

/// Everything the judges said about one picture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Judgement {
    pub judges: String,
    pub sample: usize,
    /// The picture, as the sample names it.
    pub picture: String,
    /// The picture this one was an edit of, when it was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Faces the detector found; `None` with the reason in `errors` when it
    /// could not look.
    pub faces: Option<usize>,
    /// The vision model's count of people.
    pub people_asked: Option<u32>,
    /// The record's count, when the record knows who is in the picture.
    pub people_recorded: Option<usize>,
    /// Questions asked that got no verdict: no answer, or one that could
    /// not be read. Every rate is over what was answered, so this says how
    /// much it stands on.
    #[serde(default)]
    pub unanswered: usize,
    pub clauses: Vec<Verdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_similarity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader: Option<serde_json::Value>,
    /// The compiled prompt, from the prompt log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// What a judge could not do, said rather than counted as a no.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl Judgement {
    /// More people than the record holds: a duplicate, or a stranger.
    /// `None` when either count is unknown.
    pub fn extra_people(&self) -> Option<bool> {
        Some(self.people_asked? as usize > self.people_recorded?)
    }

    /// Whether the roles the splitter set were applied, from the manifest.
    pub fn roles_applied(&self) -> Option<bool> {
        self.roles.as_ref()?.as_str().map(|r| r == "applied")
    }
}

/// Hits out of a denominator: a rate over nothing is no rate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Counted {
    pub hits: usize,
    pub of: usize,
    pub rate: Option<f64>,
}

impl Counted {
    fn of(pairs: impl Iterator<Item = bool>) -> Self {
        let (mut hits, mut of) = (0, 0);
        for hit in pairs {
            of += 1;
            hits += usize::from(hit);
        }
        Counted {
            hits,
            of,
            rate: (of > 0).then(|| hits as f64 / of as f64),
        }
    }
}

/// One arm's judgements, counted. Counts only: no clause's words, so the
/// summary can go to standard output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub judges: &'static str,
    pub pictures: usize,
    /// The vision model's count equals the record's.
    pub people_match: Counted,
    /// More people than the record holds.
    pub extra_people: Counted,
    /// The detector's faces equal the record's people.
    pub faces_match: Counted,
    /// Clauses the picture was read as showing.
    pub clauses_shown: Counted,
    /// Of an edit's clauses the base was read as not showing, those the
    /// picture was read as showing: what the edit drew.
    pub clauses_drawn: Counted,
    pub roles_applied: Counted,
    /// Over the edits only (a new picture has no base), so with how many:
    /// two arms with different mixes of edits are read by both numbers.
    pub layout_similarity_mean: Option<f64>,
    pub layouts: usize,
    /// Questions asked across the arm that got no verdict: the rates above
    /// are over what was answered, so two arms are read by this too
    /// (review of #644).
    pub unanswered: usize,
}

pub fn summarise(judged: &[Judgement]) -> Summary {
    let layouts: Vec<f64> = judged.iter().filter_map(|j| j.layout_similarity).collect();
    let verdicts = || judged.iter().flat_map(|j| &j.clauses);
    Summary {
        judges: JUDGES,
        pictures: judged.len(),
        people_match: Counted::of(
            judged
                .iter()
                .filter_map(|j| Some(j.people_asked? as usize == j.people_recorded?)),
        ),
        extra_people: Counted::of(judged.iter().filter_map(Judgement::extra_people)),
        faces_match: Counted::of(
            judged
                .iter()
                .filter_map(|j| Some(j.faces? == j.people_recorded?)),
        ),
        clauses_shown: Counted::of(verdicts().filter_map(|v| v.picture)),
        clauses_drawn: Counted::of(verdicts().filter_map(|v| match (v.base, v.picture) {
            (Some(false), Some(shown)) => Some(shown),
            _ => None,
        })),
        roles_applied: Counted::of(judged.iter().filter_map(Judgement::roles_applied)),
        layout_similarity_mean: (!layouts.is_empty())
            .then(|| layouts.iter().sum::<f64>() / layouts.len() as f64),
        layouts: layouts.len(),
        unanswered: judged.iter().map(|j| j.unanswered).sum(),
    }
}

/// A tile's caption: counts, never a clause's words, so a sheet shown
/// beside another arm's is read by its numbers.
pub fn caption(j: &Judgement) -> Vec<String> {
    let n = |v: Option<usize>| v.map_or("?".to_string(), |n| n.to_string());
    let mut lines = vec![format!("sample {}", j.sample)];
    lines.push(format!(
        "people {} of {} · faces {}",
        n(j.people_asked.map(|p| p as usize)),
        n(j.people_recorded),
        n(j.faces)
    ));
    let shown = Counted::of(j.clauses.iter().filter_map(|v| v.picture));
    if shown.of > 0 {
        lines.push(format!("clauses {}/{}", shown.hits, shown.of));
    }
    if let Some(l) = j.layout_similarity {
        lines.push(format!("layout {l:.2}"));
    }
    if let Some(r) = j.roles.as_ref().and_then(|r| r.as_str()) {
        lines.push(format!("roles {r}"));
    }
    lines
}

/// One picture on the sheet: its address and its caption.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub src: String,
    pub caption: Vec<String>,
}

/// A row of the sheet: an arm's label, and its tiles by sample.
pub type Row = (String, Vec<Tile>);

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The contact sheet: one row per arm, each picture at one height and its
/// own width, so nothing is squeezed to a square (the owner read that as
/// "stretched"). A plain page, with no script and nothing fetched but the
/// pictures it names.
pub fn sheet_html(title: &str, rows: &[Row]) -> String {
    let mut html = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\n\
         body{{font:13px system-ui,sans-serif;margin:16px;background:#fff;color:#111}}\n\
         h2{{font-size:14px;margin:18px 0 6px}}\n\
         .row{{display:flex;gap:10px;overflow-x:auto;padding-bottom:6px}}\n\
         figure{{margin:0;flex:none}}\n\
         img{{height:256px;width:auto;display:block;border:1px solid #ccc}}\n\
         figcaption{{font-size:12px;line-height:1.35;margin-top:3px}}\n\
         @media (prefers-color-scheme: dark){{body{{background:#111;color:#eee}}img{{border-color:#444}}}}\n\
         </style>\n<h1 style=\"font-size:16px\">{}</h1>\n",
        escape(title),
        escape(title)
    );
    for (label, tiles) in rows {
        html.push_str(&format!(
            "<h2>{}</h2>\n<div class=\"row\">\n",
            escape(label)
        ));
        for t in tiles {
            html.push_str(&format!(
                "<figure><img src=\"{}\" loading=\"lazy\" alt=\"\"><figcaption>{}</figcaption></figure>\n",
                escape(&t.src),
                t.caption
                    .iter()
                    .map(|l| escape(l))
                    .collect::<Vec<_>>()
                    .join("<br>")
            ));
        }
        html.push_str("</div>\n");
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Field, Origin, Person, Who};

    fn field<T>(value: T) -> Option<Field<T>> {
        Some(Field {
            value,
            origin: Origin::Clean,
        })
    }

    fn person(doing: &str, wearing: &str) -> Person {
        Person {
            who: Who::Library("mara".into()),
            at: None,
            wearing: wearing.into(),
            doing: doing.into(),
            expression: String::new(),
            origin: Origin::Clean,
        }
    }

    /// Each thing the record says is one clause; an empty field and a photo
    /// setting are none.
    #[test]
    fn a_record_becomes_one_clause_per_thing_it_says() {
        let scene = Scene {
            setting: field(Setting::Words {
                text: "a lighthouse at dusk".into(),
            }),
            style: field("watercolour".into()),
            text: field(vec![crate::scene::Words {
                words: "OPEN LATE".into(),
                at: String::new(),
                look: String::new(),
            }]),
            people: vec![person("reading a map", ""), person("", "a red scarf")],
            people_known: true,
            ..Scene::default()
        };
        let got: Vec<_> = clauses(&scene)
            .into_iter()
            .map(|c| (c.field, c.person, c.text))
            .collect();
        assert_eq!(
            got,
            vec![
                ("setting".into(), None, "a lighthouse at dusk".into()),
                ("style".into(), None, "watercolour".into()),
                ("doing".into(), Some(0), "reading a map".into()),
                ("wearing".into(), Some(1), "a red scarf".into()),
                ("text".into(), None, "OPEN LATE".into()),
            ]
        );
        let photo = Scene {
            setting: field(Setting::Photo {
                path: "inbox/room.jpg".into(),
                hash: "ab".into(),
            }),
            ..Scene::default()
        };
        assert!(clauses(&photo).is_empty());
        let asked = clauses(&scene)[2].question();
        assert!(asked.contains("\"reading a map\"") && asked.ends_with("Answer yes or no."));
    }

    /// An answer is read from its first word only, and anything unclear is
    /// no verdict.
    #[test]
    fn answers_are_read_strictly() {
        assert_eq!(yes_no("Yes."), Some(true));
        assert_eq!(yes_no(" no, it does not"), Some(false));
        assert_eq!(yes_no("<think>hmm</think>\nYes"), Some(true));
        assert_eq!(yes_no("Probably yes"), None);
        assert_eq!(yes_no(""), None);
        assert_eq!(count("2"), Some(2));
        assert_eq!(count("Three."), Some(3));
        assert_eq!(count("<think>x</think> 4 people"), Some(4));
        assert_eq!(count("about two"), None);
    }

    /// The counts: a duplicate is more asked than recorded; a clause the
    /// edit drew is one the base lacked and the picture shows; an unknown
    /// count is in no denominator.
    #[test]
    fn an_arm_is_counted_over_what_was_answered() {
        let clause = |base, picture| Verdict {
            clause: Clause {
                field: "doing".into(),
                person: Some(0),
                text: "waving".into(),
            },
            picture,
            base,
        };
        let a = Judgement {
            judges: JUDGES.into(),
            people_asked: Some(3),
            people_recorded: Some(2),
            faces: Some(2),
            clauses: vec![
                clause(Some(false), Some(true)),
                clause(Some(true), Some(true)),
            ],
            roles: Some(serde_json::json!("applied")),
            layout_similarity: Some(0.8),
            unanswered: 1,
            ..Judgement::default()
        };
        let b = Judgement {
            judges: JUDGES.into(),
            people_asked: None,
            people_recorded: Some(1),
            faces: Some(1),
            clauses: vec![clause(None, None), clause(Some(false), Some(false))],
            roles: Some(serde_json::json!("fell back: one person")),
            layout_similarity: Some(0.6),
            ..Judgement::default()
        };
        let s = summarise(&[a, b]);
        assert_eq!(s.pictures, 2);
        assert_eq!((s.extra_people.hits, s.extra_people.of), (1, 1));
        assert_eq!((s.people_match.hits, s.people_match.of), (0, 1));
        assert_eq!((s.faces_match.hits, s.faces_match.of), (2, 2));
        assert_eq!((s.clauses_shown.hits, s.clauses_shown.of), (2, 3));
        assert_eq!((s.clauses_drawn.hits, s.clauses_drawn.of), (1, 2));
        assert_eq!((s.roles_applied.hits, s.roles_applied.of), (1, 2));
        assert!((s.layout_similarity_mean.unwrap() - 0.7).abs() < 1e-9);
        assert_eq!(s.layouts, 2);
        assert_eq!(s.unanswered, 1, "what the rates do not stand on");
        assert_eq!(
            summarise(&[]).clauses_shown.rate,
            None,
            "a rate over nothing"
        );
    }

    /// The sheet keeps each picture's own shape and escapes what it prints.
    #[test]
    fn the_sheet_keeps_each_pictures_shape_and_escapes_its_text() {
        let html = sheet_html(
            "arm <a> & b",
            &[(
                "arm \"a\"".into(),
                vec![Tile {
                    src: "file:///x/y.png".into(),
                    caption: vec!["sample 0".into(), "people 2 of 2".into()],
                }],
            )],
        );
        assert!(html.contains("height:256px;width:auto"));
        assert!(!html.contains("width:256px"));
        assert!(html.contains("arm &lt;a&gt; &amp; b"));
        assert!(html.contains("arm &quot;a&quot;"));
        assert!(html.contains("sample 0<br>people 2 of 2"));
        assert!(!html.contains("<script"));
    }
}
