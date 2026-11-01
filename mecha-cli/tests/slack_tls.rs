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

/// `SocketMode::open`, the call site that regressed, dials `wss://` without
/// asking for the process default. A stub answers `apps.connections.open`
/// with a `wss://` URL on a listener that accepts and closes, so the TCP
/// connect succeeds, tungstenite builds its TLS connector, and the handshake
/// then fails on a peer that speaks no TLS. With `open` back on
/// `connect_async` (the default provider) this panics in a `monty` build:
/// the regression that shipped, which a `ws://` fixture never reaches
/// (reviews of #788).
#[tokio::test]
async fn slacks_socket_opens_over_wss_without_asking_for_a_default_provider() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let tls_peer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let wss = format!("wss://{}/link", tls_peer.local_addr().unwrap());
    tokio::spawn(async move {
        // Accepted and closed: the connect succeeds and the handshake ends.
        let _ = tls_peer.accept().await;
    });
    // `apps.connections.open`, answered once with that URL.
    let api = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", api.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut sock, _) = api.accept().await.unwrap();
        let mut buf = vec![0u8; 16 * 1024];
        let _ = sock.read(&mut buf).await;
        let body = format!(r#"{{"ok":true,"url":"{wss}"}}"#);
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(reply.as_bytes()).await;
    });
    let socket = mecha_slack::SocketMode::new(
        mecha_slack::Slack::new("xoxb-test").with_base_url(base),
        mecha_slack::SocketOptions {
            app_token: "xapp-test".into(),
            debug_reconnects: false,
        },
    );
    let opened = tokio::time::timeout(std::time::Duration::from_secs(10), socket.open())
        .await
        .expect("open finished");
    match opened {
        Ok(_) => panic!("a peer that speaks no TLS was dialled"),
        Err(e) => assert!(
            matches!(e, mecha_slack::SlackError::Disconnected(_)),
            "open failed before the dial: {e}"
        ),
    }
}

/// The socket's config builds in any build, one provider or two.
#[test]
fn slacks_socket_builds_its_tls() {
    mecha_slack::socket::tls_config().expect("the socket's TLS config builds");
}
