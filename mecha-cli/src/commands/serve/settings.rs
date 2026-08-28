//! The settings page's backend: the charter, the learned rules, and the
//! voice stack's health, behind the same owner guard as everything else.
//!
//! What may be *written* from here is decided by who owns the consequence,
//! and the answer is: exactly one thing. The charter is the owner's own
//! document, read by every run and writable by "the owner with a text
//! editor" (`docs/GOAL-SYSTEM-DESIGN.md` §11) — a validated save from the
//! owner's authenticated page is that, with a different editor. Everything
//! else on the page is a read: learned rules mutate through their own gated
//! verbs (`mecha rules retire` stages through proposals; nothing here
//! shortcuts that), and the voice stack is configured where it runs.
//! Deliberately absent, and not as an oversight: anything whose edit widens
//! security posture — `[sandbox]`, `[security]`, `[outbox]` routing — stays
//! in `config.toml` where a diff reviews it, on `names_guarded_setting`'s
//! own list of the boundaries that always reach a human.
//!
//! Two rules on the charter write, both structural:
//!
//! - **A save is validated by the same reader every run loads through**
//!   (`Charter::parse`, which `Charter::load` itself delegates to), and an
//!   invalid document is refused with the parse error — it never reaches
//!   disk. The TUI's `/charter` accepts an invalid save and reports it,
//!   because there the file was edited *in place* by `$EDITOR` and the
//!   damage is already done; here the bytes are still ours to refuse, so
//!   refusing is strictly better than the warning.
//! - **Temp-sibling-and-rename**, the store convention: a browser that
//!   disconnects mid-request must not leave half a charter where every
//!   future run's priorities live.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

type St = State<super::WebState>;

/// A per-request suffix for temp-sibling writes. The pid alone was the
/// first cut and is the wrong key: the concurrent unit on an async server
/// is the request, and two overlapping saves sharing one temp path can
/// interleave write/rename so the refused request's bytes are the ones
/// that landed. Monotonic within the process; the pid keeps two *processes*
/// (a dev serve beside the real one, aimed at one store) apart too.
fn request_stamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// The one shape both the GET and a successful save return, so the page
/// never has to merge two descriptions of the same file.
fn charter_state() -> Json<serde_json::Value> {
    let path = match mecha_core::charter::Charter::default_path() {
        Ok(p) => p,
        Err(e) => {
            return Json(serde_json::json!({ "error": format!("{e:#}") }));
        }
    };
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let body = match mecha_core::charter::Charter::load(&path) {
        Ok(charter) => serde_json::json!({
            "path": path,
            "exists": path.is_file(),
            "raw": raw,
            "lines": charter.lines().iter().map(|l| serde_json::json!({
                "id": l.id,
                "text": l.text,
            })).collect::<Vec<_>>(),
            "char_count": charter.char_count(),
            "over_budget": charter.over_budget(),
            "budget": mecha_core::charter::CHARTER_CHAR_BUDGET,
            // What the editor seeds a first charter from — the same
            // comments-only bytes the TUI's `e` writes, served so the two
            // surfaces cannot drift and the browser's first edit never
            // starts from an empty buffer.
            "template": mecha_core::charter::TEMPLATE,
        }),
        // A broken charter is a state the page must show, not a 500: the
        // TUI's rule that the failure is the headline, one surface over.
        Err(e) => serde_json::json!({
            "path": path,
            "exists": path.is_file(),
            "raw": raw,
            "parse_error": format!("{e:#}"),
        }),
    };
    Json(body)
}

/// GET /api/settings/charter
pub async fn charter(State(_state): St) -> Json<serde_json::Value> {
    charter_state()
}

#[derive(Deserialize)]
pub struct CharterSave {
    raw: String,
}

/// A charter is a handful of lines under a 2,000-character rendered budget;
/// a body orders of magnitude past that is not an edit of one, whatever it
/// parses as. Refused before the *TOML* parser sees it — the JSON envelope
/// has already been read by the time the handler runs, bounded by axum's
/// own 2 MB default — so a runaway paste cannot cost a TOML parse of
/// arbitrary input.
const MAX_CHARTER_BYTES: usize = 64 * 1024;

/// POST /api/settings/charter — validate, then write, in that order.
pub async fn charter_save(State(_state): St, Json(body): Json<CharterSave>) -> Response {
    if body.raw.len() > MAX_CHARTER_BYTES {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{} bytes is not a charter — the whole rendered budget is {} characters\n",
                body.raw.len(),
                mecha_core::charter::CHARTER_CHAR_BUDGET
            ),
        )
            .into_response();
    }
    // The same reader every run loads through. A document this refuses
    // never reaches disk, which is the property the module doc names.
    if let Err(e) = mecha_core::charter::Charter::parse(&body.raw) {
        return (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}\n")).into_response();
    }
    let path = match mecha_core::charter::Charter::default_path() {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
        }
    }
    // Temp-sibling-and-rename: same directory, so the rename cannot cross a
    // filesystem, and a crash between the two leaves the old charter whole.
    // Keyed per *request*, not per process — one async server, many
    // connections, and two saves sharing one temp path can land A's rename
    // over B's write while telling each the other's outcome.
    let tmp = path.with_extension(format!("toml.tmp.{}", request_stamp()));
    let write = std::fs::write(&tmp, &body.raw).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
    }
    charter_state().into_response()
}

/// GET /api/settings/rules — the learned-rule roster with its ledger
/// tallies, exactly what the TUI's `/learning` reads. A read: retiring goes
/// through `mecha rules retire`'s own staged path, and nothing here offers a
/// shortcut around it.
pub async fn rules(State(_state): St) -> Response {
    match super::self_cli_json(&["rules", "list", "--json"], false).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

/// GET /api/settings/voice — is the stack there at all, and where this
/// process would send an offer. Read-only: the voice worker is configured
/// where it runs (`scripts/voice/`), and a reachability answer is what a
/// settings page can honestly own from here.
pub async fn voice(State(state): St) -> Json<serde_json::Value> {
    let target = state.offer_target.as_ref().map(|t| t.as_str().to_string());
    let reachable = match &target {
        None => None,
        Some(url) => Some(probe(url).await),
    };
    // The cloned references on disk — name, duration, and when it was made.
    // Listed from the store itself rather than from any cache, because this
    // is the management view: a file someone dropped in by hand belongs on
    // it exactly as much as one recorded through the page.
    // A store that could not be read is its own answer — "configured,
    // nothing cloned yet" and "configured, could not look" are opposite
    // findings (the dash-versus-zero rule), and folding a read failure into
    // an empty list would surface the misconfiguration only after someone
    // has recorded themselves.
    let mut cloned_error: Option<String> = None;
    let cloned = state.voices_dir.as_ref().map(|dir| {
        let mut out = Vec::new();
        match std::fs::read_dir(dir.as_ref()) {
            Err(e) => cloned_error = Some(format!("{}: {e}", dir.display())),
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                        continue;
                    }
                    let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    // A bounded read: the fmt/data headers live in the first
                    // few hundred bytes, and slurping every clone's megabytes
                    // to answer a settings GET would make the page cost more
                    // the more voices it lists. `wav_seconds` reads declared
                    // sizes off headers, never the payload.
                    let seconds = read_head(&path, 64 * 1024)
                        .ok()
                        .and_then(|b| wav_seconds(&b).ok());
                    let created = entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    out.push(serde_json::json!({
                        "name": name,
                        "seconds": seconds,
                        // Unix seconds, off the mtime — which, for a store only
                        // ever written by the clone endpoint, is when it was
                        // recorded.
                        "created": created,
                    }));
                }
            }
        }
        out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        out
    });
    Json(serde_json::json!({
        // None = voice is not wired on this serve at all — a different fact
        // from "wired and down", and the page shows them differently.
        "offer_target": target,
        "worker_reachable": reachable,
        // None = cloning unconfigured ([web] voices_dir unset); an empty
        // list = configured, nothing cloned yet. Opposite findings.
        "cloned": cloned,
        // Set only when the configured directory could not be listed; the
        // page shows it instead of an empty store.
        "cloned_error": cloned_error,
        "voices_dir": state.voices_dir.as_ref().map(|d| d.display().to_string()),
    }))
}

/// A voice name is a bare filename stem, and the alphabet is closed rather
/// than denylisted: this string becomes `<voices_dir>/<name>.wav` on one
/// side and a `voice` field the TTS resolves on the other, so anything
/// beyond lowercase, digits, `-` and `_` is refused — including `default`,
/// which names the model's built-in voice and must stay unshadowable.
fn valid_voice_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name != "default"
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// At most `cap` bytes off the front of a file — enough for any header
/// walk, never the payload.
fn read_head(path: &std::path::Path, cap: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(cap.min(1 << 20));
    std::fs::File::open(path)?
        .take(cap as u64)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// Seconds of audio in a WAV, from its own header — never from the byte
/// count alone, which a sample rate the page did not promise would skew.
/// A minimal RIFF walk: `fmt ` for the byte rate, `data` for the payload
/// size. Refuses anything that is not integer PCM (`format 1`), because
/// that is what the TTS reads reference clips as.
struct WavInfo {
    seconds: f64,
    /// Byte offset one past the declared end of the `data` chunk — what a
    /// caller holding the *whole* file compares against its real length,
    /// because the duration above is computed from the header's own claim
    /// and a liar header (10s declared over a 100-byte payload) would
    /// otherwise sail past the duration floor. The listing path reads a
    /// bounded head where the payload is absent by design, so the check is
    /// the upload path's to make, not this parser's.
    data_end: usize,
}

fn wav_info(bytes: &[u8]) -> anyhow::Result<WavInfo> {
    use anyhow::{bail, Context};
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    let mut pos = 12usize;
    let mut byte_rate: Option<u32> = None;
    let mut data_len: Option<u32> = None;
    let mut data_end: Option<usize> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        match id {
            b"fmt " => {
                if body + 16 > bytes.len() {
                    bail!("truncated fmt chunk");
                }
                let format = u16::from_le_bytes(bytes[body..body + 2].try_into().unwrap());
                if format != 1 {
                    bail!("only integer PCM WAV is accepted (got format {format})");
                }
                byte_rate = Some(u32::from_le_bytes(
                    bytes[body + 8..body + 12].try_into().unwrap(),
                ));
            }
            b"data" => {
                data_len = Some(len as u32);
                data_end = Some(body + len);
            }
            _ => {}
        }
        // Chunks are word-aligned: an odd length carries a pad byte.
        pos = body + len + (len & 1);
    }
    let rate = byte_rate.context("no fmt chunk")?;
    let data = data_len.context("no data chunk")?;
    if rate == 0 {
        bail!("fmt chunk claims a zero byte rate");
    }
    Ok(WavInfo {
        seconds: f64::from(data) / f64::from(rate),
        data_end: data_end.context("no data chunk")?,
    })
}

/// The duration alone, for a caller that has no payload to check (the
/// bounded-head listing read).
fn wav_seconds(bytes: &[u8]) -> anyhow::Result<f64> {
    Ok(wav_info(bytes)?.seconds)
}

/// Bounds on a cloning reference, in seconds of audio rather than bytes:
/// under ~5s Chatterbox has too little voice to condition on, and past two
/// minutes the extra audio buys nothing while the file stores that much
/// more of somebody's speech.
const MIN_CLONE_SECONDS: f64 = 5.0;
const MAX_CLONE_SECONDS: f64 = 120.0;
/// And one cap in bytes, checked first, so a runaway upload is refused
/// before any parsing: two minutes of 48 kHz mono s16 is ~11.5 MB.
pub const MAX_CLONE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize)]
pub struct CloneQuery {
    name: String,
}

/// POST /api/settings/voice/clone?name=x — body is the WAV itself.
///
/// The file **is** the voice (the TTS resolves `<name>` to this exact WAV
/// as its cloning reference), so the checks are about what lands: a closed
/// name alphabet, integer-PCM WAV only, a duration the model can actually
/// condition on, refuse-don't-overwrite, and temp-sibling-and-rename so a
/// dropped connection cannot leave half a reference the TTS would happily
/// speak garbage from.
pub async fn voice_clone(
    State(state): St,
    axum::extract::Query(q): axum::extract::Query<CloneQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(dir) = state.voices_dir.as_ref() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "voice cloning is not configured — set [web] voices_dir to the host directory the TTS container mounts as /voices\n",
        )
            .into_response();
    };
    // `audio/wav` is not one of CORS's "simple" content types, so requiring
    // it forces any cross-origin caller through a preflight this server
    // never answers — the same protection the Json extractors get from
    // `application/json`, stated rather than inherited, because a raw
    // `Bytes` route otherwise accepts a simple-form POST from any page the
    // owner's tailnet browser happens to have open.
    if headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| !v.starts_with("audio/wav"))
    {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "send the clip as content-type: audio/wav\n",
        )
            .into_response();
    }
    if !valid_voice_name(&q.name) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "a voice name is 1-40 of a-z, 0-9, - or _, and not `default`
",
        )
            .into_response();
    }
    if body.len() > MAX_CLONE_BYTES {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{} bytes is past the {MAX_CLONE_BYTES}-byte cap
",
                body.len()
            ),
        )
            .into_response();
    }
    let info = match wav_info(&body) {
        Ok(i) => i,
        Err(e) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}\n")).into_response();
        }
    };
    // The duration came off the header's own claim; make the payload back
    // it up. A header declaring ten seconds over a hundred bytes would
    // otherwise clear the floor and land a reference the TTS reads as
    // near-silence. The in-page recorder writes honest headers, so the only
    // thing this ever refuses is a hand-crafted or truncated file.
    if info.data_end > body.len() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "the WAV header declares more audio than the file carries ({} of {} bytes)\n",
                body.len(),
                info.data_end
            ),
        )
            .into_response();
    }
    let seconds = info.seconds;
    if !(MIN_CLONE_SECONDS..=MAX_CLONE_SECONDS).contains(&seconds) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{seconds:.1}s of audio — a cloning reference needs {MIN_CLONE_SECONDS:.0}–{MAX_CLONE_SECONDS:.0}s\n"
            ),
        )
            .into_response();
    }
    let path = dir.join(format!("{}.wav", q.name));
    if path.exists() {
        return (
            StatusCode::CONFLICT,
            format!(
                "a voice named `{}` already exists — delete it first, or pick another name
",
                q.name
            ),
        )
            .into_response();
    }
    let tmp = dir.join(format!(".{}.wav.tmp.{}", q.name, request_stamp()));
    let write = std::fs::write(&tmp, &body).and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "{e:#}
"
            ),
        )
            .into_response();
    }
    voice(State(state)).await.into_response()
}

/// POST /api/settings/voice/clone/delete — remove one reference.
///
/// Deleting is offered where recording is, because a botched take that can
/// only be removed at a terminal turns the store into a pile — and the file
/// is a recording of somebody's voice, so removing it must be as easy as
/// adding it was. The same closed name alphabet is the containment: the
/// path is built from a validated stem, never from anything resolvable.
pub async fn voice_clone_delete(State(state): St, Json(q): Json<CloneQuery>) -> Response {
    let Some(dir) = state.voices_dir.as_ref() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "voice cloning is not configured
",
        )
            .into_response();
    };
    if !valid_voice_name(&q.name) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "not a voice name
",
        )
            .into_response();
    }
    let path = dir.join(format!("{}.wav", q.name));
    if !path.is_file() {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "no voice named `{}`
",
                q.name
            ),
        )
            .into_response();
    }
    if let Err(e) = std::fs::remove_file(&path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "{e:#}
"
            ),
        )
            .into_response();
    }
    voice(State(state)).await.into_response()
}

/// One cheap TCP-level probe: any HTTP answer at all means the worker
/// process is up, and nothing more is claimed. The offer endpoint itself is
/// deliberately not exercised — a probe that opened WebRTC sessions to find
/// out would be a load test wearing a health check's clothes.
async fn probe(url: &str) -> bool {
    let base = url.strip_suffix("/api/offer").unwrap_or(url).to_string();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.get(base).send().await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::{valid_voice_name, wav_seconds};

    fn wav(rate: u32, channels: u16, seconds: f64) -> Vec<u8> {
        let byte_rate = rate * u32::from(channels) * 2;
        let data_len = (f64::from(byte_rate) * seconds) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&byte_rate.to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        b.resize(b.len() + data_len as usize, 0);
        b
    }

    /// Duration comes off the header's own byte rate, so the same ten
    /// seconds reads as ten at any sample rate or channel count.
    #[test]
    fn wav_duration_is_read_from_the_header_not_the_byte_count() {
        for (rate, ch) in [(16_000u32, 1u16), (48_000, 1), (44_100, 2)] {
            let s = wav_seconds(&wav(rate, ch, 10.0)).unwrap();
            assert!((s - 10.0).abs() < 0.05, "{rate}Hz/{ch}ch read as {s}");
        }
    }

    #[test]
    fn a_non_wav_and_a_compressed_wav_are_both_refused() {
        assert!(wav_seconds(b"OggS not a wav at all, whatever the name says").is_err());
        let mut float_wav = wav(48_000, 1, 10.0);
        float_wav[20] = 3; // IEEE float, not integer PCM
        assert!(wav_seconds(&float_wav).is_err());
    }

    /// A header may not claim audio the payload does not carry: the
    /// duration floor is computed from the header, so without this check a
    /// 100-byte file declaring ten seconds would land as a reference the
    /// TTS reads as near-silence.
    #[test]
    fn a_wav_header_claiming_more_audio_than_present_is_detectable() {
        let mut liar = wav(16_000, 1, 10.0);
        liar.truncate(200); // header intact, payload gone
        let info = super::wav_info(&liar).unwrap();
        assert!(
            (info.seconds - 10.0).abs() < 0.05,
            "the header still claims 10s"
        );
        assert!(
            info.data_end > liar.len(),
            "data_end must expose the shortfall the upload path refuses on"
        );
        // And an honest file's declared end fits within it.
        let honest = wav(16_000, 1, 10.0);
        assert!(super::wav_info(&honest).unwrap().data_end <= honest.len());
    }

    /// The alphabet is closed and `default` is unshadowable — this name
    /// becomes a path on one side and a TTS `voice` field on the other.
    #[test]
    fn voice_names_are_a_closed_alphabet_and_default_is_reserved() {
        assert!(valid_voice_name("luke"));
        assert!(valid_voice_name("guest_2-b"));
        assert!(!valid_voice_name("default"));
        assert!(!valid_voice_name(""));
        assert!(!valid_voice_name("../escape"));
        assert!(!valid_voice_name("Luke"));
        assert!(!valid_voice_name("a b"));
        assert!(!valid_voice_name(&"x".repeat(41)));
    }

    /// The refusal order is load-bearing: an oversized body must be refused
    /// before the parser runs, and an invalid document must never reach
    /// disk. Exercised through `Charter::parse` directly, because the
    /// handler's other half is filesystem plumbing the charter tests in
    /// `mecha-core` already cover.
    #[test]
    fn an_invalid_charter_is_refused_by_the_same_reader_runs_use() {
        let dup = r#"
[[line]]
id = "a"
text = "first"
[[line]]
id = "a"
text = "second"
"#;
        let e = mecha_core::charter::Charter::parse(dup)
            .unwrap_err()
            .to_string();
        assert!(e.contains("more than once"), "{e}");

        let typod = r#"
[[lines]]
id = "a"
text = "first"
"#;
        assert!(mecha_core::charter::Charter::parse(typod).is_err());
    }
}
