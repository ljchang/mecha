//! A reference to something the agent is working toward.
//!
//! Nothing in mecha said what a run was *for*. `grep -i goal` over the crate
//! returned four hits before this module, all of them incidental prose, and
//! the consequence was structural rather than cosmetic: every evaluative
//! signal the system has is either a person intervening or a counter crossing
//! a threshold, so a run can be recorded as having gone badly and never as
//! having gone well. `docs/GOAL-SYSTEM-DESIGN.md` is the argument; this is the
//! reference type the rest of it is threaded on.
//!
//! **It is a pointer, not a copy.** A `Task` names a board task the knowledge
//! graph owns, exactly as `kg_task_create` requires of its callers, and this
//! module deliberately holds no title, status or due date. A second copy of
//! somebody else's record is the thing that can disagree with it.
//!
//! **Three kinds, because there are three horizons** — a standing commitment,
//! a current concern, a homeostatic setpoint. Only `Task` has a store behind
//! it today; the other two are named here because the wire format below has to
//! survive their arrival, and because a reference whose kinds are invented one
//! at a time acquires a fourth spelling of the same idea.
//!
//! ## The wire format, and why parsing has two policies
//!
//! A ref renders as `kind:id` — `task:01J8ZK…`. A flat string rather than a
//! nested object because the *model* writes this: it is one field in a tool
//! schema, and `malformed_tool_args` is a metric the harness grades models on.
//! One string is harder to get wrong than `{"kind": …, "id": …}`.
//!
//! Reading one back has two directions and they get opposite treatment,
//! following the rule `OutboxKind` and `Proposed` already set:
//!
//! - **From the model**, a malformed ref is an error reported back through
//!   `ToolOutput`, because the model can fix it and silently dropping the
//!   field would leave a plan claiming to serve nothing.
//! - **From a record** — a transcript, a carried-state block — an unknown kind
//!   degrades to *no reference*, never to a failed parse. Those are
//!   append-only and may have been written by a newer binary; a strict reader
//!   there would make one unrecognised word discard a whole plan.

use std::fmt;
use std::str::FromStr;

/// What a piece of work serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRef {
    /// A standing commitment from the charter. No store yet.
    ///
    /// **Rank is the charter's line order, and is deliberately not carried
    /// here.** Standing commitments conflict — protecting the owner against
    /// not letting a colleague down — and value conflict is the measured cause
    /// of goal drift, so the resolution has to be a total order rather than
    /// weights: no quantity of a lower commitment outranks a higher one, which
    /// is what makes *"this is urgent for very many people"* a non-argument.
    /// Order in the file is that order, on `TASK-AGENT-DESIGN.md` R1's rule
    /// one noun over — priority derives from the record and is never a field
    /// anybody maintains, because a second statement of urgency disagrees with
    /// the first the moment either is edited.
    Charter(String),
    /// A task on the GTD board, by the graph's own uid.
    Task(String),
    /// A homeostatic setpoint, by name. No store yet.
    Setpoint(String),
}

impl GoalRef {
    /// The kind word used on the wire.
    pub fn kind(&self) -> &'static str {
        match self {
            GoalRef::Charter(_) => "charter",
            GoalRef::Task(_) => "task",
            GoalRef::Setpoint(_) => "setpoint",
        }
    }

    /// The identifier this points at, without its kind.
    pub fn id(&self) -> &str {
        match self {
            GoalRef::Charter(id) | GoalRef::Task(id) | GoalRef::Setpoint(id) => id,
        }
    }

    /// Parse leniently: anything unrecognised is *no reference*.
    ///
    /// The reader for records. See the module note — a transcript written by a
    /// newer binary must not cost the plan that surrounds it.
    pub fn parse_lenient(s: &str) -> Option<GoalRef> {
        s.parse().ok()
    }
}

/// Serialised as the same `kind:id` string the model writes, never as an
/// object.
///
/// One spelling on every wire this type crosses. A derived impl would give a
/// stored record a second shape — `{"Task": "01J8ZK"}` — and then "what does a
/// goal reference look like" would have two answers depending on which file
/// you opened, which is how a reader written against one silently mis-reads
/// the other.
impl serde::Serialize for GoalRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// Read an optional reference **out of a record**, leniently.
///
/// The record half of the module's two policies, as a function so it is
/// decided once. A derived `Deserialize` could not express it: the lenient
/// answer to an unknown kind is *no reference*, and a `Deserialize for
/// GoalRef` must produce a `GoalRef` or fail the whole record. Reaching this
/// through `Option` is what lets one unrecognised word cost the reference and
/// nothing around it.
pub fn de_lenient<'de, D>(d: D) -> Result<Option<GoalRef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(Option::<String>::deserialize(d)?
        .as_deref()
        .and_then(GoalRef::parse_lenient))
}

/// The same, for a list. An unrecognised entry is dropped and its neighbours
/// survive.
pub fn de_lenient_vec<'de, D>(d: D) -> Result<Vec<GoalRef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(Vec::<String>::deserialize(d)?
        .iter()
        .filter_map(|s| GoalRef::parse_lenient(s))
        .collect())
}

impl fmt::Display for GoalRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind(), self.id())
    }
}

/// Why a string was not a goal reference, phrased for whoever wrote it — which
/// is usually a model reading the message back out of a `ToolOutput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseGoalRefError(String);

impl fmt::Display for ParseGoalRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseGoalRefError {}

impl FromStr for GoalRef {
    type Err = ParseGoalRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        // `split_once` and not `split(':')`: an id may contain a colon, and
        // only the first one is the separator.
        let Some((kind, id)) = s.split_once(':') else {
            return Err(ParseGoalRefError(format!(
                "`{s}` is not a goal reference; expected `task:<id>`"
            )));
        };
        let id = id.trim();
        if id.is_empty() {
            return Err(ParseGoalRefError(format!(
                "`{s}` names a kind with no identifier"
            )));
        }
        match kind.trim() {
            "charter" => Ok(GoalRef::Charter(id.to_string())),
            "task" => Ok(GoalRef::Task(id.to_string())),
            "setpoint" => Ok(GoalRef::Setpoint(id.to_string())),
            other => Err(ParseGoalRefError(format!(
                "`{other}` is not a kind of goal; expected charter, task or setpoint"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_round_trips_through_its_wire_form() {
        for original in [
            GoalRef::Task("01J8ZK".into()),
            GoalRef::Charter("do-no-harm".into()),
            GoalRef::Setpoint("attention-debt".into()),
        ] {
            let rendered = original.to_string();
            assert_eq!(rendered.parse::<GoalRef>().unwrap(), original);
        }
    }

    #[test]
    fn an_id_may_contain_a_colon_because_only_the_first_one_separates() {
        let r: GoalRef = "task:urn:uid:7".parse().unwrap();
        assert_eq!(r, GoalRef::Task("urn:uid:7".into()));
        assert_eq!(r.to_string(), "task:urn:uid:7");
    }

    /// The model-facing direction: a malformed reference is an error with a
    /// message, because the model can fix it on the next call.
    #[test]
    fn a_malformed_reference_says_what_was_wrong() {
        let no_colon = "notes.md".parse::<GoalRef>().unwrap_err().to_string();
        assert!(no_colon.contains("not a goal reference"), "{no_colon}");

        let bad_kind = "banana:7".parse::<GoalRef>().unwrap_err().to_string();
        assert!(bad_kind.contains("not a kind of goal"), "{bad_kind}");

        let no_id = "task:".parse::<GoalRef>().unwrap_err().to_string();
        assert!(no_id.contains("no identifier"), "{no_id}");
    }

    /// The record-facing direction: the same inputs are simply absent. A
    /// transcript written by a newer binary naming a kind this one has never
    /// heard of must cost the reference and nothing else.
    #[test]
    fn a_record_with_an_unknown_kind_degrades_to_no_reference() {
        assert_eq!(GoalRef::parse_lenient("epic:7"), None);
        assert_eq!(GoalRef::parse_lenient("notes.md"), None);
        assert_eq!(GoalRef::parse_lenient("task:"), None);
        assert_eq!(
            GoalRef::parse_lenient("task:7"),
            Some(GoalRef::Task("7".into()))
        );
    }

    #[test]
    fn surrounding_whitespace_is_not_part_of_the_identifier() {
        assert_eq!(
            "  task: 01J8ZK  ".parse::<GoalRef>().unwrap(),
            GoalRef::Task("01J8ZK".into())
        );
    }
}
