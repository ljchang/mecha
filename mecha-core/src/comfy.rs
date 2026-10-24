//! A ComfyUI on this machine, over HTTP: the transport `image_generate` and
//! `video_generate` share.
//!
//! Only the wire is here: an address proved to be loopback, a client that
//! follows no redirect and no proxy, the wait for a server that is
//! restarting, an upload into the server's temp directory, taking a job back
//! off the server, and reading back what a job wrote. What a job *is* — its
//! graph, its files, what its result must look like, what is written down
//! before the server sees it — stays with the tool that submits it
//! (`imagegen`, `videogen`), so neither tool's rules leak into the other's.
//!
//! **Nothing here starts a server.** The operator runs ComfyUI
//! (`comfyui_unit` installs mecha's own); a client that started a unit it
//! found down would be a model call starting services on the owner's machine.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Whether a URL's host is this machine: a loopback IP, or `localhost`.
/// The one statement of that rule — [`loopback_url`] and the point-wise
/// pass's R29 check (`pointwise::on_this_machine`) both ask it.
pub fn is_loopback(url: &reqwest::Url) -> bool {
    // `host_str` brackets an IPv6 literal; an IP parses, anything else is a
    // name, and the only name that is this machine by definition is localhost.
    match url
        .host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
    {
        Some(host) => match host.parse::<std::net::IpAddr>() {
            Ok(ip) => ip.is_loopback(),
            Err(_) => host.eq_ignore_ascii_case("localhost"),
        },
        None => false,
    }
}

/// A server URL, if it is on this machine. `key` is the setting it came
/// from (`[image] url`) and `tool` what sends to it (`the image tool`), both
/// for the refusal.
///
/// A tool that sends here declares no egress, and this is what makes that
/// true: a host that is not loopback is refused, so the declaration cannot be
/// configured into a lie. A tunnel listening on loopback (`ssh -L`) still
/// passes — the operator built that on purpose, and no URL can reveal it.
pub fn loopback_url(key: &str, tool: &str, raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).with_context(|| format!("{key} `{raw}`"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("{key} `{raw}` must be http or https");
    }
    if !is_loopback(&url) {
        bail!(
            "{key} `{raw}` is not on this machine — {tool} sends model-written \
             prompts to it, so it must be loopback (127.0.0.1, ::1 or localhost)"
        );
    }
    Ok(url)
}

/// Why a job did not produce what it was for.
pub(crate) enum Failure {
    Cancelled,
    Other(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Failure::Other(e)
    }
}

/// How long a job waits for a server that is not answering yet
/// ([`Client::await_server`]), and how often it asks meanwhile. Tests wait
/// two seconds, so the ones aimed at a closed port stay fast.
///
/// **A Mac waits as long as its proxy gives ComfyUI to start** (180 s, with
/// margin): this is the ceiling `UNSEEN_WAIT` is clamped under, so the two
/// move together — 90 s here made a 200 s unseen wait unreachable (review of
/// #689, pass 4).
#[cfg(not(test))]
pub(crate) const SERVER_WAIT: Duration =
    Duration::from_secs(if cfg!(target_os = "macos") { 200 } else { 90 });
#[cfg(test)]
pub(crate) const SERVER_WAIT: Duration = Duration::from_secs(2);
const SERVER_WAIT_STEP: Duration = Duration::from_millis(500);
/// The wait for a server this process has never had an answer from. The
/// processes that meet a restart are the unseen ones - a one-shot `mecha
/// run`, the first picture after a `serve` restart - so it covers the slowest
/// start measured (40 s once; 4-10 s usually), while a stopped service is
/// still reported in half of [`SERVER_WAIT`] (review of #515).
///
/// **A Mac waits as long as its proxy does.** Its ComfyUI starts on demand
/// (`comfyui_unit::for_mac`) and nothing is forwarded until it is ready, so
/// an unseen server there is one being started, which the proxy gives 180 s
/// — 45 s gave up on a cold start that would have answered (review of #689).
#[cfg(not(test))]
pub(crate) const UNSEEN_WAIT: Duration =
    Duration::from_secs(if cfg!(target_os = "macos") { 200 } else { 45 });
#[cfg(test)]
pub(crate) const UNSEEN_WAIT: Duration = Duration::from_secs(1);

/// How many polls to wait for an interrupted job to reach the server's
/// history before reading it back from there.
const FORGET_WAIT_POLLS: u32 = 10;

/// The first stretch of a server's error body — enough to name the problem,
/// never a page of it in the model's context.
pub(crate) fn clip(s: &str) -> String {
    const CAP: usize = 600;
    if s.chars().count() <= CAP {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(CAP).collect::<String>())
    }
}

/// One ComfyUI, by its vetted address.
pub(crate) struct Client {
    base: reqwest::Url,
    http: reqwest::Client,
    /// How often a job's state is asked for.
    pub(crate) poll: Duration,
    /// Whether the server has answered this process at least once. One that
    /// has, and now does not, is restarting and is waited for in full
    /// ([`SERVER_WAIT`]); one never seen may simply not be running, and gets
    /// only [`UNSEEN_WAIT`] - enough for a restart already under way, not a
    /// minute and a half of every job against a stopped service.
    pub(crate) answered: AtomicBool,
    /// How a message names this server: `the image server`.
    who: &'static str,
}

impl Client {
    /// The client for a server at `base`, which the caller has already
    /// proved loopback ([`loopback_url`]).
    pub(crate) fn new(mut base: reqwest::Url, who: &'static str) -> Result<Self> {
        // `join` replaces the last path segment unless the base ends in `/`.
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        // The vetted address is the only one this client may reach. A
        // redirect would walk it elsewhere — a 307/308 on `/prompt` re-sends
        // the body, prompt and all — while the tool goes on declaring no
        // egress; an inherited proxy setting would route the loopback call
        // through someone else. `fetch_vetted` treats a redirect as fatal for
        // the same reason (found on review of #303).
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .with_context(|| format!("building {who}'s HTTP client"))?;
        Ok(Client {
            base,
            http,
            poll: Duration::from_secs(1),
            answered: AtomicBool::new(false),
            who,
        })
    }

    /// The server's address, ending in `/`.
    pub(crate) fn base(&self) -> &reqwest::Url {
        &self.base
    }

    pub(crate) fn endpoint(&self, path: &str) -> Result<reqwest::Url> {
        Ok(self.base.join(path)?)
    }

    pub(crate) async fn get_json(&self, path: &str) -> Result<Value> {
        let res = self
            .http
            .get(self.endpoint(path)?)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            bail!("{path} answered {status}: {}", clip(&body));
        }
        Ok(serde_json::from_str(&body)?)
    }

    pub(crate) async fn post_json(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<(reqwest::StatusCode, String)> {
        let res = self
            .http
            .post(self.endpoint(path)?)
            .timeout(Duration::from_secs(30))
            .json(body)
            .send()
            .await?;
        let status = res.status();
        Ok((status, res.text().await?))
    }

    /// Wait out a server that is restarting: the first request after one
    /// meets a closed port or a slow answer, and that is a few seconds of
    /// start, not a server that is gone. ComfyUI is restarted on purpose when
    /// it sits idle holding a model (`scripts/comfyui/comfyui-idle-reset`,
    /// measured 4-10 s, once 40 s), and by every deploy; failing the job at
    /// the first refused connect told the model the server was down. Only a
    /// connect or timeout failure is waited on - a server that answers, even
    /// with an error, is answered - and a cancel ends the wait.
    pub(crate) async fn await_server(
        &self,
        cancel: Option<&CancellationToken>,
        wait: Duration,
    ) -> std::result::Result<(), Failure> {
        let wait = if self.answered.load(Ordering::Relaxed) {
            wait
        } else {
            wait.min(UNSEEN_WAIT)
        };
        let deadline = Instant::now() + wait;
        loop {
            let attempt = self
                .http
                .get(self.endpoint("queue")?)
                .timeout(Duration::from_secs(5))
                .send();
            let err = match attempt.await {
                Ok(_) => {
                    self.answered.store(true, Ordering::Relaxed);
                    return Ok(());
                }
                Err(e) if (e.is_connect() || e.is_timeout()) && Instant::now() < deadline => e,
                Err(e) => return Err(Failure::Other(e.into())),
            };
            tracing::debug!("{} not answering yet, waiting: {err}", self.who);
            let pause = tokio::time::sleep(SERVER_WAIT_STEP);
            match cancel {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => return Err(Failure::Cancelled),
                    _ = pause => {}
                },
                None => pause.await,
            }
        }
    }

    /// Put `bytes` in the server's temp directory as `filename` (a name the
    /// caller has already written down wherever it keeps them), and return
    /// the name the server filed it under; `label` names it in a refusal.
    /// Multipart by hand: one file and one field do not justify a crate
    /// feature.
    ///
    /// Only temp: the server empties it when it starts, and anywhere else a
    /// private photo or a voice would outlive the call with nothing to clear
    /// it (found on review of #306). A graph names the file with ` [temp]`
    /// after it, which `LoadImage` and `LoadAudio` both read.
    pub(crate) async fn upload_temp(
        &self,
        bytes: &[u8],
        filename: &str,
        label: &str,
    ) -> Result<String> {
        // The name goes into a header unescaped: a quote, a backslash or a
        // control character would end it early or start a header of its own.
        // Refused here rather than promised by every caller (review of #732).
        if filename.is_empty()
            || filename
                .chars()
                .any(|c| c.is_control() || c == '"' || c == '\\')
        {
            bail!("uploading {label}: `{filename}` is not a name the upload can carry");
        }
        let boundary = format!("mecha-{}", uuid::Uuid::new_v4().simple());
        let mut body = Vec::with_capacity(bytes.len() + 512);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; \
                 filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"type\"\r\n\r\n\
                 temp\r\n--{boundary}--\r\n"
            )
            .as_bytes(),
        );
        let res = self
            .http
            .post(self.endpoint("upload/image")?)
            .timeout(Duration::from_secs(60))
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            bail!("uploading {label} failed ({status}): {}", clip(&text));
        }
        let answer: Value = serde_json::from_str(&text)?;
        let kind = answer.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "temp" {
            bail!(
                "{} filed {label} as `{kind}` rather than temp, where nothing clears it",
                self.who
            );
        }
        if answer
            .get("subfolder")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            bail!("{} filed {label} under a subfolder", self.who);
        }
        answer
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("{} did not name the upload of {label}", self.who))
    }

    /// Take a job off the server wherever it is — out of the queue if it has
    /// not started, interrupted if it has — and return its history entry, if
    /// it left one, for the caller to read what it wrote before forgetting
    /// it ([`Client::forget`]). Best effort: the caller is already failing.
    ///
    /// `/interrupt` is sent only when *this* job is the one running. Current
    /// ComfyUI honours the `prompt_id` in the body, but older servers ignore
    /// it and stop whatever is executing — which would let cancelling a
    /// queued job kill another call's running one. Asking the queue first
    /// makes it right on either (found on review of #303).
    pub(crate) async fn take_off(&self, id: &str) -> Option<Value> {
        let _ = self.post_json("queue", &json!({"delete": [id]})).await;
        let running = self
            .get_json("queue")
            .await
            .ok()
            .and_then(|q| q.get("queue_running")?.as_array().cloned())
            .unwrap_or_default();
        // Each entry is `[number, prompt_id, prompt, extra, outputs]`.
        if running
            .iter()
            .any(|job| job.get(1).and_then(Value::as_str) == Some(id))
        {
            let _ = self.post_json("interrupt", &json!({"prompt_id": id})).await;
            // `/interrupt` answers when the flag is set, not when the job has
            // stopped; the server writes the interrupted job into its history
            // as it unwinds, after that. A delete sent now arrives before the
            // record exists and deletes nothing, leaving the prompt behind
            // (found on review of #306). So wait, briefly, for the record.
            for _ in 0..FORGET_WAIT_POLLS {
                let recorded = self
                    .get_json(&format!("history/{id}"))
                    .await
                    .ok()
                    .is_some_and(|h| h.get(id).is_some());
                if recorded {
                    break;
                }
                tokio::time::sleep(self.poll).await;
            }
        }
        // A cancelled or interrupted job is still recorded, prompt and all. A
        // job taken off the queue before it ran never reaches the history.
        // One that finished as it was stopped also wrote its outputs, which
        // only the record names.
        self.get_json(&format!("history/{id}"))
            .await
            .ok()
            .and_then(|mut h| h.get_mut(id).map(Value::take))
    }

    /// The bytes of one file a job wrote, as its history entry names it
    /// (`filename`, `subfolder`, `type`); `what` names it in a refusal.
    /// Unchecked: what the bytes must be is the caller's to say.
    pub(crate) async fn view(&self, file: &Value, what: &str) -> Result<Vec<u8>> {
        let field = |k: &str| {
            file.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let mut url = self.endpoint("view")?;
        url.query_pairs_mut()
            .append_pair("filename", &field("filename"))
            .append_pair("subfolder", &field("subfolder"))
            .append_pair("type", &field("type"));
        let res = self
            .http
            .get(url)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        if !res.status().is_success() {
            bail!("fetching {what} failed ({})", res.status());
        }
        Ok(res.bytes().await?.to_vec())
    }

    /// Drop a job from the server's history. Best effort.
    pub(crate) async fn forget(&self, id: &str) {
        let _ = self.post_json("history", &json!({"delete": [id]})).await;
    }

    /// The server's `/system_stats`, or `None` if it cannot say — including
    /// a server that is restarting. Any answer, even an error, is a server
    /// that is up, and marks it answered.
    pub(crate) async fn stats(&self) -> Option<Value> {
        let res = self
            .http
            .get(self.endpoint("system_stats").ok()?)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        self.answered.store(true, Ordering::Relaxed);
        if !res.status().is_success() {
            return None;
        }
        res.json().await.ok()
    }

    /// Ask the server to drop its models and free the memory they held.
    /// Whether the server took it: an answer that is not a success is no
    /// unload, and must not be recorded as one (review of #648, pass 5).
    pub(crate) async fn release(&self) -> bool {
        matches!(
            self.post_json("free", &json!({"unload_models": true, "free_memory": true}))
                .await,
            Ok((status, _)) if status.is_success()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A loopback address nothing listens on yet.
    fn closed_port() -> std::net::SocketAddr {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        addr
    }

    fn client_at(addr: std::net::SocketAddr) -> Client {
        let url = loopback_url("[test] url", "the test", &format!("http://{addr}")).unwrap();
        Client::new(url, "the test server").unwrap()
    }

    #[test]
    fn only_a_server_on_this_machine_is_accepted() {
        for ok in [
            "http://127.0.0.1:8188",
            "http://localhost:8189/",
            "http://[::1]:8188",
        ] {
            loopback_url("[video] url", "the video tool", ok)
                .unwrap_or_else(|e| panic!("{ok}: {e:#}"));
        }
        for bad in [
            "http://10.0.0.5:8188",
            "http://comfy.example:8188",
            "ftp://127.0.0.1:8188",
        ] {
            let err = format!(
                "{:#}",
                loopback_url("[video] url", "the video tool", bad).unwrap_err()
            );
            assert!(err.starts_with("[video] url"), "{bad}: {err}");
        }
    }

    /// A name that would break out of its header is refused before anything
    /// is sent (review of #732).
    #[tokio::test]
    async fn an_upload_name_that_could_end_its_header_is_refused() {
        let c = client_at(closed_port());
        for bad in ["a\".png", "a\r\nX-Evil: 1.png", "a\\b.png", ""] {
            let err = format!(
                "{:#}",
                c.upload_temp(b"x", bad, "a test file").await.unwrap_err()
            );
            assert!(
                err.contains("not a name the upload can carry"),
                "{bad:?}: {err}"
            );
        }
    }

    /// A base without a trailing slash still joins under its own path.
    #[test]
    fn endpoints_join_under_the_base() {
        let url = loopback_url("[t] url", "t", "http://127.0.0.1:8189/comfy").unwrap();
        let c = Client::new(url, "t").unwrap();
        assert_eq!(
            c.endpoint("queue").unwrap().as_str(),
            "http://127.0.0.1:8189/comfy/queue"
        );
    }

    /// A server restarting under a job (the idle reset, a deploy) is waited
    /// for: the job asks again until the port answers, rather than telling
    /// the model the server is not running. Never seen before, it is met
    /// within the short wait an idle reset fits in.
    #[tokio::test]
    async fn a_restarting_server_is_waited_for_not_reported_down() {
        let addr = closed_port();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                        )
                        .await;
                });
            }
        });
        let started = Instant::now();
        let waited = client_at(addr).await_server(None, SERVER_WAIT).await;
        assert!(waited.is_ok(), "a server that came up was reported down");
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "answered before the server existed"
        );
    }

    /// The wait is bounded: a server that never comes is reported, with the
    /// connection error the tool turns into "Is the image server running?".
    #[tokio::test]
    async fn a_server_that_never_comes_is_reported_after_the_wait() {
        let started = Instant::now();
        let waited = client_at(closed_port())
            .await_server(None, Duration::from_millis(800))
            .await;
        match waited {
            Err(Failure::Other(e)) => assert!(
                e.chain().any(|c| c.is::<reqwest::Error>()),
                "not a connection error: {e:#}"
            ),
            Err(Failure::Cancelled) => panic!("reported as cancelled"),
            Ok(()) => panic!("a closed port answered"),
        }
        assert!(started.elapsed() >= Duration::from_millis(700));
    }

    /// A server never seen by this process gets the short wait, not the full
    /// one: a stopped service is reported in seconds. One that has answered
    /// before is waited for in full, since it is restarting.
    #[tokio::test]
    async fn only_a_server_seen_before_is_waited_for_in_full() {
        let unseen = client_at(closed_port());
        let started = Instant::now();
        assert!(unseen
            .await_server(None, Duration::from_secs(60))
            .await
            .is_err());
        assert!(
            started.elapsed() < UNSEEN_WAIT + Duration::from_secs(2),
            "a never-seen server was waited on for {:?}",
            started.elapsed()
        );

        let seen = client_at(closed_port());
        seen.answered.store(true, Ordering::Relaxed);
        let started = Instant::now();
        assert!(seen.await_server(None, UNSEEN_WAIT * 3).await.is_err());
        assert!(
            started.elapsed() >= UNSEEN_WAIT * 2,
            "a server seen before was given up on after {:?}",
            started.elapsed()
        );
    }

    /// A cancel ends the wait at once: a stopped job does not sit out the
    /// rest of a 90 s window.
    #[tokio::test]
    async fn a_cancel_ends_the_wait() {
        let token = CancellationToken::new();
        let stop = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stop.cancel();
        });
        let started = Instant::now();
        let waited = client_at(closed_port())
            .await_server(Some(&token), Duration::from_secs(60))
            .await;
        assert!(matches!(waited, Err(Failure::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
