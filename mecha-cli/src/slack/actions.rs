//! The typed layer between a Slack tap and a command line.
//!
//! The sentence `docs/SLACK-ACTIONS-DESIGN.md` carries through: **a tappable
//! argv is authored by deterministic code from typed store state, and the tap
//! is a gated human; no model output and no message text is ever between a
//! finding and a command line.** The enforcement is this module's shape rather
//! than anyone's discipline — the same move as `Decision::Allow | Deny |
//! Blocked`: the split lives in the type, so no wording a caller chooses can
//! escape it.
//!
//! Three functions carry the invariant:
//!
//! - [`Action::argv`] is a total match whose verbs are literals. There is no
//!   `Action::Raw` and never will be; the executor takes an `Action`, not an
//!   argv, so nothing can hand it a command line it didn't derive.
//! - [`Action::from_remedy`] recognises exactly the argv *shapes* this surface
//!   trusts a phone with. Anything unrecognised — including every
//!   `needs_terminal` remedy — degrades to display, never to execution.
//! - [`Action::from_payload`] parses the button press coming back: a fixed
//!   verb from a closed set of literals, and a value that is **an object id
//!   only**, re-resolved against its store at execution time. The only bytes
//!   that travel through Slack and back are an id whose meaning the store,
//!   not the payload, supplies.
//!
//! The executor ([`Executor::run`]) spawns the CLI as a child process, exactly
//! as the draft cards always did and the TUI's modals do — one implementation of each
//! verb, every store guard inherited rather than reimplemented. And the
//! outcome is **read back from the store the action was supposed to change**,
//! never from the child's exit alone: a child killed after the send but before
//! exiting must not report failure over a mail that left.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use mecha_core::doctor::Remedy;
use mecha_core::outbox::{OutboxItem, OutboxStore};
use mecha_core::trigger::{RunRecord, Trigger, TriggerStore};
use serde::{Deserialize, Serialize};

/// The `action_id` literals — the closed set of verbs a button may carry.
///
/// Composition and parsing share these constants, so a card cannot mint a verb
/// [`Action::from_payload`] does not know.
pub mod ids {
    pub const OUTBOX_SEND: &str = "slack_outbox_send";
    pub const OUTBOX_SEND_CONFIRM: &str = "slack_outbox_send_confirm";
    pub const OUTBOX_REJECT: &str = "slack_outbox_reject";
    pub const RESTART_UNIT: &str = "slack_action_restart_unit";
    pub const TRIGGER_RUN: &str = "slack_action_trigger_run";
    pub const TRIGGER_CANCEL: &str = "slack_action_trigger_cancel";
}

/// The closed set of things a tap may execute. A Rust enum rather than an
/// allowlist of strings, because a string list is data and data gets appended
/// to; adding a variant here is a diff someone reviews, and the compiler
/// forces the executor's match to handle it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Release a staged draft. The store's flock and pending check make a
    /// second delivery harmless.
    OutboxSend { id: String },
    /// Reject a staged draft — terminal but local; nothing leaves.
    OutboxReject { id: String },
    /// Restart a failed `mecha-*` user unit. Re-examined at tap time: a
    /// restart is idempotent against a *failed* unit but disruptive against a
    /// *running* one.
    RestartUnit { unit: String },
    /// Doctor's manual probe: evidence, not a fire — a manual run records a
    /// row with no slot and never advances the schedule.
    TriggerRun { name: String },
    /// Stop a running trigger at its next safe point; the partial turn is
    /// kept. A sentinel file the runner polls — writing it twice is writing
    /// it once.
    TriggerCancel { name: String },
}

impl Action {
    /// The command line, derived and never carried. A total match: the verb,
    /// the flags and the subcommand are literals in the arms, and the only
    /// non-literal parts are the typed fields — each validated by the
    /// constructor that admitted it.
    pub fn argv(&self) -> Vec<String> {
        match self {
            Action::OutboxSend { id } => vec![
                "mecha".into(),
                "outbox".into(),
                "send".into(),
                id.clone(),
                "-y".into(),
            ],
            Action::OutboxReject { id } => vec![
                "mecha".into(),
                "outbox".into(),
                "reject".into(),
                id.clone(),
                "--reason".into(),
                "rejected from Slack".into(),
            ],
            Action::RestartUnit { unit } => vec![
                "systemctl".into(),
                "--user".into(),
                "restart".into(),
                unit.clone(),
            ],
            Action::TriggerRun { name } => vec![
                "mecha".into(),
                "trigger".into(),
                "run".into(),
                name.clone(),
            ],
            Action::TriggerCancel { name } => vec![
                "mecha".into(),
                "trigger".into(),
                "cancel".into(),
                name.clone(),
            ],
        }
    }

    /// The recogniser that turns a doctor remedy into a button. It matches
    /// the argv *shape*, and anything unrecognised — including every
    /// `needs_terminal` remedy — renders as copyable code exactly as before,
    /// so an unrecognised remedy degrades to display, never to execution.
    /// This is why core needs no change: a new remedy shape in core is
    /// display-only on Slack until someone deliberately adds a variant here.
    pub fn from_remedy(remedy: &Remedy) -> Option<Action> {
        // A terminal-bound remedy has no phone execution path, ever — even if
        // its argv happens to match a shape below.
        if remedy.needs_terminal {
            return None;
        }
        let argv: Vec<&str> = remedy.argv.iter().map(String::as_str).collect();
        match argv.as_slice() {
            ["systemctl", "--user", "restart", unit] if is_mecha_unit(unit) => {
                Some(Action::RestartUnit {
                    unit: (*unit).to_string(),
                })
            }
            ["mecha", "trigger", "run", name] if is_trigger_name(name) => {
                Some(Action::TriggerRun {
                    name: (*name).to_string(),
                })
            }
            _ => None,
        }
    }

    /// Parse a button press coming back: fixed verb, store-resolved object.
    /// The value is validated by shape here and re-resolved against its store
    /// by the executor — an outbox id through `OutboxStore::item` (which
    /// errors on no match and on ambiguity), a trigger name through the
    /// trigger store, a unit name by re-appearing in `systemctl`'s own
    /// failed state at tap time.
    pub fn from_payload(action_id: &str, value: &str) -> Option<Action> {
        match action_id {
            ids::OUTBOX_SEND | ids::OUTBOX_SEND_CONFIRM if is_outbox_id(value) => {
                Some(Action::OutboxSend {
                    id: value.to_string(),
                })
            }
            ids::OUTBOX_REJECT if is_outbox_id(value) => Some(Action::OutboxReject {
                id: value.to_string(),
            }),
            ids::RESTART_UNIT if is_mecha_unit(value) => Some(Action::RestartUnit {
                unit: value.to_string(),
            }),
            ids::TRIGGER_RUN if is_trigger_name(value) => Some(Action::TriggerRun {
                name: value.to_string(),
            }),
            ids::TRIGGER_CANCEL if is_trigger_name(value) => Some(Action::TriggerCancel {
                name: value.to_string(),
            }),
            _ => None,
        }
    }

    /// The verb a card composing this action puts on its button — the same
    /// literal [`Action::from_payload`] parses, so the pair cannot drift.
    pub fn action_id(&self) -> &'static str {
        match self {
            Action::OutboxSend { .. } => ids::OUTBOX_SEND,
            Action::OutboxReject { .. } => ids::OUTBOX_REJECT,
            Action::RestartUnit { .. } => ids::RESTART_UNIT,
            Action::TriggerRun { .. } => ids::TRIGGER_RUN,
            Action::TriggerCancel { .. } => ids::TRIGGER_CANCEL,
        }
    }

    /// The object id the button carries — never a command fragment.
    pub fn value(&self) -> &str {
        match self {
            Action::OutboxSend { id } | Action::OutboxReject { id } => id,
            Action::RestartUnit { unit } => unit,
            Action::TriggerRun { name } | Action::TriggerCancel { name } => name,
        }
    }

    /// One line for the dispatch card: what the tap is doing, present tense.
    pub fn describe(&self) -> String {
        match self {
            Action::OutboxSend { id } => format!("releasing draft `{id}`"),
            Action::OutboxReject { id } => format!("rejecting draft `{id}`"),
            Action::RestartUnit { unit } => format!("restarting {unit}"),
            Action::TriggerRun { name } => format!("running trigger `{name}`"),
            Action::TriggerCancel { name } => format!("cancelling trigger `{name}`"),
        }
    }
}

/// The unit shape doctor's restart remedy names: `mecha-<word>.service`,
/// lowercase letters and hyphens. Anything else — another unit, a template, a
/// path — is not this surface's business and degrades to copyable text.
fn is_mecha_unit(unit: &str) -> bool {
    unit.strip_prefix("mecha-")
        .and_then(|rest| rest.strip_suffix(".service"))
        .is_some_and(|mid| {
            !mid.is_empty() && mid.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        })
}

/// The store's own rule, not a parallel one: a name the trigger store would
/// refuse as a filename is refused here for the same reasons.
fn is_trigger_name(name: &str) -> bool {
    Trigger::valid_name(name).is_ok()
}

/// An outbox id is minted by the store (timestamp + uuid fragment). Shape
/// check only — existence and ambiguity are the store's to answer at
/// execution time.
fn is_outbox_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------------------------------------------------------------- the ledger

/// A process-wide tap id: sortable, unique across concurrent taps, and shared
/// by the dispatch row and its outcome row.
pub fn new_tap_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S"),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// `~/.mecha/slack/actions.jsonl` — who asked, from where, and what became of
/// it. The stores already record most of *what happened*; what none of them
/// record is **who pressed it**: `mecha outbox send` has no actor field, and
/// a "sent by @who" living only in Slack is a rendering, not a store this
/// system can read back.
///
/// Two rows per tap rather than one, because a crash between dispatch and
/// outcome is exactly when the record matters: a dispatch row with no outcome
/// row is the durable evidence that a tap launched something whose result was
/// lost. The ledger records the tap and points at the store; it never
/// restates the item or the unit state, because a second source of truth is
/// the disease this project keeps naming.
///
/// **An observer, never load-bearing**: a write failure is logged and the
/// action proceeds — the audit trail must not become the way to block a send
/// the store guards already govern.
pub struct ActionLedger {
    path: Option<PathBuf>,
}

/// One ledger row. `action` is the serialized typed enum; `argv` is the
/// rendering derived from it at dispatch time, kept so the record is readable
/// without this module's code.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LedgerRow {
    Dispatch {
        tap_id: String,
        at: DateTime<Utc>,
        /// The Slack user id from the signed payload — or, for an
        /// auto-released draft, the owner who set the mode.
        user_id: String,
        action: Action,
        argv: Vec<String>,
        /// Which surface composed the control: `draft-card`, `doctor`,
        /// `review-auto`.
        surface: String,
    },
    Outcome {
        tap_id: String,
        at: DateTime<Utc>,
        /// One word for scripts: `sent`, `rejected`, `restarted`, `failed`,
        /// `skipped`, `unknown`…
        status: String,
        /// The line the card showed, verbatim.
        line: String,
    },
}

impl ActionLedger {
    /// The default ledger, beside the rest of the Slack state. A home that
    /// cannot be resolved yields a ledger that only logs — the observer rule:
    /// audit trouble must never block the action.
    pub fn open_default() -> Self {
        let path = mecha_core::work::mecha_home()
            .ok()
            .map(|home| home.join("slack").join("actions.jsonl"));
        Self { path }
    }

    /// A ledger at an explicit path, for tests.
    #[cfg(test)]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    pub fn dispatched(
        &self,
        tap_id: &str,
        user_id: &str,
        action: &Action,
        surface: &str,
    ) {
        self.append(&LedgerRow::Dispatch {
            tap_id: tap_id.to_string(),
            at: Utc::now(),
            user_id: user_id.to_string(),
            action: action.clone(),
            argv: action.argv(),
            surface: surface.to_string(),
        });
    }

    pub fn resolved(&self, tap_id: &str, status: &str, line: &str) {
        self.append(&LedgerRow::Outcome {
            tap_id: tap_id.to_string(),
            at: Utc::now(),
            status: status.to_string(),
            line: line.to_string(),
        });
    }

    /// Append one row. Owner-only like every other file under `~/.mecha` —
    /// the ledger names drafts, triggers and who pressed what. A failure is a
    /// warning, never an error: log and continue.
    fn append(&self, row: &LedgerRow) {
        let Some(path) = &self.path else {
            tracing::warn!("no mecha home — a tap went unledgered");
            return;
        };
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            if let Some(dir) = path.parent() {
                mecha_slack::store::create_private_dir(dir)?;
            }
            let fresh = !path.exists();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            if fresh {
                mecha_slack::store::set_owner_only(path)?;
            }
            let line = serde_json::to_string(row).map_err(std::io::Error::other)?;
            writeln!(file, "{line}")
        };
        if let Err(e) = write() {
            tracing::warn!("could not append to the action ledger: {e}");
        }
    }
}

// ------------------------------------------------------------- the executor

/// What became of an executed action, read back from the store it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// One word for the ledger.
    pub status: String,
    /// The line the card shows.
    pub line: String,
}

impl Outcome {
    fn of(status: &str, line: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            line: line.into(),
        }
    }
}

/// Runs one action as a CLI child process and answers from the store.
pub struct Executor {
    /// Where the outbox lives — the connector resolves `[outbox] dir` once
    /// and every release must read the same store the cards were built from.
    pub outbox_root: PathBuf,
}

impl Executor {
    /// Re-examine, run, and report from the store.
    ///
    /// The re-examination is §5's rule for the restart: the finding must
    /// still be true when the tap lands, not just when the card was posted —
    /// a restart of a unit that already recovered kills in-flight work. The
    /// other actions lean on their stores' own guards (the send flock and
    /// pending check, the trigger flock, the cancel sentinel), which is the
    /// cheapest possible answer: the primitives were already safe.
    pub async fn run(&self, action: &Action) -> Outcome {
        if let Action::RestartUnit { unit } = action {
            if !unit_is_failed(unit).await {
                return Outcome::of(
                    "skipped",
                    format!("{unit} already recovered — nothing was run"),
                );
            }
        }

        let started = Utc::now();
        let child_note = self.spawn(action).await;

        // The child's exit is never the answer; the store is. A child killed
        // after the send but before exiting must not report failure over a
        // mail that left.
        match action {
            Action::OutboxSend { id } => {
                draft_outcome(true, id, self.item(id).as_ref(), child_note.as_deref())
            }
            Action::OutboxReject { id } => {
                draft_outcome(false, id, self.item(id).as_ref(), child_note.as_deref())
            }
            Action::RestartUnit { unit } => restart_outcome(unit, unit_is_failed(unit).await),
            Action::TriggerRun { name } => trigger_run_outcome(
                name,
                latest_trigger_row(name, started).as_ref(),
                child_note.as_deref(),
            ),
            Action::TriggerCancel { name } => cancel_outcome(name),
        }
    }

    /// Spawn the derived argv. `mecha` means this binary, exactly as the
    /// doctor report already resolves it; `systemctl`
    /// comes from `PATH`. Returns the first stderr line (or the spawn error)
    /// as a note for the store-answered outcome to fall back on when the
    /// store shows nothing changed.
    async fn spawn(&self, action: &Action) -> Option<String> {
        let argv = action.argv();
        let (program, rest) = argv.split_first().expect("argv() is never empty");
        let program: PathBuf = if program == "mecha" {
            std::env::current_exe().unwrap_or_else(|_| "mecha".into())
        } else {
            program.into()
        };
        match tokio::process::Command::new(program)
            .args(rest)
            .stdin(std::process::Stdio::null())
            .output()
            .await
        {
            Ok(out) if out.status.success() => None,
            Ok(out) => String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .map(str::to_string),
            Err(e) => Some(e.to_string()),
        }
    }

    fn item(&self, id: &str) -> Option<OutboxItem> {
        OutboxStore::open(&self.outbox_root)
            .ok()
            .and_then(|s| s.item(id).ok())
    }
}

/// The draft's fate, from the item — `status` and `error` are durable on it,
/// and a failed release records the error and stays pending.
///
/// Deliberately has no exit-code parameter: the signature is the fix. The
/// child's stderr appears only when the store says nothing changed, as a hint
/// about why.
pub fn draft_outcome(
    send: bool,
    id: &str,
    item: Option<&OutboxItem>,
    child_note: Option<&str>,
) -> Outcome {
    let Some(item) = item else {
        // The one honest answer when the store cannot be read: point at the
        // surface that can.
        return Outcome::of(
            "unknown",
            format!("draft `{id}` — outcome unknown; check `mecha outbox show {id}`"),
        );
    };
    match item.status.as_str() {
        "sent" => Outcome::of("sent", format!("Draft `{id}` sent")),
        "rejected" => Outcome::of("rejected", format!("Draft `{id}` rejected")),
        "pending" => match &item.error {
            // The draft is still good; the delivery was not.
            Some(error) => Outcome::of(
                "failed",
                format!("Draft `{id}` release failed: {error} — the draft is still pending"),
            ),
            None => Outcome::of(
                "failed",
                match child_note {
                    Some(note) => {
                        format!("Draft `{id}` unchanged — {note}")
                    }
                    None => format!(
                        "Draft `{id}` unchanged — still pending; check `mecha outbox show {id}`"
                    ),
                },
            ),
        },
        other => Outcome::of(
            other,
            format!(
                "Draft `{id}` is `{other}`{}",
                if send { " — nothing was sent by this tap" } else { "" }
            ),
        ),
    }
}

/// The unit's state is the outcome; the restart command's exit is not.
/// "Restarted, and it failed again" matters more than "restarted" — it says
/// the fix is upstream.
pub fn restart_outcome(unit: &str, still_failed: bool) -> Outcome {
    if still_failed {
        Outcome::of(
            "failed-again",
            format!(
                "Restarted {unit}, and it failed again — the fix is upstream \
                 (journalctl --user -u {unit} -n 20)"
            ),
        )
    } else {
        Outcome::of("restarted", format!("Restarted {unit}, and it is running"))
    }
}

/// The ledger row is the record; a second copy could disagree with it. `row`
/// is the newest row this trigger wrote after the tap — including a skip,
/// which is a real answer ("the previous run was still going").
pub fn trigger_run_outcome(
    name: &str,
    row: Option<&RunRecord>,
    child_note: Option<&str>,
) -> Outcome {
    match row {
        Some(row) => {
            let status = row.status.as_str();
            let line = match &row.error {
                Some(error) => format!("Trigger `{name}` ran: {status} — {error}"),
                None => format!("Trigger `{name}` ran: {status}"),
            };
            Outcome::of(status, line)
        }
        None => Outcome::of(
            "unknown",
            match child_note {
                Some(note) => format!("Trigger `{name}` recorded no run — {note}"),
                None => format!(
                    "Trigger `{name}` recorded no run — see `mecha trigger runs {name}`"
                ),
            },
        ),
    }
}

/// The running marker and the sentinel, re-read. Cancel of a non-running
/// trigger is "not running", idempotent by construction.
fn cancel_outcome(name: &str) -> Outcome {
    let Some(store) = TriggerStore::open_existing_default() else {
        return Outcome::of("unknown", format!("no trigger store — `{name}` unknown"));
    };
    match store.running(name) {
        Some(_) if store.cancel_requested(name) => Outcome::of(
            "cancelling",
            format!("Asked `{name}` to stop — it ends at its next safe point, partial turn kept"),
        ),
        Some(_) => Outcome::of(
            "unknown",
            format!("`{name}` is still running and no cancel was recorded — try again"),
        ),
        None => Outcome::of("stopped", format!("`{name}` is not running")),
    }
}

/// `systemctl --user is-failed <unit>` exits 0 exactly when the unit is
/// failed. A machine without systemd answers "not failed", which makes the
/// pre-check skip and the outcome read "running" — doctor would never have
/// composed the button there in the first place.
async fn unit_is_failed(unit: &str) -> bool {
    tokio::process::Command::new("systemctl")
        .args(["--user", "is-failed", unit])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// The newest ledger row for `name` written at or after `since` (with a small
/// allowance for clock rounding), whatever its status — a skip is an answer.
fn latest_trigger_row(name: &str, since: DateTime<Utc>) -> Option<RunRecord> {
    let store = TriggerStore::open_existing_default()?;
    let cutoff = since - chrono::Duration::seconds(5);
    store
        .runs()
        .ok()?
        .into_iter()
        .filter(|r| r.trigger == name && r.started_at >= cutoff)
        .max_by_key(|r| r.started_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::trigger::RunStatus;

    fn remedy(argv: &[&str], needs_terminal: bool) -> Remedy {
        Remedy {
            description: "a remedy".into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            needs_terminal,
        }
    }

    #[test]
    fn from_remedy_recognises_exactly_the_two_shapes() {
        assert_eq!(
            Action::from_remedy(&remedy(
                &["systemctl", "--user", "restart", "mecha-triggers.service"],
                false
            )),
            Some(Action::RestartUnit {
                unit: "mecha-triggers.service".into()
            })
        );
        assert_eq!(
            Action::from_remedy(&remedy(&["mecha", "trigger", "run", "briefing"], false)),
            Some(Action::TriggerRun {
                name: "briefing".into()
            })
        );
    }

    #[test]
    fn from_remedy_refuses_every_terminal_bound_and_unrecognised_shape() {
        // needs_terminal has no phone execution path even when the argv would
        // otherwise match.
        assert_eq!(
            Action::from_remedy(&remedy(
                &["systemctl", "--user", "restart", "mecha-triggers.service"],
                true
            )),
            None
        );
        // Every other shipped remedy shape degrades to display.
        for argv in [
            vec!["mecha-mail", "auth", "personal", "--provider", "google"],
            vec!["mecha-mail", "import", "google", "--provider", "google"],
            vec!["mecha", "outbox", "review"],
            vec!["mecha", "frontdoor", "list"],
            // Not the trusted unit shape: another unit, a hostile lookalike,
            // a case variant, an extra argument.
            vec!["systemctl", "--user", "restart", "nginx.service"],
            vec!["systemctl", "--user", "restart", "mecha-x.service", "--now"],
            vec!["systemctl", "--user", "restart", "mecha-X.SERVICE"],
            vec!["systemctl", "--user", "restart", "mecha-.service"],
            vec!["systemctl", "restart", "mecha-triggers.service"],
            // Not the trigger-run shape.
            vec!["mecha", "trigger", "run", "briefing", "--force"],
            vec!["mecha", "trigger", "delete", "briefing"],
            vec!["mecha", "trigger", "run", "../escape"],
            vec![],
        ] {
            let r = remedy(&argv, false);
            assert_eq!(Action::from_remedy(&r), None, "{argv:?} must not execute");
        }
    }

    #[test]
    fn argv_is_total_and_its_verbs_are_literals() {
        // Every variant renders, the program is a literal, and the typed
        // field appears as one argument — never spliced into a command word.
        let samples = [
            Action::OutboxSend { id: "abc-123".into() },
            Action::OutboxReject { id: "abc-123".into() },
            Action::RestartUnit {
                unit: "mecha-triggers.service".into(),
            },
            Action::TriggerRun {
                name: "briefing".into(),
            },
            Action::TriggerCancel {
                name: "briefing".into(),
            },
        ];
        for action in &samples {
            let argv = action.argv();
            assert!(!argv.is_empty());
            assert!(
                argv[0] == "mecha" || argv[0] == "systemctl",
                "{argv:?} spawns an unexpected program"
            );
            assert!(
                argv.iter().any(|a| a == action.value()),
                "the object id rides as its own argument: {argv:?}"
            );
        }
    }

    #[test]
    fn a_payload_round_trips_through_its_fixed_verb_and_carries_the_id_only() {
        // The value that travelled through Slack is the object id; the argv
        // is re-derived on this side, never parsed from the payload.
        for action in [
            Action::OutboxSend { id: "abc-123".into() },
            Action::OutboxReject { id: "abc-123".into() },
            Action::RestartUnit {
                unit: "mecha-triggers.service".into(),
            },
            Action::TriggerRun {
                name: "briefing".into(),
            },
            Action::TriggerCancel {
                name: "briefing".into(),
            },
        ] {
            let back = Action::from_payload(action.action_id(), action.value());
            assert_eq!(back, Some(action));
        }
        // The confirm verb resolves to the same send action — the two-step's
        // second tap executes nothing new.
        assert_eq!(
            Action::from_payload(ids::OUTBOX_SEND_CONFIRM, "abc-123"),
            Some(Action::OutboxSend { id: "abc-123".into() })
        );
    }

    #[test]
    fn a_payload_with_an_unknown_verb_or_a_hostile_value_is_refused() {
        assert_eq!(Action::from_payload("slack_stop", "D1-1.0"), None);
        assert_eq!(Action::from_payload("slack_action_run_command", "ls"), None);
        // The value is an id, not a command fragment — anything shaped
        // otherwise dies here, before any store is consulted.
        for hostile in [
            "mecha-x.service; rm -rf /",
            "../../etc/passwd",
            "mecha-X.SERVICE",
            "nginx.service",
            "",
        ] {
            assert_eq!(Action::from_payload(ids::RESTART_UNIT, hostile), None, "{hostile}");
        }
        for hostile in ["../escape", "a b", "UPPER", ""] {
            assert_eq!(Action::from_payload(ids::TRIGGER_RUN, hostile), None, "{hostile}");
            assert_eq!(Action::from_payload(ids::TRIGGER_CANCEL, hostile), None, "{hostile}");
        }
        for hostile in ["", "a b", "x/../y", &"x".repeat(200)] {
            assert_eq!(Action::from_payload(ids::OUTBOX_SEND, hostile), None, "{hostile}");
        }
    }

    #[test]
    fn the_ledger_writes_a_dispatch_row_and_an_outcome_row_that_share_a_tap_id() {
        let dir = std::env::temp_dir().join(format!(
            "mecha-action-ledger-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("actions.jsonl");
        let ledger = ActionLedger::at(&path);

        let tap = new_tap_id();
        let action = Action::RestartUnit {
            unit: "mecha-triggers.service".into(),
        };
        ledger.dispatched(&tap, "U_OWNER", &action, "doctor");
        ledger.resolved(&tap, "restarted", "Restarted mecha-triggers.service");

        let text = std::fs::read_to_string(&path).unwrap();
        let rows: Vec<LedgerRow> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        match &rows[0] {
            LedgerRow::Dispatch {
                tap_id,
                user_id,
                action: recorded,
                argv,
                surface,
                ..
            } => {
                assert_eq!(tap_id, &tap);
                assert_eq!(user_id, "U_OWNER");
                assert_eq!(recorded, &action);
                assert_eq!(argv, &action.argv(), "the argv is derived, and recorded");
                assert_eq!(surface, "doctor");
            }
            other => panic!("expected a dispatch row, got {other:?}"),
        }
        match &rows[1] {
            LedgerRow::Outcome { tap_id, status, .. } => {
                assert_eq!(tap_id, &tap, "the rows share the tap id");
                assert_eq!(status, "restarted");
            }
            other => panic!("expected an outcome row, got {other:?}"),
        }

        // Owner-only, like everything else under ~/.mecha.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the ledger names drafts and who pressed what");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_ledger_that_cannot_write_never_blocks_the_action() {
        // The observer rule: these calls must return, not error or panic.
        let ledger = ActionLedger::at("/proc/no-such-dir/actions.jsonl");
        let action = Action::TriggerRun {
            name: "briefing".into(),
        };
        ledger.dispatched("t-1", "U_OWNER", &action, "doctor");
        ledger.resolved("t-1", "ok", "ran");
    }

    #[test]
    fn tap_ids_never_collide_within_a_process() {
        let a = new_tap_id();
        let b = new_tap_id();
        assert_ne!(a, b);
    }

    fn item_with(status: &str, error: Option<&str>) -> OutboxItem {
        use mecha_core::agent::Taint;
        use mecha_core::outbox::OutboxKind;
        OutboxItem {
            id: "abc-123".into(),
            status: status.into(),
            tool: "mail__send".into(),
            kind: OutboxKind::Message,
            args_before: serde_json::json!({}),
            args: serde_json::json!({}),
            summary: "mail__send".into(),
            session_id: None,
            workspace: None,
            taint: Taint::default(),
            created_at: "2026-08-14T00:00:00Z".into(),
            resolved_at: None,
            reason: None,
            error: error.map(String::from),
        }
    }

    /// The fails-on-old-behaviour test for §6: the old path reported from the
    /// child's exit status and first stderr line. This function's signature
    /// has no exit code at all — a sent item reports sent even beside a
    /// child's complaint, which is exactly the child-killed-after-the-send
    /// case that used to report failure over a mail that left.
    #[test]
    fn a_drafts_outcome_is_the_items_status_never_the_childs_exit() {
        let sent = draft_outcome(
            true,
            "abc-123",
            Some(&item_with("sent", None)),
            Some("child was killed"),
        );
        assert_eq!(sent.status, "sent");
        assert!(sent.line.contains("sent"), "{}", sent.line);
        assert!(
            !sent.line.contains("killed"),
            "a sent mail is sent, whatever the child said: {}",
            sent.line
        );

        // A failed release records the error and stays pending — the draft is
        // still good, the delivery was not, and the card says so.
        let failed = draft_outcome(
            true,
            "abc-123",
            Some(&item_with("pending", Some("smtp said no"))),
            None,
        );
        assert_eq!(failed.status, "failed");
        assert!(failed.line.contains("smtp said no"), "{}", failed.line);
        assert!(failed.line.contains("still pending"), "{}", failed.line);

        let rejected = draft_outcome(false, "abc-123", Some(&item_with("rejected", None)), None);
        assert_eq!(rejected.status, "rejected");

        // An unreadable store is unknown, never a guess — and it names the
        // surface that can answer.
        let unknown = draft_outcome(true, "abc-123", None, None);
        assert_eq!(unknown.status, "unknown");
        assert!(unknown.line.contains("mecha outbox show abc-123"), "{}", unknown.line);
    }

    #[test]
    fn a_restart_reports_the_units_state_not_the_commands_exit() {
        let ok = restart_outcome("mecha-triggers.service", false);
        assert_eq!(ok.status, "restarted");
        assert!(ok.line.contains("running"), "{}", ok.line);

        let refailed = restart_outcome("mecha-triggers.service", true);
        assert_eq!(refailed.status, "failed-again");
        assert!(refailed.line.contains("upstream"), "{}", refailed.line);
        assert!(
            refailed.line.contains("journalctl --user -u mecha-triggers.service"),
            "{}",
            refailed.line
        );
    }

    #[test]
    fn a_trigger_runs_outcome_is_the_ledger_row_and_a_skip_is_an_answer() {
        let mut row = RunRecord::started("briefing", None, true);
        row.status = RunStatus::Error;
        row.error = Some("provider said no".into());
        let out = trigger_run_outcome("briefing", Some(&row), None);
        assert_eq!(out.status, "error");
        assert!(out.line.contains("provider said no"), "{}", out.line);

        let mut skipped = RunRecord::started("briefing", None, true);
        skipped.status = RunStatus::SkippedOverlap;
        let out = trigger_run_outcome("briefing", Some(&skipped), None);
        assert_eq!(out.status, "skipped (overlap)");

        let none = trigger_run_outcome("briefing", None, None);
        assert_eq!(none.status, "unknown");
        assert!(none.line.contains("mecha trigger runs briefing"), "{}", none.line);
    }
}
