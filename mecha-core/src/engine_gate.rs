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
    /// `LLAMA_SERVER`, else `llama-server` on the unit's `PATH`.
    pub fn engine(&self) -> Option<PathBuf> {
        if let Some((_, v)) = self.env.iter().find(|(k, _)| k == "LLAMA_SERVER") {
            return Some(PathBuf::from(expand_home(v)));
        }
        let path = self.env.iter().find(|(k, _)| k == "PATH")?.1.clone();
        std::env::split_paths(&path)
            .map(|d| d.join("llama-server"))
            .find(|p| p.is_file())
    }
}

/// `%h` in a unit, and `$HOME` in what a unit's environment hands a shell.
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
/// A drop-in that resets `ExecStart=` leaves only the effective one; with
/// several, the first is the one this reads (the router's unit has one).
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
        .filter_map(|l| {
            l.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect())
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

    /// TERM to the group, then KILL after a grace — and wait, so the memory
    /// is free before the next engine loads.
    fn stop(&mut self) {
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
        // A child of the group can outlive its leader by a moment.
        std::thread::sleep(Duration::from_secs(2));
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.exited().is_none() {
            self.stop();
        }
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
    Ok(Running { child, port, log })
}

fn http(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(timeout).build()?)
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
    let v: serde_json::Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with the one word: ready"}],
            "max_tokens": 256,
            "chat_template_kwargs": {"enable_thinking": false},
        }))
        .send()
        .await?
        .json()
        .await?;
    // Words in either channel are the engine answering: a template that
    // thinks despite the switch puts them in `reasoning_content` first.
    let said = |k: &str| {
        v["choices"][0]["message"][k]
            .as_str()
            .is_some_and(|t| !t.trim().is_empty())
    };
    if !said("content") && !said("reasoning_content") {
        bail!("the chat turn came back empty: {v}");
    }
    Ok(())
}

async fn embedding(base: &str) -> Result<Vec<f64>> {
    let client = http(Duration::from_secs(120))?;
    let v: serde_json::Value = client
        .post(format!("{base}/v1/embeddings"))
        .json(&serde_json::json!({"input": "the layout model reads a scanned page"}))
        .send()
        .await?
        .json()
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
    let v: serde_json::Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{OCR_PNG}")}},
                {"type": "text", "text": "OCR:"},
            ]}],
            "max_tokens": 64,
        }))
        .send()
        .await?
        .json()
        .await?;
    if v["choices"][0]["message"]["content"].as_str().is_none() {
        bail!("the OCR server answered without a message: {v}");
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

/// The rule a promotion passes: every smoke the old engine passed, the new
/// one passes too; and the new one is no slower, on generation or prefill,
/// beyond the noise either engine's own runs show (at least 3 %).
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
        let noise = x.spread().max(y.spread()).max(0.03);
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

/// The ledger's rows, oldest first; a line this build cannot read is skipped,
/// as an append-only store's unknown rows are.
pub fn read_ledger(mecha_home: &Path) -> Vec<LedgerRow> {
    std::fs::read_to_string(ledger_path(mecha_home))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
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

    pub fn get(&self, role: Role) -> Option<&(Server, Launcher)> {
        self.present.iter().find(|(s, _)| s.role == role)
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
            .get(Role::Router)
            .and_then(|(_, l)| l.engine())
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
                why: "no router unit, or no model resident to measure".into(),
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
pub fn drop_in_text() -> String {
    "# Written by `mecha setup engine --adopt` (FEATURES-DESIGN.md §10.3): this\n\
     # unit's launcher runs mecha's managed llama.cpp engine through the\n\
     # `current` link. Removing this file (`mecha setup engine --rollback`)\n\
     # returns it to the llama-server on its PATH, which is left in place.\n\
     [Service]\n\
     Environment=LLAMA_SERVER=%h/.mecha/sidecars/llama/current/llama-server\n"
        .to_string()
}

/// Where a unit's adopt drop-in lives.
pub fn drop_in_path(home: &Path, unit: &str) -> PathBuf {
    home.join(".config/systemd/user")
        .join(format!("{unit}.d"))
        .join("mecha-engine.conf")
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
    Some(
        Path::new(arg0) == engine
            || std::fs::canonicalize(arg0).ok()? == std::fs::canonicalize(engine).ok()?,
    )
}

/// The managed engine's binary, as an adopted unit names it.
pub fn managed_binary(mecha_home: &Path) -> PathBuf {
    crate::engine::current(mecha_home).join("llama-server")
}

/// Whether every present unit already runs the managed engine.
pub fn adopted(servers: &Servers, mecha_home: &Path) -> bool {
    let managed = managed_binary(mecha_home);
    !servers.present.is_empty()
        && servers
            .present
            .iter()
            .all(|(_, l)| l.engine().as_deref() == Some(managed.as_path()))
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

/// The command that finishes a promotion a step of which failed.
const FINISH_ADOPT: &str = "systemctl --user daemon-reload && systemctl --user restart \
     llama-local.service (and `mecha setup engine --rollback` to undo)";

/// `mecha setup engine --adopt` (§10.3): measure the engine the units run
/// today against the shipped pin, and — when the pin is no slower and passes
/// every smoke the old one passed, or with `force` — point every present unit
/// at the managed engine through a drop-in, leaving the hand install in place
/// as the first rollback. Runs under the router's switch from before the
/// first measurement to after the router answers on the winner; declines,
/// moving nothing, when a switch is pending or a run holds the router.
pub async fn adopt(
    m: &crate::sidecar::Machinery,
    base_url: &str,
    fallback_model: Option<&str>,
    force: bool,
    say: &mut dyn FnMut(&str),
) -> Result<LedgerRow> {
    let base = crate::provider::router::base(base_url);
    let servers = Servers::read()?;
    if servers.get(Role::Router).is_none() {
        bail!(
            "there is no llama-local.service to adopt — on a clean machine the units arrive \
             with step 7c, and run the managed engine from the start"
        );
    }
    if adopted(&servers, &m.mecha_home) {
        bail!("every llama.cpp unit here already runs mecha's engine — nothing to adopt");
    }
    // The pin, side by side: nothing reads `current` until a drop-in names it.
    let managed = managed_binary(&m.mecha_home);
    crate::engine::install_engine(m, &mut *say).await?;

    let holds = crate::hold::Holds::open_default()?;
    let to = format!("llama.cpp {}", crate::engine::PIN.tag);
    let switching = match holds.begin_switch(&base, Some("llama.cpp (provided)"), &to)? {
        Ok(s) => s,
        Err(other) => bail!(
            "a switch to {} is already waiting on {base} (pid {}) — nothing was measured; run \
             this again once it is done",
            other.to,
            other.pid
        ),
    };
    let live = holds.live(&base);
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

    // What the owner is using is what gets measured: the resident model,
    // else the provider's.
    let model = crate::provider::router::models(&base)
        .await
        .and_then(|ms| crate::provider::router::resident(&ms).map(str::to_owned))
        .or_else(|| fallback_model.map(str::to_owned));
    let old_engine = servers
        .get(Role::Router)
        .and_then(|(_, l)| l.engine())
        .context("the router's unit names no llama-server it can find")?;

    say("stopping the router for the measurement");
    systemctl("stop", &["llama-local.service"])?;
    let logs = crate::engine::engine_root(&m.mecha_home).join("gate");
    let measured = async {
        let (old, old_v) = measure(&servers, None, model.as_deref(), &logs, "old", say).await?;
        let new = measure(
            &servers,
            Some(&managed),
            model.as_deref(),
            &logs,
            "new",
            say,
        )
        .await;
        Ok::<_, anyhow::Error>((old, old_v, new))
    }
    .await;
    let (old, old_v, new) = match measured {
        Ok(x) => x,
        Err(e) => {
            // The old engine could not be measured: nothing moved, so the
            // router goes back as it was.
            let back = systemctl("start", &["llama-local.service"]);
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
    let new = match new {
        Ok((mut leg, new_v)) => {
            compare_embeddings(&mut leg, old_v.as_deref(), new_v.as_deref());
            leg
        }
        Err(e) => Leg {
            version: version_of(&managed),
            engine: managed.clone(),
            bench: None,
            smoke: [(
                "chat".to_string(),
                Smoke::Failed {
                    error: format!("{e:#}"),
                },
            )]
            .into(),
        },
    };

    let decision = verdict(&old, &new);
    let promote = decision.is_ok() || force;
    let outcome = if promote {
        say("promoting: pointing the units at mecha's engine");
        match promote_adopt(m, &servers, &base, model.as_deref(), &managed).await {
            Ok(()) => match decision {
                Ok(()) => Outcome::Promoted,
                Err(why) => Outcome::PromotedByForce { why },
            },
            Err((step, e)) => Outcome::Partial {
                step,
                error: format!("{e:#}"),
                finish: FINISH_ADOPT.into(),
            },
        }
    } else {
        let why = decision.err().unwrap_or_default();
        say("keeping the engine the units run today");
        match systemctl("start", &["llama-local.service"]) {
            Ok(()) => match router_back(&base, model.as_deref(), &old_engine).await {
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
    let row = LedgerRow {
        at: Utc::now(),
        action: "adopt".into(),
        model,
        old,
        new,
        outcome,
    };
    append_ledger(&m.mecha_home, &row)?;
    // Held to here: no run started on either engine between the first load
    // and the router answering on the winner.
    drop(switching);
    Ok(row)
}

/// The promotion itself, each step named on failure: the drop-ins recorded,
/// then written; the units reloaded; the on-demand backends stopped (their
/// sockets stay, so the next request starts them on the new engine); the
/// router restarted, and asked which engine it runs.
async fn promote_adopt(
    m: &crate::sidecar::Machinery,
    servers: &Servers,
    base: &str,
    model: Option<&str>,
    managed: &Path,
) -> std::result::Result<(), (String, anyhow::Error)> {
    let step = |s: &str| {
        let s = s.to_string();
        move |e: anyhow::Error| (s, e)
    };
    for (s, _) in &servers.present {
        let path = drop_in_path(&m.home, s.unit);
        crate::sidecar::Manifest::record(&m.mecha_home, "llama", &path)
            .map_err(step("recording the drop-ins"))?;
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))
            .and_then(|()| std::fs::write(&path, drop_in_text()))
            .map_err(|e| (format!("writing {}", path.display()), e.into()))?;
    }
    systemctl("daemon-reload", &[]).map_err(step("systemctl daemon-reload"))?;
    let backends: Vec<&str> = servers
        .present
        .iter()
        .filter(|(s, _)| s.role != Role::Router)
        .map(|(s, _)| s.unit)
        .collect();
    if !backends.is_empty() {
        systemctl("stop", &backends).map_err(step("stopping the on-demand backends"))?;
    }
    systemctl("start", &["llama-local.service"]).map_err(step("starting the router"))?;
    router_back(base, model, managed)
        .await
        .map_err(step("the router on the new engine"))?;
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
        assert_eq!(on_path.engine(), Some(dir.join("bin/llama-server")));
        let adopted = Launcher {
            argv: vec!["/x".into()],
            env: vec![
                ("PATH".into(), dir.join("bin").display().to_string()),
                ("LLAMA_SERVER".into(), "/m/current/llama-server".into()),
            ],
        };
        assert_eq!(
            adopted.engine(),
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
        let rows = read_ledger(&home);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], row);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_drop_in_names_the_current_link() {
        let t = drop_in_text();
        assert!(t.contains("[Service]"));
        assert!(
            t.contains("Environment=LLAMA_SERVER=%h/.mecha/sidecars/llama/current/llama-server")
        );
        assert_eq!(
            drop_in_path(Path::new("/h"), "llama-local.service"),
            PathBuf::from("/h/.config/systemd/user/llama-local.service.d/mecha-engine.conf")
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
        let old_engine = servers.present[0].1.engine().unwrap();
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
