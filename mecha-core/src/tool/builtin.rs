//! Built-in tools. No server required — these are ordinary Rust functions.
//!
//! Every filesystem path here arrives as model output, so it goes through
//! [`ToolCtx::resolve`] before it reaches the filesystem. Shell commands are
//! likewise untrusted: they run under the approval gate, not around it.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::sandbox::Sandbox;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn all(sandbox: Arc<Sandbox>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(FsRead),
        Arc::new(FsWrite),
        Arc::new(FsEdit),
        Arc::new(FsList),
        Arc::new(Shell::new(sandbox)),
        Arc::new(HttpFetch),
        Arc::new(super::todo::TodoTool::new()),
    ]
}

/// Model output can be enormous; truncate at a size that stays readable in
/// context instead of blowing the window on one file.
const MAX_OUTPUT_BYTES: usize = 200_000;

fn truncate(mut s: String, what: &str) -> String {
    if s.len() > MAX_OUTPUT_BYTES {
        let mut cut = MAX_OUTPUT_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        let total = s.len();
        s.truncate(cut);
        s.push_str(&format!(
            "\n\n[truncated: {what} was {total} bytes, showing first {cut}]"
        ));
    }
    s
}

fn arg_str<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

pub struct FsRead;

#[async_trait]
impl Tool for FsRead {
    fn name(&self) -> &str {
        "fs_read"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace. Use `offset` and `limit` (1-indexed lines) \
         to read part of a large file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace root, or absolute inside it."},
                "offset": {"type": "integer", "description": "First line to return, 1-indexed."},
                "limit": {"type": "integer", "description": "Maximum number of lines to return."}
            },
            "required": ["path"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        // Your files are the definition of private data.
        Capabilities::default().private()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&input, "path")?)?;
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        };

        let offset = input
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .map(|l| l as usize);
        if offset == 1 && limit.is_none() {
            return Ok(ToolOutput::ok(truncate(text, "file")));
        }

        let selected: Vec<&str> = text
            .lines()
            .skip(offset - 1)
            .take(limit.unwrap_or(usize::MAX))
            .collect();
        Ok(ToolOutput::ok(truncate(selected.join("\n"), "selection")))
    }
}

pub struct FsWrite;

#[async_trait]
impl Tool for FsWrite {
    fn name(&self) -> &str {
        "fs_write"
    }

    fn description(&self) -> &str {
        "Create a file or replace its entire contents. To change part of an existing file, \
         prefer fs_edit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().destructive()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&input, "path")?)?;
        let content = arg_str(&input, "content")?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolOutput::ok(format!(
                "wrote {} bytes to {}",
                content.len(),
                path.display()
            ))),
            Err(e) => Ok(ToolOutput::err(format!(
                "cannot write {}: {e}",
                path.display()
            ))),
        }
    }
}

pub struct FsEdit;

#[async_trait]
impl Tool for FsEdit {
    fn name(&self) -> &str {
        "fs_edit"
    }

    fn description(&self) -> &str {
        "Replace one exact occurrence of `old` with `new` in a file. Fails if `old` appears \
         zero times or more than once, so include enough surrounding context to be unique."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old": {"type": "string", "description": "Exact text to replace, including indentation."},
                "new": {"type": "string"}
            },
            "required": ["path", "old", "new"]
        })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().destructive()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&input, "path")?)?;
        let old = arg_str(&input, "old")?;
        let new = arg_str(&input, "new")?;

        let text = match tokio::fs::read_to_string(&path).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        };

        // Ambiguity here silently edits the wrong line, so refuse instead.
        match text.matches(old).count() {
            0 => return Ok(ToolOutput::err("`old` does not appear in the file")),
            1 => {}
            n => {
                return Ok(ToolOutput::err(format!(
                    "`old` appears {n} times; include more surrounding context to make it unique"
                )))
            }
        }

        tokio::fs::write(&path, text.replacen(old, new, 1)).await?;
        Ok(ToolOutput::ok(format!("edited {}", path.display())))
    }
}

pub struct FsList;

#[async_trait]
impl Tool for FsList {
    fn name(&self) -> &str {
        "fs_list"
    }

    fn description(&self) -> &str {
        "List the entries of a directory. Directories are suffixed with `/`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Defaults to the workspace root."}
            }
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let raw = input.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = ctx.resolve(raw)?;
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "cannot list {}: {e}",
                    path.display()
                )))
            }
        };

        let mut out = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            out.push(if is_dir { format!("{name}/") } else { name });
        }
        out.sort();
        Ok(ToolOutput::ok(if out.is_empty() {
            "(empty directory)".to_string()
        } else {
            out.join("\n")
        }))
    }
}

/// Runs commands, confined by whatever [`Sandbox`] it was built with.
///
/// The policy lives on the tool rather than in [`ToolCtx`] because it decides
/// the tool's *capabilities*, and `capabilities()` has no context to consult.
/// The workspace still comes from the context at call time, so per-run jails —
/// an eval case's private copy of a fixture — are confined to that copy.
pub struct Shell {
    sandbox: Arc<Sandbox>,
}

impl Shell {
    pub fn new(sandbox: Arc<Sandbox>) -> Self {
        Shell { sandbox }
    }
}

#[async_trait]
impl Tool for Shell {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Run a shell command in the workspace and return its combined stdout and stderr. \
         The command runs to completion; long-running or interactive commands will time out."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "cwd": {"type": "string", "description": "Working directory, defaults to the workspace root."}
            },
            "required": ["command"]
        })
    }

    fn capabilities(&self) -> Capabilities {
        // Unconfined, `shell` is universal: it reads your machine, it can
        // `curl` data out, and it can delete things. Taint tracking cannot see
        // inside a command, so it is deliberately NOT marked as an untrusted
        // *source* — the mitigation for that is the sandbox, not a label.
        //
        // Confined without a network, it stops being an exfiltration route, and
        // *that* is the claim the sandbox earns: the interlock can stop
        // refusing outbound-looking work that provably cannot go anywhere. It
        // narrows only because something else enforces it — see
        // `Sandbox::preflight`, which refuses to start if it doesn't.
        //
        // `private_data` stays true regardless. A confined shell still reads
        // the workspace, and `fs_read` — which reads exactly the same files —
        // is marked private on the grounds that your files are the definition
        // of private data. Narrowing it here would open a hole rather than
        // close one: `shell: cat secrets` would set no taint where
        // `fs_read: secrets` does, and the cheapest way around the interlock
        // would be to use the more dangerous tool.
        Capabilities {
            private_data: true,
            untrusted_input: false,
            external_send: self.sandbox.can_reach_network(),
            destructive: true,
        }
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let command = arg_str(&input, "command")?;
        let cwd = match input.get("cwd").and_then(Value::as_str) {
            Some(c) => ctx.resolve(c)?,
            None => ctx.workspace.clone(),
        };

        // A sandbox that cannot be built refuses the call. Running the command
        // unconfined instead would silently break the promise the capabilities
        // above are making on its behalf.
        let mut command = match self.sandbox.command(command, &ctx.workspace, &cwd) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "refusing to run: the {} sandbox could not be set up ({e:#}). \
                     Nothing was executed.",
                    self.sandbox.backend().as_str()
                )))
            }
        };

        // Streams are drained with a cap rather than collected with
        // `output()`: a command can print without bound, and the harness must
        // not buffer without bound on its behalf. `kill_on_drop` also closes
        // an older hole — without it, a timed-out command kept running after
        // the run had reported it dead.
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => return Ok(ToolOutput::err(format!("cannot run command: {e}"))),
        };
        let out_pipe = child.stdout.take();
        let err_pipe = child.stderr.take();

        let fut = async {
            tokio::join!(
                drain_capped(out_pipe, MAX_OUTPUT_BYTES),
                drain_capped(err_pipe, MAX_OUTPUT_BYTES),
                child.wait(),
            )
        };
        let ((stdout, out_dropped), (stderr, err_dropped), status) =
            match tokio::time::timeout(ctx.shell_timeout, fut).await {
                Err(_) => {
                    return Ok(ToolOutput::err(format!(
                        "command timed out after {}s",
                        ctx.shell_timeout.as_secs()
                    )))
                }
                Ok(v) => v,
            };
        let status = match status {
            Ok(s) => s,
            Err(e) => return Ok(ToolOutput::err(format!("cannot run command: {e}"))),
        };

        let mut body = String::new();
        body.push_str(&String::from_utf8_lossy(&stdout));
        if !stderr.is_empty() {
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(&String::from_utf8_lossy(&stderr));
        }
        // Bound the transcript copy *before* naming the discard: `truncate`
        // cuts the tail, and the tail is exactly where the marker goes.
        let mut body = truncate(body, "output");
        if out_dropped || err_dropped {
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(&format!(
                "[output exceeded {MAX_OUTPUT_BYTES} bytes; the rest was discarded as it streamed. \
                 Redirect to a file and read it in pieces if more is needed.]"
            ));
        }
        if body.trim().is_empty() {
            body.push_str("(no output)");
        }

        let code = status.code().unwrap_or(-1);
        if code != 0 {
            body = format!("exit status {code}\n{body}");
        }
        Ok(ToolOutput {
            content: body,
            is_error: code != 0,
            external: false,
        })
    }
}

/// Read a child's stream to EOF, keeping at most `cap` bytes and discarding
/// the rest as it arrives. Discarding matters as much as capping: a reader
/// that simply stopped would fill the pipe and deadlock the child against a
/// harness that has already decided not to keep the output.
async fn drain_capped(
    pipe: Option<impl tokio::io::AsyncRead + Unpin>,
    cap: usize,
) -> (Vec<u8>, bool) {
    use tokio::io::AsyncReadExt;
    let Some(mut pipe) = pipe else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut dropped = false;
    let mut buf = [0u8; 8192];
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let take = n.min(cap.saturating_sub(kept.len()));
                kept.extend_from_slice(&buf[..take]);
                dropped |= take < n;
            }
        }
    }
    (kept, dropped)
}

pub struct HttpFetch;

#[async_trait]
impl Tool for HttpFetch {
    fn name(&self) -> &str {
        "http_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL over HTTP(S) and return the response body as text."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"}
            },
            "required": ["url"]
        })
    }

    fn read_only(&self) -> bool {
        // Read-only with respect to *your* data — but see `capabilities`: a GET
        // is also an exfiltration channel, because the payload fits in the URL.
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().untrusted().sends()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let url = arg_str(&input, "url")?;
        let vetted = match check_url(url, ctx).await {
            Ok(v) => v,
            Err(e) => return Ok(ToolOutput::err(e.to_string())),
        };

        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            // Following a redirect re-opens everything check_url just closed:
            // a public host can 302 straight to 169.254.169.254.
            .redirect(reqwest::redirect::Policy::none());
        // Pin the connection to the addresses that passed the private-IP
        // check. Without this the client re-resolves the hostname itself, and
        // a DNS answer with TTL 0 can hand the check a public address and the
        // connection 169.254.169.254 — the classic rebinding TOCTOU.
        if let Some((host, addrs)) = &vetted {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = builder.build()?;
        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolOutput::err(format!("request failed: {e}"))),
        };

        if resp.status().is_redirection() {
            let target = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(no location header)");
            return Ok(ToolOutput::err(format!(
                "{} redirect to {target} — not followed. Call http_fetch again with that URL if you want it.",
                resp.status()
            )));
        }

        let status = resp.status();
        // Read at most one byte past the cap, then stop — `.text()` buffers
        // however much the server chooses to send, and the server is the
        // untrusted side of this call. `truncate` below marks the cut.
        let mut raw: Vec<u8> = Vec::new();
        let mut body_stream = resp.bytes_stream();
        while let Some(chunk) = body_stream.next().await {
            match chunk {
                Ok(c) => raw.extend_from_slice(&c),
                Err(e) => {
                    return Ok(ToolOutput::err(format!(
                        "reading the response body failed: {e}"
                    )))
                }
            }
            if raw.len() > MAX_OUTPUT_BYTES {
                break;
            }
        }
        let body = String::from_utf8_lossy(&raw);
        // The body is third-party content even on a 4xx — an injection hides
        // just as well in an error page.
        Ok(ToolOutput {
            content: truncate(format!("HTTP {status}\n\n{body}"), "body"),
            is_error: !status.is_success(),
            external: true,
        })
    }
}

/// Refuse a URL before any packet leaves. Model output decides where this
/// request goes, so "it's just a GET" is not a defense: the LAN, localhost,
/// and cloud metadata endpoints are all reachable from here by default.
///
/// Returns the host and the addresses that passed the private-IP check, so
/// the caller can pin the connection to exactly those — a check that lets the
/// client resolve again afterwards is only advice. `None` when the private-IP
/// guard is off and there is nothing to pin.
async fn check_url(
    url: &str,
    ctx: &ToolCtx,
) -> Result<Option<(String, Vec<std::net::SocketAddr>)>> {
    let parsed = reqwest::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => anyhow::bail!("scheme {other:?} is not allowed (use http or https)"),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("url has no host"))?
        .to_ascii_lowercase();

    let policy = &ctx.security;
    let matches = |pattern: &str| {
        let pattern = pattern.trim_start_matches('.').to_ascii_lowercase();
        host == pattern || host.ends_with(&format!(".{pattern}"))
    };

    if policy.blocked_domains.iter().any(|d| matches(d)) {
        anyhow::bail!("{host} is on the blocked-domain list");
    }
    if !policy.allowed_domains.is_empty() && !policy.allowed_domains.iter().any(|d| matches(d)) {
        anyhow::bail!("{host} is not on the allowed-domain list");
    }

    if policy.block_private_ips {
        // Resolve first and check every address: a hostname under the
        // attacker's control can point at 127.0.0.1 or the metadata service.
        let port = parsed.port_or_known_default().unwrap_or(80);
        let addrs = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| anyhow::anyhow!("cannot resolve {host}: {e}"))?;

        let mut vetted = Vec::new();
        for addr in addrs {
            if is_internal(&addr.ip()) {
                anyhow::bail!(
                    "{host} resolves to the internal address {} — refused",
                    addr.ip()
                );
            }
            vetted.push(addr);
        }
        if vetted.is_empty() {
            anyhow::bail!("{host} did not resolve to any address");
        }
        return Ok(Some((host, vetted)));
    }

    Ok(None)
}

/// Addresses an agent has no business reaching on the user's behalf.
fn is_internal(ip: &std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                // 169.254.0.0/16 — includes the cloud metadata endpoint.
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                // 100.64.0.0/10, carrier-grade NAT and tailnets.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
                // 0.0.0.0/8
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique-local, fe80::/10 link-local.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped addresses must be checked as IPv4.
                || v6.to_ipv4_mapped().is_some_and(|v4| is_internal(&IpAddr::V4(v4)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{Backend, SandboxConfig};
    use crate::tool::ToolCtx;

    fn shell_with(kind: Backend, network: bool) -> Shell {
        Shell::new(Arc::new(Sandbox::new(SandboxConfig {
            kind,
            network,
            ..SandboxConfig::default()
        })))
    }

    #[test]
    fn confining_the_shell_closes_the_send_route_and_nothing_else() {
        let loose = shell_with(Backend::None, false).capabilities();
        assert!(loose.private_data && loose.external_send && loose.destructive);

        // The one thing the sandbox earns: with no network, a command cannot
        // carry anything off the machine, so it is no longer a trifecta sink.
        let confined = shell_with(Backend::Bwrap, false).capabilities();
        assert!(!confined.external_send, "no network means no way out");
        assert!(confined.destructive, "it can still destroy the workspace");

        // Confined *with* a network is a way out again.
        assert!(
            shell_with(Backend::Bwrap, true)
                .capabilities()
                .external_send
        );
    }

    #[test]
    fn a_confined_shell_is_still_private_because_it_still_reads_your_files() {
        // The hole this guards: if a sandboxed `shell` stopped counting as
        // private, `shell: cat secrets.txt` would set no taint while
        // `fs_read: secrets.txt` — the same bytes, the safer tool — would. The
        // cheapest route around the interlock must never be the more dangerous
        // tool.
        for (kind, network) in [
            (Backend::None, false),
            (Backend::Bwrap, false),
            (Backend::Docker, false),
        ] {
            assert!(
                shell_with(kind, network).capabilities().private_data,
                "{kind:?} shell reads the workspace, exactly as fs_read does"
            );
        }
        assert!(
            FsRead.capabilities().private_data,
            "the rule this is matching"
        );
    }

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            workspace: dir.to_path_buf(),
            shell_timeout: std::time::Duration::from_secs(5),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn escaping_the_workspace_is_refused() {
        let dir = std::env::temp_dir().join(format!("mecha-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = ctx(&dir);

        assert!(ctx.resolve("../../etc/passwd").is_err());
        assert!(ctx.resolve("/etc/passwd").is_err());
        assert!(ctx.resolve("notes.md").is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_command_that_floods_stdout_is_capped_not_buffered() {
        let dir = std::env::temp_dir().join(format!("mecha-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let shell = shell_with(Backend::None, false);

        // 1 MB of output against a 200 KB cap. The old `output()` collector
        // kept all of it in memory; a `yes` running to the timeout kept
        // gigabytes.
        let out = shell
            .call(
                json!({"command": "yes flood | head -c 1000000"}),
                &ctx(&dir),
            )
            .await
            .unwrap();

        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.len() <= MAX_OUTPUT_BYTES + 300,
            "kept {} bytes",
            out.content.len()
        );
        assert!(out.content.contains("discarded"), "the cut must be named");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_oversized_response_body_is_cut_at_the_cap_not_buffered_whole() {
        use tokio::io::AsyncWriteExt;

        // A local server that answers with 1 MB. The client must stop reading
        // at the cap — `.text()` would buffer whatever the server sent.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut sock, _) = listener.accept().await.unwrap();
            // Read the request head before answering, so the client never
            // sees a response racing its own send.
            let mut req = Vec::new();
            let mut tmp = [0u8; 4096];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&tmp[..n]),
                }
            }
            let body = vec![b'a'; 1_000_000];
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.shutdown().await;
        });

        // Loopback is the test server, so the private-IP guard steps aside.
        let ctx = ToolCtx {
            security: crate::config::SecurityConfig {
                block_private_ips: false,
                ..Default::default()
            },
            ..ToolCtx::default()
        };
        let out = HttpFetch
            .call(json!({"url": format!("http://{addr}/big")}), &ctx)
            .await
            .unwrap();

        assert!(out.external, "not an HTTP response: {}", out.content);
        assert!(
            out.content.len() <= MAX_OUTPUT_BYTES + 300,
            "kept {} bytes",
            out.content.len()
        );
        assert!(out.content.contains("[truncated"), "the cut must be named");
    }

    #[tokio::test]
    async fn internal_addresses_are_refused_and_public_ones_come_back_pinned() {
        let ctx = ToolCtx::default();

        // Names and literals that resolve internally are refused outright.
        for url in [
            "http://localhost/x",
            "http://127.0.0.1/x",
            "http://169.254.169.254/meta",
        ] {
            let err = check_url(url, &ctx).await.unwrap_err().to_string();
            assert!(err.contains("internal"), "{url}: {err}");
        }

        // A public literal passes, and the vetted addresses come back so the
        // caller can pin the connection to them — returning only Ok(()) here
        // is the rebinding hole: the client would resolve again on its own.
        let vetted = check_url("http://93.184.216.34/x", &ctx).await.unwrap();
        let (host, addrs) = vetted.expect("the private-IP guard is on, so there is a pin");
        assert_eq!(host, "93.184.216.34");
        assert_eq!(addrs, vec!["93.184.216.34:80".parse().unwrap()]);

        // With the guard off there is nothing to pin — the old behaviour.
        let open = ToolCtx {
            security: crate::config::SecurityConfig {
                block_private_ips: false,
                ..Default::default()
            },
            ..ToolCtx::default()
        };
        assert!(check_url("http://127.0.0.1/x", &open)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn edit_refuses_ambiguous_matches() {
        let dir = std::env::temp_dir().join(format!("mecha-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "a\na\n").unwrap();

        let out = FsEdit
            .call(json!({"path": "f.txt", "old": "a", "new": "b"}), &ctx(&dir))
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("appears 2 times"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
