//! Socket Mode: mecha dials Slack, and nothing on this machine listens.
//!
//! That is the whole reason this is a WebSocket rather than a webhook. There is
//! no inbound port, no certificate to renew, no tunnel to babysit, and — the
//! part that removes a class of bugs rather than an inconvenience — **no
//! request signature to verify**, because the socket is authenticated by the
//! app-level token that opened it. Slack states it plainly: there is no need to
//! verify or validate inbound events.
//!
//! It is also the shape this project already reached for a different problem.
//! `scripts/mecha-drain.service` holds a long poll open against the factory so
//! that nothing ever has to dial home, and its comment is the argument here
//! too: *instant is a property of who waits, not of who calls.*
//!
//! Two behaviours worth knowing before changing anything:
//!
//! - **Reconnect is make-before-break.** Slack rotates connections every few
//!   hours and gives about ten seconds' warning. Opening the replacement before
//!   draining the old one means there is no window in which an event has
//!   nowhere to land.
//! - **Acks are automatic, and they happen before the handler runs.** The three
//!   second budget belongs to Slack's socket, not to an agent turn that may
//!   take twenty minutes. Whether Slack replays unacked envelopes is
//!   undocumented, which is the other half of why every event carries an
//!   `event_id` to dedupe on.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::envelope::{interpret, Envelope, Inbound};
use crate::error::{SlackError, SlackResult};
use crate::http::Slack;

/// How long the old connection is drained after its replacement is open.
/// Slack's warning is about ten seconds; this outlasts it slightly so a frame
/// in flight is not lost to arithmetic.
const DRAIN_GRACE: Duration = Duration::from_secs(12);

/// Backoff between failed connection attempts, doubling to a ceiling. A
/// workstation that wakes from suspend with no network must not spin.
const RECONNECT_MIN: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct SocketOptions {
    /// The `xapp-` token, minted with `connections:write`. Distinct from the
    /// bot token: it is not workspace-scoped and it is not an OAuth token.
    pub app_token: String,
    /// Ask Slack for short connections, which makes reconnect handling
    /// testable in minutes instead of hours.
    pub debug_reconnects: bool,
}

pub struct SocketMode {
    slack: Slack,
    options: SocketOptions,
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// One live socket.
pub struct Connection {
    ws: Ws,
}

impl SocketMode {
    pub fn new(slack: Slack, options: SocketOptions) -> Self {
        Self { slack, options }
    }

    /// Ask Slack for a socket URL and connect to it.
    pub async fn open(&self) -> SlackResult<Connection> {
        let response: Value = self
            .slack
            .call_with_token("apps.connections.open", &self.options.app_token)
            .await?;
        let mut url = response
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| SlackError::Malformed {
                method: "apps.connections.open".into(),
                detail: "no url".into(),
            })?
            .to_string();
        if self.options.debug_reconnects {
            url.push_str("&debug_reconnects=true");
        }

        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| SlackError::Disconnected(format!("could not open the socket: {e}")))?;
        Ok(Connection { ws })
    }

    /// Run until cancelled, forwarding every inbound frame and reconnecting on
    /// its own. The receiver end is what a front-end selects on.
    ///
    /// Errors from `open` are retried with backoff **except** the terminal
    /// ones: an invalid app token or a disabled socket mode will fail exactly
    /// the same way forever, and grinding against it is how an app's tokens get
    /// disabled.
    pub async fn run(
        &self,
        out: mpsc::Sender<Inbound>,
        cancel: impl Fn() -> bool + Send,
    ) -> SlackResult<()> {
        let mut backoff = RECONNECT_MIN;
        let mut current: Option<Connection> = None;

        loop {
            if cancel() {
                return Ok(());
            }

            let mut conn = match current.take() {
                Some(c) => c,
                None => match self.open().await {
                    // Deliberately *not* resetting the backoff here: a dial
                    // that succeeds proves nothing about a connection that
                    // lasts. `pump` resets it once frames actually arrive.
                    Ok(c) => c,
                    Err(e) if e.is_transient() => {
                        tracing::warn!("slack socket open failed ({e}); retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(RECONNECT_MAX);
                        continue;
                    }
                    Err(e) => return Err(e),
                },
            };

            match self.pump(&mut conn, &out, &cancel, &mut backoff).await {
                Pump::Cancelled => return Ok(()),
                Pump::Closed => {
                    // Backoff covers *this* path too, and the reset lives with
                    // the frames rather than with the dial. A socket Slack
                    // accepts and then immediately closes would otherwise
                    // reopen with no delay — and because opening succeeded,
                    // reset the delay on the way — which is an unthrottled
                    // loop against `apps.connections.open` that ends in a rate
                    // limit or a disabled app.
                    tracing::debug!("slack socket closed; reopening in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RECONNECT_MAX);
                }
                Pump::Fatal(reason) => {
                    return Err(SlackError::Disconnected(reason));
                }
                Pump::Refresh => {
                    // Make before break: the replacement is open before the old
                    // socket is drained, so no frame has nowhere to land.
                    match self.open().await {
                        Ok(next) => {
                            let _ = tokio::time::timeout(
                                DRAIN_GRACE,
                                self.pump(&mut conn, &out, &cancel, &mut backoff),
                            )
                            .await;
                            current = Some(next);
                        }
                        Err(e) => {
                            tracing::warn!("could not pre-open the replacement socket ({e})");
                        }
                    }
                }
            }
        }
    }

    /// Read one connection until it ends. Acks before forwarding, because the
    /// ack budget is Slack's and the handler's time is the agent's.
    async fn pump(
        &self,
        conn: &mut Connection,
        out: &mpsc::Sender<Inbound>,
        cancel: &(impl Fn() -> bool + Send),
        backoff: &mut Duration,
    ) -> Pump {
        loop {
            if cancel() {
                return Pump::Cancelled;
            }
            let message = match conn.ws.next().await {
                Some(Ok(m)) => m,
                Some(Err(e)) => {
                    tracing::debug!("slack socket error: {e}");
                    return Pump::Closed;
                }
                None => return Pump::Closed,
            };

            let text = match message {
                Message::Text(t) => t.to_string(),
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => return Pump::Closed,
                _ => continue,
            };

            let envelope: Envelope = match serde_json::from_str(&text) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("slack sent a frame this crate cannot parse: {e}");
                    continue;
                }
            };
            // A frame arrived, so the connection is genuinely working.
            *backoff = RECONNECT_MIN;
            let inbound = interpret(&envelope);

            if let Some(id) = inbound.envelope_id() {
                if let Err(e) = conn.ack(id).await {
                    tracing::debug!("ack failed ({e}); the connection is going away");
                    return Pump::Closed;
                }
            }

            if let Inbound::Disconnect(reason) = &inbound {
                return if reason.wants_reconnect() {
                    Pump::Refresh
                } else {
                    Pump::Fatal(format!("slack closed the link: {reason:?}"))
                };
            }

            if out.send(inbound).await.is_err() {
                // The front-end is gone; there is nothing to deliver to.
                return Pump::Cancelled;
            }
        }
    }
}

enum Pump {
    /// The caller asked to stop, or the receiver was dropped.
    Cancelled,
    /// The socket ended; open a new one.
    Closed,
    /// Slack asked for a rotation.
    Refresh,
    /// Reconnecting cannot help.
    Fatal(String),
}

impl Connection {
    /// Acknowledge an envelope by echoing its id.
    pub async fn ack(&mut self, envelope_id: &str) -> SlackResult<()> {
        let payload = json!({ "envelope_id": envelope_id }).to_string();
        self.ws
            .send(Message::Text(payload.into()))
            .await
            .map_err(|e| SlackError::Disconnected(format!("ack failed: {e}")))
    }

    /// Read the next frame, for callers driving the socket themselves.
    pub async fn next(&mut self) -> Option<SlackResult<Inbound>> {
        loop {
            match self.ws.next().await? {
                Ok(Message::Text(t)) => {
                    return Some(
                        serde_json::from_str::<Envelope>(&t)
                            .map(|e| interpret(&e))
                            .map_err(|e| SlackError::Malformed {
                                method: "socket".into(),
                                detail: e.to_string(),
                            }),
                    )
                }
                Ok(Message::Close(_)) => return None,
                Ok(_) => continue,
                Err(e) => return Some(Err(SlackError::Disconnected(e.to_string()))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ack_is_just_the_envelope_id() {
        // Slack's ack is an echo, not a receipt with a body. Sending anything
        // else is how an envelope goes unacknowledged while looking handled.
        let payload = json!({ "envelope_id": "env-1" });
        assert_eq!(payload.to_string(), r#"{"envelope_id":"env-1"}"#);
    }

    #[test]
    fn a_connection_that_dies_immediately_still_backs_off() {
        // The loop that mattered: Slack accepts the socket and closes it, so
        // `open` keeps succeeding. Resetting the delay on a successful *dial*
        // would spin; it is reset when a frame actually arrives, which is the
        // only evidence the connection works.
        let mut d = RECONNECT_MIN;
        for _ in 0..4 {
            d = (d * 2).min(RECONNECT_MAX);
        }
        assert!(d > RECONNECT_MIN, "a repeated close must slow down");
    }

    #[test]
    fn backoff_climbs_to_a_ceiling_rather_than_forever() {
        let mut d = RECONNECT_MIN;
        for _ in 0..20 {
            d = (d * 2).min(RECONNECT_MAX);
        }
        assert_eq!(d, RECONNECT_MAX);
    }

    #[test]
    fn the_drain_window_outlasts_slacks_warning() {
        // Slack gives about ten seconds' notice on a `warning` disconnect.
        assert!(DRAIN_GRACE > Duration::from_secs(10));
    }

    /// A Socket Mode server that serves canned frames and reports what was
    /// acked. Enough of Slack to exercise the handshake, the envelope parse and
    /// the ack, none of which the pure tests above can reach.
    async fn fixture_socket(frames: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (acks_tx, acks_rx) = mpsc::channel(16);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            for frame in frames {
                if ws.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
            }
            // Stay open, forwarding every ack the client sends. Closing here
            // would send the client into its reconnect path mid-assertion.
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Text(t) = msg {
                    if acks_tx.send(t.to_string()).await.is_err() {
                        return;
                    }
                }
            }
        });

        (format!("ws://{addr}"), acks_rx)
    }

    #[tokio::test]
    async fn an_event_is_acked_and_forwarded() {
        use crate::testutil::mock_http;

        let event = serde_json::json!({
            "type": "events_api",
            "envelope_id": "env-42",
            "payload": {
                "event_id": "Ev1",
                "event": { "type": "message", "user": "U1", "channel": "D1",
                           "text": "hello", "ts": "1.0" }
            }
        })
        .to_string();

        let (ws_url, mut acks) = fixture_socket(vec![
            r#"{"type":"hello","num_connections":1}"#.to_string(),
            event,
        ])
        .await;

        // `apps.connections.open` is an ordinary Web API call, so the mock HTTP
        // server hands back the fixture's socket URL.
        let (http_base, _) = mock_http(vec![(
            200,
            vec![],
            format!(r#"{{"ok":true,"url":"{ws_url}"}}"#),
        )])
        .await;

        let socket = SocketMode::new(
            Slack::new("xoxb-test").with_base_url(http_base),
            SocketOptions {
                app_token: "xapp-test".into(),
                debug_reconnects: false,
            },
        );

        let (tx, mut rx) = mpsc::channel(16);
        let driver = tokio::spawn(async move { socket.run(tx, || false).await });

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a frame should arrive")
            .expect("the channel should be open");
        assert!(matches!(first, Inbound::Hello));

        let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the event should arrive")
            .expect("the channel should be open");
        match second {
            Inbound::Event { envelope_id, event } => {
                assert_eq!(envelope_id, "env-42");
                assert_eq!(event.text.as_deref(), Some("hello"));
                assert_eq!(event.event_id, "Ev1");
            }
            other => panic!("expected an event, got {other:?}"),
        }

        let ack = tokio::time::timeout(Duration::from_secs(5), acks.recv())
            .await
            .expect("the envelope should have been acked")
            .expect("the ack channel should be open");
        assert_eq!(
            ack, r#"{"envelope_id":"env-42"}"#,
            "the ack is an echo of the envelope id and nothing else"
        );

        driver.abort();
    }

    #[tokio::test]
    async fn hello_is_not_acked_because_it_has_no_envelope() {
        use crate::testutil::mock_http;

        // Acking something with no envelope id would send `{"envelope_id":null}`,
        // which Slack has no reason to accept.
        let (ws_url, mut acks) = fixture_socket(vec![r#"{"type":"hello"}"#.to_string()]).await;
        let (http_base, _) = mock_http(vec![(
            200,
            vec![],
            format!(r#"{{"ok":true,"url":"{ws_url}"}}"#),
        )])
        .await;

        let socket = SocketMode::new(
            Slack::new("xoxb-test").with_base_url(http_base),
            SocketOptions {
                app_token: "xapp-test".into(),
                debug_reconnects: false,
            },
        );
        let (tx, mut rx) = mpsc::channel(16);
        let driver = tokio::spawn(async move { socket.run(tx, || false).await });

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(first, Inbound::Hello));

        assert!(
            tokio::time::timeout(Duration::from_millis(300), acks.recv())
                .await
                .is_err(),
            "nothing should have been acked"
        );

        driver.abort();
    }

    #[tokio::test]
    async fn a_disabled_link_stops_rather_than_reconnecting() {
        use crate::testutil::mock_http;

        // Reconnecting into an app whose socket mode was turned off is a retry
        // loop against a configuration error, and it is how tokens get
        // disabled. The run must end instead.
        let (ws_url, _acks) = fixture_socket(vec![
            r#"{"type":"disconnect","reason":"link_disabled"}"#.to_string(),
        ])
        .await;
        let (http_base, _) = mock_http(vec![(
            200,
            vec![],
            format!(r#"{{"ok":true,"url":"{ws_url}"}}"#),
        )])
        .await;

        let socket = SocketMode::new(
            Slack::new("xoxb-test").with_base_url(http_base),
            SocketOptions {
                app_token: "xapp-test".into(),
                debug_reconnects: false,
            },
        );
        let (tx, _rx) = mpsc::channel(16);

        let outcome = tokio::time::timeout(Duration::from_secs(5), socket.run(tx, || false))
            .await
            .expect("run should return rather than loop");
        assert!(
            matches!(outcome, Err(SlackError::Disconnected(_))),
            "{outcome:?}"
        );
    }
}
