//! `video_generate` and `video_prompt_guide`: the clip as the chat's job.
//!
//! The call reads what it needs and decides everything a bad name could
//! cost — the chat's kept settings, the scene, the persona's own character,
//! the library characters the prompt names, each picture and voice — before
//! it hands the render over as the chat's background job
//! (`jobs::DeferredJob`; `docs/BACKGROUND-JOBS-DESIGN.md`). The job waits its
//! turn, waits for the machine, uploads, renders, and writes the clip into
//! the chat's `videos/` through the path jail.
//!
//! **Nothing here starts a server or shells out.** The private server ran
//! `systemctl --user start` when it found ComfyUI down, and `mecha system
//! admit`, `mecha system probe voice.live`, `mecha imagelib show` and
//! `ffmpeg` for its gates and lookups; here they are
//! [`crate::system::floor::admit_now`], [`crate::system::voice_live`], the
//! library's own API and a WAV trim in Rust.

use super::{args, compose, graph, VideoConfig};
use crate::comfy::{clip, Client, Failure, SERVER_WAIT};
use crate::tool::{Capabilities, PersonaSelf, Tool, ToolCtx, ToolOutput};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

pub const NAME: &str = "video_generate";
pub const GUIDE_NAME: &str = "video_prompt_guide";

/// The prompting guide `video_prompt_guide` answers with.
const GUIDE: &str = include_str!("guide.md");

/// How long a clip may wait for its turn, and then for the machine, before
/// it gives up, said; and how long a render may run.
const QUEUE_LIMIT: Duration = Duration::from_secs(3 * 3600);
const RENDER_LIMIT: Duration = Duration::from_secs(3 * 3600);

/// How long a voice reference is cut to, at most (and never past the clip).
const VOICE_SECONDS: f64 = 5.0;

/// The largest picture or voice file read for a clip.
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// The smallest face crop a clip is given, in pixels on its shorter side.
/// H3 scales a reference down but never up, at 16 pixels a token, so a
/// 142 px crop is a 9×9 face: that character's clips matched his portrait
/// at 0.26, against 0.45 with the whole 401 px portrait in the same clip
/// (2026-10-24). Every other crop in the library measured was 253 px or more.
/// 224 is the round 14-token side between the two: low enough that none of
/// those falls back to a whole portrait and its outfit, high enough that the
/// failing size never passes.
const MIN_FACE_CROP: u32 = 224;

/// Whether a face crop is large enough to give a clip in place of its
/// portrait; one whose header does not read is not.
fn big_enough(crop: &[u8]) -> bool {
    image::ImageReader::new(std::io::Cursor::new(crop))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .is_some_and(|(w, h)| w.min(h) >= MIN_FACE_CROP)
}

/// Consecutive failed status polls before a render is given up on: a
/// server restarting under a job loses the job anyway.
const POLL_FAILURES: u32 = 10;

/// Where `SaveVideo` files a clip under the server's output folder: the
/// private server's folder, so clips from both sit together.
const OUTPUT_FOLDER: &str = "mecha-video";

/// Memory a clip is admitted against. Measured 2026-10-12 with the unit's
/// `--vram-headroom 12` (which makes ComfyUI drop the encoder rather than
/// hold it beside the DiT): a 5 s draft peaked at 26.2 GiB from text,
/// 23.0 GiB from one reference. Full is calibrated from its own run.
fn memory_need(size: graph::Size) -> u64 {
    let gib = match size {
        graph::Size::Draft => 28,
        graph::Size::Full => 32,
    };
    gib * 1024 * 1024 * 1024
}

const DESCRIPTION: &str = "Renders a brief clip, picture and sound together, with MiniMax H3 on this machine. Expect roughly 6 minutes for a 5 s full-size turbo clip and around an hour for 10 s without turbo. The first time a conversation asks for a clip, read video_prompt_guide before writing it: H3 follows its own prompt layout much better than free prose. When the render finishes, the clip shows up at this call together with the settings it used, so there is nothing to poll. `turbo`, `seconds`, `size` and `aspect` carry over from this chat's previous clip; pass them only to change them. Clips render one after another, and a clip waits out a live voice call or a shortage of memory. Library characters named anywhere in the prompt bring their portrait, stored description and voice along.";

const GUIDE_DESCRIPTION: &str = "The prompt layout MiniMax H3 was trained on, for video_generate: shots and their timing, numbered speakers and their lines, camera moves, ambient sound and score, and a worked example. Read it once per conversation, before the first clip.";

/// What a second clip in the same conversation is told while the line is
/// full: the harness's words, as a background MCP tool's are.
const BUSY: &str = "Not run: `video_generate` was not started, because this conversation \
                    already has as much background work waiting as it may. Try again after one \
                    of those lands.";

/// What the machine is asked before a clip may start. A trait so tests can
/// stand in for the machine, whose memory and calls they cannot set.
pub trait Gates: Send + Sync {
    /// Whether a voice call is live now; an error when it cannot be ruled out.
    fn voice_live(&self) -> Result<bool>;
    /// `None` when `need` bytes are admitted now, else why not.
    fn admit(&self, need: u64) -> Option<String>;
}

/// The machine's own gates: the voice-live reading and the shared memory
/// floor (`[system]`).
pub struct MachineGates {
    pub floor: crate::system::floor::SystemConfig,
}

impl Gates for MachineGates {
    fn voice_live(&self) -> Result<bool> {
        crate::system::voice_live()
    }

    fn admit(&self, need: u64) -> Option<String> {
        let series = crate::system::db_path().ok();
        crate::system::floor::admit_now(need, &self.floor, series.as_deref()).sentence("a clip")
    }
}

/// The `video_generate` tool.
#[derive(Clone)]
pub struct VideoGenerate {
    cfg: VideoConfig,
    wire: Arc<Client>,
    /// The mecha home: the clip records (`video/jobs/`), the render turn,
    /// the owner's voice map and the chats' kept settings. `None` when it
    /// cannot be resolved; then there is no turn file to take, and a clip is
    /// refused rather than rendered beside another.
    home: Option<PathBuf>,
    library_dir: Option<PathBuf>,
    personas_dir: Option<PathBuf>,
    /// `[voice] voices_dir`: `<name>.wav` for each voice.
    voices_dir: Option<PathBuf>,
    /// Who "self" is in a persona's chat.
    self_as: Option<PersonaSelf>,
    /// Whether a chat's clips keep its last settings: a chat's surface only,
    /// never an eval or a replay, where an arm measures what it names.
    chat_settings: bool,
    gates: Arc<dyn Gates>,
    /// How often a render's state is asked for, and the gates re-read.
    poll: Duration,
    wait_step: Duration,
}

impl VideoGenerate {
    /// Refuses a configuration whose server is not on this machine.
    pub fn new(cfg: VideoConfig) -> Result<Self> {
        let wire = Client::new(cfg.loopback_url()?, "the video server")?;
        Ok(VideoGenerate {
            cfg,
            wire: Arc::new(wire),
            home: crate::work::mecha_home().ok(),
            library_dir: crate::imagelib::Library::default_dir().ok(),
            personas_dir: crate::persona::Store::default_dir().ok(),
            voices_dir: None,
            self_as: None,
            chat_settings: false,
            gates: Arc::new(MachineGates {
                floor: crate::system::floor::SystemConfig::default(),
            }),
            poll: Duration::from_secs(3),
            wait_step: Duration::from_secs(10),
        })
    }

    /// Admit clips against the operator's `[system]` floor.
    pub fn with_floor(mut self, floor: crate::system::floor::SystemConfig) -> Self {
        self.gates = Arc::new(MachineGates { floor });
        self
    }

    /// Where the voice library's `<name>.wav` files are.
    pub fn with_voices_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.voices_dir = dir;
        self
    }

    /// A chat's clips keep its last `turbo`, `seconds`, `size` and `aspect`.
    pub fn with_chat_settings(mut self) -> Self {
        self.chat_settings = true;
        self
    }

    /// Resolve characters against this library instead of the mecha home's;
    /// `None` reads no library at all, as with the `library` feature off.
    pub fn with_library_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.library_dir = dir;
        self
    }

    /// Read personas' characters and voices from this store; `None` reads
    /// none, as with the `personas` feature off.
    pub fn with_personas_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.personas_dir = dir;
        self
    }

    #[cfg(test)]
    fn for_test(mut self, home: &Path, gates: Arc<dyn Gates>, poll: Duration) -> Self {
        self.home = Some(home.to_path_buf());
        self.library_dir = Some(home.join("imagelib"));
        self.personas_dir = Some(home.join("personas"));
        self.gates = gates;
        self.poll = poll;
        self.wait_step = poll;
        if let Some(wire) = Arc::get_mut(&mut self.wire) {
            wire.poll = poll;
        }
        self
    }

    fn video_dir(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|h| h.join("video"))
    }

    fn jobs_dir(&self) -> Option<PathBuf> {
        self.video_dir().map(|d| d.join("jobs"))
    }

    /// Where this chat's kept settings live: beside the clip records,
    /// outside the chat's own folder, which is the model's workspace, so the
    /// model can neither list nor rewrite them.
    fn settings_path(&self, ctx: &ToolCtx) -> Option<PathBuf> {
        if !self.chat_settings {
            return None;
        }
        let home = self.home.as_ref()?.canonicalize().ok()?;
        let chat = ctx.workspace.canonicalize().ok()?;
        let relative = chat.strip_prefix(&home).unwrap_or(&chat);
        Some(
            self.video_dir()?
                .join("chat-settings")
                .join(format!("{}.json", args::settings_name(relative))),
        )
    }
}

/// A picture to send, and for a library character the description the
/// library holds for it, pasted beside its pointer as `image_generate`'s
/// compiler does: the portrait carries the face, the words carry what a
/// portrait cannot show.
struct Picture {
    bytes: Vec<u8>,
    ext: String,
    /// The picture is the library's face crop of the portrait.
    face_only: bool,
    character: Option<String>,
    description: Option<String>,
}

/// A voice a clip carries: which reference it speaks for, and its clip
/// when the voice library holds one.
struct Voice {
    reference: usize,
    character: String,
    name: String,
    clip: Option<Vec<u8>>,
}

/// Everything a clip is, decided before its job is handed over.
struct Plan {
    mode: graph::Mode,
    prompt: String,
    width: u32,
    height: u32,
    frames: u32,
    /// The length the call asked for: what a chat keeps, never the
    /// rendered length `frames` snaps it up to.
    seconds: f64,
    size: graph::Size,
    aspect: String,
    turbo: bool,
    seed: u64,
    references: Vec<Picture>,
    references_asked: Vec<String>,
    first: Option<Picture>,
    first_asked: Option<String>,
    last: Option<Picture>,
    last_asked: Option<String>,
    voices: Vec<Voice>,
    /// Each reference's `<Audio j>` slot, when it has a voice here.
    audio: Vec<Option<usize>>,
    notes: Vec<String>,
    settings: Option<PathBuf>,
}

impl VideoGenerate {
    /// The call's arguments, filled in and resolved: every name and path
    /// checked before any machine resource is taken, so a bad one costs
    /// nothing.
    fn plan(&self, input: &Value, ctx: &ToolCtx) -> Result<Plan> {
        let settings = self.settings_path(ctx);
        let last = settings
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let (input, kept) = args::with_chat_settings(input, last.as_ref());
        let (input, mut notes) = args::with_scene(&input)?;
        notes.extend(kept);
        let library = self
            .library_dir
            .as_ref()
            .map(|d| crate::imagelib::Library::load(d).0)
            .unwrap_or_default();
        let characters: Vec<(String, Option<String>)> = library
            .approved()
            .filter(|e| e.kind == crate::imagelib::Kind::Character)
            .map(|e| {
                let text = e.text.trim().trim_end_matches('.').to_string();
                (e.name.clone(), (!text.is_empty()).then_some(text))
            })
            .collect();
        // The persona's own character, only while the library holds it
        // approved with a portrait, as `image_generate`'s "self" is: one the
        // owner unapproved or removed would otherwise be every call's lone
        // reference, and refuse every call (review of #733).
        let character = self
            .self_as
            .as_ref()
            .and_then(|s| s.character.as_deref())
            .map(args::library_key)
            .filter(|c| {
                library
                    .get(crate::imagelib::Kind::Character, c)
                    .is_some_and(|e| {
                        e.status == crate::imagelib::Status::Approved && e.portrait.is_some()
                    })
            });
        let input = args::persona_defaults(&input, character.as_deref());
        let names: Vec<String> = characters.iter().map(|(n, _)| n.clone()).collect();
        let named = args::named_in(input["prompt"].as_str().unwrap_or(""), &names);
        // Keyed first, so a reference the model wrote as `Maya` and the
        // name the prompt mentions are one reference, not two.
        let input = args::with_named(&args::keyed_names(&input), &named);
        // A clip without references describes, in words, the persona and
        // everyone the prompt names. Reserved: the persona and every named
        // character become references, so this fills only when `references`
        // held nothing usable; the private server's path, kept for parity.
        let mut in_words: Vec<(String, String)> = Vec::new();
        for n in character.iter().chain(named.iter()) {
            if let Some((_, Some(text))) = characters.iter().find(|(name, _)| name == n) {
                if !in_words.iter().any(|(d, _)| d == n) {
                    in_words.push((n.clone(), text.clone()));
                }
            }
        }

        let prompt = input["prompt"]
            .as_str()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| anyhow!("`prompt` is required"))?;
        let seconds = input["seconds"].as_f64().unwrap_or(5.0);
        if !(graph::MIN_SECONDS..=graph::MAX_SECONDS).contains(&seconds) {
            bail!(
                "`seconds` must be between {} and {}",
                graph::MIN_SECONDS,
                graph::MAX_SECONDS
            );
        }
        let aspect = input["aspect"].as_str().unwrap_or("16:9").to_string();
        // Full by default (the owner, 2026-10-14): at ~480p a distant face
        // has too few pixels to hold through a walk.
        let size = input["size"].as_str().unwrap_or("full");
        let size = graph::Size::parse(size)
            .ok_or_else(|| anyhow!("`size` is draft or full, not {size}"))?;
        let (width, height) = graph::canvas(&aspect, size)
            .ok_or_else(|| anyhow!("`aspect` is one of {:?}", graph::ASPECTS))?;
        let turbo = input["turbo"].as_bool().unwrap_or(true);
        let seed = input["seed"].as_u64().unwrap_or_else(fresh_seed);
        let references_asked: Vec<String> = input["references"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if references_asked.len() > graph::MAX_REFERENCES {
            bail!("at most {} references", graph::MAX_REFERENCES);
        }
        let first_asked = input["first_frame"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let last_asked = input["last_frame"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let mode = if references_asked.is_empty() {
            graph::Mode::Frames
        } else {
            graph::Mode::References
        };

        let references = references_asked
            .iter()
            .map(|r| self.picture(r, &library, ctx))
            .collect::<Result<Vec<_>>>()?;
        let first = first_asked
            .as_deref()
            .map(|f| self.picture(f, &library, ctx))
            .transpose()?;
        let last = last_asked
            .as_deref()
            .map(|f| self.picture(f, &library, ctx))
            .transpose()?;

        // Voices only in references mode (H3's frames checkpoint takes none),
        // and only those the voice library holds, at most three, in order.
        let own_voice = self
            .self_as
            .as_ref()
            .and_then(|s| s.voice.as_deref())
            .map(str::trim)
            .filter(|v| crate::config::valid_voice_name(v));
        let own = character.as_deref().zip(own_voice);
        let voices: Vec<Voice> = if mode == graph::Mode::References {
            let owner = self.owner_voices();
            let personas = self.persona_voices();
            args::voices_for(&references_asked, own, &owner, &personas)
                .into_iter()
                .map(|v| Voice {
                    clip: self.voice_clip(&v.name),
                    reference: v.reference,
                    character: v.character,
                    name: v.name,
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut audio: Vec<Option<usize>> = vec![None; references.len()];
        let mut heard = 0;
        for v in &voices {
            if v.clip.is_some() && heard < graph::MAX_VOICES {
                heard += 1;
                audio[v.reference] = Some(heard);
            }
        }

        // The prompt in the shape H3 was trained on (`compose`): the model's
        // timeline and sound, with the references, voices and keyframes
        // written here. A clip with no references carries no portrait, so the
        // persona and the characters the prompt names are described in words.
        let parts = compose::Parts {
            prompt,
            soundscape: input["soundscape"].as_str(),
            music: input["music"].as_str(),
            seconds: graph::frames(seconds) as f64 / graph::FPS as f64,
        };
        let prompt = match mode {
            graph::Mode::References => {
                let people: Vec<compose::Person> = references
                    .iter()
                    .zip(&audio)
                    .map(|(r, a)| compose::Person {
                        name: r.character.clone(),
                        description: r.description.clone(),
                        audio: *a,
                        face_only: r.face_only,
                    })
                    .collect();
                compose::references(&parts, &people, (first.is_some(), last.is_some()))
            }
            graph::Mode::Frames => {
                compose::frames(&parts, first.is_some(), last.is_some(), &in_words)
            }
        };
        Ok(Plan {
            mode,
            prompt,
            width,
            height,
            frames: graph::frames(seconds),
            seconds,
            size,
            aspect,
            turbo,
            seed,
            references,
            references_asked,
            first,
            first_asked,
            last,
            last_asked,
            voices,
            audio,
            notes,
            settings,
        })
    }

    /// A character-library name, or a picture in this chat's folder (through
    /// the path jail).
    fn picture(
        &self,
        given: &str,
        library: &crate::imagelib::Library,
        ctx: &ToolCtx,
    ) -> Result<Picture> {
        if args::looks_like_name(given) {
            let entry = library
                .get(crate::imagelib::Kind::Character, given)
                .filter(|e| e.status == crate::imagelib::Status::Approved && e.portrait.is_some())
                .ok_or_else(|| {
                    anyhow!("no character named `{given}` with a portrait in the image library")
                })?;
            // The portrait under its hash check, then the library's face
            // crop of it when one was made: a whole portrait carried its
            // outfit into every clip, even when the prompt said not to; the
            // face alone did not (the owner's experiment, 2026-10-13). A
            // crop under MIN_FACE_CROP gives way to the portrait.
            let (bytes, ext) = library.read_portrait(entry)?;
            let crop = entry
                .portrait
                .as_deref()
                .map(|blob| crate::face::cache_paths(library.dir(), blob).0)
                .and_then(|p| std::fs::read(p).ok())
                .filter(|crop| big_enough(crop));
            let description =
                Some(entry.text.trim().trim_end_matches('.').to_string()).filter(|t| !t.is_empty());
            return Ok(match crop {
                Some(crop) => Picture {
                    bytes: crop,
                    ext: "png".into(),
                    face_only: true,
                    character: Some(given.to_string()),
                    description,
                },
                None => Picture {
                    bytes,
                    ext: ext.to_string(),
                    face_only: false,
                    character: Some(given.to_string()),
                    description,
                },
            });
        }
        let path = ctx.resolve(given).map_err(|e| {
            anyhow!("{given}: {e:#}; give a library name or a picture from this chat")
        })?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            bail!("{given} is not a png, jpeg or webp image");
        }
        Ok(Picture {
            bytes: read_capped(&path).with_context(|| format!("no image at {given}"))?,
            ext,
            face_only: false,
            character: None,
            description: None,
        })
    }

    /// The owner's own character-to-voice map, `<home>/video/voices.toml`
    /// (`maya = "maya-low"`): for a character no persona links, the owner
    /// among them. Plain names only; a bad line is skipped.
    fn owner_voices(&self) -> HashMap<String, String> {
        let text = self
            .video_dir()
            .and_then(|d| std::fs::read_to_string(d.join("voices.toml")).ok())
            .unwrap_or_default();
        let table: toml::Table = toml::from_str(&text).unwrap_or_default();
        table
            .into_iter()
            .filter_map(|(c, v)| Some((c, v.as_str()?.to_string())))
            .filter(|(_, v)| crate::config::valid_voice_name(v))
            .collect()
    }

    /// Each approved persona's library character and voice; the first
    /// persona linking a character speaks for it.
    fn persona_voices(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let Some(dir) = &self.personas_dir else {
            return map;
        };
        let store = crate::persona::Store::load(dir);
        for p in store.all() {
            if p.state.status != crate::persona::Status::Approved {
                continue;
            }
            let (Some(c), Some(v)) = (&p.settings.character, &p.settings.voice) else {
                continue;
            };
            if crate::config::valid_voice_name(v) {
                map.entry(args::library_key(c))
                    .or_insert_with(|| v.to_string());
            }
        }
        map
    }

    /// The voice's clip, `<voices_dir>/<name>.wav`, proved to be inside the
    /// folder. `None` when there is no such clip: the clip is then made with
    /// a voice of H3's own, and the result says so.
    fn voice_clip(&self, name: &str) -> Option<Vec<u8>> {
        if !crate::config::valid_voice_name(name) {
            return None;
        }
        let dir = self.voices_dir.as_ref()?.canonicalize().ok()?;
        let clip = dir.join(format!("{name}.wav")).canonicalize().ok()?;
        (clip.starts_with(&dir) && clip.is_file())
            .then(|| read_capped(&clip).ok())
            .flatten()
    }
}

fn read_capped(path: &Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_INPUT_BYTES {
        bail!(
            "{} is {} MB, over the {} MB a clip reads",
            path.display(),
            len / (1024 * 1024),
            MAX_INPUT_BYTES / (1024 * 1024)
        );
    }
    Ok(std::fs::read(path)?)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A clip's id, `v<seconds>-<hex>`: the private server's shape, so records
/// from both read alike.
fn fresh_id() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let tail = uuid::Uuid::new_v4().as_u128() as u32 & 0xffff;
    format!("v{}-{tail:04x}", t.as_secs())
}

fn fresh_seed() -> u64 {
    (uuid::Uuid::new_v4().as_u128() as u64) >> 16
}

fn minutes(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else {
        format!("{:.1} min", secs as f64 / 60.0)
    }
}

/// What a clip was rendered at, as a result says it: the owner asked for
/// "no turbo" in 9 of 14 clips that then rendered turbo, and nothing said so.
fn quality(record: &Value) -> String {
    let steps = record["steps"].as_u64().unwrap_or(0);
    let size = record["size"].as_str().unwrap_or("?");
    if record["turbo"].as_bool().unwrap_or(true) {
        format!("turbo ({steps} steps), {size} size")
    } else {
        format!("full quality ({steps} steps), {size} size")
    }
}

/// A clip's notes, said after a stop or a failure too: a setting kept from
/// the last clip is what the next attempt would repeat.
fn said_notes(notes: &[String]) -> String {
    if notes.is_empty() {
        String::new()
    } else {
        format!(". Notes: {}", notes.join(" "))
    }
}

/// How long a clip should take, in seconds, from this machine's own clips:
/// the median time per frame-step-megapixel of finished clips with as many
/// references (3 or more counted together) and the same turbo, then the
/// same turbo, then all, whichever is the narrowest with three clips; with
/// the count it came from. `None` with fewer than three finished clips.
/// Measured 2026-10-16 over 59 clips: a median error of 17%.
fn estimate(
    jobs: &Path,
    frames: u32,
    steps: u32,
    width: u32,
    height: u32,
    refs: usize,
    turbo: bool,
) -> Option<(u64, usize)> {
    let units = |f: f64, st: f64, w: f64, h: f64| f * st * w * h / 1e6;
    let mut all: Vec<(usize, bool, f64)> = Vec::new();
    for entry in std::fs::read_dir(jobs).ok()?.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(j) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if j.get("error").is_some_and(|e| !e.is_null()) || j["output"].as_str().is_none() {
            continue;
        }
        let (Some(c), Some(f), Some(fr), Some(st), Some(w), Some(h)) = (
            j["created"].as_u64(),
            j["finished"].as_u64(),
            j["frames"].as_f64(),
            j["steps"].as_f64(),
            j["width"].as_f64(),
            j["height"].as_f64(),
        ) else {
            continue;
        };
        let u = units(fr, st, w, h);
        if f <= c || u <= 0.0 {
            continue;
        }
        let n = j["references"].as_array().map_or(0, Vec::len).min(3);
        let t = j["turbo"].as_bool().unwrap_or(true);
        all.push((n, t, (f - c) as f64 / u));
    }
    let bucket = refs.min(3);
    let pick = |keep: &dyn Fn(&(usize, bool, f64)) -> bool| -> Vec<f64> {
        all.iter().filter(|c| keep(c)).map(|c| c.2).collect()
    };
    let mut rates = [
        pick(&|c| c.0 == bucket && c.1 == turbo),
        pick(&|c| c.1 == turbo),
        pick(&|_| true),
    ]
    .into_iter()
    .find(|r| r.len() >= 3)?;
    rates.sort_by(f64::total_cmp);
    let k = rates[rates.len() / 2];
    let secs = k * units(frames as f64, steps as f64, width as f64, height as f64);
    Some((secs.round() as u64, rates.len()))
}

/// The first `seconds` of a PCM WAV, header rewritten: ComfyUI encodes the
/// whole voice clip (MiniMax's own path caps it at the video's length), a
/// long one mumbles the first half-second, and the owner heard 5 s as
/// crisper than 15–20 s (2026-10-13). `None` for anything this cannot read
/// as a WAV with a `fmt ` before its `data`: the whole clip is sent then,
/// as the private server sent it when `ffmpeg` failed.
fn trim_wav(wav: &[u8], seconds: f64) -> Option<Vec<u8>> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(wav.get(at..at + 4)?.try_into().ok()?))
    };
    let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
    let mut pos = 12;
    let mut format: Option<(u32, u16)> = None;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let size = u32_at(pos + 4)? as usize;
        let body = pos + 8;
        let end = body.saturating_add(size).min(wav.len());
        if id == b"fmt " {
            if size < 16 || end - body < 16 {
                return None;
            }
            let byte_rate = u32_at(body + 8)?;
            let align = u16::from_le_bytes(wav[body + 12..body + 14].try_into().ok()?);
            if byte_rate == 0 || align == 0 {
                return None;
            }
            format = Some((byte_rate, align));
            out.extend_from_slice(&wav[pos..end]);
            if (end - body) % 2 == 1 {
                out.push(0);
            }
        } else if id == b"data" {
            let (byte_rate, align) = format?;
            let want = (seconds * byte_rate as f64) as usize;
            let keep = (want / align as usize * align as usize).min(end - body);
            out.extend_from_slice(b"data");
            out.extend_from_slice(&(keep as u32).to_le_bytes());
            out.extend_from_slice(&wav[body..body + keep]);
            if keep % 2 == 1 {
                out.push(0);
            }
            let riff = (out.len() - 8) as u32;
            out[4..8].copy_from_slice(&riff.to_le_bytes());
            return Some(out);
        }
        pos = body.saturating_add(size).saturating_add(size & 1);
    }
    None
}

/// A turn to render: a file created exclusively, holding the holder's pid,
/// removed when the clip is done. A holder that is gone frees it, so a crash
/// never wedges the line. The private server's file, so the two take turns
/// while both are installed.
#[derive(Debug)]
struct Turn(PathBuf);

impl Drop for Turn {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Take a stale turn out of the way, only if it is still the one read as
/// `held`. Two waiters behind one dead holder both find it stale; with a
/// plain remove, the second deleted the turn the first had just taken, and
/// both rendered (review of #733, pass 4). The turn is moved aside to a name
/// of this process's own first, so only one waiter moves any given file, and
/// what was moved is checked: a new holder's turn goes back where it was.
fn reclaim(path: &Path, held: &str) -> bool {
    let aside = path.with_extension(format!("stale.{}", std::process::id()));
    if std::fs::rename(path, &aside).is_err() {
        return false;
    }
    if std::fs::read_to_string(&aside).is_ok_and(|now| now == held) {
        let _ = std::fs::remove_file(&aside);
        return true;
    }
    // Back only if the name is still free: a link fails where a file is.
    let _ = std::fs::hard_link(&aside, path);
    let _ = std::fs::remove_file(&aside);
    false
}

/// Whether a process is alive: `kill(pid, 0)` reaches it, or exists but is
/// another user's.
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 sends nothing; it only asks whether the pid exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Sleep `step`, or end at once on a cancel.
async fn pause(step: Duration, cancel: &CancellationToken) -> std::result::Result<(), Failure> {
    tokio::select! {
        _ = cancel.cancelled() => Err(Failure::Cancelled),
        _ = tokio::time::sleep(step) => Ok(()),
    }
}

impl VideoGenerate {
    async fn wait_turn(
        &self,
        cancel: &CancellationToken,
        deadline: std::time::Instant,
    ) -> std::result::Result<Turn, Failure> {
        let dir = self.video_dir().ok_or_else(|| {
            anyhow!("the mecha home could not be found, so no turn could be taken")
        })?;
        std::fs::create_dir_all(&dir).context("making the video folder")?;
        let path = dir.join("render.turn");
        loop {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    // Owned before it is written: a failed write would
                    // otherwise leave a turn nothing removes, and an empty
                    // one is stale only after QUEUE_LIMIT.
                    let turn = Turn(path.clone());
                    writeln!(f, "{} {}", std::process::id(), now()).context("taking the turn")?;
                    return Ok(turn);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let held = std::fs::read_to_string(&path).unwrap_or_default();
                    let pid = held
                        .split_whitespace()
                        .next()
                        .and_then(|p| p.parse::<u32>().ok());
                    // A holder whose process is gone, or an unreadable turn
                    // older than the longest clip, is stale.
                    let gone = match pid {
                        Some(pid) => !alive(pid),
                        None => std::fs::metadata(&path)
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| t.elapsed().ok())
                            .is_some_and(|age| age > QUEUE_LIMIT),
                    };
                    // Reclaimed and taken at once; a reclaim that fails
                    // waits like a live holder, rather than spinning (review
                    // of #733).
                    if gone && reclaim(&path, &held) {
                        continue;
                    }
                }
                Err(e) => return Err(anyhow!(e).context("taking the render turn").into()),
            }
            if std::time::Instant::now() > deadline {
                return Err(anyhow!("waited three hours behind other clips; not started").into());
            }
            pause(self.wait_step, cancel).await?;
        }
    }

    /// Holding the turn, wait until the machine can take a clip: no voice
    /// call live, nothing else in the video server's queue (an experiment,
    /// say), and memory admitted. Each waits rather than refuses; a voice
    /// state that cannot be read refuses, since unknown is not "no".
    async fn wait_until_free(
        &self,
        size: graph::Size,
        cancel: &CancellationToken,
        deadline: std::time::Instant,
    ) -> std::result::Result<(), Failure> {
        loop {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            let late = std::time::Instant::now() > deadline;
            match self.gates.voice_live() {
                Err(e) => {
                    return Err(e.context("not starting a clip").into());
                }
                Ok(true) if late => {
                    return Err(anyhow!(
                        "a voice call stayed live for three hours; the clip was not started"
                    )
                    .into())
                }
                Ok(true) => {}
                Ok(false) => {
                    // A queue that cannot be read is busy, not idle: the
                    // gate waits rather than fails open (review of #733).
                    let queue = self.wire.get_json("queue").await.ok();
                    let busy = queue.is_none_or(|q| {
                        ["queue_running", "queue_pending"]
                            .iter()
                            .any(|k| q[k].as_array().is_some_and(|a| !a.is_empty()))
                    });
                    if !busy {
                        match self.gates.admit(memory_need(size)) {
                            None => return Ok(()),
                            Some(why) if late => {
                                return Err(anyhow!(
                                    "{why} Try a draft size, or again once a model has unloaded."
                                )
                                .into())
                            }
                            Some(_) => {}
                        }
                    } else if late {
                        return Err(anyhow!(
                            "the video server stayed busy for three hours; the clip was not started"
                        )
                        .into());
                    }
                }
            }
            pause(self.wait_step, cancel).await?;
        }
    }

    /// Every file and node the graph names is one the server has, asked
    /// before anything is uploaded: a missing one otherwise fails deep in a
    /// render, as a node validation error the model cannot act on. Returns
    /// the decoder to use: `[video] decoder` only where the server lists it.
    async fn preflight(&self, plan: &Plan) -> Result<Option<String>> {
        let mut fetched: HashMap<&'static str, Value> = HashMap::new();
        let decoder = match &self.cfg.decoder {
            Some(name) => self
                .lists(&mut fetched, "VAELoader", "vae_name", name)
                .await?
                .then(|| name.clone()),
            None => None,
        };
        let names: Vec<String> = Vec::new();
        let spec = graph::Spec {
            mode: plan.mode,
            prompt: "",
            width: plan.width,
            height: plan.height,
            frames: plan.frames,
            turbo: plan.turbo,
            seed: plan.seed,
            first_frame: None,
            last_frame: None,
            references: &names,
            reference_audios: &names,
            filename_prefix: "",
            decoder: decoder.as_deref(),
        };
        for need in graph::needs(&self.cfg, &spec) {
            if !self
                .lists(&mut fetched, need.node, need.input, &need.file)
                .await?
            {
                bail!(
                    "the video server has no `{}` for {}; the operator puts the model files on \
                     the server",
                    need.file,
                    need.node
                );
            }
        }
        let pinned = plan.first.is_some() || plan.last.is_some();
        for node in graph::h3_nodes(plan.mode, pinned) {
            let v = self.wire.get_json(&format!("object_info/{node}")).await?;
            if v.get(*node).is_none() {
                bail!(
                    "the video server's ComfyUI predates MiniMax H3 support (no {node}); update it"
                );
            }
        }
        Ok(decoder)
    }

    /// Whether the server's `node` offers `file` for `input`, each node's
    /// `/object_info` asked once per clip.
    async fn lists(
        &self,
        fetched: &mut HashMap<&'static str, Value>,
        node: &'static str,
        input: &str,
        file: &str,
    ) -> Result<bool> {
        if !fetched.contains_key(node) {
            let info = self.wire.get_json(&format!("object_info/{node}")).await?;
            fetched.insert(node, info);
        }
        Ok(fetched[node]
            .pointer(&format!("/{node}/input/required/{input}/0"))
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(file))))
    }

    /// The render, from the turn to the file in `videos/`: the chat's job.
    async fn render(
        &self,
        plan: Plan,
        ctx: &ToolCtx,
        cancel: &CancellationToken,
    ) -> std::result::Result<String, Failure> {
        // Clips queue rather than refuse (the owner, 2026-10-15: two persona
        // chats' clips failed with "already rendering"). One renders at a
        // time on this machine: the turn is held until this clip is done,
        // and a waiting clip still answers Stop.
        // One three-hour budget for all the waiting, the turn and the machine
        // together, so "waited three hours" is true (review of #733).
        let deadline = std::time::Instant::now() + QUEUE_LIMIT;
        let _turn = self.wait_turn(cancel, deadline).await?;
        match self.wire.await_server(Some(cancel), SERVER_WAIT).await {
            Err(Failure::Cancelled) => return Err(Failure::Cancelled),
            Err(Failure::Other(_)) => {
                return Err(anyhow!(
                    "the video server at {} is not answering; it is the operator's to start",
                    self.wire.base()
                )
                .into())
            }
            Ok(()) => {}
        }
        self.wait_until_free(plan.size, cancel, deadline).await?;
        let decoder = self.preflight(&plan).await?;

        let id = fresh_id();
        let name = |slot: &str, ext: &str| format!("mecha-video-{id}-{slot}.{ext}");
        let mut refs = Vec::new();
        for (i, p) in plan.references.iter().enumerate() {
            let label = plan.references_asked[i].clone();
            refs.push(
                self.wire
                    .upload_temp(&p.bytes, &name(&format!("ref{i}"), &p.ext), &label)
                    .await?,
            );
        }
        let first = match &plan.first {
            Some(p) => Some(
                self.wire
                    .upload_temp(&p.bytes, &name("first", &p.ext), "the opening frame")
                    .await?,
            ),
            None => None,
        };
        let last = match &plan.last {
            Some(p) => Some(
                self.wire
                    .upload_temp(&p.bytes, &name("last", &p.ext), "the closing frame")
                    .await?,
            ),
            None => None,
        };
        // Each voice trimmed to about a line's length.
        let trim = (plan.frames as f64 / graph::FPS as f64).clamp(2.0, VOICE_SECONDS);
        let mut voices = Vec::new();
        for (j, v) in plan
            .voices
            .iter()
            .filter(|v| plan.audio.get(v.reference).copied().flatten().is_some())
            .enumerate()
        {
            let Some(bytes) = &v.clip else { continue };
            let cut = trim_wav(bytes, trim);
            voices.push(
                self.wire
                    .upload_temp(
                        cut.as_deref().unwrap_or(bytes),
                        &name(&format!("voice{j}"), "wav"),
                        &format!("`{}`'s voice", v.name),
                    )
                    .await?,
            );
        }

        let prefix = format!("{OUTPUT_FOLDER}/{id}");
        let spec = graph::Spec {
            mode: plan.mode,
            prompt: &plan.prompt,
            width: plan.width,
            height: plan.height,
            frames: plan.frames,
            turbo: plan.turbo,
            seed: plan.seed,
            first_frame: first.as_deref(),
            last_frame: last.as_deref(),
            references: &refs,
            reference_audios: &voices,
            filename_prefix: &prefix,
            decoder: decoder.as_deref(),
        };
        let steps = spec.steps(&self.cfg);
        let jobs = self.jobs_dir();
        let eta = jobs.as_deref().and_then(|j| {
            estimate(
                j,
                plan.frames,
                steps,
                plan.width,
                plan.height,
                plan.references.len(),
                plan.turbo,
            )
        });
        let asked = uuid::Uuid::new_v4().to_string();
        let mut record = json!({
            "id": id, "prompt_id": asked, "created": now(), "mode": plan.mode.name(),
            "prompt": plan.prompt, "seconds": plan.frames as f64 / graph::FPS as f64,
            // What the call asked for, which a chat keeps: the rendered length
            // is snapped up to H3's frame grid, so keeping it drifted every
            // chat to 5.2 s and wedged one at 15.08 s, past the limit
            // (review of #733).
            "asked_seconds": plan.seconds,
            "frames": plan.frames, "width": plan.width, "height": plan.height,
            "aspect": plan.aspect, "size": plan.size.name(), "turbo": plan.turbo,
            "steps": steps, "seed": plan.seed,
            "decoder": decoder.clone().unwrap_or_else(|| self.cfg.vae.clone()),
            "references": plan.references_asked, "first_frame": plan.first_asked,
            "last_frame": plan.last_asked,
            "face_crops": plan.references.iter().map(|r| r.face_only).collect::<Vec<_>>(),
            "notes": plan.notes,
            "voices": plan.voices.iter().map(|v| json!({
                "character": v.character, "voice": v.name,
                "used": plan.audio.get(v.reference).copied().flatten().is_some(),
                "found": v.clip.is_some(),
            })).collect::<Vec<_>>(),
            "eta_s": eta.map(|(secs, _)| secs),
            "eta_from": eta.map(|(_, n)| n),
        });
        let write = |record: &Value| {
            if let Some(dir) = &jobs {
                if let Err(e) = write_record(dir, record) {
                    tracing::warn!("video_generate: the clip's record: {e:#}");
                }
            }
        };
        write(&record);
        let (status, body) = self
            .wire
            .post_json(
                "prompt",
                &json!({"prompt": graph::build(&self.cfg, &spec), "client_id": "mecha",
                        "prompt_id": asked}),
            )
            .await?;
        if !status.is_success() {
            let _ = self.wire.release().await;
            return Err(anyhow!(
                "the video server refused the clip ({status}): {}",
                clip(&body)
            )
            .into());
        }
        let id_of = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["prompt_id"].as_str().map(str::to_string))
            .unwrap_or(asked);
        record["prompt_id"] = json!(id_of);
        write(&record);

        let finished = self.wait_for(&id_of, cancel).await;
        record["finished"] = json!(now());
        let entry = match finished {
            Ok(entry) => entry,
            Err(f) => {
                let _ = self.wire.release().await;
                record["error"] = json!(match &f {
                    Failure::Cancelled => "stopped by the owner".to_string(),
                    Failure::Other(e) => format!("{e:#}"),
                });
                write(&record);
                return Err(f);
            }
        };
        let made = self.collect(&entry, &id, ctx).await;
        self.wire.forget(&id_of).await;
        // The memory goes back either way: the chat model lives in the same
        // pool, and a video model left resident starves it.
        let _ = self.wire.release().await;
        let saved = match made {
            Ok(saved) => saved,
            Err(e) => {
                record["error"] = json!(format!("{e:#}"));
                write(&record);
                return Err(e.into());
            }
        };
        record["output"] = json!(ctx.workspace.join(&saved));
        write(&record);
        // Kept only once a clip is made: a stopped or failed one is not what
        // the chat settled on.
        if let Some(path) = &plan.settings {
            let kept = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(path, args::kept_settings(&record).to_string()));
            if let Err(e) = kept {
                tracing::warn!("video_generate: keeping the chat's clip settings: {e}");
            }
        }
        Ok(result_text(&record, &saved, &plan))
    }

    /// The finished job's history entry, polled until the server writes it.
    async fn wait_for(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> std::result::Result<Value, Failure> {
        let deadline = std::time::Instant::now() + RENDER_LIMIT;
        let mut failed = 0u32;
        loop {
            if pause(self.poll, cancel).await.is_err() {
                // A Stop takes the clip out of the server's line, or
                // interrupts it.
                self.wire.take_off(id).await;
                self.wire.forget(id).await;
                return Err(Failure::Cancelled);
            }
            match self.wire.get_json(&format!("history/{id}")).await {
                Ok(h) => {
                    failed = 0;
                    if let Some(entry) = h.get(id) {
                        return Ok(entry.clone());
                    }
                }
                Err(e) => {
                    failed += 1;
                    if failed >= POLL_FAILURES {
                        self.wire.take_off(id).await;
                        return Err(e
                            .context("the video server stopped answering about the clip")
                            .into());
                    }
                }
            }
            if std::time::Instant::now() > deadline {
                self.wire.take_off(id).await;
                self.wire.forget(id).await;
                return Err(anyhow!("the clip did not finish within three hours").into());
            }
        }
    }

    /// The clip a finished entry names, fetched through `/view` and written
    /// new into the chat's `videos/`, never copied off the server by host
    /// path. Returns its workspace path.
    async fn collect(&self, entry: &Value, id: &str, ctx: &ToolCtx) -> Result<String> {
        let status = entry
            .pointer("/status/status_str")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if status != "success" {
            let messages: Vec<String> = entry
                .pointer("/status/messages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|m| m[0] == "execution_error")
                .map(|m| {
                    format!(
                        "{} in {}: {}",
                        m[1]["exception_type"].as_str().unwrap_or("error"),
                        m[1]["node_type"].as_str().unwrap_or("?"),
                        m[1]["exception_message"].as_str().unwrap_or("").trim()
                    )
                })
                .collect();
            bail!(
                "{}",
                if messages.is_empty() {
                    format!("the video server's status was {status}")
                } else {
                    clip(&messages.join("; "))
                }
            );
        }
        let file = entry["outputs"]
            .as_object()
            .into_iter()
            .flat_map(|o| o.values())
            .filter_map(Value::as_object)
            .flat_map(|o| o.values())
            .filter_map(Value::as_array)
            .flatten()
            .find(|f| f["filename"].as_str().is_some_and(|n| n.ends_with(".mp4")))
            .cloned()
            .ok_or_else(|| anyhow!("the video server reported success but saved no mp4"))?;
        let bytes = self.wire.view(&file, "the finished clip").await?;
        if bytes.get(4..8) != Some(b"ftyp") {
            bail!("the video server returned something that is not an mp4");
        }
        save_clip(ctx, id, &bytes).await
    }
}

/// Write the clip to `videos/<id>.mp4` in the workspace, created new and
/// never through a link, as `image_generate` writes a picture.
async fn save_clip(ctx: &ToolCtx, id: &str, bytes: &[u8]) -> Result<String> {
    use tokio::io::AsyncWriteExt;
    for n in 0..100u32 {
        let name = if n == 0 {
            format!("videos/{id}.mp4")
        } else {
            format!("videos/{id}-{n}.mp4")
        };
        let path = ctx.resolve(&name)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        match options.open(&path).await {
            Ok(mut file) => {
                file.write_all(bytes).await?;
                file.flush().await?;
                return Ok(name);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => bail!("cannot write {name}: {e}"),
        }
    }
    bail!("no free file name under videos/ for this clip")
}

fn write_record(dir: &Path, record: &Value) -> Result<()> {
    let id = record["id"].as_str().unwrap_or_default();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("`{id}` is not a clip id");
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{id}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// What a finished clip's result says: where it is, what it was rendered
/// at, how long it took, and anything the model should know — a voice not
/// found, the scene compiler's notes.
fn result_text(record: &Value, saved: &str, plan: &Plan) -> String {
    let took = minutes(
        record["finished"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(record["created"].as_u64().unwrap_or(0)),
    );
    // A voice asked for and not found is said, not silently dropped.
    let missing: Vec<String> = plan
        .voices
        .iter()
        .filter(|v| v.clip.is_none())
        .map(|v| format!("`{}`", v.name))
        .collect();
    // So is one found past H3's voice slots, which `plan` leaves out.
    let over: Vec<String> = plan
        .voices
        .iter()
        .filter(|v| v.clip.is_some())
        .skip(graph::MAX_VOICES)
        .map(|v| format!("`{}`", v.name))
        .collect();
    let mut tail = if missing.is_empty() {
        String::new()
    } else {
        format!(
            " No clip in the voice library for {}, so that speech is in a voice of the model's own.",
            missing.join(", ")
        )
    };
    if !over.is_empty() {
        tail.push_str(&format!(
            " H3 takes {} voices, so {} went unused and that speech is in a voice of the model's own.",
            graph::MAX_VOICES,
            over.join(", ")
        ));
    }
    if !plan.notes.is_empty() {
        tail.push_str(&format!(" Notes: {}", plan.notes.join(" ")));
    }
    let estimated = record["eta_s"]
        .as_u64()
        .map(|e| format!(" (estimated about {})", minutes(e)))
        .unwrap_or_default();
    format!(
        "video: {saved}\nA {:.1} s clip with sound, {}x{}, {}, {} mode, seed {}, made in {took}{estimated}. It is on the owner's screen; you have not seen it.{tail}",
        record["seconds"].as_f64().unwrap_or(0.0),
        record["width"],
        record["height"],
        quality(record),
        record["mode"].as_str().unwrap_or("?"),
        record["seed"],
    )
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string", "description": "Free-form timeline in H3's layout (video_prompt_guide explains it): open with one line on the look, then [Shot 1] with the framing, who is there by name with their clothes and places, the setting, and what happens in order including any camera move. Each later shot begins [Shot N] At mm:ss.mmm. A speaker is written Name (S1) says, <d>[English] words</d>. Budget roughly 2.5 spoken words for each second."},
            "scene": {
                "type": "object",
                "description": "Structured request, preferred over `prompt`: from it the tool writes H3's prompt, numbers who speaks, attaches voices and checks the timing. Send this or `prompt`.",
                "properties": {
                    "style": {"type": "string", "description": "Look and mood in one concrete line: medium, light, colour, pace. For instance 'Live-action, cinematic, calm and wry, cool overcast light'."},
                    "setting": {"type": "object", "properties": {"description": {"type": "string"}, "light": {"type": "string"}}, "description": "The place."},
                    "cast": {"type": "array", "items": {"type": "object", "properties": {
                        "name": {"type": "string", "description": "Library name of a character, or a few words describing anyone else."},
                        "wearing": {"type": "string", "description": "What they have on in this clip. Never leave it out: nothing of a portrait's outfit is kept."},
                        "doing": {"type": "string", "description": "Their activity when the clip starts."},
                        "notes": {"type": "string", "description": "Anything further about them for this clip."}
                    }, "required": ["name"]}},
                    "shots": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {
                        "at": {"type": "number", "description": "Second at which this shot begins; not for the first. Omitted, shots share the length evenly."},
                        "framing": {"type": "string", "description": "For instance 'A wide two-shot' or 'A close-up of Maya'."},
                        "camera": {"type": "string", "description": "For instance 'The camera holds still' or 'The camera tracks slowly to the left'."},
                        "action": {"type": "string", "description": "Prose for what happens during the shot, in sequence."}
                    }}},
                    "lines": {"type": "array", "items": {"type": "object", "properties": {
                        "who": {"type": "string"},
                        "says": {"type": "string", "description": "Just the spoken words; roughly 2.5 for each second of clip."},
                        "delivery": {"type": "string", "description": "The manner of speaking, for instance 'dryly, without looking up'."},
                        "shot": {"type": "integer", "minimum": 1, "description": "The shot it belongs to (1 when omitted)."}
                    }, "required": ["who", "says"]}},
                    "soundscape": {"type": "string"},
                    "music": {"type": "string"},
                    "aspect": {"type": "string", "enum": graph::ASPECTS},
                    "seconds": {"type": "number", "minimum": graph::MIN_SECONDS, "maximum": graph::MAX_SECONDS}
                },
                "required": ["shots"]
            },
            "soundscape": {"type": "string", "description": "Ambient and physical sound through the clip in one to four sentences, with no speech and no score."},
            "music": {"type": "string", "description": "Non-diegetic score in one to three sentences: instruments, tempo, dynamics. Leave out for silence."},
            "seconds": {"type": "number", "minimum": graph::MIN_SECONDS, "maximum": graph::MAX_SECONDS, "description": "Clip length from 4 to 15 s; 5 unless this chat's previous clip set another. Longer clips cost far more time."},
            "aspect": {"type": "string", "enum": graph::ASPECTS, "description": "16:9 unless given."},
            "size": {"type": "string", "enum": ["full", "draft"], "description": "full renders at H3's native 768p, where faces stay stable, and is used unless this chat's previous clip chose draft; draft is about 480p and quicker, for a fast try."},
            "turbo": {"type": "boolean", "description": "Few-step sampling, on unless this chat's previous clip turned it off. false runs all 20 steps: noticeably better faces and lip sync for roughly four times the wait. Send false whenever the owner asks for no turbo."},
            "references": {"type": "array", "items": {"type": "string"}, "maxItems": graph::MAX_REFERENCES, "description": "Pictures to keep a likeness from: library character names or picture paths. Library characters the prompt names are added automatically, each with portrait, stored description and voice, so list them only to add someone unnamed. In a persona's chat \"self\" means the persona, who is included when no one else is. Works together with first_frame and last_frame."},
            "first_frame": {"type": "string", "description": "A picture the clip must start on exactly, for continuity: one from this chat, such as images/….png, or a frame taken from an earlier clip. Characters in it still bring their portraits and voices."},
            "last_frame": {"type": "string", "description": "A picture the clip must finish on exactly."},
            "seed": {"type": "integer", "minimum": 0, "description": "Reproduce a clip exactly; omitted means random."}
        },
        "required": []
    })
}

#[async_trait]
impl Tool for VideoGenerate {
    // A literal, as `NAME` is: the read-only classification test reads the
    // registered name off this body.
    fn name(&self) -> &str {
        "video_generate"
    }

    /// Its calls answer `being made: …` and render as the chat's job.
    fn defers(&self) -> bool {
        true
    }

    /// Eligible for a persona (`docs/PERSONA-DESIGN.md` §3.3) on the same
    /// footing as `image_generate`: its request goes only to the loopback
    /// video server `[video]` names. What it reads beyond the chat — the
    /// image library's approved characters, the voice library, and which
    /// voice each persona speaks in — is shared with personas on purpose.
    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(Arc::new((*self).clone()))
    }

    /// The persona form, knowing who "self" is and the voice she speaks in.
    fn for_persona_as(self: Arc<Self>, who: &PersonaSelf) -> Option<Arc<dyn Tool>> {
        let mut form = (*self).clone();
        form.self_as = Some(who.clone());
        Some(Arc::new(form))
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn input_schema(&self) -> Value {
        input_schema()
    }

    /// Read-only in the approval gate's sense, as `image_generate` is: what
    /// it writes is a new file under `videos/` in the run's own workspace,
    /// never an existing one.
    fn read_only(&self) -> bool {
        true
    }

    /// Nothing private comes back (a path and what the clip was rendered
    /// at), nothing external, and nothing leaves: the destination is a
    /// loopback server the operator configured, which the schema cannot
    /// name another of.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let plan = match self.plan(&input, ctx) {
            Ok(plan) => plan,
            Err(e) => return Ok(ToolOutput::err(format!("Not made: {e:#}"))),
        };
        let cancel = CancellationToken::new();
        let mut job_ctx = ctx.clone();
        job_ctx.cancel = Some(cancel.clone());
        // Never the run's event sender: a job holding one would hold the
        // whole chat until the clip is done.
        job_ctx.events = None;
        let me = self.clone();
        let token = cancel.clone();
        let job = async move {
            let notes = plan.notes.clone();
            match me.render(plan, &job_ctx, &token).await {
                Ok(text) => ToolOutput::ok(text),
                Err(Failure::Cancelled) => {
                    ToolOutput::err(format!("not made: stopped{}", said_notes(&notes)))
                }
                Err(Failure::Other(e)) => {
                    ToolOutput::err(format!("not made: {e:#}{}", said_notes(&notes)))
                }
            }
        };
        let job = crate::jobs::DeferredJob::new(job, cancel, BUSY)
            .with_label(NAME)
            // A clip is not the picture the measured closing lines name.
            .with_closings(
                crate::agent::WORK_ON_ITS_WAY,
                crate::agent::WORK_STILL_RUNNING,
            );
        Ok(ToolOutput::deferred(
            format!(
                "{}`{NAME}` is running. Its result reaches the owner here when it is done.",
                crate::imagegen::BEING_MADE
            ),
            job,
        ))
    }
}

/// `video_prompt_guide`: the prompting guide, on request, so its words ride
/// in a conversation only when a clip is being written.
pub struct VideoPromptGuide;

#[async_trait]
impl Tool for VideoPromptGuide {
    fn name(&self) -> &str {
        "video_prompt_guide"
    }

    fn description(&self) -> &str {
        GUIDE_DESCRIPTION
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn read_only(&self) -> bool {
        true
    }

    /// Fixed text compiled in: nothing read, nothing sent.
    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(self)
    }

    async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        Ok(ToolOutput::ok(GUIDE))
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
