//! Measure before promote: the gate an engine passes before the servers run
//! it (`docs/FEATURES-DESIGN.md` §10.3, step 7b-3).
//!
//! **Each server is measured as it is configured.** The three servers that
//! run llama.cpp — the router, the embeddings server, the OCR server — are
//! systemd user units whose launchers read `${LLAMA_SERVER:-llama-server}`
//! and a port variable. The gate runs each unit's own launcher, with the
//! unit's own environment, on a port of its own, once with the engine the
//! unit runs today and once with the candidate (`LLAMA_SERVER` pointed at
//! it). So the measurement carries the unit's real flags — presets, MTP,
//! context — and never the live ports (:8080, :8081, :8085), whose answer
//! would grade the engine already running and credit the candidate.
//!
//! **One engine at a time, under a held switch.** The router keeps its model
//! resident, so the gate stops it first; it loads the model on one engine,
//! measures, stops it, then the other, so it never holds two copies of the
//! chat model. It does so only after taking the router's switch
//! (`hold::Holds::begin_switch`), which makes runs that start meanwhile wait,
//! and seeing no run live — **if any is, it withdraws and declines** rather
//! than wait (§10.3's departure from `hold.rs`'s wait-forever ruling). A
//! switch already pending (a `mecha model use`) declines the same way.
//!
//! **A smoke test whose model is not installed is not run, never passed.**
//! The ledger row names each leg it ran and each it did not.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a server is for, which decides how the gate asks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Router,
    Embeddings,
    Ocr,
}

/// A server that runs on the engine: its unit, and the variable its launcher
/// reads for the port it listens on.
#[derive(Debug, Clone, Copy)]
pub struct Server {
    pub role: Role,
    pub unit: &'static str,
    pub port_var: &'static str,
}

/// The three servers one engine serves (LLAMA-SERVER.md). The embeddings and
/// OCR units are started on demand by their sockets' proxies; the service is
/// what runs the launcher.
pub const SERVERS: [Server; 3] = [
    Server {
        role: Role::Router,
        unit: "llama-local.service",
        port_var: "MECHA_ROUTER_PORT",
    },
    Server {
        role: Role::Embeddings,
        unit: "llama-embed.service",
        port_var: "MECHA_EMBED_PORT",
    },
    Server {
        role: Role::Ocr,
        unit: "llama-ocr.service",
        port_var: "MECHA_OCR_PORT",
    },
];

/// A unit's launcher and environment, as systemd would run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launcher {
    pub argv: Vec<String>,
    /// The unit's own `Environment=` lines, drop-ins included.
    pub env: Vec<(String, String)>,
}

impl Launcher {
    /// The engine this launcher runs when nothing overrides it: the unit's
    /// `LLAMA_SERVER`, else `llama-server` on its `PATH` — the unit's own, or,
    /// when it names none, the manager's (`base`), which is what systemd
    /// hands it. Read from the environment the launcher is run with, never a
    /// narrower one: a unit without a `path.conf` drop-in starts fine on the
    /// manager's `PATH`, and must not read as having no engine.
    pub fn engine(&self, base: &[(String, String)]) -> Option<PathBuf> {
        if let Some((_, v)) = self.env.iter().find(|(k, _)| k == "LLAMA_SERVER") {
            return Some(PathBuf::from(expand_home(v)));
        }
        // `%h` expanded here as in `LLAMA_SERVER`: one belief about one
        // kind of string (systemd reports both expanded; this is the guard).
        let path = expand_home(&self.env.iter().chain(base).find(|(k, _)| k == "PATH")?.1);
        std::env::split_paths(&path)
            .map(|d| d.join("llama-server"))
            .find(|p| p.is_file())
    }
}

/// `%h` in a value read from a unit, as systemd would expand it.
fn expand_home(v: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    v.replace("%h", &home)
}

/// Read a unit's launcher, or `None` when systemd has no such unit.
pub fn read_launcher(unit: &str) -> Result<Option<Launcher>> {
    let out = std::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "-p",
            "LoadState",
            "-p",
            "ExecStart",
            "-p",
            "Environment",
        ])
        .output()
        .context("running systemctl --user show")?;
    if !out.status.success() {
        bail!(
            "systemctl --user show {unit} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_show(&String::from_utf8_lossy(&out.stdout)))
}

/// `systemctl show`'s `Key=value` lines, for one unit.
fn parse_show(text: &str) -> Option<Launcher> {
    let mut props = BTreeMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            props.insert(k.to_string(), v.to_string());
        }
    }
    if props.get("LoadState").map(String::as_str) != Some("loaded") {
        return None;
    }
    let argv = parse_exec_start(props.get("ExecStart")?)?;
    let env = props
        .get("Environment")
        .map(|e| parse_environment(e))
        .unwrap_or_default();
    Some(Launcher { argv, env })
}

/// `{ path=/x ; argv[]=/x a b ; ignore_errors=no ; … }` → `["/x", "a", "b"]`.
/// A drop-in that resets `ExecStart=` leaves only the effective one, which is
/// the one this reads (the units here have one each).
fn parse_exec_start(v: &str) -> Option<Vec<String>> {
    let argv = v.split(" ; ").find_map(|f| {
        f.trim()
            .trim_start_matches('{')
            .trim()
            .strip_prefix("argv[]=")
            .map(str::to_owned)
    })?;
    let words = split_quoted(&argv);
    (!words.is_empty()).then_some(words)
}

/// `A=1 "B=two words" C=3` → pairs. systemd quotes an assignment that holds
/// a space.
fn parse_environment(v: &str) -> Vec<(String, String)> {
    split_quoted(v)
        .into_iter()
        .filter_map(|w| {
            w.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

fn split_quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The user manager's environment (`systemctl --user show-environment`): what
/// a unit starts with before its own `Environment=` lines.
pub fn manager_env() -> Result<Vec<(String, String)>> {
    let out = std::process::Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .context("running systemctl --user show-environment")?;
    if !out.status.success() {
        bail!("systemctl --user show-environment failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_string(), unquote(v))))
        .collect())
}

/// `show-environment` quotes a value that needs it, as a shell would; the
/// value a process sees has no quotes.
fn unquote(v: &str) -> String {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return v[1..v.len() - 1].to_string();
        }
    }
    v.to_string()
}

/// A free loopback port, asked of the kernel.
fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// One server the gate started: its process group, killed whole on drop, so
/// a router's children go with it and a failed leg never leaves a model
/// holding memory.
struct Running {
    child: std::process::Child,
    port: u16,
    log: PathBuf,
    stopped: bool,
}

impl Running {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(12)..].join("\n")
    }

    /// TERM to the group, then KILL — and wait, so the memory is free before
    /// the next engine loads. **The group is signalled whether or not the
    /// leader is still alive:** a router leader that died leaves its
    /// per-model child (tens of GB) in the group, and that is the case the
    /// group kill exists for. A group id is not reused while the group has
    /// members, so a signal to an emptied group meets `ESRCH`, nothing else.
    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let pgid = self.child.id() as i32;
        // SAFETY: kill(2) with a negative pid signals a process group; no
        // memory is touched.
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        let started = std::time::Instant::now();
        while self.exited().is_none() && started.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(200));
        }
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
        let _ = self.child.wait();
        // The group's other members can outlive its leader by a moment.
        std::thread::sleep(Duration::from_secs(2));
    }
}

impl Drop for Running {
    /// Every way out of a leg — a failed load, a failed bench, a leader that
    /// died — stops the group and waits for it, as the explicit stop does.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start a unit's launcher on a port of its own, with `engine` as
/// `LLAMA_SERVER` when given (else whatever the unit runs).
fn start(
    server: &Server,
    launcher: &Launcher,
    base_env: &[(String, String)],
    engine: Option<&Path>,
    logs: &Path,
    label: &str,
) -> Result<Running> {
    use std::os::unix::process::CommandExt;
    let port = free_port()?;
    std::fs::create_dir_all(logs)?;
    let log = logs.join(format!("{:?}-{label}.log", server.role).to_lowercase());
    let file = std::fs::File::create(&log)?;
    let mut c = std::process::Command::new(expand_home(&launcher.argv[0]));
    c.args(launcher.argv[1..].iter().map(|a| expand_home(a)))
        .env_clear()
        .envs(base_env.iter().cloned())
        .envs(
            launcher
                .env
                .iter()
                .map(|(k, v)| (k.clone(), expand_home(v))),
        )
        .env(server.port_var, port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file)
        .process_group(0);
    if let Some(e) = engine {
        c.env("LLAMA_SERVER", e);
    }
    let child = c
        .spawn()
        .with_context(|| format!("starting {}'s launcher", server.unit))?;
    Ok(Running {
        child,
        port,
        log,
        stopped: false,
    })
}

fn http(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(timeout).build()?)
}

/// POST JSON and read JSON back, the status checked first: a 500 is said as
/// a 500, never as a reply that "came back empty".
async fn post_json(
    client: &reqwest::Client,
    url: String,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{url} answered {status}: {}", text.trim());
    }
    serde_json::from_str(&text).with_context(|| format!("{url} answered no JSON: {}", text.trim()))
}

/// Wait for `/health`, giving up when the process exits or the wait runs out.
async fn ready(r: &mut Running, wait: Duration) -> Result<()> {
    let client = http(Duration::from_secs(5))?;
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = r.exited() {
            bail!("it exited ({status}) before answering:\n{}", r.log_tail());
        }
        if let Ok(resp) = client.get(format!("{}/health", r.base())).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        if started.elapsed() > wait {
            bail!(
                "it did not answer /health within {} s:\n{}",
                wait.as_secs(),
                r.log_tail()
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Three readings of one rate, and their median.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    pub runs: Vec<f64>,
}

impl Rate {
    pub fn median(&self) -> f64 {
        let mut v = self.runs.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    }

    /// The spread as a fraction of the median: the noise one engine's own
    /// runs show, which a difference must clear to mean anything.
    pub fn spread(&self) -> f64 {
        let lo = self.runs.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = self.runs.iter().copied().fold(0.0, f64::max);
        let m = self.median();
        if m > 0.0 && lo.is_finite() {
            (hi - lo) / m
        } else {
            0.0
        }
    }
}

/// The chat model's speed on one engine: generation (`scripts/bench-slots.sh`'s
/// single stream, `ignore_eos` so every run makes the same tokens) and prefill
/// (one long prompt, one token out), each the server's own timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bench {
    pub generation_tps: Rate,
    pub prefill_tps: Rate,
}

const RUNS: usize = 3;
const GEN_TOKENS: u32 = 128;

/// A prompt of roughly `words` words, the same on every engine.
fn long_prompt(words: usize) -> String {
    let sentence = "The quick survey of distributed consensus covers leaders, terms, logs and \
                    the commit index, and why a majority is enough. ";
    let per = sentence.split_whitespace().count();
    sentence.repeat(words.div_ceil(per))
}

async fn timing(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
    key: &str,
) -> Result<f64> {
    let resp = client
        .post(format!("{base}/completion"))
        .json(&body)
        .send()
        .await
        .context("POST /completion")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("reading /completion")?;
    if !status.is_success() {
        bail!("/completion answered {status}: {v}");
    }
    v.get("timings")
        .and_then(|t| t.get(key))
        .and_then(serde_json::Value::as_f64)
        .filter(|r| *r > 0.0)
        .with_context(|| format!("/completion reported no timings.{key}: {v}"))
}

async fn bench(base: &str, model: &str) -> Result<Bench> {
    let client = http(Duration::from_secs(300))?;
    let mut generation = Vec::new();
    let mut prefill = Vec::new();
    for i in 0..RUNS {
        generation.push(
            timing(
                &client,
                base,
                serde_json::json!({
                    "model": model,
                    "prompt": format!("[{i}] Write a detailed technical description of a distributed system."),
                    "n_predict": GEN_TOKENS,
                    "ignore_eos": true,
                    "cache_prompt": false,
                    "temperature": 0.8,
                    "seed": 42,
                }),
                "predicted_per_second",
            )
            .await?,
        );
        prefill.push(
            timing(
                &client,
                base,
                serde_json::json!({
                    "model": model,
                    "prompt": format!("[{i}] {}", long_prompt(3000)),
                    "n_predict": 1,
                    "cache_prompt": false,
                }),
                "prompt_per_second",
            )
            .await?,
        );
    }
    Ok(Bench {
        generation_tps: Rate { runs: generation },
        prefill_tps: Rate { runs: prefill },
    })
}

/// One chat turn, thinking off, that must come back with words: the chat
/// smoke test.
async fn chat_smoke(base: &str, model: &str) -> Result<()> {
    let client = http(Duration::from_secs(300))?;
    let v = post_json(
        &client,
        format!("{base}/v1/chat/completions"),
        serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with the one word: ready"}],
            // Above the presets' reasoning budget, or the reply is a 200 with
            // empty content (LLAMA-SERVER.md, the request contract).
            "max_tokens": crate::provider::LOCAL_MAX_TOKENS,
            "chat_template_kwargs": {"enable_thinking": false},
        }),
    )
    .await?;
    // An answer, in `content`: with the allowance above the reasoning budget,
    // thinking that ran to its end still ends in one, and reasoning alone is
    // not the engine answering.
    let answered = v["choices"][0]["message"]["content"]
        .as_str()
        .is_some_and(|t| !t.trim().is_empty());
    if !answered {
        bail!("the chat turn came back empty: {v}");
    }
    Ok(())
}

async fn embedding(base: &str) -> Result<Vec<f64>> {
    let client = http(Duration::from_secs(120))?;
    let v = post_json(
        &client,
        format!("{base}/v1/embeddings"),
        serde_json::json!({"input": "the layout model reads a scanned page"}),
    )
    .await?;
    let e: Vec<f64> = v["data"][0]["embedding"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_f64).collect())
        .unwrap_or_default();
    if e.is_empty() {
        bail!("no embedding came back: {v}");
    }
    Ok(e)
}

fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// A 48×16 grayscale PNG of dark bars: enough to send through the vision
/// path (mtmd) the chat measurement does not exercise.
const OCR_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAADAAAAAQCAAAAAB1xaNtAAAAHElEQVR42mP4TyJgGMQaGICAKHpUA001DOG0BADVMF2/4IsrXgAAAABJRU5ErkJggg==";

async fn ocr_smoke(base: &str) -> Result<()> {
    let client = http(Duration::from_secs(300))?;
    let v = post_json(
        &client,
        format!("{base}/v1/chat/completions"),
        serde_json::json!({
            "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{OCR_PNG}")}},
                {"type": "text", "text": "OCR:"},
            ]}],
            "max_tokens": 64,
        }),
    )
    .await?;
    // An empty reply is llama-server's 200-with-nothing, not an answer: a
    // vision path that broke would otherwise read as passing on both engines.
    if !v["choices"][0]["message"]["content"]
        .as_str()
        .is_some_and(|t| !t.trim().is_empty())
    {
        bail!("the OCR server answered without words: {v}");
    }
    Ok(())
}

/// What one smoke test came to on one engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Smoke {
    Passed,
    Failed {
        error: String,
    },
    /// Not run, and why — never counted as passed.
    NotRun {
        why: String,
    },
    /// A variant from a newer mecha: read, never counted as passed.
    #[serde(other)]
    Unknown,
}

impl Smoke {
    fn of(r: Result<()>) -> Smoke {
        match r {
            Ok(()) => Smoke::Passed,
            Err(e) => Smoke::Failed {
                error: format!("{e:#}"),
            },
        }
    }
}

/// Everything the gate learned about one engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    /// The engine binary, as run.
    pub engine: PathBuf,
    /// Its `--version` line.
    pub version: String,
    pub bench: Option<Bench>,
    pub smoke: BTreeMap<String, Smoke>,
}

/// The widest the runs of one rate may disagree (as a fraction of their
/// median) and still be a measurement. Past it the gate cannot say faster or
/// slower, and says so rather than pass.
pub const MAX_SPREAD: f64 = 0.25;

/// The band "no slower" allows: the old engine's own noise, at least 3 % and
/// at most 10 %. Never the candidate's — its instability must not widen its
/// own acceptance.
fn band(old: &Rate) -> f64 {
    old.spread().clamp(0.03, 0.10)
}

/// The rule a promotion passes: every smoke the old engine passed, the new
/// one passes too; both engines' runs agree well enough to be a measurement;
/// and the new one is no slower, on generation or prefill, beyond the old
/// engine's own noise (3 % to 10 %).
pub fn verdict(old: &Leg, new: &Leg) -> std::result::Result<(), String> {
    for (name, was) in &old.smoke {
        let now = new.smoke.get(name);
        if matches!(was, Smoke::Passed) && !matches!(now, Some(Smoke::Passed)) {
            return Err(format!(
                "the {name} smoke test passes on the old engine and not on the new: {}",
                match now {
                    Some(Smoke::Failed { error }) => error.clone(),
                    Some(Smoke::NotRun { why }) => why.clone(),
                    _ => "not run".into(),
                }
            ));
        }
    }
    let (Some(a), Some(b)) = (&old.bench, &new.bench) else {
        return Err("the chat model was not measured on both engines".into());
    };
    for (what, x, y) in [
        ("generation", &a.generation_tps, &b.generation_tps),
        ("prefill", &a.prefill_tps, &b.prefill_tps),
    ] {
        for (which, r) in [("old", x), ("new", y)] {
            if r.spread() > MAX_SPREAD {
                return Err(format!(
                    "the {which} engine's {what} runs disagree by {:.0} % — not a measurement; \
                     run this again when the machine is quiet",
                    r.spread() * 100.0
                ));
            }
        }
        let noise = band(x);
        if y.median() < x.median() * (1.0 - noise) {
            return Err(format!(
                "{what} is slower: {:.1} tok/s against {:.1} (noise {:.0} %)",
                y.median(),
                x.median(),
                noise * 100.0
            ));
        }
    }
    Ok(())
}

/// One row of the engine ledger: what was measured, and what came of it —
/// which is what turns "updates help" into measured rows (§10.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerRow {
    pub at: DateTime<Utc>,
    pub action: String,
    /// The chat model the router measured.
    pub model: Option<String>,
    pub old: Leg,
    pub new: Leg,
    pub outcome: Outcome,
}

/// How a gated change ended. A partial one names the step that failed, so it
/// never reads as complete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Promoted,
    PromotedByForce {
        why: String,
    },
    Kept {
        why: String,
    },
    Partial {
        step: String,
        error: String,
        finish: String,
    },
    /// A `--rollback`: no measurement, the servers moved back to `to`.
    RolledBack {
        to: String,
    },
    /// A variant from a newer mecha: the row still reads.
    #[serde(other)]
    Unknown,
}

pub fn ledger_path(mecha_home: &Path) -> PathBuf {
    crate::engine::engine_root(mecha_home).join("ledger.jsonl")
}

pub fn append_ledger(mecha_home: &Path, row: &LedgerRow) -> Result<()> {
    use std::io::Write;
    let p = ledger_path(mecha_home);
    std::fs::create_dir_all(p.parent().context("ledger has no parent")?)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)?;
    writeln!(f, "{}", serde_json::to_string(row)?)?;
    Ok(())
}

/// The ledger's rows, oldest first. Absent is empty; unreadable is an error —
/// an unreadable record is a finding, never "never measured". A row this
/// build cannot parse at all is counted in `skipped`, not dropped in silence;
/// an unknown outcome or smoke variant reads as `Unknown` and keeps its row.
pub fn read_ledger(mecha_home: &Path) -> Result<Ledger> {
    let p = ledger_path(mecha_home);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", p.display())),
    };
    let mut ledger = Ledger::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(row) => ledger.rows.push(row),
            Err(_) => ledger.skipped += 1,
        }
    }
    Ok(ledger)
}

/// The ledger as read: its rows, and how many lines would not parse.
#[derive(Debug, Default)]
pub struct Ledger {
    pub rows: Vec<LedgerRow>,
    pub skipped: usize,
}

/// An engine's `--version` line, for the record.
pub fn version_of(engine: &Path) -> String {
    std::process::Command::new(engine)
        .arg("--version")
        .output()
        .map(|o| {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            text.lines()
                .find(|l| l.contains("version:"))
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

/// What the gate needs to know about this machine's servers, read once.
pub struct Servers {
    /// Each present unit, with its launcher. An absent one is a smoke test
    /// not run.
    pub present: Vec<(Server, Launcher)>,
    pub base_env: Vec<(String, String)>,
}

impl Servers {
    pub fn read() -> Result<Servers> {
        let mut present = Vec::new();
        for s in SERVERS {
            if let Some(l) = read_launcher(s.unit)? {
                present.push((s, l));
            }
        }
        Ok(Servers {
            present,
            base_env: manager_env()?,
        })
    }

    /// For a read-only view: a machine where `systemctl --user` cannot be
    /// asked (macOS, or no user session bus) has no units to show, which is
    /// said, not an error — `doctor`'s rule for an absent init system. The
    /// moves (`--adopt`, `--rollback`) use [`Servers::read`], which fails.
    pub fn read_or_none() -> (Servers, Option<String>) {
        match Servers::read() {
            Ok(s) => (s, None),
            Err(e) => (
                Servers {
                    present: Vec::new(),
                    base_env: Vec::new(),
                },
                Some(format!("{e:#}")),
            ),
        }
    }

    pub fn get(&self, role: Role) -> Option<&(Server, Launcher)> {
        self.present.iter().find(|(s, _)| s.role == role)
    }

    /// The engine a server's unit runs today.
    pub fn engine(&self, role: Role) -> Option<PathBuf> {
        self.get(role).and_then(|(_, l)| l.engine(&self.base_env))
    }
}

/// Measure one engine: the router's model (bench and chat smoke), then the
/// embeddings and OCR servers' smoke tests, each started and stopped in turn.
/// `engine` `None` is whatever the units run today. The embedding comes back
/// too, so the new engine's can be held against the old's.
async fn measure(
    servers: &Servers,
    engine: Option<&Path>,
    model: Option<&str>,
    logs: &Path,
    label: &str,
    say: &mut dyn FnMut(&str),
) -> Result<(Leg, Option<Vec<f64>>)> {
    let binary = match engine {
        Some(e) => e.to_path_buf(),
        None => servers
            .engine(Role::Router)
            .context("the router's unit names no llama-server it can find")?,
    };
    let mut leg = Leg {
        version: version_of(&binary),
        engine: binary,
        bench: None,
        smoke: BTreeMap::new(),
    };
    if let (Some((s, l)), Some(model)) = (servers.get(Role::Router), model) {
        say(&format!("{label}: loading {model} on its own router"));
        let mut r = start(s, l, &servers.base_env, engine, logs, label)?;
        ready(&mut r, Duration::from_secs(180))
            .await
            .context("the router did not come up")?;
        crate::provider::router::load(&r.base(), model, Duration::from_secs(900))
            .await
            .with_context(|| format!("loading {model}"))?;
        let chat = chat_smoke(&r.base(), model).await;
        leg.smoke.insert("chat".into(), Smoke::of(chat));
        say(&format!("{label}: measuring generation and prefill"));
        leg.bench = Some(bench(&r.base(), model).await?);
        r.stop();
    } else {
        leg.smoke.insert(
            "chat".into(),
            Smoke::NotRun {
                why: if servers.get(Role::Router).is_none() {
                    "no router unit on this machine".into()
                } else {
                    "the router had no model loaded, and the provider names none".into()
                },
            },
        );
    }
    let mut vector = None;
    for role in [Role::Embeddings, Role::Ocr] {
        let name = match role {
            Role::Embeddings => "embeddings",
            _ => "ocr",
        };
        let Some((s, l)) = servers.get(role) else {
            leg.smoke.insert(
                name.into(),
                Smoke::NotRun {
                    why: format!(
                        "no {} unit on this machine",
                        SERVERS
                            .iter()
                            .find(|x| x.role == role)
                            .map_or("", |x| x.unit)
                    ),
                },
            );
            continue;
        };
        say(&format!("{label}: {name} smoke test"));
        let outcome = async {
            let mut r = start(s, l, &servers.base_env, engine, logs, label)?;
            ready(&mut r, Duration::from_secs(180)).await?;
            let out = match role {
                Role::Embeddings => embedding(&r.base()).await.map(|v| vector = Some(v)),
                _ => ocr_smoke(&r.base()).await,
            };
            r.stop();
            out
        }
        .await;
        leg.smoke.insert(name.into(), Smoke::of(outcome));
    }
    Ok((leg, vector))
}

/// The embedding the new engine gives must be the old one's: llama.cpp is
/// deterministic for one input on one model, and a different vector is a
/// changed graph, whatever its speed.
const EMBED_AGREE: f64 = 0.999;

/// Hold the embedding comparison against the new leg.
fn compare_embeddings(new: &mut Leg, old_v: Option<&[f64]>, new_v: Option<&[f64]>) {
    if let (Some(a), Some(b)) = (old_v, new_v) {
        let c = cosine(a, b);
        if c < EMBED_AGREE {
            new.smoke.insert(
                "embeddings".into(),
                Smoke::Failed {
                    error: format!("its embedding differs from the old engine's (cosine {c:.6})"),
                },
            );
        }
    }
}

/// The drop-in `--adopt` writes into each unit: its launcher runs mecha's
/// managed engine through the `current` link.
/// The path is `managed` itself, absolute — the one `install_engine`
/// wrote under this mecha home, `MECHA_HOME` honoured — never re-derived from
/// `%h`, which would name another tree under a non-default home.
pub fn drop_in_text(managed: &Path) -> String {
    format!(
        "# Written by `mecha setup engine --adopt` (FEATURES-DESIGN.md §10.3): this\n\
         # unit's launcher runs mecha's managed llama.cpp engine through the\n\
         # `current` link. Removing this file (`mecha setup engine --rollback`)\n\
         # returns it to the llama-server on its PATH, which is left in place.\n\
         [Service]\n\
         Environment=\"LLAMA_SERVER={}\"\n",
        managed.display()
    )
}

/// Where a unit's adopt drop-in lives, under the user unit directory systemd
/// reads — `Machinery::unit_dirs`' first, which honours `XDG_CONFIG_HOME`.
pub fn drop_in_path(unit_dir: &Path, unit: &str) -> PathBuf {
    unit_dir.join(format!("{unit}.d")).join("mecha-engine.conf")
}

/// The user unit directory drop-ins are written into.
pub fn unit_dir(m: &crate::sidecar::Machinery) -> Result<&Path> {
    m.unit_dirs
        .first()
        .map(PathBuf::as_path)
        .context("no user unit directory (no XDG config directory) to write a drop-in into")
}

/// `systemctl --user <verb> <units…>`, named on failure.
pub fn systemctl(verb: &str, units: &[&str]) -> Result<()> {
    let out = std::process::Command::new("systemctl")
        .arg("--user")
        .arg(verb)
        .args(units)
        .output()
        .with_context(|| format!("running systemctl --user {verb}"))?;
    if !out.status.success() {
        bail!(
            "systemctl --user {verb} {} failed: {}",
            units.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Whether the router serving `base` runs `model` on `engine`: its `/models`
/// entry names the binary it started the child with.
pub async fn router_runs(base: &str, model: &str, engine: &Path) -> Option<bool> {
    let models = crate::provider::router::models(base).await?;
    let m = models.iter().find(|m| m.id == model && m.is_resident())?;
    let arg0 = m.status.args.first()?;
    if Path::new(arg0) == engine {
        return Some(true);
    }
    // Through the `current` link: the same file either way. A path that
    // will not resolve is not this engine.
    Some(
        match (std::fs::canonicalize(arg0), std::fs::canonicalize(engine)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        },
    )
}

/// The managed engine's binary, as an adopted unit names it.
pub fn managed_binary(mecha_home: &Path) -> PathBuf {
    crate::engine::current(mecha_home).join("llama-server")
}

/// Whether every present unit already runs the managed engine.
pub fn adopted(servers: &Servers, mecha_home: &Path) -> bool {
    !servers.present.is_empty() && servers.present.len() == adopted_units(servers, mecha_home)
}

/// How many present units run the managed engine.
pub fn adopted_units(servers: &Servers, mecha_home: &Path) -> usize {
    let managed = managed_binary(mecha_home);
    servers
        .present
        .iter()
        .filter(|(_, l)| l.engine(&servers.base_env).as_deref() == Some(managed.as_path()))
        .count()
}

/// Wait for the router at `base` to answer, then load `model` on it and
/// confirm it runs on `engine` — what "the router answers on the winning
/// engine" means, asked of the router rather than inferred from a restart.
pub async fn router_back(base: &str, model: Option<&str>, engine: &Path) -> Result<()> {
    let client = http(Duration::from_secs(5))?;
    let started = std::time::Instant::now();
    while !client
        .get(format!("{base}/health"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
    {
        if started.elapsed() > Duration::from_secs(180) {
            bail!("the router at {base} did not answer /health within 180 s");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if let Some(model) = model {
        crate::provider::router::load(base, model, Duration::from_secs(900))
            .await
            .with_context(|| format!("loading {model} on the restarted router"))?;
        match router_runs(base, model, engine).await {
            Some(true) => {}
            Some(false) => bail!(
                "the router serves {model} on another engine than {}",
                engine.display()
            ),
            None => bail!("the router at {base} does not say which engine runs {model}"),
        }
    }
    Ok(())
}

/// Decline before anything is downloaded or moved when the router is busy:
/// a switch already pending, or a run holding it. Cheap, and asked again
/// under the switch itself ([`take_switch`]) after the download.
pub fn preflight(holds: &crate::hold::Holds, base: &str) -> Result<()> {
    if let Some(other) = holds.pending(base) {
        bail!(
            "a switch to {} is already waiting on {base} (pid {}) — nothing was measured; run \
             this again once it is done",
            other.to,
            other.pid
        );
    }
    let live = holds.live(base);
    if !live.is_empty() {
        let what: Vec<String> = live.iter().map(|h| h.what.clone()).collect();
        bail!(
            "{} run(s) hold the router ({}) — nothing was measured or moved; run this again \
             when the box is quiet",
            live.len(),
            what.join(", ")
        );
    }
    Ok(())
}

/// Take the router's switch — written before the holds are read, so a run
/// starting meanwhile waits on it — and decline, withdrawing it, if a run
/// holds the router: the gate declines rather than waits (§10.3).
pub fn take_switch(
    holds: &crate::hold::Holds,
    base: &str,
    from: &str,
    to: &str,
) -> Result<crate::hold::Switching> {
    let switching = match holds.begin_switch(base, Some(from), to)? {
        Ok(s) => s,
        Err(other) => bail!(
            "a switch to {} is already waiting on {base} (pid {}) — nothing was measured; run \
             this again once it is done",
            other.to,
            other.pid
        ),
    };
    let live = holds.live(base);
    if !live.is_empty() {
        drop(switching);
        let what: Vec<String> = live.iter().map(|h| h.what.clone()).collect();
        bail!(
            "{} run(s) hold the router ({}) — nothing was measured or moved; run this again \
             when the box is quiet",
            live.len(),
            what.join(", ")
        );
    }
    switching.past_the_wait()?;
    Ok(switching)
}

/// The model to measure: the one the owner has loaded, else the provider's.
/// A `/models` answer this build cannot fully read is a finding, never "none
/// loaded" (`router::readable`'s rule) — a newer llama.cpp's status would
/// otherwise swap the owner's model for the configured one. And with nothing
/// to measure, the router is not stopped for nothing.
pub fn measured_model(
    list: Option<&[crate::provider::router::RouterModel]>,
    fallback: Option<&str>,
) -> Result<String> {
    let resident = match list {
        // An empty list is a router with no presets answering: nothing loaded.
        Some([]) => None,
        Some(l) if !crate::provider::router::readable(l) => bail!(
            "the router's model list has a status this build does not know, so which model is \
             loaded is unknown — nothing was measured or moved"
        ),
        Some(l) => crate::provider::router::resident(l).map(str::to_owned),
        None => None,
    };
    resident.or_else(|| fallback.map(str::to_owned)).context(
        "no model is loaded and the provider names none, so there is nothing to measure — load \
         one (`mecha model use <name>`) and run this again",
    )
}

/// The ledger row for an adopt whose old engine could not be measured — the
/// router was stopped, so the attempt happened, and nothing moved.
fn not_measured(
    action: &str,
    error: &anyhow::Error,
    back: &Result<()>,
    model: Option<String>,
    old_engine: &Path,
    managed: &Path,
) -> LedgerRow {
    let failed = |engine: &Path, smoke: Smoke| Leg {
        version: version_of(engine),
        engine: engine.to_path_buf(),
        bench: None,
        smoke: [("chat".to_string(), smoke)].into(),
    };
    LedgerRow {
        at: Utc::now(),
        action: action.into(),
        model,
        old: failed(
            old_engine,
            Smoke::Failed {
                error: format!("{error:#}"),
            },
        ),
        new: failed(
            managed,
            Smoke::NotRun {
                why: "not measured: the engine the units run today could not be".into(),
            },
        ),
        outcome: match back {
            Ok(()) => Outcome::Kept {
                why: format!("the engine the units run today could not be measured: {error:#}"),
            },
            Err(b) => Outcome::Partial {
                step: "restarting the router after the measurement failed".into(),
                error: format!("{b:#}"),
                finish: "systemctl --user start llama-local.service".into(),
            },
        },
    }
}

/// Every reason `--adopt` would refuse before it moves anything — asked by
/// `adopt` itself and, before its prompt, by the CLI, so the owner is never
/// asked to agree to a router stop that is then refused. The engine the
/// router runs today, when it may go ahead.
pub fn check_adoptable(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    holds: &crate::hold::Holds,
    base: &str,
) -> Result<PathBuf> {
    if servers.get(Role::Router).is_none() {
        bail!(
            "there is no llama-local.service to adopt — on a clean machine the units arrive \
             with step 7c, and run the managed engine from the start"
        );
    }
    if adopted(servers, &m.mecha_home) {
        bail!("every llama.cpp unit here already runs mecha's engine — nothing to adopt");
    }
    // Part-adopted: the install repoints `current`, which an adopted unit's
    // drop-in names, so its "old" leg would run the candidate and the
    // measurement would compare the new engine with itself.
    let some = adopted_units(servers, &m.mecha_home);
    if some > 0 {
        bail!(
            "{some} of these units already run mecha's engine — `mecha setup engine \
             --rollback` first, then `--adopt` measures every unit against the engine it ran \
             before"
        );
    }
    // A router whose engine cannot be found has nothing to measure against.
    let old_engine = servers
        .engine(Role::Router)
        .context("the router's unit names no llama-server it can find")?;
    preflight(holds, base)?;
    Ok(old_engine)
}

/// The router's model list for the gate. A router that answers `/health` but
/// not `/models` — a two-second read lost on a busy machine — is a finding,
/// not "no router": falling through to the configured model would measure a
/// model the owner does not have and then load it in place of theirs. Only a
/// router that answers nothing at all reads as down.
pub async fn listed_models(
    base: &str,
) -> Result<Option<Vec<crate::provider::router::RouterModel>>> {
    for attempt in 0..3 {
        if let Some(list) = crate::provider::router::models(base).await {
            return Ok(Some(list));
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    let up = http(Duration::from_secs(5))?
        .get(format!("{base}/health"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    if up {
        bail!(
            "the router at {base} answers /health but not /models, so which model is loaded is \
             unknown — nothing was measured or moved"
        );
    }
    Ok(None)
}

/// What a gated change is: the labels its switch and ledger carry, the
/// candidate the new leg runs, the build the router must report once it is
/// promoted, and how it is promoted. Adopt and upgrade differ only here.
pub struct Change {
    pub action: &'static str,
    pub from: String,
    pub to: String,
    /// The candidate's binary, as the new leg runs it.
    pub candidate: PathBuf,
    /// What the router's model must report (`/props` `build_info`) after a
    /// promotion: the model's own process saying which engine it runs.
    pub build: u32,
    pub commit: String,
    pub promotion: Promotion,
}

/// How a winning candidate reaches the servers.
pub enum Promotion {
    /// A drop-in per unit names the managed engine (`current`).
    Adopt,
    /// `current` moves to the new build; `previous` keeps the old one.
    Upgrade { from_tag: String, to_tag: String },
}

impl Promotion {
    /// The command that finishes, or undoes, a promotion a step of which failed.
    fn finish(&self) -> &'static str {
        match self {
            Promotion::Adopt => {
                "systemctl --user daemon-reload && systemctl --user restart llama-local.service \
                 (and `mecha setup engine --rollback` to undo)"
            }
            Promotion::Upgrade { .. } => {
                "systemctl --user restart llama-local.service (and `mecha setup engine \
                 --rollback` to go back to the previous build)"
            }
        }
    }
}

/// The ledger row a rollback writes: no measurement, both engines named, so
/// the status's last line never still says "promoted" after one.
pub fn rollback_row(from: &Path, to: &Path, model: Option<String>) -> LedgerRow {
    let leg = |e: &Path| Leg {
        version: version_of(e),
        engine: e.to_path_buf(),
        bench: None,
        smoke: BTreeMap::new(),
    };
    LedgerRow {
        at: Utc::now(),
        action: "rollback".into(),
        model,
        old: leg(from),
        new: leg(to),
        outcome: Outcome::RolledBack {
            to: to.display().to_string(),
        },
    }
}

/// Ask the router which build serves `model` — `/props` with `autoload=false`,
/// so the question loads nothing — and require it to be the one expected.
pub async fn verify_build(base: &str, model: &str, build: u32, commit: &str) -> Result<()> {
    let resp = http(Duration::from_secs(10))?
        .get(format!("{base}/props"))
        .query(&[("model", model), ("autoload", "false")])
        .send()
        .await
        .context("asking the router which build serves the model")?;
    let status = resp.status();
    if !status.is_success() {
        bail!(
            "the router answered {status} when asked which build serves {model}: {}",
            resp.text().await.unwrap_or_default().trim()
        );
    }
    let v: serde_json::Value = resp.json().await.context("reading /props")?;
    let info = v["build_info"].as_str().unwrap_or("");
    if !crate::engine::build_info_is(info, build, commit) {
        bail!(
            "the router's {model} reports build `{info}`, not b{build} at {}",
            &commit[..commit.len().min(9)]
        );
    }
    Ok(())
}

/// `mecha setup engine --adopt` (§10.3): measure the engine the units run
/// today against the shipped pin, and — when the pin is no slower and passes
/// every smoke the old one passed, or with `force` — point every present unit
/// at the managed engine through a drop-in, leaving the hand install in place
/// as the first rollback. Runs under the router's switch from before the
/// first measurement to after the router answers on the winner; declines,
/// moving nothing, when a switch is pending or a run holds the router.
#[allow(clippy::too_many_arguments)]
pub async fn adopt(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    holds: &crate::hold::Holds,
    base_url: &str,
    fallback_model: Option<&str>,
    force: bool,
    cancel: &tokio_util::sync::CancellationToken,
    say: &mut dyn FnMut(&str),
) -> Result<LedgerRow> {
    let base = crate::provider::router::base(base_url);
    let old_engine = check_adoptable(m, servers, holds, &base)?;
    // The pin, side by side: nothing reads `current` until a drop-in names it.
    // Downloaded before the switch is taken — holding it through a download
    // would make every run that starts meanwhile wait on it.
    tokio::select! {
        r = crate::engine::install_engine(m, &mut *say) => { r?; }
        () = cancel.cancelled() => bail!(
            "interrupted while fetching — nothing was moved, and the download resumes next time"
        ),
    }
    let change = Change {
        action: "adopt",
        from: "llama.cpp (provided)".into(),
        to: format!("llama.cpp {}", crate::engine::PIN.tag),
        // The pin's own build, not whatever `current` names: the pin's
        // install leaves an upgraded `current` alone, so after upgrades and
        // rollbacks the link can name another build — and what is measured
        // must be what the promotion then asserts.
        candidate: crate::engine::engine_root(&m.mecha_home)
            .join(crate::engine::PIN.tag)
            .join("llama-server"),
        build: crate::engine::PIN.build,
        commit: crate::engine::PIN.commit.into(),
        promotion: Promotion::Adopt,
    };
    run_gate(
        m,
        servers,
        holds,
        &base,
        &old_engine,
        fallback_model,
        force,
        cancel,
        say,
        change,
    )
    .await
}

/// Every reason `--upgrade` would refuse before it moves anything: the units
/// must already run mecha's engine (on a provided engine a promotion no
/// server reads would record an upgrade that reached nothing — `--adopt` is
/// that machine's step), and the router must be quiet. The engine the router
/// runs today and the build `current` names, when it may go ahead.
pub fn check_upgradable(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    holds: &crate::hold::Holds,
    base: &str,
) -> Result<(PathBuf, String)> {
    if servers.get(Role::Router).is_none() {
        bail!("there is no llama-local.service whose engine could be upgraded");
    }
    if !adopted(servers, &m.mecha_home) {
        bail!(
            "these units run a llama.cpp installed by hand — `mecha setup engine --adopt` moves \
             them onto mecha's engine first; an upgrade moves mecha's engine, and only once every \
             unit runs it"
        );
    }
    let old_tag = crate::engine::link_tag(&crate::engine::current(&m.mecha_home))
        .context("mecha's `current` link names no build")?;
    let old_engine = servers
        .engine(Role::Router)
        .context("the router's unit names no llama-server it can find")?;
    preflight(holds, base)?;
    Ok((old_engine, old_tag))
}

/// `mecha setup engine --upgrade` (§10.3, F10): install the release the owner
/// confirmed beside the current build, measure it against the current one
/// under the same gate as an adopt, and promote it — `current` moved to it,
/// `previous` keeping the old build for `--rollback` — only when it is no
/// slower and passes every smoke the old one passed, or with `force`.
#[allow(clippy::too_many_arguments)]
pub async fn upgrade(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    holds: &crate::hold::Holds,
    base_url: &str,
    spec: &crate::engine::Spec,
    target: crate::engine::Target,
    fallback_model: Option<&str>,
    force: bool,
    cancel: &tokio_util::sync::CancellationToken,
    say: &mut dyn FnMut(&str),
) -> Result<LedgerRow> {
    let base = crate::provider::router::base(base_url);
    let (old_engine, old_tag) = check_upgradable(m, servers, holds, &base)?;
    if spec.tag == old_tag {
        bail!("the units already run llama.cpp {old_tag}");
    }
    let dir = tokio::select! {
        r = crate::engine::install_build(m, target, spec, &mut *say) => r?,
        () = cancel.cancelled() => bail!(
            "interrupted while fetching — nothing was moved, and the download resumes next time"
        ),
    };
    let change = Change {
        action: "upgrade",
        from: format!("llama.cpp {old_tag}"),
        to: format!("llama.cpp {}", spec.tag),
        candidate: dir.join("llama-server"),
        build: spec.build,
        commit: spec.commit.clone(),
        promotion: Promotion::Upgrade {
            from_tag: old_tag,
            to_tag: spec.tag.clone(),
        },
    };
    run_gate(
        m,
        servers,
        holds,
        &base,
        &old_engine,
        fallback_model,
        force,
        cancel,
        say,
        change,
    )
    .await
}

/// The gate both moves share: take the switch (declining on a busy router),
/// read the model the owner has loaded, stop the router, measure the old
/// engine and then the candidate — each leg interruptible on its own — and
/// promote or keep, recording the result either way. The switch is held from
/// before the first load to after the router answers on the winner.
#[allow(clippy::too_many_arguments)]
async fn run_gate(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    holds: &crate::hold::Holds,
    base: &str,
    old_engine: &Path,
    fallback_model: Option<&str>,
    force: bool,
    cancel: &tokio_util::sync::CancellationToken,
    say: &mut dyn FnMut(&str),
    change: Change,
) -> Result<LedgerRow> {
    let switching = take_switch(holds, base, &change.from, &change.to)?;
    // What the owner is using is what gets measured, read before anything
    // stops: an answer that cannot be read declines here.
    let listed = listed_models(base).await?;
    let model = measured_model(listed.as_deref(), fallback_model)?;
    say("stopping the router for the measurement");
    systemctl("stop", &["llama-local.service"])?;
    let logs = crate::engine::engine_root(&m.mecha_home).join("gate");
    let candidate = &change.candidate;
    // Each leg is interruptible on its own, so an interrupt after a good old
    // leg is recorded as that — measured, then abandoned — never as an old
    // engine that could not be measured. Dropping a leg drops its servers,
    // whose guards stop each one's whole group.
    let old_measured = tokio::select! {
        r = measure(servers, None, Some(&model), &logs, "old", say) => r,
        () = cancel.cancelled() => Err(anyhow::anyhow!("interrupted while measuring the engine the units run today")),
    };
    let (old, old_v) = match old_measured {
        Ok(x) => x,
        Err(e) => {
            // Nothing moved, so the router goes back as it was, with the model
            // that was loaded — and the attempt is recorded: "never measured"
            // and "could not be measured" are different findings.
            let back = match systemctl("start", &["llama-local.service"]) {
                Ok(()) => router_back(base, Some(&model), old_engine).await,
                Err(e) => Err(e),
            };
            let row = not_measured(
                change.action,
                &e,
                &back,
                Some(model.clone()),
                old_engine,
                candidate,
            );
            if let Err(le) = append_ledger(&m.mecha_home, &row) {
                say(&format!(
                    "the attempt could not be written to {}: {le:#}",
                    ledger_path(&m.mecha_home).display()
                ));
            }
            // The candidate is kept: nothing was decided about it, and the
            // next run finds it unpacked rather than fetching it again.
            return Err(e.context(match back {
                Ok(()) => "measuring the engine the units run today — the router was restarted \
                           on it, and nothing was changed"
                    .to_string(),
                Err(b) => format!(
                    "measuring the engine the units run today, and restarting the router \
                     failed too ({b:#}) — `systemctl --user start llama-local.service`"
                ),
            }));
        }
    };
    let new = tokio::select! {
        r = measure(servers, Some(candidate), Some(&model), &logs, "new", say) => r.map(Some),
        () = cancel.cancelled() => Ok(None),
    };
    let interrupted = matches!(new, Ok(None));
    let failed_leg = |smoke: Smoke| Leg {
        version: version_of(candidate),
        engine: candidate.clone(),
        bench: None,
        smoke: [("chat".to_string(), smoke)].into(),
    };
    let new = match new {
        Ok(Some((mut leg, new_v))) => {
            compare_embeddings(&mut leg, old_v.as_deref(), new_v.as_deref());
            leg
        }
        Ok(None) => failed_leg(Smoke::NotRun {
            why: "interrupted before the new engine was measured".into(),
        }),
        Err(e) => failed_leg(Smoke::Failed {
            error: format!("{e:#}"),
        }),
    };

    let decision = if interrupted {
        Err("interrupted after the old engine was measured, before the new one was".to_string())
    } else {
        verdict(&old, &new)
    };
    // `--force` overrides a measurement, never its absence: an engine that
    // produced no bench at all did not start well enough to be measured.
    let promote = decision.is_ok() || (force && new.bench.is_some());
    let outcome = if promote {
        say(&format!("promoting: the servers move to {}", change.to));
        let promoted = match &change.promotion {
            Promotion::Adopt => promote_adopt(m, servers, base, &model).await,
            Promotion::Upgrade { from_tag, to_tag } => {
                promote_upgrade(m, servers, base, &model, from_tag, to_tag).await
            }
        };
        let promoted = match promoted {
            Ok(()) => verify_build(base, &model, change.build, &change.commit)
                .await
                .map_err(|e| ("the router's build after the promotion".to_string(), e)),
            Err(x) => Err(x),
        };
        match promoted {
            Ok(()) => match decision {
                Ok(()) => Outcome::Promoted,
                Err(why) => Outcome::PromotedByForce { why },
            },
            Err((step, e)) => Outcome::Partial {
                step,
                error: format!("{e:#}"),
                finish: change.promotion.finish().into(),
            },
        }
    } else {
        let why = decision.err().unwrap_or_default();
        say("keeping the engine the units run today");
        match systemctl("start", &["llama-local.service"]) {
            Ok(()) => match router_back(base, Some(&model), old_engine).await {
                Ok(()) => Outcome::Kept { why },
                Err(e) => Outcome::Partial {
                    step: "the router on the old engine".into(),
                    error: format!("{e:#}"),
                    finish: "systemctl --user restart llama-local.service".into(),
                },
            },
            Err(e) => Outcome::Partial {
                step: "restarting the router".into(),
                error: format!("{e:#}"),
                finish: "systemctl --user start llama-local.service".into(),
            },
        }
    };
    // Two builds on disk, not every one ever tried: a kept candidate goes,
    // a promoted one's predecessor stays as `previous`. A partial promotion
    // keeps everything until it is finished.
    if !matches!(outcome, Outcome::Partial { .. }) {
        if let Err(e) = crate::engine::prune_builds(&m.mecha_home) {
            say(&format!("old builds could not be removed: {e:#}"));
        }
    }
    let row = LedgerRow {
        at: Utc::now(),
        action: change.action.into(),
        model: Some(model),
        old,
        new,
        outcome,
    };
    // The units have already moved (or not) by now: a ledger that cannot be
    // written is said, never returned as a failed change.
    if let Err(e) = append_ledger(&m.mecha_home, &row) {
        say(&format!(
            "the measurement could not be written to {}: {e:#}",
            ledger_path(&m.mecha_home).display()
        ));
    }
    // Held to here: no run started on either engine between the first load
    // and the router answering on the winner.
    drop(switching);
    Ok(row)
}

/// The adopt promotion, each step named on failure: the drop-ins recorded,
/// then written; the units reloaded; the on-demand backends stopped (their
/// sockets stay, so the next request starts them on the new engine); the
/// router restarted with the model it had.
async fn promote_adopt(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    base: &str,
    model: &str,
) -> std::result::Result<(), (String, anyhow::Error)> {
    let step = |s: &str| {
        let s = s.to_string();
        move |e: anyhow::Error| (s, e)
    };
    // `current` to the pin — the build the gate measured — before any unit
    // names it.
    crate::engine::point_current(&m.mecha_home, crate::engine::PIN.tag)
        .map_err(step("pointing `current` at the pin"))?;
    // An adopt starts the engine's history: a `previous` left from before a
    // de-adopt would make this adopt's rollback step onto a stale build
    // instead of removing its drop-ins.
    match std::fs::remove_file(crate::engine::previous(&m.mecha_home)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(("clearing a stale `previous`".into(), e.into()))
        }
        _ => {}
    }
    let managed = managed_binary(&m.mecha_home);
    let dir = unit_dir(m).map_err(step("finding the user unit directory"))?;
    for (s, _) in &servers.present {
        let path = drop_in_path(dir, s.unit);
        crate::sidecar::Manifest::record(&m.mecha_home, "llama", &path)
            .map_err(step("recording the drop-ins"))?;
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))
            .and_then(|()| std::fs::write(&path, drop_in_text(&managed)))
            .map_err(|e| (format!("writing {}", path.display()), e.into()))?;
    }
    systemctl("daemon-reload", &[]).map_err(step("systemctl daemon-reload"))?;
    restart_on(servers, base, model, &managed)
        .await
        .map_err(|(s, e)| (s.to_string(), e))
}

/// The upgrade promotion: `previous` keeps the old build, `current` moves to
/// the new one — each one rename — then the servers restart on it.
async fn promote_upgrade(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    base: &str,
    model: &str,
    from_tag: &str,
    to_tag: &str,
) -> std::result::Result<(), (String, anyhow::Error)> {
    crate::engine::point_previous(&m.mecha_home, from_tag)
        .map_err(|e| ("keeping the old build as `previous`".to_string(), e))?;
    crate::engine::point_current(&m.mecha_home, to_tag)
        .map_err(|e| ("pointing `current` at the new build".to_string(), e))?;
    restart_on(servers, base, model, &managed_binary(&m.mecha_home))
        .await
        .map_err(|(s, e)| (s.to_string(), e))
}

/// Stop the embeddings and OCR services, leaving their sockets: the next
/// request starts each on whatever engine the units name by then.
pub fn stop_backends(servers: &Servers) -> Result<()> {
    let backends: Vec<&str> = servers
        .present
        .iter()
        .filter(|(s, _)| s.role != Role::Router)
        .map(|(s, _)| s.unit)
        .collect();
    if backends.is_empty() {
        return Ok(());
    }
    systemctl("stop", &backends)
}

/// The servers onto the engine the units now name: the on-demand backends
/// stopped (their sockets stay, so the next request starts them on it), the
/// router restarted, and the model the owner had loaded on it again.
pub async fn restart_on(
    servers: &Servers,
    base: &str,
    model: &str,
    engine: &Path,
) -> std::result::Result<(), (&'static str, anyhow::Error)> {
    stop_backends(servers).map_err(|e| ("stopping the on-demand backends", e))?;
    systemctl("restart", &["llama-local.service"]).map_err(|e| ("restarting the router", e))?;
    router_back(base, Some(model), engine)
        .await
        .map_err(|e| ("the router on the new engine", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemctl_show_reads_a_unit_s_launcher() {
        let text = "LoadState=loaded\n\
            ExecStart={ path=/home/u/Github/mecha/scripts/start-router.sh ; argv[]=/home/u/Github/mecha/scripts/start-router.sh ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }\n\
            Environment=PATH=/home/u/.local/bin:/usr/bin \"NOTE=two words\" MECHA_EMBED_PORT=18081\n";
        let l = parse_show(text).unwrap();
        assert_eq!(l.argv, vec!["/home/u/Github/mecha/scripts/start-router.sh"]);
        assert_eq!(
            l.env,
            vec![
                ("PATH".into(), "/home/u/.local/bin:/usr/bin".into()),
                ("NOTE".into(), "two words".into()),
                ("MECHA_EMBED_PORT".into(), "18081".into()),
            ]
        );
        assert!(parse_show("LoadState=not-found\nExecStart=\n").is_none());
    }

    #[test]
    fn exec_start_keeps_arguments() {
        assert_eq!(
            parse_exec_start("{ path=/x ; argv[]=/x --a \"b c\" ; ignore_errors=no }").unwrap(),
            vec!["/x", "--a", "b c"]
        );
        assert!(parse_exec_start("").is_none());
    }

    /// The unit's own `LLAMA_SERVER` wins over its `PATH` — which is how an
    /// adopted unit reads, and how the old leg finds the hand install.
    #[test]
    fn a_launcher_s_engine_is_its_llama_server_then_its_path() {
        let dir = std::env::temp_dir().join(format!("mecha-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/llama-server"), "").unwrap();
        let on_path = Launcher {
            argv: vec!["/x".into()],
            env: vec![(
                "PATH".into(),
                format!("/nowhere:{}", dir.join("bin").display()),
            )],
        };
        assert_eq!(on_path.engine(&[]), Some(dir.join("bin/llama-server")));
        // A unit naming no PATH runs on the manager's, and reads that way.
        let no_path = Launcher {
            argv: vec!["/x".into()],
            env: vec![("MECHA_EMBED_PORT".into(), "18081".into())],
        };
        assert_eq!(no_path.engine(&[]), None);
        let manager = [("PATH".to_string(), dir.join("bin").display().to_string())];
        assert_eq!(no_path.engine(&manager), Some(dir.join("bin/llama-server")));
        let adopted = Launcher {
            argv: vec!["/x".into()],
            env: vec![
                ("PATH".into(), dir.join("bin").display().to_string()),
                ("LLAMA_SERVER".into(), "/m/current/llama-server".into()),
            ],
        };
        assert_eq!(
            adopted.engine(&[]),
            Some(PathBuf::from("/m/current/llama-server"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn leg(gen: [f64; 3], pre: [f64; 3], smoke: &[(&str, Smoke)]) -> Leg {
        Leg {
            engine: PathBuf::from("/e"),
            version: String::new(),
            bench: Some(Bench {
                generation_tps: Rate { runs: gen.to_vec() },
                prefill_tps: Rate { runs: pre.to_vec() },
            }),
            smoke: smoke
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn a_new_engine_no_slower_beyond_the_noise_passes() {
        let ok = &[("chat", Smoke::Passed)];
        let old = leg([60.0, 61.0, 62.0], [2000.0, 2010.0, 2020.0], ok);
        // 2 % slower is inside the 3 % floor.
        assert!(verdict(&old, &leg([59.0, 59.8, 60.0], [1980.0, 1990.0, 2000.0], ok)).is_ok());
        assert!(verdict(&old, &leg([70.0, 71.0, 72.0], [2500.0, 2500.0, 2500.0], ok)).is_ok());
        // 10 % slower generation is not.
        let err = verdict(&old, &leg([54.0, 55.0, 55.5], [2010.0; 3], ok)).unwrap_err();
        assert!(err.contains("generation is slower"), "{err}");
        // Noisy runs widen the band: a 6 % drop inside 8 % spread passes.
        let noisy = leg([58.0, 61.0, 63.0], [2000.0; 3], ok);
        assert!(verdict(&noisy, &leg([57.5, 57.5, 57.5], [2000.0; 3], ok)).is_ok());
    }

    /// A smoke the old engine passed and the new one does not — failed, or
    /// not run — keeps the old engine, however fast the new one is.
    #[test]
    fn a_smoke_the_old_engine_passed_must_pass_on_the_new() {
        let old = leg(
            [60.0; 3],
            [2000.0; 3],
            &[("chat", Smoke::Passed), ("ocr", Smoke::Passed)],
        );
        let new = leg(
            [80.0; 3],
            [3000.0; 3],
            &[
                ("chat", Smoke::Passed),
                (
                    "ocr",
                    Smoke::Failed {
                        error: "mtmd: unsupported".into(),
                    },
                ),
            ],
        );
        let err = verdict(&old, &new).unwrap_err();
        assert!(err.contains("ocr") && err.contains("mtmd"), "{err}");
        // Not run on both is no evidence either way, and does not block.
        let both = &[
            ("chat", Smoke::Passed),
            (
                "ocr",
                Smoke::NotRun {
                    why: "no unit".into(),
                },
            ),
        ];
        assert!(verdict(
            &leg([60.0; 3], [2000.0; 3], both),
            &leg([60.0; 3], [2000.0; 3], both)
        )
        .is_ok());
    }

    #[test]
    fn an_embedding_that_moved_fails_the_new_leg() {
        let mut new = leg([60.0; 3], [2000.0; 3], &[("embeddings", Smoke::Passed)]);
        compare_embeddings(&mut new, Some(&[1.0, 0.0]), Some(&[1.0, 0.0]));
        assert_eq!(new.smoke["embeddings"], Smoke::Passed);
        compare_embeddings(&mut new, Some(&[1.0, 0.0]), Some(&[0.9, 0.4]));
        assert!(matches!(new.smoke["embeddings"], Smoke::Failed { .. }));
    }

    /// One outlier run cannot open the band: runs that disagree past
    /// `MAX_SPREAD` are not a measurement, and the band never exceeds 10 %
    /// nor takes the candidate's own noise.
    #[test]
    fn the_band_has_a_ceiling_and_is_the_old_engine_s() {
        let ok = &[("chat", Smoke::Passed)];
        // Old prefill [5, 55, 60]: spread 1.0 — not a measurement.
        let wild = leg([60.0; 3], [5.0, 55.0, 60.0], ok);
        let err = verdict(&wild, &leg([60.0; 3], [5.0; 3], ok)).unwrap_err();
        assert!(err.contains("not a measurement"), "{err}");
        // A noisy candidate does not widen its own band: old is steady, new
        // varies 20 % around a median 15 % slower.
        let steady = leg([60.0; 3], [2000.0; 3], ok);
        let err = verdict(&steady, &leg([46.0, 51.0, 57.0], [2000.0; 3], ok)).unwrap_err();
        assert!(err.contains("generation is slower"), "{err}");
        // An old engine at 20 % spread still allows only 10 %.
        let noisy = leg([54.0, 60.0, 66.0], [2000.0; 3], ok);
        assert!(verdict(&noisy, &leg([53.0; 3], [2000.0; 3], ok)).is_err());
        assert!(verdict(&noisy, &leg([55.0; 3], [2000.0; 3], ok)).is_ok());
    }

    fn holds() -> (crate::hold::Holds, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mecha-gate-holds-{}", uuid::Uuid::new_v4()));
        (crate::hold::Holds::new(&dir), dir)
    }

    const ROUTER: &str = "http://127.0.0.1:8080";

    /// The refusal a call must have made, as text.
    fn refused<T>(r: Result<T>) -> String {
        match r {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e.to_string(),
        }
    }

    /// The safety the gate rests on: a run holding the router, or a switch
    /// already waiting, declines — before the download, and again under the
    /// switch — and leaves no switch behind.
    #[test]
    fn a_busy_router_declines_and_leaves_no_switch() {
        let (h, dir) = holds();
        preflight(&h, ROUTER).unwrap();
        let held = h.try_hold(ROUTER, "web chat").unwrap().unwrap();
        let err = preflight(&h, ROUTER).unwrap_err().to_string();
        assert!(err.contains("web chat"), "{err}");
        let err = refused(take_switch(
            &h,
            ROUTER,
            "llama.cpp (provided)",
            "llama.cpp b1",
        ));
        assert!(err.contains("hold the router"), "{err}");
        assert!(
            h.pending(ROUTER).is_none(),
            "the declined switch is withdrawn"
        );
        drop(held);
        // A switch already waiting declines both too, and is left alone.
        let other = h.begin_switch(ROUTER, Some("a"), "qwen").unwrap().unwrap();
        assert!(preflight(&h, ROUTER)
            .unwrap_err()
            .to_string()
            .contains("qwen"));
        assert!(take_switch(&h, ROUTER, "llama.cpp (provided)", "llama.cpp b1").is_err());
        assert_eq!(h.pending(ROUTER).map(|s| s.to), Some("qwen".to_string()));
        drop(other);
        // Quiet: the switch is taken and held.
        let s = take_switch(&h, ROUTER, "llama.cpp (provided)", "llama.cpp b1").unwrap();
        assert_eq!(
            h.pending(ROUTER).map(|s| s.to),
            Some("llama.cpp b1".to_string())
        );
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A machine where some units already run mecha's engine is refused
    /// before anything is fetched or stopped: the install would repoint
    /// `current` under them, and their "old" leg would run the candidate.
    #[tokio::test]
    async fn a_part_adopted_machine_is_refused_before_anything_moves() {
        let root = std::env::temp_dir().join(format!("mecha-part-{}", uuid::Uuid::new_v4()));
        let m = crate::sidecar::Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![root.join("units")],
            path: vec![],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        let unit = |role, env: Vec<(String, String)>| {
            let s = SERVERS.iter().find(|s| s.role == role).copied().unwrap();
            (
                s,
                Launcher {
                    argv: vec!["/x".into()],
                    env,
                },
            )
        };
        let path = ("PATH".to_string(), "/usr/bin".to_string());
        let servers = Servers {
            present: vec![
                unit(Role::Router, vec![path.clone()]),
                unit(
                    Role::Embeddings,
                    vec![
                        path.clone(),
                        (
                            "LLAMA_SERVER".into(),
                            managed_binary(&m.mecha_home).display().to_string(),
                        ),
                    ],
                ),
            ],
            base_env: vec![],
        };
        let (h, dir) = holds();
        let cancel = tokio_util::sync::CancellationToken::new();
        let err = refused(
            adopt(
                &m,
                &servers,
                &h,
                ROUTER,
                Some("q"),
                false,
                &cancel,
                &mut |_| {},
            )
            .await,
        );
        assert!(err.contains("--rollback"), "{err}");
        assert!(
            !crate::engine::engine_root(&m.mecha_home).exists(),
            "nothing fetched"
        );
        assert!(h.pending(ROUTER).is_none(), "no switch taken");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A router that answers `/health` but not `/models` is a finding — the
    /// gate declines rather than measure (and load) the configured model —
    /// while a router answering nothing reads as down.
    /// One canned answer, every request line kept: what `verify_build` asked
    /// is asserted literally, as `brief.rs` and `preflight.rs` do.
    fn mock_props(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                log.lock()
                    .unwrap()
                    .push(req.lines().next().unwrap_or("").to_string());
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (base, seen)
    }

    /// The promotion's last word: the router asked which build serves the
    /// model — with `autoload=false`, so the question loads nothing (a probe
    /// without it loads the model: LLAMA-SERVER.md) — and a non-success
    /// answer or a missing `build_info` refused.
    #[tokio::test]
    async fn verify_build_asks_without_loading_and_refuses_what_it_cannot_read() {
        let commit = "2bc563573479d53b30b8793039485887bc0fdda8";
        let (base, seen) = mock_props("200 OK", r#"{"build_info":"b11391-2bc563573"}"#);
        verify_build(&base, "qwen", 11391, commit).await.unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            "GET /props?model=qwen&autoload=false HTTP/1.1"
        );
        let (base, _) = mock_props("200 OK", r#"{"build_info":"b1193-95887577"}"#);
        let err = refused(verify_build(&base, "qwen", 11391, commit).await);
        assert!(err.contains("b1193-95887577"), "{err}");
        let (base, _) = mock_props("500 Internal Server Error", r#"{"error":"boom"}"#);
        let err = refused(verify_build(&base, "qwen", 11391, commit).await);
        assert!(err.contains("500"), "{err}");
        let (base, _) = mock_props("200 OK", r#"{"model_alias":"qwen"}"#);
        assert!(verify_build(&base, "qwen", 11391, commit).await.is_err());
    }

    #[tokio::test]
    async fn a_router_that_will_not_list_its_models_declines() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let (status, body) = if req.starts_with("GET /health") {
                    ("200 OK", "{\"status\":\"ok\"}")
                } else {
                    ("500 Internal Server Error", "{}")
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let err = listed_models(&format!("http://127.0.0.1:{port}"))
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("answers /health but not /models"), "{err}");
        // Nothing listening: down, and the configured model stands.
        let free = free_port().unwrap();
        assert!(listed_models(&format!("http://127.0.0.1:{free}"))
            .await
            .unwrap()
            .is_none());
    }

    fn machinery(root: &Path) -> crate::sidecar::Machinery {
        crate::sidecar::Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![root.join("units")],
            path: vec![],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        }
    }

    fn servers_naming(engine: Option<&Path>) -> Servers {
        let mut env = vec![("PATH".to_string(), "/usr/bin".to_string())];
        if let Some(e) = engine {
            env.push(("LLAMA_SERVER".into(), e.display().to_string()));
        }
        Servers {
            present: vec![(
                SERVERS[0],
                Launcher {
                    argv: vec!["/x".into()],
                    env,
                },
            )],
            base_env: vec![],
        }
    }

    /// An upgrade moves mecha's engine, so a machine still on a hand-built
    /// one is sent to `--adopt` — and one already on the tag asked for is
    /// told so — before anything is fetched or stopped.
    #[tokio::test]
    async fn an_upgrade_needs_an_adopted_machine_and_a_new_tag() {
        let root = std::env::temp_dir().join(format!("mecha-upg-{}", uuid::Uuid::new_v4()));
        let m = machinery(&root);
        let (h, dir) = holds();
        let provided = servers_naming(None);
        let err = refused(check_upgradable(&m, &provided, &h, ROUTER));
        assert!(err.contains("--adopt"), "{err}");

        let adopted = servers_naming(Some(&managed_binary(&m.mecha_home)));
        std::fs::create_dir_all(crate::engine::engine_root(&m.mecha_home)).unwrap();
        crate::engine::point_current(&m.mecha_home, "b11391").unwrap();
        let (_, tag) = check_upgradable(&m, &adopted, &h, ROUTER).unwrap();
        assert_eq!(tag, "b11391");
        let spec = crate::engine::Spec::pin(crate::engine::Target::LinuxArm64Cuda13).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let err = refused(
            upgrade(
                &m,
                &adopted,
                &h,
                ROUTER,
                &spec,
                crate::engine::Target::LinuxArm64Cuda13,
                Some("q"),
                false,
                &cancel,
                &mut |_| {},
            )
            .await,
        );
        assert!(err.contains("already run"), "{err}");
        assert!(h.pending(ROUTER).is_none(), "no switch taken");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rollback_row_names_where_it_went_and_measures_nothing() {
        let row = rollback_row(
            Path::new("/m/b20000/llama-server"),
            Path::new("/m/b11391/llama-server"),
            None,
        );
        assert_eq!(row.action, "rollback");
        assert!(matches!(&row.outcome, Outcome::RolledBack { to } if to.contains("b11391")));
        assert!(row.old.bench.is_none() && row.new.smoke.is_empty());
    }

    fn model(id: &str, status: &str) -> crate::provider::router::RouterModel {
        serde_json::from_value(serde_json::json!({"id": id, "status": {"value": status}})).unwrap()
    }

    #[test]
    fn the_measured_model_is_the_resident_one_and_unknown_declines() {
        let list = vec![model("a", "unloaded"), model("b", "loaded")];
        assert_eq!(measured_model(Some(&list), Some("cfg")).unwrap(), "b");
        let idle = vec![model("a", "unloaded")];
        assert_eq!(measured_model(Some(&idle), Some("cfg")).unwrap(), "cfg");
        assert_eq!(measured_model(None, Some("cfg")).unwrap(), "cfg");
        // A status this build does not know: unknown, not "none loaded".
        let newer = vec![model("a", "unloaded"), model("b", "warming")];
        let err = measured_model(Some(&newer), Some("cfg"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not know"), "{err}");
        // Nothing to measure: declined before the router stops.
        assert!(measured_model(Some(&idle), None).is_err());
        // An empty list is nothing loaded, not an unknown status.
        assert_eq!(measured_model(Some(&[]), Some("cfg")).unwrap(), "cfg");
    }

    #[test]
    fn show_environment_values_lose_their_quotes() {
        assert_eq!(unquote("\"/a b/bin:/usr/bin\""), "/a b/bin:/usr/bin");
        assert_eq!(unquote("'x y'"), "x y");
        assert_eq!(unquote("/usr/bin"), "/usr/bin");
        assert_eq!(unquote("\""), "\"");
    }

    /// An attempt whose old engine could not be measured is a row of its
    /// own — kept, or partial when the router did not come back — never
    /// "never measured".
    #[test]
    fn an_old_engine_that_could_not_be_measured_is_recorded() {
        let e = anyhow::anyhow!("the router did not come up");
        let kept = not_measured(
            "adopt",
            &e,
            &Ok(()),
            Some("q".into()),
            Path::new("/old"),
            Path::new("/new"),
        );
        assert!(matches!(&kept.outcome, Outcome::Kept { why } if why.contains("did not come up")));
        assert!(matches!(kept.old.smoke["chat"], Smoke::Failed { .. }));
        assert!(matches!(kept.new.smoke["chat"], Smoke::NotRun { .. }));
        let stuck = not_measured(
            "adopt",
            &e,
            &Err(anyhow::anyhow!("unit failed")),
            None,
            Path::new("/old"),
            Path::new("/new"),
        );
        assert!(
            matches!(&stuck.outcome, Outcome::Partial { finish, .. } if finish.contains("systemctl"))
        );
        let home = std::env::temp_dir().join(format!("mecha-nm-{}", uuid::Uuid::new_v4()));
        append_ledger(&home, &kept).unwrap();
        assert_eq!(read_ledger(&home).unwrap().rows.len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rates_have_a_median_and_a_spread() {
        let r = Rate {
            runs: vec![62.0, 60.0, 61.0],
        };
        assert_eq!(r.median(), 61.0);
        assert!((r.spread() - 2.0 / 61.0).abs() < 1e-9);
        assert_eq!(Rate { runs: vec![] }.spread(), 0.0);
    }

    #[test]
    fn the_ledger_appends_and_reads_back() {
        let home = std::env::temp_dir().join(format!("mecha-ledger-{}", uuid::Uuid::new_v4()));
        let row = LedgerRow {
            at: Utc::now(),
            action: "adopt".into(),
            model: Some("qwen".into()),
            old: leg([60.0; 3], [2000.0; 3], &[("chat", Smoke::Passed)]),
            new: leg([61.0; 3], [2100.0; 3], &[("chat", Smoke::Passed)]),
            outcome: Outcome::Kept {
                why: "slower".into(),
            },
        };
        append_ledger(&home, &row).unwrap();
        append_ledger(&home, &row).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(ledger_path(&home))
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"{\"from a newer mecha\": 1}\n"))
            .unwrap();
        let ledger = read_ledger(&home).unwrap();
        assert_eq!(ledger.rows.len(), 2);
        assert_eq!(ledger.rows[0], row);
        assert_eq!(
            ledger.skipped, 1,
            "the unparseable line is counted, not hidden"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The drop-in names the engine `install_engine` put under *this* mecha
    /// home — a non-default one included — so the units and the install can
    /// never point at two trees.
    #[test]
    fn the_drop_in_names_this_home_s_current_link() {
        let t = drop_in_text(&managed_binary(Path::new("/srv/mecha")));
        assert!(t.contains("[Service]"));
        assert!(
            t.contains(
                "Environment=\"LLAMA_SERVER=/srv/mecha/sidecars/llama/current/llama-server\""
            ),
            "{t}"
        );
        assert!(!t.contains("%h"), "{t}");
        assert_eq!(
            drop_in_path(Path::new("/cfg/systemd/user"), "llama-local.service"),
            PathBuf::from("/cfg/systemd/user/llama-local.service.d/mecha-engine.conf")
        );
    }

    /// The backends' legs on this machine, for real and on private ports:
    /// each unit's own launcher, once on the engine it runs and once on the
    /// candidate `MECHA_TEST_ENGINE` names. The router is left alone — no
    /// router entry, so the chat leg reads not run. Needs the units.
    #[tokio::test]
    #[ignore]
    async fn the_backends_measure_on_both_engines() {
        let Some(candidate) = std::env::var_os("MECHA_TEST_ENGINE").map(PathBuf::from) else {
            eprintln!("MECHA_TEST_ENGINE unset; skipping");
            return;
        };
        let mut servers = Servers::read().unwrap();
        servers.present.retain(|(s, _)| s.role != Role::Router);
        assert!(
            !servers.present.is_empty(),
            "no backend units on this machine"
        );
        let logs = std::env::temp_dir().join(format!("mecha-gate-logs-{}", uuid::Uuid::new_v4()));
        let mut said = Vec::new();
        // The old leg needs an engine name for its record; the backends' own.
        let old_engine = servers.present[0].1.engine(&servers.base_env).unwrap();
        let (mut old, old_v) = measure(&servers, Some(&old_engine), None, &logs, "old", &mut |s| {
            said.push(s.to_string())
        })
        .await
        .unwrap();
        let (mut new, new_v) = measure(&servers, Some(&candidate), None, &logs, "new", &mut |s| {
            said.push(s.to_string())
        })
        .await
        .unwrap();
        compare_embeddings(&mut new, old_v.as_deref(), new_v.as_deref());
        eprintln!("old {}: {:?}", old.version, old.smoke);
        eprintln!("new {}: {:?}", new.version, new.smoke);
        eprintln!(
            "cosine {:.6}",
            cosine(
                old_v.as_deref().unwrap_or(&[]),
                new_v.as_deref().unwrap_or(&[])
            )
        );
        for leg in [&mut old, &mut new] {
            assert!(matches!(leg.smoke["chat"], Smoke::NotRun { .. }));
            for name in ["embeddings", "ocr"] {
                assert_eq!(
                    leg.smoke.get(name),
                    Some(&Smoke::Passed),
                    "{name} on {}",
                    leg.version
                );
            }
        }
        let _ = std::fs::remove_dir_all(&logs);
    }

    /// A row from a newer mecha — an outcome or smoke this build does not
    /// know — still reads, its unknown parts as `Unknown`: the measurement
    /// history is the record §10.3 accumulates.
    #[test]
    fn a_row_with_unknown_variants_still_reads() {
        let mut v = serde_json::to_value(LedgerRow {
            at: Utc::now(),
            action: "upgrade".into(),
            model: None,
            old: leg([60.0; 3], [2000.0; 3], &[("chat", Smoke::Passed)]),
            new: leg([60.0; 3], [2000.0; 3], &[("chat", Smoke::Passed)]),
            outcome: Outcome::Promoted,
        })
        .unwrap();
        v["outcome"] = serde_json::json!({"outcome": "measured_twice", "n": 2});
        v["new"]["smoke"]["chat"] = serde_json::json!({"outcome": "flaky"});
        let row: LedgerRow = serde_json::from_value(v).unwrap();
        assert_eq!(row.outcome, Outcome::Unknown);
        assert_eq!(row.new.smoke["chat"], Smoke::Unknown);
        // An unknown smoke is never a pass.
        assert!(verdict(&row.old, &row.new).is_err());
    }

    #[test]
    fn an_unreadable_ledger_is_an_error_not_never() {
        let home = std::env::temp_dir().join(format!("mecha-ledger-{}", uuid::Uuid::new_v4()));
        assert_eq!(read_ledger(&home).unwrap().rows.len(), 0);
        std::fs::create_dir_all(ledger_path(&home)).unwrap();
        assert!(
            read_ledger(&home).is_err(),
            "a directory where the file should be"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_ocr_image_is_a_png() {
        use base64::Engine;
        let b = base64::engine::general_purpose::STANDARD
            .decode(OCR_PNG)
            .unwrap();
        assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn the_long_prompt_is_long() {
        assert!(long_prompt(3000).split_whitespace().count() >= 3000);
    }
}
