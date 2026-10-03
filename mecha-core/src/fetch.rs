//! Pinned downloads (FEATURES-DESIGN.md §10.2 items 1 and 6): the one
//! Hugging Face hub resolver, and a resumable, hash-verified fetch of a file a
//! [`crate::recommend::Source`] pins.
//!
//! **What may be fetched is the registry's, never a caller's.** The public
//! entry points take a pinned `Source`'s parts — repository, revision, path,
//! sha256, size — so a URL from config, a project file or a model has no way
//! in (§8). The serving channel's own hash is never trusted here: the sha256
//! is the one a reviewer committed, and a file that does not match it is
//! deleted, not kept (F10's tag-confirmed upgrade is the one exception, and it
//! lives in 7b, not here).
//!
//! **Interrupted is resumable, never finished.** Bytes land in a `.part` file
//! beside the destination; a later call sends a `Range` from where it stopped,
//! re-hashing what is already on disk first, and only a file whose size and
//! sha256 both match is renamed into place. A server that ignores the range
//! starts the file again rather than appending to it.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

use crate::recommend::HubFile;

/// Where Hugging Face's own files are fetched from.
const HF_BASE: &str = "https://huggingface.co";

/// The hub directory, resolved the one way the downloader, the unit
/// templates and the launchers share (§10.2 item 6): `HF_HUB` (mecha's own),
/// then the `hf` CLI's `HF_HUB_CACHE`, then `HF_HOME/hub`, then
/// `XDG_CACHE_HOME/huggingface/hub` — `hf`'s own default for `HF_HOME` —
/// then `~/.cache/huggingface/hub`.
pub fn hub_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home directory to put the model cache under")?;
    Ok(hub_dir_from(
        |k| std::env::var_os(k).filter(|v| !v.is_empty()),
        &home,
    ))
}

fn hub_dir_from(env: impl Fn(&str) -> Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    if let Some(v) = env("HF_HUB") {
        return PathBuf::from(v);
    }
    if let Some(v) = env("HF_HUB_CACHE") {
        return PathBuf::from(v);
    }
    if let Some(v) = env("HF_HOME") {
        return PathBuf::from(v).join("hub");
    }
    if let Some(v) = env("XDG_CACHE_HOME") {
        return PathBuf::from(v).join("huggingface/hub");
    }
    home.join(".cache/huggingface/hub")
}

/// `models--org--name`, the cache's directory for a repository.
fn repo_dir(hub: &Path, repo: &str) -> PathBuf {
    hub.join(format!("models--{}", repo.replace('/', "--")))
}

/// Where a pinned file is read from: the snapshot path a launcher globs.
pub fn snapshot_path(hub: &Path, repo: &str, revision: &str, file: &HubFile) -> PathBuf {
    repo_dir(hub, repo)
        .join("snapshots")
        .join(revision)
        .join(file.path)
}

/// Where its bytes live: the blob named by its sha256, as `hf` stores it.
fn blob_path(hub: &Path, repo: &str, file: &HubFile) -> PathBuf {
    repo_dir(hub, repo).join("blobs").join(file.sha256)
}

/// What the cache holds for one pinned file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cached {
    /// The snapshot path resolves to a file of the pinned size whose sha256
    /// matched the pin — priced at zero in a plan (§10.2 item 4).
    Verified,
    /// Something is there, and it is not the pinned file: a different size or
    /// a different hash. Neither provided nor handed to a launcher.
    Mismatch,
    /// A plain file of the pinned size, not a blob named by its sha256 — put
    /// there by hand. The cheap check does not read it (22 GB for the chat
    /// pin); only a hash can say whether it is the pin.
    Unverified,
    Absent,
}

/// Look in the cache for a pinned file. With `hash`, read it all and compare
/// the sha256 (seconds for the 22 GB chat model). Without, never read a
/// byte: the size and the blob's name — `hf` names a blob by its sha256 —
/// stand in, a file whose name and size agree with the pin is taken as it,
/// and a plain file of the right size is `Unverified`.
pub fn cached(
    hub: &Path,
    repo: &str,
    revision: &str,
    file: &HubFile,
    hash: bool,
) -> Result<Cached> {
    if !plain_relative(file.path) || !plain_relative(repo) || !plain_relative(revision) {
        bail!("refusing to look up a pin whose repository, revision or path is not a plain relative name");
    }
    let snap = snapshot_path(hub, repo, revision, file);
    let meta = match std::fs::metadata(&snap) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Cached::Absent),
        Err(e) => return Err(e).with_context(|| format!("reading {}", snap.display())),
    };
    if meta.len() != file.bytes {
        return Ok(Cached::Mismatch);
    }
    if hash {
        return Ok(if sha256_file(&snap)? == file.sha256 {
            Cached::Verified
        } else {
            Cached::Mismatch
        });
    }
    let named = std::fs::canonicalize(&snap)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    Ok(match named {
        Some(n) if n == file.sha256 => Cached::Verified,
        _ => Cached::Unverified,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A registry path is relative and plain: no `..`, no root, no prefix — it
/// is joined under the cache, and a pin is reviewed, but the join is checked
/// anyway.
fn plain_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Fetch one pinned Hugging Face file into the cache, as `hf` would lay it
/// out: the bytes at `blobs/<sha256>`, the snapshot path a relative link to
/// them. Already there and matching: nothing is fetched. Interrupted: the
/// next call resumes. `progress` sees the bytes on disk so far.
pub async fn fetch_hub_file(
    hub: &Path,
    repo: &'static str,
    revision: &'static str,
    file: &HubFile,
    progress: &mut dyn FnMut(u64),
) -> Result<PathBuf> {
    fetch_hub_file_from(HF_BASE, hub, repo, revision, file, progress).await
}

async fn fetch_hub_file_from(
    base: &str,
    hub: &Path,
    repo: &'static str,
    revision: &'static str,
    file: &HubFile,
    progress: &mut dyn FnMut(u64),
) -> Result<PathBuf> {
    if !plain_relative(file.path) || !plain_relative(repo) || !plain_relative(revision) {
        bail!("refusing a pin whose repository, revision or path is not a plain relative name: {repo} {revision} {}", file.path);
    }
    let snap = snapshot_path(hub, repo, revision, file);
    match cached(hub, repo, revision, file, false)? {
        Cached::Verified => return Ok(snap),
        // A hand-placed file of the right size: only its hash can say.
        Cached::Unverified if cached(hub, repo, revision, file, true)? == Cached::Verified => {
            return Ok(snap);
        }
        _ => {}
    }
    let blob = blob_path(hub, repo, file);
    if !(blob.exists()
        && std::fs::metadata(&blob)?.len() == file.bytes
        && sha256_file(&blob)? == file.sha256)
    {
        let url = format!("{base}/{repo}/resolve/{revision}/{}", file.path);
        fetch_to(&url, &blob, file.sha256, file.bytes, progress).await?;
    }
    link_snapshot(&snap, &blob)?;
    Ok(snap)
}

/// Point the snapshot path at the blob with a relative link, as `hf` does,
/// so the cache moves as a whole. A file already at the snapshot path is not
/// mecha's to replace (§10.2 item 4: never overwrite what mecha did not
/// write) — `cached` has already said it does not match.
fn link_snapshot(snap: &Path, blob: &Path) -> Result<()> {
    let parent = snap.parent().context("a snapshot path with no parent")?;
    std::fs::create_dir_all(parent)?;
    match std::fs::symlink_metadata(snap) {
        Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(snap)?,
        Ok(_) => bail!(
            "{} is a file mecha did not write and does not match its pin; move it aside to fetch the pinned one",
            snap.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let rel = relative_to(blob, parent);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&rel, snap)?;
    #[cfg(not(unix))]
    std::fs::copy(blob, snap).map(|_| ())?;
    Ok(())
}

/// `blob` relative to `dir`, both under one repository directory.
fn relative_to(blob: &Path, dir: &Path) -> PathBuf {
    let b: Vec<_> = blob.components().collect();
    let d: Vec<_> = dir.components().collect();
    let common = b.iter().zip(&d).take_while(|(x, y)| x == y).count();
    let mut rel = PathBuf::new();
    for _ in common..d.len() {
        rel.push("..");
    }
    for c in &b[common..] {
        rel.push(c);
    }
    rel
}

/// The resumable, verified fetch every entry point ends in. `dest` is written
/// only by a rename of a `.part` file whose size and sha256 match.
async fn fetch_to(
    url: &str,
    dest: &Path,
    sha256: &str,
    bytes: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let parent = dest.parent().context("a destination with no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name()
            .context("a destination with no name")?
            .to_string_lossy()
    ));

    // What is already on disk counts toward the hash, so a resumed file is
    // verified whole, never only its tail.
    let mut have = match tokio::fs::metadata(&part).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };
    if have > bytes {
        tokio::fs::remove_file(&part).await?;
        have = 0;
    }
    let mut hasher = Sha256::new();
    if have > 0 {
        let p = part.clone();
        hasher = tokio::task::spawn_blocking(move || -> Result<Sha256> {
            use std::io::Read;
            let mut f = std::fs::File::open(&p)?;
            let mut h = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(h)
        })
        .await??;
    }

    if have < bytes {
        // A server that accepts and then goes quiet ends as a resumable
        // error, never a hang.
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()?;
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("fetching {url}"))?;
        let status = resp.status();
        // A partial answer must start where the file stopped; one that does
        // not is refused, rather than appended and later thrown away whole by
        // the hash check.
        let resumes_here = status == reqwest::StatusCode::PARTIAL_CONTENT
            && content_range_start(resp.headers()) == Some(have);
        if status == reqwest::StatusCode::PARTIAL_CONTENT && !resumes_here {
            bail!(
                "{url} answered a range that does not start at byte {have}; delete {} to start it again",
                part.display()
            );
        }
        let mut file = if have > 0 && resumes_here {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .await?
        } else if status.is_success() {
            // A fresh start, or a server that ignored the range: begin again.
            have = 0;
            hasher = Sha256::new();
            tokio::fs::File::create(&part).await?
        } else {
            bail!("{url} answered {status}");
        };
        progress(have);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            // A connection cut mid-file arrives as an error here, not as a
            // short end: keep what landed, and say how to go on.
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    file.flush().await?;
                    file.sync_all().await?;
                    bail!(
                        "{url} stopped at {have} of {bytes} bytes ({e}); run the same command again to resume"
                    );
                }
            };
            have += chunk.len() as u64;
            if have > bytes {
                drop(file);
                let _ = tokio::fs::remove_file(&part).await;
                bail!("{url} sent more than the pinned {bytes} bytes — refused, and the partial file deleted");
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            progress(have);
        }
        file.flush().await?;
        file.sync_all().await?;
    }

    if have != bytes {
        bail!("{url} stopped at {have} of {bytes} bytes; run the same command again to resume");
    }
    let got = hex(&hasher.finalize());
    if got != sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        bail!("{url}'s sha256 is {got}, not the pinned {sha256} — refused, and the file deleted");
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

/// The first byte of a `Content-Range: bytes START-END/TOTAL`.
fn content_range_start(h: &reqwest::header::HeaderMap) -> Option<u64> {
    let v = h.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    v.strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_hub_is_resolved_in_one_order() {
        let home = Path::new("/home/u");
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| std::ffi::OsString::from(v))
            }
        };
        assert_eq!(
            hub_dir_from(env(&[]), home),
            PathBuf::from("/home/u/.cache/huggingface/hub")
        );
        assert_eq!(
            hub_dir_from(env(&[("XDG_CACHE_HOME", "/x")]), home),
            PathBuf::from("/x/huggingface/hub")
        );
        assert_eq!(
            hub_dir_from(env(&[("XDG_CACHE_HOME", "/x"), ("HF_HOME", "/h")]), home),
            PathBuf::from("/h/hub")
        );
        assert_eq!(
            hub_dir_from(env(&[("HF_HOME", "/h")]), home),
            PathBuf::from("/h/hub")
        );
        assert_eq!(
            hub_dir_from(env(&[("HF_HOME", "/h"), ("HF_HUB_CACHE", "/c")]), home),
            PathBuf::from("/c")
        );
        assert_eq!(
            hub_dir_from(
                env(&[("HF_HOME", "/h"), ("HF_HUB_CACHE", "/c"), ("HF_HUB", "/m")]),
                home
            ),
            PathBuf::from("/m")
        );
    }

    /// Every script that resolves the hub resolves it in `hub_dir`'s order, so
    /// a download and the server that loads it cannot look in different
    /// places. Found by walking `scripts/`, never listed by hand, so a new
    /// launcher with the old expression fails here.
    #[test]
    fn every_launcher_resolves_the_hub_in_the_same_order() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts");
        let line = r#"${HF_HUB:-${HF_HUB_CACHE:-${HF_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/huggingface}/hub}}"#;
        let mut found = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for e in std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap()) {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&p) else {
                    continue;
                };
                // A shell default on HF_HUB is a hub resolver.
                if !text.contains("${HF_HUB:-") {
                    continue;
                }
                assert!(
                    text.contains(line),
                    "{} resolves the hub, but not as `{line}`",
                    p.display()
                );
                found.push(p.strip_prefix(&root).unwrap().display().to_string());
            }
        }
        found.sort();
        assert_eq!(found.len(), 8, "the hub resolvers found: {found:?}");
    }

    #[test]
    fn a_pin_path_must_be_plain_and_relative() {
        assert!(plain_relative("vae/model.safetensors"));
        assert!(!plain_relative("../escape"));
        assert!(!plain_relative("/abs"));
        assert!(!plain_relative(""));
    }

    #[test]
    fn the_snapshot_links_to_its_blob_relatively() {
        let hub = Path::new("/c/hub");
        let f = HubFile {
            path: "a/b.gguf",
            sha256: "ab",
            bytes: 1,
        };
        let snap = snapshot_path(hub, "org/name", "rev", &f);
        assert_eq!(
            snap,
            PathBuf::from("/c/hub/models--org--name/snapshots/rev/a/b.gguf")
        );
        let rel = relative_to(&blob_path(hub, "org/name", &f), snap.parent().unwrap());
        assert_eq!(rel, PathBuf::from("../../../blobs/ab"));
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// A one-file HTTP server that honours `Range` (or, with `ranges: false`,
    /// ignores it), can cut a response short, and records what it was asked.
    struct Server {
        base: String,
        seen: Arc<Mutex<Vec<String>>>,
    }

    async fn serve(body: Vec<u8>, ranges: bool, cut_first_at: Option<usize>) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let cut = Arc::new(Mutex::new(cut_first_at));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                let seen = seen2.clone();
                let cut = cut.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let headers: HashMap<String, String> = req
                        .lines()
                        .skip(1)
                        .filter_map(|l| l.split_once(": "))
                        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
                        .collect();
                    seen.lock().unwrap().push(
                        req.lines().next().unwrap_or("").to_string()
                            + headers
                                .get("range")
                                .map(|r| format!(" [{r}]"))
                                .as_deref()
                                .unwrap_or(""),
                    );
                    let start = match headers.get("range") {
                        Some(r) if ranges => r
                            .trim_start_matches("bytes=")
                            .trim_end_matches('-')
                            .parse()
                            .unwrap_or(0),
                        _ => 0,
                    };
                    let slice = &body[start..];
                    let status = if start > 0 {
                        "206 Partial Content"
                    } else {
                        "200 OK"
                    };
                    let range = if start > 0 {
                        format!(
                            "Content-Range: bytes {start}-{}/{}\r\n",
                            body.len() - 1,
                            body.len()
                        )
                    } else {
                        String::new()
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{range}Connection: close\r\n\r\n",
                        slice.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let limit = cut.lock().unwrap().take();
                    let send = match limit {
                        Some(n) => &slice[..n.min(slice.len())],
                        None => slice,
                    };
                    let _ = sock.write_all(send).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Server { base, seen }
    }

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mecha-fetch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn a_pinned_file_lands_as_a_blob_and_a_snapshot_link() {
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let s = serve(body.clone(), true, None).await;
        let hub = scratch();
        let sum = sha(&body);
        let f = HubFile {
            path: "dir/model.gguf",
            sha256: Box::leak(sum.into_boxed_str()),
            bytes: body.len() as u64,
        };
        let mut last = 0;
        let snap = fetch_hub_file_from(&s.base, &hub, "org/name", "rev1", &f, &mut |n| last = n)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&snap).unwrap(), body);
        assert!(std::fs::symlink_metadata(&snap)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(last, body.len() as u64);
        assert_eq!(
            cached(&hub, "org/name", "rev1", &f, true).unwrap(),
            Cached::Verified
        );
        assert_eq!(
            s.seen.lock().unwrap()[0],
            "GET /org/name/resolve/rev1/dir/model.gguf HTTP/1.1"
        );
        // Already there: nothing is asked of the server.
        fetch_hub_file_from(&s.base, &hub, "org/name", "rev1", &f, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(s.seen.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&hub);
    }

    /// Cut off half way, the next call asks for the rest with a range and the
    /// whole file — old bytes and new — is what gets verified.
    #[tokio::test]
    async fn an_interrupted_fetch_resumes_from_where_it_stopped() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
        let s = serve(body.clone(), true, Some(70_000)).await;
        let dir = scratch();
        let dest = dir.join("blob");
        let first = fetch_to(
            &format!("{}/f", s.base),
            &dest,
            &sha(&body),
            body.len() as u64,
            &mut |_| {},
        )
        .await;
        let err = first.unwrap_err().to_string();
        assert!(
            err.contains("run the same command again to resume"),
            "{err}"
        );
        assert!(!dest.exists(), "an interrupted file must never be in place");
        assert_eq!(
            std::fs::metadata(dir.join("blob.part")).unwrap().len(),
            70_000
        );
        fetch_to(
            &format!("{}/f", s.base),
            &dest,
            &sha(&body),
            body.len() as u64,
            &mut |_| {},
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!dir.join("blob.part").exists());
        assert_eq!(s.seen.lock().unwrap()[1], "GET /f HTTP/1.1 [bytes=70000-]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that ignores the range sends the whole file from the start;
    /// it is written from the start, never appended to the partial one.
    #[tokio::test]
    async fn a_server_that_ignores_the_range_starts_the_file_again() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 7) as u8).collect();
        let s = serve(body.clone(), false, Some(20_000)).await;
        let dir = scratch();
        let dest = dir.join("blob");
        let _ = fetch_to(
            &format!("{}/f", s.base),
            &dest,
            &sha(&body),
            body.len() as u64,
            &mut |_| {},
        )
        .await;
        fetch_to(
            &format!("{}/f", s.base),
            &dest,
            &sha(&body),
            body.len() as u64,
            &mut |_| {},
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The wrong bytes are refused and deleted — nothing is left that a later
    /// call could mistake for a resumable start.
    #[tokio::test]
    async fn a_file_that_does_not_match_its_pin_is_refused_and_deleted() {
        let body = b"not the pinned file".to_vec();
        let s = serve(body.clone(), true, None).await;
        let dir = scratch();
        let dest = dir.join("blob");
        let wrong = sha(b"the pinned file");
        let err = fetch_to(
            &format!("{}/f", s.base),
            &dest,
            &wrong,
            body.len() as u64,
            &mut |_| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("not the pinned"), "{err}");
        assert!(!dest.exists());
        assert!(!dir.join("blob.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A partial answer that does not start where the file stopped is refused,
    /// never appended (and the partial file kept, to be resumed or deleted).
    #[tokio::test]
    async fn a_range_that_starts_elsewhere_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let body = b"0123456789";
            let head = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-9/20\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(body).await;
        });
        let dir = scratch();
        let dest = dir.join("blob");
        std::fs::write(dir.join("blob.part"), b"0123456789").unwrap();
        let err = fetch_to(
            &format!("{base}/f"),
            &dest,
            &sha(&[0u8; 20]),
            20,
            &mut |_| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("does not start at byte 10"), "{err}");
        assert_eq!(std::fs::read(dir.join("blob.part")).unwrap(), b"0123456789");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that sends more than the pinned size is cut off at once.
    #[tokio::test]
    async fn more_bytes_than_the_pin_are_refused() {
        let body = vec![1u8; 10_000];
        let s = serve(body.clone(), true, None).await;
        let dir = scratch();
        let dest = dir.join("blob");
        let err = fetch_to(
            &format!("{}/f", s.base),
            &dest,
            &sha(&body[..5_000]),
            5_000,
            &mut |_| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("more than the pinned"), "{err}");
        assert!(!dir.join("blob.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file mecha did not write sits at the snapshot path and does not match:
    /// it is reported, never overwritten.
    #[tokio::test]
    async fn a_hand_placed_file_that_does_not_match_is_never_overwritten() {
        let body = b"pinned bytes".to_vec();
        let s = serve(body.clone(), true, None).await;
        let hub = scratch();
        let sum = sha(&body);
        let f = HubFile {
            path: "m.gguf",
            sha256: Box::leak(sum.into_boxed_str()),
            bytes: body.len() as u64,
        };
        let snap = snapshot_path(&hub, "org/name", "r", &f);
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, b"someone else's").unwrap();
        assert_eq!(
            cached(&hub, "org/name", "r", &f, false).unwrap(),
            Cached::Mismatch
        );
        let err = fetch_hub_file_from(&s.base, &hub, "org/name", "r", &f, &mut |_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not write"), "{err}");
        assert_eq!(std::fs::read(&snap).unwrap(), b"someone else's");
        let _ = std::fs::remove_dir_all(&hub);
    }

    /// A file of the pinned size placed by hand: the cheap check reads none
    /// of it and says so; a fetch hashes it, takes it when it is the pin
    /// (asking the server nothing), and leaves it alone when it is not.
    #[tokio::test]
    async fn a_hand_placed_file_of_the_right_size_is_hashed_before_it_counts() {
        let body = b"pinned bytes".to_vec();
        let s = serve(body.clone(), true, None).await;
        let hub = scratch();
        let sum = sha(&body);
        let f = HubFile {
            path: "m.gguf",
            sha256: Box::leak(sum.into_boxed_str()),
            bytes: body.len() as u64,
        };
        let snap = snapshot_path(&hub, "org/name", "r", &f);
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, &body).unwrap();
        assert_eq!(
            cached(&hub, "org/name", "r", &f, false).unwrap(),
            Cached::Unverified
        );
        fetch_hub_file_from(&s.base, &hub, "org/name", "r", &f, &mut |_| {})
            .await
            .unwrap();
        assert!(
            s.seen.lock().unwrap().is_empty(),
            "a matching hand-placed file needs no download"
        );
        std::fs::write(&snap, b"other bytes!").unwrap();
        assert_eq!(
            cached(&hub, "org/name", "r", &f, false).unwrap(),
            Cached::Unverified
        );
        let err = fetch_hub_file_from(&s.base, &hub, "org/name", "r", &f, &mut |_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not write"), "{err}");
        assert_eq!(std::fs::read(&snap).unwrap(), b"other bytes!");
        let _ = std::fs::remove_dir_all(&hub);
    }

    /// Against Hugging Face itself — a redirect to its CDN, and a range
    /// honoured there: the layout pin, fetched whole, then cut in half and
    /// resumed. `cargo test -p mecha-core --lib fetch -- --ignored`.
    #[tokio::test]
    #[ignore = "network: fetches the 130 MB layout model from huggingface.co"]
    async fn the_layout_pin_fetches_and_resumes_against_hugging_face() {
        let slot = crate::recommend::SLOTS
            .iter()
            .find(|s| s.id == "layout")
            .unwrap();
        let crate::recommend::Source::HuggingFace {
            repo,
            revision,
            files,
        } = slot.rows[0].sources[0]
        else {
            panic!("layout is pinned to a Hugging Face file");
        };
        let f = &files[0];
        let hub = scratch();
        let snap = fetch_hub_file(&hub, repo, revision, f, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            cached(&hub, repo, revision, f, true).unwrap(),
            Cached::Verified
        );
        // Cut it to a partial file and drop the link: the next call resumes.
        let blob = blob_path(&hub, repo, f);
        let part = blob.with_file_name(format!("{}.part", f.sha256));
        std::fs::rename(&blob, &part).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&part)
            .unwrap()
            .set_len(f.bytes / 2)
            .unwrap();
        std::fs::remove_file(&snap).unwrap();
        let mut first_seen = None;
        fetch_hub_file(&hub, repo, revision, f, &mut |n| {
            first_seen.get_or_insert(n);
        })
        .await
        .unwrap();
        assert_eq!(
            first_seen,
            Some(f.bytes / 2),
            "the fetch must start from the partial file"
        );
        assert_eq!(
            cached(&hub, repo, revision, f, true).unwrap(),
            Cached::Verified
        );
        let _ = std::fs::remove_dir_all(&hub);
    }
}
