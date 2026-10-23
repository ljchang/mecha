use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The bytes `/view` answers with: an mp4's opening box, enough for the
/// `ftyp` check.
const MP4: &[u8] = b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2";

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mecha-videogen-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The machine as a test sets it: no call unless said, memory admitted
/// unless said.
#[derive(Default)]
struct Stub {
    live: Mutex<Vec<Result<bool, String>>>,
    short: AtomicBool,
}

impl Gates for Stub {
    fn voice_live(&self) -> Result<bool> {
        let mut live = self.live.lock().unwrap();
        let next = if live.len() > 1 {
            live.remove(0)
        } else {
            live.first().cloned().unwrap_or(Ok(false))
        };
        next.map_err(|why| anyhow!("{why}"))
    }

    fn admit(&self, _need: u64) -> Option<String> {
        self.short
            .load(Ordering::SeqCst)
            .then(|| "Not enough memory for a clip now.".to_string())
    }
}

/// What the fake server does beyond answering.
#[derive(Clone, Default)]
struct Fake {
    /// Polls answered "not yet" before the clip is done.
    pending_polls: usize,
    /// The finished entry's status, `success` unless set.
    status: Option<&'static str>,
    /// Files `/object_info` does not list.
    missing: Vec<String>,
    /// Never finishes.
    forever: bool,
}

type Log = Arc<Mutex<Vec<String>>>;

async fn fake(opts: Fake, cfg: &VideoConfig) -> (String, Log) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log: Log = Arc::default();
    let seen = Arc::clone(&log);
    let polls = Arc::new(Mutex::new(0usize));
    let files: Vec<String> = [
        &cfg.frames_model,
        &cfg.references_model,
        &cfg.frames_turbo_lora,
        &cfg.references_turbo_lora,
        &cfg.text_encoder,
        &cfg.vae,
        &cfg.audio_vae,
    ]
    .iter()
    .map(|s| s.to_string())
    .filter(|f| !opts.missing.contains(f))
    .collect();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (log, polls, files, opts) = (
                Arc::clone(&seen),
                Arc::clone(&polls),
                files.clone(),
                opts.clone(),
            );
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut tmp = [0u8; 8192];
                let mut head_end = None;
                loop {
                    if let Some(end) = head_end {
                        let head = String::from_utf8_lossy(&req[..end]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if req.len() >= end + 4 + len {
                            break;
                        }
                    }
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&tmp[..n]),
                    }
                    if head_end.is_none() {
                        head_end = req.windows(4).position(|w| w == b"\r\n\r\n");
                    }
                }
                let text = String::from_utf8_lossy(&req).to_string();
                let line = text.lines().next().unwrap_or_default().to_string();
                let body = head_end
                    .map(|e| String::from_utf8_lossy(&req[e + 4..]).to_string())
                    .unwrap_or_default();
                let path = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                log.lock().unwrap().push(format!("{line} {body}"));
                let (kind, out): (&str, Vec<u8>) =
                    if let Some(node) = path.strip_prefix("/object_info/") {
                        let input = match node {
                            "UNETLoader" => "unet_name",
                            "CLIPLoader" => "clip_name",
                            "VAELoader" => "vae_name",
                            "LoraLoaderModelOnly" => "lora_name",
                            _ => "",
                        };
                        let v = if input.is_empty() {
                            json!({node: {"input": {}}})
                        } else {
                            json!({node: {"input": {"required": {input: [files, {}]}}}})
                        };
                        ("application/json", v.to_string().into_bytes())
                    } else if path == "/upload/image" {
                        let sent = body
                            .split("filename=\"")
                            .nth(1)
                            .and_then(|rest| rest.split('"').next())
                            .unwrap_or("up.png")
                            .to_string();
                        (
                            "application/json",
                            json!({"name": sent, "subfolder": "", "type": "temp"})
                                .to_string()
                                .into_bytes(),
                        )
                    } else if path == "/prompt" {
                        let asked = serde_json::from_str::<Value>(&body)
                            .ok()
                            .and_then(|v| v["prompt_id"].as_str().map(str::to_string))
                            .unwrap_or_default();
                        (
                            "application/json",
                            json!({"prompt_id": asked}).to_string().into_bytes(),
                        )
                    } else if let Some(id) = path.strip_prefix("/history/") {
                        let n = {
                            let mut p = polls.lock().unwrap();
                            *p += 1;
                            *p
                        };
                        let v = if opts.forever || n <= opts.pending_polls {
                            json!({})
                        } else {
                            json!({id: {
                                "status": {"status_str": opts.status.unwrap_or("success"),
                                    "messages": [["execution_error", {
                                        "exception_type": "RuntimeError",
                                        "node_type": "VAEDecode",
                                        "exception_message": "out of memory"}]]},
                                "outputs": {"save": {"images": [{"filename": "x_00001_.mp4",
                                    "subfolder": "mecha-video", "type": "output"}],
                                    "animated": [true]}}
                            }})
                        };
                        ("application/json", v.to_string().into_bytes())
                    } else if path.starts_with("/view?") {
                        ("video/mp4", MP4.to_vec())
                    } else if path == "/queue" && line.starts_with("GET") {
                        (
                            "application/json",
                            json!({"queue_running": [], "queue_pending": []})
                                .to_string()
                                .into_bytes(),
                        )
                    } else {
                        ("application/json", b"{}".to_vec())
                    };
                let mut reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n",
                    out.len()
                )
                .into_bytes();
                reply.extend_from_slice(&out);
                let _ = sock.write_all(&reply).await;
            });
        }
    });
    (format!("http://{addr}"), log)
}

fn tool(url: &str, home: &Path, gates: Arc<dyn Gates>) -> VideoGenerate {
    VideoGenerate::new(VideoConfig {
        url: url.into(),
        ..VideoConfig::default()
    })
    .unwrap()
    .for_test(home, gates, Duration::from_millis(20))
}

fn later() -> std::time::Instant {
    std::time::Instant::now() + Duration::from_secs(60)
}

fn ctx(dir: &Path) -> ToolCtx {
    ToolCtx {
        workspace: dir.to_path_buf(),
        ..Default::default()
    }
}

/// A call through the trait, its job awaited inline as a host-less run does.
async fn run(t: &VideoGenerate, input: Value, ctx: &ToolCtx) -> ToolOutput {
    let out = <VideoGenerate as Tool>::call(t, input, ctx).await.unwrap();
    match out.deferred.clone() {
        Some(job) => crate::jobs::run_inline(&job, ctx.cancel.as_ref()).await,
        None => out,
    }
}

fn character(lib: &Path, name: &str, text: &str) {
    let img = image::RgbImage::from_pixel(2, 2, image::Rgb([40, 10, 10]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    crate::imagelib::create(
        lib,
        crate::imagelib::NewEntry {
            kind: crate::imagelib::Kind::Character,
            name: name.to_string(),
            text: text.to_string(),
            portrait: Some(png.into_inner()),
            source_seed: None,
            origin: crate::imagelib::Origin::Owner,
            locked: false,
        },
    )
    .unwrap();
}

/// A mono 16-bit WAV at 8 kHz, `seconds` long.
fn wav(seconds: u32) -> Vec<u8> {
    let rate = 8000u32;
    let data = vec![0u8; (rate * 2 * seconds) as usize];
    let mut w = b"RIFF".to_vec();
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

/// The call answers at once in the harness's words and defers; the job
/// renders, fetches the clip through `/view` into `videos/`, keeps the
/// record the estimate reads, and frees the server's memory after.
#[tokio::test]
async fn a_clip_is_rendered_into_the_chats_videos_folder() {
    let home = scratch();
    let chat = home.join("work").join("web").join("c-1");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, log) = fake(
        Fake {
            pending_polls: 2,
            ..Fake::default()
        },
        &cfg,
    )
    .await;
    let t = tool(&url, &home, Arc::new(Stub::default())).with_chat_settings();
    let ctx = ctx(&chat);
    let input =
        json!({"prompt": "A kettle boils on a workshop stove.", "seconds": 6, "turbo": false});
    let out = <VideoGenerate as Tool>::call(&t, input.clone(), &ctx)
        .await
        .unwrap();
    assert!(
        out.content
            .starts_with("being made: `video_generate` is running."),
        "{}",
        out.content
    );
    assert!(out.deferred.is_some() && t.defers());
    let done = run(&t, input, &ctx).await;
    assert!(!done.is_error, "{}", done.content);
    let first = done.content.lines().next().unwrap();
    let saved = first.strip_prefix("video: ").expect("a video line");
    assert!(
        saved.starts_with("videos/v") && saved.ends_with(".mp4"),
        "{saved}"
    );
    assert_eq!(std::fs::read(chat.join(saved)).unwrap(), MP4);
    assert!(
        done.content.contains("full quality (20 steps), full size"),
        "{}",
        done.content
    );
    let seen = log.lock().unwrap().join("\n");
    assert!(seen.contains("POST /free"), "the memory goes back: {seen}");
    let prompt = seen
        .lines()
        .find(|l| l.starts_with("POST /prompt"))
        .unwrap();
    assert!(
        !prompt.contains("\"LoraLoaderModelOnly\""),
        "turbo off: no LoRA link: {prompt}"
    );
    // The record the next clip's estimate reads, beside the kept settings.
    let jobs: Vec<_> = std::fs::read_dir(home.join("video/jobs"))
        .unwrap()
        .collect();
    assert_eq!(jobs.len(), 1);
    let kept = home.join("video/chat-settings/work_web_c-1.json");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(kept).unwrap()).unwrap();
    assert_eq!(kept["turbo"], json!(false));
    // The next clip in this chat keeps them, and says so.
    let again = run(&t, json!({"prompt": "The kettle whistles."}), &ctx).await;
    assert!(
        again
            .content
            .contains("Kept from this chat's last clip: full quality (turbo off)"),
        "{}",
        again.content
    );
    std::fs::remove_dir_all(&home).ok();
}

/// A chat keeps the length it asked for, not the rendered one: a 15 s clip
/// snaps to 15.08 s of frames, and keeping that refused every later clip in
/// the chat (review of #733).
#[tokio::test]
async fn a_chat_keeps_the_length_it_asked_for() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, _log) = fake(Fake::default(), &cfg).await;
    let t = tool(&url, &home, Arc::new(Stub::default())).with_chat_settings();
    let ctx = ctx(&chat);
    let first = run(&t, json!({"prompt": "p", "seconds": 15}), &ctx).await;
    assert!(!first.is_error, "{}", first.content);
    let again = run(&t, json!({"prompt": "p"}), &ctx).await;
    assert!(!again.is_error, "{}", again.content);
    assert!(
        again
            .content
            .contains("Kept from this chat's last clip: 15 s."),
        "{}",
        again.content
    );
    // The default length is not said as a kept one.
    let five = run(&t, json!({"prompt": "p", "seconds": 5}), &ctx).await;
    assert!(!five.is_error, "{}", five.content);
    let after = run(&t, json!({"prompt": "p"}), &ctx).await;
    assert!(!after.content.contains("Kept from"), "{}", after.content);
    std::fs::remove_dir_all(&home).ok();
}

/// A name no library character answers to, a path outside the chat, or a
/// length out of range is refused at the call: nothing is queued and no
/// turn is taken.
#[tokio::test]
async fn a_bad_reference_is_refused_before_anything_is_queued() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let t = tool("http://127.0.0.1:9", &home, Arc::new(Stub::default()));
    for (input, says) in [
        (
            json!({"prompt": "p", "references": ["nobody"]}),
            "no character named `nobody`",
        ),
        (
            json!({"prompt": "p", "first_frame": "/etc/hostname.png"}),
            "",
        ),
        (
            json!({"prompt": "p", "seconds": 40}),
            "`seconds` must be between",
        ),
        (json!({}), "give `scene`"),
    ] {
        let out = <VideoGenerate as Tool>::call(&t, input, &ctx(&chat))
            .await
            .unwrap();
        assert!(out.is_error && out.deferred.is_none(), "{}", out.content);
        assert!(out.content.contains(says), "{}", out.content);
    }
    assert!(!home.join("video/render.turn").exists());
    std::fs::remove_dir_all(&home).ok();
}

/// A persona's own clip shows her library character and carries her own
/// voice, cut to a line's length; a character the prompt mentions comes
/// along as well.
#[tokio::test]
async fn a_persona_clip_has_her_face_and_her_voice() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    character(&home.join("imagelib"), "maya", "Tall, with short grey hair");
    character(&home.join("imagelib"), "wren", "Freckled, with a red bob");
    let voices = home.join("voices");
    std::fs::create_dir_all(&voices).unwrap();
    std::fs::write(voices.join("maya-low.wav"), wav(20)).unwrap();
    let cfg = VideoConfig::default();
    let (url, log) = fake(Fake::default(), &cfg).await;
    let t = Arc::new(tool(&url, &home, Arc::new(Stub::default())).with_voices_dir(Some(voices)));
    let form = Arc::clone(&t)
        .for_persona_as(&PersonaSelf {
            name: "maya".into(),
            display: "Maya".into(),
            character: Some("maya".into()),
            voice: Some("maya-low".into()),
        })
        .unwrap();
    let out = form
        .call(
            json!({"prompt": "Maya waves at Wren across the workshop."}),
            &ctx(&chat),
        )
        .await
        .unwrap();
    let done = crate::jobs::run_inline(out.deferred.as_ref().unwrap(), None).await;
    assert!(!done.is_error, "{}", done.content);
    assert!(done.content.contains("references mode"), "{}", done.content);
    let seen = log.lock().unwrap().clone();
    let uploads: Vec<&String> = seen
        .iter()
        .filter(|l| l.starts_with("POST /upload"))
        .collect();
    assert_eq!(uploads.len(), 3, "two portraits and one voice");
    // The voice went up trimmed: five seconds of 16 kB/s and a header, not
    // the twenty seconds on disk.
    let voice = uploads.iter().find(|l| l.contains("-voice0.wav")).unwrap();
    assert!(voice.len() < 5 * 16_000 + 2_000, "{} bytes", voice.len());
    let prompt = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
    assert!(prompt.contains("MiniMaxH3ReferenceToVideo"), "{prompt}");
    assert!(prompt.contains("ref_audios.ref_audio_0"), "{prompt}");
    assert!(
        prompt.contains("LoraLoaderModelOnly"),
        "turbo by default: {prompt}"
    );
    std::fs::remove_dir_all(&home).ok();
}

/// A persona whose character the library does not hold approved is not
/// made every clip's lone reference: the clip is made from words, as her
/// picture is (review of #733).
#[tokio::test]
async fn a_persona_without_a_library_character_still_gets_a_clip() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, log) = fake(Fake::default(), &cfg).await;
    let t = Arc::new(tool(&url, &home, Arc::new(Stub::default())));
    let form = Arc::clone(&t)
        .for_persona_as(&PersonaSelf {
            name: "ada".into(),
            display: "Ada".into(),
            character: Some("ada".into()),
            voice: None,
        })
        .unwrap();
    let out = form
        .call(json!({"prompt": "Rain on a window."}), &ctx(&chat))
        .await
        .unwrap();
    assert!(out.deferred.is_some(), "{}", out.content);
    let done = crate::jobs::run_inline(out.deferred.as_ref().unwrap(), None).await;
    assert!(done.content.contains("frames mode"), "{}", done.content);
    assert!(!log
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.starts_with("POST /upload")));
    std::fs::remove_dir_all(&home).ok();
}

/// A reference written as the model capitalises it and the same name in
/// the prompt are one reference, not two.
#[tokio::test]
async fn a_name_given_twice_is_one_reference() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    character(&home.join("imagelib"), "wren", "Freckled, with a red bob");
    let cfg = VideoConfig::default();
    let (url, log) = fake(Fake::default(), &cfg).await;
    let t = tool(&url, &home, Arc::new(Stub::default()));
    let done = run(
        &t,
        json!({"prompt": "Wren waves.", "references": ["Wren"]}),
        &ctx(&chat),
    )
    .await;
    assert!(!done.is_error, "{}", done.content);
    let uploads = log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.starts_with("POST /upload"))
        .count();
    assert_eq!(uploads, 1);
    std::fs::remove_dir_all(&home).ok();
}

/// A voice call holds a clip until it ends; a call that cannot be ruled out
/// refuses, since unknown is not "no".
#[tokio::test]
async fn a_live_call_holds_the_clip_and_an_unknown_one_refuses() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, log) = fake(Fake::default(), &cfg).await;
    let stub = Arc::new(Stub {
        live: Mutex::new(vec![Ok(true), Ok(true), Ok(false)]),
        ..Stub::default()
    });
    let t = tool(&url, &home, stub.clone());
    let done = run(&t, json!({"prompt": "p"}), &ctx(&chat)).await;
    assert!(!done.is_error, "{}", done.content);
    assert_eq!(stub.live.lock().unwrap().len(), 1, "asked until it ended");
    let unknown = Arc::new(Stub {
        live: Mutex::new(vec![Err("the journal could not be read".into())]),
        ..Stub::default()
    });
    let t = tool(&url, &home, unknown);
    let before = log.lock().unwrap().len();
    let done = run(&t, json!({"prompt": "p"}), &ctx(&chat)).await;
    assert!(done.is_error, "{}", done.content);
    assert!(done.content.contains("journal"), "{}", done.content);
    assert!(
        !log.lock().unwrap()[before..]
            .iter()
            .any(|l| l.starts_with("POST /prompt")),
        "nothing submitted"
    );
    std::fs::remove_dir_all(&home).ok();
}

/// Short memory waits, rather than failing a clip the machine could take
/// in a minute.
#[tokio::test]
async fn short_memory_waits_for_the_machine() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, _log) = fake(Fake::default(), &cfg).await;
    let stub = Arc::new(Stub::default());
    stub.short.store(true, Ordering::SeqCst);
    let t = tool(&url, &home, stub.clone());
    let release = Arc::clone(&stub);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        release.short.store(false, Ordering::SeqCst);
    });
    let done = run(&t, json!({"prompt": "p"}), &ctx(&chat)).await;
    assert!(!done.is_error, "{}", done.content);
    std::fs::remove_dir_all(&home).ok();
}

/// A file the graph names that the server lacks is said by name before
/// anything is uploaded.
#[tokio::test]
async fn a_missing_model_file_is_named_before_any_upload() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, log) = fake(
        Fake {
            missing: vec![cfg.frames_turbo_lora.clone()],
            ..Fake::default()
        },
        &cfg,
    )
    .await;
    let t = tool(&url, &home, Arc::new(Stub::default()));
    let done = run(&t, json!({"prompt": "p"}), &ctx(&chat)).await;
    assert!(done.is_error, "{}", done.content);
    assert!(
        done.content.contains(&cfg.frames_turbo_lora),
        "{}",
        done.content
    );
    assert!(!log
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.starts_with("POST /upload") || l.starts_with("POST /prompt")));
    std::fs::remove_dir_all(&home).ok();
}

/// Stop takes a render off the server, frees its memory and gives the turn
/// back.
#[tokio::test]
async fn a_stop_takes_the_clip_off_the_server() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, log) = fake(
        Fake {
            forever: true,
            ..Fake::default()
        },
        &cfg,
    )
    .await;
    let t = tool(&url, &home, Arc::new(Stub::default()));
    let out = <VideoGenerate as Tool>::call(&t, json!({"prompt": "p"}), &ctx(&chat))
        .await
        .unwrap();
    let job = out.deferred.unwrap();
    let token = job.cancel_token().clone();
    let watch = Arc::clone(&log);
    tokio::spawn(async move {
        while !watch
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("GET /history/"))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        token.cancel();
    });
    let done = crate::jobs::run_inline(&job, None).await;
    assert!(done.is_error);
    assert!(
        done.content.starts_with("not made: stopped"),
        "{}",
        done.content
    );
    let seen = log.lock().unwrap().join("\n");
    assert!(
        seen.contains("POST /queue"),
        "taken out of the line: {seen}"
    );
    assert!(seen.contains("POST /free"), "{seen}");
    assert!(
        !home.join("video/render.turn").exists(),
        "the turn is given back"
    );
    std::fs::remove_dir_all(&home).ok();
}

/// A failed render says the server's own error, briefly.
#[tokio::test]
async fn a_failed_render_says_why() {
    let home = scratch();
    let chat = home.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    let cfg = VideoConfig::default();
    let (url, _log) = fake(
        Fake {
            status: Some("error"),
            ..Fake::default()
        },
        &cfg,
    )
    .await;
    let t = tool(&url, &home, Arc::new(Stub::default()));
    let done = run(&t, json!({"prompt": "p"}), &ctx(&chat)).await;
    assert!(done.is_error);
    assert!(
        done.content
            .contains("RuntimeError in VAEDecode: out of memory"),
        "{}",
        done.content
    );
    std::fs::remove_dir_all(&home).ok();
}

/// One clip at a time across callers: the second waits for the first's
/// turn and takes it once the first is done; a holder that is gone frees
/// its turn, and Stop ends a wait.
#[tokio::test]
async fn one_clip_renders_at_a_time_and_a_dead_holder_frees_the_turn() {
    let home = scratch();
    let t = tool("http://127.0.0.1:9", &home, Arc::new(Stub::default()));
    let first = t
        .wait_turn(&CancellationToken::new(), later())
        .await
        .ok()
        .unwrap();
    let t2 = t.clone();
    let got = Arc::new(AtomicBool::new(false));
    let got2 = Arc::clone(&got);
    let waiter = tokio::spawn(async move {
        let turn = t2
            .wait_turn(&CancellationToken::new(), later())
            .await
            .ok()
            .unwrap();
        got2.store(true, Ordering::SeqCst);
        drop(turn);
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!got.load(Ordering::SeqCst), "took a turn that was held");
    drop(first);
    waiter.await.unwrap();
    assert!(got.load(Ordering::SeqCst));

    let path = home.join("video/render.turn");
    // pid 2^22 + 7 is above Linux's pid_max ceiling: never alive.
    std::fs::write(&path, "4194311 0\n").unwrap();
    let turn = t
        .wait_turn(&CancellationToken::new(), later())
        .await
        .ok()
        .unwrap();
    let held = std::fs::read_to_string(&path).unwrap();
    assert!(held.starts_with(&std::process::id().to_string()), "{held}");
    let stop = CancellationToken::new();
    stop.cancel();
    assert!(matches!(
        t.wait_turn(&stop, later()).await,
        Err(Failure::Cancelled)
    ));
    drop(turn);
    assert!(!path.exists());
    std::fs::remove_dir_all(&home).ok();
}

/// The estimate is this machine's own median rate for clips with as many
/// references, times this clip's frames, steps and megapixels; a failed
/// clip is not a rate, and too few clips is no estimate.
#[test]
fn a_clip_is_estimated_from_this_machines_own_clips() {
    let jobs = scratch();
    let job = |id: &str, secs: u64, refs: usize, turbo: bool, error: bool| {
        let mut j = json!({"id": id, "created": 1000, "finished": 1000 + secs,
            "frames": 100, "steps": 10, "width": 1000, "height": 1000, "turbo": turbo,
            "references": vec!["maya"; refs], "output": "x.mp4"});
        if error {
            j["error"] = json!("stopped by the owner");
        }
        std::fs::write(jobs.join(format!("{id}.json")), j.to_string()).unwrap();
    };
    job("a", 500, 1, true, false);
    assert_eq!(estimate(&jobs, 200, 10, 1000, 1000, 1, true), None);
    job("b", 600, 1, true, false);
    job("c", 700, 1, true, false);
    job("d", 9000, 1, true, true);
    // 1000 units each: one-reference turbo rates .5, .6, .7; the median .6,
    // times 2000 units.
    assert_eq!(
        estimate(&jobs, 200, 10, 1000, 1000, 1, true),
        Some((1200, 3))
    );
    for id in ["e", "f", "g"] {
        job(id, 2000, 0, true, false);
    }
    assert_eq!(
        estimate(&jobs, 200, 10, 1000, 1000, 1, true),
        Some((1200, 3))
    );
    assert_eq!(
        estimate(&jobs, 100, 10, 1000, 1000, 0, true),
        Some((2000, 3))
    );
    // Three references, none like it: the turbo clips' median of six.
    assert_eq!(
        estimate(&jobs, 100, 10, 1000, 1000, 3, true),
        Some((2000, 6))
    );
    for id in ["h", "i", "j"] {
        job(id, 300, 2, false, false);
    }
    assert_eq!(
        estimate(&jobs, 200, 10, 1000, 1000, 1, false),
        Some((600, 3))
    );
    std::fs::remove_dir_all(&jobs).ok();
}

#[test]
fn a_result_says_turbo_or_full_quality_and_the_steps() {
    assert_eq!(
        quality(&json!({"turbo": false, "steps": 20, "size": "full"})),
        "full quality (20 steps), full size"
    );
    assert_eq!(
        quality(&json!({"turbo": true, "steps": 4, "size": "draft"})),
        "turbo (4 steps), draft size"
    );
}

/// A voice is cut to its first seconds on a whole sample, its header
/// rewritten; anything that is not a readable WAV is left to be sent whole.
#[test]
fn a_voice_is_trimmed_to_a_lines_length() {
    let long = wav(20);
    let cut = trim_wav(&long, 5.0).unwrap();
    assert_eq!(cut.len(), 44 + 5 * 16_000);
    assert_eq!(&cut[..4], b"RIFF");
    assert_eq!(
        u32::from_le_bytes(cut[4..8].try_into().unwrap()) as usize,
        cut.len() - 8
    );
    assert_eq!(
        u32::from_le_bytes(cut[40..44].try_into().unwrap()),
        5 * 16_000
    );
    // Shorter than the cut: kept whole.
    assert_eq!(trim_wav(&wav(1), 5.0).unwrap().len(), 44 + 16_000);
    assert!(trim_wav(b"OggS not a wav", 5.0).is_none());
    assert!(trim_wav(&long[..30], 5.0).is_none());
}

#[tokio::test]
async fn the_guide_is_answered_whole() {
    let out = VideoPromptGuide
        .call(json!({}), &ToolCtx::default())
        .await
        .unwrap();
    assert!(out.content.starts_with("# Prompting video_generate"));
    assert!(Arc::new(VideoPromptGuide).for_persona().is_some());
}

/// The registered names are the constants setup and the docs use.
#[test]
fn the_names_are_the_constants() {
    let t = VideoGenerate::new(VideoConfig::default()).unwrap();
    assert_eq!(t.name(), NAME);
    assert_eq!(VideoPromptGuide.name(), GUIDE_NAME);
}
