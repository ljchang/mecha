//! Who may drive this agent, and where that answer is kept.
//!
//! Slack is a multi-party medium and mecha has one principal, so this module is
//! the whole reconciliation: **an allowlist of Slack user ids, and everyone
//! else is not a lesser user but a stranger** — handled, if at all, by the
//! front door rather than by a weaker permission here. There is deliberately no
//! middle tier; see `docs/SLACK-DESIGN.md` §3.
//!
//! Three decisions are worth not undoing:
//!
//! - **The binding is a store, never config.** `[[hook]]`, `[[mcp]]` and
//!   `[[subagent]]` can all be declared in a project's `mecha.toml`, which is a
//!   file that arrives with a cloned repository. Triggers already live outside
//!   the layered config because a repo that could declare one has been handed a
//!   cron slot; a repo that could declare a Slack owner has been handed the
//!   remote control, which is strictly worse and gets the same treatment.
//! - **Binding is proved by a nonce the local CLI prints**, not by an email
//!   address. `users:read.email` returns the *workspace's claim* about an
//!   address, which a workspace admin can change; typing a nonce that was
//!   printed on this machine proves shell access to the machine the agent runs
//!   on, which is the claim that actually matters.
//! - **The gate says why it refused.** A boolean would make "ignored because
//!   you are not an owner" and "ignored because this is the wrong workspace"
//!   indistinguishable, and a state nobody can explain is the failure this
//!   design keeps trying to avoid.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{SlackError, SlackResult};

/// The two tokens. The bot token acts in the workspace; the app-level token
/// only opens sockets and is not workspace-scoped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub bot_token: String,
    pub app_token: String,
}

/// Who is bound, and to which workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub team_id: String,
    #[serde(default)]
    pub enterprise_id: Option<String>,
    pub owners: Vec<String>,
    pub bound_at: DateTime<Utc>,
}

/// Why an inbound event was or was not honoured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Allowed,
    /// Nothing is bound yet; `mecha slack link` has not been run.
    Unbound,
    /// The event came from a workspace this install is not bound to.
    WrongWorkspace {
        saw: Option<String>,
    },
    /// A real person, and not one of ours. This is the ordinary case in a
    /// shared channel, and it is not an error.
    NotAnOwner {
        saw: Option<String>,
    },
}

impl Gate {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Gate::Allowed)
    }

    /// One line for a log. Never posted back to Slack: telling a stranger why
    /// they were ignored tells them an agent is listening.
    pub fn reason(&self) -> String {
        match self {
            Gate::Allowed => "owner".into(),
            Gate::Unbound => "no binding exists".into(),
            Gate::WrongWorkspace { saw } => {
                format!(
                    "workspace {} is not the bound one",
                    saw.as_deref().unwrap_or("<none>")
                )
            }
            Gate::NotAnOwner { saw } => {
                format!(
                    "user {} is not an owner",
                    saw.as_deref().unwrap_or("<none>")
                )
            }
        }
    }
}

impl Binding {
    /// The check that runs on **every** inbound event, before a message can
    /// become a prompt and before any button is honoured.
    ///
    /// Fails closed in both directions: a missing user id and a missing team id
    /// are both refusals, because an event that cannot say who sent it is not
    /// evidence that the owner did.
    pub fn gate(&self, user_id: Option<&str>, team_id: Option<&str>) -> Gate {
        let Some(team) = team_id else {
            return Gate::WrongWorkspace { saw: None };
        };
        let matches_workspace = team == self.team_id || self.enterprise_id.as_deref() == Some(team);
        if !matches_workspace {
            return Gate::WrongWorkspace {
                saw: Some(team.to_string()),
            };
        }
        match user_id {
            Some(user) if self.owners.iter().any(|o| o == user) => Gate::Allowed,
            other => Gate::NotAnOwner {
                saw: other.map(str::to_owned),
            },
        }
    }
}

/// The gate, over a binding that may not exist yet.
///
/// This is the entry point a connector calls, and it exists so that "nothing is
/// bound" is a refusal with a name rather than an `unwrap` or an early return
/// somewhere in an event loop. An install with no binding answers nobody.
pub fn check(binding: Option<&Binding>, user_id: Option<&str>, team_id: Option<&str>) -> Gate {
    match binding {
        None => Gate::Unbound,
        Some(b) => b.gate(user_id, team_id),
    }
}

/// A one-time code, printed on the workstation and typed into Slack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLink {
    pub nonce: String,
    pub expires_at: DateTime<Utc>,
}

/// Unambiguous characters only — someone is reading this off a terminal and
/// typing it into a phone, and `0`/`O` and `1`/`l` are where that goes wrong.
const NONCE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const NONCE_LEN: usize = 10;

impl PendingLink {
    pub fn mint(ttl: Duration) -> Self {
        Self {
            nonce: mint_nonce(),
            expires_at: Utc::now() + ttl,
        }
    }

    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        now < self.expires_at
    }

    /// Whether some text a user sent contains this nonce.
    ///
    /// Compared in constant time. The realistic threat is small — a ten-minute
    /// single-use code over a medium we do not control the timing of — but the
    /// comparison is three lines and the habit is worth more than the analysis.
    pub fn matches(&self, typed: &str) -> bool {
        let candidate: String = typed
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        // The user may have typed a sentence around it; check any window of the
        // right length rather than demanding the message be only the code.
        let needle = self.nonce.as_bytes();
        candidate
            .as_bytes()
            .windows(needle.len())
            .any(|w| constant_time_eq(w, needle))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn mint_nonce() -> String {
    // `uuid`'s v4 is backed by the OS random source, which is the property that
    // matters here; the formatting is just to make it typable.
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    bytes
        .iter()
        .take(NONCE_LEN)
        .map(|b| NONCE_ALPHABET[(*b as usize) % NONCE_ALPHABET.len()] as char)
        .collect()
}

/// The owner-only directory holding the tokens, the binding, and a pending
/// link. The caller supplies the root, so this crate never has to know where a
/// mecha home is — which is also what makes every test here run in a temp dir.
pub struct SlackStore {
    root: PathBuf,
}

impl SlackStore {
    pub fn open(root: impl Into<PathBuf>) -> SlackResult<Self> {
        let root = root.into();
        create_private_dir(&root).map_err(|e| store_error("creating the slack directory", e))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn credentials(&self) -> SlackResult<Option<Credentials>> {
        self.read("credentials.json")
    }

    pub fn save_credentials(&self, credentials: &Credentials) -> SlackResult<()> {
        self.write("credentials.json", credentials)
    }

    pub fn binding(&self) -> SlackResult<Option<Binding>> {
        self.read("binding.json")
    }

    pub fn save_binding(&self, binding: &Binding) -> SlackResult<()> {
        self.write("binding.json", binding)
    }

    pub fn clear_binding(&self) -> SlackResult<()> {
        self.remove("binding.json")
    }

    pub fn pending_link(&self) -> SlackResult<Option<PendingLink>> {
        self.read("pending-link.json")
    }

    pub fn save_pending_link(&self, link: &PendingLink) -> SlackResult<()> {
        self.write("pending-link.json", link)
    }

    pub fn clear_pending_link(&self) -> SlackResult<()> {
        self.remove("pending-link.json")
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> SlackResult<Option<T>> {
        let path = self.root.join(name);
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|e| store_error(&format!("reading {name}"), e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(store_error(&format!("reading {name}"), e)),
        }
    }

    /// Temp sibling then rename, like every other store under `~/.mecha`: a
    /// crash halfway through a write must not leave a half-parsed binding,
    /// because a binding that fails to load is a remote control that silently
    /// stops answering.
    fn write<T: Serialize>(&self, name: &str, value: &T) -> SlackResult<()> {
        let body = serde_json::to_string_pretty(value)
            .map_err(|e| store_error(&format!("encoding {name}"), e))?;
        let tmp = self.root.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, body.as_bytes())
            .map_err(|e| store_error(&format!("writing {name}"), e))?;
        set_owner_only(&tmp).map_err(|e| store_error(&format!("securing {name}"), e))?;
        std::fs::rename(&tmp, self.root.join(name))
            .map_err(|e| store_error(&format!("installing {name}"), e))
    }

    fn remove(&self, name: &str) -> SlackResult<()> {
        match std::fs::remove_file(self.root.join(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(store_error(&format!("removing {name}"), e)),
        }
    }
}

fn store_error(doing: &str, e: impl std::fmt::Display) -> SlackError {
    SlackError::Store(format!("{doing}: {e}"))
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_owner_only(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> Binding {
        Binding {
            team_id: "T1".into(),
            enterprise_id: None,
            owners: vec!["U_OWNER".into()],
            bound_at: Utc::now(),
        }
    }

    #[test]
    fn the_owner_is_let_through_and_nobody_else_is() {
        let b = binding();
        assert_eq!(b.gate(Some("U_OWNER"), Some("T1")), Gate::Allowed);
        assert!(matches!(
            b.gate(Some("U_STRANGER"), Some("T1")),
            Gate::NotAnOwner { .. }
        ));
    }

    #[test]
    fn an_event_that_cannot_say_who_sent_it_is_refused() {
        // Fail closed in both directions: absence of evidence is not evidence
        // that the owner did it.
        let b = binding();
        assert!(matches!(b.gate(None, Some("T1")), Gate::NotAnOwner { .. }));
        assert!(matches!(
            b.gate(Some("U_OWNER"), None),
            Gate::WrongWorkspace { .. }
        ));
    }

    #[test]
    fn the_right_user_in_the_wrong_workspace_is_still_refused() {
        // A distributed install could deliver events from somewhere else, and
        // user ids are not globally unique across workspaces.
        let b = binding();
        assert!(matches!(
            b.gate(Some("U_OWNER"), Some("T_OTHER")),
            Gate::WrongWorkspace { .. }
        ));
    }

    #[test]
    fn an_org_install_matches_on_the_enterprise_id() {
        let b = Binding {
            enterprise_id: Some("E1".into()),
            ..binding()
        };
        assert_eq!(b.gate(Some("U_OWNER"), Some("E1")), Gate::Allowed);
        assert_eq!(b.gate(Some("U_OWNER"), Some("T1")), Gate::Allowed);
    }

    #[test]
    fn a_refusal_explains_itself() {
        let b = binding();
        assert!(b
            .gate(Some("U_X"), Some("T1"))
            .reason()
            .contains("not an owner"));
        assert!(b
            .gate(Some("U_OWNER"), Some("T_OTHER"))
            .reason()
            .contains("not the bound one"));
    }

    #[test]
    fn an_unbound_install_answers_nobody() {
        // Including the person who will become the owner five minutes later.
        assert_eq!(check(None, Some("U_OWNER"), Some("T1")), Gate::Unbound);
        assert!(!check(None, Some("U_OWNER"), Some("T1")).is_allowed());
        let b = binding();
        assert_eq!(check(Some(&b), Some("U_OWNER"), Some("T1")), Gate::Allowed);
    }

    #[test]
    fn a_nonce_is_typable_and_unambiguous() {
        let n = mint_nonce();
        assert_eq!(n.chars().count(), NONCE_LEN);
        for c in n.chars() {
            assert!(
                !"01OIl".contains(c),
                "{c} is the character someone mistypes"
            );
            assert!(NONCE_ALPHABET.contains(&(c as u8)), "{c}");
        }
    }

    #[test]
    fn nonces_differ() {
        let a = mint_nonce();
        let b = mint_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn a_nonce_is_found_inside_a_sentence_and_is_case_insensitive() {
        let link = PendingLink {
            nonce: "ABCDEFGHJK".into(),
            expires_at: Utc::now() + Duration::minutes(10),
        };
        assert!(link.matches("ABCDEFGHJK"));
        assert!(link.matches("my code is abcdefghjk thanks"));
        assert!(link.matches("  ABCDEFGHJK  "));
        assert!(!link.matches("ABCDEFGHJX"));
        assert!(!link.matches("nope"));
    }

    #[test]
    fn a_nonce_expires() {
        let link = PendingLink {
            nonce: "ABCDEFGHJK".into(),
            expires_at: Utc::now() - Duration::seconds(1),
        };
        assert!(!link.is_live(Utc::now()));
        assert!(PendingLink::mint(Duration::minutes(10)).is_live(Utc::now()));
    }

    #[test]
    fn constant_time_compare_still_compares() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn the_store_round_trips_and_absence_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("mecha-slack-test-{}", uuid::Uuid::new_v4()));
        let store = SlackStore::open(&dir).unwrap();

        assert!(store.binding().unwrap().is_none(), "nothing bound yet");
        assert!(store.credentials().unwrap().is_none());

        let b = binding();
        store.save_binding(&b).unwrap();
        let read = store.binding().unwrap().expect("saved");
        assert_eq!(read.owners, b.owners);
        assert_eq!(read.team_id, b.team_id);

        store.clear_binding().unwrap();
        assert!(store.binding().unwrap().is_none());
        // Clearing something that is already gone is not an error.
        store.clear_binding().unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn secrets_are_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("mecha-slack-perm-{}", uuid::Uuid::new_v4()));
        let store = SlackStore::open(&dir).unwrap();
        store
            .save_credentials(&Credentials {
                bot_token: "xoxb-1".into(),
                app_token: "xapp-1".into(),
            })
            .unwrap();

        let mode = std::fs::metadata(dir.join("credentials.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "tokens must not be group- or world-readable");

        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);

        std::fs::remove_dir_all(&dir).ok();
    }
}
