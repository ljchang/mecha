//! Text to vectors, from the loopback embeddings server (`:8081`, an
//! OpenAI-compatible `/v1/embeddings` on llama-server).
//!
//! Ported from mecha-graph's `embed` (the owner's direction of 2026-09-24:
//! the graph converges into mecha, and an overlapping mechanism is built in
//! core). What came with it, each a lesson there:
//!
//! - **The instruction rides on the query side only.** The Qwen3 and Harrier
//!   families embed a passage bare and a query as `Instruct: …\nQuery: …`;
//!   embedding both alike is a silent loss of quality, not an error.
//! - **A batch that overflows the server's context is split, not abandoned.**
//!   A batch of ordinary inputs can overflow where each fits; halving
//!   converges on the one input that does not.
//! - **Position is the only thing tying a vector to its input**, so a reply
//!   with a different count is refused rather than zipped short.
//!
//! What it does not carry: the graph's dimension bookkeeping for `sqlite-vec`
//! tables. Here a vector's length is recorded beside it and an index built by
//! another model is rebuilt, never compared (`persona::search`).

use anyhow::{anyhow, bail, Context, Result};
use std::time::Duration;

/// What an embedding is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// A passage to be found.
    Passage,
    /// A search for passages.
    Query,
    /// A message in a conversation, finding the memories it calls up — a
    /// persona's recall (PERSONA-DESIGN §9.7). Its own instruction, because
    /// the instruction is part of what the vector means.
    Recall,
}

/// The query-side instruction, in the Qwen3/Harrier form. Part of an
/// index's identity: change it and the stored passages were embedded for a
/// different question.
pub const QUERY_INSTRUCTION: &str =
    "Given a question about a document, retrieve the passages of it that answer the question";

/// The recall-side instruction: a message, and the memories of earlier
/// conversations it should bring to mind.
pub const RECALL_INSTRUCTION: &str =
    "Given a message in a conversation, retrieve the memories of earlier conversations that are relevant to it";

/// Characters kept per input — far under the server's window, and a passage
/// here is a chunk, not a document.
const MAX_CHARS: usize = 8_000;

/// A client for one embeddings server.
#[derive(Debug, Clone)]
pub struct Embedder {
    url: String,
    http: reqwest::Client,
}

impl Embedder {
    /// `url` must be loopback, for the reason the OCR server's must: what is
    /// embedded is the owner's material.
    pub fn new(url: &str) -> Result<Self> {
        let url = crate::document::ocr_url(url)
            // `ocr_url` names its own key; this is `embed_url`'s refusal.
            .map_err(|_| {
                anyhow!(
                    "[documents] embed_url `{url}` must be an http URL on this machine (loopback)"
                )
            })?
            .as_str()
            .trim_end_matches('/')
            .to_string();
        Ok(Embedder {
            url,
            http: reqwest::Client::builder()
                // The server is socket-activated: the first request waits
                // out a model load (CLAUDE.md, the local model server).
                .timeout(Duration::from_secs(120))
                .build()
                .context("building the embeddings client")?,
        })
    }

    /// One vector per input, in order.
    pub async fn embed(&self, texts: &[String], task: Task) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        // Halving on overflow, iteratively: a stack of slices still to send.
        let mut todo = vec![texts];
        while let Some(batch) = todo.pop() {
            match self.once(batch, task).await {
                Ok(v) => out.push(v),
                Err(e) if batch.len() > 1 && overflowed(&e) => {
                    let (a, b) = batch.split_at(batch.len() / 2);
                    // Popped last-in first-out: push the second half first.
                    todo.push(b);
                    todo.push(a);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(out.into_iter().flatten().collect())
    }

    /// The vector for recalling memories from a message (`Task::Recall`).
    pub async fn recall_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(&[text.to_string()], Task::Recall)
            .await?
            .pop()
            .ok_or_else(|| anyhow!("the embeddings server returned nothing"))
    }

    /// The vector for a search.
    pub async fn query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(&[text.to_string()], Task::Query)
            .await?
            .pop()
            .ok_or_else(|| anyhow!("the embeddings server returned nothing"))
    }

    async fn once(&self, texts: &[String], task: Task) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let input: Vec<String> = texts
            .iter()
            .map(|t| {
                let t: String = t.chars().take(MAX_CHARS).collect();
                match task {
                    Task::Passage => t,
                    Task::Query => format!("Instruct: {QUERY_INSTRUCTION}\nQuery: {t}"),
                    Task::Recall => format!("Instruct: {RECALL_INSTRUCTION}\nQuery: {t}"),
                }
            })
            .collect();
        let resp = self
            .http
            .post(format!("{}/v1/embeddings", self.url))
            .json(&serde_json::json!({ "input": input }))
            .send()
            .await
            .with_context(|| format!("the embeddings server at {} did not answer", self.url))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "the embeddings server answered {status}: {}",
                body.chars().take(300).collect::<String>()
            );
        }
        let body: serde_json::Value = resp.json().await.context("an embeddings reply")?;
        let data = body["data"]
            .as_array()
            .ok_or_else(|| anyhow!("an embeddings reply with no data"))?;
        if data.len() != input.len() {
            bail!("asked for {} embeddings, got {}", input.len(), data.len());
        }
        data.iter()
            .map(|item| {
                let raw = item["embedding"]
                    .as_array()
                    .ok_or_else(|| anyhow!("an embedding that is not a list"))?;
                // A non-number is refused, not skipped: a vector short by
                // one would be stored and silently never ranked.
                let v: Vec<f32> = raw
                    .iter()
                    .map(|x| x.as_f64().map(|f| f as f32))
                    .collect::<Option<_>>()
                    .ok_or_else(|| anyhow!("an embedding with a value that is not a number"))?;
                if v.is_empty() {
                    bail!("an empty embedding");
                }
                Ok(v)
            })
            .collect()
    }
}

/// The server's way of saying a batch was too long for its context.
fn overflowed(e: &anyhow::Error) -> bool {
    let s = format!("{e:#}").to_lowercase();
    s.contains("context")
        && (s.contains("exceed") || s.contains("too large") || s.contains("too long"))
        || s.contains("input is too large")
}

/// Cosine similarity; `0.0` for vectors of different lengths or a zero one,
/// which a caller compares as "unrelated" rather than failing on.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// A vector as stored: little-endian `f32`s.
pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A stored vector back; `None` for a blob that is not whole `f32`s.
pub fn from_blob(b: &[u8]) -> Option<Vec<f32>> {
    b.len().is_multiple_of(4).then(|| {
        b.as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_and_blobs() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0, "different lengths");
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0, "a zero vector");
        let v = vec![0.5, -1.25, 3.0];
        assert_eq!(from_blob(&to_blob(&v)).unwrap(), v);
        assert_eq!(from_blob(&[1, 2, 3]), None);
    }

    #[test]
    fn only_a_loopback_server_is_taken() {
        assert!(Embedder::new("http://127.0.0.1:8081").is_ok());
        assert!(Embedder::new("http://192.168.1.2:8081").is_err());
    }

    /// A server that answers the shape llama-server does: the query side
    /// carries the instruction, a passage does not, and an overflowing
    /// batch is split until each part fits.
    #[tokio::test]
    async fn queries_carry_the_instruction_and_an_overflow_splits() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let log = std::sync::Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let log = std::sync::Arc::clone(&log);
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    let body = loop {
                        let n = sock.read(&mut chunk).await.unwrap();
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(at) = text.find("\r\n\r\n") {
                            let len: usize = text
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse().unwrap())
                                })
                                .unwrap_or(0);
                            if buf.len() >= at + 4 + len {
                                break text[at + 4..].to_string();
                            }
                        }
                    };
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                    let input: Vec<String> = v["input"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|s| s.as_str().unwrap().to_string())
                        .collect();
                    log.lock().unwrap().push(input.clone());
                    let (status, reply) = if input.len() > 2 {
                        (
                            "500 Internal Server Error",
                            serde_json::json!({"error": {"message": "input (9292 tokens) exceeds the context size"}}),
                        )
                    } else {
                        (
                            "200 OK",
                            serde_json::json!({"data": input.iter().enumerate().map(|(i, _)| serde_json::json!({"embedding": [i as f32 + 1.0, 0.5]})).collect::<Vec<_>>()}),
                        )
                    };
                    let reply = reply.to_string();
                    let _ = sock
                        .write_all(format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).as_bytes())
                        .await;
                });
            }
        });
        let e = Embedder::new(&format!("http://127.0.0.1:{port}")).unwrap();
        e.query("what do urchins eat").await.unwrap();
        let passages: Vec<String> = (0..5).map(|i| format!("passage {i}")).collect();
        let got = e.embed(&passages, Task::Passage).await.unwrap();
        assert_eq!(got.len(), 5, "one vector per input, in order");
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen[0][0].starts_with("Instruct: ")
                && seen[0][0].ends_with("Query: what do urchins eat")
        );
        assert!(
            seen[1..]
                .iter()
                .flatten()
                .all(|s| s.starts_with("passage ")),
            "{seen:?}"
        );
        assert_eq!(seen[1].len(), 5, "first the whole batch");
        // Then halves until each fits; what the server took, in order.
        let taken: Vec<&String> = seen[2..]
            .iter()
            .filter(|b| b.len() <= 2)
            .flatten()
            .collect();
        assert_eq!(taken, passages.iter().collect::<Vec<_>>(), "{seen:?}");
    }
}
