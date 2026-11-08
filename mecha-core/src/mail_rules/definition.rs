use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionGroup {
    Mailbox,
    PrivateRecord,
    Report,
    Board,
    FixedWords,
    VerbatimForward,
    TypedFields,
    DraftedProse,
    #[default]
    #[serde(other)]
    Unknown,
}

impl ActionGroup {
    pub fn is_outbound(self) -> bool {
        matches!(
            self,
            Self::FixedWords | Self::VerbatimForward | Self::TypedFields | Self::DraftedProse
        )
    }

    pub fn once_per_thread(self) -> bool {
        self.is_outbound() || matches!(self, Self::Board | Self::Unknown)
    }

    pub fn supported(self) -> bool {
        matches!(
            self,
            Self::Mailbox | Self::PrivateRecord | Self::Report | Self::Board | Self::FixedWords
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionVerb {
    Archive,
    Read,
    Unread,
    Trash,
    Spam,
    Tag,
    Digest,
    Task,
    Reply,
    Forward,
    Send,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Money,
    Number,
    Date,
    Email,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Matcher {
    pub from: Vec<String>,
    pub from_domain: Vec<String>,
    pub to: Vec<String>,
    pub account: Vec<String>,
    pub list_id: Vec<String>,
    pub subject_contains: Vec<String>,
    pub has_attachment: Option<bool>,
    pub header: BTreeMap<String, Vec<String>>,
    pub describe: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    #[serde(rename = "do")]
    pub verb: ActionVerb,
    #[serde(default)]
    pub to: Vec<String>,
    pub template: Option<String>,
    pub note: Option<String>,
    pub draft_with: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldType>,
    pub tag: Option<String>,
}

impl Action {
    pub fn group(&self) -> ActionGroup {
        match self.verb {
            ActionVerb::Archive | ActionVerb::Read | ActionVerb::Unread | ActionVerb::Trash => {
                ActionGroup::Mailbox
            }
            ActionVerb::Spam => ActionGroup::Report,
            ActionVerb::Tag | ActionVerb::Digest => ActionGroup::PrivateRecord,
            ActionVerb::Task => ActionGroup::Board,
            ActionVerb::Reply | ActionVerb::Send if self.draft_with.is_some() => {
                ActionGroup::DraftedProse
            }
            ActionVerb::Reply => ActionGroup::FixedWords,
            ActionVerb::Forward => ActionGroup::VerbatimForward,
            ActionVerb::Send => ActionGroup::TypedFields,
            ActionVerb::Unknown => ActionGroup::Unknown,
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(self.verb != ActionVerb::Unknown, "unknown action verb");
        let outbound = self.group().is_outbound();
        ensure!(
            outbound
                || (self.template.is_none()
                    && self.note.is_none()
                    && self.draft_with.is_none()
                    && self.to.is_empty()
                    && self.fields.is_empty()),
            "mailbox/record/Board action has outbound parameters"
        );
        ensure!(
            self.draft_with.is_none() || self.template.is_none(),
            "draft_with and template are mutually exclusive"
        );
        ensure!(
            self.note.is_none() || self.verb == ActionVerb::Forward,
            "note belongs to forward"
        );
        ensure!(
            self.fields.is_empty() || self.group() == ActionGroup::TypedFields,
            "fields belong to a typed send"
        );
        ensure!(
            self.tag.is_none() || self.verb == ActionVerb::Tag,
            "tag belongs to tag"
        );
        match self.verb {
            ActionVerb::Reply => {
                ensure!(
                    self.to.is_empty(),
                    "reply takes its recipient from the authenticated sender"
                );
                ensure!(
                    self.template.is_some() || self.draft_with.is_some(),
                    "reply requires template or draft_with"
                );
            }
            ActionVerb::Forward => {
                ensure!(!self.to.is_empty(), "forward requires to");
                ensure!(
                    self.template.is_none() && self.draft_with.is_none(),
                    "forward is verbatim"
                );
            }
            ActionVerb::Send => {
                ensure!(!self.to.is_empty(), "send requires to");
                ensure!(
                    self.draft_with.is_some()
                        || (self.template.is_some() && !self.fields.is_empty()),
                    "send requires draft_with or template and fields"
                );
            }
            ActionVerb::Tag => ensure!(
                self.tag.as_ref().is_some_and(|s| !s.trim().is_empty()),
                "tag requires a tag name"
            ),
            _ => {}
        }
        if let Some(name) = &self.draft_with {
            crate::skill::validate_name(name)?;
        }
        Ok(())
    }
}

fn default_priority() -> u32 {
    100
}
fn rule_cap() -> u32 {
    20
}
fn global_cap() -> u32 {
    50
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDefinition {
    pub description: String,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "rule_cap")]
    pub daily_cap: u32,
    #[serde(rename = "match")]
    pub matcher: Matcher,
    #[serde(rename = "action")]
    pub actions: Vec<Action>,
}

impl RuleDefinition {
    pub fn parse(text: &str) -> Result<Self> {
        let rule: Self = toml::from_str(text)?;
        ensure!(!rule.actions.is_empty(), "a rule needs an action");
        if let Some(describe) = &rule.matcher.describe {
            ensure!(
                !describe.trim().is_empty(),
                "match.describe must say what mail it is for"
            );
            ensure!(
                describe.chars().count() <= super::describe::DESCRIBE_MAX_CHARS,
                "match.describe is limited to {} characters; it rides in every classification",
                super::describe::DESCRIBE_MAX_CHARS
            );
        }
        for (header, values) in &rule.matcher.header {
            ensure!(
                !values.is_empty(),
                "match.header.{header} must contain at least one value"
            );
        }
        for (index, action) in rule.actions.iter().enumerate() {
            action.validate()?;
            ensure!(
                !rule.actions[..index].contains(action),
                "duplicate action in effect"
            );
        }
        // Two replies would stage two drafts to one sender for one message,
        // which no rule means (review of #856); a reply beside a forward or
        // a send is §4.3's one rule answering through its slot.
        ensure!(
            rule.actions
                .iter()
                .filter(|a| a.group() == ActionGroup::FixedWords)
                .count()
                <= 1,
            "a rule replies at most once per message; keep one reply"
        );
        Ok(rule)
    }

    pub fn groups(&self) -> BTreeSet<ActionGroup> {
        self.actions.iter().map(Action::group).collect()
    }

    pub fn is_description(&self) -> bool {
        self.matcher.describe.is_some()
    }

    pub fn uses_limits(&self) -> bool {
        self.groups()
            .iter()
            .any(|g| g.is_outbound() || *g == ActionGroup::Board)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Aliases {
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "global_cap")]
    pub daily_cap: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            daily_cap: global_cap(),
        }
    }
}

/// The verb stays readable across builds; the digest covers the action's own
/// TOML table, never its index or the contents of another action.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionIdentity {
    #[serde(default, deserialize_with = "super::ledger::wire")]
    pub verb: ActionVerb,
    #[serde(default)]
    pub digest: String,
}

#[derive(Debug, Clone)]
pub struct LoadedRule {
    pub name: String,
    pub definition: RuleDefinition,
    pub actions: Vec<ActionIdentity>,
    pub directory: PathBuf,
    pub files: BTreeMap<String, Vec<u8>>,
    pub approval_digest: ApprovalDigest,
}

/// Empty means absent, not a version supplied by the reading build.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalDigest {
    pub directory: String,
    pub skills: BTreeMap<String, String>,
    pub aliases: String,
    pub limits: String,
    pub confirmation_version: String,
    pub extraction_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchDigest {
    pub matcher: String,
    pub priority: String,
    pub origin_gate: String,
    pub confirmation_version: String,
}

impl LoadedRule {
    /// The confirmation version is written, empty when absent: only a
    /// description rule's confirmed groups carry it. Gate status is supplied
    /// mechanically by the provenance reader, never inferred as clean.
    pub fn match_digest(&self, group: ActionGroup, gate: super::GateStatus) -> Result<MatchDigest> {
        Ok(MatchDigest {
            matcher: hash(&serde_json::to_vec(&self.definition.matcher)?),
            priority: self.definition.priority.to_string(),
            origin_gate: if group.once_per_thread() {
                gate.as_str().into()
            } else {
                String::new()
            },
            // The compiled confirmation frame decides this group (§4.2): a
            // change to it starts new measurement for exactly these groups.
            confirmation_version: if self.definition.is_description()
                && super::describe::confirmation_decides(group)
                && gate == super::GateStatus::Passing
            {
                crate::mail_triage::confirm::CONFIRMATION_VERSION.clone()
            } else {
                String::new()
            },
        })
    }
}

pub(super) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
