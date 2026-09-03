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

use std::path::{Path, PathBuf};
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
    pub const TRIGGER_ENABLE: &str = "slack_action_trigger_enable";
    pub const TRIGGER_DISABLE: &str = "slack_action_trigger_disable";
    pub const MAIL_IMPORT: &str = "slack_action_mail_import";
    pub const TASK_DONE: &str = "slack_action_task_done";
    pub const TASK_NEXT: &str = "slack_action_task_next";
    /// Modal callback ids, parsed by [`super::Action::from_submission`] — the
    /// one constructor that accepts owner-typed text, and only from a signed,
    /// gated `view_submission`. Never valid in [`super::Action::from_payload`]:
    /// a button cannot carry free text into an argv.
    pub const FRONTDOOR_CLOSE_SUBMIT: &str = "slack_frontdoor_close_submit";
    pub const FRONTDOOR_NEEDS_INFO_SUBMIT: &str = "slack_frontdoor_needs_info_submit";
}

/// The most characters of owner-typed modal text an action will carry into an
/// argv. The modal's input element declares the same cap so the person is told
/// at typing time; this constant is the enforcement, because a client-side cap
/// is a courtesy and not a boundary.
pub const MODAL_TEXT_MAX: usize = 500;

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
    /// Re-arm a disabled trigger. A reversible flag; setting it to its
    /// current value is a no-op, so replay is harmless by construction.
    TriggerEnable { name: String },
    /// Silence a trigger without deleting it — the phone-safe answer to "make
    /// this stop", where delete stays terminal-only.
    TriggerDisable { name: String },
    /// Bring a legacy per-provider mail login into the unified registry —
    /// additive: the import refuses to overwrite live credentials, so a
    /// second tap fails loudly rather than swapping a mailbox.
    MailImport { provider: String },
    /// Mark a board task done. Phone-safe on the board's own argument: the
    /// change reaches nobody (`kg_task_*` is `openWorldHint: false`), every
    /// status is one move from where it was, and the tool surface has no
    /// delete — so replay re-asserts a state rather than compounding one.
    TaskDone { id: String },
    /// Commit an inbox capture to `next`. The same reversibility argument.
    TaskNext { id: String },
    /// Close a frontdoor request, with the reason the frontdoor design makes
    /// mandatory. `reason` is **owner-authored text from a gated modal
    /// submission** — see [`Action::from_submission`], the only constructor
    /// that can produce this variant.
    FrontdoorClose { seq: i64, reason: String },
    /// Park a frontdoor request until the requester answers. `question` has
    /// the same provenance rule as a close's reason.
    FrontdoorNeedsInfo { seq: i64, question: String },
}

impl Action {
    /// The command line, derived and never carried. A total match: the verb,
    /// the flags and the subcommand are literals in the arms, and the only
    /// non-literal parts are the typed fields — each validated by the
    /// constructor that admitted it.
    ///
    /// Provenance, extended for the modal variants: the verb and the object id
    /// are machine-authored from typed store state as ever; a `reason` or
    /// `question` is **owner-authored text that arrived through a gated modal
    /// submission**, length-capped by [`Action::from_submission`], and it
    /// rides as **one argv element** — never through a shell, so its bytes
    /// reach exactly one `--reason`/`--note` argument and can name no second
    /// command. Nothing model-authored composes any part of any arm.
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
            Action::TriggerRun { name } => {
                vec!["mecha".into(), "trigger".into(), "run".into(), name.clone()]
            }
            Action::TriggerCancel { name } => vec![
                "mecha".into(),
                "trigger".into(),
                "cancel".into(),
                name.clone(),
            ],
            Action::TriggerEnable { name } => vec![
                "mecha".into(),
                "trigger".into(),
                "enable".into(),
                name.clone(),
            ],
            Action::TriggerDisable { name } => vec![
                "mecha".into(),
                "trigger".into(),
                "disable".into(),
                name.clone(),
            ],
            Action::MailImport { provider } => vec![
                "mecha-mail".into(),
                "import".into(),
                provider.clone(),
                "--provider".into(),
                provider.clone(),
            ],
            Action::TaskDone { id } => vec![
                "mecha".into(),
                "tasks".into(),
                "set".into(),
                id.clone(),
                "--status".into(),
                "done".into(),
            ],
            Action::TaskNext { id } => vec![
                "mecha".into(),
                "tasks".into(),
                "set".into(),
                id.clone(),
                "--status".into(),
                "next".into(),
            ],
            Action::FrontdoorClose { seq, reason } => vec![
                "mecha".into(),
                "frontdoor".into(),
                "close".into(),
                seq.to_string(),
                "--reason".into(),
                reason.clone(),
            ],
            Action::FrontdoorNeedsInfo { seq, question } => vec![
                "mecha".into(),
                "frontdoor".into(),
                "needs-info".into(),
                seq.to_string(),
                "--note".into(),
                question.clone(),
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
            // The legacy-store import: additive (it refuses to overwrite live
            // credentials), and the doctor remedy always names the provider
            // twice — account name and provider flag — so a shape where they
            // disagree is not this remedy and stays copyable text.
            ["mecha-mail", "import", name, "--provider", provider]
                if name == provider && is_mail_provider(provider) =>
            {
                Some(Action::MailImport {
                    provider: (*provider).to_string(),
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
            ids::TRIGGER_ENABLE if is_trigger_name(value) => Some(Action::TriggerEnable {
                name: value.to_string(),
            }),
            ids::TRIGGER_DISABLE if is_trigger_name(value) => Some(Action::TriggerDisable {
                name: value.to_string(),
            }),
            ids::MAIL_IMPORT if is_mail_provider(value) => Some(Action::MailImport {
                provider: value.to_string(),
            }),
            ids::TASK_DONE if is_task_id(value) => Some(Action::TaskDone {
                id: value.to_string(),
            }),
            ids::TASK_NEXT if is_task_id(value) => Some(Action::TaskNext {
                id: value.to_string(),
            }),
            // Deliberately unreachable here: the frontdoor variants carry
            // owner-typed text, and a button's payload must never be able to
            // smuggle text into an argv. They are constructible only through
            // [`Action::from_submission`].
            _ => None,
        }
    }

    /// Parse a gated modal submission into an action. The **only** constructor
    /// that accepts free text, and the provenance is the point: the verb is
    /// the modal's `callback_id`, fixed at compose time from the closed set;
    /// the seq is machine-authored correlation state (`private_metadata`,
    /// written by the code that opened the modal); and the text is
    /// **owner-authored**, typed into a modal that only opened for a
    /// gate-passing tap and only parses here after the submission's signed
    /// user passed the same gate. Nothing model-authored composes any of it.
    ///
    /// Fail-closed on every field: an unknown callback, a seq that is not a
    /// positive integer, empty-after-trim text, or text past
    /// [`MODAL_TEXT_MAX`] all parse to `None` — the length cap holds here even
    /// though the modal's input element declares the same cap, because a
    /// client-side cap is a courtesy and not a boundary.
    pub fn from_submission(callback_id: &str, seq: &str, text: &str) -> Option<Action> {
        let seq: i64 = seq.parse().ok().filter(|s| *s > 0)?;
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MODAL_TEXT_MAX {
            return None;
        }
        match callback_id {
            ids::FRONTDOOR_CLOSE_SUBMIT => Some(Action::FrontdoorClose {
                seq,
                reason: text.to_string(),
            }),
            ids::FRONTDOOR_NEEDS_INFO_SUBMIT => Some(Action::FrontdoorNeedsInfo {
                seq,
                question: text.to_string(),
            }),
            _ => None,
        }
    }

    /// The verb a card composing this action puts on its button — the same
    /// literal [`Action::from_payload`] parses, so the pair cannot drift. For
    /// the modal variants it is the `callback_id` the submission carries, and
    /// [`Action::from_submission`] is the parser it cannot drift from.
    pub fn action_id(&self) -> &'static str {
        match self {
            Action::OutboxSend { .. } => ids::OUTBOX_SEND,
            Action::OutboxReject { .. } => ids::OUTBOX_REJECT,
            Action::RestartUnit { .. } => ids::RESTART_UNIT,
            Action::TriggerRun { .. } => ids::TRIGGER_RUN,
            Action::TriggerCancel { .. } => ids::TRIGGER_CANCEL,
            Action::TriggerEnable { .. } => ids::TRIGGER_ENABLE,
            Action::TriggerDisable { .. } => ids::TRIGGER_DISABLE,
            Action::MailImport { .. } => ids::MAIL_IMPORT,
            Action::TaskDone { .. } => ids::TASK_DONE,
            Action::TaskNext { .. } => ids::TASK_NEXT,
            Action::FrontdoorClose { .. } => ids::FRONTDOOR_CLOSE_SUBMIT,
            Action::FrontdoorNeedsInfo { .. } => ids::FRONTDOOR_NEEDS_INFO_SUBMIT,
        }
    }

    /// The object id the button carries — never a command fragment, and never
    /// the modal text: a frontdoor variant's value is its seq, because the
    /// text travels only inside the typed action, from the submission.
    pub fn value(&self) -> String {
        match self {
            Action::OutboxSend { id } | Action::OutboxReject { id } => id.clone(),
            Action::RestartUnit { unit } => unit.clone(),
            Action::TriggerRun { name }
            | Action::TriggerCancel { name }
            | Action::TriggerEnable { name }
            | Action::TriggerDisable { name } => name.clone(),
            Action::MailImport { provider } => provider.clone(),
            Action::TaskDone { id } | Action::TaskNext { id } => id.clone(),
            Action::FrontdoorClose { seq, .. } | Action::FrontdoorNeedsInfo { seq, .. } => {
                seq.to_string()
            }
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
            Action::TriggerEnable { name } => format!("enabling trigger `{name}`"),
            Action::TriggerDisable { name } => format!("disabling trigger `{name}`"),
            Action::MailImport { provider } => {
                format!("importing the legacy {provider} mail login")
            }
            Action::TaskDone { id } => format!("marking task `{id}` done"),
            Action::TaskNext { id } => format!("moving task `{id}` to next"),
            Action::FrontdoorClose { seq, .. } => format!("closing request {seq}"),
            Action::FrontdoorNeedsInfo { seq, .. } => {
                format!("parking request {seq} for more information")
            }
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

/// A task id is minted by the graph: `task-` plus an id fragment. Shape check
/// only — existence is the store's to answer at execution time, where
/// `kg_task_update` errors on an unknown id rather than creating one.
fn is_task_id(id: &str) -> bool {
    id.strip_prefix("task-").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

/// The two legacy per-provider stores that exist. A closed set rather than a
/// shape check, because the set is closed: `mecha-google` and `mecha-outlook`
/// are the shipped binaries `mecha-mail import` migrates from.
fn is_mail_provider(provider: &str) -> bool {
    matches!(provider, "google" | "outlook")
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

// ------------------------------------------- the confirm card's fingerprint

/// A fingerprint of the exact argument bytes a tainted confirm card showed,
/// carried in the Send-anyway button's value so the press can prove the store
/// still holds them (design §5: store state is the defence, the card is
/// convenience). FNV-1a 64 rather than `DefaultHasher`, because the card may
/// be pressed by a different connector process than composed it and the
/// value must mean the same thing across restarts and builds. Not a secret
/// and not authorisation — the press is still gated on the signed user, and
/// the id is still re-resolved against the store; the fingerprint only ever
/// equals or differs.
pub fn fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The Send-anyway button's value: the item id plus the fingerprint of the
/// arguments the card shows. Still no command fragment — an id whose meaning
/// the store supplies, and correlation state the press verifies.
pub fn confirm_value(id: &str, args: &str) -> String {
    format!("{id}#{}", fingerprint(args))
}

/// `(id, fingerprint)` back out of a confirm button's value. A card composed
/// before values carried a fingerprint has a bare id — parsed, fingerprint
/// absent, and the press re-cards rather than sends on it. An id failing the
/// shape check parses to nothing, exactly as in [`Action::from_payload`].
pub fn parse_confirm_value(value: &str) -> Option<(&str, Option<&str>)> {
    match value.split_once('#') {
        Some((id, fp)) => is_outbox_id(id).then_some((id, Some(fp))),
        None => is_outbox_id(value).then_some((value, None)),
    }
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

    pub fn dispatched(&self, tap_id: &str, user_id: &str, action: &Action, surface: &str) {
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
            // The guard is shared with the TUI's /doctor modal
            // (`commands::doctor::recovered_before_restart`) so the two tap
            // surfaces cannot disagree about when a restart is skipped.
            if let Some(line) =
                crate::commands::doctor::recovered_before_restart(unit, unit_is_failed(unit).await)
            {
                return Outcome::of("skipped", line);
            }
            let _ = self.spawn(action).await;
            return restart_outcome(unit, unit_is_failed(unit).await);
        }

        // A task action's store is the graph behind MCP — there is no local
        // file to re-read. But `mecha tasks set` prints `kg_task_update`'s
        // own JSON answer, which is the store's word *after* the write,
        // delivered on stdout; parsing that is the read-back, and the
        // child's exit is still never the answer by itself (a clean exit
        // with an unreadable answer reports unknown, not done).
        if let Action::TaskDone { id } | Action::TaskNext { id } = action {
            let argv = action.argv();
            let (_, rest) = argv.split_first().expect("argv() is never empty");
            let out = tokio::process::Command::new(crate::exe::self_exe())
                .args(rest)
                .stdin(std::process::Stdio::null())
                .output()
                .await;
            let want = match action {
                Action::TaskNext { .. } => "next",
                _ => "done",
            };
            return match out {
                Ok(out) if out.status.success() => {
                    task_outcome(id, want, serde_json::from_slice(&out.stdout).ok().as_ref())
                }
                Ok(out) => Outcome::of(
                    "failed",
                    format!(
                        "Task `{id}` unchanged — {}",
                        String::from_utf8_lossy(&out.stderr)
                            .lines()
                            .next_back()
                            .unwrap_or("mecha tasks set failed")
                    ),
                ),
                Err(e) => Outcome::of("failed", format!("Task `{id}` unchanged — {e}")),
            };
        }

        let started = Utc::now();
        let child_note = self.spawn(action).await;

        // The read-back is store reads — blocking fs — and this future runs
        // on the connector's one runtime, so it goes to the blocking pool
        // exactly as the systemctl probe above does. Losing the read-back
        // task is answered as unknown, never a guess.
        let action = action.clone();
        let outbox_root = self.outbox_root.clone();
        tokio::task::spawn_blocking(move || {
            store_outcome(&outbox_root, &action, started, child_note.as_deref())
        })
        .await
        .unwrap_or_else(|e| {
            Outcome::of(
                "unknown",
                format!("the outcome could not be read back: {e}"),
            )
        })
    }

    /// Spawn the derived argv. `mecha` means this binary, exactly as the
    /// doctor report already resolves it; `mecha-mail` is looked for beside
    /// this binary first (the release layout) before falling back to `PATH`;
    /// `systemctl` comes from `PATH`. Returns the first stderr line (or the
    /// spawn error) as a note for the store-answered outcome to fall back on
    /// when the store shows nothing changed.
    async fn spawn(&self, action: &Action) -> Option<String> {
        let argv = action.argv();
        let (program, rest) = argv.split_first().expect("argv() is never empty");
        let program: PathBuf = if program == "mecha" {
            crate::exe::self_exe()
        } else if let Some(sibling) = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join(program)))
            .filter(|p| program.starts_with("mecha-") && p.is_file())
        {
            sibling
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

    /// The one draft an action named, off the runtime: an exact single-file
    /// read (card values carry full store-minted ids), on the blocking pool.
    pub(crate) async fn item(&self, id: &str) -> Option<OutboxItem> {
        let root = self.outbox_root.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || item_in(&root, &id))
            .await
            .ok()
            .flatten()
    }
}

/// The blocking single-item read behind the executor's read-backs: the exact
/// item file, never `items()`'s scan of every draft ever staged. Unreadable
/// — hostile shape, torn file, unopenable store — collapses to `None`, which
/// every outcome function reports as unknown rather than guessed.
fn item_in(root: &Path, id: &str) -> Option<OutboxItem> {
    OutboxStore::open(root).ok()?.item_exact(id).ok().flatten()
}

/// The store read-back for every non-restart action, run on the blocking
/// pool by [`Executor::run`]: a full outbox scan or a ledger read on the
/// connector's runtime would stall every thread's event dispatch. The
/// restart arm stays in `run` — its probe is already `spawn_blocking`.
///
/// The child's exit is never the answer; the store is. A child killed after
/// the send but before exiting must not report failure over a mail that
/// left.
fn store_outcome(
    outbox_root: &Path,
    action: &Action,
    started: DateTime<Utc>,
    child_note: Option<&str>,
) -> Outcome {
    match action {
        Action::OutboxSend { id } => {
            draft_outcome(true, id, item_in(outbox_root, id).as_ref(), child_note)
        }
        Action::OutboxReject { id } => {
            draft_outcome(false, id, item_in(outbox_root, id).as_ref(), child_note)
        }
        // Answered in `run`, before this function is reached; an arm here
        // anyway, so the match stays total without a panic in spawned work.
        Action::RestartUnit { unit } => Outcome::of(
            "unknown",
            format!("{unit} — the restart outcome is read in run()"),
        ),
        Action::TriggerRun { name } => {
            trigger_run_outcome(name, latest_trigger_row(name, started).as_ref(), child_note)
        }
        Action::TriggerCancel { name } => cancel_outcome(name),
        Action::TriggerEnable { name } => {
            toggle_outcome(name, true, trigger_enabled(name), child_note)
        }
        Action::TriggerDisable { name } => {
            toggle_outcome(name, false, trigger_enabled(name), child_note)
        }
        Action::MailImport { provider } => {
            import_outcome(provider, registry_credentials_exist(provider), child_note)
        }
        // Answered in `run`, before this function is reached — the graph has
        // no local store to read back from here; arms anyway, so the match
        // stays total without a panic in spawned work.
        Action::TaskDone { id } | Action::TaskNext { id } => Outcome::of(
            "unknown",
            format!("task `{id}` — the outcome is read in run()"),
        ),
        Action::FrontdoorClose { seq, .. } => mark_outcome(
            *seq,
            mecha_core::frontdoor::CLOSED,
            request_state(*seq).as_deref(),
            child_note,
        ),
        Action::FrontdoorNeedsInfo { seq, .. } => mark_outcome(
            *seq,
            mecha_core::frontdoor::NEEDS_INFO,
            request_state(*seq).as_deref(),
            child_note,
        ),
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
                if send {
                    " — nothing was sent by this tap"
                } else {
                    ""
                }
            ),
        ),
    }
}

/// The task as the graph now holds it is the outcome. `answer` is
/// `kg_task_update`'s reply (`{"status": "updated", "task": {...}}`), passed
/// through `mecha tasks set`'s stdout; the nested task's status is the field
/// that answers, because the envelope's `"updated"` says the call ran and
/// not what it left behind.
pub fn task_outcome(id: &str, want: &str, answer: Option<&serde_json::Value>) -> Outcome {
    let task = answer.map(|a| &a["task"]);
    let name = task
        .and_then(|t| t["name"].as_str())
        .map(|n| format!(" — {n}"))
        .unwrap_or_default();
    match task.and_then(|t| t["status"].as_str()) {
        Some(status) if status == want => {
            Outcome::of(want, format!("Task `{id}` is {want}{name}"))
        }
        // The store answered with a different state than the tap asked for —
        // report what it holds, never what was wanted.
        Some(status) => Outcome::of(
            "failed",
            format!("Task `{id}` reads `{status}` after the tap{name} — check `mecha tasks list`"),
        ),
        None => Outcome::of(
            "unknown",
            format!("Task `{id}` — the update ran but the answer was unreadable; check `mecha tasks list`"),
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
                None => {
                    format!("Trigger `{name}` recorded no run — see `mecha trigger runs {name}`")
                }
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

/// The trigger file, re-read: enable and disable report the flag as it now
/// stands, never the child's exit.
fn trigger_enabled(name: &str) -> Option<bool> {
    let store = TriggerStore::open_existing_default()?;
    store.get(name).ok().map(|t| t.enabled)
}

/// The flag as it stands is the outcome. Setting it to its current value is a
/// no-op, so "already" and "now" collapse into the same honest line.
pub fn toggle_outcome(
    name: &str,
    want_enabled: bool,
    enabled_now: Option<bool>,
    child_note: Option<&str>,
) -> Outcome {
    let (verb, undo) = if want_enabled {
        ("enabled", "disable")
    } else {
        ("disabled", "enable")
    };
    match enabled_now {
        Some(state) if state == want_enabled => Outcome::of(
            verb,
            format!("Trigger `{name}` is {verb} — `mecha trigger {undo} {name}` undoes it"),
        ),
        Some(_) => Outcome::of(
            "failed",
            match child_note {
                Some(note) => format!("Trigger `{name}` is unchanged — {note}"),
                None => format!("Trigger `{name}` is unchanged — see `mecha trigger show {name}`"),
            },
        ),
        None => Outcome::of(
            "unknown",
            format!("Trigger `{name}` could not be re-read — see `mecha trigger show {name}`"),
        ),
    }
}

/// Whether the unified registry now holds credentials for the imported
/// account. The import writes `~/.mecha/mail/<provider>/oauth.json`; its
/// presence, not the child's exit, is the outcome.
fn registry_credentials_exist(provider: &str) -> bool {
    mecha_core::work::mecha_home()
        .map(|home| {
            home.join("mail")
                .join(provider)
                .join("oauth.json")
                .is_file()
        })
        .unwrap_or(false)
}

/// The registry is the outcome. An import moves the login, not its health —
/// the doctor finding that offered this button was about a *dead* legacy
/// login, so the success line says what still needs a terminal.
pub fn import_outcome(
    provider: &str,
    credentials_present: bool,
    child_note: Option<&str>,
) -> Outcome {
    if credentials_present {
        Outcome::of(
            "imported",
            format!(
                "Imported the legacy {provider} login into the unified registry as \
                 `{provider}`. The import moves the login, not its health — \
                 re-authenticate at a terminal: `mecha-mail auth {provider} \
                 --provider {provider}`"
            ),
        )
    } else {
        Outcome::of(
            "failed",
            match child_note {
                Some(note) => format!("The {provider} import made no account — {note}"),
                None => {
                    format!("The {provider} import made no account — see `mecha-mail accounts`")
                }
            },
        )
    }
}

/// The request's state, re-read from the store the CLI wrote.
fn request_state(seq: i64) -> Option<String> {
    let store = mecha_core::frontdoor::Frontdoor::open_default().ok()?;
    store.record(seq).ok().map(|r| r.state)
}

/// The request store answers for close and needs-info: the state as it now
/// stands, never the child's exit.
pub fn mark_outcome(
    seq: i64,
    want: &str,
    state_now: Option<&str>,
    child_note: Option<&str>,
) -> Outcome {
    match state_now {
        Some(state) if state == want => match want {
            mecha_core::frontdoor::NEEDS_INFO => Outcome::of(
                want,
                format!("Request {seq} is parked as `needs_info` — it waits on the requester now"),
            ),
            _ => Outcome::of(want, format!("Request {seq} is `{want}`")),
        },
        Some(state) => Outcome::of(
            "failed",
            match child_note {
                Some(note) => format!("Request {seq} is still `{state}` — {note}"),
                None => {
                    format!("Request {seq} is still `{state}` — see `mecha frontdoor show {seq}`")
                }
            },
        ),
        None => Outcome::of(
            "unknown",
            format!("Request {seq} could not be re-read — see `mecha frontdoor list`"),
        ),
    }
}

/// The probe behind the shared restart guard, off the async loop. The one
/// implementation is `commands::doctor::unit_is_failed` — the same line the
/// TUI's guard runs — wrapped in `spawn_blocking` because `systemctl` on a
/// sick D-Bus can stall, and stalls belong on the blocking pool.
async fn unit_is_failed(unit: &str) -> bool {
    let unit = unit.to_string();
    tokio::task::spawn_blocking(move || crate::commands::doctor::unit_is_failed(&unit))
        .await
        .unwrap_or(false)
}

/// F11: whether one ledger row is the run *this tap* started. Two honest
/// facts carry the attribution: a tapped probe is `mecha trigger run`, which
/// records a **manual** row (no slot — machine-checkable), and the child's
/// clock is this machine's clock, so its row starts at or after the dispatch
/// stamp. A daemon's scheduled row is never manual, so a tap that ran nothing
/// (flock lost, spawn failed) can no longer adopt the daemon's run — the old
/// fixed −5s window did exactly that.
fn row_is_this_taps(row: &RunRecord, name: &str, since: DateTime<Utc>) -> bool {
    row.trigger == name && row.manual && row.started_at >= since
}

/// The newest ledger row this tap's run wrote, whatever its status — a skip
/// is an answer ("the previous run was still going").
fn latest_trigger_row(name: &str, since: DateTime<Utc>) -> Option<RunRecord> {
    latest_trigger_row_in(&TriggerStore::open_existing_default()?, name, since)
}

/// The tail scan behind it: the ledger is append-only, so the first matching
/// row scanning newest-first is the newest, and the scan stops there instead
/// of materializing every `RunRecord` ever written to answer for one tap.
fn latest_trigger_row_in(
    store: &TriggerStore,
    name: &str,
    since: DateTime<Utc>,
) -> Option<RunRecord> {
    let mut found = None;
    let _ = store.scan_runs_rev(|row| {
        if row_is_this_taps(&row, name, since) {
            found = Some(row);
            false
        } else {
            true
        }
    });
    found
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
    fn from_remedy_recognises_exactly_the_three_shapes() {
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
        // Phase 2: the legacy-store import. This shape was in the refusal
        // list until this pass deliberately admitted it — the fails-on-old
        // direction is that it *now* grows a button.
        assert_eq!(
            Action::from_remedy(&remedy(
                &["mecha-mail", "import", "google", "--provider", "google"],
                false
            )),
            Some(Action::MailImport {
                provider: "google".into()
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
            // Not the trusted import shape: a provider outside the closed
            // set, the account and provider disagreeing, extra arguments.
            vec!["mecha-mail", "import", "aol", "--provider", "aol"],
            vec!["mecha-mail", "import", "personal", "--provider", "google"],
            vec!["mecha-mail", "import", "google", "--provider", "outlook"],
            vec![
                "mecha-mail",
                "import",
                "google",
                "--provider",
                "google",
                "--force",
            ],
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
            Action::OutboxSend {
                id: "abc-123".into(),
            },
            Action::OutboxReject {
                id: "abc-123".into(),
            },
            Action::RestartUnit {
                unit: "mecha-triggers.service".into(),
            },
            Action::TriggerRun {
                name: "briefing".into(),
            },
            Action::TriggerCancel {
                name: "briefing".into(),
            },
            Action::TriggerEnable {
                name: "briefing".into(),
            },
            Action::TriggerDisable {
                name: "briefing".into(),
            },
            Action::MailImport {
                provider: "google".into(),
            },
            Action::TaskDone {
                id: "task-1a2b3c4d".into(),
            },
            Action::TaskNext {
                id: "task-1a2b3c4d".into(),
            },
            Action::FrontdoorClose {
                seq: 5,
                reason: "spam".into(),
            },
            Action::FrontdoorNeedsInfo {
                seq: 5,
                question: "which Tuesday?".into(),
            },
        ];
        for action in &samples {
            let argv = action.argv();
            assert!(!argv.is_empty());
            assert!(
                argv[0] == "mecha" || argv[0] == "systemctl" || argv[0] == "mecha-mail",
                "{argv:?} spawns an unexpected program"
            );
            assert!(
                argv.iter().any(|a| *a == action.value()),
                "the object id rides as its own argument: {argv:?}"
            );
        }
    }

    /// The modal text crosses into the argv as **exactly one element**, never
    /// through a shell — a reason full of spaces, quotes, semicolons and a
    /// would-be second command reaches `--reason` as one argument and can
    /// name no second command. This is the provenance comment's enforceable
    /// half.
    #[test]
    fn owner_typed_modal_text_rides_as_a_single_argv_element() {
        let hostile = r#"done"; rm -rf ~; echo "--yes $(cat /etc/passwd)"#;
        let close = Action::FrontdoorClose {
            seq: 9,
            reason: hostile.into(),
        };
        let argv = close.argv();
        assert_eq!(
            argv,
            vec![
                "mecha".to_string(),
                "frontdoor".into(),
                "close".into(),
                "9".into(),
                "--reason".into(),
                hostile.into(),
            ],
            "the text is one element, bytes intact, position fixed"
        );
        assert_eq!(
            argv.iter().filter(|a| a.contains("rm -rf")).count(),
            1,
            "the hostile text exists only inside the one reason argument"
        );

        let park = Action::FrontdoorNeedsInfo {
            seq: 9,
            question: "which Tuesday — this week's, or next?".into(),
        };
        let argv = park.argv();
        assert_eq!(argv[4], "--note");
        assert_eq!(argv[5], "which Tuesday — this week's, or next?");
        assert_eq!(argv.len(), 6);
    }

    #[test]
    fn a_payload_round_trips_through_its_fixed_verb_and_carries_the_id_only() {
        // The value that travelled through Slack is the object id; the argv
        // is re-derived on this side, never parsed from the payload.
        for action in [
            Action::OutboxSend {
                id: "abc-123".into(),
            },
            Action::OutboxReject {
                id: "abc-123".into(),
            },
            Action::RestartUnit {
                unit: "mecha-triggers.service".into(),
            },
            Action::TriggerRun {
                name: "briefing".into(),
            },
            Action::TriggerCancel {
                name: "briefing".into(),
            },
            Action::TriggerEnable {
                name: "briefing".into(),
            },
            Action::TriggerDisable {
                name: "briefing".into(),
            },
            Action::MailImport {
                provider: "outlook".into(),
            },
        ] {
            let back = Action::from_payload(action.action_id(), &action.value());
            assert_eq!(back, Some(action));
        }
        // The confirm verb resolves to the same send action — the two-step's
        // second tap executes nothing new.
        assert_eq!(
            Action::from_payload(ids::OUTBOX_SEND_CONFIRM, "abc-123"),
            Some(Action::OutboxSend {
                id: "abc-123".into()
            })
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
            assert_eq!(
                Action::from_payload(ids::RESTART_UNIT, hostile),
                None,
                "{hostile}"
            );
        }
        for hostile in ["../escape", "a b", "UPPER", ""] {
            assert_eq!(
                Action::from_payload(ids::TRIGGER_RUN, hostile),
                None,
                "{hostile}"
            );
            assert_eq!(
                Action::from_payload(ids::TRIGGER_CANCEL, hostile),
                None,
                "{hostile}"
            );
            assert_eq!(
                Action::from_payload(ids::TRIGGER_ENABLE, hostile),
                None,
                "{hostile}"
            );
            assert_eq!(
                Action::from_payload(ids::TRIGGER_DISABLE, hostile),
                None,
                "{hostile}"
            );
        }
        for hostile in ["", "a b", "x/../y", &"x".repeat(200)] {
            assert_eq!(
                Action::from_payload(ids::OUTBOX_SEND, hostile),
                None,
                "{hostile}"
            );
        }
        // The provider set is closed; anything else — including a legal shell
        // word — is not a legacy store.
        for hostile in ["aol", "google; rm -rf /", "GOOGLE", "google outlook", ""] {
            assert_eq!(
                Action::from_payload(ids::MAIL_IMPORT, hostile),
                None,
                "{hostile}"
            );
        }
        // A task value must be a graph-minted id: the `task-` prefix and an
        // id fragment, nothing shell-shaped, and never a bare word that
        // `mecha tasks set` would take as some other task's name.
        for hostile in [
            "",
            "task-",
            "buy milk",
            "task-1a2b; rm -rf /",
            "task-../escape",
            "task-a b",
        ] {
            assert_eq!(
                Action::from_payload(ids::TASK_DONE, hostile),
                None,
                "{hostile}"
            );
            assert_eq!(
                Action::from_payload(ids::TASK_NEXT, hostile),
                None,
                "{hostile}"
            );
        }
        // The frontdoor verbs are modal callback ids, and a button payload
        // must never construct them: text can only arrive through the gated
        // submission parser.
        assert_eq!(
            Action::from_payload(ids::FRONTDOOR_CLOSE_SUBMIT, "5"),
            None,
            "a button cannot close a request — the reason comes from a modal"
        );
        assert_eq!(
            Action::from_payload(ids::FRONTDOOR_NEEDS_INFO_SUBMIT, "5"),
            None
        );
    }

    /// A board tap round-trips: the id the card carried comes back as the
    /// same typed action, and its argv drives `mecha tasks set` with the
    /// status as a literal — the tap picks a verb, never composes one.
    #[test]
    fn a_task_tap_round_trips_and_its_status_is_a_literal() {
        let done = Action::from_payload(ids::TASK_DONE, "task-1a2b3c4d").unwrap();
        assert_eq!(
            done,
            Action::TaskDone {
                id: "task-1a2b3c4d".into()
            }
        );
        assert_eq!(
            done.argv(),
            vec![
                "mecha".to_string(),
                "tasks".into(),
                "set".into(),
                "task-1a2b3c4d".into(),
                "--status".into(),
                "done".into(),
            ]
        );
        let next = Action::from_payload(ids::TASK_NEXT, "task-1a2b3c4d").unwrap();
        assert_eq!(next.argv()[5], "next");
    }

    /// The read-back answers from the graph's own reply, never the child's
    /// exit: the nested task's status decides, a different status than the
    /// tap asked for reports as what the store holds, and an unreadable
    /// answer is unknown — not done.
    #[test]
    fn a_task_outcome_is_the_stores_word_or_unknown() {
        use serde_json::json;
        let answer = json!({
            "v": 1, "status": "updated",
            "task": { "id": "task-aa", "name": "email Dirk", "status": "done" }
        });
        let out = task_outcome("task-aa", "done", Some(&answer));
        assert_eq!(out.status, "done");
        assert!(out.line.contains("email Dirk"), "{}", out.line);

        // The envelope says "updated" but the task reads otherwise — the
        // store's word wins over the tap's intent.
        let wrong = json!({ "status": "updated", "task": { "status": "inbox" } });
        let out = task_outcome("task-aa", "done", Some(&wrong));
        assert_eq!(out.status, "failed");
        assert!(out.line.contains("`inbox`"), "{}", out.line);

        let out = task_outcome("task-aa", "done", None);
        assert_eq!(out.status, "unknown");
    }

    #[test]
    fn a_submission_round_trips_and_carries_the_owner_text_typed() {
        let close = Action::from_submission(
            ids::FRONTDOOR_CLOSE_SUBMIT,
            "5",
            "  spam, politely declined  ",
        );
        assert_eq!(
            close,
            Some(Action::FrontdoorClose {
                seq: 5,
                reason: "spam, politely declined".into()
            }),
            "trimmed, typed, and nothing else changed"
        );
        let park =
            Action::from_submission(ids::FRONTDOOR_NEEDS_INFO_SUBMIT, "12", "which Tuesday?");
        assert_eq!(
            park,
            Some(Action::FrontdoorNeedsInfo {
                seq: 12,
                question: "which Tuesday?".into()
            })
        );
    }

    #[test]
    fn a_submission_with_a_bad_seq_an_empty_text_or_an_over_cap_text_is_refused() {
        // The seq is machine-authored correlation state, but the parser
        // trusts nothing: it must be a positive integer and only that.
        for bad_seq in ["", "abc", "-1", "0", "5; rm -rf /", "5 6", "1e3"] {
            assert_eq!(
                Action::from_submission(ids::FRONTDOOR_CLOSE_SUBMIT, bad_seq, "a reason"),
                None,
                "{bad_seq:?}"
            );
        }
        // A required reason that is empty after trimming is no reason.
        for empty in ["", "   ", "\n\t"] {
            assert_eq!(
                Action::from_submission(ids::FRONTDOOR_CLOSE_SUBMIT, "5", empty),
                None,
                "{empty:?}"
            );
        }
        // The length cap holds here even though the modal's input element
        // declares the same cap — a client-side cap is a courtesy, not a
        // boundary.
        let over = "x".repeat(MODAL_TEXT_MAX + 1);
        assert_eq!(
            Action::from_submission(ids::FRONTDOOR_CLOSE_SUBMIT, "5", &over),
            None
        );
        let at_cap = "x".repeat(MODAL_TEXT_MAX);
        assert!(Action::from_submission(ids::FRONTDOOR_CLOSE_SUBMIT, "5", &at_cap).is_some());
        // An unknown callback id constructs nothing, exactly like an unknown
        // button verb.
        assert_eq!(
            Action::from_submission("slack_outbox_send", "5", "a reason"),
            None,
            "a message verb is not a modal verb"
        );
        assert_eq!(
            Action::from_submission("anything_else", "5", "a reason"),
            None
        );
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
            filled_defaults: Vec::new(),
            call_id: None,
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
        assert!(
            unknown.line.contains("mecha outbox show abc-123"),
            "{}",
            unknown.line
        );
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
            refailed
                .line
                .contains("journalctl --user -u mecha-triggers.service"),
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
        assert!(
            none.line.contains("mecha trigger runs briefing"),
            "{}",
            none.line
        );
    }

    /// §6's rule for the flag: the trigger file re-read is the outcome. The
    /// signature has no exit code, so a child that died after writing the
    /// flag still reports the flag.
    #[test]
    fn an_enable_or_disable_reports_the_flag_as_it_now_stands() {
        let on = toggle_outcome("briefing", true, Some(true), Some("child was killed"));
        assert_eq!(on.status, "enabled");
        assert!(
            on.line.contains("mecha trigger disable briefing"),
            "{}",
            on.line
        );
        assert!(!on.line.contains("killed"), "{}", on.line);

        let off = toggle_outcome("briefing", false, Some(false), None);
        assert_eq!(off.status, "disabled");
        assert!(
            off.line.contains("mecha trigger enable briefing"),
            "{}",
            off.line
        );

        let unchanged = toggle_outcome("briefing", false, Some(true), Some("store locked"));
        assert_eq!(unchanged.status, "failed");
        assert!(
            unchanged.line.contains("store locked"),
            "{}",
            unchanged.line
        );

        let unknown = toggle_outcome("briefing", true, None, None);
        assert_eq!(unknown.status, "unknown");
        assert!(
            unknown.line.contains("mecha trigger show briefing"),
            "{}",
            unknown.line
        );
    }

    #[test]
    fn an_import_reports_the_registry_and_names_the_reauth_it_cannot_do() {
        let ok = import_outcome("google", true, None);
        assert_eq!(ok.status, "imported");
        // The finding this button rode on was about a *dead* login; the
        // import moves it, and the outcome says what still needs a terminal.
        assert!(
            ok.line.contains("mecha-mail auth google --provider google"),
            "{}",
            ok.line
        );

        let failed = import_outcome("outlook", false, Some("already has credentials"));
        assert_eq!(failed.status, "failed");
        assert!(
            failed.line.contains("already has credentials"),
            "{}",
            failed.line
        );
    }

    /// F11, failing on the old −5s window: a daemon-fired (scheduled) row
    /// that starts right around the tap is never the tap's run — a tap's run
    /// is `mecha trigger run`, which records a manual row, begun at or after
    /// the dispatch stamp. The old rule attributed any recent row.
    #[test]
    fn a_daemon_fired_run_is_never_attributed_to_a_tap() {
        let since = Utc::now();
        let late = since + chrono::Duration::seconds(2);

        // The daemon's row: scheduled (a slot, not manual), started after the
        // tap — inside the old window, refused by the new rule.
        let mut daemon = RunRecord::started("briefing", Some(since), false);
        daemon.started_at = late;
        assert!(
            !row_is_this_taps(&daemon, "briefing", since),
            "a scheduled row is the daemon's evidence, not the tap's"
        );

        // The tap's own run: manual, begun after dispatch.
        let mut manual = RunRecord::started("briefing", None, true);
        manual.started_at = late;
        assert!(row_is_this_taps(&manual, "briefing", since));

        // An earlier manual run (someone at a terminal minutes ago) is not
        // this tap's either — the old −5s allowance let one in.
        let mut earlier = RunRecord::started("briefing", None, true);
        earlier.started_at = since - chrono::Duration::seconds(2);
        assert!(!row_is_this_taps(&earlier, "briefing", since));

        // Another trigger's manual row never answers for this one.
        let mut other = RunRecord::started("nightly", None, true);
        other.started_at = late;
        assert!(!row_is_this_taps(&other, "briefing", since));
    }

    /// The tap's row comes from the ledger *tail*: newest matching row, scan
    /// stopped at it. The fails-on-old proof is the torn old line — invalid
    /// UTF-8 the tail scan never decodes, where the old full parse
    /// (`runs()` → `read_to_string`) dies on the whole file and answered
    /// `None` for a run that plainly happened.
    #[test]
    fn a_taps_row_is_found_from_the_ledger_tail_past_a_torn_old_line() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "mecha-action-tail-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = mecha_core::trigger::TriggerStore::open(&dir).unwrap();

        // A torn, invalid-UTF-8 row from long ago, at the head of the file.
        {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(store.ledger_path())
                .unwrap();
            file.write_all(b"{\"trigger\": \"briefing\xff\xfe\n")
                .unwrap();
        }

        let since = Utc::now();
        // An older manual row (before the tap) and the tap's own — a full
        // parse-then-filter would pick the same newest row; the tail scan
        // proves itself by answering at all.
        let mut before = RunRecord::started("briefing", None, true);
        before.started_at = since - chrono::Duration::seconds(30);
        store.append_run(&before).unwrap();
        let mut mine = RunRecord::started("briefing", None, true);
        mine.started_at = since + chrono::Duration::seconds(2);
        mine.status = RunStatus::Ok;
        store.append_run(&mine).unwrap();

        let row = latest_trigger_row_in(&store, "briefing", since).expect("the tap's row");
        assert_eq!(row.started_at, mine.started_at);
        assert!(row.manual);

        // The contrast the fix rests on, pinned: the full parse cannot even
        // read this ledger.
        assert!(store.runs().is_err());

        // No matching row is still an honest None, not a guess.
        assert!(latest_trigger_row_in(&store, "nightly", since).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// F6's carrier: the confirm value round-trips id and fingerprint, a
    /// changed byte changes the fingerprint, and a pre-fingerprint bare id
    /// still parses (with no fingerprint, so the press re-cards rather than
    /// sends on it).
    #[test]
    fn a_confirm_value_carries_the_id_and_a_fingerprint_of_the_shown_bytes() {
        let args = "{\n  \"to\": \"a@x.org\"\n}";
        let value = confirm_value("abc-123", args);
        let (id, fp) = parse_confirm_value(&value).expect("round trips");
        assert_eq!(id, "abc-123");
        assert_eq!(fp, Some(fingerprint(args).as_str()));

        // One changed byte is a different fingerprint — that is the whole
        // drift detector.
        assert_ne!(
            fingerprint(args),
            fingerprint("{\n  \"to\": \"b@x.org\"\n}")
        );
        // Deterministic across processes: a literal, not DefaultHasher.
        assert_eq!(fingerprint(""), "cbf29ce484222325");

        // A legacy value is a bare id.
        assert_eq!(parse_confirm_value("abc-123"), Some(("abc-123", None)));
        // A hostile id fails the same shape check as every payload value.
        assert_eq!(parse_confirm_value("a b#deadbeef"), None);
        assert_eq!(parse_confirm_value("../x#00"), None);
        assert_eq!(parse_confirm_value(""), None);
    }

    #[test]
    fn a_frontdoor_mark_reports_the_requests_state_never_the_childs_exit() {
        let closed = mark_outcome(5, "closed", Some("closed"), Some("child was killed"));
        assert_eq!(closed.status, "closed");
        assert!(!closed.line.contains("killed"), "{}", closed.line);

        let parked = mark_outcome(5, "needs_info", Some("needs_info"), None);
        assert_eq!(parked.status, "needs_info");
        assert!(parked.line.contains("requester"), "{}", parked.line);

        let stuck = mark_outcome(
            5,
            "closed",
            Some("extracted"),
            Some("no request with seq 5"),
        );
        assert_eq!(stuck.status, "failed");
        assert!(stuck.line.contains("extracted"), "{}", stuck.line);
        assert!(
            stuck.line.contains("no request with seq 5"),
            "{}",
            stuck.line
        );

        let unknown = mark_outcome(5, "closed", None, None);
        assert_eq!(unknown.status, "unknown");
        assert!(
            unknown.line.contains("mecha frontdoor list"),
            "{}",
            unknown.line
        );
    }
}
