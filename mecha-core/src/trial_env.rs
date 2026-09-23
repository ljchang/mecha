//! An experiment's environment: the world a trial home is built from.
//!
//! **A trial home used to be a copy of the operator's.** Its config was the
//! operator's file with keys scrubbed, and its learning store, skills and
//! charter were copied from the real home — so every `[[mcp]]` server and
//! every `[[hook]]` rode in too. On 2026-09-23 that put eleven synthetic
//! sessions into the owner's live knowledge graph: a `session_end` hook ran
//! `mecha distill` inside each trial home, and that home's `graph` server was
//! the owner's. Nothing about the run said so.
//!
//! So the world is now the experiment's own, from a directory the manifest
//! names (`[environment] dir`, default [`DEFAULT_DIR`]):
//!
//! - `config.toml` — the harness: `[agent]`, `[[mcp]]`, `[[hook]]`,
//!   `[outbox]`, skills, subagents. It may not name a **machine fact**
//!   ([`MACHINE_TABLES`]): the providers and their keys, the sandbox, the
//!   security switches, the approval rules and the search backends describe
//!   this machine and the owner's standing word, so they come from the
//!   operator's config and nowhere else — an environment cannot lift a
//!   `forbid` any more than an arm can.
//! - `charter.toml`, `skills/`, `learning/` — seeded into each home once
//!   (`experiment::seed_home`, from here), in place of the real home's.
//! - `stores/<server>/` and `stores/<server>.calls.jsonl` — a server's
//!   starting state. A server in the environment's config that names
//!   [`STORE_TOKEN`] in an `env` value or an argument gets its own store
//!   directory under the home; the token becomes that path. The store is
//!   built once per experiment — the directory copied, then every call in
//!   the `.calls.jsonl` replayed through the server's own tools — and copied
//!   into each home from there. The graph is the case this exists for: the
//!   real `mecha-graph-mcp`, every tool, reads and writes, on
//!   `${STORE}/graph.db`.
//!
//! An operator's own server reaches a trial only by name, in
//! `live_servers`, and the names are part of the condition hash, as the
//! environment directory's whole content is: an arm that ran against the
//! live world and one that did not are different conditions, and a world
//! edited between two runs is a different world.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::{Config, McpServerConfig};

/// The environment a manifest that names none runs in, relative to the
/// checkout `mecha exp` runs from.
pub const DEFAULT_DIR: &str = "eval/envs/default";

/// Stands for a server's store directory under the trial home.
pub const STORE_TOKEN: &str = "${STORE}";

/// The top-level keys an environment's config may not set: this machine's
/// facts and the owner's standing word, taken from the operator's config.
/// The rule tables' TOML names are `rule` and `search`.
pub const MACHINE_TABLES: [&str; 7] = [
    "default_provider",
    "providers",
    "security",
    "sandbox",
    "rule",
    "approval",
    "search",
];

/// Tables a checked-out file may never set: the four a project layer is
/// stripped of (`Config::merge_file`), for the reason given there — a file
/// that arrives with a cloned repository must not name the Slack surface,
/// the web surface, the mailbox, or `[harness] source_dir`, the authority a
/// `ruminate` stage's diagnostician reads on which protections are
/// load-bearing. An environment directory is resolved against a checkout,
/// so it is refused them outright rather than stripped with a warning
/// (found on review).
pub const OPERATOR_ONLY_TABLES: [&str; 4] = ["harness", "messages", "slack", "web"];

/// Marks a finished store build, so a crash mid-build is a rebuild, not a
/// half-seeded world.
const BUILT: &str = ".built-by-mecha-exp";

/// The manifest's `[environment]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// The environment directory, relative to the checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
    /// Operator `[[mcp]]` servers, by name, a trial may reach — the owner's
    /// live world, opted into. Empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_servers: Vec<String>,
}

impl Environment {
    /// The environment directory, resolved against the checkout.
    pub fn dir(&self, base: &Path) -> PathBuf {
        base.join(
            self.dir
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_DIR)),
        )
    }

    /// The harness a trial home starts from: the environment's config, with
    /// the machine facts taken from the operator's and the named live
    /// servers carried over. Relative paths the child would otherwise read
    /// from its workspace — the system prompt, a server's command and file
    /// arguments — are resolved against the checkout.
    pub fn base_config(&self, real: &Config, base: &Path) -> Result<Config> {
        let dir = self.dir(base);
        refuse_operator_home(&dir, &crate::work::mecha_home()?)?;
        let path = dir.join("config.toml");
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "reading the experiment environment's config {} (a manifest with no \
                 [environment] runs in {DEFAULT_DIR}, relative to the checkout)",
                path.display()
            )
        })?;
        let table: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for key in MACHINE_TABLES {
            anyhow::ensure!(
                !table.contains_key(key),
                "{}: `{key}` is a machine fact — providers, sandbox, security, approval rules \
                 and search come from your own config, never from an experiment environment",
                path.display()
            );
        }
        for key in OPERATOR_ONLY_TABLES {
            anyhow::ensure!(
                !table.contains_key(key),
                "{}: `[{key}]` may not be set by a file that arrives with a checkout, and an \
                 experiment environment is one",
                path.display()
            );
        }
        // A stored server's name is a directory under the home and a
        // `remove_dir_all` target on every fresh trial: one plain
        // component, as `Fixtures::validate` requires of a fixture server
        // (where `..` once resolved to the home itself).
        if let Some(servers) = table.get("mcp").and_then(|v| v.as_array()) {
            for server in servers {
                if let Some(name) = server.get("name").and_then(|v| v.as_str()) {
                    anyhow::ensure!(
                        !name.is_empty()
                            && Path::new(name).components().count() == 1
                            && !matches!(name, "." | "..")
                            && !name.contains(['/', '\\'])
                            && !name.contains("__"),
                        "{}: server name `{name}` must be one plain path component \
                         (no `/`, `.`, `..` or `__`) — it names a store directory",
                        path.display()
                    );
                }
            }
        }
        let mut cfg = Config::default();
        cfg.merge_environment_file(&path)?;
        cfg.default_provider = real.default_provider.clone();
        cfg.providers = real.providers.clone();
        cfg.security = real.security.clone();
        cfg.sandbox = real.sandbox.clone();
        cfg.rules = real.rules.clone();
        cfg.approval = real.approval.clone();
        cfg.search = real.search.clone();
        if let Some(prompt) = &cfg.agent.system_prompt_file {
            if prompt.is_relative() {
                cfg.agent.system_prompt_file = Some(base.join(prompt));
            }
        }
        for server in cfg.mcp.iter_mut() {
            let mut argv = vec![server.command.clone()];
            argv.extend(server.args.iter().cloned());
            crate::experiment::resolve_file_args(&mut argv, base);
            server.command = argv.remove(0);
            server.args = argv;
        }
        for name in &self.live_servers {
            anyhow::ensure!(
                !cfg.mcp.iter().any(|s| &s.name == name),
                "live server `{name}` has the same name as one of the environment's own"
            );
            let live = real.mcp.iter().find(|s| &s.name == name).with_context(|| {
                format!(
                    "live_servers names `{name}`, which is not an [[mcp]] server in your config"
                )
            })?;
            cfg.mcp.push(live.clone());
        }
        cfg.validate()
            .with_context(|| format!("the experiment environment {}", dir.display()))?;
        Ok(cfg)
    }

    /// A digest of everything the environment is: every file under its
    /// directory, by relative path and content, plus the live servers it
    /// opens. A term of every row's condition hash.
    pub fn digest(&self, base: &Path) -> Result<String> {
        let dir = self.dir(base);
        let mut files = Vec::new();
        collect_files(&dir, &dir, &mut files)
            .with_context(|| format!("reading the experiment environment {}", dir.display()))?;
        files.sort();
        let mut bytes = Vec::new();
        for rel in &files {
            bytes.extend_from_slice(rel.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&std::fs::read(dir.join(rel))?);
            bytes.push(0);
        }
        let mut live = self.live_servers.clone();
        live.sort();
        bytes.extend_from_slice(format!("live={}", live.join(",")).as_bytes());
        Ok(crate::experiment::fnv64(&bytes))
    }
}

/// An environment is authored data, never a part of the operator's home:
/// refused if it is the real mecha home, contains it, or lies inside it.
/// Canonical rather than lexical, unlike `refuse_unsafe_home`: an
/// environment must already exist, so a symlink can be seen through. Until
/// this check, `dir = "~/.mecha"` was stopped only because the operator's
/// config happens to name a machine table (found on review).
pub fn refuse_operator_home(dir: &Path, real: &Path) -> Result<()> {
    let env = dir.canonicalize().with_context(|| {
        format!(
            "the experiment environment {} does not exist",
            dir.display()
        )
    })?;
    // A home not created yet is resolved as far as it exists: absolute,
    // its nearest existing ancestor canonical, the rest appended. Compared
    // as written it waved everything through when relative, and missed a
    // symlinked ancestor (macOS's /var → /private/var) (found on review).
    let real = resolve_existing_prefix(real)?;
    anyhow::ensure!(
        !env.starts_with(&real) && !real.starts_with(&env),
        "the experiment environment {} is, contains or lies inside your mecha home {} — an \
         environment is authored data, never your home",
        env.display(),
        real.display()
    );
    Ok(())
}

/// A path in the form `canonicalize` would give, for a path that may not
/// exist yet: made absolute, `.`/`..` resolved, the longest existing prefix
/// canonicalized (through any symlink) and the remainder appended.
fn resolve_existing_prefix(p: &Path) -> Result<PathBuf> {
    let absolute = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .context("cannot determine the working directory")?
            .join(p)
    };
    let mut lexical = PathBuf::new();
    for c in absolute.components() {
        match c {
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            std::path::Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut existing = lexical.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            let mut out = canonical;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return Ok(lexical),
        }
    }
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).expect("under the root");
            out.push(rel.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

/// The environment's servers that keep a store: those naming
/// [`STORE_TOKEN`] in an `env` value or an argument.
pub fn stored_servers(cfg: &Config) -> Vec<&McpServerConfig> {
    cfg.mcp
        .iter()
        .filter(|s| {
            s.env.values().any(|v| v.contains(STORE_TOKEN))
                || s.args.iter().any(|a| a.contains(STORE_TOKEN))
        })
        .collect()
}

/// Replace [`STORE_TOKEN`] in one server's `env` and arguments.
fn bind(server: &mut McpServerConfig, store: &Path) {
    let dir = store.to_string_lossy();
    for v in server.env.values_mut() {
        *v = v.replace(STORE_TOKEN, &dir);
    }
    for a in server.args.iter_mut() {
        *a = a.replace(STORE_TOKEN, &dir);
    }
}

/// A server's store under a home.
pub fn store_dir(home: &Path, server: &str) -> PathBuf {
    home.join("stores").join(server)
}

/// Point every stored server in `cfg` at its store under `home`.
pub fn bind_stores(cfg: &mut Config, home: &Path) {
    for server in cfg.mcp.iter_mut() {
        let store = store_dir(home, &server.name);
        bind(server, &store);
    }
}

/// Build every stored server's starting state once, under `cache`
/// (`<experiment>/environment/<digest>/stores/`): copy the environment's
/// `stores/<name>/`, then replay `stores/<name>.calls.jsonl` in order. A
/// line is either a call through the server's own tools,
/// `{"tool": "<name>", "arguments": {…}}`, or a command,
/// `{"run": ["argv", …]}`, run with the server's bound environment and
/// nothing else of ours but `PATH` — the graph's `mecha-graph embed`, which
/// sizes a fresh database's vector tables to the embedder and embeds what
/// the calls wrote. Any call answered as an error, and any command that
/// exits non-zero, fails the build: a world that seeded partly is not the
/// world the design names. A finished build is marked and never redone; a
/// crashed one is cleared and rebuilt.
pub async fn build_stores(env_dir: &Path, cfg: &Config, cache: &Path) -> Result<()> {
    for server in stored_servers(cfg) {
        let done = cache.join(&server.name);
        if done.join(BUILT).exists() {
            continue;
        }
        if done.exists() {
            std::fs::remove_dir_all(&done)
                .with_context(|| format!("clearing a half-built store {}", done.display()))?;
        }
        let staging = cache.join(format!(
            ".{}.building-{}",
            server.name,
            uuid::Uuid::new_v4()
        ));
        let built = build_one(env_dir, server, &cfg.sandbox, &staging).await;
        if let Err(e) = built {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e.context(format!("building the `{}` store", server.name)));
        }
        std::fs::write(staging.join(BUILT), b"built by mecha exp\n")?;
        std::fs::rename(&staging, &done).with_context(|| format!("placing {}", done.display()))?;
    }
    Ok(())
}

async fn build_one(
    env_dir: &Path,
    server: &McpServerConfig,
    sandbox: &crate::sandbox::SandboxConfig,
    staging: &Path,
) -> Result<()> {
    std::fs::create_dir_all(staging)?;
    let seed = env_dir.join("stores").join(&server.name);
    if seed.is_dir() {
        crate::experiment::copy_tree(&seed, staging)?;
    }
    let calls = env_dir
        .join("stores")
        .join(format!("{}.calls.jsonl", server.name));
    if !calls.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&calls)?;
    let mut bound = server.clone();
    bind(&mut bound, staging);
    // The operator's sandbox, as every other `connect` site passes: a
    // server the trial child confines is confined while its store is built
    // too (found on review — the first cut built it unconfined).
    let sandbox = crate::sandbox::Sandbox::new(sandbox.clone());
    let client = crate::mcp::McpClient::connect(&bound, &sandbox, staging)
        .await
        .with_context(|| format!("starting `{}` to replay {}", server.name, calls.display()))?;
    let replayed: Result<()> = async {
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let step: SeedStep = serde_json::from_str(line)
                .with_context(|| format!("{}:{}", calls.display(), n + 1))?;
            match step {
                SeedStep::Call { tool, arguments } => {
                    let out = client
                        .call_tool(&tool, arguments)
                        .await
                        .with_context(|| format!("{}:{} ({tool})", calls.display(), n + 1))?;
                    anyhow::ensure!(
                        !out.is_error,
                        "{}:{}: `{tool}` answered with an error: {}",
                        calls.display(),
                        n + 1,
                        out.content.chars().take(300).collect::<String>()
                    );
                }
                SeedStep::Run { run } => {
                    // A command runs on the host with an environment
                    // allowlist and nothing more. A server that asks to be
                    // confined cannot have its store touched that way, so
                    // it is refused rather than run unconfined.
                    anyhow::ensure!(
                        !server.sandbox,
                        "{}:{}: `run` steps execute unconfined, and `{}` is configured with \
                         `sandbox = true` — seed it through its tools instead",
                        calls.display(),
                        n + 1,
                        server.name
                    );
                    let (program, args) = run.split_first().with_context(|| {
                        format!("{}:{}: an empty `run`", calls.display(), n + 1)
                    })?;
                    let mut cmd = tokio::process::Command::new(program);
                    cmd.args(args)
                        .env_clear()
                        .envs(&bound.env)
                        .current_dir(staging)
                        .stdin(std::process::Stdio::null());
                    if let Some(path) = std::env::var_os("PATH") {
                        cmd.env("PATH", path);
                    }
                    let out = cmd.output().await.with_context(|| {
                        format!("{}:{}: starting `{program}`", calls.display(), n + 1)
                    })?;
                    anyhow::ensure!(
                        out.status.success(),
                        "{}:{}: `{}` exited {}: {}",
                        calls.display(),
                        n + 1,
                        run.join(" "),
                        out.status,
                        String::from_utf8_lossy(&out.stderr)
                            .chars()
                            .take(300)
                            .collect::<String>()
                    );
                }
            }
        }
        Ok(())
    }
    .await;
    let _ = client.close().await;
    replayed
}

/// One line of a `.calls.jsonl`.
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum SeedStep {
    Call {
        tool: String,
        #[serde(default)]
        arguments: serde_json::Value,
    },
    Run {
        run: Vec<String>,
    },
}

/// Put each stored server's state into `home` from the built cache: always
/// when `fresh` (a `single` starts every trial from the seed), otherwise
/// only where the home has none yet (a lifetime keeps what its tasks did).
pub fn place_stores(cfg: &Config, cache: &Path, home: &Path, fresh: bool) -> Result<()> {
    for server in stored_servers(cfg) {
        let to = store_dir(home, &server.name);
        if to.exists() {
            if !fresh {
                continue;
            }
            std::fs::remove_dir_all(&to).with_context(|| format!("clearing {}", to.display()))?;
        }
        let from = cache.join(&server.name);
        anyhow::ensure!(
            from.join(BUILT).exists(),
            "the `{}` store was never built — `mecha exp run` builds it before the first trial",
            server.name
        );
        crate::experiment::copy_tree(&from, &to)?;
        let _ = std::fs::remove_file(to.join(BUILT));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory removed when the test ends.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("mecha-trial-env-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn env_at(root: &Path, config: &str) -> Environment {
        let dir = root.join("env");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), config).unwrap();
        Environment {
            dir: Some(PathBuf::from("env")),
            live_servers: Vec::new(),
        }
    }

    fn operator() -> Config {
        let mut real = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        real.providers.insert("local".into(), Default::default());
        real.security.trifecta = crate::config::TrifectaPolicy::Ask;
        real.mcp.push(McpServerConfig {
            name: "graph".into(),
            command: "mecha-graph-mcp".into(),
            ..McpServerConfig::default()
        });
        real.hooks.push(crate::config::HookConfig {
            event: "session_end".into(),
            command: "nohup mecha distill &".into(),
            ..Default::default()
        });
        real
    }

    /// The operator's servers and hooks do not ride in — the leak this
    /// module closes — while the machine facts do, and a server reaches a
    /// trial only by name.
    #[test]
    fn the_harness_is_the_environments_and_the_machine_is_the_operators() {
        let tmp = Scratch::new();
        let env = env_at(
            tmp.path(),
            r#"
[agent]
max_turns = 7
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"
env = { MECHA_GRAPH_DB = "${STORE}/graph.db" }
"#,
        );
        let real = operator();
        let cfg = env.base_config(&real, tmp.path()).unwrap();
        assert_eq!(cfg.agent.max_turns, 7);
        assert!(cfg.hooks.is_empty(), "the operator's hooks stay home");
        assert_eq!(cfg.mcp.len(), 1);
        assert_eq!(cfg.mcp[0].env["MECHA_GRAPH_DB"], "${STORE}/graph.db");
        assert_eq!(cfg.default_provider, "local");
        assert!(cfg.providers.contains_key("local"));
        assert_eq!(cfg.security.trifecta, real.security.trifecta);

        let mut bound = cfg.clone();
        bind_stores(&mut bound, Path::new("/h"));
        assert_eq!(
            bound.mcp[0].env["MECHA_GRAPH_DB"],
            "/h/stores/graph/graph.db"
        );
        assert_eq!(stored_servers(&cfg).len(), 1);

        // A live server is named, and collides with nothing.
        let clash = Environment {
            live_servers: vec!["graph".into()],
            ..env.clone()
        };
        assert!(clash.base_config(&real, tmp.path()).is_err());
        let unknown = Environment {
            live_servers: vec!["nope".into()],
            ..env.clone()
        };
        assert!(unknown.base_config(&real, tmp.path()).is_err());
    }

    /// A machine fact in an environment is a load error, whichever one.
    #[test]
    fn an_environment_cannot_name_a_machine_fact() {
        for body in [
            "default_provider = \"x\"",
            "[providers.x]\nkind = \"openai\"",
            "[security]\ntrifecta = \"allow\"",
            "[sandbox]\nbackend = \"none\"",
            "[[rule]]\nprefix = [\"ls\"]\ndecision = \"allow\"",
            "[approval]\nstrict_inline_eval = false",
            "[[search]]\nkind = \"brave\"",
        ] {
            let tmp = Scratch::new();
            let env = env_at(tmp.path(), body);
            let err = env.base_config(&operator(), tmp.path()).unwrap_err();
            assert!(
                format!("{err:#}").contains("machine fact"),
                "{body}: {err:#}"
            );
        }
        // And the four a checked-out file never sets.
        for body in [
            "[harness]\nsource_dir = \"/elsewhere\"",
            "[messages]\nenabled = true",
            "[slack]",
            "[web]",
        ] {
            let tmp = Scratch::new();
            let env = env_at(tmp.path(), body);
            let err = env.base_config(&operator(), tmp.path()).unwrap_err();
            assert!(format!("{err:#}").contains("checkout"), "{body}: {err:#}");
        }
    }

    /// An environment that is, contains or sits inside the real home is
    /// refused, through a symlink too; a sibling is fine.
    #[test]
    fn an_environment_is_never_the_operators_home() {
        let tmp = Scratch::new();
        let real = tmp.path().join("home/.mecha");
        std::fs::create_dir_all(real.join("envs/inside")).unwrap();
        let sibling = tmp.path().join("checkout/eval/envs/x");
        std::fs::create_dir_all(&sibling).unwrap();
        std::os::unix::fs::symlink(&real, tmp.path().join("alias")).unwrap();
        assert!(refuse_operator_home(&real, &real).is_err());
        assert!(refuse_operator_home(&tmp.path().join("home"), &real).is_err());
        assert!(refuse_operator_home(&real.join("envs/inside"), &real).is_err());
        assert!(refuse_operator_home(&tmp.path().join("alias"), &real).is_err());
        refuse_operator_home(&sibling, &real).unwrap();
        assert!(refuse_operator_home(&tmp.path().join("missing"), &real).is_err());
        // A home not created yet is compared lexically, not waved through.
        let unborn = tmp.path().join("fresh/.mecha");
        assert!(refuse_operator_home(&tmp.path().join("home"), &unborn).is_ok());
        std::fs::create_dir_all(tmp.path().join("fresh")).unwrap();
        assert!(refuse_operator_home(&tmp.path().join("fresh"), &unborn).is_err());
        // Through a symlinked ancestor, as macOS's temp directory is.
        let linked = tmp.path().join("linked");
        std::os::unix::fs::symlink(tmp.path().join("fresh"), &linked).unwrap();
        assert!(refuse_operator_home(&tmp.path().join("fresh"), &linked.join(".mecha")).is_err());
    }

    /// A relative path that does not exist yet resolves against the working
    /// directory, and a missing tail keeps its components.
    #[test]
    fn a_missing_path_resolves_as_far_as_it_exists() {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(
            resolve_existing_prefix(Path::new("no-such-dir/x/../y")).unwrap(),
            cwd.join("no-such-dir/y")
        );
    }

    /// A server's name is a store directory and a `remove_dir_all` target:
    /// one plain component or a load error.
    #[test]
    fn a_server_name_is_one_plain_component() {
        for name in ["../..", "..", ".", "a/b", "a__b", ""] {
            let tmp = Scratch::new();
            let env = env_at(
                tmp.path(),
                &format!(
                    "[[mcp]]\nname = {name:?}\ncommand = \"x\"\nenv = {{ D = \"${{STORE}}\" }}\n"
                ),
            );
            assert!(
                env.base_config(&operator(), tmp.path()).is_err(),
                "{name:?}"
            );
        }
    }

    /// The shipped default environment loads, names only files that exist,
    /// and seeds its graph through lines that parse and end in the embed
    /// that sizes the vector tables. The first cut shipped without the
    /// seed file (a `*.jsonl` ignore rule) and no test noticed.
    #[test]
    fn the_default_environment_is_complete() {
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let env = Environment::default();
        let dir = env.dir(checkout);
        let mut real = operator();
        real.mcp.clear();
        let cfg = env.base_config(&real, checkout).unwrap();
        assert!(cfg.hooks.is_empty());
        assert!(cfg
            .agent
            .system_prompt_file
            .as_ref()
            .is_some_and(|p| p.is_file()));
        let stored: Vec<&str> = stored_servers(&cfg)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(stored, ["graph", "mail"]);
        for server in &cfg.mcp {
            for arg in &server.args {
                if arg.ends_with(".py") {
                    assert!(Path::new(arg).is_file(), "{arg}");
                }
            }
        }
        assert!(dir.join("stores/graph/config.toml").is_file());
        assert!(dir.join("stores/mail/mailbox.json").is_file());
        let calls = std::fs::read_to_string(dir.join("stores/graph.calls.jsonl")).unwrap();
        let steps: Vec<SeedStep> = calls
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with("//"))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(steps.len() > 1);
        assert!(
            matches!(steps.last(), Some(SeedStep::Run { run }) if run == &["mecha-graph", "embed"])
        );
        crate::charter::Charter::parse(&std::fs::read_to_string(dir.join("charter.toml")).unwrap())
            .unwrap();
    }

    /// The digest moves with any file's content and with the live servers,
    /// and not with nothing.
    #[test]
    fn the_digest_is_the_whole_directory_and_the_live_servers() {
        let tmp = Scratch::new();
        let env = env_at(tmp.path(), "[agent]\nmax_turns = 7\n");
        let a = env.digest(tmp.path()).unwrap();
        assert_eq!(a, env.digest(tmp.path()).unwrap());
        std::fs::write(tmp.path().join("env/charter.toml"), "# x\n").unwrap();
        let b = env.digest(tmp.path()).unwrap();
        assert_ne!(a, b);
        let live = Environment {
            live_servers: vec!["graph".into()],
            ..env.clone()
        };
        assert_ne!(b, live.digest(tmp.path()).unwrap());
    }

    /// A seed line is a tool call or a command, never both and never
    /// anything else: an unknown key is a load error, not an ignored field.
    #[test]
    fn a_seed_line_is_a_call_or_a_command() {
        let call: SeedStep =
            serde_json::from_str(r#"{"tool": "kg_upsert", "arguments": {"source": "s"}}"#).unwrap();
        assert!(matches!(call, SeedStep::Call { ref tool, .. } if tool == "kg_upsert"));
        let run: SeedStep = serde_json::from_str(r#"{"run": ["mecha-graph", "embed"]}"#).unwrap();
        assert!(matches!(run, SeedStep::Run { ref run } if run.len() == 2));
        for bad in [
            r#"{"tool": "x", "run": ["y"]}"#,
            r#"{"tool": "x", "args": {}}"#,
            r#"{"cmd": ["y"]}"#,
        ] {
            assert!(serde_json::from_str::<SeedStep>(bad).is_err(), "{bad}");
        }
    }

    /// A store is copied from the built cache into a home: every time for a
    /// `single`, once for a lifetime, and never from a build that did not
    /// finish.
    #[test]
    fn stores_are_placed_from_a_finished_build_only() {
        let tmp = Scratch::new();
        let env = env_at(
            tmp.path(),
            "[[mcp]]\nname = \"graph\"\ncommand = \"x\"\nenv = { DB = \"${STORE}/g.db\" }\n",
        );
        let cfg = env.base_config(&operator(), tmp.path()).unwrap();
        let cache = tmp.path().join("cache");
        let home = tmp.path().join("home");
        assert!(
            place_stores(&cfg, &cache, &home, true).is_err(),
            "never built"
        );
        std::fs::create_dir_all(cache.join("graph")).unwrap();
        std::fs::write(cache.join("graph/g.db"), "seed").unwrap();
        std::fs::write(cache.join("graph").join(BUILT), "").unwrap();
        place_stores(&cfg, &cache, &home, true).unwrap();
        let db = store_dir(&home, "graph").join("g.db");
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "seed");
        assert!(!store_dir(&home, "graph").join(BUILT).exists());
        std::fs::write(&db, "written by a task").unwrap();
        place_stores(&cfg, &cache, &home, false).unwrap();
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "written by a task");
        place_stores(&cfg, &cache, &home, true).unwrap();
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "seed");
    }
}
