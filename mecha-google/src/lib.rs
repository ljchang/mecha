//! `mecha-google` — Gmail and Google Calendar, extracted from flowmail.
//!
//! The library layer is a plain client: [`gmail::GmailProvider`] and
//! [`calendar::CalendarProvider`] are constructible from an access token,
//! [`auth`] runs the desktop OAuth flow, [`token`] owns the credential
//! lifecycle flowmail kept in its JS frontend. A GUI (a future flowmail)
//! would depend on this crate directly.
//!
//! The binary ([`server`]) is the MCP face: a stdio JSON-RPC server exposing
//! the clients as tools for mecha or any other MCP client.

pub mod auth;
pub mod calendar;
pub mod gmail;
pub mod http;
pub mod server;
pub mod text;
pub mod token;
pub mod types;
