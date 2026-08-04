//! Microsoft: Outlook mail and calendar over Graph, plus the device-code
//! OAuth flow, which is the right shape for a CLI — no redirect URI to
//! register, and no port to forward when you are working over SSH.

pub mod auth;
pub mod graph_calendar;
pub mod graph_mail;
pub mod server;
