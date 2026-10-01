//! Searching a persona's files (PERSONA-DESIGN §10.4, D15, D19): what a
//! collection too large for the first turn is read through.
//!
//! **One index for the store, keyed by content.** A file's text is cut into
//! passages, page by page, and kept in `<store>/.search.db` under the hash
//! of the file's bytes — so the same paper in two folders is indexed once,
//! and a persona reaches a passage only through a file its own folders hold
//! (`FileSearch` lists them, hashes them, and asks for those hashes only).
//! Nothing here is keyed by persona, so there is no persona filter to forget.
//!
//! **Two ways to find a passage, fused.** Meaning, by cosine over vectors
//! from the `:8081` embeddings server, stored as blobs and compared in Rust
//! (D19); and words, by SQLite's FTS5, which finds the exact name, number or
//! term a vector blurs. The two rankings are fused by reciprocal rank. With
//! the embeddings server down, words alone answer — a degraded search, said
//! as such, never a failed one.
//!
//! **A passage comes back as a page.** `FileSearch` renders each hit under
//! the `document: … ·` header and `=== page N of M ·` marker `file_read`
//! uses, so `persona::cite` checks a quote taken from a search result like
//! any other — the check needs no new reader.
//!
//! **An index built for other vectors is rebuilt, not compared.** The
//! query-side instruction and the vectors' length are the index's identity
//! (`meta`); a change to either drops every stored vector, and passages are
//! embedded again as their files come round.

use super::files::{list, roots, sha_of, Source};
use super::Store;
use crate::embed::{self, Embedder, Task};
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where the index lives, beside the personas' folders. A dot-name, so
/// `Store::load` and the files walk both pass over it.
pub const INDEX_FILE: &str = ".search.db";

/// About a passage's length, in characters: long enough to hold an
/// argument, short enough that a hit is the paragraph and not the paper.
const PASSAGE_CHARS: usize = 1_200;

/// How much of the passage before is carried into the next, so a sentence
/// cut at a boundary is whole in one of them.
const OVERLAP_CHARS: usize = 200;

/// Passages sent to the embeddings server per request.
const BATCH: usize = 16;

/// Reciprocal-rank fusion's constant: the usual 60, which keeps a passage
/// ranked well by one method from being drowned by the other's tail.
const RRF_K: f32 = 60.0;

/// One passage of one file, as a search returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub sha: String,
    /// `None` for a text file, which has no pages.
    pub page: Option<u32>,
    /// The file's page count, for the `=== page N of M` marker.
    pub pages: Option<u32>,
    pub text: String,
}

/// How a search found what it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// By meaning and by words.
    Both,
    /// By words alone: the embeddings server did not answer, or nothing
    /// asked is embedded yet.
    WordsOnly,
}

/// The store's index.
pub struct Index {
    conn: Connection,
}

impl Index {
    /// Open, creating the file and its tables on first use.
    pub fn open(store: &Path) -> Result<Index> {
        let path = store.join(INDEX_FILE);
        let conn = Connection::open(&path)
            .with_context(|| format!("opening the search index at {}", path.display()))?;
        // The timeout first, so the journal switch itself waits out a
        // concurrent opener rather than failing (as `persona::memory` does).
        conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS docs (
                 sha TEXT PRIMARY KEY,
                 pages INTEGER,
                 passages INTEGER NOT NULL,
                 rendered TEXT
             );
             CREATE TABLE IF NOT EXISTS passages (
                 id INTEGER PRIMARY KEY,
                 sha TEXT NOT NULL,
                 page INTEGER,
                 text TEXT NOT NULL,
                 vec BLOB
             );
             CREATE INDEX IF NOT EXISTS passages_sha ON passages (sha);
             CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts USING fts5 (text);",
        )
        .context("creating the search index")?;
        Ok(Index { conn })
    }

    /// Whether `sha`'s passages are in, and all of them embedded.
    pub fn complete(&self, sha: &str) -> Result<bool> {
        let row: Option<i64> = self
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM passages WHERE sha = ?1 AND vec IS NULL)
                 FROM docs WHERE sha = ?1",
                [sha],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row == Some(0))
    }

    /// Whether `sha` has passages at all — searchable by words.
    pub fn has(&self, sha: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM docs WHERE sha = ?1", [sha], |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// The hash of the rendering `sha` was last cut from.
    fn rendered(&self, sha: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT rendered FROM docs WHERE sha = ?1", [sha], |r| {
                r.get(0)
            })
            .optional()?
            .flatten())
    }

    /// Keep `sha`'s passages, cut from its rendered text, replacing any
    /// before. Vectors are filled by [`Index::embed_missing`].
    pub fn put(&mut self, sha: &str, rendered: &str) -> Result<usize> {
        let pages = super::cite::pages_of(rendered);
        let total = pages.iter().filter_map(|p| p.0).max();
        let tx = self.conn.transaction()?;
        // A passage cut the same way again keeps its vector: re-putting a
        // scan whose later pages have since been read embeds only what is
        // new.
        let old: Vec<(i64, String, Option<Vec<u8>>)> = {
            let mut q = tx.prepare("SELECT id, text, vec FROM passages WHERE sha = ?1")?;
            let rows = q
                .query_map([sha], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            rows
        };
        let kept: HashMap<String, Vec<u8>> = old
            .iter()
            .filter_map(|(_, text, vec)| Some((text.clone(), vec.clone()?)))
            .collect();
        for (id, _, _) in &old {
            tx.execute("DELETE FROM passages_fts WHERE rowid = ?1", [id])?;
        }
        tx.execute("DELETE FROM passages WHERE sha = ?1", [sha])?;
        let mut n = 0usize;
        for (page, text) in &pages {
            for passage in passages_of(text) {
                tx.execute(
                    "INSERT INTO passages (sha, page, text, vec) VALUES (?1, ?2, ?3, ?4)",
                    params![sha, page, passage, kept.get(&passage)],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO passages_fts (rowid, text) VALUES (?1, ?2)",
                    params![id, passage],
                )?;
                n += 1;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO docs (sha, pages, passages, rendered) VALUES (?1, ?2, ?3, ?4)",
            params![
                sha,
                total,
                n as i64,
                crate::document::sha256_hex(rendered.as_bytes())
            ],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// The passages of `sha` with no vector yet, as (id, text).
    fn unembedded(&self, sha: &str) -> Result<Vec<(i64, String)>> {
        let mut q = self
            .conn
            .prepare("SELECT id, text FROM passages WHERE sha = ?1 AND vec IS NULL ORDER BY id")?;
        let rows = q
            .query_map([sha], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Record the vectors' identity; a different one drops every vector
    /// stored, so nothing compares a query with a passage embedded another
    /// way. Returns whether it dropped any.
    pub fn claim_identity(&mut self, dims: usize) -> Result<bool> {
        let identity = format!("{dims}|{}", embed::QUERY_INSTRUCTION);
        let had: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'identity'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if had.as_deref() == Some(identity.as_str()) {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        let dropped = had.is_some() && tx.execute("UPDATE passages SET vec = NULL", [])? > 0;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('identity', ?1)",
            [&identity],
        )?;
        tx.commit()?;
        Ok(dropped)
    }

    /// Every passage of `shas` — the corpus a search ranks — with its
    /// vector where `vectors`; ids alone otherwise, so a search by words
    /// decodes no blob (review of #467).
    fn corpus(&self, shas: &[String], vectors: bool) -> Result<Vec<(i64, Option<Vec<f32>>)>> {
        let mut out = Vec::new();
        let mut q = self.conn.prepare(if vectors {
            "SELECT id, vec FROM passages WHERE sha = ?1"
        } else {
            "SELECT id, NULL FROM passages WHERE sha = ?1"
        })?;
        for sha in shas {
            let rows = q.query_map([sha], |r| {
                let blob: Option<Vec<u8>> = r.get(1)?;
                Ok((r.get::<_, i64>(0)?, blob.and_then(|b| embed::from_blob(&b))))
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// Up to `k` passages of `shas` best matching `query`, fusing meaning
    /// (when `qvec` is given) with words. A hit is a passage of one of
    /// `shas`, never another — the scope is the caller's listing.
    pub fn search(
        &self,
        shas: &[String],
        query: &str,
        qvec: Option<&[f32]>,
        k: usize,
    ) -> Result<(Vec<Hit>, Found)> {
        let corpus = self.corpus(shas, qvec.is_some())?;
        let mut scores: HashMap<i64, f32> = HashMap::new();
        let mut by_meaning = false;
        if let Some(q) = qvec {
            // Only vectors made the query's way: one of another length is
            // from another model, and its cosine of 0 would tie every
            // passage and rank them by row (review of #467). None of them,
            // and the search is by words — and says so.
            let mut ranked: Vec<(i64, f32)> = corpus
                .iter()
                .filter_map(|(id, v)| {
                    let v = v.as_ref().filter(|v| v.len() == q.len())?;
                    Some((*id, embed::cosine(q, v)))
                })
                .collect();
            // A ranking over a few embedded passages is not a ranking of the
            // files: its best would tie the best word hit on no merit (review
            // of #467). Under half embedded, words alone — and said.
            by_meaning = ranked.len() * 2 >= corpus.len() && !ranked.is_empty();
            if !by_meaning {
                ranked.clear();
            }
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (rank, (id, _)) in ranked.iter().take(k * 8).enumerate() {
                *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
            }
        }
        if let Some(fts) = fts_query(query) {
            let wanted: std::collections::HashSet<i64> = corpus.iter().map(|c| c.0).collect();
            // Within the asked files in the query itself: a store's other
            // files must not crowd these out of the limit (review of #467).
            // `wanted` stays as the belt.
            let marks = vec!["?"; shas.len()].join(", ");
            let mut q = self.conn.prepare(&format!(
                "SELECT rowid FROM passages_fts WHERE passages_fts MATCH ?1 \
                 AND rowid IN (SELECT id FROM passages WHERE sha IN ({marks})) \
                 ORDER BY bm25(passages_fts) LIMIT 500"
            ))?;
            let args: Vec<&dyn rusqlite::ToSql> = std::iter::once(&fts as &dyn rusqlite::ToSql)
                .chain(shas.iter().map(|s| s as &dyn rusqlite::ToSql))
                .collect();
            let ids = q.query_map(args.as_slice(), |r| r.get::<_, i64>(0))?;
            let mut rank = 0usize;
            for id in ids {
                let id = id?;
                // Outside the caller's files: not theirs to see, and not
                // counted against the rank of those that are.
                if !wanted.contains(&id) {
                    continue;
                }
                *scores.entry(id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
                rank += 1;
                if rank >= k * 8 {
                    break;
                }
            }
        }
        let mut best: Vec<(i64, f32)> = scores.into_iter().collect();
        best.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        best.truncate(k);
        let mut hits = Vec::with_capacity(best.len());
        let mut q = self.conn.prepare(
            "SELECT p.sha, p.page, d.pages, p.text FROM passages p JOIN docs d ON d.sha = p.sha WHERE p.id = ?1",
        )?;
        for (id, _) in best {
            hits.push(q.query_row([id], |r| {
                Ok(Hit {
                    sha: r.get(0)?,
                    page: r.get::<_, Option<i64>>(1)?.map(|n| n as u32),
                    pages: r.get::<_, Option<i64>>(2)?.map(|n| n as u32),
                    text: r.get(3)?,
                })
            })?);
        }
        Ok((
            hits,
            if by_meaning {
                Found::Both
            } else {
                Found::WordsOnly
            },
        ))
    }
}

/// Embed whatever of `sha` has no vector yet. Off the runtime for the
/// database, on it for the server. An error leaves the passages searchable
/// by words, and the next pass tries again.
pub async fn embed_missing(store: PathBuf, sha: String, embedder: &Embedder) -> Result<usize> {
    let (s, h) = (store.clone(), sha.clone());
    let todo = tokio::task::spawn_blocking(move || Index::open(&s)?.unembedded(&h)).await??;
    let mut done = 0usize;
    for batch in todo.chunks(BATCH) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let vectors = embedder.embed(&texts, Task::Passage).await?;
        // Position ties a vector to its passage: a short reply is refused,
        // never zipped short (the rule `embed` keeps).
        if vectors.len() != batch.len() {
            anyhow::bail!(
                "asked for {} embeddings, got {}",
                batch.len(),
                vectors.len()
            );
        }
        let rows: Vec<(i64, Vec<u8>)> = batch
            .iter()
            .zip(&vectors)
            .map(|((id, _), v)| (*id, embed::to_blob(v)))
            .collect();
        let dims = vectors.first().map(Vec::len).unwrap_or(0);
        let s = store.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut index = Index::open(&s)?;
            index.claim_identity(dims)?;
            let tx = index.conn.transaction()?;
            for (id, blob) in rows {
                tx.execute(
                    "UPDATE passages SET vec = ?1 WHERE id = ?2",
                    params![blob, id],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await??;
        done += batch.len();
    }
    Ok(done)
}

/// Index one of a persona's files from its rendered text — `files::read`
/// with every page — and embed what is not yet embedded. The passages are
/// kept even when the embeddings server does not answer: words still find
/// them, and the next pass embeds them. Returns how many were embedded.
pub async fn index_file(
    store: PathBuf,
    src: &Source,
    rendered: String,
    embedder: Option<&Embedder>,
) -> Result<usize> {
    let src = src.clone();
    let s = store.clone();
    let sha = tokio::task::spawn_blocking(move || -> Result<String> {
        let sha = sha_of(&src).with_context(|| format!("hashing {}", src.name))?;
        let mut index = Index::open(&s)?;
        // Cut again when the rendering changed, not only the bytes: a scan
        // past `max_ocr_pages` renders its later pages as placeholders until
        // they are read, and an index kept from that first pass would never
        // learn them (review of #467). Same rendering, nothing to do.
        let now = crate::document::sha256_hex(rendered.as_bytes());
        if index.rendered(&sha)?.as_deref() != Some(now.as_str()) {
            index.put(&sha, &rendered)?;
        }
        Ok(sha)
    })
    .await??;
    match embedder {
        Some(e) => embed_missing(store, sha, e).await,
        None => Ok(0),
    }
}

/// `file_search`: passages of the persona's files that answer a question.
/// Persona-only, built per chat beside `file_read`, over the same listing.
pub struct FileSearch {
    store: PathBuf,
    persona: String,
    embedder: Option<Embedder>,
    /// The document reader's cap, `0` while document reading is off.
    max_doc_bytes: u64,
}

impl FileSearch {
    /// `max_doc_bytes` is the document reader's cap — `0` while document
    /// reading is off — so a file nothing could index is never hashed to
    /// find out ([`may_be_indexed`]).
    pub fn new(
        store: PathBuf,
        persona: String,
        embedder: Option<Embedder>,
        max_doc_bytes: u64,
    ) -> Self {
        FileSearch {
            store,
            persona,
            embedder,
            max_doc_bytes,
        }
    }
}

/// Whether `src` could be in the index at all, asked as `files::readiness`
/// asks before it hashes: within its reader's cap, and — for a document —
/// a format the reader takes, from its first kilobyte. Asked before hashing
/// because the hash is a whole read, on a path the model can call as often
/// as it likes (review of #467). `max_doc_bytes` is `0` where no document
/// is ever indexed: reading off, or the extraction cache off.
pub fn may_be_indexed(src: &Source, max_doc_bytes: u64) -> bool {
    match src.kind {
        super::files::Kind::Text => src.bytes <= super::files::MAX_TEXT_BYTES,
        super::files::Kind::Document => {
            src.bytes <= max_doc_bytes
                && super::files::head(&src.path).is_ok_and(|h| crate::document::kind_of(&h).is_ok())
        }
    }
}

/// Passages a search returns, at most.
const MAX_HITS: usize = 12;

#[async_trait]
impl Tool for FileSearch {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Find the passages of your files that answer a question — for when your files are too \
         long to read whole. Ask in plain words; each passage comes back with its file and page, \
         to read around with `file_read` and to cite as [file, p. N: \"an exact quote\"]."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What you are looking for, in plain words"},
                "limit": {"type": "integer", "description": "Passages to return, 1 to 12. Default 6"}
            },
            "required": ["query"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// As `file_read`: the owner's material in a third party's words, and
    /// no way out — the query goes to the loopback embeddings server and
    /// nowhere else.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private().untrusted()
    }

    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(self)
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(query) = input
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
        else {
            return Ok(ToolOutput::err(
                "`query` is required: what you are looking for.",
            ));
        };
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, MAX_HITS))
            .unwrap_or(6);
        // The persona's files, each hashed: the only passages it may see.
        let (store, persona, cap) = (self.store.clone(), self.persona.clone(), self.max_doc_bytes);
        let listed = tokio::task::spawn_blocking(move || {
            let st = Store::load(&store);
            let sources = st.get(&persona).map(|p| list(&roots(&st, p)))?;
            let index = Index::open(&store).ok()?;
            let mut named: HashMap<String, Source> = HashMap::new();
            let mut unindexed = Vec::new();
            for src in sources {
                if !may_be_indexed(&src, cap) {
                    unindexed.push(src.name);
                    continue;
                }
                match sha_of(&src) {
                    Some(sha) if index.has(&sha).unwrap_or(false) => {
                        named.entry(sha).or_insert(src);
                    }
                    _ => unindexed.push(src.name),
                }
            }
            Some((named, unindexed))
        })
        .await;
        let Ok(Some((named, unindexed))) = listed else {
            return Ok(ToolOutput::err(
                "Your files could not be searched just now; read them with `file_read`, or tell the owner.",
            ));
        };
        if named.is_empty() {
            return Ok(ToolOutput::err(if unindexed.is_empty() {
                "You have no files to search.".to_string()
            } else {
                "None of your files can be searched yet. Read them with `file_read`.".to_string()
            }));
        }
        // Meaning when the server answers; words alone when it does not.
        let qvec = match &self.embedder {
            Some(e) => e.query(query).await.ok(),
            None => None,
        };
        // The query is the one embedding every search makes, so it is where
        // the index learns the model changed — even one with nothing left
        // to embed (review of #467). A changed identity drops the stored
        // vectors: this search is by words, and the files are embedded
        // again as they come round.
        if let Some(q) = &qvec {
            let (store, dims) = (self.store.clone(), q.len());
            let _ = tokio::task::spawn_blocking(move || Index::open(&store)?.claim_identity(dims))
                .await;
        }
        let shas: Vec<String> = named.keys().cloned().collect();
        let (store, q) = (self.store.clone(), query.to_string());
        let found = tokio::task::spawn_blocking(move || {
            Index::open(&store)?.search(&shas, &q, qvec.as_deref(), limit)
        })
        .await;
        let (hits, how) = match found {
            Ok(Ok(found)) => found,
            _ => {
                return Ok(ToolOutput::err(
                    "The search failed just now; read your files with `file_read` meanwhile.",
                ))
            }
        };
        // The harness's own notes first: before any `document:` header,
        // where `persona::cite` reads nothing as a file's words — after the
        // last hit they read as that page's text (review of #467).
        let mut out = String::new();
        if how == Found::WordsOnly && !hits.is_empty() {
            out.push_str(if self.embedder.is_some() {
                "(Found by words only: the search by meaning is unavailable just now.)\n"
            } else {
                "(Found by words only: no search by meaning is set up.)\n"
            });
        }
        if !unindexed.is_empty() {
            out.push_str(&format!(
                "(Not searchable yet — read with `file_read`: {}.)\n",
                unindexed
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !out.is_empty() {
            out.push('\n');
        }
        if hits.is_empty() {
            out.push_str(
                "No passage of your files matches that. Try other words, or read with `file_read`.",
            );
        }
        for hit in &hits {
            let name = named.get(&hit.sha).map(|s| s.name.as_str()).unwrap_or("?");
            // `file_read`'s header and page marker, so a quote taken from
            // here is checked like any other (`persona::cite`).
            out.push_str(&format!("document: {name} \u{b7} search result\n"));
            match (hit.page, hit.pages) {
                (Some(page), Some(pages)) => out.push_str(&format!(
                    "\n=== page {page} of {pages} \u{b7} a passage found by search ===\n"
                )),
                _ => out.push('\n'),
            }
            out.push_str(hit.text.trim_end());
            out.push_str("\n\n");
        }
        Ok(ToolOutput::ok(out.trim_end().to_string()).from_outside())
    }
}

/// A file's text cut into passages: paragraphs gathered up to about
/// [`PASSAGE_CHARS`], a paragraph longer than that cut at sentence ends,
/// each passage after the first opening with the last [`OVERLAP_CHARS`] of
/// the one before.
fn passages_of(text: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if para.chars().count() <= PASSAGE_CHARS {
            pieces.push(para.to_string());
            continue;
        }
        // A long paragraph, by sentence; a sentence longer than a passage,
        // by characters.
        let mut cur = String::new();
        for sentence in para.split_inclusive(['.', '?', '!']) {
            if !cur.is_empty() && cur.chars().count() + sentence.chars().count() > PASSAGE_CHARS {
                pieces.push(std::mem::take(&mut cur).trim().to_string());
            }
            cur.push_str(sentence);
            while cur.chars().count() > PASSAGE_CHARS {
                let cut: String = cur.chars().take(PASSAGE_CHARS).collect();
                cur = cur.chars().skip(PASSAGE_CHARS).collect();
                pieces.push(cut.trim().to_string());
            }
        }
        if !cur.trim().is_empty() {
            pieces.push(cur.trim().to_string());
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for piece in pieces {
        if !cur.is_empty() && cur.chars().count() + piece.chars().count() + 2 > PASSAGE_CHARS {
            let tail: String = {
                let n = cur.chars().count();
                cur.chars().skip(n.saturating_sub(OVERLAP_CHARS)).collect()
            };
            out.push(std::mem::take(&mut cur));
            // Start the overlap at a word.
            cur = match tail.find(' ') {
                Some(i) => tail[i + 1..].to_string(),
                None => tail,
            };
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(&piece);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// A query as FTS5 reads it: its words, each quoted (so nothing in them is
/// FTS syntax), any of them. `None` when it has no word.
pub(crate) fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        // A single letter is noise; a single digit is the "3" of "table 3".
        .filter(|w| {
            w.chars().count() > 1 || (!w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|w| format!("\"{}\"", w.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mecha-search-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const PAPER: &str = "document: kelp.pdf \u{b7} pdf \u{b7} 2 page(s) \u{b7} sha256 x\n\
        \n=== page 1 of 2 \u{b7} text layer (the file's own words) ===\n\
        Sea urchins graze kelp holdfasts at night.\n\
        \n=== page 2 of 2 \u{b7} text layer (the file's own words) ===\n\
        Sea otters keep urchin barrens in check.\n";

    #[test]
    fn a_long_text_is_cut_into_overlapping_passages_at_sentence_ends() {
        let sentence = "Urchins graze the kelp holdfasts at night. ";
        let text = sentence.repeat(80);
        let got = passages_of(&text);
        assert!(got.len() >= 3, "{}", got.len());
        assert!(got
            .iter()
            .all(|p| p.chars().count() <= PASSAGE_CHARS + OVERLAP_CHARS + 2));
        // The second passage opens with the end of the first.
        let tail: String = got[0]
            .chars()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        assert!(got[1].contains(tail.trim()), "overlap");
        assert_eq!(passages_of("one.\n\ntwo."), ["one.\n\ntwo."]);
        assert!(passages_of("   \n\n  ").is_empty());
    }

    #[test]
    fn a_query_is_words_never_fts_syntax() {
        assert_eq!(
            fts_query("urchins AND \"kelp\" NEAR(x)").unwrap(),
            "\"urchins\" OR \"and\" OR \"kelp\" OR \"near\""
        );
        assert_eq!(fts_query("? !"), None);
    }

    /// Words alone find the exact term, page by page; a passage of a file
    /// the caller did not list is never returned, even when it matches
    /// better.
    #[test]
    fn words_find_passages_of_the_listed_files_only() {
        let store = scratch();
        let mut index = Index::open(&store).unwrap();
        assert_eq!(index.put("aaa", PAPER).unwrap(), 2);
        index
            .put(
                "bbb",
                "document: other.md \u{b7} text\n\nOtters otters otters keep urchins in check.",
            )
            .unwrap();
        assert!(index.has("aaa").unwrap());
        assert!(!index.complete("aaa").unwrap(), "no vectors yet");
        let (hits, found) = index.search(&["aaa".into()], "otters", None, 5).unwrap();
        assert_eq!(found, Found::WordsOnly);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(
            (hits[0].sha.as_str(), hits[0].page, hits[0].pages),
            ("aaa", Some(2), Some(2))
        );
        assert!(hits[0].text.contains("otters keep urchin barrens"));
        // Put again: replaced, not doubled — in the words index too.
        index.put("aaa", PAPER).unwrap();
        let (hits, _) = index
            .search(&["aaa".into(), "bbb".into()], "urchins", None, 10)
            .unwrap();
        let pages: Vec<(&str, Option<u32>)> =
            hits.iter().map(|h| (h.sha.as_str(), h.page)).collect();
        // Page 2 says "urchin": one hit each, and none doubled by the put.
        assert_eq!(pages.len(), 2, "{pages:?}");
        assert!(pages.contains(&("aaa", Some(1))) && pages.contains(&("bbb", None)));
        std::fs::remove_dir_all(store).ok();
    }

    /// Meaning and words fused: with vectors in, the passage nearest the
    /// query's vector ranks first even when its words do not match; and a
    /// change of vector length drops what was stored rather than compare.
    #[test]
    fn meaning_ranks_by_vector_and_another_identity_drops_the_vectors() {
        let store = scratch();
        let mut index = Index::open(&store).unwrap();
        index.put("aaa", PAPER).unwrap();
        index.claim_identity(2).unwrap();
        let rows = index.unembedded("aaa").unwrap();
        // Page 1 points one way, page 2 another.
        for ((id, _), v) in rows.iter().zip([[1.0f32, 0.0], [0.0, 1.0]]) {
            index
                .conn
                .execute(
                    "UPDATE passages SET vec = ?1 WHERE id = ?2",
                    params![embed::to_blob(&v), id],
                )
                .unwrap();
        }
        assert!(index.complete("aaa").unwrap());
        let (hits, found) = index
            .search(&["aaa".into()], "predators", Some(&[0.1, 0.9]), 1)
            .unwrap();
        assert_eq!(found, Found::Both);
        assert_eq!(hits[0].page, Some(2), "nearest by meaning, no shared word");
        assert!(
            index.claim_identity(3).unwrap(),
            "a new length drops the old vectors"
        );
        assert!(!index.complete("aaa").unwrap());
        std::fs::remove_dir_all(store).ok();
    }

    /// `file_search` sees the persona's own folders only: a passage of
    /// another persona's file never comes back, however well it matches.
    /// Its result renders as pages, so a quote taken from it is checked
    /// like `file_read`'s (`persona::cite`).
    #[tokio::test]
    async fn a_persona_searches_its_own_files_and_its_quotes_are_checked() {
        use super::super::tests_support::*;
        use crate::persona::{create, ensure_layout};
        let dir = scratch();
        ensure_layout(&dir).unwrap();
        create(&dir, &no_lib(), new("mara")).unwrap();
        create(&dir, &no_lib(), new("rhea")).unwrap();
        std::fs::write(
            dir.join("mara/files/urchins.md"),
            "Sea urchins graze kelp holdfasts at night.",
        )
        .unwrap();
        std::fs::write(
            dir.join("rhea/files/otters.md"),
            "Otters eat urchins urchins urchins.",
        )
        .unwrap();
        let store = Store::load(&dir);
        for name in ["mara", "rhea"] {
            for src in list(&roots(&store, store.get(name).unwrap())) {
                let text = super::super::files::read(&src, None, "all", None)
                    .await
                    .unwrap();
                index_file(dir.clone(), &src, text, None).await.unwrap();
            }
        }
        let tool = FileSearch::new(dir.clone(), "mara".into(), None, 0);
        let ctx = ToolCtx::default();
        let out = tool.call(json!({"query": "urchins"}), &ctx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.external, "a file's words are outside content");
        assert!(
            out.content.contains("document: urchins.md"),
            "{}",
            out.content
        );
        assert!(
            !out.content.contains("Otters"),
            "another persona's file: {}",
            out.content
        );
        assert!(out.content.contains("by words only"), "no embedder here");

        // The quote a persona takes from it is checked against it.
        let ask = crate::message::Message::assistant(vec![crate::message::Block::ToolUse {
            id: "s1".into(),
            name: "file_search".into(),
            input: json!({"query": "urchins"}),
        }]);
        let back = crate::message::Message {
            role: crate::message::Role::User,
            content: vec![crate::message::Block::ToolResult {
                tool_use_id: "s1".into(),
                content: out.content.clone(),
                is_error: false,
            }],
            ..crate::message::Message::user("")
        };
        let reply = crate::message::Message::assistant(vec![crate::message::Block::Text {
            text: "[urchins.md: \"urchins graze kelp holdfasts\"] \
                   [urchins.md: \"Found by words only: the search by meaning\"]"
                .into(),
        }]);
        let got = super::super::cite::check_conversation(&[
            crate::message::Message::user("go"),
            ask,
            back,
            reply,
        ]);
        assert_eq!(
            got[0].verdict,
            super::super::cite::Verdict::Quoted { found: None }
        );
        // The harness's note is not the file's words (review of #467).
        assert_eq!(got[1].verdict, super::super::cite::Verdict::NotFound);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Review of #467: stored vectors of another length are never ranked —
    /// a query from a new model finds by words, says so, and does not hand
    /// out row-order "meaning" hits.
    #[test]
    fn vectors_of_another_length_are_not_ranked() {
        let store = scratch();
        let mut index = Index::open(&store).unwrap();
        index.put("aaa", PAPER).unwrap();
        index.claim_identity(2).unwrap();
        for (id, _) in index.unembedded("aaa").unwrap() {
            index
                .conn
                .execute(
                    "UPDATE passages SET vec = ?1 WHERE id = ?2",
                    params![embed::to_blob(&[1.0, 0.0]), id],
                )
                .unwrap();
        }
        let (hits, found) = index
            .search(&["aaa".into()], "otters", Some(&[0.0, 1.0, 0.0]), 5)
            .unwrap();
        assert_eq!(found, Found::WordsOnly);
        assert_eq!(hits.len(), 1, "the word hit only: {hits:?}");
        assert_eq!(hits[0].page, Some(2));
        assert_eq!(fts_query("table 3").unwrap(), "\"table\" OR \"3\"");
        std::fs::remove_dir_all(store).ok();
    }

    /// Review of #467, pass 2: a rendering that grows — a scan's later
    /// pages read since — is cut again, keeping the vectors of passages cut
    /// the same; and the word search is within the asked files, however
    /// many better matches the store holds elsewhere.
    #[tokio::test]
    async fn a_grown_rendering_is_recut_and_others_cannot_crowd_the_words_out() {
        use super::super::tests_support::*;
        use crate::persona::ensure_layout;
        let dir = scratch();
        ensure_layout(&dir).unwrap();
        std::fs::write(dir.join("files/scan.md"), "bytes that do not change").unwrap();
        let src = list(&[(String::new(), dir.join("files"))]).pop().unwrap();
        let sha = sha_of(&src).unwrap();
        let first = "document: scan.pdf \u{b7} pdf \u{b7} 2 page(s) \u{b7} sha256 x\n\
            \n=== page 1 of 2 \u{b7} text layer (the file's own words) ===\nUrchins graze kelp.\n\
            \n=== page 2 of 2 \u{b7} text layer (the file's own words) ===\n(no text layer on this page)\n";
        index_file(dir.clone(), &src, first.into(), None)
            .await
            .unwrap();
        {
            let mut index = Index::open(&dir).unwrap();
            index.claim_identity(2).unwrap();
            let rows = index.unembedded(&sha).unwrap();
            for (id, _) in rows {
                index
                    .conn
                    .execute(
                        "UPDATE passages SET vec = ?1 WHERE id = ?2",
                        params![embed::to_blob(&[1.0, 0.0]), id],
                    )
                    .unwrap();
            }
        }
        let grown = first.replace(
            "(no text layer on this page)",
            "Otters keep the barrens in check.",
        );
        index_file(dir.clone(), &src, grown, None).await.unwrap();
        let index = Index::open(&dir).unwrap();
        let (hits, _) = index
            .search(std::slice::from_ref(&sha), "otters", None, 3)
            .unwrap();
        assert_eq!(hits.len(), 1, "the page read since is searchable: {hits:?}");
        let left = index.unembedded(&sha).unwrap();
        assert_eq!(
            left.len(),
            1,
            "only the new passage needs a vector: {left:?}"
        );
        assert!(left[0].1.contains("Otters"));

        // Six hundred better word matches elsewhere in the store.
        let mut index = Index::open(&dir).unwrap();
        for n in 0..600 {
            index
                .put(
                    &format!("other{n}"),
                    "document: o.md \u{b7} text\n\nOtters otters otters otters.",
                )
                .unwrap();
        }
        let (hits, _) = index
            .search(std::slice::from_ref(&sha), "otters", None, 3)
            .unwrap();
        assert_eq!(hits.len(), 1, "not crowded out: {hits:?}");
        std::fs::remove_dir_all(dir).ok();
    }

    /// Review of #467, pass 3: a file over its reader's cap is never
    /// hashed by a search — it could not be indexed, so it is "not
    /// searchable" without the read.
    #[test]
    fn a_file_past_its_cap_is_not_a_search_candidate() {
        let at = |kind, bytes| Source {
            name: "f".into(),
            path: PathBuf::from("/nowhere"),
            bytes,
            kind,
        };
        use super::super::files::{Kind, MAX_TEXT_BYTES};
        assert!(may_be_indexed(&at(Kind::Text, MAX_TEXT_BYTES), 0));
        assert!(!may_be_indexed(&at(Kind::Text, MAX_TEXT_BYTES + 1), 0));
        // A document within the cap must also be a format the reader takes.
        let dir = scratch();
        std::fs::write(dir.join("p.pdf"), b"%PDF-1.4 a paper").unwrap();
        std::fs::write(dir.join("q.pdf"), b"plain words, not a pdf").unwrap();
        let doc = |name: &str, bytes| Source {
            name: name.into(),
            path: dir.join(name),
            bytes,
            kind: Kind::Document,
        };
        assert!(may_be_indexed(&doc("p.pdf", 16), 100));
        assert!(!may_be_indexed(&doc("p.pdf", 101), 100), "over the cap");
        assert!(
            !may_be_indexed(&doc("q.pdf", 22), 100),
            "no format the reader takes"
        );
        std::fs::remove_dir_all(dir).ok();
        assert!(
            !may_be_indexed(&at(Kind::Document, 1), 0),
            "reading switched off"
        );
    }

    /// Review of #467: the vector leg is scoped by the asked hashes as the
    /// words leg is — a passage of another file nearer the query is never
    /// returned — and a ranking over a sliver of embedded passages is not
    /// used.
    #[test]
    fn meaning_is_scoped_too_and_a_sliver_of_vectors_is_not_a_ranking() {
        let store = scratch();
        let mut index = Index::open(&store).unwrap();
        index.put("aaa", PAPER).unwrap();
        index
            .put(
                "bbb",
                "document: o.md \u{b7} text\n\nSomething else entirely.",
            )
            .unwrap();
        index.claim_identity(2).unwrap();
        let set = |index: &Index, sha: &str, v: [f32; 2]| {
            for (id, _) in index.unembedded(sha).unwrap() {
                index
                    .conn
                    .execute(
                        "UPDATE passages SET vec = ?1 WHERE id = ?2",
                        params![embed::to_blob(&v), id],
                    )
                    .unwrap();
            }
        };
        set(&index, "bbb", [0.0, 1.0]);
        // Only one of aaa's two passages embedded: half, so meaning is used.
        let first = index.unembedded("aaa").unwrap()[0].0;
        index
            .conn
            .execute(
                "UPDATE passages SET vec = ?1 WHERE id = ?2",
                params![embed::to_blob(&[1.0, 0.0]), first],
            )
            .unwrap();
        let (hits, found) = index
            .search(&["aaa".into()], "zzz", Some(&[0.0, 1.0]), 5)
            .unwrap();
        assert_eq!(found, Found::Both);
        assert!(
            hits.iter().all(|h| h.sha == "aaa"),
            "bbb is nearer and not asked: {hits:?}"
        );
        // Add a third passage-less file's worth: aaa's corpus is now a sliver.
        index.put("aaa", &format!("{PAPER}\n=== page 3 of 3 \u{b7} text layer ===\nA.\n\n\n=== page 4 of 4 \u{b7} text layer ===\nB.\n")).unwrap();
        let (_, found) = index
            .search(&["aaa".into()], "zzz", Some(&[0.0, 1.0]), 5)
            .unwrap();
        assert_eq!(found, Found::WordsOnly, "under half embedded");
        std::fs::remove_dir_all(store).ok();
    }
}
