//! `POST /v1/mecha-direct`: a delivery direction for one sentence about to
//! be spoken (`mecha_core::voice_direction`, `docs/VOICE-BREEZE-DESIGN.md`).
//!
//! The worker asks once per sentence when its speech engine honours
//! `instructions`, and speaks without one on any answer but a line. Three
//! rules carry the design:
//!
//! - **It never touches the reply.** No slot, no slot lock, no host's
//!   `speak`: each of those barges in, and a direction is asked while the
//!   reply it belongs to is still streaming. The source pin
//!   `the_director_never_barges_in` holds this.
//! - **It runs on whichever model is loaded.** The router is held for the
//!   call and followed per call, never named from a cached binding — a
//!   request naming a model the router has swapped out loads it back. A
//!   switch in flight is a skip, never a wait: the sentence goes out
//!   undirected rather than late.
//! - **It is recorded, except in an incognito chat.** Every answer becomes a
//!   `Record::SpokenDirection` in the conversation's own transcript, written
//!   after the answer has gone. An incognito chat has no transcript by type,
//!   and this module writes no journal line naming one, its sentence or its
//!   direction (owner ruling, 2026-10-03: the director runs there, and
//!   nothing is kept).

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use mecha_core::session::{Record, Session};
use mecha_core::voice_direction::{self as vd, Cue, Directed, Outcome, Scene, SpokenDirection};
use serde_json::json;
use tokio::sync::watch;

use super::{write_json, Shared, VoiceStream};

/// What a hosted turn hands the facade for its directions: where they are
/// recorded, and who is speaking. Built by the host that owns the
/// conversation, because only it knows either.
#[derive(Default)]
pub struct DirectorSeed {
    /// The transcript directions are appended to. `None` for an incognito
    /// chat, by type (`Recording::kept`), which is what keeps them unwritten.
    pub transcript: Option<Arc<Session>>,
    /// A persona's identity; `None` for the assistant.
    pub character: Option<String>,
    /// The speaker's last reply before this turn.
    pub last_reply: Option<String>,
}

/// The last thing the speaker said, for the director's scene.
pub(crate) fn last_reply(messages: &[mecha_core::message::Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == mecha_core::message::Role::Assistant)
        .map(|m| m.text())
        .filter(|t| !t.trim().is_empty())
}

/// How one call to the director ended, before it is recorded.
#[derive(Debug, Clone)]
pub(crate) enum Asked {
    Line {
        line: String,
        model: String,
    },
    Empty {
        model: String,
    },
    Refused {
        model: String,
    },
    Failed {
        reason: String,
        model: Option<String>,
    },
    Timeout {
        model: Option<String>,
    },
    Skipped(&'static str),
}

impl Asked {
    fn outcome(&self) -> (Outcome, Option<String>, Option<String>, Option<String>) {
        // (outcome, direction, model, reason)
        match self {
            Asked::Line { line, model } => {
                (Outcome::Ok, Some(line.clone()), Some(model.clone()), None)
            }
            Asked::Empty { model } => (Outcome::Empty, None, Some(model.clone()), None),
            Asked::Refused { model } => (
                Outcome::Error,
                None,
                Some(model.clone()),
                Some("refused".into()),
            ),
            Asked::Failed { reason, model } => {
                (Outcome::Error, None, model.clone(), Some(reason.clone()))
            }
            Asked::Timeout { model } => (Outcome::Timeout, None, model.clone(), None),
            Asked::Skipped(why) => (Outcome::Skipped, None, None, Some((*why).to_string())),
        }
    }
}

/// One reply's directing state, per conversation key. Replaced wholesale
/// when the next request on the key arrives, so a late sentence from a
/// barged-in reply is directed against the newer turn — its record still
/// carries its own `context`, which is what a study joins on.
pub(crate) struct TurnDirection {
    turn: String,
    /// False for a turn the harness answered without a model (a spoken
    /// "yes" to a draft): everything spoken in it is harness speech.
    model_turn: bool,
    incognito: bool,
    scene: Scene,
    transcript: Option<Arc<Session>>,
    /// The opening's result, once it lands; `None` when none was started.
    opening: Option<watch::Receiver<Option<Asked>>>,
    state: StdMutex<TurnState>,
}

#[derive(Default)]
struct TurnState {
    directed: Vec<(String, String)>,
    /// What the harness itself said in this turn: offers, read-backs, the
    /// failure line. Spoken in the assistant's voice, never directed.
    harness: Vec<String>,
}

/// The registry, beside the slots rather than inside one: a hosted turn has
/// no slot here and still gets directed.
#[derive(Default)]
pub(crate) struct Directions {
    turns: StdMutex<HashMap<String, Arc<TurnDirection>>>,
}

fn normalise(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl TurnDirection {
    fn is_harness(&self, sentence: &str) -> bool {
        if !self.model_turn {
            return true;
        }
        let s = normalise(sentence);
        if s.is_empty() {
            return true;
        }
        // The persona's crisis pause speaks a fixed message as the reply.
        if normalise(mecha_core::persona::safety::SAFE_MESSAGE).contains(&s) {
            return true;
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.harness.iter().any(|h| normalise(h).contains(&s))
    }
}

/// Whether a conversation key (`chat:<id>` / `voice:<slot>`) names an
/// incognito chat. The key alone decides, as `stamp_presence` decides.
pub(crate) fn names_incognito(key: &str) -> bool {
    key.strip_prefix("chat:")
        .is_some_and(crate::commands::serve::incognito::is_incognito_key)
}

impl Directions {
    /// A request arrived on `key`: whatever it turns out to be, the previous
    /// turn's state is done with. Harness-only until [`model_turn`] says a
    /// model is answering.
    ///
    /// [`model_turn`]: Directions::model_turn
    pub(crate) fn begin(&self, key: &str, turn: &str, utterance: &str) {
        let entry = TurnDirection {
            turn: turn.to_string(),
            model_turn: false,
            incognito: names_incognito(key),
            scene: Scene {
                utterance: utterance.to_string(),
                ..Default::default()
            },
            transcript: None,
            opening: None,
            state: StdMutex::default(),
        };
        self.lock().insert(key.to_string(), Arc::new(entry));
    }

    /// A model is answering this turn: set the scene, and when the worker
    /// said its engine takes directions, start the opening now — from the
    /// owner's words, while the reply's first sentence is still being
    /// written — so the first sentence's wait overlaps the model's.
    pub(crate) fn model_turn(
        &self,
        shared: &Arc<Shared>,
        key: &str,
        turn: &str,
        utterance: &str,
        seed: DirectorSeed,
        start_opening: bool,
    ) {
        let scene = Scene {
            character: seed.character,
            voice: None,
            last_reply: seed.last_reply,
            utterance: utterance.to_string(),
        };
        let opening = start_opening.then(|| {
            let (tx, rx) = watch::channel(None);
            let user = vd::prompt(&scene, &[], Cue::Opening);
            let shared = Arc::clone(shared);
            shared.handlers.clone().spawn(async move {
                let asked = ask(&shared, &user, vd::OPENING_DEADLINE).await;
                let _ = tx.send(Some(asked));
            });
            rx
        });
        let entry = TurnDirection {
            turn: turn.to_string(),
            model_turn: true,
            incognito: names_incognito(key),
            scene,
            transcript: seed.transcript,
            opening,
            state: StdMutex::default(),
        };
        self.lock().insert(key.to_string(), Arc::new(entry));
    }

    /// The harness is about to say `text` in turn `turn`.
    pub(crate) fn note_harness(&self, turn: &str, text: &str) {
        let turns = self.lock();
        for entry in turns.values().filter(|e| e.turn == turn) {
            entry
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .harness
                .push(text.to_string());
        }
    }

    fn get(&self, key: &str) -> Option<Arc<TurnDirection>> {
        self.lock().get(key).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<TurnDirection>>> {
        self.turns.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One call to the director on the model the router has loaded: held for
/// the call, followed for the call, bounded by `deadline` and by shutdown.
///
/// TODO(single slot): on a preset served with `parallel = 1` this call
/// queues behind the reply still being written, and can evict the chat's
/// slot cache. Whether the loaded preset has one slot is not something the
/// binding holds (`follow::Bound` carries the config, not the router's
/// preset), and asking the router per sentence is the probe this module
/// must not make, so nothing skips on it yet: a `single_slot` skip needs the
/// follower to keep the preset's slot count when it observes the router.
async fn ask(shared: &Arc<Shared>, user: &str, deadline: Duration) -> Asked {
    // Held, never waited for: a switch in flight is a skip.
    let held = match shared.follower.try_hold("voice direction") {
        Ok(Ok(held)) => held,
        Ok(Err(_switch)) => return Asked::Skipped("switching"),
        Err(e) => {
            return Asked::Failed {
                reason: first_line(&format!("{e:#}")),
                model: None,
            }
        }
    };
    let call = async {
        let bound = shared.follower.follow().await?;
        let model = bound.model.clone();
        let directed = vd::direct(bound.agent.provider(), &model, user).await?;
        anyhow::Ok((directed, model))
    };
    let result = tokio::select! {
        r = tokio::time::timeout(deadline, call) => r,
        _ = shared.stopping.cancelled() => return Asked::Skipped("shutdown"),
    };
    drop(held);
    match result {
        Err(_) => Asked::Timeout {
            model: Some(shared.follower.current().model.clone()),
        },
        Ok(Err(e)) => Asked::Failed {
            reason: first_line(&format!("{e:#}")),
            model: None,
        },
        Ok(Ok((Directed::Line(line), model))) => Asked::Line { line, model },
        Ok(Ok((Directed::Empty, model))) => Asked::Empty { model },
        Ok(Ok((Directed::Refused, model))) => Asked::Refused { model },
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(200).collect()
}

#[derive(serde::Deserialize)]
struct Ask {
    session: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    index: u32,
    sentence: String,
    #[serde(default)]
    voice: Option<String>,
}

/// `POST /v1/mecha-direct`. Always a 200 with `{direction, outcome}` for a
/// well-formed ask: the worker's only decision is whether a line came back.
pub(crate) async fn mecha_direct(
    stream: &mut VoiceStream,
    shared: &Arc<Shared>,
    body: &[u8],
) -> anyhow::Result<()> {
    let asked_at = Instant::now();
    let Ok(ask) = serde_json::from_slice::<Ask>(body) else {
        return write_json(stream, 400, &json!({"error": "invalid direction request"})).await;
    };
    let sentence = ask.sentence.trim().to_string();
    let Some(entry) = shared.directions.get(&ask.session) else {
        return write_json(
            stream,
            200,
            &json!({"direction": null, "outcome": "skipped", "reason": "no_turn"}),
        )
        .await;
    };

    let mut opening = false;
    let (answer, digest) = if entry.is_harness(&sentence) {
        (Asked::Skipped("harness"), None)
    } else if ask.index == 0 && entry.opening.is_some() {
        opening = true;
        let mut rx = entry.opening.clone().expect("checked above");
        let landed = tokio::time::timeout(vd::FIRST_SENTENCE_DEADLINE, async {
            loop {
                if let Some(a) = rx.borrow().clone() {
                    return a;
                }
                if rx.changed().await.is_err() {
                    return Asked::Failed {
                        reason: "the opening was dropped".into(),
                        model: None,
                    };
                }
            }
        })
        .await;
        let digest = vd::digest(&vd::prompt(&entry.scene, &[], Cue::Opening));
        (
            landed.unwrap_or(Asked::Timeout { model: None }),
            Some(digest),
        )
    } else {
        let directed = entry
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .directed
            .clone();
        let user = vd::prompt(&entry.scene, &directed, Cue::Sentence(&sentence));
        let deadline = if ask.index == 0 {
            vd::FIRST_SENTENCE_DEADLINE
        } else {
            vd::SENTENCE_DEADLINE
        };
        let digest = vd::digest(&user);
        (ask_director(shared, &user, deadline).await, Some(digest))
    };

    if let Asked::Line { line, .. } = &answer {
        entry
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .directed
            .push((sentence.clone(), line.clone()));
    }
    let (outcome, direction, model, reason) = answer.outcome();
    let latency_ms = asked_at.elapsed().as_millis() as u64;
    let written = write_json(
        stream,
        200,
        &json!({"direction": direction, "outcome": outcome, "reason": reason}),
    )
    .await;

    // After the answer has gone: the record must never cost the sentence.
    if entry.incognito {
        return written;
    }
    if let Some(transcript) = &entry.transcript {
        let record = SpokenDirection {
            ts: Some(chrono::Utc::now()),
            turn: entry.turn.clone(),
            context: ask.context,
            index: ask.index,
            sentence,
            opening,
            direction,
            model,
            latency_ms,
            outcome,
            reason,
            voice: ask.voice,
            prompt_digest: digest,
        };
        if let Err(e) = transcript.append(&Record::SpokenDirection(record)) {
            tracing::debug!("a spoken direction was not recorded: {e:#}");
        }
    }
    tracing::debug!(?outcome, latency_ms, index = ask.index, "voice direction");
    written
}

/// The per-sentence call, behind one name so the source pin below can see
/// every path a direction is asked through.
async fn ask_director(shared: &Arc<Shared>, user: &str, deadline: Duration) -> Asked {
    ask(shared, user, deadline).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No path from a direction request may barge in on the reply it
    /// belongs to.
    #[test]
    fn the_director_never_barges_in() {
        let src = include_str!("direct.rs");
        let body = src.split("\n#[cfg(test)]").next().unwrap();
        for forbidden in ["take_slot(", ".speak(", "slots.lock()", ".enter("] {
            assert!(
                !body.contains(forbidden),
                "the director must not call {forbidden}"
            );
        }
    }

    /// An incognito chat's direction leaves no record and no journal line
    /// that names it, its sentence or its direction.
    #[test]
    fn an_incognito_direction_is_neither_recorded_nor_traced() {
        let src = include_str!("direct.rs");
        let body = src.split("\n#[cfg(test)]").next().unwrap();
        let handler = body
            .split("pub(crate) async fn mecha_direct(")
            .nth(1)
            .expect("the handler");
        let gate = handler
            .find("if entry.incognito {")
            .expect("the incognito gate");
        let append = handler.find(".append(").expect("the record");
        let first_trace = handler.find("tracing::").expect("a journal line");
        assert!(gate < append && gate < first_trace, "the gate comes first");
        // And no journal line carries the words, a direction or a voice.
        for (at, _) in handler.match_indices("tracing::") {
            let statement = handler[at..].split(';').next().unwrap();
            for text in [
                "sentence",
                "direction)",
                "%direction",
                "?direction",
                "ask.voice",
                "ask.session",
                "scene",
            ] {
                assert!(!statement.contains(text), "{statement} names {text}");
            }
        }
    }

    #[test]
    fn harness_speech_is_never_directed() {
        let d = Directions::default();
        d.begin("voice:a", "t1", "yes");
        assert!(d.get("voice:a").unwrap().is_harness("Sent."));
        let entry = TurnDirection {
            turn: "t2".into(),
            model_turn: true,
            incognito: false,
            scene: Scene::default(),
            transcript: None,
            opening: None,
            state: StdMutex::default(),
        };
        d.lock().insert("voice:b".into(), Arc::new(entry));
        d.note_harness("t2", " Shall I send the draft to Ada now?");
        let b = d.get("voice:b").unwrap();
        assert!(b.is_harness("Shall I send the draft  to Ada now?"));
        assert!(!b.is_harness("That sounds lovely."));
        let pause = mecha_core::persona::safety::SAFE_MESSAGE;
        let first = pause.split(". ").next().unwrap();
        assert!(b.is_harness(first));
    }

    /// The director's model: one fixed line, every request kept.
    struct Directs {
        seen: Arc<StdMutex<Vec<mecha_core::message::CompletionRequest>>>,
    }

    #[async_trait::async_trait]
    impl mecha_core::provider::Provider for Directs {
        fn id(&self) -> &str {
            "local"
        }
        fn default_model(&self) -> &str {
            "loaded-model"
        }
        async fn complete(
            &self,
            req: &mecha_core::message::CompletionRequest,
            _: Option<&mecha_core::provider::StreamSink>,
        ) -> anyhow::Result<mecha_core::message::CompletionResponse> {
            self.seen.lock().unwrap().push(req.clone());
            Ok(mecha_core::message::CompletionResponse {
                message: mecha_core::message::Message::assistant(vec![
                    mecha_core::message::Block::text("Warm, unhurried, smiling."),
                ]),
                stop_reason: mecha_core::message::StopReason::EndTurn,
                usage: Default::default(),
                refusal: None,
                model: "loaded-model".into(),
                malformed_tool_args: 0,
            })
        }
    }

    type Seen = Arc<StdMutex<Vec<mecha_core::message::CompletionRequest>>>;

    fn facade(home: &crate::testenv::HomeGuard) -> (super::super::Facade, Seen) {
        let seen: Seen = Arc::default();
        let config = mecha_core::config::Config::default();
        let agent = mecha_core::agent::Agent::new(
            Box::new(Directs { seen: seen.clone() }),
            mecha_core::tool::Registry::new(),
            Arc::new(mecha_core::tool::ModeApprover {
                mode: mecha_core::config::PermissionMode::ReadOnly,
            }),
            mecha_core::tool::ToolCtx::default(),
            config.agent.clone(),
            None,
        )
        .unwrap();
        let follower = crate::follow::Follower::fixed(agent, "local", "loaded-model", config);
        let facade = super::super::Facade::new(
            Arc::new(follower),
            home.dir.join("outbox"),
            None,
            super::super::Mount::default(),
        )
        .unwrap();
        (facade, seen)
    }

    fn transcript(home: &crate::testenv::HomeGuard, id: &str) -> Arc<Session> {
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(
            Session::create(
                &dir,
                mecha_core::session::SessionMeta {
                    id: id.into(),
                    created_at: chrono::Utc::now(),
                    provider: "local".into(),
                    model: "loaded-model".into(),
                    workspace: home.dir.clone(),
                    title: None,
                    kind: None,
                },
            )
            .unwrap(),
        )
    }

    /// The handler, over a real socket, as the worker reaches it.
    async fn ask_over_socket(shared: &Arc<Shared>, body: serde_json::Value) -> serde_json::Value {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::clone(shared);
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let mut sock = VoiceStream::new(sock, tokio_util::sync::CancellationToken::new());
            mecha_direct(&mut sock, &shared, body.to_string().as_bytes())
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut got = Vec::new();
        client.read_to_end(&mut got).await.unwrap();
        server.await.unwrap();
        let got = String::from_utf8(got).unwrap();
        serde_json::from_str(got.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    fn directions_on_file(session: &Session) -> Vec<SpokenDirection> {
        std::fs::read_to_string(&session.path)
            .unwrap()
            .lines()
            .filter_map(|l| match serde_json::from_str::<Record>(l) {
                Ok(Record::SpokenDirection(d)) => Some(d),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_spoken_reply_is_directed_on_the_loaded_model_and_every_sentence_recorded() {
        let home = crate::testenv::HomeGuard::new("director-turn");
        let (facade, seen) = facade(&home);
        let shared = &facade.shared;
        let session = transcript(&home, "directed-turn");
        shared.directions.model_turn(
            shared,
            "chat:main",
            "chatcmpl-1",
            "I finally finished the grant.",
            DirectorSeed {
                transcript: Some(session.clone()),
                character: None,
                last_reply: Some("Good luck with it!".into()),
            },
            true,
        );
        let first = ask_over_socket(
            shared,
            json!({"session": "chat:main", "context": "ctx-1", "index": 0,
                   "sentence": "Oh, you did it!", "voice": "house"}),
        )
        .await;
        assert_eq!(first["direction"], "Warm, unhurried, smiling.");
        assert_eq!(first["outcome"], "ok");
        let second = ask_over_socket(
            shared,
            json!({"session": "chat:main", "context": "ctx-1", "index": 1,
                   "sentence": "Go rest.", "voice": "house"}),
        )
        .await;
        assert_eq!(second["outcome"], "ok");

        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen.len(),
            2,
            "the opening, then the second sentence: {seen:?}"
        );
        for req in &seen {
            assert_eq!(req.model, "loaded-model", "the loaded model, never another");
            assert_eq!(req.think, Some(false));
            assert!(req.tools.is_empty());
            assert_eq!(req.messages.len(), 1);
        }
        assert!(seen[0].messages[0].text().contains("Now: the opening"));
        let later = seen[1].messages[0].text();
        assert!(later.contains("- \"Oh, you did it!\" -> Warm, unhurried, smiling."));
        assert!(later.ends_with("Now: \"Go rest.\""));

        let kept = directions_on_file(&session);
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(kept[0].opening && kept[0].index == 0);
        assert_eq!(kept[0].sentence, "Oh, you did it!");
        assert_eq!(kept[1].sentence, "Go rest.");
        for d in &kept {
            assert_eq!(d.turn, "chatcmpl-1");
            assert_eq!(d.context.as_deref(), Some("ctx-1"));
            assert_eq!(d.outcome, Outcome::Ok);
            assert_eq!(d.model.as_deref(), Some("loaded-model"));
            assert!(d.prompt_digest.is_some());
        }
    }

    #[tokio::test]
    async fn an_incognito_chat_is_directed_and_nothing_is_written() {
        let home = crate::testenv::HomeGuard::new("director-incognito");
        let (facade, seen) = facade(&home);
        let shared = &facade.shared;
        // A transcript handed in anyway: the key alone must keep it unwritten.
        let session = transcript(&home, "never-written");
        let before = std::fs::read_to_string(&session.path).unwrap();
        let key = format!("chat:{}", crate::commands::serve::incognito::new_key());
        shared.directions.model_turn(
            shared,
            &key,
            "chatcmpl-2",
            "hello",
            DirectorSeed {
                transcript: Some(session.clone()),
                ..Default::default()
            },
            false,
        );
        let got = ask_over_socket(
            shared,
            json!({"session": key, "index": 0, "sentence": "Hi there."}),
        )
        .await;
        assert_eq!(got["outcome"], "ok", "the director runs in incognito");
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(&session.path).unwrap(), before);
    }

    #[tokio::test]
    async fn harness_speech_and_an_unknown_conversation_ask_nothing() {
        let home = crate::testenv::HomeGuard::new("director-harness");
        let (facade, seen) = facade(&home);
        let shared = &facade.shared;
        let session = transcript(&home, "harness-turn");
        shared.directions.model_turn(
            shared,
            "voice:a",
            "chatcmpl-3",
            "send it",
            DirectorSeed {
                transcript: Some(session.clone()),
                ..Default::default()
            },
            false,
        );
        shared
            .directions
            .note_harness("chatcmpl-3", " Shall I send the draft to Ada?");
        let got = ask_over_socket(
            shared,
            json!({"session": "voice:a", "index": 2, "sentence": "Shall I send the draft to Ada?"}),
        )
        .await;
        assert_eq!(got["outcome"], "skipped");
        assert_eq!(got["reason"], "harness");
        let none = ask_over_socket(
            shared,
            json!({"session": "voice:nobody", "index": 0, "sentence": "Hello."}),
        )
        .await;
        assert_eq!(none["reason"], "no_turn");
        assert!(seen.lock().unwrap().is_empty(), "nothing reached the model");
        let kept = directions_on_file(&session);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].outcome, Outcome::Skipped);
        assert_eq!(kept[0].reason.as_deref(), Some("harness"));
    }

    #[test]
    fn an_incognito_key_is_known_by_its_prefix() {
        let key = crate::commands::serve::incognito::new_key();
        assert!(names_incognito(&format!("chat:{key}")));
        assert!(!names_incognito("chat:main"));
        assert!(!names_incognito("voice:webrtc-1"));
    }
}
