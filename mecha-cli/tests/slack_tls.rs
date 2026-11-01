//! Slack's socket must connect in the build that ships.
//!
//! With code mode's interpreter built in (the cargo feature `monty`, on by
//! default since #781), `monty-pool` links rustls's aws-lc-rs provider beside
//! the ring one the rest of mecha uses. rustls then refuses to pick a process
//! default, and anything that asks for one panics. `connect_async` asked, so on
//! 2026-11-01 the first deploy with `monty` crash-looped the Slack connector
//! (12 panics in a minute). Nothing in CI opened Slack's socket, so CI passed.
//! The socket now names its provider (`mecha_slack::socket::tls_config`).

/// In this build there is no default provider to ask for, which is the
/// hazard, and the socket's config builds anyway. On the old path
/// (`connect_async`, which calls `ClientConfig::builder()`) the second half
/// is the panic the first half shows.
#[cfg(feature = "monty")]
#[test]
fn slacks_socket_builds_its_tls_though_two_crypto_providers_are_linked() {
    let asked = std::panic::catch_unwind(|| {
        let _ = rustls::ClientConfig::builder();
    });
    assert!(
        asked.is_err(),
        "this build has a process default after all: one provider linked, or one installed. \
         Re-read the reason for `tls_config` before keeping this test as it is"
    );
    mecha_slack::socket::tls_config().expect("the socket's TLS config builds");
}

/// The socket's config builds in any build, one provider or two.
#[test]
fn slacks_socket_builds_its_tls() {
    mecha_slack::socket::tls_config().expect("the socket's TLS config builds");
}
