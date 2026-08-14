//! A Slack client, sized for driving one agent from one workspace.
//!
//! This crate is the transport half of the remote control designed in
//! `docs/SLACK-DESIGN.md`. It speaks Socket Mode (so mecha dials Slack and no
//! port on this machine is ever exposed), the handful of Web API methods a
//! remote control needs, and files in both directions.
//!
//! **It knows nothing about agents, and that is enforced by its dependency
//! list rather than by review.** There is no `mecha-core` in `Cargo.toml`, so
//! nothing here can learn what a run, a tool, a conversation or an approval is.
//! The wiring that does know all of those lives in `mecha-cli/src/slack/`,
//! which is where `tui/` lives, for the same reason.
//!
//! Three things about Slack's API that shape everything below:
//!
//! - **A refusal arrives as HTTP 200.** Slack answers `{"ok": false, "error":
//!   "..."}` with a success status, so a client that checks the status code and
//!   then reads the body will happily deserialise a failure into whatever it
//!   expected. Every call goes through [`Slack::call`], which checks `ok`
//!   first. This is the same shape as the Anthropic backend's
//!   `stop_reason: "refusal"` at HTTP 200 — check the envelope before reading
//!   the content.
//! - **Socket Mode is pre-authenticated.** The WebSocket is opened with an
//!   app-level token, so inbound events carry no signature and need none;
//!   there is deliberately no request-verification code in this crate, because
//!   the HTTP path it would serve does not exist.
//! - **A private file download can return a login page with a 200.** See
//!   [`files::download`], which refuses it four different ways.
//!
//! Rate limits are honoured rather than guessed at: a 429 carries
//! `Retry-After` and it is obeyed, up to a cap, above which it is reported as a
//! failure instead of becoming an invisible multi-minute nap.

pub mod binding;
pub mod blocks;
pub mod chat;
pub mod envelope;
pub mod error;
pub mod files;
pub mod http;
pub mod socket;
pub mod store;
pub mod views;

#[cfg(test)]
mod testutil;

pub use binding::{Binding, Credentials, Gate, PendingLink, SlackStore};
pub use error::{SlackError, SlackResult};
pub use http::Slack;
pub use socket::{SocketMode, SocketOptions};
