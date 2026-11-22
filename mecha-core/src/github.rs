//! The GitHub board's collector (`docs/GITHUB-DASHBOARD-DESIGN.md`, step 1):
//! `mecha github collect` reads the owner's selection file, asks `gh api`,
//! and writes `~/.mecha/github/github.db`, which the HUD reads through an
//! ordinary `sqlite` source. No agent loop: the fetch is fixed, and the one
//! model call is the quick summary ([`summary`], step 4) — a quarantined
//! one-shot per new or edited body, made after the collection is written, so
//! a pass that cannot run never costs the board its tables.
//!
//! **mecha never holds the token (G9).** Every call shells out to `gh`, whose
//! login stays in `gh`'s own store; nothing here reads, passes or prints it.
//! The binary is `gh` on `PATH`, or `$MECHA_GH_BIN` ([`Gh::from_env`]), which
//! is also how the tests put a fake in its place.
//!
//! **The selection is the owner's file, never layered config** (§2): a cloned
//! repository must not add itself to a watch list that spends the owner's
//! quota and puts its issue text in front of a model. A file that does not
//! parse selects nothing, and says so.
//!
//! **Refused and failed stay different findings** (§3). A rate limit, a login
//! that is absent or refused, a `gh` that is not there, a repository that
//! vanished, a run out of time and any other failure each write their own
//! `collect_log` kind, and `mecha doctor` reads them ([`read_log`]). A board
//! showing yesterday's green because the collector has failed since is the
//! silently-degrading shape, so every run writes a `finished` row, whatever
//! happened, and the tables keep what the last good answer said.
//!
//! **Unknown is never zero.** A count GitHub did not answer is `NULL`, and an
//! item listing that could not be read to its end marks nothing closed.
//!
//! **A body is never written here.** It is read with the listing, held in
//! memory for the summary pass, and stored only as its hash — the key that
//! says whether a summary is of the body GitHub holds now (§5).

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, Instant};

/// `<home>/github/`.
pub fn dir(home: &Path) -> PathBuf {
    home.join("github")
}

/// `<home>/github/github.db` — the collector's store, the HUD's source.
pub fn db_path(home: &Path) -> PathBuf {
    dir(home).join("github.db")
}

/// `<home>/github/repos.toml` — the owner's selection (§2).
pub fn selection_path(home: &Path) -> PathBuf {
    dir(home).join("repos.toml")
}

/// Default-branch workflow runs are kept this long (§3).
pub const CI_KEEP_DAYS: i64 = 90;
/// An item is kept while open, and this long after it was last seen open.
pub const ITEM_KEEP_DAYS: i64 = 30;
/// The collector's own log.
pub const LOG_KEEP_DAYS: i64 = 90;

/// One run's whole allowance for calls to `gh`. A bound per call is not a
/// bound per run (`ARCHITECTURE.md` §System state): four hundred selected
/// repositories at a slow minute each would outlast the unit's start timeout,
/// and systemd would kill the run before it wrote anything. Every call gets
/// what is left of this; a test keeps it under the unit's `TimeoutStartSec`
/// with room for the write.
pub const RUN_BUDGET: StdDuration = StdDuration::from_secs(8 * 60);
/// The most any one call may take.
pub const CALL_TIMEOUT: StdDuration = StdDuration::from_secs(90);
/// The most of one `gh` answer this process will hold, capped at the door:
/// read past it is discarded as it arrives, and the answer is refused
/// ([`GhError::TooLarge`]), never parsed cut short. Bodies are a stranger's
/// length — GitHub allows 65,536 characters each, so a batch of ten
/// repositories' two hundred items could reach ~130 MB on a machine holding
/// a model resident — and the items listing, refused at this size, is asked
/// again without them.
pub const ANSWER_BYTES_MAX: usize = 32 * 1024 * 1024;
/// What is kept of `gh`'s stderr: only its first line is ever read.
const STDERR_BYTES_MAX: usize = 64 * 1024;
/// Repositories per GraphQL query: ten repositories of a hundred issues and
/// a hundred pull requests with twenty labels each is ~42,000 nodes, well
/// under GitHub's 500,000.
const BATCH: usize = 10;
/// Follow-up pages per connection (100 each) before a listing is called
/// incomplete — and an incomplete listing marks nothing closed.
const MAX_PAGES: usize = 20;
/// Workflow runs asked for per repository per run.
const CI_PER_PAGE: &str = "100";
/// The associations whose pull requests are the owner's own side's (G5).
/// Anything else, including a value GitHub adds later, is outside.
const INSIDE: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];

/// Every time this store writes, to the millisecond: a run's time is its
/// identity in `collect_log`, and two runs in one second are two runs.
fn ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

// ---- names ----

/// GitHub's grammar for an owner login: letters, digits, `-` (and `_`, which
/// managed accounts use). Checked on everything, from the file or from
/// GitHub, before it reaches a path or a query — so no value can walk a path
/// or carry an escape.
pub fn valid_owner(s: &str) -> bool {
    (1..=39).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// GitHub's grammar for a repository name: letters, digits, `.`, `-`, `_`,
/// never `.` or `..` as the whole name, never `%` (§4).
pub fn valid_repo_name(s: &str) -> bool {
    (1..=100).contains(&s.len())
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// `owner/name`, both halves valid.
pub fn split_full_name(s: &str) -> Option<(&str, &str)> {
    let (owner, name) = s.split_once('/')?;
    (valid_owner(owner) && valid_repo_name(name)).then_some((owner, name))
}

// ---- the selection ----

/// `repos.toml` (§2): owners whose repositories are included unless excluded,
/// and per-repository exceptions either way.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    #[serde(default, rename = "owner")]
    pub owners: Vec<OwnerRule>,
    #[serde(default, rename = "repo")]
    pub repos: Vec<RepoRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerRule {
    pub login: String,
    #[serde(default)]
    pub include: Include,
    /// Forks are excluded unless this says otherwise (G4).
    #[serde(default)]
    pub forks: bool,
    /// Archived repositories are excluded unless this says otherwise.
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Include {
    /// Every repository of the owner, less forks and archived ones.
    #[default]
    All,
    /// Only those a `[[repo]]` line includes.
    Listed,
}

/// An exception: `include` is required, because a line that only names a
/// repository could mean either.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoRule {
    pub name: String,
    pub include: bool,
}

/// What reading the selection file found.
#[derive(Debug, Clone)]
pub enum SelectionRead {
    Absent,
    /// It is there and cannot be used; nothing is selected.
    Invalid(String),
    Read(Selection),
}

pub fn read_selection(path: &Path) -> SelectionRead {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SelectionRead::Absent,
        Err(e) => return SelectionRead::Invalid(format!("{}: {e}", path.display())),
    };
    match parse_selection(&text) {
        Ok(s) => SelectionRead::Read(s),
        Err(e) => SelectionRead::Invalid(format!("{}: {e}", path.display())),
    }
}

pub fn parse_selection(text: &str) -> Result<Selection, String> {
    let s: Selection = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut owners = BTreeSet::new();
    for o in &s.owners {
        if !valid_owner(&o.login) {
            return Err(format!(
                "[[owner]] login {:?} is not a GitHub login",
                o.login
            ));
        }
        if !owners.insert(o.login.to_ascii_lowercase()) {
            return Err(format!("[[owner]] {:?} is listed twice", o.login));
        }
    }
    let mut repos = BTreeSet::new();
    for r in &s.repos {
        if split_full_name(&r.name).is_none() {
            return Err(format!(
                "[[repo]] name {:?} is not owner/name in GitHub's grammar",
                r.name
            ));
        }
        if !repos.insert(r.name.to_ascii_lowercase()) {
            return Err(format!("[[repo]] {:?} is listed twice", r.name));
        }
    }
    Ok(s)
}

impl Selection {
    fn rule_for(&self, full_name: &str) -> Option<&RepoRule> {
        self.repos
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(full_name))
    }

    /// Whether `repo` is watched. A `[[repo]]` line decides first, either way
    /// and over the fork and archive filters; then its owner's line; a
    /// repository of an owner the file does not name is not watched.
    pub fn selects(&self, repo: &Repo) -> bool {
        if let Some(rule) = self.rule_for(&repo.full_name) {
            return rule.include;
        }
        let Some(owner) = self
            .owners
            .iter()
            .find(|o| o.login.eq_ignore_ascii_case(&repo.owner))
        else {
            return false;
        };
        owner.include == Include::All
            && (owner.forks || !repo.fork)
            && (owner.archived || !repo.archived)
    }

    /// The `[[repo]]` lines that include, by name.
    fn included_by_name(&self) -> impl Iterator<Item = &str> {
        self.repos
            .iter()
            .filter(|r| r.include)
            .map(|r| r.name.as_str())
    }
}

// ---- what is collected ----

/// One repository the login can see.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Repo {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub fork: bool,
    pub archived: bool,
    pub private: bool,
    pub default_branch: Option<String>,
    pub pushed_at: Option<String>,
    /// In the login's own listing, rather than fetched because a `[[repo]]`
    /// line named it.
    pub listed: bool,
}

fn parse_repo(v: &Value, listed: bool) -> Option<Repo> {
    let full_name = v.get("full_name")?.as_str()?;
    let (owner, name) = split_full_name(full_name)?;
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Repo {
        full_name: full_name.to_string(),
        owner: owner.to_string(),
        name: name.to_string(),
        fork: flag("fork"),
        archived: flag("archived"),
        private: flag("private"),
        default_branch: text("default_branch"),
        pushed_at: text("pushed_at"),
        listed,
    })
}

/// Stars, forks, watchers and open issues for one day. `None` is a number
/// GitHub did not give, never a zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Counts {
    pub stars: Option<i64>,
    pub forks: Option<i64>,
    pub watchers: Option<i64>,
    pub open_issues: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Issue,
    PullRequest,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Issue => "issue",
            ItemKind::PullRequest => "pull_request",
        }
    }
}

/// An open issue, or an open pull request from outside (G5). The title is a
/// stranger's text, which is why the board's source is `third-party`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub number: i64,
    pub kind: ItemKind,
    pub title: String,
    pub author: Option<String>,
    pub author_association: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub labels: Vec<String>,
    pub comments: Option<i64>,
    /// The body's head, at most [`summary::BODY_CHARS_MAX`] characters, for
    /// the summary pass alone: **never written to the store and never
    /// serialized**, so no `--json` and no table carries a stranger's prose
    /// beyond the title the board already shows. `None` when GitHub gave none.
    #[serde(skip)]
    pub body: Option<String>,
    /// The hash of the whole body as GitHub gave it ([`summary::body_hash`]),
    /// which a summary is keyed by: an unchanged body is never summarised
    /// again, an edited one is. `None` when GitHub gave no body field.
    pub body_hash: Option<String>,
}

/// What one selected repository's GraphQL answer held.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoItems {
    pub repo: String,
    pub counts: Counts,
    pub items: Vec<Item>,
    /// Every open issue and pull request was read, so one not among them is
    /// no longer open. `false` marks nothing closed.
    pub complete: bool,
}

/// One default-branch workflow run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CiRun {
    pub id: i64,
    pub repo: String,
    pub workflow: Option<String>,
    pub workflow_id: Option<i64>,
    pub run_number: Option<i64>,
    pub attempt: Option<i64>,
    pub event: Option<String>,
    pub status: Option<String>,
    /// `None` while the run is in progress.
    pub conclusion: Option<String>,
    pub head_sha: Option<String>,
    pub branch: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
}

/// What `rate_limit` said, which costs nothing against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rate {
    pub core_remaining: i64,
    pub core_reset: i64,
    pub graphql_remaining: i64,
    pub graphql_reset: i64,
}

fn parse_rate(v: &Value) -> Option<Rate> {
    let r = v.get("resources")?;
    let of = |k: &str, f: &str| r.get(k)?.get(f)?.as_i64();
    Some(Rate {
        core_remaining: of("core", "remaining")?,
        core_reset: of("core", "reset")?,
        graphql_remaining: of("graphql", "remaining")?,
        graphql_reset: of("graphql", "reset")?,
    })
}

/// Calls spent between two readings of one window; `None` across a reset,
/// where the subtraction would mean nothing.
fn spent(before: Option<Rate>, after: Option<Rate>) -> (Option<i64>, Option<i64>) {
    let (Some(b), Some(a)) = (before, after) else {
        return (None, None);
    };
    let one = |rb: i64, ra: i64, bt: i64, at: i64| (bt == at && rb >= ra).then_some(rb - ra);
    (
        one(
            b.core_remaining,
            a.core_remaining,
            b.core_reset,
            a.core_reset,
        ),
        one(
            b.graphql_remaining,
            a.graphql_remaining,
            b.graphql_reset,
            a.graphql_reset,
        ),
    )
}

// ---- the log ----

/// A `collect_log` row's kind: a wire format, so a name never changes once
/// written. Refusals and failures are kept apart on purpose (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The run's closing row, written by every run whatever happened.
    Finished,
    /// GitHub refused for quota: REST, GraphQL or a secondary limit.
    RateLimited,
    /// `gh` has no login, or GitHub refused the one it has.
    Auth,
    /// No `gh` could be started.
    GhMissing,
    /// A repository that was watched is not there any more.
    Vanished,
    /// The run's budget was spent before it finished.
    OutOfTime,
    /// There is no `repos.toml`; nothing is selected.
    NoSelection,
    /// `repos.toml` is there and cannot be used; nothing is selected.
    BadSelection,
    /// Anything else: a transport error, an answer that did not parse.
    Failed,
    /// The summary pass did not run, or stopped before it was done: no
    /// model, no seat, a model switch, an outage, its budget spent. The
    /// detail says which; the items it did not reach wait for the next run.
    SummarySkipped,
    /// One item's summary call failed for a reason of its own (a request the
    /// model server rejected, a body that did not fit). Not a refusal: a
    /// refusal is an answer, stored, and never asked again for that body.
    SummaryFailed,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Finished,
        Kind::RateLimited,
        Kind::Auth,
        Kind::GhMissing,
        Kind::Vanished,
        Kind::OutOfTime,
        Kind::NoSelection,
        Kind::BadSelection,
        Kind::Failed,
        Kind::SummarySkipped,
        Kind::SummaryFailed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Finished => "finished",
            Kind::RateLimited => "rate_limited",
            Kind::Auth => "auth",
            Kind::GhMissing => "gh_missing",
            Kind::Vanished => "vanished",
            Kind::OutOfTime => "out_of_time",
            Kind::NoSelection => "no_selection",
            Kind::BadSelection => "bad_selection",
            Kind::Failed => "failed",
            Kind::SummarySkipped => "summary_skipped",
            Kind::SummaryFailed => "summary_failed",
        }
    }

    /// A problem of the summary pass's (§5), not the collection's: it never
    /// makes a repository "not read in full", and never fails the unit.
    pub fn is_summary(self) -> bool {
        matches!(self, Kind::SummarySkipped | Kind::SummaryFailed)
    }

    /// `None` for a kind this build does not know (a newer build's row),
    /// which the doctor reports rather than reading as clean.
    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Which part of a run a problem belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Selection,
    /// The first call, `rate_limit`, which also finds a missing `gh` or login.
    Preflight,
    Inventory,
    Items,
    Ci,
    /// The quick-summary pass (§5), after the collection is written.
    Summaries,
}

impl Step {
    pub fn as_str(self) -> &'static str {
        match self {
            Step::Selection => "selection",
            Step::Preflight => "preflight",
            Step::Inventory => "inventory",
            Step::Items => "items",
            Step::Ci => "ci",
            Step::Summaries => "summaries",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    pub kind: Kind,
    pub step: Step,
    pub repo: Option<String>,
    pub detail: String,
}

// ---- gh ----

/// Why a call to `gh` did not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhError {
    Missing(String),
    Auth(String),
    RateLimited(String),
    NotFound(String),
    OutOfTime,
    /// The answer passed [`ANSWER_BYTES_MAX`] (the cap it passed): refused
    /// whole, never read cut short.
    TooLarge(usize),
    Failed(String),
}

impl GhError {
    fn problem(&self, step: Step, repo: Option<&str>) -> Problem {
        let (kind, detail) = match self {
            GhError::Missing(d) => (Kind::GhMissing, d.clone()),
            GhError::Auth(d) => (Kind::Auth, d.clone()),
            GhError::RateLimited(d) => (Kind::RateLimited, d.clone()),
            // A not-found on a call that is not a repository's own lookup is
            // a failure of that call; `collect` turns the repository lookups'
            // into `Vanished` itself.
            GhError::NotFound(d) => (Kind::Failed, d.clone()),
            GhError::OutOfTime => (
                Kind::OutOfTime,
                format!(
                    "the run's {} s budget was spent before this step",
                    RUN_BUDGET.as_secs()
                ),
            ),
            GhError::TooLarge(cap) => (Kind::Failed, too_large(*cap)),
            GhError::Failed(d) => (Kind::Failed, d.clone()),
        };
        Problem {
            kind,
            step,
            repo: repo.map(str::to_string),
            detail,
        }
    }
}

fn too_large(cap: usize) -> String {
    format!(
        "gh's answer was larger than {} MiB and was not read",
        cap / (1024 * 1024)
    )
}

/// Read `pipe` to its end, keeping at most `cap` bytes: the rest is read and
/// dropped, so the child never stalls on a full pipe and this process never
/// holds more than the cap. `true` when anything was dropped.
fn drain_bounded(mut pipe: impl Read, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut over = false;
    let mut buf = [0u8; 64 * 1024];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                if n > room {
                    over = true;
                }
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    (kept, over)
}

/// What one finished `gh` process said.
#[derive(Debug, Clone)]
struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// The first line of what `gh` said on stderr, bounded: GitHub's own error
/// text, never the response body, which can hold a stranger's words.
fn first_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("gh failed and said nothing");
    line.chars().take(300).collect()
}

/// Classify a failed REST call, or a GraphQL call with no `data`, from what
/// `gh` printed: its exit code, its stderr (`gh: Not Found (HTTP 404)`), and
/// the `message` of the error body on stdout. Measured with gh 2.45: no login
/// exits 4 with "To get started with GitHub CLI, please run: gh auth login";
/// a refused token exits 1 with "gh: Bad credentials (HTTP 401)".
fn classify(ran: &Ran) -> GhError {
    let message = serde_json::from_str::<Value>(&ran.stdout)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let text = format!("{}\n{message}", ran.stderr).to_ascii_lowercase();
    let detail = first_line(&ran.stderr);
    if ran.code == Some(4)
        || text.contains("gh auth login")
        || text.contains("http 401")
        || text.contains("bad credentials")
    {
        GhError::Auth(detail)
    } else if text.contains("rate limit") || text.contains("http 429") {
        GhError::RateLimited(detail)
    } else if text.contains("http 404") {
        GhError::NotFound(detail)
    } else {
        GhError::Failed(detail)
    }
}

/// How the collector reaches `gh`. One run's calls share [`RUN_BUDGET`].
#[derive(Debug, Clone)]
pub struct Gh {
    bin: OsString,
    deadline: Instant,
    call_timeout: StdDuration,
    answer_cap: usize,
}

impl Gh {
    pub fn new(bin: impl Into<OsString>) -> Gh {
        Gh {
            bin: bin.into(),
            deadline: Instant::now() + RUN_BUDGET,
            call_timeout: CALL_TIMEOUT,
            answer_cap: ANSWER_BYTES_MAX,
        }
    }

    /// `$MECHA_GH_BIN` when set, else `gh` on `PATH`.
    pub fn from_env() -> Gh {
        match std::env::var_os("MECHA_GH_BIN") {
            Some(b) if !b.is_empty() => Gh::new(b),
            _ => Gh::new("gh"),
        }
    }

    /// A shorter allowance than [`RUN_BUDGET`], for a test.
    pub fn with_budget(mut self, budget: StdDuration) -> Gh {
        self.deadline = Instant::now() + budget;
        self
    }

    /// A smaller cap than [`ANSWER_BYTES_MAX`], for a test.
    pub fn with_answer_cap(mut self, cap: usize) -> Gh {
        self.answer_cap = cap;
        self
    }

    fn run(&self, args: &[&str]) -> Result<Ran, GhError> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        let timeout = left.min(self.call_timeout);
        if timeout.is_zero() {
            return Err(GhError::OutOfTime);
        }
        let mut tries = 0;
        let mut child = loop {
            let spawned = std::process::Command::new(&self.bin)
                .args(args)
                // Never a prompt, a pager, colour or an update check: this
                // runs with nobody watching.
                .env("GH_PROMPT_DISABLED", "1")
                .env("GH_NO_UPDATE_NOTIFIER", "1")
                .env("GH_PAGER", "cat")
                .env("NO_COLOR", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();
            match spawned {
                Ok(c) => break c,
                // A file just written whose descriptor another thread's fork
                // still holds; it clears on its own (`engine::run_within`).
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(StdDuration::from_millis(50));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(GhError::Missing(format!(
                        "{} was not found (gh on PATH, or MECHA_GH_BIN)",
                        self.bin.to_string_lossy()
                    )))
                }
                Err(e) => {
                    return Err(GhError::Missing(format!(
                        "{} could not be started: {e}",
                        self.bin.to_string_lossy()
                    )))
                }
            }
        };
        // Drained while it runs: an inventory of four hundred repositories is
        // more than a pipe holds, and waiting first would stall the child.
        // Bounded as it is read — the cap is at the door, not after the parse.
        let drain = |pipe: Option<Box<dyn Read + Send>>, cap: usize| {
            std::thread::spawn(move || match pipe {
                Some(p) => drain_bounded(p, cap),
                None => (Vec::new(), false),
            })
        };
        let out = drain(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            self.answer_cap,
        );
        let err = drain(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            STDERR_BYTES_MAX,
        );
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if started.elapsed() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(StdDuration::from_millis(20)),
                Err(_) => break None,
            }
        };
        let (stdout, stdout_over) = out.join().unwrap_or_default();
        let (stderr, _) = err.join().unwrap_or_default();
        let stdout = String::from_utf8_lossy(&stdout).into_owned();
        let stderr = String::from_utf8_lossy(&stderr).into_owned();
        match status {
            None if Instant::now() >= self.deadline => Err(GhError::OutOfTime),
            None => Err(GhError::Failed(format!(
                "gh did not answer within {} s",
                timeout.as_secs()
            ))),
            // Refused whole, whatever the exit: a JSON document cut at the
            // cap is not the answer, and a prefix of it may still parse.
            Some(_) if stdout_over => Err(GhError::TooLarge(self.answer_cap)),
            Some(s) => Ok(Ran {
                code: s.code(),
                stdout,
                stderr,
            }),
        }
    }

    /// A REST call; its body when it succeeded.
    fn rest(&self, args: &[&str]) -> Result<String, GhError> {
        let ran = self.run(args)?;
        if ran.code == Some(0) {
            Ok(ran.stdout)
        } else {
            Err(classify(&ran))
        }
    }

    /// A GraphQL call. An answer carrying `data` is returned even when `gh`
    /// exited non-zero, because one repository not found in a batch is an
    /// error beside the other nine's data.
    fn graphql(&self, query: &str, vars: &[(String, String)]) -> Result<Value, GhError> {
        let q = format!("query={query}");
        let pairs: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut args = vec!["api", "graphql", "-f", q.as_str()];
        for p in &pairs {
            args.push("-f");
            args.push(p);
        }
        let ran = self.run(&args)?;
        let parsed = serde_json::from_str::<Value>(&ran.stdout).ok();
        match parsed {
            Some(v) if v.get("data").is_some_and(|d| !d.is_null()) => Ok(v),
            Some(v) if ran.code == Some(0) || graphql_errors(&v).next().is_some() => {
                // No data: the errors say why, by type.
                match graphql_errors(&v).find_map(|(_, t, m)| (t == "RATE_LIMITED").then_some(m)) {
                    Some(m) => Err(GhError::RateLimited(m)),
                    None if ran.code == Some(0) => {
                        Err(GhError::Failed("a GraphQL answer with no data".into()))
                    }
                    None => Err(classify(&ran)),
                }
            }
            _ => Err(classify(&ran)),
        }
    }

    fn rate(&self) -> Result<Rate, GhError> {
        let body = self.rest(&["api", "rate_limit"])?;
        serde_json::from_str::<Value>(&body)
            .ok()
            .as_ref()
            .and_then(parse_rate)
            .ok_or_else(|| GhError::Failed("rate_limit answered something unreadable".into()))
    }
}

/// `(first path element, type, message)` of each GraphQL error.
fn graphql_errors(v: &Value) -> impl Iterator<Item = (Option<String>, String, String)> + '_ {
    v.get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            let path = e
                .get("path")
                .and_then(Value::as_array)
                .and_then(|p| p.first())
                .and_then(Value::as_str)
                .map(str::to_string);
            let kind = e
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let message: String = e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(300)
                .collect();
            (path, kind, message)
        })
}

/// Every JSON value in `text`: `gh api --paginate` prints one array per page,
/// back to back.
fn json_stream(text: &str) -> Result<Vec<Value>, String> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("an answer that did not parse: {e}"))
}

// ---- the GraphQL ----

/// `body` is a scalar: it adds bytes to the answer and nothing to GitHub's
/// node count, so the batch's cost bound (`BATCH`) is unchanged by it. The
/// bytes are bounded at the door ([`ANSWER_BYTES_MAX`]); an answer refused
/// for its size is asked again with [`ITEM_FIELDS_NO_BODY`], so the
/// collection never depends on what strangers wrote at length.
const ITEM_FIELDS: &str = "number title body createdAt updatedAt authorAssociation \
     author { login } comments { totalCount } labels(first: 20) { nodes { name } }";
const ITEM_FIELDS_NO_BODY: &str = "number title createdAt updatedAt authorAssociation \
     author { login } comments { totalCount } labels(first: 20) { nodes { name } }";

fn item_fields(bodies: bool) -> &'static str {
    if bodies {
        ITEM_FIELDS
    } else {
        ITEM_FIELDS_NO_BODY
    }
}

/// Why a listing's items carry no body this run, for the log.
fn bodies_unread(cap: usize) -> String {
    format!(
        "the listing with issue bodies was larger than {} MiB, so it was read again without \
         them: its items are not summarised this run",
        cap / (1024 * 1024)
    )
}

fn batch_query(n: usize, bodies: bool) -> String {
    let fields = item_fields(bodies);
    let mut q = String::from("query(");
    for i in 0..n {
        if i > 0 {
            q.push_str(", ");
        }
        q.push_str(&format!("$o{i}: String!, $n{i}: String!"));
    }
    q.push_str(") {\n");
    for i in 0..n {
        q.push_str(&format!(
            "  r{i}: repository(owner: $o{i}, name: $n{i}) {{ ...R }}\n"
        ));
    }
    q.push_str(&format!(
        "}}\nfragment R on Repository {{\n  stargazerCount forkCount watchers {{ totalCount }}\n  \
         issues(states: OPEN, first: 100, orderBy: {{field: CREATED_AT, direction: DESC}}) {{\n    \
         totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} }}\n  }}\n  \
         pullRequests(states: OPEN, first: 100, orderBy: {{field: CREATED_AT, direction: DESC}}) {{\n    \
         totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} }}\n  }}\n}}\n"
    ));
    q
}

fn page_query(connection: &str, bodies: bool) -> String {
    let fields = item_fields(bodies);
    format!(
        "query($o: String!, $n: String!, $after: String!) {{\n  repository(owner: $o, name: $n) {{\n    \
         {connection}(states: OPEN, first: 100, after: $after, orderBy: {{field: CREATED_AT, direction: DESC}}) {{\n      \
         pageInfo {{ hasNextPage endCursor }} nodes {{ {fields} }}\n    }}\n  }}\n}}\n"
    )
}

fn parse_item(v: &Value, kind: ItemKind) -> Option<Item> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let body = v.get("body").and_then(Value::as_str);
    Some(Item {
        body: body.map(summary::body_head),
        body_hash: body.map(summary::body_hash),
        number: v.get("number")?.as_i64()?,
        kind,
        title: text("title")?,
        author: v
            .get("author")
            .and_then(|a| a.get("login"))
            .and_then(Value::as_str)
            .map(str::to_string),
        author_association: text("authorAssociation"),
        created_at: text("createdAt"),
        updated_at: text("updatedAt"),
        labels: v
            .get("labels")
            .and_then(|l| l.get("nodes"))
            .and_then(Value::as_array)
            .map(|ns| {
                ns.iter()
                    .filter_map(|n| n.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        comments: v
            .get("comments")
            .and_then(|c| c.get("totalCount"))
            .and_then(Value::as_i64),
    })
}

/// Whether an item is on the board: every issue, and a pull request whose
/// author is not the owner's own side (G5).
fn kept(item: &Item) -> bool {
    match item.kind {
        ItemKind::Issue => true,
        ItemKind::PullRequest => !item
            .author_association
            .as_deref()
            .is_some_and(|a| INSIDE.contains(&a)),
    }
}

/// One connection's nodes and whether there is more.
fn connection_page(conn: &Value, kind: ItemKind) -> (Vec<Item>, Option<String>) {
    let items = conn
        .get("nodes")
        .and_then(Value::as_array)
        .map(|ns| ns.iter().filter_map(|n| parse_item(n, kind)).collect())
        .unwrap_or_default();
    let info = conn.get("pageInfo");
    let more = info
        .and_then(|p| p.get("hasNextPage"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cursor = info
        .and_then(|p| p.get("endCursor"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (
        items,
        if more {
            cursor.or(Some(String::new()))
        } else {
            None
        },
    )
}

// ---- one run ----

/// Everything one run found, before any of it is written ([`record`]).
#[derive(Debug, Clone, Serialize)]
pub struct Collected {
    pub started: DateTime<Utc>,
    pub finished: DateTime<Utc>,
    /// The login's listing, each with whether the selection takes it. `None`
    /// when it could not be listed: the table keeps the last one.
    pub inventory: Option<Vec<(Repo, bool)>>,
    /// How many repositories the selection took; `None` when the run
    /// stopped before it could tell (no `gh`, no login), never a zero.
    pub selected: Option<usize>,
    pub repos: Vec<RepoItems>,
    pub ci: Vec<CiRun>,
    /// Repositories whose workflow runs were read, none or many.
    pub ci_read: Vec<String>,
    pub problems: Vec<Problem>,
    pub rate_before: Option<Rate>,
    pub rate_after: Option<Rate>,
}

impl Collected {
    /// A run that started at `now` and has found nothing yet.
    pub fn new(now: DateTime<Utc>) -> Collected {
        Collected {
            started: now,
            finished: now,
            inventory: None,
            selected: None,
            repos: Vec::new(),
            ci: Vec::new(),
            ci_read: Vec::new(),
            problems: Vec::new(),
            rate_before: None,
            rate_after: None,
        }
    }

    /// Selected repositories whose items and workflow runs were both read
    /// this run, with no problem naming them.
    pub fn collected(&self) -> usize {
        let troubled: BTreeSet<&str> = self
            .problems
            .iter()
            .filter(|p| !p.kind.is_summary())
            .filter_map(|p| p.repo.as_deref())
            .collect();
        let ci: BTreeSet<&str> = self.ci_read.iter().map(String::as_str).collect();
        self.repos
            .iter()
            .filter(|r| ci.contains(r.repo.as_str()) && !troubled.contains(r.repo.as_str()))
            .count()
    }
}

/// What the store last held of the inventory: the fallback selection when the
/// listing fails, and the watched set a vanished repository is missed from.
fn prior_inventory(db: &Path) -> Vec<(Repo, bool)> {
    let Ok(conn) = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT repo, owner, name, fork, archived, private, default_branch, pushed_at, listed, selected FROM inventory",
    ) else {
        return Vec::new();
    };
    stmt.query_map([], |r| {
        Ok((
            Repo {
                full_name: r.get(0)?,
                owner: r.get(1)?,
                name: r.get(2)?,
                fork: r.get(3)?,
                archived: r.get(4)?,
                private: r.get(5)?,
                default_branch: r.get(6)?,
                pushed_at: r.get(7)?,
                listed: r.get(8)?,
            },
            r.get::<_, bool>(9)?,
        ))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// The repositories a board may link an issue to (§4): rows of the
/// collector's `inventory` — repositories the login was seen to reach — that
/// the selection file includes **now**. Both halves, because neither is
/// enough alone: an owner line with `include = "all"` (G6's default) names no
/// repositories, so the file alone leaves the repository unconstrained; and
/// the inventory alone holds every repository the login can see, selected or
/// not.
///
/// Read where a dataset is served, never by a board's loader — the board's
/// own rows are what is being checked, so they cannot also be the list they
/// are checked against. **Fail closed**: no store, no inventory table, an
/// absent or unusable selection file — each is an empty set, and a board
/// over an empty set renders every reference as text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watched(BTreeSet<(String, String)>);

impl Watched {
    pub fn read(home: &Path) -> Watched {
        let SelectionRead::Read(selection) = read_selection(&selection_path(home)) else {
            return Watched::default();
        };
        Watched(
            prior_inventory(&db_path(home))
                .into_iter()
                .filter(|(r, _)| selection.selects(r))
                .map(|(r, _)| (r.owner.to_ascii_lowercase(), r.name.to_ascii_lowercase()))
                .collect(),
        )
    }

    /// GitHub's names are case-insensitive, so the comparison is too; the
    /// link is built from the row's own spelling, which reaches the same
    /// repository.
    pub fn contains(&self, owner: &str, repo: &str) -> bool {
        self.0
            .contains(&(owner.to_ascii_lowercase(), repo.to_ascii_lowercase()))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One collection: read the selection, ask `gh`, and say what was found.
/// Writes nothing — [`record`] does, in one transaction.
pub fn collect(gh: &Gh, home: &Path, now: DateTime<Utc>) -> Collected {
    let clock = Instant::now();
    let mut c = Collected::new(now);
    run_steps(gh, home, now, &mut c);
    c.finished = now + Duration::from_std(clock.elapsed()).unwrap_or_default();
    c
}

fn run_steps(gh: &Gh, home: &Path, now: DateTime<Utc>, c: &mut Collected) {
    let selection = match read_selection(&selection_path(home)) {
        SelectionRead::Read(s) => s,
        SelectionRead::Absent => {
            c.problems.push(Problem {
                kind: Kind::NoSelection,
                step: Step::Selection,
                repo: None,
                detail: format!(
                    "{} is not there; nothing is selected",
                    selection_path(home).display()
                ),
            });
            Selection::default()
        }
        SelectionRead::Invalid(why) => {
            c.problems.push(Problem {
                kind: Kind::BadSelection,
                step: Step::Selection,
                repo: None,
                detail: why,
            });
            Selection::default()
        }
    };

    // The first call finds a missing `gh` and a missing login before any
    // other is made, and reads the quota the run then spends.
    match gh.rate() {
        Ok(r) => c.rate_before = Some(r),
        Err(e) => {
            c.problems.push(e.problem(Step::Preflight, None));
            return;
        }
    }

    // ---- inventory ----
    let prior = prior_inventory(&db_path(home));
    let mut rest_limited = false;
    let listing = gh
        .rest(&[
            "api",
            "--paginate",
            "user/repos?per_page=100&affiliation=owner,collaborator,organization_member",
        ])
        .and_then(|body| json_stream(&body).map_err(GhError::Failed));
    let mut repos: BTreeMap<String, Repo> = BTreeMap::new();
    let listed = match listing {
        Ok(pages) => {
            for page in &pages {
                for v in page.as_array().into_iter().flatten() {
                    match parse_repo(v, true) {
                        Some(r) => {
                            repos.insert(r.full_name.to_ascii_lowercase(), r);
                        }
                        None => c.problems.push(Problem {
                            kind: Kind::Failed,
                            step: Step::Inventory,
                            repo: None,
                            detail:
                                "a repository whose name is outside GitHub's grammar was skipped"
                                    .into(),
                        }),
                    }
                }
            }
            true
        }
        Err(e) => {
            if matches!(e, GhError::Auth(_) | GhError::Missing(_)) {
                c.problems.push(e.problem(Step::Inventory, None));
                return;
            }
            rest_limited = matches!(e, GhError::RateLimited(_));
            let out_of_time = e == GhError::OutOfTime;
            c.problems.push(e.problem(Step::Inventory, None));
            if out_of_time {
                return;
            }
            // The last inventory decides what is watched this run.
            for (r, _) in &prior {
                repos.insert(r.full_name.to_ascii_lowercase(), r.clone());
            }
            false
        }
    };

    if listed {
        // A watched repository the login's listing no longer holds.
        for (r, was_selected) in &prior {
            if *was_selected && r.listed && !repos.contains_key(&r.full_name.to_ascii_lowercase()) {
                c.problems.push(Problem {
                    kind: Kind::Vanished,
                    step: Step::Inventory,
                    repo: Some(r.full_name.clone()),
                    detail: "no longer in the login's listing: deleted, renamed, transferred, \
                             or no longer visible to this login"
                        .into(),
                });
            }
        }
        // A `[[repo]]` line that includes a repository outside the listing
        // (someone else's public one) is asked for by name.
        let wanted: Vec<String> = selection
            .included_by_name()
            .filter(|n| !repos.contains_key(&n.to_ascii_lowercase()))
            .map(str::to_string)
            .collect();
        for name in wanted {
            if rest_limited {
                break;
            }
            let path = format!("repos/{name}");
            match gh.rest(&["api", path.as_str()]) {
                Ok(body) => match serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| parse_repo(&v, false))
                {
                    Some(r) => {
                        repos.insert(r.full_name.to_ascii_lowercase(), r);
                    }
                    None => c.problems.push(Problem {
                        kind: Kind::Failed,
                        step: Step::Inventory,
                        repo: Some(name.clone()),
                        detail: "its repository answer could not be read".into(),
                    }),
                },
                Err(GhError::NotFound(d)) => c.problems.push(Problem {
                    kind: Kind::Vanished,
                    step: Step::Inventory,
                    repo: Some(name.clone()),
                    detail: format!("a [[repo]] line includes it and GitHub says: {d}"),
                }),
                Err(e) => {
                    let stop = matches!(
                        e,
                        GhError::Auth(_) | GhError::Missing(_) | GhError::OutOfTime
                    );
                    rest_limited |= matches!(e, GhError::RateLimited(_));
                    c.problems.push(e.problem(Step::Inventory, Some(&name)));
                    if stop {
                        return;
                    }
                }
            }
        }
    }

    let selected: Vec<Repo> = repos
        .values()
        .filter(|r| selection.selects(r))
        .cloned()
        .collect();
    // Unknown, not zero, when nothing told us what the login can see: the
    // listing failed and no earlier inventory stood in for it. Zero here read
    // to the doctor as "the selection matches nothing" (review of #898).
    c.selected = (listed || !prior.is_empty()).then_some(selected.len());
    if listed {
        c.inventory = Some(
            repos
                .values()
                .map(|r| (r.clone(), selection.selects(r)))
                .collect(),
        );
    }

    // ---- items and counts, by GraphQL, a batch per owner ----
    let mut by_owner: BTreeMap<String, Vec<&Repo>> = BTreeMap::new();
    for r in &selected {
        by_owner
            .entry(r.owner.to_ascii_lowercase())
            .or_default()
            .push(r);
    }
    'graphql: for chunk in by_owner.values().flat_map(|rs| rs.chunks(BATCH)) {
        let vars: Vec<(String, String)> = chunk
            .iter()
            .enumerate()
            .flat_map(|(i, r)| {
                [
                    (format!("o{i}"), r.owner.clone()),
                    (format!("n{i}"), r.name.clone()),
                ]
            })
            .collect();
        // Bodies first; a batch refused for its size is asked once more
        // without them, so the collection is never the price of a long body.
        let answer = match gh.graphql(&batch_query(chunk.len(), true), &vars) {
            Err(GhError::TooLarge(cap)) => {
                for r in chunk {
                    c.problems.push(Problem {
                        kind: Kind::SummarySkipped,
                        step: Step::Items,
                        repo: Some(r.full_name.clone()),
                        detail: bodies_unread(cap),
                    });
                }
                gh.graphql(&batch_query(chunk.len(), false), &vars)
            }
            other => other,
        };
        let answer = match answer {
            Ok(v) => v,
            Err(e) => match e {
                GhError::Auth(_) | GhError::Missing(_) | GhError::OutOfTime => {
                    c.problems.push(e.problem(Step::Items, None));
                    return;
                }
                GhError::RateLimited(_) => {
                    c.problems.push(e.problem(Step::Items, None));
                    break 'graphql;
                }
                _ => {
                    for r in chunk {
                        c.problems.push(e.problem(Step::Items, Some(&r.full_name)));
                    }
                    continue;
                }
            },
        };
        let errors: Vec<_> = graphql_errors(&answer).collect();
        let mut limited = false;
        for (i, r) in chunk.iter().enumerate() {
            let alias = format!("r{i}");
            let data = answer.get("data").and_then(|d| d.get(&alias));
            match data {
                Some(d) if d.is_object() => match repo_items(gh, r, d) {
                    Ok((items, unread)) => {
                        if let Some(cap) = unread {
                            c.problems.push(Problem {
                                kind: Kind::SummarySkipped,
                                step: Step::Items,
                                repo: Some(r.full_name.clone()),
                                detail: bodies_unread(cap),
                            });
                        }
                        // A listing not read to its end marks nothing closed
                        // (`store`), and is said as what it is: unknown is
                        // never clean, so the repo is not "read in full" and
                        // the doctor sees the row (review of #898).
                        if !items.complete {
                            c.problems.push(Problem {
                                kind: Kind::Failed,
                                step: Step::Items,
                                repo: Some(r.full_name.clone()),
                                detail: "the issue and pull-request listing was cut short \
                                         (a page did not answer, or the page cap was reached); \
                                         nothing was marked closed"
                                    .into(),
                            });
                        }
                        c.repos.push(items);
                    }
                    Err(e) => {
                        let stop = matches!(
                            e,
                            GhError::Auth(_) | GhError::Missing(_) | GhError::OutOfTime
                        );
                        let limited = matches!(e, GhError::RateLimited(_));
                        c.problems.push(e.problem(Step::Items, Some(&r.full_name)));
                        if stop {
                            return;
                        }
                        // A quota refused on a follow-up page is the same
                        // refusal the batch and alias paths stop GraphQL for:
                        // spending it further is how a secondary limit trips
                        // (review of #898).
                        if limited {
                            break 'graphql;
                        }
                    }
                },
                _ => {
                    let mine = errors
                        .iter()
                        .find(|(p, _, _)| p.as_deref() == Some(alias.as_str()));
                    let (kind, detail) = match mine {
                        Some((_, t, m)) if t == "NOT_FOUND" => (Kind::Vanished, m.clone()),
                        Some((_, t, m)) if t == "RATE_LIMITED" => {
                            limited = true;
                            (Kind::RateLimited, m.clone())
                        }
                        Some((_, _, m)) => (Kind::Failed, m.clone()),
                        None => (Kind::Failed, "no data for it in the GraphQL answer".into()),
                    };
                    c.problems.push(Problem {
                        kind,
                        step: Step::Items,
                        repo: Some(r.full_name.clone()),
                        detail,
                    });
                }
            }
        }
        if limited {
            break 'graphql;
        }
    }

    // ---- CI on the default branch ----
    let since = (now - Duration::days(CI_KEEP_DAYS))
        .format("created=>=%Y-%m-%d")
        .to_string();
    for r in &selected {
        if rest_limited {
            break;
        }
        let Some(branch) = r.default_branch.as_deref() else {
            // No default branch (an empty repository): nothing to read.
            c.ci_read.push(r.full_name.clone());
            continue;
        };
        let path = format!("repos/{}/actions/runs", r.full_name);
        let branch_arg = format!("branch={branch}");
        let per_page = format!("per_page={CI_PER_PAGE}");
        let args = [
            "api",
            "-X",
            "GET",
            path.as_str(),
            "-f",
            branch_arg.as_str(),
            "-f",
            per_page.as_str(),
            "-f",
            "exclude_pull_requests=true",
            "-f",
            since.as_str(),
        ];
        match gh.rest(&args) {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(v) => {
                    c.ci.extend(parse_runs(&v, r, branch));
                    c.ci_read.push(r.full_name.clone());
                }
                Err(e) => c.problems.push(Problem {
                    kind: Kind::Failed,
                    step: Step::Ci,
                    repo: Some(r.full_name.clone()),
                    detail: format!("its workflow runs did not parse: {e}"),
                }),
            },
            Err(e) => {
                let stop = matches!(
                    e,
                    GhError::Auth(_) | GhError::Missing(_) | GhError::OutOfTime
                );
                rest_limited = matches!(e, GhError::RateLimited(_));
                let repo = (!rest_limited).then_some(r.full_name.as_str());
                c.problems.push(e.problem(Step::Ci, repo));
                if stop {
                    return;
                }
            }
        }
    }

    c.rate_after = gh.rate().ok();
}

/// A repository's GraphQL object, with any further pages of its issues and
/// pull requests asked for — with bodies while they fit, and, from the first
/// page refused for its size, without them. The cap that page passed rides
/// out beside the items, for the log.
fn repo_items(gh: &Gh, r: &Repo, d: &Value) -> Result<(RepoItems, Option<usize>), GhError> {
    let mut bodies = true;
    let mut unread = None;
    let total = |k: &str| {
        d.get(k)
            .and_then(|v| v.get("totalCount"))
            .and_then(Value::as_i64)
    };
    let counts = Counts {
        stars: d.get("stargazerCount").and_then(Value::as_i64),
        forks: d.get("forkCount").and_then(Value::as_i64),
        watchers: total("watchers"),
        open_issues: total("issues"),
    };
    let mut items = Vec::new();
    let mut complete = true;
    for (field, kind) in [
        ("issues", ItemKind::Issue),
        ("pullRequests", ItemKind::PullRequest),
    ] {
        let Some(conn) = d.get(field).filter(|v| v.is_object()) else {
            complete = false;
            continue;
        };
        let (mut got, mut more) = connection_page(conn, kind);
        items.append(&mut got);
        let mut pages = 0;
        while let Some(cursor) = more.take() {
            if pages >= MAX_PAGES || cursor.is_empty() {
                complete = false;
                break;
            }
            pages += 1;
            let vars = [
                ("o".to_string(), r.owner.clone()),
                ("n".to_string(), r.name.clone()),
                ("after".to_string(), cursor),
            ];
            let page = match gh.graphql(&page_query(field, bodies), &vars) {
                Err(GhError::TooLarge(cap)) if bodies => {
                    bodies = false;
                    unread = Some(cap);
                    gh.graphql(&page_query(field, false), &vars)?
                }
                other => other?,
            };
            match page
                .get("data")
                .and_then(|d| d.get("repository"))
                .and_then(|r| r.get(field))
                .filter(|v| v.is_object())
            {
                Some(conn) => {
                    let (mut got, next) = connection_page(conn, kind);
                    items.append(&mut got);
                    more = next;
                }
                None => {
                    complete = false;
                    break;
                }
            }
        }
    }
    items.retain(kept);
    Ok((
        RepoItems {
            repo: r.full_name.clone(),
            counts,
            items,
            complete,
        },
        unread,
    ))
}

/// The default branch's own runs: on that branch, and from this repository —
/// a fork's pull request from a branch of the same name is not one.
fn parse_runs(v: &Value, r: &Repo, branch: &str) -> Vec<CiRun> {
    let text = |run: &Value, k: &str| run.get(k).and_then(Value::as_str).map(str::to_string);
    let int = |run: &Value, k: &str| run.get(k).and_then(Value::as_i64);
    v.get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|run| run.get("head_branch").and_then(Value::as_str) == Some(branch))
        .filter(|run| {
            run.get("head_repository")
                .and_then(|h| h.get("full_name"))
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(&r.full_name))
        })
        .filter_map(|run| {
            Some(CiRun {
                id: int(run, "id")?,
                repo: r.full_name.clone(),
                workflow: text(run, "name"),
                workflow_id: int(run, "workflow_id"),
                run_number: int(run, "run_number"),
                attempt: int(run, "run_attempt"),
                event: text(run, "event"),
                status: text(run, "status"),
                conclusion: text(run, "conclusion"),
                head_sha: text(run, "head_sha"),
                branch: text(run, "head_branch"),
                created_at: text(run, "created_at")?,
                started_at: text(run, "run_started_at"),
                updated_at: text(run, "updated_at"),
            })
        })
        .collect()
}

// ---- the store ----

/// The store's tables. `pub(crate)` so a reader's tests build the real
/// schema rather than a copy of it.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS inventory (
  repo TEXT PRIMARY KEY, owner TEXT NOT NULL, name TEXT NOT NULL,
  fork INTEGER NOT NULL, archived INTEGER NOT NULL, private INTEGER NOT NULL,
  default_branch TEXT, pushed_at TEXT,
  listed INTEGER NOT NULL, selected INTEGER NOT NULL, seen_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS ci_runs (
  id INTEGER PRIMARY KEY, repo TEXT NOT NULL, workflow TEXT, workflow_id INTEGER,
  run_number INTEGER, attempt INTEGER, event TEXT, status TEXT, conclusion TEXT,
  head_sha TEXT, branch TEXT, created_at TEXT NOT NULL, started_at TEXT, updated_at TEXT,
  seen_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS ci_runs_repo ON ci_runs (repo, created_at);
CREATE TABLE IF NOT EXISTS items (
  repo TEXT NOT NULL, number INTEGER NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
  author TEXT, author_association TEXT, created_at TEXT, updated_at TEXT,
  labels TEXT NOT NULL, comments INTEGER,
  first_seen TEXT NOT NULL, last_seen TEXT NOT NULL, gone_at TEXT, body_hash TEXT,
  PRIMARY KEY (repo, number));
CREATE TABLE IF NOT EXISTS summaries (
  repo TEXT NOT NULL, number INTEGER NOT NULL, body_hash TEXT NOT NULL,
  outcome TEXT NOT NULL, summary TEXT, kind TEXT, refusal TEXT, model TEXT, at TEXT NOT NULL,
  PRIMARY KEY (repo, number, body_hash));
CREATE TABLE IF NOT EXISTS counts (
  repo TEXT NOT NULL, day TEXT NOT NULL,
  stars INTEGER, forks INTEGER, watchers INTEGER, open_issues INTEGER, at TEXT NOT NULL,
  PRIMARY KEY (repo, day));
CREATE TABLE IF NOT EXISTS collect_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT, run TEXT NOT NULL, at TEXT NOT NULL,
  kind TEXT NOT NULL, step TEXT, repo TEXT, detail TEXT,
  selected INTEGER, collected INTEGER,
  core_used INTEGER, core_remaining INTEGER, graphql_used INTEGER, graphql_remaining INTEGER,
  summarised INTEGER, summary_refused INTEGER, summary_failed INTEGER, summary_deferred INTEGER);
CREATE INDEX IF NOT EXISTS collect_log_run ON collect_log (run);
";

/// Columns added to a table after its first release, as `(table, column,
/// declaration)`. `SCHEMA`'s `CREATE TABLE IF NOT EXISTS` makes them on a new
/// store and does nothing to an old one, so [`open_store`] adds each that a
/// store lacks — and a reader asks [`has_column`] before reading one, because
/// a store the new collector has not written yet does not have it.
const ADDED: [(&str, &str, &str); 5] = [
    ("items", "body_hash", "TEXT"),
    ("collect_log", "summarised", "INTEGER"),
    ("collect_log", "summary_refused", "INTEGER"),
    ("collect_log", "summary_failed", "INTEGER"),
    ("collect_log", "summary_deferred", "INTEGER"),
];

pub(crate) fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2")?;
    Ok(stmt.exists(params![table, column])?)
}

/// The store, opened for writing: its directory made, the schema applied and
/// any column a store from an earlier build lacks added.
pub(crate) fn open_store(db: &Path) -> Result<Connection> {
    if let Some(dir) = db.parent() {
        crate::create_private_dir(dir)?;
    }
    let conn = Connection::open(db).with_context(|| format!("opening {}", db.display()))?;
    // As `system::record`: a board refreshing mid-write makes the writer wait
    // rather than lose the run, and rollback-journal mode because the HUD's
    // loaders open read-only and could not recreate WAL's -shm file.
    conn.busy_timeout(StdDuration::from_secs(5))?;
    conn.execute_batch(SCHEMA)?;
    for (table, column, decl) in ADDED {
        if !has_column(&conn, table, column)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
        }
    }
    Ok(conn)
}

/// Write one run: the inventory (when it was listed), today's counts (once a
/// day: a row already there is kept), items and workflow runs, what has
/// closed, retention, and the run's log rows — in one transaction, so a
/// reader never sees half a run.
pub fn record(db: &Path, c: &Collected) -> Result<()> {
    let mut conn = open_store(db)?;
    let tx = conn.transaction()?;
    let run = ts(c.started);
    let at = ts(c.finished);

    if let Some(inventory) = &c.inventory {
        tx.execute("DELETE FROM inventory", [])?;
        for (r, selected) in inventory {
            tx.execute(
                "INSERT INTO inventory (repo, owner, name, fork, archived, private, default_branch, pushed_at, listed, selected, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    r.full_name,
                    r.owner,
                    r.name,
                    r.fork,
                    r.archived,
                    r.private,
                    r.default_branch,
                    r.pushed_at,
                    r.listed,
                    selected,
                    run
                ],
            )?;
        }
    }

    let day = c.started.format("%Y-%m-%d").to_string();
    for r in &c.repos {
        tx.execute(
            "INSERT OR IGNORE INTO counts (repo, day, stars, forks, watchers, open_issues, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                r.repo,
                day,
                r.counts.stars,
                r.counts.forks,
                r.counts.watchers,
                r.counts.open_issues,
                run
            ],
        )?;
        for i in &r.items {
            tx.execute(
                "INSERT INTO items (repo, number, kind, title, author, author_association, created_at, updated_at, labels, comments, first_seen, last_seen, gone_at, body_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, NULL, ?12)
                 ON CONFLICT (repo, number) DO UPDATE SET
                   kind = excluded.kind, title = excluded.title, author = excluded.author,
                   author_association = excluded.author_association, created_at = excluded.created_at,
                   updated_at = excluded.updated_at, labels = excluded.labels, comments = excluded.comments,
                   last_seen = excluded.last_seen, gone_at = NULL, body_hash = excluded.body_hash",
                params![
                    r.repo,
                    i.number,
                    i.kind.as_str(),
                    i.title,
                    i.author,
                    i.author_association,
                    i.created_at,
                    i.updated_at,
                    serde_json::to_string(&i.labels)?,
                    i.comments,
                    run,
                    i.body_hash
                ],
            )?;
        }
        if r.complete {
            // Read to its end and not there: no longer open (closed, merged,
            // transferred or deleted — the listing cannot say which).
            tx.execute(
                "UPDATE items SET gone_at = ?2 WHERE repo = ?1 AND gone_at IS NULL AND last_seen <> ?2",
                params![r.repo, run],
            )?;
        }
    }

    for w in &c.ci {
        tx.execute(
            "INSERT INTO ci_runs (id, repo, workflow, workflow_id, run_number, attempt, event, status, conclusion, head_sha, branch, created_at, started_at, updated_at, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT (id) DO UPDATE SET
               workflow = excluded.workflow, attempt = excluded.attempt, status = excluded.status,
               conclusion = excluded.conclusion, started_at = excluded.started_at,
               updated_at = excluded.updated_at, seen_at = excluded.seen_at",
            params![
                w.id,
                w.repo,
                w.workflow,
                w.workflow_id,
                w.run_number,
                w.attempt,
                w.event,
                w.status,
                w.conclusion,
                w.head_sha,
                w.branch,
                w.created_at,
                w.started_at,
                w.updated_at,
                run
            ],
        )?;
    }

    // Retention, against the run's own clock.
    tx.execute(
        "DELETE FROM ci_runs WHERE created_at < ?1",
        params![ts(c.started - Duration::days(CI_KEEP_DAYS))],
    )?;
    tx.execute(
        "DELETE FROM items WHERE coalesce(gone_at, last_seen) < ?1",
        params![ts(c.started - Duration::days(ITEM_KEEP_DAYS))],
    )?;
    // A summary goes with its item (§3): every revision's row, once the item
    // itself is pruned.
    tx.execute(
        "DELETE FROM summaries WHERE NOT EXISTS
           (SELECT 1 FROM items i WHERE i.repo = summaries.repo AND i.number = summaries.number)",
        [],
    )?;
    tx.execute(
        "DELETE FROM collect_log WHERE run < ?1",
        params![ts(c.started - Duration::days(LOG_KEEP_DAYS))],
    )?;

    for p in &c.problems {
        tx.execute(
            "INSERT INTO collect_log (run, at, kind, step, repo, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run, at, p.kind.as_str(), p.step.as_str(), p.repo, p.detail],
        )?;
    }
    let (core_used, graphql_used) = spent(c.rate_before, c.rate_after);
    let last = c.rate_after.or(c.rate_before);
    tx.execute(
        "INSERT INTO collect_log (run, at, kind, selected, collected, core_used, core_remaining, graphql_used, graphql_remaining)
         VALUES (?1, ?2, 'finished', ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run,
            at,
            c.selected.map(|n| n as i64),
            c.collected() as i64,
            core_used,
            last.map(|r| r.core_remaining),
            graphql_used,
            last.map(|r| r.graphql_remaining)
        ],
    )?;
    tx.commit()?;
    Ok(())
}

// ---- reading the log back: the doctor's question ----

/// A logged problem, read back. `kind` is `None` for a kind this build does
/// not know.
#[derive(Debug, Clone, PartialEq)]
pub struct Logged {
    pub kind: Option<Kind>,
    pub raw_kind: String,
    pub step: Option<String>,
    pub repo: Option<String>,
    pub detail: Option<String>,
}

/// The most recent run, and how long each of its problem kinds has lasted.
#[derive(Debug, Clone, PartialEq)]
pub struct LastRun {
    pub run: DateTime<Utc>,
    /// `None` where the run wrote no `finished` row (a newer or older build).
    pub selected: Option<i64>,
    pub collected: Option<i64>,
    /// What the summary pass did, from the `finished` row. `None` when the
    /// pass did not run at all — switched off (`--no-summaries`), or a build
    /// from before it — which is not a pass that summarised nothing.
    pub summaries: Option<summary::Tally>,
    pub problems: Vec<Logged>,
    /// For each kind in this run: the earliest run of the unbroken streak of
    /// runs that all had it, and how many runs that is.
    pub since: BTreeMap<String, (DateTime<Utc>, usize)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogReport {
    pub last: Option<LastRun>,
    /// Repositories recorded vanished within the window, with when.
    pub vanished: Vec<(String, DateTime<Utc>)>,
}

/// Read the collector's log without creating or changing anything.
/// `Ok(LogReport::default())`: the store holds no run yet.
pub fn read_log(db: &Path, vanished_within: Duration, now: DateTime<Utc>) -> Result<LogReport> {
    let conn = Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let latest: Option<String> =
        conn.query_row("SELECT max(run) FROM collect_log", [], |r| r.get(0))?;
    let Some(latest) = latest else {
        return Ok(LogReport::default());
    };
    let run = parse_ts(&latest)
        .with_context(|| format!("collect_log holds an unreadable run time {latest:?}"))?;
    let finished: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT selected, collected FROM collect_log WHERE run = ?1 AND kind = 'finished' ORDER BY id DESC LIMIT 1",
            params![latest],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    // A store the summarising collector has not written yet has no such
    // columns: no tally, never an unreadable store.
    let summaries = if has_column(&conn, "collect_log", "summarised")? {
        conn.query_row(
            "SELECT summarised, summary_refused, summary_failed, summary_deferred FROM collect_log
             WHERE run = ?1 AND kind = 'finished' ORDER BY id DESC LIMIT 1",
            params![latest],
            |r| {
                let n = |i: usize| -> rusqlite::Result<usize> {
                    Ok(r.get::<_, Option<i64>>(i)?.unwrap_or(0).max(0) as usize)
                };
                Ok(match r.get::<_, Option<i64>>(0)? {
                    None => None,
                    Some(_) => Some(summary::Tally {
                        summarised: n(0)?,
                        refused: n(1)?,
                        failed: n(2)?,
                        deferred: n(3)?,
                    }),
                })
            },
        )
        .optional()?
        .flatten()
    } else {
        None
    };
    let mut stmt = conn.prepare(
        "SELECT kind, step, repo, detail FROM collect_log WHERE run = ?1 AND kind <> 'finished' ORDER BY id",
    )?;
    let problems: Vec<Logged> = stmt
        .query_map(params![latest], |r| {
            let raw: String = r.get(0)?;
            Ok(Logged {
                kind: Kind::parse(&raw),
                raw_kind: raw,
                step: r.get(1)?,
                repo: r.get(2)?,
                detail: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    // Each run's kinds, newest first, for the streaks.
    let mut stmt = conn.prepare("SELECT run, kind FROM collect_log ORDER BY run DESC")?;
    let mut runs: Vec<(String, BTreeSet<String>)> = Vec::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (r, k) = row?;
        match runs.last_mut() {
            Some((last, kinds)) if *last == r => {
                kinds.insert(k);
            }
            _ => runs.push((r, BTreeSet::from([k]))),
        }
    }
    let mut since = BTreeMap::new();
    for p in &problems {
        if since.contains_key(&p.raw_kind) {
            continue;
        }
        let streak: Vec<&String> = runs
            .iter()
            .take_while(|(_, kinds)| kinds.contains(&p.raw_kind))
            .map(|(r, _)| r)
            .collect();
        if let Some(first) = streak.last().and_then(|r| parse_ts(r)) {
            since.insert(p.raw_kind.clone(), (first, streak.len()));
        }
    }

    let mut stmt = conn.prepare(
        "SELECT repo, max(run) FROM collect_log WHERE kind = 'vanished' AND repo IS NOT NULL AND run >= ?1 GROUP BY repo ORDER BY repo",
    )?;
    let vanished = stmt
        .query_map(params![ts(now - vanished_within)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .filter_map(Result::ok)
        .filter_map(|(repo, at)| Some((repo, parse_ts(&at)?)))
        .collect();

    Ok(LogReport {
        last: Some(LastRun {
            run,
            selected: finished.and_then(|f| f.0),
            collected: finished.and_then(|f| f.1),
            summaries,
            problems,
            since,
        }),
        vanished,
    })
}

pub mod summary;

#[cfg(test)]
mod tests;
