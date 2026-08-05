//! `mecha-mail` — Gmail, Outlook, and their calendars, extracted from
//! flowmail.
//!
//! The library layer is a plain client per provider: [`google`] and
//! [`microsoft`] each hold API clients constructible from an access token
//! plus their OAuth flow. [`token`] owns the credential lifecycle flowmail
//! kept in its JS frontend; [`http`] (retry), [`text`] (HTML→text and prompt
//! sanitizing), and [`mcp`] (the stdio transport) are shared. A GUI — a
//! future flowmail — would depend on this crate directly.
//!
//! Each binary is one provider's MCP face, with its own credential store, so
//! a deployment can wire either or both.

pub mod accounts;
pub mod google;
pub mod http;
pub mod mcp;
pub mod microsoft;
pub mod text;
pub mod time;
pub mod token;
pub mod types;
pub mod unified;

/// Logging to **stderr only** — stdout belongs to the MCP transport, and a
/// stray log line there is a protocol error.
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("MECHA_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}
