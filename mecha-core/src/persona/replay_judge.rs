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
//! - **Identity** (since `replay-judges-2`): each face the detector finds,
//!   embedded (`face_id`) and matched to the record's library cast by their
//!   portraits, the best assignment over all of them ([`assign`]).
//! - **Each person's own clauses**, asked on a crop around that person
//!   ([`person_crop`]): what they are doing, wearing and showing. And the
//!   **bleed**: each person's doing and wearing asked of every *other*
//!   identified person's crop, where a yes is an attribute drawn on the
//!   wrong person, which a whole-picture clause ("a person in this picture
//!   is wearing it") passes (the replay smoke of #644).
//!
//! A whole-picture clause still names no one: the vision model cannot tell
//! the cast apart by name. Who is who is the identity judge's answer.
//!
//! Everything here is a pure function of the answers; the questions are
//! asked by the caller (`mecha replay --judge`), as quarantined one-shots
//! with the picture beside them (`QuarantinedPass::ask_about`).

use crate::scene::{Scene, Setting};
use serde::{Deserialize, Serialize};

/// The judges' version. Bump it whenever a question, a parse or a count
/// changes, so a verdict is never compared with one the old judges gave.
pub const JUDGES: &str = "replay-judges-2";

/// The ArcFace similarity at which a face is taken for a library
/// character's (`face_id`): the line a3's judges drew, under which two
/// different people's faces fall (0.35 at most on py-feat's own measure).
pub const IDENTITY_MATCH: f32 = 0.35;

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

impl Clause {
    /// The question for one person's crop: about the person in its middle,
    /// since a crop can catch the edge of someone beside them.
    pub fn crop_question(&self) -> String {
        let quoted = self.text.trim();
        let ask = match self.field.as_str() {
            "doing" => {
                format!("Is the person in the middle of this picture doing this: \"{quoted}\"?")
            }
            "wearing" => {
                format!("Is the person in the middle of this picture wearing this: \"{quoted}\"?")
            }
            "expression" => format!(
                "Does the person in the middle of this picture have this expression: \"{quoted}\"?"
            ),
            other => format!(
                "Does the person in the middle of this picture show this {other}: \"{quoted}\"?"
            ),
        };
        format!("{ask} Answer yes or no.")
    }

    /// Whether another person showing it means it was drawn on the wrong
    /// one: what someone is doing or wearing, not an expression two people
    /// may share.
    pub fn bleeds(&self) -> bool {
        matches!(self.field.as_str(), "doing" | "wearing")
    }
}

/// The best assignment of faces to people: `similarity[p][f]` is person
/// p's portrait against face f, and each face goes to at most one person,
/// so that the total over the matched pairs is the greatest. A total, not
/// each pair's own best: two near-tied cross pairs can outscore the right
/// one, which in ArcFace's measured range (different people at 0.35 or
/// under) stays below the match line (review of #655). Exhaustive,
/// which a picture's handful of people allows; beyond eight people or faces
/// each person takes their best free face in turn.
pub fn assign(similarity: &[Vec<f32>]) -> Vec<Option<usize>> {
    let people = similarity.len();
    let faces = similarity.iter().map(Vec::len).max().unwrap_or(0);
    if people > 8 || faces > 8 {
        let mut taken = vec![false; faces];
        return similarity
            .iter()
            .map(|row| {
                let best = (0..row.len())
                    .filter(|&f| !taken[f])
                    .max_by(|&a, &b| row[a].total_cmp(&row[b]))?;
                taken[best] = true;
                Some(best)
            })
            .collect();
    }
    fn walk(
        sim: &[Vec<f32>],
        p: usize,
        taken: &mut Vec<bool>,
        now: &mut Vec<Option<usize>>,
        score: f32,
        best: &mut (f32, Vec<Option<usize>>),
    ) {
        if p == sim.len() {
            if score > best.0 {
                *best = (score, now.clone());
            }
            return;
        }
        now.push(None);
        walk(sim, p + 1, taken, now, score, best);
        now.pop();
        for f in 0..sim[p].len() {
            if taken[f] {
                continue;
            }
            taken[f] = true;
            now.push(Some(f));
            walk(sim, p + 1, taken, now, score + sim[p][f], best);
            now.pop();
            taken[f] = false;
        }
    }
    let mut best = (f32::NEG_INFINITY, vec![None; people]);
    walk(
        similarity,
        0,
        &mut vec![false; faces],
        &mut Vec::new(),
        0.0,
        &mut best,
    );
    best.1
}

/// The part of a `width`×`height` picture that is the person whose face
/// box is `face` (`x0, y0, x1, y1`): about a shoulder's width either side
/// of the face, from just above it to about seven face heights below,
/// inside the picture. As `(x, y, width, height)`.
pub fn person_crop(face: [f32; 4], width: u32, height: u32) -> (u32, u32, u32, u32) {
    let [x0, y0, x1, y1] = face;
    let (fw, fh) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    let cx = (x0 + x1) / 2.0;
    let left = (cx - 1.6 * fw).clamp(0.0, width as f32);
    let right = (cx + 1.6 * fw).clamp(0.0, width as f32);
    let top = (y0 - 0.6 * fh).clamp(0.0, height as f32);
    let bottom = (y1 + 7.0 * fh).clamp(0.0, height as f32);
    // At least a pixel, inside the picture even for a face decoded beyond
    // its edge (review of #655).
    let x = (left as u32).min(width.saturating_sub(1));
    let y = (top as u32).min(height.saturating_sub(1));
    (
        x,
        y,
        ((right - left) as u32).clamp(1, (width - x).max(1)),
        ((bottom - top) as u32).clamp(1, (height - y).max(1)),
    )
}

/// Whether the record gives `other` the same `clause` text in the same
/// field: then reading it on their crop is the record's own word, never a
/// bleed, and is not asked (review of #655).
pub fn shared(scene: &Scene, clause: &Clause, other: usize) -> bool {
    let Some(p) = scene.people.get(other) else {
        return false;
    };
    let theirs = match clause.field.as_str() {
        "doing" => &p.doing,
        "wearing" => &p.wearing,
        "expression" => &p.expression,
        _ => return false,
    };
    theirs.trim().eq_ignore_ascii_case(clause.text.trim())
}

/// The identity judge's models and the library its cast is read from: the
/// face detector, ArcFace, and each character's portrait embedded once.
pub struct IdentityJudge {
    reader: crate::face_id::Reader,
    library: crate::imagelib::Library,
    portraits:
        std::sync::Mutex<std::collections::HashMap<String, std::result::Result<Vec<f32>, String>>>,
}

/// What the identity judge made of one picture: how many faces, whose
/// face each library character's is, a crop of each person identified
/// (by the record's index), and what it could not do.
pub struct Seen {
    pub faces: Option<usize>,
    pub identities: Vec<Identity>,
    pub crops: Vec<(usize, crate::quarantine::Shown)>,
    pub errors: Vec<String>,
}

impl IdentityJudge {
    /// The two models from their weights, and the library at `library`.
    pub fn load(
        detector: &std::path::Path,
        arcface: &std::path::Path,
        library: &std::path::Path,
    ) -> anyhow::Result<Self> {
        Ok(IdentityJudge {
            reader: crate::face_id::Reader::load(detector, arcface)?,
            library: crate::imagelib::Library::load(library).0,
            portraits: Default::default(),
        })
    }

    /// Every face in `img`, with its embedding, in `img`'s own pixels
    /// (`face_id::Reader::read`).
    fn found(&self, img: &image::RgbImage) -> anyhow::Result<Vec<(crate::face::Face, Vec<f32>)>> {
        self.reader.read(img)
    }

    /// A library character's portrait, embedded: its largest face. Once
    /// per run per character; a portrait that cannot be had is said.
    fn portrait(&self, who: &str) -> std::result::Result<Vec<f32>, String> {
        let key = who.trim().to_lowercase();
        let mut known = self.portraits.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(done) = known.get(&key) {
            return done.clone();
        }
        let got = (|| {
            let entry = self
                .library
                .get(crate::imagelib::Kind::Character, &key)
                .ok_or_else(|| format!("{who} is not in the library"))?;
            // The portrait, which a new picture is drawn from (`compile`
            // sends it): a judge grades the generator against what it saw,
            // never a `face` reference it did not (review of #705, pass 3).
            let (bytes, _) = self
                .library
                .read_portrait(entry)
                .map_err(|e| format!("{who}'s portrait could not be read: {e:#}"))?;
            let img = crate::image::decode_upright(&bytes, "the portrait")
                .map_err(|e| format!("{who}'s portrait did not decode: {e:#}"))?
                .to_rgb8();
            let faces = self
                .found(&img)
                .map_err(|e| format!("{who}'s portrait: {e:#}"))?;
            faces
                .into_iter()
                .max_by(|a, b| a.0.area().total_cmp(&b.0.area()))
                .map(|(_, e)| e)
                .ok_or_else(|| format!("{who}'s portrait shows no face"))
        })();
        known.insert(key, got.clone());
        got
    }

    /// The faces in a picture's `bytes`, matched to the library cast
    /// `scene` names, and a crop of each person identified. CPU work: run
    /// it off an async runtime.
    pub fn identify(&self, bytes: &[u8], scene: Option<&Scene>) -> Seen {
        let mut seen = Seen {
            faces: None,
            identities: Vec::new(),
            crops: Vec::new(),
            errors: Vec::new(),
        };
        // Upright, as `mecha imagelib face compare` reads a picture
        // (`face_id::Reader::read_bytes`): a phone photo's EXIF turn read
        // one way here and another there gave the same picture two numbers
        // (review of #705, pass 4).
        let img = match crate::image::decode_upright(bytes, "the picture") {
            Ok(i) => i.to_rgb8(),
            Err(e) => {
                seen.errors.push(format!("the picture did not decode: {e}"));
                return seen;
            }
        };
        let found = match self.found(&img) {
            Ok(f) => f,
            Err(e) => {
                seen.errors.push(format!("the face detector failed: {e:#}"));
                return seen;
            }
        };
        seen.faces = Some(found.len());
        let Some(scene) = scene else {
            return seen;
        };
        // The record's library characters with a portrait to compare.
        let mut cast = Vec::new();
        for (i, p) in scene.people.iter().enumerate() {
            if let crate::scene::Who::Library(who) = &p.who {
                match self.portrait(who) {
                    Ok(e) => cast.push((i, who.clone(), e)),
                    Err(why) => seen.errors.push(why),
                }
            }
        }
        let similarity: Vec<Vec<f32>> = cast
            .iter()
            .map(|(_, _, e)| {
                found
                    .iter()
                    .map(|(_, f)| crate::face_id::similarity(e, f))
                    .collect()
            })
            .collect();
        for (k, ((person, who, _), face)) in cast.iter().zip(assign(&similarity)).enumerate() {
            let identity = Identity {
                person: *person,
                who: who.clone(),
                face,
                similarity: face.map(|f| similarity[k][f]),
            };
            if let (true, Some(f)) = (identity.identified(), face) {
                let b = found[f].0;
                let (x, y, w, h) = person_crop([b.x0, b.y0, b.x1, b.y1], img.width(), img.height());
                let crop = image::imageops::crop_imm(&img, x, y, w, h).to_image();
                let mut png = Vec::new();
                match crop
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .map_err(anyhow::Error::from)
                    .and_then(|()| crate::quarantine::Shown::of(&png))
                {
                    Ok(shown) => seen.crops.push((*person, shown)),
                    Err(e) => seen
                        .errors
                        .push(format!("{who}'s crop could not be shown: {e:#}")),
                }
            }
            seen.identities.push(identity);
        }
        seen
    }
}

/// What a scene record says its picture shows, as clauses: the setting's
/// words, the style, what is between the people, each person's doing,
/// wearing and expression, and each run of words the picture renders. A
/// photo used as the setting is the owner's picture, not words to check;
/// clothes the harness filled in because nobody named any say nothing;
/// light and camera say how it was shot, which a yes/no reads poorly; an
/// expression under a camera behind the people is one the compiler did not
/// send and the picture cannot show; and an empty field says nothing. Those
/// are left out.
pub fn clauses(scene: &Scene) -> Vec<Clause> {
    // Seen from behind, a face is out of sight and its expression was not
    // sent (`imagelib::CastMember::expression`; review of #757, pass 8).
    let face_away = scene
        .camera
        .as_ref()
        .is_some_and(|f| crate::picture::faces_away(&f.value));
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
        // The harness's own stand-in for clothes nobody named is true of
        // any outfit, so it is no clause (`picture::WEARING_UNSAID`).
        if p.wearing.trim() != crate::picture::WEARING_UNSAID {
            push("wearing", Some(i), &p.wearing);
        }
        if !face_away {
            push("expression", Some(i), &p.expression);
        }
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
    /// Each library character the record names, and the face that is
    /// theirs, if one is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<Identity>,
    /// Each identified person's own clauses, on their own crop, and on
    /// everyone else's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub person_clauses: Vec<PersonVerdict>,
    /// What a judge could not do, said rather than counted as a no.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// A library character the record names whose portrait could be compared,
/// and the face matched to them. One with no portrait to compare is in
/// `errors`, never here, so "not identified" means a face was looked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// The record's person, by index.
    pub person: usize,
    pub who: String,
    /// The detector's face, by index, best match first assigned.
    pub face: Option<usize>,
    /// ArcFace similarity to their portrait; `None` when no face was
    /// assigned to them: none was left, or every face left would have
    /// lowered the total (`assign`).
    pub similarity: Option<f32>,
}

impl Identity {
    /// Whether the face matched is theirs.
    pub fn identified(&self) -> bool {
        self.similarity.is_some_and(|s| s >= IDENTITY_MATCH)
    }
}

/// A person's clause, asked of their own crop and of each other identified
/// person's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonVerdict {
    #[serde(flatten)]
    pub clause: Clause,
    /// On their own crop; `None` when they were not identified.
    pub own: Option<bool>,
    /// On each other identified person's crop: a yes is the clause drawn
    /// on them too. Empty for an expression, which two people may share.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elsewhere: Vec<Elsewhere>,
}

/// A person's clause read on someone else's crop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Elsewhere {
    /// Whose crop: the record's person, by index.
    pub person: usize,
    /// A yes is the clause drawn on them: a bleed onto `person`.
    pub shows: Option<bool>,
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
    /// Library characters the record names whose face was found and
    /// matched.
    pub identified: Counted,
    /// Identified people's own clauses read on their own crop.
    pub own_shown: Counted,
    /// A person's doing or wearing read on another person's crop: drawn on
    /// the wrong person.
    pub bleeds: Counted,
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
        identified: Counted::of(
            judged
                .iter()
                .flat_map(|j| &j.identities)
                .map(Identity::identified),
        ),
        own_shown: Counted::of(
            judged
                .iter()
                .flat_map(|j| &j.person_clauses)
                .filter_map(|v| v.own),
        ),
        bleeds: Counted::of(
            judged
                .iter()
                .flat_map(|j| &j.person_clauses)
                .flat_map(|v| v.elsewhere.iter().filter_map(|e| e.shows)),
        ),
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
    if !j.identities.is_empty() {
        let found = j.identities.iter().filter(|i| i.identified()).count();
        let bled = j
            .person_clauses
            .iter()
            .flat_map(|v| &v.elsewhere)
            .filter(|e| e.shows == Some(true))
            .count();
        lines.push(format!(
            "identified {found}/{} · bleed {bled}",
            j.identities.len()
        ));
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
            people: vec![
                person("reading a map", ""),
                person("", "a red scarf"),
                person("", crate::picture::WEARING_UNSAID),
            ],
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

    /// Seen from behind, an expression is no clause: the compiler did not
    /// send it and the picture cannot show it. From the front it is one.
    #[test]
    fn an_expression_is_no_clause_from_behind() {
        let with_camera = |camera: &str| Scene {
            camera: field(camera.into()),
            people: vec![Person {
                expression: "grinning".into(),
                ..person("kneading clay", "an apron")
            }],
            people_known: true,
            ..Scene::default()
        };
        let fields =
            |s: &Scene| -> Vec<String> { clauses(s).into_iter().map(|c| c.field).collect() };
        assert!(fields(&with_camera("eye level, facing her")).contains(&"expression".into()));
        assert!(!fields(&with_camera("seen from behind her at the wheel"))
            .contains(&"expression".into()));
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

    /// Faces go to people for the greatest total similarity, not each
    /// person's first choice, and a face goes to one person only.
    #[test]
    fn faces_are_assigned_for_the_best_total() {
        // Person 0 likes face 0 a little more, but person 1 has no one else.
        let sim = vec![vec![0.70, 0.65], vec![0.68, 0.10]];
        assert_eq!(assign(&sim), vec![Some(1), Some(0)]);
        // More people than faces: one goes without.
        let sim = vec![vec![0.9], vec![0.5]];
        assert_eq!(assign(&sim), vec![Some(0), None]);
        // No faces at all.
        assert_eq!(assign(&[vec![], vec![]]), vec![None, None]);
    }

    /// A person's crop is around their face, reaching down for the body,
    /// and inside the picture.
    #[test]
    fn a_persons_crop_is_around_them_and_inside_the_picture() {
        let (x, y, w, h) = person_crop([100.0, 100.0, 140.0, 150.0], 1000, 1000);
        assert_eq!((x, y), (56, 70));
        assert_eq!(w, 128);
        assert_eq!(h, 430);
        let (x, y, w, h) = person_crop([0.0, 0.0, 60.0, 60.0], 200, 300);
        assert!(x + w <= 200 && y + h <= 300, "{x} {y} {w} {h}");
        assert_eq!((x, y), (0, 0));
        // A face decoded wholly beyond the picture still gives a pixel in it.
        let (x, y, w, h) = person_crop([500.0, 500.0, 560.0, 560.0], 200, 300);
        assert!(
            w >= 1 && h >= 1 && x + w <= 200 && y + h <= 300,
            "{x} {y} {w} {h}"
        );
    }

    /// The identity judge's counts: identified over the people compared,
    /// a person's own clauses on their own crop, and a bleed when another
    /// person's crop shows it.
    #[test]
    fn identity_and_bleed_are_counted() {
        let clause = |field: &str| Clause {
            field: field.into(),
            person: Some(0),
            text: "a green raincoat".into(),
        };
        let j = Judgement {
            identities: vec![
                Identity {
                    person: 0,
                    who: "a".into(),
                    face: Some(0),
                    similarity: Some(0.6),
                },
                Identity {
                    person: 1,
                    who: "b".into(),
                    face: Some(1),
                    similarity: Some(0.2),
                },
            ],
            person_clauses: vec![PersonVerdict {
                clause: clause("wearing"),
                own: Some(true),
                elsewhere: vec![Elsewhere {
                    person: 1,
                    shows: Some(true),
                }],
            }],
            ..Judgement::default()
        };
        let s = summarise(std::slice::from_ref(&j));
        assert_eq!((s.identified.hits, s.identified.of), (1, 2));
        assert_eq!((s.own_shown.hits, s.own_shown.of), (1, 1));
        assert_eq!(
            (s.bleeds.hits, s.bleeds.of),
            (1, 1),
            "drawn on the wrong person"
        );
        assert!(caption(&j).iter().any(|l| l == "identified 1/2 · bleed 1"));
        assert!(clause("wearing").bleeds() && !clause("expression").bleeds());
        // Two people the record dresses alike: the other's crop is not asked.
        let mut a = Person {
            who: Who::Library("a".into()),
            at: None,
            wearing: "A Green Raincoat".into(),
            doing: String::new(),
            expression: String::new(),
            origin: Origin::Clean,
        };
        let scene = Scene {
            people: vec![a.clone(), {
                a.wearing = "a green raincoat ".into();
                a
            }],
            ..Scene::default()
        };
        assert!(shared(&scene, &clause("wearing"), 1));
        assert!(!shared(&scene, &clause("doing"), 1));
        assert!(!shared(&scene, &clause("wearing"), 7));
        assert!(clause("wearing")
            .crop_question()
            .starts_with("Is the person in the middle of this picture wearing"));
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
