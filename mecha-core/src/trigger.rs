//! Triggers: prompts that run on a schedule, unattended.
//!
//! This is what turns the harness into an assistant rather than a REPL —
//! nobody types "check my inbox" every morning. A trigger is a prompt, a cron
//! schedule, and the policy an unattended run needs; everything that makes such
//! a run safe already existed (the outbox stages what would be sent, the
//! interlock refuses exfiltration, the sandbox confines `shell`, budgets bound
//! the spend, and the session recording feeds `reflect`). This module only adds
//! the clock and the ledger.
//!
//! Four decisions carry the design:
//!
//! **Triggers live in the user's own store, never in the layered config.**
//! `[[hook]]`, `[[mcp]]` and `[[subagent]]` are all declarable in a project's
//! `mecha.toml`, which is a file that arrives with a cloned repository. A
//! trigger is a *scheduled unattended agent run*, and a repository that can
//! contribute one has been handed a cron slot on your machine. So they are
//! files under `~/.mecha/triggers/`, one per trigger, and a trigger run reads
//! the global config only — [`crate::config::Config::load_global`] exists for
//! exactly this.
//!
//! **The schedule is answered backwards.** "Is this due?" asks for the most
//! recent slot at or before now ([`crate::cron::Schedule::prev_at_or_before`])
//! and compares it against the last slot that fired. A laptop closed for a week
//! therefore wakes up owing *one* briefing, not forty, and a tick that arrives
//! late has lost nothing — which is what lets the scheduler be a dumb
//! once-a-minute loop with no state of its own.
//!
//! **A manual run is evidence, not a fire.** `mecha trigger run briefing` at
//! noon records a row with no slot, so it never advances the marker and never
//! cancels tomorrow morning's. Testing a trigger must not silently disarm it.
//!
//! **Read-only unless the file says otherwise.** Nobody is watching to approve
//! anything, and `PermissionMode::Ask` in that situation means "deny with a
//! message telling you to pass `--yes`", which is useless advice at 03:00. A
//! trigger states its permission mode; the default is the narrow one, and
//! widening it is a line someone wrote down. Note what read-only does *not*
//! block: an outbox-routed call still stages, because staging executes nothing.
//! Draft-my-replies-overnight is the safe default shape, and it needs no
//! privilege at all.
//!
//! Storage follows the outbox's rules — one file per trigger so `$EDITOR` and
//! `git diff` work on it, temp-sibling-and-rename for every write, an advisory
//! flock for read-modify-write, and an append-only JSONL ledger of every fire.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::agent::Taint;
use crate::config::PermissionMode;
use crate::cron::Schedule;

/// How long after a missed slot it is still worth running.
///
/// Both extremes are legitimate and neither is a safe default for the other: a
/// nightly rumination wants to catch up whenever the machine comes back, and a
/// 07:00 briefing delivered at 23:00 is noise. One knob, three behaviours.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CatchUp {
    /// Run the missed slot whenever it is noticed. systemd's `Persistent=true`.
    #[default]
    Always,
    /// Only run a slot that is still fresh. A missed one is recorded as skipped
    /// and the schedule moves on.
    Never,
    /// Run a missed slot if it is younger than this.
    Within(chrono::Duration),
}

/// How late a `Never` trigger may still fire: the scheduler ticks on the
/// minute, so a slot is always a few tens of seconds old by the time anything
/// looks at it. Without this, `catch_up = "never"` would mean "never run".
const TICK_GRACE: chrono::Duration = chrono::Duration::minutes(2);

impl std::fmt::Display for CatchUp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatchUp::Always => f.write_str("always"),
            CatchUp::Never => f.write_str("never"),
            CatchUp::Within(d) => write!(f, "{}", render_duration(*d)),
        }
    }
}

impl std::str::FromStr for CatchUp {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "always" | "true" => Ok(CatchUp::Always),
            "never" | "false" => Ok(CatchUp::Never),
            other => Ok(CatchUp::Within(parse_duration(other).with_context(
                || format!("catch_up `{s}` is not `always`, `never`, or a duration like `2h`"),
            )?)),
        }
    }
}

impl Serialize for CatchUp {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CatchUp {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// `90s`, `30m`, `2h`, `1d`. A bare number is seconds.
pub fn parse_duration(text: &str) -> Result<chrono::Duration> {
    let text = text.trim();
    anyhow::ensure!(!text.is_empty(), "is empty");
    let (digits, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let n: i64 = digits
        .parse()
        .map_err(|_| anyhow::anyhow!("`{text}` does not start with a number"))?;
    let d = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "s" | "sec" | "secs" | "seconds" => chrono::Duration::seconds(n),
        "m" | "min" | "mins" | "minutes" => chrono::Duration::minutes(n),
        "h" | "hr" | "hrs" | "hours" => chrono::Duration::hours(n),
        "d" | "day" | "days" => chrono::Duration::days(n),
        other => anyhow::bail!("unknown unit `{other}` (use s, m, h, or d)"),
    };
    anyhow::ensure!(d > chrono::Duration::zero(), "must be positive");
    Ok(d)
}

pub fn render_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds();
    if secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs % 3_600 == 0 {
        format!("{}h", secs / 3_600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn default_true() -> bool {
    true
}

fn default_permission() -> PermissionMode {
    PermissionMode::ReadOnly
}

/// One scheduled prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    /// The file's stem. Never read from the file itself — a name that can
    /// disagree with its filename is a class of bug with no upside.
    #[serde(skip)]
    pub name: String,

    /// Five-field cron, in `timezone`.
    pub schedule: Schedule,

    /// What to ask. This is the whole action: a trigger runs an agent, not a
    /// command. (Scheduled *commands* are what cron is for, and giving one a
    /// place in this store would mean answering how it gets confined and which
    /// environment it sees — questions the MCP and sandbox work already
    /// answered the expensive way.)
    pub prompt: String,

    /// One line for `mecha trigger list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// IANA name. Written explicitly by `mecha trigger add`, resolved from
    /// `[agent] timezone` at the time: "07:00" must not quietly mean something
    /// different after a config edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,

    #[serde(default = "default_true")]
    pub enabled: bool,

    /// The anchor for the first fire. Without it a trigger added at 08:00 would
    /// find 07:00 unfired and run immediately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The path jail for this run. Defaults to the daemon's working directory,
    /// which is usually not what you want — say it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,

    /// Read-only by default. See the module docs: an unattended run has nobody
    /// to ask, and outbox staging works at every level.
    #[serde(default = "default_permission")]
    pub permission_mode: PermissionMode,

    /// Only these tools, if set. The narrowest useful control there is: a
    /// briefing that can read mail and nothing else cannot be talked into
    /// anything else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,

    /// Skip MCP servers entirely for this run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_mcp: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Needs prices on the provider. A cap that cannot fire is refused at load
    /// rather than ignored at 03:00 — see [`Trigger::validate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,

    /// Wall-clock ceiling on one run. Cancels at the next safe point, keeping
    /// the partial answer, exactly as Ctrl-C does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,

    #[serde(default, skip_serializing_if = "is_default_catch_up")]
    pub catch_up: CatchUp,

    /// A command run when the trigger produces an answer, with the answer on
    /// stdin — `notify-send`, a `mail` invocation, an append to a file.
    ///
    /// An observer, like `post_tool`: its failure is logged and never fails the
    /// run. The answer is already in the session transcript, which is the
    /// record; this is delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<String>,
}

fn is_default_catch_up(c: &CatchUp) -> bool {
    *c == CatchUp::Always
}

/// How long a run may take before it is cancelled, when the trigger does not
/// say. Long enough for a real briefing over a local model, short enough that a
/// wedged run does not hold the scheduler until someone notices.
pub const DEFAULT_TIMEOUT: chrono::Duration = chrono::Duration::minutes(20);

impl Trigger {
    pub fn new(name: impl Into<String>, schedule: Schedule, prompt: impl Into<String>) -> Self {
        Trigger {
            name: name.into(),
            schedule,
            prompt: prompt.into(),
            description: None,
            timezone: None,
            enabled: true,
            created_at: Some(Utc::now()),
            provider: None,
            model: None,
            workspace: None,
            permission_mode: default_permission(),
            tools: Vec::new(),
            no_mcp: false,
            max_turns: None,
            max_output_tokens: None,
            max_cost_usd: None,
            timeout: None,
            catch_up: CatchUp::default(),
            notify: None,
        }
    }

    /// A trigger name is a filename, a log line, and a CLI argument. Keep it to
    /// what is unambiguous in all three.
    pub fn valid_name(name: &str) -> Result<()> {
        anyhow::ensure!(!name.is_empty(), "a trigger needs a name");
        anyhow::ensure!(
            name.len() <= 64,
            "trigger name `{name}` is too long (64 characters max)"
        );
        anyhow::ensure!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
            "trigger name `{name}` may only contain lowercase letters, digits, `-` and `_`"
        );
        Ok(())
    }

    /// Everything that can be wrong with a trigger before it ever runs.
    ///
    /// Called on load, so a typo surfaces on `mecha trigger list` at a
    /// keyboard, not on the fire it was meant to control.
    pub fn validate(&self) -> Result<()> {
        Self::valid_name(&self.name)?;
        anyhow::ensure!(
            !self.prompt.trim().is_empty(),
            "trigger `{}` has an empty prompt",
            self.name
        );
        if let Some(tz) = &self.timezone {
            tz.parse::<Tz>()
                .map_err(|_| anyhow::anyhow!("trigger `{}`: unknown timezone `{tz}`", self.name))?;
        }
        if let Some(t) = &self.timeout {
            parse_duration(t).with_context(|| format!("trigger `{}`: bad timeout", self.name))?;
        }
        Ok(())
    }

    /// The zone its wall-clock schedule is read in.
    pub fn tz(&self, fallback: Option<Tz>) -> Tz {
        self.timezone
            .as_deref()
            .and_then(|n| n.parse().ok())
            .or(fallback)
            .unwrap_or(chrono_tz::UTC)
    }

    pub fn timeout_duration(&self) -> chrono::Duration {
        self.timeout
            .as_deref()
            .and_then(|t| parse_duration(t).ok())
            .unwrap_or(DEFAULT_TIMEOUT)
    }

    /// When this trigger would next fire after `at`.
    pub fn next_fire(&self, at: DateTime<Utc>, fallback_tz: Option<Tz>) -> Option<DateTime<Utc>> {
        self.schedule.next_after(at, self.tz(fallback_tz))
    }

    /// Is it due, and if not, when?
    ///
    /// `last_slot` is the most recent slot that has already been accounted for
    /// — fired, or deliberately skipped. `None` means nothing has, in which
    /// case `created_at` is the anchor: a trigger must not fire for a slot that
    /// predates its own existence.
    pub fn due(
        &self,
        last_slot: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        fallback_tz: Option<Tz>,
    ) -> Due {
        if !self.enabled {
            return Due::Disabled;
        }
        let tz = self.tz(fallback_tz);
        let Some(slot) = self.schedule.prev_at_or_before(now, tz) else {
            return Due::Not {
                next: self.schedule.next_after(now, tz),
            };
        };
        let anchor = last_slot.or(self.created_at);
        if anchor.is_some_and(|a| slot <= a) {
            return Due::Not {
                next: self.schedule.next_after(now, tz),
            };
        }

        let age = now - slot;
        let fresh = match self.catch_up {
            CatchUp::Always => true,
            CatchUp::Never => age <= TICK_GRACE,
            // The grace applies here too, or `catch_up = "1m"` would be
            // unfireable for the same reason `never` would be.
            CatchUp::Within(d) => age <= d.max(TICK_GRACE),
        };
        if fresh {
            Due::Now { slot }
        } else {
            Due::Stale { slot, age }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Due {
    /// Fire, for this slot.
    Now {
        slot: DateTime<Utc>,
    },
    /// A slot was missed and is past its catch-up window. Recorded as skipped
    /// — evidence, not silence — and the marker advances past it.
    Stale {
        slot: DateTime<Utc>,
        age: chrono::Duration,
    },
    Not {
        next: Option<DateTime<Utc>>,
    },
    Disabled,
}

/// What happened on one fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    Ok,
    /// The run failed — a provider error, a timeout, a refused sandbox.
    Error,
    /// The previous run of this trigger was still going. Never stack: a
    /// five-minute trigger whose run takes six minutes must not become an
    /// unbounded fan-out.
    SkippedOverlap,
    /// The slot was missed by more than its catch-up window.
    SkippedStale,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Ok => "ok",
            RunStatus::Error => "error",
            RunStatus::SkippedOverlap => "skipped (overlap)",
            RunStatus::SkippedStale => "skipped (stale)",
        }
    }
}

/// One line of the ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub trigger: String,
    /// The scheduled slot this accounts for. `None` for a manual run — which
    /// is why a manual run never advances the schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    /// The transcript, which is where the full answer lives. The ledger keeps
    /// a one-line summary and points here rather than storing a second copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub blocked_sends: u32,
    /// Calls the outbox staged — the number to look at in the morning.
    #[serde(default)]
    pub staged: u32,
    #[serde(default)]
    pub taint: Taint,
    /// Why the loop stopped, when it was not the model deciding it was done —
    /// a timeout, a budget, a shutdown. Without it a run cut short records as
    /// plain `ok` and a trigger that has been quietly truncating its answer
    /// every morning looks exactly like one that works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_cause: Option<crate::agent::StopCause>,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `mecha trigger run <name>`, not the clock.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

impl RunRecord {
    pub fn started(trigger: &str, slot: Option<DateTime<Utc>>, manual: bool) -> Self {
        RunRecord {
            trigger: trigger.to_string(),
            slot,
            started_at: Utc::now(),
            finished_at: None,
            status: RunStatus::Ok,
            session_id: None,
            turns: 0,
            cost_usd: None,
            blocked_sends: 0,
            staged: 0,
            taint: Taint::default(),
            stop_cause: None,
            summary: String::new(),
            error: None,
            manual,
        }
    }
}

pub struct TriggerStore {
    root: PathBuf,
}

/// Holds the store's writer lock for as long as it lives.
pub struct StoreLock {
    _file: std::fs::File,
}

/// Holds one trigger's run lock — proof that no other run of it is in flight.
pub struct RunLock {
    _file: std::fs::File,
}

impl TriggerStore {
    pub fn default_root() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_TRIGGERS_DIR") {
            return Ok(PathBuf::from(dir));
        }
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".mecha").join("triggers"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(TriggerStore { root })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_root()?)
    }

    /// Open only if it already exists — for read paths that must not create
    /// state as a side effect.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(TriggerStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_of(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.toml"))
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.root.join("runs.jsonl")
    }

    /// Every trigger, by name.
    ///
    /// An unreadable or invalid file is reported and skipped rather than
    /// failing the whole listing: one bad trigger must not stop the other
    /// three from firing.
    pub fn list(&self) -> Result<(Vec<Trigger>, Vec<String>)> {
        let mut out = Vec::new();
        let mut problems = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, problems)),
            Err(e) => return Err(e).context("reading the trigger store"),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            match self.load_path(&path, &name) {
                Ok(t) => out.push(t),
                Err(e) => problems.push(format!("{}: {e:#}", path.display())),
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((out, problems))
    }

    fn load_path(&self, path: &Path, name: &str) -> Result<Trigger> {
        let text = std::fs::read_to_string(path)?;
        let mut trigger: Trigger = toml::from_str(&text)?;
        trigger.name = name.to_string();
        // A hand-written file has no `created_at`; anchor it to the file itself
        // so it does not fire for every slot since the epoch.
        if trigger.created_at.is_none() {
            trigger.created_at = std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from);
        }
        trigger.validate()?;
        Ok(trigger)
    }

    pub fn get(&self, name: &str) -> Result<Trigger> {
        let path = self.path_of(name);
        anyhow::ensure!(path.exists(), "no trigger named `{name}`");
        self.load_path(&path, name)
    }

    pub fn exists(&self, name: &str) -> bool {
        self.path_of(name).exists()
    }

    pub fn save(&self, trigger: &Trigger) -> Result<()> {
        trigger.validate()?;
        let path = self.path_of(&trigger.name);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(trigger)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let path = self.path_of(name);
        anyhow::ensure!(path.exists(), "no trigger named `{name}`");
        std::fs::remove_file(&path)?;
        Ok(())
    }

    /// Append one row. Held under the store lock so two ticks cannot interleave
    /// a line.
    pub fn append_run(&self, record: &RunRecord) -> Result<()> {
        use std::io::Write;
        let _lock = self.lock()?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.ledger_path())?;
        writeln!(file, "{}", serde_json::to_string(record)?)?;
        Ok(())
    }

    /// The ledger, oldest first. A torn or unparseable line is skipped: this is
    /// an audit trail, and one bad line must not hide the rest.
    pub fn runs(&self) -> Result<Vec<RunRecord>> {
        let path = self.ledger_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)?;
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str::<RunRecord>(l) {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::warn!("skipping unreadable ledger row: {e}");
                    None
                }
            })
            .collect())
    }

    /// The most recent accounted-for slot per trigger — one ledger scan for the
    /// whole tick. Manual runs carry no slot and so are invisible here, which
    /// is the point.
    pub fn last_slots(&self) -> Result<BTreeMap<String, DateTime<Utc>>> {
        let mut out: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
        for run in self.runs()? {
            if let Some(slot) = run.slot {
                out.entry(run.trigger)
                    .and_modify(|s| {
                        if slot > *s {
                            *s = slot
                        }
                    })
                    .or_insert(slot);
            }
        }
        Ok(out)
    }

    /// Writer lock for the ledger.
    pub fn lock(&self) -> Result<StoreLock> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the trigger store");
        }
        Ok(StoreLock { _file: file })
    }

    /// Claim the right to run `name`, or `None` if a run of it is already in
    /// flight — in this process or any other.
    ///
    /// Non-blocking on purpose: the answer "someone else is running it" is the
    /// useful one, and waiting for a twenty-minute briefing to finish so a
    /// second copy can start is never what anybody wanted.
    pub fn try_claim(&self, name: &str) -> Result<Option<RunLock>> {
        use std::os::unix::io::AsRawFd;
        let dir = self.root.join("locks");
        crate::create_private_dir(&dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{name}.lock")))?;
        // SAFETY: flock on an fd we own, held open by the returned guard. The
        // lock is released by the kernel if the process dies, so a crashed run
        // does not wedge a trigger forever.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(err).context("claiming the trigger run lock");
        }
        Ok(Some(RunLock { _file: file }))
    }

    fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    fn marker_path(&self, name: &str) -> PathBuf {
        self.locks_dir().join(format!("{name}.running"))
    }

    fn cancel_path(&self, name: &str) -> PathBuf {
        self.locks_dir().join(format!("{name}.cancel"))
    }

    /// Announce that a run has started, for anything that wants to *display*
    /// whether one is in flight.
    ///
    /// Deliberately not the flock. The obvious way to ask "is it running?" is
    /// to try to claim it and see — but `try_claim` acquires the lock and then
    /// drops it, so a UI polling that question would occasionally hold the
    /// lock at the instant the scheduler tried to fire, and the scheduler
    /// would record a spurious overlap skip. Watching must never perturb what
    /// is watched. The flock stays the real mutual exclusion (the kernel frees
    /// it if the process dies); this is advisory state beside it.
    pub fn mark_running(&self, name: &str, slot: Option<DateTime<Utc>>) -> Result<()> {
        crate::create_private_dir(&self.locks_dir())?;
        let marker = RunMarker {
            pid: std::process::id(),
            started_at: Utc::now(),
            slot,
        };
        let path = self.marker_path(name);
        let tmp = path.with_extension("running.tmp");
        std::fs::write(&tmp, serde_json::to_string(&marker)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Clear the marker and any unclaimed cancel request. Both, because a
    /// cancel that arrives as a run is ending must not be left lying around to
    /// kill the *next* one.
    pub fn clear_running(&self, name: &str) {
        let _ = std::fs::remove_file(self.marker_path(name));
        let _ = std::fs::remove_file(self.cancel_path(name));
    }

    /// The run in flight, if there is one.
    ///
    /// A marker whose process is gone is a crashed run, not a running one — it
    /// is cleaned up and reported as absent, so a hard kill cannot leave a
    /// trigger looking permanently busy in every UI that asks.
    pub fn running(&self, name: &str) -> Option<RunMarker> {
        let text = std::fs::read_to_string(self.marker_path(name)).ok()?;
        let marker: RunMarker = serde_json::from_str(&text).ok()?;
        if process_alive(marker.pid) {
            Some(marker)
        } else {
            self.clear_running(name);
            None
        }
    }

    /// Ask the run in flight to stop. Returns false when there is nothing to
    /// stop, so a caller can say so rather than pretending.
    ///
    /// A file rather than a signal, because the run may belong to the daemon's
    /// process and SIGTERM there would take the whole scheduler down with it.
    /// The runner polls for this and cancels its own token, which stops the run
    /// at the next safe point with its partial answer and ledger row intact —
    /// the same path as Ctrl-C and the timeout.
    pub fn request_cancel(&self, name: &str) -> Result<bool> {
        if self.running(name).is_none() {
            return Ok(false);
        }
        crate::create_private_dir(&self.locks_dir())?;
        std::fs::write(self.cancel_path(name), Utc::now().to_rfc3339())?;
        Ok(true)
    }

    /// Has a cancel been requested for the run in flight?
    pub fn cancel_requested(&self, name: &str) -> bool {
        self.cancel_path(name).exists()
    }
}

/// Who is running a trigger right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMarker {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<DateTime<Utc>>,
}

/// Is this pid still around? `kill(pid, 0)` checks without delivering
/// anything; `EPERM` means it exists and is not ours, which still counts.
///
/// The range check is not defensive padding — it is the whole correctness of
/// the function. `kill(2)` gives non-positive pids entirely different
/// meanings: `0` is "every process in my group", `-1` is "every process I may
/// signal" (which succeeds, always), and any other negative is a process
/// group. A corrupt marker holding one of those would report a long-dead run
/// as alive and leave the trigger looking permanently busy in every UI that
/// asks. Found by a test using `u32::MAX`, which sign-flips to exactly the
/// `-1` case.
fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 delivers nothing and only probes for the process.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-trigger-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn daily_7am(name: &str) -> Trigger {
        let mut t = Trigger::new(name, "0 7 * * *".parse().unwrap(), "brief me");
        t.timezone = Some("America/New_York".into());
        t.created_at = Some(utc("2026-08-01T00:00:00Z"));
        t
    }

    #[test]
    fn a_trigger_round_trips_through_its_file_and_takes_its_name_from_it() {
        let root = scratch("roundtrip");
        let store = TriggerStore::open(&root).unwrap();

        let mut t = daily_7am("morning-briefing");
        t.description = Some("inbox and calendar".into());
        t.max_turns = Some(20);
        t.catch_up = CatchUp::Within(chrono::Duration::hours(2));
        store.save(&t).unwrap();

        let loaded = store.get("morning-briefing").unwrap();
        assert_eq!(loaded.name, "morning-briefing");
        assert_eq!(loaded.schedule.source(), "0 7 * * *");
        assert_eq!(loaded.max_turns, Some(20));
        assert_eq!(loaded.catch_up, CatchUp::Within(chrono::Duration::hours(2)));
        // The narrow default, not the config's mode.
        assert_eq!(loaded.permission_mode, PermissionMode::ReadOnly);

        // Renaming the file renames the trigger; nothing inside can disagree.
        std::fs::rename(store.path_of("morning-briefing"), store.path_of("evening")).unwrap();
        assert_eq!(store.get("evening").unwrap().name, "evening");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn one_broken_trigger_does_not_hide_the_others() {
        let root = scratch("broken");
        let store = TriggerStore::open(&root).unwrap();
        store.save(&daily_7am("good")).unwrap();
        std::fs::write(
            store.path_of("bad"),
            "schedule = \"nonsense\"\nprompt = \"x\"\n",
        )
        .unwrap();

        let (list, problems) = store.list().unwrap();
        assert_eq!(list.len(), 1, "the good one still fires");
        assert_eq!(list[0].name, "good");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("bad.toml"), "{:?}", problems);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The property the scheduler rests on: however long the gap, one run.
    #[test]
    fn a_week_of_missed_slots_owes_exactly_one_run() {
        let t = daily_7am("briefing");
        let last = utc("2026-08-03T11:00:00Z"); // 07:00 EDT on the 3rd
        let now = utc("2026-08-10T12:30:00Z"); // a week later, 08:30 EDT

        let Due::Now { slot } = t.due(Some(last), now, None) else {
            panic!("a missed slot must fire");
        };
        assert_eq!(
            slot,
            utc("2026-08-10T11:00:00Z"),
            "today's slot, not the 4th's"
        );

        // Once that slot is recorded, it is not due again...
        assert!(matches!(t.due(Some(slot), now, None), Due::Not { .. }));
        // ...and the next fire is tomorrow.
        let Due::Not { next: Some(next) } = t.due(Some(slot), now, None) else {
            panic!("should report the next fire")
        };
        assert_eq!(next, utc("2026-08-11T11:00:00Z"));
    }

    #[test]
    fn a_trigger_never_fires_for_a_slot_older_than_itself() {
        let mut t = daily_7am("briefing");
        // Created at 08:00 EDT, after today's 07:00 slot.
        t.created_at = Some(utc("2026-08-05T12:00:00Z"));
        let now = utc("2026-08-05T12:30:00Z");

        let Due::Not { next: Some(next) } = t.due(None, now, None) else {
            panic!("this morning's briefing already happened without it");
        };
        assert_eq!(next, utc("2026-08-06T11:00:00Z"));
    }

    #[test]
    fn catch_up_decides_whether_a_stale_slot_still_runs() {
        let now = utc("2026-08-05T23:30:00Z"); // 19:30 EDT, twelve hours late

        let always = daily_7am("a");
        assert!(matches!(always.due(None, now, None), Due::Now { .. }));

        let mut never = daily_7am("b");
        never.catch_up = CatchUp::Never;
        let Due::Stale { age, .. } = never.due(None, now, None) else {
            panic!("`never` must not run a twelve-hour-old briefing")
        };
        assert!(age > chrono::Duration::hours(11));

        let mut within = daily_7am("c");
        within.catch_up = CatchUp::Within(chrono::Duration::hours(2));
        assert!(matches!(within.due(None, now, None), Due::Stale { .. }));

        // And on time, every policy fires — the tick grace covers the seconds
        // between the slot and the scheduler noticing.
        let on_time = utc("2026-08-05T11:00:30Z");
        assert!(matches!(never.due(None, on_time, None), Due::Now { .. }));
        assert!(matches!(within.due(None, on_time, None), Due::Now { .. }));
    }

    #[test]
    fn a_disabled_trigger_is_never_due() {
        let mut t = daily_7am("briefing");
        t.enabled = false;
        assert_eq!(
            t.due(None, utc("2026-08-05T11:00:30Z"), None),
            Due::Disabled
        );
    }

    /// Testing a trigger by hand must not disarm the schedule.
    #[test]
    fn a_manual_run_does_not_advance_the_schedule() {
        let root = scratch("manual");
        let store = TriggerStore::open(&root).unwrap();
        let t = daily_7am("briefing");
        store.save(&t).unwrap();

        let mut manual = RunRecord::started("briefing", None, true);
        manual.status = RunStatus::Ok;
        store.append_run(&manual).unwrap();

        assert!(!store.last_slots().unwrap().contains_key("briefing"));
        // So the scheduled slot is still owed.
        let now = utc("2026-08-05T11:00:30Z");
        let last = store.last_slots().unwrap().get("briefing").copied();
        assert!(matches!(t.due(last, now, None), Due::Now { .. }));

        // A scheduled run does advance it.
        let mut fired = RunRecord::started("briefing", Some(utc("2026-08-05T11:00:00Z")), false);
        fired.status = RunStatus::Ok;
        store.append_run(&fired).unwrap();
        let last = store.last_slots().unwrap().get("briefing").copied();
        assert!(matches!(t.due(last, now, None), Due::Not { .. }));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A skipped slot is recorded, so it is accounted for and not retried every
    /// minute for the rest of the day.
    #[test]
    fn a_stale_skip_is_written_down_and_moves_the_marker() {
        let root = scratch("stale");
        let store = TriggerStore::open(&root).unwrap();
        let mut t = daily_7am("briefing");
        t.catch_up = CatchUp::Never;
        store.save(&t).unwrap();

        let now = utc("2026-08-05T23:30:00Z");
        let Due::Stale { slot, .. } = t.due(None, now, None) else {
            panic!()
        };
        let mut rec = RunRecord::started("briefing", Some(slot), false);
        rec.status = RunStatus::SkippedStale;
        store.append_run(&rec).unwrap();

        let last = store.last_slots().unwrap().get("briefing").copied();
        assert!(
            matches!(t.due(last, now, None), Due::Not { .. }),
            "not reconsidered"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_run_in_flight_cannot_be_started_twice() {
        let root = scratch("claim");
        let store = TriggerStore::open(&root).unwrap();
        let held = store.try_claim("briefing").unwrap();
        assert!(held.is_some(), "the first claim wins");
        assert!(
            store.try_claim("briefing").unwrap().is_none(),
            "a five-minute trigger whose run takes six must not stack"
        );
        assert!(
            store.try_claim("other").unwrap().is_some(),
            "and it is per trigger"
        );

        drop(held);
        assert!(
            store.try_claim("briefing").unwrap().is_some(),
            "released when the run ends"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Watching must not perturb what is watched: asking "is it running?" via
    /// `try_claim` would hold the lock for an instant, and a scheduler firing
    /// in that instant would record a spurious overlap skip. The marker exists
    /// so the question can be asked without touching the lock.
    #[test]
    fn asking_whether_a_run_is_in_flight_does_not_disturb_the_lock() {
        let root = scratch("running");
        let store = TriggerStore::open(&root).unwrap();

        assert!(store.running("briefing").is_none(), "nothing running yet");
        store
            .mark_running("briefing", Some(utc("2026-08-05T11:00:00Z")))
            .unwrap();

        let marker = store.running("briefing").expect("should report the run");
        assert_eq!(marker.pid, std::process::id());
        assert_eq!(marker.slot, Some(utc("2026-08-05T11:00:00Z")));

        // The claim is still available: the marker is advisory, not the lock.
        assert!(
            store.try_claim("briefing").unwrap().is_some(),
            "the marker must not be a second, weaker lock"
        );

        store.clear_running("briefing");
        assert!(store.running("briefing").is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A hard kill must not leave a trigger looking busy forever.
    #[test]
    fn a_marker_from_a_dead_process_reads_as_not_running() {
        let root = scratch("stale-marker");
        let store = TriggerStore::open(&root).unwrap();
        store.mark_running("briefing", None).unwrap();

        let path = store.root().join("locks").join("briefing.running");
        let rewrite = |pid: u32| {
            let mut marker: RunMarker =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            marker.pid = pid;
            std::fs::write(&path, serde_json::to_string(&marker).unwrap()).unwrap();
        };

        // A real-looking pid that no longer exists: far above any pid_max.
        rewrite(i32::MAX as u32);
        assert!(
            store.running("briefing").is_none(),
            "a dead pid is not a running trigger"
        );
        assert!(!path.exists(), "and the stale marker is cleaned up");

        // And the one that found the bug: `u32::MAX` sign-flips to -1, which
        // `kill(2)` reads as "every process I may signal" and answers yes to.
        store.mark_running("briefing", None).unwrap();
        rewrite(u32::MAX);
        assert!(
            store.running("briefing").is_none(),
            "a pid that is not a pid must never read as a live run"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cancel_can_only_be_requested_against_a_run_that_exists() {
        let root = scratch("cancel");
        let store = TriggerStore::open(&root).unwrap();

        assert!(
            !store.request_cancel("briefing").unwrap(),
            "nothing to cancel"
        );
        assert!(!store.cancel_requested("briefing"));

        store.mark_running("briefing", None).unwrap();
        assert!(store.request_cancel("briefing").unwrap());
        assert!(store.cancel_requested("briefing"));

        // Ending the run clears the request too — a cancel that lands as a run
        // finishes must not kill the *next* one.
        store.clear_running("briefing");
        assert!(!store.cancel_requested("briefing"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn names_are_checked_because_they_are_filenames() {
        assert!(Trigger::valid_name("morning-briefing").is_ok());
        assert!(Trigger::valid_name("inbox_triage2").is_ok());
        assert!(Trigger::valid_name("../../etc/cron").is_err());
        assert!(Trigger::valid_name("Briefing").is_err());
        assert!(Trigger::valid_name("").is_err());
    }

    #[test]
    fn durations_parse_the_way_people_write_them() {
        assert_eq!(
            parse_duration("90s").unwrap(),
            chrono::Duration::seconds(90)
        );
        assert_eq!(
            parse_duration("30m").unwrap(),
            chrono::Duration::minutes(30)
        );
        assert_eq!(parse_duration("2h").unwrap(), chrono::Duration::hours(2));
        assert_eq!(parse_duration("1d").unwrap(), chrono::Duration::days(1));
        assert_eq!(parse_duration("45").unwrap(), chrono::Duration::seconds(45));
        assert!(parse_duration("0m").is_err());
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("2 fortnights").is_err());

        // Round-trips through the file, which is what `catch_up` needs.
        assert_eq!(render_duration(chrono::Duration::hours(2)), "2h");
        assert_eq!(render_duration(chrono::Duration::minutes(90)), "90m");
        assert_eq!("2h".parse::<CatchUp>().unwrap().to_string(), "2h");
        assert_eq!("never".parse::<CatchUp>().unwrap(), CatchUp::Never);
        assert!("sometimes".parse::<CatchUp>().is_err());
    }

    #[test]
    fn an_invalid_trigger_fails_at_the_keyboard_not_at_three_in_the_morning() {
        let root = scratch("validate");
        let store = TriggerStore::open(&root).unwrap();

        let mut t = daily_7am("briefing");
        t.timezone = Some("Mars/Olympus".into());
        assert!(store.save(&t).is_err(), "an unknown zone is caught on save");

        let mut t = daily_7am("briefing");
        t.timeout = Some("soon".into());
        assert!(store.save(&t).is_err());

        let mut t = daily_7am("briefing");
        t.prompt = "   ".into();
        assert!(store.save(&t).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }
}
