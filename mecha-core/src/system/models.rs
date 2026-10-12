//! Which models are running, what each one holds, and how fast it answered
//! (`docs/SYSTEM-STATE-DESIGN.md` §7.4; the owner's ruling of 2026-10-12).
//!
//! One row per model *server* per minute, for a closed list of servers
//! ([`SERVERS`]): the chat router, the embeddings and OCR servers, speech in
//! and out, and the image server. Each row says whether the server is
//! running, which model it serves, its memory, CPU and GPU memory, and what
//! its own log says it did since the previous sample.
//!
//! **The model's name is recorded; nothing else about the process is.** R13
//! kept every name out of the series; the owner moved model names inside the
//! line on 2026-10-12 ("see what models are active … which model is loaded
//! as some change periodically"). The name is the router's alias for the
//! chat model, and otherwise the *file name* of the model the server was
//! started with — never its directory, its command line, its unit or its
//! pid — and it passes through [`clean_name`], which keeps a short run of
//! plain characters and nothing else.
//!
//! **Speed comes from each server's own log, read as numbers.** The
//! llama-server family prints a timing per request, the speech engines a
//! real-time factor per utterance, ComfyUI the seconds per picture. A fixed
//! parser per kind reads those lines and keeps sums; the text itself is
//! never written anywhere, so a transcript fragment in a log line has no
//! path to the store. A journal that could not be read leaves the speed
//! columns NULL, never zero: "it did nothing" and "we could not look" are
//! opposite findings.
//!
//! **The kinds' numbers are a wire format**, like [`super::Category`]'s: a
//! row stores its kind as a number for seven days, so a number is never
//! reordered or reused, and the `model_kinds` table is rewritten from the
//! enum on every sample for loaders to join.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::source::{self, Ran};

/// What a model server does. Numbers are fixed: never reorder, never reuse
/// one; a new kind takes the next free number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ModelKind {
    Chat = 1,
    Embeddings = 2,
    Ocr = 3,
    SpeechToText = 4,
    TextToSpeech = 5,
    Image = 6,
}

impl ModelKind {
    pub const ALL: [ModelKind; 6] = [
        ModelKind::Chat,
        ModelKind::Embeddings,
        ModelKind::Ocr,
        ModelKind::SpeechToText,
        ModelKind::TextToSpeech,
        ModelKind::Image,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ModelKind::Chat => "chat",
            ModelKind::Embeddings => "embeddings",
            ModelKind::Ocr => "document ocr",
            ModelKind::SpeechToText => "speech to text",
            ModelKind::TextToSpeech => "text to speech",
            ModelKind::Image => "image generation",
        }
    }

    /// A stored number back to a kind; one this build does not know is
    /// `None`, never a neighbour.
    pub fn from_u8(n: u8) -> Option<ModelKind> {
        ModelKind::ALL.into_iter().find(|k| *k as u8 == n)
    }
}

/// Where a server's model name is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameFrom {
    /// The router's `GET /models`: every model whose status is `loaded`.
    Router,
    /// The main process's `-m` / `--model` argument.
    ModelArg,
    /// An environment variable of the main process.
    Env(&'static str),
    /// `[image] diffusion_model` in the configuration.
    ImageConfig,
}

/// One model server: its kind, the units that may run it (the first one
/// installed here is used), and where its model's name comes from.
#[derive(Debug, Clone, Copy)]
pub struct Server {
    pub kind: ModelKind,
    units: &'static [&'static str],
    name: NameFrom,
}

/// The model servers mecha runs, in code: a change here is a review's to
/// see. The same units [`super::category_of`] maps; the socket proxies in
/// front of the on-demand servers hold no model and are left out.
pub const SERVERS: [Server; 6] = [
    Server {
        kind: ModelKind::Chat,
        units: &["llama-local"],
        name: NameFrom::Router,
    },
    Server {
        kind: ModelKind::Embeddings,
        units: &["llama-embed"],
        name: NameFrom::ModelArg,
    },
    Server {
        kind: ModelKind::Ocr,
        units: &["llama-ocr"],
        name: NameFrom::ModelArg,
    },
    Server {
        kind: ModelKind::SpeechToText,
        units: &["mecha-parakeet"],
        name: NameFrom::Env("PARAKEET_DIR"),
    },
    Server {
        kind: ModelKind::TextToSpeech,
        units: &["mecha-breeze-tts"],
        name: NameFrom::Env("BREEZE_GGUF"),
    },
    Server {
        kind: ModelKind::Image,
        // `mecha-comfyui` is the unit `features enable image` installs.
        units: &["comfyui", "mecha-comfyui"],
        name: NameFrom::ImageConfig,
    },
];

/// What a server's log says it did over one window. Sums, so a window of
/// any length folds the same way; rates are derived when the row is
/// written ([`Perf::prompt_tps`] and the rest).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Perf {
    /// Requests that finished in the window.
    pub requests: u64,
    pub prompt_tokens: u64,
    pub prompt_ms: f64,
    pub gen_tokens: u64,
    pub gen_ms: f64,
    /// Speech: seconds of audio heard or spoken, and the milliseconds it
    /// took to process them.
    pub audio_secs: f64,
    pub work_ms: f64,
    /// Pictures: seconds from queue to finished, summed.
    pub job_secs: f64,
}

impl Perf {
    fn per_sec(n: u64, ms: f64) -> Option<f64> {
        (n > 0 && ms > 0.0).then(|| n as f64 / (ms / 1000.0))
    }

    /// Prompt processing, tokens a second, over the window's requests.
    pub fn prompt_tps(&self) -> Option<f64> {
        Perf::per_sec(self.prompt_tokens, self.prompt_ms)
    }

    /// Generation, tokens a second.
    pub fn gen_tps(&self) -> Option<f64> {
        Perf::per_sec(self.gen_tokens, self.gen_ms)
    }

    /// Real-time factor: seconds of work per second of audio (below 1 is
    /// faster than real time).
    pub fn rtf(&self) -> Option<f64> {
        (self.audio_secs > 0.0).then(|| self.work_ms / 1000.0 / self.audio_secs)
    }

    /// Mean seconds per picture.
    pub fn job_secs(&self) -> Option<f64> {
        (self.requests > 0 && self.job_secs > 0.0).then(|| self.job_secs / self.requests as f64)
    }
}

/// One model server, as one sample found it. The unit name lived only in
/// [`collect`]; nothing here carries one.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelLoad {
    pub kind: ModelKind,
    /// The model it serves, when it is running and the name could be read.
    pub model: Option<String>,
    /// Whether `model` is a finding: false when a running server's name
    /// could not be read, so a NULL model is "unknown", never "no model
    /// loaded" (review of #658).
    pub model_known: bool,
    pub running: bool,
    pub mem_bytes: u64,
    /// Cumulative CPU time of the unit; a percentage needs the previous
    /// sample, which the store keeps.
    pub cpu_usec: u64,
    /// `None` when the GPU did not answer.
    pub gpu_mib: Option<u64>,
    /// `None` when the log could not be read.
    pub perf: Option<Perf>,
}

// ---- parsing: pure, so every number is testable without the machine ----

/// A model's name, as the series may hold it: the last path component,
/// without a model file's extension, kept to plain characters and a bounded
/// length. Empty is `None`.
pub fn clean_name(raw: &str) -> Option<String> {
    let base = raw.trim().trim_end_matches('/');
    let base = base.rsplit('/').next().unwrap_or(base);
    let base = [".gguf", ".safetensors", ".onnx", ".bin"]
        .iter()
        .find_map(|ext| base.strip_suffix(ext))
        .unwrap_or(base);
    let kept: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "._+-:".contains(*c))
        .take(96)
        .collect();
    (!kept.is_empty()).then_some(kept)
}

/// The model file a process was started with: the argument after `-m` or
/// `--model`, or `--model=…`, from a NUL-separated `/proc/<pid>/cmdline`.
pub fn model_arg(cmdline: &[u8]) -> Option<String> {
    let args: Vec<String> = cmdline
        .split(|b| *b == 0)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    args.iter().enumerate().find_map(|(i, a)| {
        if a == "-m" || a == "--model" {
            args.get(i + 1).and_then(|v| clean_name(v))
        } else {
            a.strip_prefix("--model=").and_then(clean_name)
        }
    })
}

/// One variable's value from a NUL-separated `/proc/<pid>/environ`, as a
/// model name.
pub fn env_name(environ: &[u8], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    environ
        .split(|b| *b == 0)
        .map(String::from_utf8_lossy)
        .find_map(|kv| kv.strip_prefix(&prefix).and_then(clean_name))
}

/// What `systemctl show -p Id,LoadState,ActiveState,MainPID u…` says about
/// each unit: whether it is installed, whether it runs, and its main pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitState {
    pub installed: bool,
    pub running: bool,
    pub pid: u32,
}

pub fn parse_unit_states(text: &str) -> BTreeMap<String, UnitState> {
    let mut out = BTreeMap::new();
    for block in text.split("\n\n") {
        let mut id = None;
        let mut st = UnitState {
            installed: false,
            running: false,
            pid: 0,
        };
        for line in block.lines() {
            match line.split_once('=') {
                Some(("Id", v)) => id = Some(v.strip_suffix(".service").unwrap_or(v).to_string()),
                Some(("LoadState", v)) => st.installed = v == "loaded",
                Some(("ActiveState", v)) => st.running = v == "active" || v == "reloading",
                Some(("MainPID", v)) => st.pid = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        if let Some(id) = id {
            out.insert(id, st);
        }
    }
    out
}

/// What the router's `/models` listing says, for the chat row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterView {
    /// The resident model(s), when any is.
    pub model: Option<String>,
    /// Whether `model` is a finding: false when the listing was unanswered
    /// or not one this build can fully read.
    pub known: bool,
    /// The resident instances' ports, which their log lines are prefixed
    /// with. `None` when a line cannot be attributed — the listing was not
    /// read, or a resident instance's port is not in it — so the row's speed
    /// is unknown, never a quiet minute.
    pub ports: Option<Vec<u16>>,
}

/// Read a `/models` answer through the provider's own types and rules, so
/// this reader agrees with the other four: resident is
/// [`RouterModel::is_resident`] (`loaded`, `loading`, `sleeping`), and a
/// listing with a status this build does not know supports no conclusion
/// ([`crate::provider::router::readable`]) — "nothing resident" is a claim
/// (review of #658).
pub fn router_view(body: Option<&str>) -> RouterView {
    use crate::provider::router::{readable, RouterModel};
    #[derive(serde::Deserialize)]
    struct Listing {
        #[serde(default)]
        data: Vec<RouterModel>,
    }
    let unread = RouterView {
        model: None,
        known: false,
        ports: None,
    };
    let Some(models) = body
        .and_then(|b| serde_json::from_str::<Listing>(b).ok())
        .map(|l| l.data)
        .filter(|m| readable(m))
    else {
        return unread;
    };
    let mut resident: Vec<(String, Option<u16>)> = models
        .iter()
        .filter(|m| m.is_resident())
        .filter_map(|m| {
            let args = &m.status.args;
            let port = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1).map(String::as_str))
                .or_else(|| args.iter().find_map(|a| a.strip_prefix("--port=")))
                .and_then(|p| p.parse().ok());
            Some((clean_name(&m.id)?, port))
        })
        .collect();
    resident.sort();
    let ports: Option<Vec<u16>> = resident.iter().map(|(_, p)| *p).collect();
    let names: Vec<String> = resident.into_iter().map(|(n, _)| n).collect();
    RouterView {
        model: (!names.is_empty()).then(|| names.join(" + ")),
        known: true,
        ports,
    }
}

/// `journalctl -o json` lines to each unit's messages. A message journald
/// stored as bytes (ANSI colour, as ComfyUI's are) arrives as an array of
/// numbers and is read lossily.
pub fn parse_journal(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(unit) = v["_SYSTEMD_USER_UNIT"].as_str() else {
            continue;
        };
        let msg = match &v["MESSAGE"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(bytes) => String::from_utf8_lossy(
                &bytes
                    .iter()
                    .filter_map(|b| b.as_u64().map(|b| b as u8))
                    .collect::<Vec<u8>>(),
            )
            .into_owned(),
            _ => continue,
        };
        let unit = unit.strip_suffix(".service").unwrap_or(unit);
        out.entry(unit.to_string()).or_default().push(msg);
    }
    out
}

/// The number just before `unit` in `text` after `after`: `"= 163.29 ms"`
/// with `after = "= "`, `unit = " ms"` reads 163.29.
fn number_before(text: &str, after: &str, unit: &str) -> Option<f64> {
    let rest = &text[text.find(after)? + after.len()..];
    rest[..rest.find(unit)?].trim().parse().ok()
}

/// A llama-server timing line's milliseconds and tokens:
/// `eval time =   13015.54 ms /  1257 tokens (…)`.
fn timing(line: &str, label: &str) -> Option<(f64, u64)> {
    let at = line.find(label)?;
    let rest = &line[at + label.len()..];
    let ms = number_before(rest, "=", " ms")?;
    let tokens = number_before(rest, "/", " tokens")? as u64;
    Some((ms, tokens))
}

/// Fold one server's log lines over a window into sums. `ports` limits the
/// router's lines to the instances loaded now: each instance prefixes its
/// lines with `[port]`, and a line from an instance that has since been
/// unloaded belongs to a model this row does not name, so it is left out.
pub fn fold_perf(kind: ModelKind, lines: &[String], ports: Option<&[u16]>) -> Perf {
    let mut p = Perf::default();
    for line in lines {
        if let Some(ports) = ports {
            let ours = ports
                .iter()
                .any(|port| line.trim_start().starts_with(&format!("[{port}]")));
            if !ours {
                continue;
            }
        }
        match kind {
            ModelKind::Chat | ModelKind::Ocr => {
                if !line.contains("print_timing") {
                    continue;
                }
                if let Some((ms, n)) = timing(line, "prompt eval time") {
                    p.prompt_ms += ms;
                    p.prompt_tokens += n;
                } else if let Some((ms, n)) = timing(line, "eval time") {
                    p.gen_ms += ms;
                    p.gen_tokens += n;
                    p.requests += 1;
                }
            }
            ModelKind::Embeddings => {
                let after = "stop processing: n_tokens =";
                let n = line.find(after).and_then(|i| {
                    let rest = &line[i + after.len()..];
                    rest.split(',').next()?.trim().parse::<u64>().ok()
                });
                if let Some(n) = n {
                    p.requests += 1;
                    p.prompt_tokens += n;
                }
            }
            ModelKind::SpeechToText | ModelKind::TextToSpeech => {
                if let (Some(ms), Some(audio)) = (
                    number_before(line, "[Perf] Total", " ms"),
                    number_before(line, "audio ", " s"),
                ) {
                    p.requests += 1;
                    p.work_ms += ms;
                    p.audio_secs += audio;
                }
            }
            ModelKind::Image => {
                if let Some(secs) = number_before(line, "Prompt executed in", " seconds") {
                    p.requests += 1;
                    p.job_secs += secs;
                }
            }
        }
    }
    p
}

// ---- reading the machine ----

/// A loopback HTTP GET, bounded: the body on a 200, `None` otherwise. Plain
/// HTTP/1.0, so the answer is never chunked and ends when the server closes.
pub fn loopback_get(port: u16, path: &str, timeout: Duration) -> Option<String> {
    use std::io::{Read, Write};
    if timeout.is_zero() {
        return None;
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    s.set_write_timeout(Some(timeout)).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").ok()?;
    let mut buf = Vec::new();
    s.take(4 << 20).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.lines().next()?.split_whitespace().nth(1)?;
    (status == "200").then(|| body.to_string())
}

/// Each server's row, from the unit counters and GPU memory the sample
/// already read, the units' states, and their journals since
/// `journal_since` — the previous sample's own instant ([`super::journal_cursor`]),
/// so consecutive windows meet with nothing between them.
///
/// `units` and `gpu_apps` carry unit names; this is the last place they are
/// held, and nothing returned names a unit.
pub fn collect(
    units: &[super::UnitCounters],
    gpu_apps: Option<&[(String, u64)]>,
    journal_since: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
    cfg: Option<&crate::config::Config>,
    budget: impl Fn() -> Duration,
) -> Vec<ModelLoad> {
    let strip = |u: &str| u.strip_suffix(".service").unwrap_or(u).to_string();
    let all_units: Vec<String> = SERVERS
        .iter()
        .flat_map(|s| s.units.iter().map(|u| format!("{u}.service")))
        .collect();
    let mut args = vec!["--user", "show", "-p", "Id,LoadState,ActiveState,MainPID"];
    args.extend(all_units.iter().map(String::as_str));
    let states = match source::run_bounded("systemctl", &args, budget()) {
        Ran::Out(t) => parse_unit_states(&t),
        _ => BTreeMap::new(),
    };

    // The servers here: the first of each one's units that is installed.
    let present: Vec<(&Server, &str, UnitState)> = SERVERS
        .iter()
        .filter_map(|s| {
            s.units.iter().find_map(|u| {
                states
                    .get(*u)
                    .filter(|st| st.installed)
                    .map(|st| (s, *u, *st))
            })
        })
        .collect();

    // One journal read for every server's lines since the previous sample's
    // own instant, to the microsecond, so no line falls between two windows;
    // the first sample, or one after a long gap, looks back one minute.
    let from = journal_since
        .filter(|s| *s < at && at - *s <= chrono::Duration::minutes(10))
        .unwrap_or(at - chrono::Duration::minutes(1));
    let stamp = |t: DateTime<Utc>| format!("@{}.{:06}", t.timestamp(), t.timestamp_subsec_micros());
    let mut jargs: Vec<String> = vec![
        "--user".into(),
        "-q".into(),
        "--no-pager".into(),
        "-o".into(),
        "json".into(),
        "--output-fields=MESSAGE,_SYSTEMD_USER_UNIT".into(),
        format!("--since={}", stamp(from)),
        format!("--until={}", stamp(at)),
    ];
    for (_, unit, _) in &present {
        jargs.push("-u".into());
        jargs.push(format!("{unit}.service"));
    }
    let jrefs: Vec<&str> = jargs.iter().map(String::as_str).collect();
    let journal = if present.is_empty() {
        None
    } else {
        source::run_bounded("journalctl", &jrefs, budget())
            .out()
            .map(|t| parse_journal(&t))
    };

    present
        .into_iter()
        .map(|(server, unit, st)| {
            let (mut mem, mut cpu) = (0, 0);
            for (u, m, c, _) in units {
                if strip(u) == unit {
                    mem += m;
                    cpu += c;
                }
            }
            let gpu_mib = gpu_apps.map(|apps| {
                apps.iter()
                    .filter(|(u, _)| strip(u) == unit)
                    .map(|(_, m)| m)
                    .sum()
            });
            let proc_file = |f: &str| -> Option<Vec<u8>> {
                (st.running && st.pid > 0)
                    .then(|| std::fs::read(format!("/proc/{}/{f}", st.pid)).ok())
                    .flatten()
            };
            // Only the router filters its lines by instance; for it, `None`
            // means no line can be attributed (`RouterView::ports`).
            let mut ports: Option<Vec<u16>> = None;
            let mut unattributable = false;
            let mut known = true;
            let model = if !st.running {
                None
            } else {
                match server.name {
                    NameFrom::Router => {
                        let port = crate::router_unit::Naming::shipped().port;
                        let view = router_view(
                            loopback_get(port, "/models", budget().min(Duration::from_secs(3)))
                                .as_deref(),
                        );
                        unattributable = view.ports.is_none();
                        ports = view.ports;
                        known = view.known;
                        view.model
                    }
                    NameFrom::ModelArg => proc_file("cmdline").and_then(|c| model_arg(&c)),
                    NameFrom::Env(key) => proc_file("environ").and_then(|e| env_name(&e, key)),
                    NameFrom::ImageConfig => {
                        let image = cfg.and_then(|c| c.image.clone()).unwrap_or_default();
                        clean_name(&image.diffusion_model)
                    }
                }
            };
            let model_known =
                !st.running || model.is_some() || (server.name == NameFrom::Router && known);
            let perf = journal.as_ref().filter(|_| !unattributable).map(|j| {
                let lines = j.get(unit).map(Vec::as_slice).unwrap_or(&[]);
                fold_perf(server.kind, lines, ports.as_deref())
            });
            ModelLoad {
                kind: server.kind,
                model,
                model_known,
                running: st.running,
                mem_bytes: mem,
                cpu_usec: cpu,
                gpu_mib,
                perf,
            }
        })
        .collect()
}
