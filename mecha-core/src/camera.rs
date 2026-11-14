//! Where a picture's camera stood, read from the picture, and where an ask
//! moves it to.
//!
//! A re-shoot (`picture::Plan::reshot`) turns the camera **from where the
//! picture has it**, so it needs that place. The record's camera words are
//! what someone asked for, loosely, and often not where the picture ended
//! up: counted from them, a picture already shot from behind was turned
//! half-way round again and showed the front (replay, 2026-11-12). The
//! picture itself is read instead, by a quarantined pass whose whole answer
//! is three closed names ([`ShotReader`]): no tools, no history, and no
//! prose of its own reaching any prompt.
//!
//! An ask that names no place of its own ("from the other side", "a touch
//! closer") is a move, and a move has a place only against a start
//! ([`relative`]).

use crate::scene::{Distance, Height, Shot};

/// How long a camera read may take: it holds a picture's turn, and a picture
/// restaged without it is only what every picture was before.
pub const SHOT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Who reads where a picture's camera stood. Handed to `image_generate` per
/// turn (`ToolCtx::shot_reader`).
#[async_trait::async_trait]
pub trait ShotReader: Send + Sync + std::fmt::Debug {
    /// The camera's place in `picture` (its bytes), every part named, or why
    /// it could not be read.
    async fn read(&self, picture: &[u8]) -> Result<Shot, String>;
}

/// The reader on a model that sees pictures: the persona's own.
pub struct ModelShotReader {
    pub provider: Box<dyn crate::provider::Provider>,
    pub model: String,
}

impl std::fmt::Debug for ModelShotReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelShotReader")
            .field("model", &self.model)
            .finish()
    }
}

const SYSTEM: &str = "You are shown one picture. Say where the camera stood relative to the main \
person in it. around: which side of the person the camera sees (front: their face and chest are \
toward the camera; back: their back is toward the camera; left or right: the person's own left \
or right side is toward the camera; the quarter names are between). Judge by the torso, not by \
a head turned over a shoulder. height: low (the camera is below the person's waist, looking \
up), eye (level with them), elevated (above head height, looking a little down), high (well \
above, looking steeply down). distance: close (head and shoulders, or tighter), medium (from \
the waist or the knees up), wide (the whole body with room around it). Answer with JSON only.";

const QUESTION: &str = "Where did the camera stand?";

/// The answer's shape: the three parts, each one of its closed names, as a
/// call's `scene.view` names them.
fn schema() -> serde_json::Value {
    let ids = |all: &[&str]| serde_json::json!({"type": "string", "enum": all});
    serde_json::json!({
        "type": "object",
        "properties": {
            "around": ids(&Shot::AROUND),
            "height": ids(&Height::ALL.map(Height::id)),
            "distance": ids(&Distance::ALL.map(Distance::id)),
        },
        "required": ["around", "height", "distance"],
        "additionalProperties": false,
    })
}

/// The reader's answer, read: the JSON object in it as a [`Shot`] with
/// every part named. Anything else is a failure, never a guess: a part left
/// out would be counted as the front, eye level or a medium shot, which is
/// the mistake the read exists to stop.
pub fn read_answer(text: &str) -> Result<Shot, String> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else {
        return Err("the camera read gave no answer".into());
    };
    let v: serde_json::Value = serde_json::from_str(&text[a..=b])
        .map_err(|_| "the camera read's answer was not JSON".to_string())?;
    let shot = Shot::of_json(&v)?;
    if shot.around.is_none() || shot.height.is_none() || shot.distance.is_none() {
        return Err("the camera read left a part unsaid".into());
    }
    Ok(shot)
}

#[async_trait::async_trait]
impl ShotReader for ModelShotReader {
    async fn read(&self, picture: &[u8]) -> Result<Shot, String> {
        let shown = crate::quarantine::Shown::of(picture)
            .map_err(|e| format!("the picture could not be shown ({e:#})"))?;
        let request =
            crate::quarantine::QuarantinedPass::new(&self.model, crate::provider::LOCAL_MAX_TOKENS)
                .system(SYSTEM)
                .no_thinking()
                .response_schema(self.provider.structured_output().then(schema))
                .ask_about(&shown, QUESTION);
        let response = self
            .provider
            .complete(&request, None)
            .await
            .map_err(|e| format!("the camera read could not be reached ({e:#})"))?;
        // The envelope before the content: a refusal arrives as success.
        if response.stop_reason == crate::message::StopReason::Refusal {
            return Err("the camera read was refused".into());
        }
        read_answer(&response.message.text())
    }
}

/// What an ask says of the camera beyond the camera a picture already has:
/// when its words open with the recorded camera's (a panel joins a move to
/// the shot it moves, `persona::edit`), only what follows them. The recorded
/// words are where someone once asked the camera to be, and read again as
/// this ask they would turn a picture that never got there.
pub fn asked<'a>(camera: &'a str, recorded: Option<&str>) -> &'a str {
    let camera = camera.trim();
    match recorded.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) if camera.len() > r.len() && camera.starts_with(r) => {
            camera[r.len()..].trim_start_matches([',', ' ', '.', ';'])
        }
        _ => camera,
    }
}

/// Where an ask puts a camera that stood at `was`, for a call whose words
/// may restate the `recorded` camera before the move ([`asked`]). A typed
/// `view` answered beside restated words describes the restated whole, so
/// it is read only where the words were the ask too: kept, a restated
/// distance would beat a step nearer and the same frame be shot again.
pub fn moved_to(
    view: Option<Shot>,
    words: &str,
    recorded: Option<&str>,
    was: Shot,
) -> Option<Shot> {
    let ask = asked(words, recorded);
    let view = view.filter(|_| ask.len() == words.trim().len());
    target(view, ask, was)
}

/// Where a camera that stood at `was` stands after a move said against it:
/// the other side is half-way round, nearer and further are one step of
/// distance, higher and lower one step of height. The parts the words do
/// not move are `None`. `None` altogether when the words say no such move.
pub fn relative(words: &str, was: Shot) -> Option<Shot> {
    let w: Vec<String> = words
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    let has = |x: &str| w.iter().any(|t| t == x);
    let run = |seq: &[&str]| w.windows(seq.len()).any(|win| win == seq);
    let around = (run(&["other", "side"])
        || run(&["opposite", "side"])
        || run(&["opposite", "angle"])
        || run(&["reverse", "angle"]))
    .then(|| (was.around.unwrap_or(0) + 4) % 8);
    let near = has("closer")
        || has("nearer")
        || has("tighter")
        || run(&["zoom", "in"])
        || run(&["move", "in"])
        || run(&["push", "in"]);
    let far = has("wider")
        || has("further")
        || has("farther")
        || run(&["zoom", "out"])
        || run(&["pull", "back"])
        || run(&["step", "back"]);
    let at = was.distance.unwrap_or(Distance::Medium);
    let distance = match (near, far) {
        (true, false) => Some(match at {
            Distance::Wide => Distance::Medium,
            _ => Distance::CloseUp,
        }),
        (false, true) => Some(match at {
            Distance::CloseUp => Distance::Medium,
            _ => Distance::Wide,
        }),
        _ => None,
    };
    let stood = was.height.unwrap_or(Height::Eye);
    let step = |by: i8| {
        let i = Height::ALL.iter().position(|h| *h == stood).unwrap_or(1) as i8;
        Height::ALL[(i + by).clamp(0, Height::ALL.len() as i8 - 1) as usize]
    };
    let height = match (has("higher"), has("lower")) {
        (true, false) => Some(step(1)),
        (false, true) => Some(step(-1)),
        _ => None,
    };
    let moved = Shot {
        around,
        height,
        distance,
    };
    (moved != Shot::default()).then_some(moved)
}

/// Where an ask puts a camera that stands at `was`: the place its words (or
/// the call's typed `view`) name, then what they move against `was`, then
/// `was` for the rest. `None` when the ask names no place and no move.
pub fn target(view: Option<Shot>, words: &str, was: Shot) -> Option<Shot> {
    // The view leads its words where it speaks; a part it leaves out is the
    // words' to name (a typed side beside "a close-up").
    let named = view.unwrap_or_default().over(Shot::read(words));
    let moved = relative(words, was).unwrap_or_default();
    let said = named.over(moved);
    (said != Shot::default()).then(|| said.over(was))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(around: u8, height: Height, distance: Distance) -> Shot {
        Shot {
            around: Some(around),
            height: Some(height),
            distance: Some(distance),
        }
    }

    /// A view typed beside words that restate the recorded camera is the
    /// restatement's, and never beats the move the words go on to ask.
    #[test]
    fn a_view_beside_a_restated_camera_does_not_undo_the_move() {
        let was = at(4, Height::Eye, Distance::Wide);
        let restated = Some(Shot {
            around: Some(4),
            height: None,
            distance: Some(Distance::Wide),
        });
        assert_eq!(
            moved_to(
                restated,
                "a wide shot from behind, a bit closer",
                Some("a wide shot from behind"),
                was
            ),
            Some(at(4, Height::Eye, Distance::Medium))
        );
        // Words that are the whole ask keep their view.
        assert_eq!(
            moved_to(
                Some(Shot {
                    around: Some(2),
                    ..Default::default()
                }),
                "from the side",
                Some("a wide shot from behind"),
                was
            ),
            Some(at(2, Height::Eye, Distance::Wide))
        );
    }

    /// The read's answer is three closed names and nothing else: a part
    /// left out, a name outside the set, or prose is a failure.
    #[test]
    fn a_camera_read_is_three_closed_names_or_a_failure() {
        assert_eq!(
            read_answer(r#"{"around": "back_left", "height": "elevated", "distance": "wide"}"#),
            Ok(at(5, Height::Elevated, Distance::Wide))
        );
        assert_eq!(
            read_answer(
                "```json\n{\"around\":\"front\",\"height\":\"eye\",\"distance\":\"close\"}\n```"
            ),
            Ok(at(0, Height::Eye, Distance::CloseUp))
        );
        for bad in [
            r#"{"around": "back", "height": "eye"}"#,
            r#"{"around": "above", "height": "eye", "distance": "wide"}"#,
            "the camera is behind her",
            "",
        ] {
            assert!(read_answer(bad).is_err(), "{bad}");
        }
        let names = schema();
        assert_eq!(names["properties"]["around"]["enum"][5], "back_left");
        assert_eq!(names["required"].as_array().unwrap().len(), 3);
    }

    /// A move has a place only against the start: the other side of a
    /// picture shot from behind is its front.
    #[test]
    fn a_move_is_counted_from_where_the_picture_has_the_camera() {
        let behind = at(4, Height::Eye, Distance::Medium);
        let front = at(0, Height::Eye, Distance::Wide);
        assert_eq!(
            target(None, "from the other side", behind),
            Some(at(0, Height::Eye, Distance::Medium))
        );
        assert_eq!(
            target(None, "from the other side", front),
            Some(at(4, Height::Eye, Distance::Wide))
        );
        assert_eq!(
            target(None, "a touch closer", front),
            Some(at(0, Height::Eye, Distance::Medium))
        );
        assert_eq!(
            target(None, "pull back and a little higher", behind),
            Some(at(4, Height::Elevated, Distance::Wide))
        );
        // A place named outright is that place, whatever the start.
        assert_eq!(
            target(None, "a close up from behind", front),
            Some(at(4, Height::Eye, Distance::CloseUp))
        );
        // A typed view that names one part leaves the rest to the words.
        assert_eq!(
            target(
                Some(Shot {
                    around: Some(2),
                    ..Default::default()
                }),
                "a close up from the side",
                front
            ),
            Some(at(2, Height::Eye, Distance::CloseUp))
        );
        // The call's typed view leads its words.
        assert_eq!(
            target(
                Some(Shot {
                    around: Some(2),
                    ..Default::default()
                }),
                "from behind",
                front
            ),
            Some(at(2, Height::Eye, Distance::Wide))
        );
        assert_eq!(target(None, "soft focus, a candid feel", front), None);
    }

    /// A panel joins a move to the recorded camera; only the move is the
    /// ask.
    #[test]
    fn the_recorded_cameras_words_are_not_the_ask() {
        assert_eq!(
            asked(
                "a wide shot from behind, a touch closer",
                Some("a wide shot from behind")
            ),
            "a touch closer"
        );
        assert_eq!(
            asked("from the left side", Some("a wide shot")),
            "from the left side"
        );
        assert_eq!(asked("a wide shot", Some("a wide shot")), "a wide shot");
        assert_eq!(asked("  from below ", None), "from below");
    }
}
