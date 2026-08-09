//! A scripted HTTP server, so the retry policy and the socket handshake are
//! exercised over a real connection rather than asserted about in the abstract.
//!
//! The same shape as `mecha-core`'s `mock_http`: each connection gets the next
//! canned response and is closed, and anything past the script gets a 500 —
//! which fails the test through the assertion on the request count rather than
//! by hanging.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub type MockResponse = (u16, Vec<(&'static str, String)>, String);

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Serve `responses` in order. Returns the base URL and a counter of how many
/// requests actually arrived.
pub async fn mock_http(responses: Vec<MockResponse>) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);

    tokio::spawn(async move {
        let mut responses = responses.into_iter();
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            counter.fetch_add(1, Ordering::SeqCst);

            // Read the head and then its body, so the client never sees a
            // reset while it is still writing.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let (head_end, body_len) = loop {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break (buf.len(), 0);
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = find(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            let l = l.to_ascii_lowercase();
                            l.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    break (pos + 4, len);
                }
            };
            while buf.len() < head_end + body_len {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }

            let (status, headers, body) =
                responses
                    .next()
                    .unwrap_or((500, Vec::new(), "script exhausted".into()));
            let mut resp = format!(
                "HTTP/1.1 {status} R\r\ncontent-length: {}\r\nconnection: close\r\n",
                body.len()
            );
            for (k, v) in headers {
                resp.push_str(&format!("{k}: {v}\r\n"));
            }
            resp.push_str("\r\n");
            resp.push_str(&body);
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
        }
    });

    (format!("http://{addr}"), count)
}

pub fn ok_body(extra: &str) -> String {
    if extra.is_empty() {
        r#"{"ok":true}"#.to_string()
    } else {
        format!(r#"{{"ok":true,{extra}}}"#)
    }
}
