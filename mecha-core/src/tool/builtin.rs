//! Built-in tools. No server required — these are ordinary Rust functions.
//!
//! Every filesystem path here arrives as model output, so it goes through
//! [`ToolCtx::resolve`] before it reaches the filesystem. Shell commands are
//! likewise untrusted: they run under the approval gate, not around it.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn all() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(FsRead),
        Arc::new(FsWrite),
        Arc::new(FsEdit),
        Arc::new(FsList),
        Arc::new(Shell),
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
            Err(e) => return Ok(ToolOutput::err(format!("cannot read {}: {e}", path.display()))),
        };

        let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        let limit = input.get("limit").and_then(Value::as_u64).map(|l| l as usize);
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
            Err(e) => Ok(ToolOutput::err(format!("cannot write {}: {e}", path.display()))),
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
            Err(e) => return Ok(ToolOutput::err(format!("cannot read {}: {e}", path.display()))),
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
            Err(e) => return Ok(ToolOutput::err(format!("cannot list {}: {e}", path.display()))),
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

pub struct Shell;

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
        // `shell` is universal: it reads your machine, it can `curl` data out,
        // and it can delete things. Taint tracking cannot see inside a command,
        // so it is deliberately NOT marked as an untrusted *source* — the real
        // mitigation for shell is a sandbox, not classification. Marking it as
        // a sink is what matters here.
        Capabilities::default().private().sends().destructive()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let command = arg_str(&input, "command")?;
        let cwd = match input.get("cwd").and_then(Value::as_str) {
            Some(c) => ctx.resolve(c)?,
            None => ctx.workspace.clone(),
        };

        let child = tokio::process::Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .output();

        let output = match tokio::time::timeout(ctx.shell_timeout, child).await {
            Err(_) => {
                return Ok(ToolOutput::err(format!(
                    "command timed out after {}s",
                    ctx.shell_timeout.as_secs()
                )))
            }
            Ok(Err(e)) => return Ok(ToolOutput::err(format!("cannot run command: {e}"))),
            Ok(Ok(o)) => o,
        };

        let mut body = String::new();
        body.push_str(&String::from_utf8_lossy(&output.stdout));
        if !output.stderr.is_empty() {
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if body.trim().is_empty() {
            body.push_str("(no output)");
        }

        let code = output.status.code().unwrap_or(-1);
        if code != 0 {
            body = format!("exit status {code}\n{body}");
        }
        Ok(ToolOutput {
            content: truncate(body, "output"),
            is_error: code != 0,
            external: false,
        })
    }
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
        if let Err(e) = check_url(url, ctx).await {
            return Ok(ToolOutput::err(e.to_string()));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            // Following a redirect re-opens everything check_url just closed:
            // a public host can 302 straight to 169.254.169.254.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
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
        let body = resp.text().await.unwrap_or_default();
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
async fn check_url(url: &str, ctx: &ToolCtx) -> Result<()> {
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

        let mut any = false;
        for addr in addrs {
            any = true;
            if is_internal(&addr.ip()) {
                anyhow::bail!(
                    "{host} resolves to the internal address {} — refused",
                    addr.ip()
                );
            }
        }
        if !any {
            anyhow::bail!("{host} did not resolve to any address");
        }
    }

    Ok(())
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
    use crate::tool::ToolCtx;

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
