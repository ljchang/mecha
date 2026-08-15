//! What arrives over the socket, and the typed shape the caller gets instead
//! of raw JSON.
//!
//! Two things here are load-bearing beyond the parsing:
//!
//! - **Every event carries an `event_id`**, exposed so the caller can dedupe.
//!   Whether Slack replays unacked envelopes across a dropped connection is
//!   undocumented, so handlers have to be idempotent and this is what lets them
//!   be. See `docs/SLACK-RESEARCH.md` §12.
//! - **A disconnect says why**, and one of the reasons means "open the next
//!   connection before closing this one". Losing that distinction turns a
//!   routine refresh into a window where an event has nowhere to land.

use serde::Deserialize;
use serde_json::Value;

/// The raw Socket Mode frame.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub envelope_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub retry_attempt: Option<u32>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Why Slack is closing a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Routine: Slack rotates connections every few hours.
    RefreshRequested,
    /// About ten seconds' notice before a close.
    Warning,
    /// The app's socket mode was turned off. Reconnecting will not help.
    LinkDisabled,
    Other(String),
}

impl DisconnectReason {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "refresh_requested" => DisconnectReason::RefreshRequested,
            "warning" => DisconnectReason::Warning,
            "link_disabled" => DisconnectReason::LinkDisabled,
            other => DisconnectReason::Other(other.to_string()),
        }
    }

    /// Whether the right response is to open the next connection *before*
    /// closing this one. `link_disabled` is deliberately excluded: reconnecting
    /// into a disabled app is a retry loop against a configuration error.
    pub fn wants_reconnect(&self) -> bool {
        !matches!(self, DisconnectReason::LinkDisabled)
    }
}

/// A file a user attached.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FileRef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mimetype: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url_private: Option<String>,
}

/// An Events API event, flattened to the fields a remote control needs.
#[derive(Debug, Clone, Default)]
pub struct SlackEvent {
    /// `message`, `app_mention`, `app_home_opened`, `file_shared`, …
    pub kind: String,
    /// Slack's own id for this delivery. Dedupe on it.
    pub event_id: String,
    pub team_id: Option<String>,
    pub user: Option<String>,
    pub channel: Option<String>,
    /// `im`, `channel`, `group`, `mpim`.
    pub channel_type: Option<String>,
    pub text: Option<String>,
    pub ts: Option<String>,
    pub thread_ts: Option<String>,
    pub subtype: Option<String>,
    /// Present when the message came from an app rather than a person.
    pub bot_id: Option<String>,
    pub files: Vec<FileRef>,
}

impl SlackEvent {
    /// Whether this is something a person typed.
    ///
    /// Slack's own security guidance says to refuse messages from other bots
    /// and automated systems, and the connector needs it for a second reason:
    /// its own posts arrive back as events, so without this a reply is an
    /// input and the loop never ends.
    pub fn is_from_a_human(&self) -> bool {
        self.bot_id.is_none()
            && self.user.is_some()
            && !matches!(self.subtype.as_deref(), Some("bot_message"))
    }

    /// The thread this belongs to. A top-level message starts a thread whose
    /// id is its own `ts`, which is what makes "a thread is a conversation"
    /// expressible without a separate identifier.
    pub fn thread_key(&self) -> Option<String> {
        self.thread_ts.clone().or_else(|| self.ts.clone())
    }
}

/// A button press or a modal submission.
#[derive(Debug, Clone, Default)]
pub struct Interaction {
    pub kind: String,
    /// **The only field that authorises anything.** Never gate on an action's
    /// `value`, which is a correlation id chosen by whatever composed the
    /// message — and never on a view's `private_metadata`, which is the same
    /// thing in a different pocket.
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub channel_id: Option<String>,
    pub message_ts: Option<String>,
    pub thread_ts: Option<String>,
    /// Expires in three seconds and may be used once. Open a modal with it
    /// before doing any other work.
    pub trigger_id: Option<String>,
    pub response_url: Option<String>,
    pub actions: Vec<ActionRef>,
    /// Present when this is a `view_submission`: what the modal was and what
    /// was typed into it. A submission carries no channel or container, so the
    /// caller's correlation state rides in `private_metadata`.
    pub view: Option<ViewRef>,
}

/// A submitted modal, flattened to what a caller needs: which modal
/// (`callback_id`), the opaque state its composer stashed
/// (`private_metadata`), and each input's value keyed by its `action_id`.
///
/// The values are what a person typed into the modal — the caller decides
/// what, if anything, they authorise; this crate only carries them.
#[derive(Debug, Clone, Default)]
pub struct ViewRef {
    pub callback_id: String,
    pub private_metadata: String,
    /// `action_id` → typed value, for every filled `plain_text_input`.
    pub values: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ActionRef {
    #[serde(default)]
    pub action_id: String,
    #[serde(default)]
    pub value: Option<String>,
}

/// What the caller sees.
#[derive(Debug, Clone)]
pub enum Inbound {
    Hello,
    Disconnect(DisconnectReason),
    Event {
        envelope_id: String,
        event: Box<SlackEvent>,
    },
    Interactive {
        envelope_id: String,
        interaction: Box<Interaction>,
    },
    /// A frame type this crate does not model. Acked and ignored — but named,
    /// so an unhandled subscription is visible in a log rather than silent.
    Other {
        envelope_id: Option<String>,
        kind: String,
    },
}

impl Inbound {
    pub fn envelope_id(&self) -> Option<&str> {
        match self {
            Inbound::Event { envelope_id, .. } | Inbound::Interactive { envelope_id, .. } => {
                Some(envelope_id)
            }
            Inbound::Other { envelope_id, .. } => envelope_id.as_deref(),
            _ => None,
        }
    }
}

/// Interpret one frame. Pure, so the whole mapping is testable without a
/// socket — which matters because the shapes below are the part most likely to
/// drift when Slack moves the surface again.
pub fn interpret(envelope: &Envelope) -> Inbound {
    match envelope.kind.as_str() {
        "hello" => Inbound::Hello,
        "disconnect" => Inbound::Disconnect(DisconnectReason::parse(
            envelope.reason.as_deref().unwrap_or("unknown"),
        )),
        "events_api" => {
            let id = envelope.envelope_id.clone().unwrap_or_default();
            Inbound::Event {
                envelope_id: id,
                event: Box::new(parse_event(&envelope.payload)),
            }
        }
        "interactive" => {
            let id = envelope.envelope_id.clone().unwrap_or_default();
            Inbound::Interactive {
                envelope_id: id,
                interaction: Box::new(parse_interaction(&envelope.payload)),
            }
        }
        other => Inbound::Other {
            envelope_id: envelope.envelope_id.clone(),
            kind: other.to_string(),
        },
    }
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn parse_event(payload: &Value) -> SlackEvent {
    let inner = payload.get("event").unwrap_or(&Value::Null);
    let files = inner
        .get("files")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|f| serde_json::from_value::<FileRef>(f.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    SlackEvent {
        kind: str_at(inner, "type").unwrap_or_default(),
        event_id: str_at(payload, "event_id").unwrap_or_default(),
        team_id: str_at(payload, "team_id").or_else(|| str_at(inner, "team")),
        user: str_at(inner, "user"),
        channel: str_at(inner, "channel"),
        channel_type: str_at(inner, "channel_type"),
        text: str_at(inner, "text"),
        ts: str_at(inner, "ts"),
        thread_ts: str_at(inner, "thread_ts"),
        subtype: str_at(inner, "subtype"),
        bot_id: str_at(inner, "bot_id"),
        files,
    }
}

fn parse_interaction(payload: &Value) -> Interaction {
    let actions = payload
        .get("actions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|a| serde_json::from_value::<ActionRef>(a.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    Interaction {
        kind: str_at(payload, "type").unwrap_or_default(),
        user_id: payload.get("user").and_then(|u| str_at(u, "id")),
        // `team` is null for an org-wide install, so the enterprise id is the
        // fallback rather than an alternative.
        team_id: payload
            .get("team")
            .and_then(|t| str_at(t, "id"))
            .or_else(|| payload.get("enterprise").and_then(|e| str_at(e, "id"))),
        channel_id: payload.get("channel").and_then(|c| str_at(c, "id")),
        message_ts: payload
            .get("container")
            .and_then(|c| str_at(c, "message_ts"))
            .or_else(|| payload.get("message").and_then(|m| str_at(m, "ts"))),
        thread_ts: payload
            .get("container")
            .and_then(|c| str_at(c, "thread_ts"))
            .or_else(|| payload.get("message").and_then(|m| str_at(m, "thread_ts"))),
        trigger_id: str_at(payload, "trigger_id"),
        response_url: str_at(payload, "response_url"),
        actions,
        view: payload.get("view").map(parse_view),
    }
}

/// Flatten a submitted view: `state.values` is a map of block id → action id
/// → `{type, value}`, and the block-id layer is the composer's own labelling —
/// keying the result by `action_id` is what lets a caller ask for the field it
/// named without re-walking the nesting.
fn parse_view(view: &Value) -> ViewRef {
    let mut values = std::collections::BTreeMap::new();
    if let Some(blocks) = view
        .get("state")
        .and_then(|s| s.get("values"))
        .and_then(Value::as_object)
    {
        for inputs in blocks.values() {
            let Some(inputs) = inputs.as_object() else {
                continue;
            };
            for (action_id, input) in inputs {
                if let Some(text) = input.get("value").and_then(Value::as_str) {
                    values.insert(action_id.clone(), text.to_string());
                }
            }
        }
    }
    ViewRef {
        callback_id: str_at(view, "callback_id").unwrap_or_default(),
        private_metadata: str_at(view, "private_metadata").unwrap_or_default(),
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(v: Value) -> Envelope {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_dm_message_parses_with_its_thread_and_event_id() {
        let e = envelope(json!({
            "type": "events_api",
            "envelope_id": "env-1",
            "payload": {
                "event_id": "Ev123",
                "team_id": "T1",
                "event": {
                    "type": "message",
                    "channel_type": "im",
                    "user": "U1",
                    "channel": "D1",
                    "text": "run the tests",
                    "ts": "1000.1"
                }
            }
        }));
        let Inbound::Event { envelope_id, event } = interpret(&e) else {
            panic!("expected an event");
        };
        assert_eq!(envelope_id, "env-1");
        assert_eq!(event.event_id, "Ev123", "dedupe depends on this");
        assert_eq!(event.text.as_deref(), Some("run the tests"));
        assert_eq!(
            event.thread_key().as_deref(),
            Some("1000.1"),
            "a top-level message is its own thread"
        );
        assert!(event.is_from_a_human());
    }

    #[test]
    fn a_reply_belongs_to_the_thread_it_replies_to() {
        let e = envelope(json!({
            "type": "events_api",
            "envelope_id": "env-2",
            "payload": { "event_id": "Ev2", "event": {
                "type": "message", "user": "U1", "channel": "D1",
                "ts": "2000.5", "thread_ts": "1000.1"
            }}
        }));
        let Inbound::Event { event, .. } = interpret(&e) else {
            panic!()
        };
        assert_eq!(event.thread_key().as_deref(), Some("1000.1"));
    }

    #[test]
    fn the_apps_own_messages_are_not_input() {
        // Without this the connector's own reply is an event, and the loop
        // never ends.
        let e = envelope(json!({
            "type": "events_api", "envelope_id": "e",
            "payload": { "event_id": "Ev3", "event": {
                "type": "message", "channel": "D1", "bot_id": "B1", "text": "hi"
            }}
        }));
        let Inbound::Event { event, .. } = interpret(&e) else {
            panic!()
        };
        assert!(!event.is_from_a_human());
    }

    #[test]
    fn attachments_come_off_the_message_event() {
        let e = envelope(json!({
            "type": "events_api", "envelope_id": "e",
            "payload": { "event_id": "Ev4", "event": {
                "type": "message", "user": "U1", "channel": "D1", "ts": "1.0",
                "files": [{ "id": "F1", "name": "shot.png", "mimetype": "image/png",
                            "size": 1234, "url_private": "https://files.slack.com/x" }]
            }}
        }));
        let Inbound::Event { event, .. } = interpret(&e) else {
            panic!()
        };
        assert_eq!(event.files.len(), 1);
        assert_eq!(event.files[0].id, "F1");
        assert_eq!(event.files[0].size, Some(1234));
    }

    #[test]
    fn a_button_press_carries_the_user_and_the_trigger() {
        let e = envelope(json!({
            "type": "interactive",
            "envelope_id": "env-3",
            "payload": {
                "type": "block_actions",
                "user": { "id": "U1" },
                "team": { "id": "T1" },
                "channel": { "id": "C1" },
                "container": { "message_ts": "1.5", "thread_ts": "1.0" },
                "trigger_id": "trig",
                "response_url": "https://hooks.slack.com/x",
                "actions": [{ "action_id": "approve", "value": "call-9" }]
            }
        }));
        let Inbound::Interactive { interaction, .. } = interpret(&e) else {
            panic!("expected an interaction");
        };
        assert_eq!(interaction.user_id.as_deref(), Some("U1"));
        assert_eq!(interaction.trigger_id.as_deref(), Some("trig"));
        assert_eq!(interaction.actions[0].action_id, "approve");
        assert_eq!(interaction.thread_ts.as_deref(), Some("1.0"));
    }

    #[test]
    fn a_view_submission_carries_the_signed_user_the_callback_and_what_was_typed() {
        let e = envelope(json!({
            "type": "interactive",
            "envelope_id": "env-4",
            "payload": {
                "type": "view_submission",
                "user": { "id": "U1" },
                "team": { "id": "T1" },
                "view": {
                    "callback_id": "close_request",
                    "private_metadata": "{\"seq\":5}",
                    "state": { "values": {
                        "block_reason": { "reason": {
                            "type": "plain_text_input", "value": "spam"
                        }}
                    }}
                }
            }
        }));
        let Inbound::Interactive { interaction, .. } = interpret(&e) else {
            panic!("expected an interaction");
        };
        assert_eq!(interaction.kind, "view_submission");
        // The gate runs on this exactly as on a button press; the field is
        // the one Slack signed, never anything the view carries.
        assert_eq!(interaction.user_id.as_deref(), Some("U1"));
        let view = interaction.view.expect("the view rides along");
        assert_eq!(view.callback_id, "close_request");
        assert_eq!(view.private_metadata, "{\"seq\":5}");
        assert_eq!(view.values.get("reason").map(String::as_str), Some("spam"));
    }

    #[test]
    fn a_button_press_has_no_view_and_a_bodiless_submission_still_parses() {
        let press = envelope(json!({
            "type": "interactive", "envelope_id": "e",
            "payload": { "type": "block_actions", "user": {"id":"U1"},
                         "actions": [{ "action_id": "a", "value": "v" }] }
        }));
        let Inbound::Interactive { interaction, .. } = interpret(&press) else {
            panic!()
        };
        assert!(interaction.view.is_none());

        let bare = envelope(json!({
            "type": "interactive", "envelope_id": "e",
            "payload": { "type": "view_submission", "user": {"id":"U1"},
                         "view": { "callback_id": "cb" } }
        }));
        let Inbound::Interactive { interaction, .. } = interpret(&bare) else {
            panic!()
        };
        let view = interaction.view.unwrap();
        assert_eq!(view.callback_id, "cb");
        assert!(
            view.values.is_empty(),
            "no state means no values, not a crash"
        );
    }

    #[test]
    fn an_org_install_falls_back_to_the_enterprise_id() {
        // `team` is null for an org-wide install, and the workspace binding
        // check needs *something* to compare against.
        let e = envelope(json!({
            "type": "interactive", "envelope_id": "e",
            "payload": { "type": "block_actions", "user": {"id":"U1"},
                         "team": null, "enterprise": {"id": "E1"} }
        }));
        let Inbound::Interactive { interaction, .. } = interpret(&e) else {
            panic!()
        };
        assert_eq!(interaction.team_id.as_deref(), Some("E1"));
    }

    #[test]
    fn disconnect_reasons_decide_whether_to_come_back() {
        let refresh = envelope(json!({"type":"disconnect","reason":"refresh_requested"}));
        assert!(matches!(
            interpret(&refresh),
            Inbound::Disconnect(DisconnectReason::RefreshRequested)
        ));
        assert!(DisconnectReason::RefreshRequested.wants_reconnect());
        assert!(DisconnectReason::Warning.wants_reconnect());
        assert!(
            !DisconnectReason::LinkDisabled.wants_reconnect(),
            "reconnecting into a disabled app is a loop against a config error"
        );
    }

    #[test]
    fn an_unmodelled_frame_is_named_rather_than_dropped() {
        let e = envelope(json!({"type":"slash_commands","envelope_id":"env-9","payload":{}}));
        match interpret(&e) {
            Inbound::Other { kind, envelope_id } => {
                assert_eq!(kind, "slash_commands");
                assert_eq!(envelope_id.as_deref(), Some("env-9"));
            }
            other => panic!("expected other, got {other:?}"),
        }
    }

    #[test]
    fn everything_ackable_exposes_its_envelope_id() {
        let e = envelope(json!({"type":"events_api","envelope_id":"env-1","payload":{}}));
        assert_eq!(interpret(&e).envelope_id(), Some("env-1"));
        let hello = envelope(json!({"type":"hello"}));
        assert_eq!(interpret(&hello).envelope_id(), None);
    }
}
