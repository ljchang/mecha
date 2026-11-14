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

/// Host-declared encoding identity. These labels describe the configured artifact;
/// an echoed alias is not proof of the server's weights or preprocessing.
#[derive(Debug, Clone)]
pub struct EncodingProfileSpec {
    pub expected_alias: String,
    pub artifact_revision: String,
    pub dimensions: usize,
    pub pooling: String,
    pub normalization: String,
    pub query_instruction: String,
    /// Exactly one `{instruction}` and one `{text}` placeholder.
    pub query_template: String,
    pub preprocessing_version: String,
    pub representation_version: String,
}

/// Validated, immutable encoding identity. No client-side normalization or
/// preprocessing is implied by its labels.
#[derive(Debug, Clone)]
pub struct EncodingProfile {
    spec: EncodingProfileSpec,
    id: String,
}

impl EncodingProfile {
    pub fn new(spec: EncodingProfileSpec) -> Result<Self> {
        if !(1..=8192).contains(&spec.dimensions) {
            bail!("embedding dimensions must be in 1..=8192");
        }
        let fields = [
            &spec.expected_alias,
            &spec.artifact_revision,
            &spec.pooling,
            &spec.normalization,
            &spec.query_instruction,
            &spec.query_template,
            &spec.preprocessing_version,
            &spec.representation_version,
        ];
        if fields.iter().any(|s| s.trim().is_empty() || s.len() > 8192) {
            bail!("encoding identity fields must be nonempty and at most 8192 bytes");
        }
        if spec.query_template.matches("{instruction}").count() != 1
            || spec.query_template.matches("{text}").count() != 1
        {
            bail!("query template requires exactly one instruction and text placeholder");
        }
        use sha2::{Digest, Sha256};
        // A versioned JSON tuple is unambiguous even when labels contain delimiters.
        let identity = serde_json::to_vec(&serde_json::json!([
            "mecha-encoding-v1",
            spec.expected_alias,
            spec.artifact_revision,
            spec.dimensions,
            spec.pooling,
            spec.normalization,
            spec.query_instruction,
            spec.query_template,
            spec.preprocessing_version,
            spec.representation_version
        ]))?;
        let id = Sha256::digest(identity)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Self { spec, id })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn expected_alias(&self) -> &str {
        &self.spec.expected_alias
    }
    pub fn artifact_revision(&self) -> &str {
        &self.spec.artifact_revision
    }
    pub fn dimensions(&self) -> usize {
        self.spec.dimensions
    }
    pub fn pooling(&self) -> &str {
        &self.spec.pooling
    }
    pub fn normalization(&self) -> &str {
        &self.spec.normalization
    }
    pub fn query_instruction(&self) -> &str {
        &self.spec.query_instruction
    }
    pub fn query_template(&self) -> &str {
        &self.spec.query_template
    }
    pub fn preprocessing_version(&self) -> &str {
        &self.spec.preprocessing_version
    }
    pub fn representation_version(&self) -> &str {
        &self.spec.representation_version
    }

    fn format(&self, input: &EmbeddingInput) -> Result<String> {
        // Check before allocation, including the otherwise unbounded caller text.
        if input.text.chars().take(MAX_CHARS + 1).count() > MAX_CHARS {
            bail!("checked embedding input exceeds scalar limit");
        }
        let formatted = match input.task {
            Task::Passage => input.text.clone(),
            Task::Recall => bail!("checked embeddings accept query and passage tasks only"),
            Task::Query => {
                // Substitute only tokens in the template, never placeholder-looking
                // text inside the instruction or user input.
                let template = self.query_template();
                let mut tokens = [
                    (
                        template.find("{instruction}").unwrap(),
                        "{instruction}",
                        self.query_instruction(),
                    ),
                    (
                        template.find("{text}").unwrap(),
                        "{text}",
                        input.text.as_str(),
                    ),
                ];
                tokens.sort_by_key(|x| x.0);
                let mut text = String::new();
                let mut start = 0;
                for (position, token, value) in tokens {
                    text.push_str(&template[start..position]);
                    text.push_str(value);
                    start = position + token.len();
                }
                text.push_str(&template[start..]);
                text
            }
        };
        if formatted.chars().take(MAX_CHARS + 1).count() > MAX_CHARS {
            bail!("formatted checked embedding input exceeds scalar limit");
        }
        Ok(formatted)
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddingInput {
    pub text: String,
    pub task: Task,
}

/// Complete, ordered vectors and bounded transport accounting. No partial batch
/// escapes on failure. The server alias is reported separately from host identity.
#[derive(Debug)]
pub struct EmbeddedBatch {
    pub profile_id: String,
    pub server_model: Option<String>,
    pub vectors: Vec<Vec<f32>>,
    pub attempts: usize,
    pub request_bytes: usize,
    pub response_bytes: usize,
}

const CHECKED_BATCH: usize = 16;
const CHECKED_REQUEST_BYTES: usize = 262_144;
const CHECKED_RESPONSE_BYTES: usize = 8_388_608;
const CHECKED_ATTEMPTS: usize = 64;
const CHECKED_DURATION: Duration = Duration::from_secs(120);

fn checked_request(input: &[String], profile: &EncodingProfile) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "model": profile.expected_alias(), "input": input, "encoding_format": "float"
    }))?;
    if bytes.len() > CHECKED_REQUEST_BYTES {
        bail!("checked embedding request exceeds byte limit");
    }
    Ok(bytes)
}

fn checked_vectors(body: &[u8], count: usize, profile: &EncodingProfile) -> Result<Vec<Vec<f32>>> {
    let body: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| anyhow!("invalid checked embedding JSON"))?;
    if body["model"].as_str() != Some(profile.expected_alias()) {
        bail!("checked embedding model alias mismatch");
    }
    let data = body["data"]
        .as_array()
        .ok_or_else(|| anyhow!("checked embedding data missing"))?;
    if data.len() != count {
        bail!("checked embedding count mismatch");
    }
    let mut ordered = vec![None; count];
    for item in data {
        let index = item["index"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < count)
            .ok_or_else(|| anyhow!("invalid checked embedding index"))?;
        if ordered[index].is_some() {
            bail!("duplicate checked embedding index");
        }
        let raw = item["embedding"]
            .as_array()
            .ok_or_else(|| anyhow!("checked embedding vector missing"))?;
        if raw.len() != profile.dimensions() {
            bail!("checked embedding dimension mismatch");
        }
        let mut vector = Vec::with_capacity(raw.len());
        let mut norm = 0f64;
        for coordinate in raw {
            let value = coordinate
                .as_f64()
                .map(|v| v as f32)
                .filter(|v| v.is_finite())
                .ok_or_else(|| anyhow!("invalid checked embedding coordinate"))?;
            norm += f64::from(value).powi(2);
            vector.push(value);
        }
        if norm == 0.0 || !norm.is_finite() {
            bail!("invalid checked embedding norm");
        }
        ordered[index] = Some(vector);
    }
    ordered
        .into_iter()
        .map(|v| v.ok_or_else(|| anyhow!("missing checked embedding index")))
        .collect()
}

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
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(Duration::from_secs(120))
                .build()
                .context("building the embeddings client")?,
        })
    }

    /// Strict, mixed query/passage embeddings for invocation-local retrieval.
    /// All inputs and initial requests are validated before any HTTP request.
    /// Splits share one deadline and attempt budget; errors contain no server text.
    pub async fn embed_checked(
        &self,
        inputs: &[EmbeddingInput],
        profile: &EncodingProfile,
    ) -> Result<EmbeddedBatch> {
        self.checked_with_budget(inputs, profile, CHECKED_DURATION, CHECKED_ATTEMPTS)
            .await
    }

    async fn checked_with_budget(
        &self,
        inputs: &[EmbeddingInput],
        profile: &EncodingProfile,
        duration: Duration,
        max_attempts: usize,
    ) -> Result<EmbeddedBatch> {
        tokio::time::timeout(duration, async {
            if !crate::provider::router::is_literal_loopback(&self.url) {
                bail!("checked embeddings require literal loopback HTTP without credentials, query or fragment");
            }
            if inputs.len() > CHECKED_BATCH * max_attempts {
                bail!("checked embedding inputs exceed attempt budget");
            }
            let formatted: Vec<String> = inputs.iter().map(|input| profile.format(input)).collect::<Result<_>>()?;
            // Validate every initial request before sending the first. Split requests
            // are subsets, but checked again at their actual serialization boundary.
            for batch in formatted.chunks(CHECKED_BATCH) { checked_request(batch, profile)?; }
            let mut pending: Vec<&[String]> = formatted.chunks(CHECKED_BATCH).rev().collect();
            let mut result = EmbeddedBatch { profile_id: profile.id().into(), server_model: None,
                vectors: Vec::with_capacity(inputs.len()), attempts: 0, request_bytes: 0, response_bytes: 0 };
            while let Some(batch) = pending.pop() {
                if result.attempts == max_attempts { bail!("checked embedding attempt budget exhausted"); }
                let request = checked_request(batch, profile)?;
                result.attempts += 1;
                // Each total is bounded by attempts times its per-attempt cap.
                result.request_bytes += request.len();
                let mut response = self.http.post(format!("{}/v1/embeddings", self.url))
                    .header(reqwest::header::CONTENT_TYPE, "application/json").body(request)
                    .send().await.map_err(|_| anyhow!("checked embedding transport failed"))?;
                let status = response.status();
                if response.content_length().is_some_and(|n| n > CHECKED_RESPONSE_BYTES as u64) {
                    bail!("checked embedding response exceeds byte limit");
                }
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| anyhow!("checked embedding response read failed"))? {
                    if chunk.len() > CHECKED_RESPONSE_BYTES - body.len() {
                        bail!("checked embedding response exceeds byte limit");
                    }
                    body.extend_from_slice(&chunk);
                }
                result.response_bytes += body.len();
                if !status.is_success() {
                    // Classification is internal only: never retain the server body
                    // in an error chain, even when it echoes private input.
                    let overflow = overflowed(&anyhow!(String::from_utf8_lossy(&body).into_owned()));
                    if (status.is_client_error() || status.is_server_error()) && overflow && batch.len() > 1 {
                        let (left, right) = batch.split_at(batch.len()/2);
                        pending.push(right); pending.push(left); continue;
                    }
                    bail!("checked embedding server returned HTTP {}", status.as_u16());
                }
                result.vectors.extend(checked_vectors(&body, batch.len(), profile)?);
                result.server_model = Some(profile.expected_alias().into());
            }
            Ok(result)
        }).await.map_err(|_| anyhow!("checked embedding operation deadline exceeded"))?
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

#[cfg(test)]
#[path = "embed_checked_tests.rs"]
mod checked_tests;
